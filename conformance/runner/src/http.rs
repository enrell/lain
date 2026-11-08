//! A minimal blocking HTTP/1.1 client over `std::net`, enough for the
//! suite: one request per connection (`Connection: close`), chunked or
//! sized bodies, and a streaming reader for `text/event-stream`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::mpsc;
use std::time::Duration;

/// A server base URL (`http://host:port`, no TLS).
#[derive(Debug, Clone)]
pub struct Base {
    pub host: String,
    pub port: u16,
}

impl Base {
    pub fn parse(url: &str) -> Result<Base, String> {
        let rest = url.strip_prefix("http://").ok_or("LAIN_URL must start with http://")?;
        let hostport = rest.trim_end_matches('/');
        if hostport.contains('/') {
            return Err("LAIN_URL must not have a path".into());
        }
        let (host, port) = match hostport.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse().map_err(|_| format!("bad port in {url}"))?),
            None => (hostport.to_string(), 80),
        };
        Ok(Base { host, port })
    }

    fn connect(&self, timeout: Duration) -> Result<TcpStream, String> {
        let s = TcpStream::connect((self.host.as_str(), self.port)).map_err(|e| format!("connect {}:{}: {e}", self.host, self.port))?;
        s.set_read_timeout(Some(timeout)).ok();
        s.set_write_timeout(Some(timeout)).ok();
        Ok(s)
    }
}

/// One response.
#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    /// Lower-cased names.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers.iter().find(|(k, _)| *k == name).map(|(_, v)| v.as_str())
    }
}

fn write_request(s: &mut TcpStream, base: &Base, method: &str, path: &str, headers: &[(String, String)], body: Option<&[u8]>, close: bool) -> Result<(), String> {
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {}:{}\r\n", base.host, base.port);
    if close {
        req.push_str("Connection: close\r\n");
    }
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(b) = body {
        req.push_str(&format!("Content-Length: {}\r\n", b.len()));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    if let Some(b) = body {
        s.write_all(b).map_err(|e| e.to_string())?;
    }
    s.flush().map_err(|e| e.to_string())
}

fn read_head<R: BufRead>(r: &mut R) -> Result<(u16, Vec<(String, String)>), String> {
    let mut line = String::new();
    r.read_line(&mut line).map_err(|e| format!("read status: {e}"))?;
    let status = line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("bad status line {line:?}"))?;
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        r.read_line(&mut h).map_err(|e| format!("read header: {e}"))?;
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    Ok((status, headers))
}

/// Reads one chunk of a chunked body; `None` at the terminating chunk.
fn read_chunk<R: BufRead>(r: &mut R) -> Result<Option<Vec<u8>>, String> {
    let mut size = String::new();
    r.read_line(&mut size).map_err(|e| format!("chunk size: {e}"))?;
    let size = size.trim().split(';').next().unwrap_or("");
    let n = usize::from_str_radix(size, 16).map_err(|_| format!("bad chunk size {size:?}"))?;
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf).map_err(|e| format!("chunk body: {e}"))?;
    let mut crlf = [0u8; 2];
    r.read_exact(&mut crlf).map_err(|e| format!("chunk end: {e}"))?;
    if n == 0 {
        return Ok(None);
    }
    Ok(Some(buf))
}

/// Sends one request and reads the whole response.
pub fn request(base: &Base, method: &str, path: &str, headers: &[(String, String)], body: Option<&[u8]>) -> Result<Response, String> {
    let mut s = base.connect(Duration::from_secs(30))?;
    write_request(&mut s, base, method, path, headers, body, true)?;
    let mut r = BufReader::new(s);
    let (status, headers) = read_head(&mut r)?;
    let mut body = Vec::new();
    let chunked = headers.iter().any(|(k, v)| k == "transfer-encoding" && v.eq_ignore_ascii_case("chunked"));
    if method == "HEAD" || status == 204 || status == 304 {
        // no body
    } else if chunked {
        while let Some(c) = read_chunk(&mut r)? {
            body.extend_from_slice(&c);
        }
    } else if let Some(n) = headers.iter().find(|(k, _)| k == "content-length").and_then(|(_, v)| v.parse::<usize>().ok()) {
        body.resize(n, 0);
        r.read_exact(&mut body).map_err(|e| format!("body: {e}"))?;
    } else {
        r.read_to_end(&mut body).map_err(|e| format!("body: {e}"))?;
    }
    Ok(Response { status, headers, body })
}

/// One server-sent event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: String,
    pub data: String,
}

/// An open event stream; events arrive on a channel from a reader
/// thread. Dropping it closes the connection.
pub struct SseStream {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub events: mpsc::Receiver<SseEvent>,
    conn: TcpStream,
}

impl Drop for SseStream {
    fn drop(&mut self) {
        let _ = self.conn.shutdown(std::net::Shutdown::Both);
    }
}

impl SseStream {
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers.iter().find(|(k, _)| *k == name).map(|(_, v)| v.as_str())
    }
}

/// Opens a `GET` event stream. A non-200 answer is returned with no
/// events.
pub fn open_sse(base: &Base, path: &str, headers: &[(String, String)]) -> Result<SseStream, String> {
    let mut s = base.connect(Duration::from_secs(10))?;
    let mut hs = headers.to_vec();
    hs.push(("Accept".into(), "text/event-stream".into()));
    write_request(&mut s, base, "GET", path, &hs, None, false)?;
    let conn = s.try_clone().map_err(|e| e.to_string())?;
    s.set_read_timeout(None).ok();
    let mut r = BufReader::new(s);
    let (status, headers) = read_head(&mut r)?;
    let (tx, rx) = mpsc::channel();
    let chunked = headers.iter().any(|(k, v)| k == "transfer-encoding" && v.eq_ignore_ascii_case("chunked"));
    if status == 200 {
        std::thread::spawn(move || {
            let mut pending = Vec::new();
            let mut event = String::new();
            let mut data: Vec<String> = Vec::new();
            loop {
                let bytes = if chunked {
                    match read_chunk(&mut r) {
                        Ok(Some(c)) => c,
                        _ => return,
                    }
                } else {
                    let mut buf = [0u8; 4096];
                    match r.read(&mut buf) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf[..n].to_vec(),
                    }
                };
                pending.extend_from_slice(&bytes);
                while let Some(pos) = pending.iter().position(|b| *b == b'\n') {
                    let line: Vec<u8> = pending.drain(..=pos).collect();
                    let line = String::from_utf8_lossy(&line).trim_end_matches(['\n', '\r']).to_string();
                    if line.is_empty() {
                        if !data.is_empty() || !event.is_empty() {
                            let ev = SseEvent {
                                event: if event.is_empty() { "message".into() } else { std::mem::take(&mut event) },
                                data: data.join("\n"),
                            };
                            data.clear();
                            if tx.send(ev).is_err() {
                                return;
                            }
                        }
                    } else if line.starts_with(':') {
                        // comment / keep-alive
                    } else if let Some(v) = line.strip_prefix("event:") {
                        event = v.trim_start().to_string();
                    } else if let Some(v) = line.strip_prefix("data:") {
                        data.push(v.strip_prefix(' ').unwrap_or(v).to_string());
                    }
                }
            }
        });
    }
    Ok(SseStream { status, headers, events: rx, conn })
}
