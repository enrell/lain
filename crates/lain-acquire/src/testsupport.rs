//! A local HTTP fixture server for tests (no network, no crates).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

/// One request as the handler sees it.
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
}

impl Seen {
    pub fn param(&self, key: &str) -> String {
        self.query
            .split('&')
            .filter_map(|p| p.split_once('='))
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.replace('+', " "))
            .unwrap_or_default()
    }
}

pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn ok(body: impl Into<Vec<u8>>) -> Reply {
        Reply { status: 200, headers: Vec::new(), body: body.into() }
    }
    pub fn status(status: u16) -> Reply {
        Reply { status, headers: Vec::new(), body: Vec::new() }
    }
    pub fn redirect(to: &str) -> Reply {
        Reply { status: 302, headers: vec![("Location".into(), to.into())], body: Vec::new() }
    }
}

type Handler = dyn Fn(&Seen) -> Reply + Send + Sync;

pub struct Fixture {
    pub url: String,
    pub last: Arc<Mutex<Vec<Seen>>>,
}

impl Fixture {
    pub fn last_query(&self) -> String {
        self.last.lock().unwrap().last().map(|s| s.query.clone()).unwrap_or_default()
    }
}

/// Serves `handler` on 127.0.0.1 until the process ends.
pub fn serve(handler: impl Fn(&Seen) -> Reply + Send + Sync + 'static) -> Fixture {
    let ln = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", ln.local_addr().unwrap());
    let last = Arc::new(Mutex::new(Vec::new()));
    let seen = last.clone();
    let handler: Arc<Handler> = Arc::new(handler);
    std::thread::spawn(move || {
        for conn in ln.incoming() {
            let Ok(mut conn) = conn else { continue };
            let handler = handler.clone();
            let seen = seen.clone();
            std::thread::spawn(move || {
                let mut r = BufReader::new(conn.try_clone().unwrap());
                let mut line = String::new();
                if r.read_line(&mut line).is_err() {
                    return;
                }
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap_or("").to_string();
                let target = parts.next().unwrap_or("").to_string();
                let (path, query) = target.split_once('?').unwrap_or((&target, ""));
                let mut s = Seen { method, path: path.into(), query: query.into(), headers: Vec::new() };
                let mut len = 0usize;
                loop {
                    let mut h = String::new();
                    if r.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                        break;
                    }
                    if let Some((k, v)) = h.trim_end().split_once(':') {
                        if k.eq_ignore_ascii_case("content-length") {
                            len = v.trim().parse().unwrap_or(0);
                        }
                        s.headers.push((k.to_string(), v.trim().to_string()));
                    }
                }
                let mut body = vec![0u8; len];
                let _ = r.read_exact(&mut body);
                seen.lock().unwrap().push(s.clone());
                let rep = handler(&s);
                let mut out =
                    format!("HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n", rep.status, rep.body.len());
                for (k, v) in &rep.headers {
                    out.push_str(&format!("{k}: {v}\r\n"));
                }
                out.push_str("\r\n");
                let _ = conn.write_all(out.as_bytes());
                let _ = conn.write_all(&rep.body);
            });
        }
    });
    Fixture { url, last }
}
