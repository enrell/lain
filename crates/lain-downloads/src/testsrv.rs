//! An in-process fixture HTTP server for the tests: one thread per
//! connection, handlers write raw responses so they can stall, cut a
//! body or ignore ranges.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, Default)]
pub struct TestReq {
    pub path: String,
    pub headers: Vec<(String, String)>,
}

impl TestReq {
    pub fn header(&self, name: &str) -> String {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone()).unwrap_or_default()
    }
}

type Handler = dyn Fn(&TestReq, &mut TcpStream) + Send + Sync;

pub struct Srv {
    pub url: String,
    pub hits: AtomicUsize,
    pub log: Mutex<Vec<TestReq>>,
}

impl Srv {
    pub fn start(h: impl Fn(&TestReq, &mut TcpStream) + Send + Sync + 'static) -> Arc<Srv> {
        let ln = TcpListener::bind("127.0.0.1:0").unwrap();
        let srv = Arc::new(Srv {
            url: format!("http://{}", ln.local_addr().unwrap()),
            hits: AtomicUsize::new(0),
            log: Mutex::default(),
        });
        let h: Arc<Handler> = Arc::new(h);
        let weak = Arc::downgrade(&srv);
        std::thread::spawn(move || {
            for conn in ln.incoming() {
                let Ok(mut s) = conn else { continue };
                let Some(srv) = weak.upgrade() else { return };
                let h = h.clone();
                std::thread::spawn(move || {
                    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
                    let mut r = BufReader::new(s.try_clone().unwrap());
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let path = line.split(' ').nth(1).unwrap_or("/").to_string();
                    let mut headers = Vec::new();
                    loop {
                        let mut l = String::new();
                        if r.read_line(&mut l).unwrap_or(0) == 0 {
                            break;
                        }
                        let l = l.trim_end();
                        if l.is_empty() {
                            break;
                        }
                        if let Some((k, v)) = l.split_once(':') {
                            headers.push((k.trim().to_string(), v.trim().to_string()));
                        }
                    }
                    let req = TestReq { path, headers };
                    srv.hits.fetch_add(1, Ordering::SeqCst);
                    srv.log.lock().unwrap().push(req.clone());
                    drop(srv);
                    h(&req, &mut s);
                    let _ = s.flush();
                });
            }
        });
        srv
    }

    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    /// The `Range` header of every request so far.
    pub fn ranges(&self) -> Vec<String> {
        self.log.lock().unwrap().iter().map(|r| r.header("range")).collect()
    }

    pub fn if_ranges(&self) -> Vec<String> {
        self.log.lock().unwrap().iter().map(|r| r.header("if-range")).collect()
    }
}

pub fn respond(s: &mut TcpStream, status: u16, headers: &[(&str, &str)], body: &[u8]) {
    let _ = s.write_all(&head(status, headers, Some(body.len())));
    let _ = s.write_all(body);
}

pub fn head(status: u16, headers: &[(&str, &str)], len: Option<usize>) -> Vec<u8> {
    let mut h = format!("HTTP/1.1 {status} X\r\n");
    for (k, v) in headers {
        h.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(n) = len {
        h.push_str(&format!("Content-Length: {n}\r\n"));
    }
    h.push_str("Connection: close\r\n\r\n");
    h.into_bytes()
}

/// A one-shot gate the handlers wait on.
#[derive(Clone, Default)]
pub struct Gate(Arc<(Mutex<bool>, Condvar)>);

impl Gate {
    pub fn open(&self) {
        *self.0.0.lock().unwrap() = true;
        self.0.1.notify_all();
    }
    pub fn wait(&self) {
        let mut g = self.0.0.lock().unwrap();
        while !*g {
            g = self.0.1.wait(g).unwrap();
        }
    }
}

/// Go's `http.ServeContent` subset: `Accept-Ranges`, ETag,
/// `Last-Modified`, one `bytes=N-` range honored when `If-Range` (if
/// any) matches the ETag, 416 past the end. `stall_after` bytes of the
/// body go out, then the handler waits for the gate.
pub fn serve_content(req: &TestReq, s: &mut TcpStream, body: &[u8], etag: &str, gate: Option<(&Gate, usize)>) {
    let mut hs: Vec<(String, String)> = vec![
        ("Accept-Ranges".into(), "bytes".into()),
        ("Last-Modified".into(), "Tue, 14 Nov 2023 22:13:20 GMT".into()),
    ];
    if gate.is_none() {
        hs.push(("Content-Disposition".into(), r#"attachment; filename="Frieren - 01.cbz""#.into()));
    }
    if !etag.is_empty() {
        hs.push(("ETag".into(), etag.into()));
    }
    let rg = req.header("range");
    let ir = req.header("if-range");
    let mut start = 0usize;
    let mut status = 200;
    if let Some(n) = rg.strip_prefix("bytes=").and_then(|v| v.strip_suffix('-')).and_then(|v| v.parse::<usize>().ok())
        && (ir.is_empty() || (!etag.is_empty() && ir == etag))
    {
        if n >= body.len() {
            hs.push(("Content-Range".into(), format!("bytes */{}", body.len())));
            let refs: Vec<(&str, &str)> = hs.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
            respond(s, 416, &refs, b"");
            return;
        }
        start = n;
        status = 206;
        hs.push(("Content-Range".into(), format!("bytes {n}-{}/{}", body.len() - 1, body.len())));
    }
    let refs: Vec<(&str, &str)> = hs.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let rest = &body[start..];
    let _ = s.write_all(&head(status, &refs, Some(rest.len())));
    match gate {
        Some((g, after)) if after < rest.len() => {
            let _ = s.write_all(&rest[..after]);
            let _ = s.flush();
            g.wait();
            let _ = s.write_all(&rest[after..]);
        }
        _ => {
            let _ = s.write_all(rest);
        }
    }
}
