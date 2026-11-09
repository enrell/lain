//! A scripted HTTP/1.1 server on a Unix socket or loopback TCP, standing
//! in for the bridged apps in tests (this crate's and the server's
//! component tests). Not for production use.

use std::io::{BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::Value;

/// One request the fake received.
#[derive(Debug, Clone)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Value>,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// The script: a request in, `(status, JSON body)` out.
pub type Handler = Arc<dyn Fn(&Seen) -> (u16, Value) + Send + Sync>;

/// A running fake; dropped, it stops accepting.
pub struct Fake {
    /// `unix:///path` or `http://127.0.0.1:port`.
    pub url: String,
    pub seen: Arc<Mutex<Vec<Seen>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    cleanup: Option<std::path::PathBuf>,
}

fn serve_conn<S: Read + Write>(s: S, handler: &Handler, seen: &Mutex<Vec<Seen>>) {
    let mut r = BufReader::new(s);
    let mut line = String::new();
    if r.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).unwrap_or(0) == 0 {
            return;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let len = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0; len];
    if r.read_exact(&mut body).is_err() {
        return;
    }
    let req = Seen { method, path, headers, body: serde_json::from_slice(&body).ok() };
    seen.lock().unwrap().push(req.clone());
    let (status, out) = handler(&req);
    let raw = serde_json::to_vec(&out).unwrap();
    let mut s = r.into_inner();
    let _ = write!(
        s,
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        raw.len()
    );
    let _ = s.write_all(&raw);
    let _ = s.flush();
}

impl Fake {
    /// Listens on loopback TCP.
    pub fn tcp(handler: Handler) -> Fake {
        let lis = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        lis.set_nonblocking(true).unwrap();
        let url = format!("http://{}", lis.local_addr().unwrap());
        Self::run(
            url,
            None,
            move || match lis.accept() {
                Ok((s, _)) => {
                    s.set_nonblocking(false).ok();
                    Some(Box::new(s) as Box<dyn ReadWrite>)
                }
                Err(_) => None,
            },
            handler,
        )
    }

    /// Listens on a Unix socket at `path`.
    #[cfg(unix)]
    pub fn unix(path: &std::path::Path, handler: Handler) -> Fake {
        let _ = std::fs::remove_file(path);
        let lis = std::os::unix::net::UnixListener::bind(path).unwrap();
        lis.set_nonblocking(true).unwrap();
        let url = format!("unix://{}", path.display());
        Self::run(
            url,
            Some(path.to_path_buf()),
            move || match lis.accept() {
                Ok((s, _)) => {
                    s.set_nonblocking(false).ok();
                    Some(Box::new(s) as Box<dyn ReadWrite>)
                }
                Err(_) => None,
            },
            handler,
        )
    }

    fn run(
        url: String,
        cleanup: Option<std::path::PathBuf>,
        accept: impl Fn() -> Option<Box<dyn ReadWrite>> + Send + 'static,
        handler: Handler,
    ) -> Fake {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (seen2, stop2) = (seen.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            while !stop2.load(Ordering::SeqCst) {
                match accept() {
                    Some(c) => serve_conn(c, &handler, &seen2),
                    None => std::thread::sleep(Duration::from_millis(5)),
                }
            }
        });
        Fake { url, seen, stop, thread: Some(thread), cleanup }
    }

    /// The requests so far.
    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    /// The `host:port` of a TCP fake.
    pub fn port(&self) -> u16 {
        self.url.rsplit(':').next().and_then(|p| p.parse().ok()).unwrap_or(0)
    }
}

/// A byte stream the fake can serve.
pub trait ReadWrite: Read + Write + Send {}
impl<T: Read + Write + Send> ReadWrite for T {}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        if let Some(p) = &self.cleanup {
            let _ = std::fs::remove_file(p);
        }
    }
}
