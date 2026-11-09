//! A small blocking HTTP/1.1 client for loopback services: a Unix
//! socket (`unix:///path`) or plain TCP (`http://host:port`). The
//! bridged apps all listen on this machine without TLS, so the
//! outbound client of `lain-http` (rustls, pacing, retries) is not the
//! right tool, and none of them needs more than one request per
//! connection (`Connection: close`). Bodies are bounded both ways.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::time::Duration;

/// Where a service listens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    Unix(PathBuf),
    /// `host:port`.
    Tcp(String),
}

impl Endpoint {
    /// `unix:///abs/path`, `http://host:port` (no path, no TLS), or a
    /// bare absolute path (a socket).
    pub fn parse(raw: &str) -> Result<Endpoint, String> {
        let raw = raw.trim();
        if let Some(p) = raw.strip_prefix("unix://") {
            if p.is_empty() {
                return Err("unix:// needs a socket path".into());
            }
            return Ok(Endpoint::Unix(PathBuf::from(p)));
        }
        if let Some(rest) = raw.strip_prefix("http://") {
            let hostport = rest.trim_end_matches('/');
            if hostport.is_empty() || hostport.contains('/') || hostport.contains('@') {
                return Err(format!("{raw}: want http://host:port"));
            }
            let hostport = if hostport.rsplit_once(':').is_some_and(|(_, p)| p.parse::<u16>().is_ok())
                && !hostport.ends_with(']')
            {
                hostport.to_string()
            } else {
                format!("{hostport}:80")
            };
            return Ok(Endpoint::Tcp(hostport));
        }
        if raw.starts_with('/') {
            return Ok(Endpoint::Unix(PathBuf::from(raw)));
        }
        Err(format!("{raw}: want unix:///path or http://host:port"))
    }

    /// The `Host` header: loopback services check it against DNS
    /// rebinding, so it names the address actually dialed.
    fn host_header(&self) -> String {
        match self {
            Endpoint::Unix(_) => "localhost".into(),
            Endpoint::Tcp(hp) => hp.clone(),
        }
    }

    /// For logs and errors: never a path inside a query, never a token.
    pub fn describe(&self) -> String {
        match self {
            Endpoint::Unix(p) => format!("unix:{}", p.display()),
            Endpoint::Tcp(hp) => hp.clone(),
        }
    }
}

/// One answer.
#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.body).ok()
    }
}

/// One request.
pub struct Request<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub bearer: Option<&'a str>,
    pub json: Option<&'a serde_json::Value>,
    pub timeout: Duration,
    /// The largest body read back.
    pub max_body: usize,
}

impl<'a> Request<'a> {
    pub fn new(method: &'a str, path: &'a str) -> Self {
        Request { method, path, bearer: None, json: None, timeout: Duration::from_secs(5), max_body: 1 << 20 }
    }
}

trait Stream: Read + Write {}
impl<T: Read + Write> Stream for T {}

fn connect(ep: &Endpoint, timeout: Duration) -> io::Result<Box<dyn Stream>> {
    match ep {
        #[cfg(unix)]
        Endpoint::Unix(p) => {
            let s = std::os::unix::net::UnixStream::connect(p)?;
            s.set_read_timeout(Some(timeout))?;
            s.set_write_timeout(Some(timeout))?;
            Ok(Box::new(s))
        }
        #[cfg(not(unix))]
        Endpoint::Unix(_) => Err(io::Error::new(io::ErrorKind::Unsupported, "unix sockets need a Unix host")),
        Endpoint::Tcp(hp) => {
            use std::net::ToSocketAddrs;
            let mut last = io::Error::new(io::ErrorKind::NotFound, "no address");
            for addr in hp.to_socket_addrs()? {
                match std::net::TcpStream::connect_timeout(&addr, timeout) {
                    Ok(s) => {
                        s.set_read_timeout(Some(timeout))?;
                        s.set_write_timeout(Some(timeout))?;
                        s.set_nodelay(true).ok();
                        return Ok(Box::new(s));
                    }
                    Err(e) => last = e,
                }
            }
            Err(last)
        }
    }
}

fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// Sends one request and reads the whole answer.
pub fn send(ep: &Endpoint, req: &Request<'_>) -> io::Result<Response> {
    let mut s = connect(ep, req.timeout)?;
    let body = match req.json {
        Some(v) => serde_json::to_vec(v).map_err(io::Error::other)?,
        None => Vec::new(),
    };
    let mut head = format!(
        "{} {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: lain-bridge\r\nAccept: application/json\r\nConnection: close\r\n",
        req.method,
        req.path,
        ep.host_header()
    );
    if let Some(t) = req.bearer {
        head.push_str(&format!("Authorization: Bearer {t}\r\n"));
    }
    if req.json.is_some() {
        head.push_str("Content-Type: application/json\r\n");
    }
    if req.json.is_some() || matches!(req.method, "POST" | "PUT" | "PATCH") {
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes())?;
    s.write_all(&body)?;
    s.flush()?;
    read_response(BufReader::new(s), req.max_body, req.method == "HEAD")
}

