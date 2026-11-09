//! The upstream fixture server: recorded answers of the remote APIs
//! lain's network providers call (metadata services, subtitle
//! providers), served from `conformance/fixtures/upstream/` on
//! loopback so no test and no conformance case reaches the network.
//! Both servers point their provider base URLs at it
//! (`LAIN_METADATA_<NAME>_URL=<url>/<name>`).
//!
//! `routes.json` (`lain.upstream-fixtures@1`) lists routes, first match
//! wins:
//!
//! ```json
//! {"method": "GET", "path": "/tvmaze/search/shows", "query": {"q": "Show Name"},
//!  "headers": {"Api-Key": "key-1"}, "body_contains": ["Page("],
//!  "status": 200, "file": "tvmaze/search-show-name.json"}
//! ```
//!
//! `query` values compare decoded (so `+` and `%20` are one space),
//! header names ignore case, `body` (any JSON) or `file` is the answer
//! and `{{upstream}}` in it becomes the server's own URL. An unmatched
//! request answers 404 `{"error":"no fixture"}` and is logged to
//! stderr. Every request is recorded for tests that count calls.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::Value;

/// One recorded request.
#[derive(Debug, Clone, Default)]
pub struct Seen {
    pub method: String,
    pub path: String,
    /// Decoded query pairs, in order.
    pub query: Vec<(String, String)>,
    pub raw_query: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn param(&self, name: &str) -> Option<&str> {
        self.query.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone)]
struct Route {
    method: String,
    path: String,
    query: BTreeMap<String, String>,
    headers: BTreeMap<String, String>,
    body_contains: Vec<String>,
    status: u16,
    response: Vec<u8>,
}

impl Route {
    fn matches(&self, s: &Seen) -> bool {
        self.method.eq_ignore_ascii_case(&s.method)
            && self.path == s.path
            && self.query.iter().all(|(k, v)| s.param(k) == Some(v.as_str()))
            && self.headers.iter().all(|(k, v)| s.header(k) == Some(v.as_str()))
            && self.body_contains.iter().all(|b| s.body.contains(b.as_str()))
    }
}

/// The fixture directory in this repository.
pub fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/upstream")
}

fn strings(v: &Value) -> BTreeMap<String, String> {
    v.as_object()
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn load(dir: &Path) -> Result<Vec<Route>, String> {
    let path = dir.join("routes.json");
    let raw = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let doc: Value = serde_json::from_slice(&raw).map_err(|e| format!("{}: {e}", path.display()))?;
    if doc["format"] != "lain.upstream-fixtures@1" {
        return Err(format!("{}: format must be lain.upstream-fixtures@1", path.display()));
    }
    let mut out = Vec::new();
    for (i, r) in doc["routes"].as_array().ok_or("routes must be an array")?.iter().enumerate() {
        let response = match (&r["file"], &r["body"]) {
            (Value::String(f), _) => {
                std::fs::read(dir.join(f)).map_err(|e| format!("route {i}: {}: {e}", dir.join(f).display()))?
            }
            (_, Value::Null) => Vec::new(),
            (_, Value::String(s)) => s.as_bytes().to_vec(),
            (_, v) => serde_json::to_vec(v).map_err(|e| e.to_string())?,
        };
        out.push(Route {
            method: r["method"].as_str().unwrap_or("GET").to_string(),
            path: r["path"].as_str().ok_or_else(|| format!("route {i}: path required"))?.to_string(),
            query: strings(&r["query"]),
            headers: strings(&r["headers"]),
            body_contains: match &r["body_contains"] {
                Value::String(s) => vec![s.clone()],
                Value::Array(a) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
                _ => Vec::new(),
            },
            status: r["status"].as_u64().unwrap_or(200) as u16,
            response,
        });
    }
    Ok(out)
}

fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(v) => {
                    out.push(v);
                    i += 2;
                }
                Err(_) => out.push(b'%'),
            },
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn read_request(r: &mut BufReader<TcpStream>) -> Option<Seen> {
    let mut line = String::new();
    if r.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();
    let (path, raw_query) = target.split_once('?').unwrap_or((&target, ""));
    let mut seen = Seen {
        method,
        path: unescape(path),
        raw_query: raw_query.to_string(),
        query: raw_query
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|p| {
                let (k, v) = p.split_once('=').unwrap_or((p, ""));
                (unescape(k), unescape(v))
            })
            .collect(),
        ..Seen::default()
    };
    let mut chunked = false;
    let mut len = 0usize;
    loop {
        let mut h = String::new();
        r.read_line(&mut h).ok()?;
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        let (k, v) = h.split_once(':')?;
        let (k, v) = (k.trim().to_string(), v.trim().to_string());
        if k.eq_ignore_ascii_case("content-length") {
            len = v.parse().ok()?;
        }
        if k.eq_ignore_ascii_case("transfer-encoding") && v.eq_ignore_ascii_case("chunked") {
            chunked = true;
        }
        seen.headers.push((k, v));
    }
    let mut body = Vec::new();
    if chunked {
        loop {
            let mut size = String::new();
            r.read_line(&mut size).ok()?;
            let n = usize::from_str_radix(size.trim().split(';').next()?, 16).ok()?;
            let mut chunk = vec![0; n + 2];
            r.read_exact(&mut chunk).ok()?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..n]);
        }
    } else {
        body = vec![0; len];
        r.read_exact(&mut body).ok()?;
    }
    seen.body = String::from_utf8_lossy(&body).into_owned();
    Some(seen)
}

/// A running fixture server. It stops serving when dropped.
pub struct Upstream {
    pub url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
}

impl Upstream {
    /// Serves `dir` (with its `routes.json`) on `127.0.0.1:port` (0 =
    /// any free port).
    pub fn start(dir: &Path, port: u16) -> Result<Upstream, String> {
        let routes = Arc::new(load(dir)?);
        let l = TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("upstream bind: {e}"))?;
        let url = format!("http://{}", l.local_addr().map_err(|e| e.to_string())?);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (s2, stop2, own) = (seen.clone(), stop.clone(), url.clone());
        std::thread::spawn(move || {
            for stream in l.incoming() {
                if stop2.load(std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                let Ok(stream) = stream else { continue };
                let (routes, seen, own) = (routes.clone(), s2.clone(), own.clone());
                std::thread::spawn(move || {
                    let Ok(mut w) = stream.try_clone() else { return };
                    let mut r = BufReader::new(stream);
                    while let Some(req) = read_request(&mut r) {
                        let (status, body) = match routes.iter().find(|rt| rt.matches(&req)) {
                            Some(rt) => {
                                let text = String::from_utf8_lossy(&rt.response).replace("{{upstream}}", &own);
                                (rt.status, text.into_bytes())
                            }
                            None => {
                                eprintln!("upstream: no fixture for {} {}?{}", req.method, req.path, req.raw_query);
                                (404, br#"{"error":"no fixture"}"#.to_vec())
                            }
                        };
                        seen.lock().unwrap_or_else(|e| e.into_inner()).push(req);
                        let head = format!(
                            "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                            if status < 400 { "OK" } else { "Error" },
                            body.len()
                        );
                        if w.write_all(head.as_bytes()).and_then(|_| w.write_all(&body)).is_err() {
                            return;
                        }
                    }
                });
            }
        });
        Ok(Upstream { url, seen, stop })
    }

    /// Every request so far, in arrival order.
    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Requests whose path starts with `prefix`.
    pub fn hits(&self, prefix: &str) -> usize {
        self.seen().iter().filter(|s| s.path.starts_with(prefix)).count()
    }

    /// Blocks serving until the process ends (the CLI mode).
    pub fn wait(self) -> ! {
        loop {
            std::thread::park();
        }
    }
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        // Wake the accept loop so it sees the flag.
        let _ = TcpStream::connect(self.url.trim_start_matches("http://"));
    }
}