fn read_line<R: BufRead>(r: &mut R) -> io::Result<String> {
    let mut line = Vec::new();
    let n = r.by_ref().take(16 << 10).read_until(b'\n', &mut line)?;
    if n == 0 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "connection closed"));
    }
    if !line.ends_with(b"\n") {
        return Err(bad("header line too long"));
    }
    while line.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
        line.pop();
    }
    String::from_utf8(line).map_err(|_| bad("header is not UTF-8"))
}

fn read_response<R: BufRead>(mut r: R, max_body: usize, head_only: bool) -> io::Result<Response> {
    let status_line = read_line(&mut r)?;
    let mut parts = status_line.splitn(3, ' ');
    let proto = parts.next().unwrap_or("");
    if !proto.starts_with("HTTP/1.") {
        return Err(bad("not an HTTP/1 answer"));
    }
    let status: u16 = parts.next().and_then(|s| s.parse().ok()).ok_or_else(|| bad("bad status line"))?;
    let mut headers = Vec::new();
    loop {
        let line = read_line(&mut r)?;
        if line.is_empty() {
            break;
        }
        if headers.len() > 100 {
            return Err(bad("too many headers"));
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let resp = Response { status, headers, body: Vec::new() };
    if head_only || status == 204 || status == 304 || (100..200).contains(&status) {
        return Ok(resp);
    }
    let chunked = resp.header("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
    let length = resp.header("content-length").map(|v| v.parse::<usize>().map_err(|_| bad("bad content-length")));
    let body = if chunked {
        read_chunked(&mut r, max_body)?
    } else if let Some(len) = length {
        let len = len?;
        if len > max_body {
            return Err(bad(format!("body over {max_body} bytes")));
        }
        let mut b = vec![0; len];
        r.read_exact(&mut b)?;
        b
    } else {
        let mut b = Vec::new();
        r.take(max_body as u64 + 1).read_to_end(&mut b)?;
        if b.len() > max_body {
            return Err(bad(format!("body over {max_body} bytes")));
        }
        b
    };
    Ok(Response { body, ..resp })
}

fn read_chunked<R: BufRead>(r: &mut R, max_body: usize) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let line = read_line(r)?;
        let size = usize::from_str_radix(line.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| bad("bad chunk size"))?;
        if size == 0 {
            // Trailers until the blank line.
            while !read_line(r)?.is_empty() {}
            return Ok(out);
        }
        if out.len() + size > max_body {
            return Err(bad(format!("body over {max_body} bytes")));
        }
        let start = out.len();
        out.resize(start + size, 0);
        r.read_exact(&mut out[start..])?;
        let mut crlf = [0u8; 2];
        r.read_exact(&mut crlf)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_parse() {
        assert_eq!(Endpoint::parse("unix:///run/p.sock").unwrap(), Endpoint::Unix("/run/p.sock".into()));
        assert_eq!(Endpoint::parse("/run/p.sock").unwrap(), Endpoint::Unix("/run/p.sock".into()));
        assert_eq!(Endpoint::parse("http://127.0.0.1:9377").unwrap(), Endpoint::Tcp("127.0.0.1:9377".into()));
        assert_eq!(Endpoint::parse("http://127.0.0.1:9377/").unwrap(), Endpoint::Tcp("127.0.0.1:9377".into()));
        assert_eq!(Endpoint::parse("http://localhost").unwrap(), Endpoint::Tcp("localhost:80".into()));
        assert!(Endpoint::parse("https://x:1").is_err());
        assert!(Endpoint::parse("http://h:1/path").is_err());
        assert!(Endpoint::parse("http://u@h:1").is_err());
        assert!(Endpoint::parse("unix://").is_err());
    }

    #[test]
    fn answers_are_read_and_bounded() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nX-A: b\r\n\r\n{}";
        let r = read_response(&raw[..], 10, false).unwrap();
        assert_eq!((r.status, r.body.as_slice(), r.header("x-a")), (200, &b"{}"[..], Some("b")));
        let raw = b"HTTP/1.0 404 Not Found\r\n\r\nnope";
        assert_eq!(read_response(&raw[..], 10, false).unwrap().body, b"nope");
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2;x=1\r\nde\r\n0\r\n\r\n";
        assert_eq!(read_response(&raw[..], 10, false).unwrap().body, b"abcde");
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\n\r\n";
        assert!(read_response(&raw[..], 10, false).is_err());
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nff\r\n";
        assert!(read_response(&raw[..], 10, false).is_err());
        assert!(read_response(&b"SSH-2.0\r\n\r\n"[..], 10, false).is_err());
        assert!(read_response(&b""[..], 10, false).is_err());
    }
}
