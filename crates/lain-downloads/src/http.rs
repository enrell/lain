//! The outbound HTTP seam shared by downloads and the acquire module
//! (indexers, `.torrent` and subtitle fetches). [`PlainHttp`] is an
//! in-tree HTTP/1.1 client for `http://` origins; the server injects a
//! TLS-capable [`Http`] for `https://`. Error messages name the host
//! only: request URLs may carry API keys.

use std::fmt;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// A cancel flag shared between a caller and a transfer.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Cancel {
        Cancel::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_canceled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// One request.
#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// Connect timeout and the longest a read may stall.
    pub timeout: Duration,
    /// Follow up to 5 redirects to http(s) URLs; a redirect to another
    /// scheme (a `magnet:` link) is answered to the caller as is.
    pub follow_redirects: bool,
    /// Checked while connecting and between reads; a canceled request
    /// fails with [`HttpErrorKind::Canceled`].
    pub cancel: Option<Cancel>,
}

impl Request {
    pub fn get(url: &str) -> Request {
        Request {
            method: "GET".into(),
            url: url.into(),
            headers: Vec::new(),
            timeout: Duration::from_secs(30),
            follow_redirects: true,
            cancel: None,
        }
    }
    pub fn header(mut self, name: &str, value: &str) -> Request {
        self.headers.push((name.into(), value.into()));
        self
    }
}

/// A response with a streamed body.
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Box<dyn Read + Send>,
}

impl fmt::Debug for Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Response").field("status", &self.status).field("headers", &self.headers).finish()
    }
}

impl Response {
    /// A header value, case-insensitive (`None` when absent).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// The declared body length; -1 when chunked or unstated.
    pub fn content_length(&self) -> i64 {
        if self.header("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked")) {
            return -1;
        }
        self.header("content-length").and_then(|v| v.trim().parse::<i64>().ok()).filter(|n| *n >= 0).unwrap_or(-1)
    }

    /// The whole body, refused past `max` bytes.
    pub fn read_all(mut self, max: usize) -> Result<Vec<u8>, HttpError> {
        let mut out = Vec::new();
        let n = (&mut self.body).take(max as u64 + 1).read_to_end(&mut out).map_err(|e| HttpError::from_io("", &e))?;
        if n > max {
            return Err(HttpError::new(HttpErrorKind::TooLarge, "response too large"));
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpErrorKind {
    /// Bad URL or unsupported scheme.
    Invalid,
    Connect,
    Timeout,
    Canceled,
    /// A malformed or truncated answer.
    Protocol,
    TooLarge,
}

/// A transport failure. The message names the host, never the URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpError {
    pub kind: HttpErrorKind,
    pub msg: String,
}

impl HttpError {
    pub fn new(kind: HttpErrorKind, msg: impl Into<String>) -> HttpError {
        HttpError { kind, msg: msg.into() }
    }

    fn from_io(host: &str, e: &io::Error) -> HttpError {
        let (kind, what) = match e.kind() {
            io::ErrorKind::Interrupted => (HttpErrorKind::Canceled, "canceled".to_string()),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => (HttpErrorKind::Timeout, "timed out".to_string()),
            io::ErrorKind::UnexpectedEof => (HttpErrorKind::Protocol, "unexpected EOF".to_string()),
            _ => (HttpErrorKind::Protocol, e.to_string()),
        };
        HttpError::new(kind, if host.is_empty() { what } else { format!("{host}: {what}") })
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.msg)
    }
}

impl std::error::Error for HttpError {}

/// The HTTP client seam.
pub trait Http: Send + Sync {
    fn send(&self, req: &Request) -> Result<Response, HttpError>;
}

/// A parsed absolute URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    /// Lower-cased.
    pub scheme: String,
    /// Without brackets for IPv6.
    pub host: String,
    pub port: u16,
    /// `/path?query`, never empty; the fragment is dropped.
    pub path_and_query: String,
}

impl Url {
    /// `host[:port]` as the `Host` header wants it (default ports left out).
    pub fn authority(&self) -> String {
        let h = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        match (self.scheme.as_str(), self.port) {
            ("http", 80) | ("https", 443) => h,
            _ => format!("{h}:{}", self.port),
        }
    }

    pub fn to_url_string(&self) -> String {
        format!("{}://{}{}", self.scheme, self.authority(), self.path_and_query)
    }
}

/// The scheme of an absolute reference (`http`, `magnet`, ...), lower-cased.
pub fn scheme_of(s: &str) -> Option<String> {
    let i = s.find(':')?;
    let sch = &s[..i];
    let mut cs = sch.chars();
    let first = cs.next()?;
    (first.is_ascii_alphabetic() && cs.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')))
        .then(|| sch.to_ascii_lowercase())
}

/// Parses an absolute `scheme://host[:port][/path][?query]` URL.
pub fn parse_url(raw: &str) -> Option<Url> {
    let raw = raw.trim();
    let scheme = scheme_of(raw)?;
    let rest = raw[scheme.len() + 1..].strip_prefix("//")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let auth = &rest[..end];
    let mut tail = &rest[end..];
    if let Some(i) = tail.find('#') {
        tail = &tail[..i];
    }
    let hostport = auth.rsplit('@').next().unwrap_or(auth);
    let (host, port) = if let Some(v6) = hostport.strip_prefix('[') {
        let close = v6.find(']')?;
        let after = &v6[close + 1..];
        let port = match after.strip_prefix(':') {
            Some(p) if !p.is_empty() => Some(p.parse::<u16>().ok()?),
            Some(_) | None if after.is_empty() || after == ":" => None,
            _ => return None,
        };
        (v6[..close].to_string(), port)
    } else {
        match hostport.rsplit_once(':') {
            Some((h, p)) if !p.is_empty() => (h.to_string(), Some(p.parse::<u16>().ok()?)),
            Some((h, _)) => (h.to_string(), None),
            None => (hostport.to_string(), None),
        }
    };
    if host.is_empty() || host.contains(|c: char| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let default = match scheme.as_str() {
        "http" => 80,
        "https" => 443,
        _ => 0,
    };
    let path_and_query = if tail.is_empty() || tail.starts_with('?') { format!("/{tail}") } else { tail.to_string() };
    if path_and_query.contains(|c: char| c.is_control() || c == ' ') {
        return None;
    }
    Some(Url { scheme, host, port: port.unwrap_or(default), path_and_query })
}

/// The host of a URL for error messages (`"origin"` when unparsable).
pub fn host_of(raw: &str) -> String {
    parse_url(raw).map(|u| u.authority()).unwrap_or_else(|| "origin".into())
}

/// Resolves a `Location` against the URL that answered it.
fn resolve(base: &Url, loc: &str) -> Option<String> {
    if scheme_of(loc).is_some() {
        return Some(loc.to_string());
    }
    if let Some(rest) = loc.strip_prefix("//") {
        return Some(format!("{}://{rest}", base.scheme));
    }
    let root = format!("{}://{}", base.scheme, base.authority());
    if loc.starts_with('/') {
        return Some(format!("{root}{loc}"));
    }
    let path = base.path_and_query.split('?').next().unwrap_or("/");
    let dir = &path[..path.rfind('/').map(|i| i + 1).unwrap_or(1)];
    Some(format!("{root}{dir}{loc}"))
}

/// An in-tree HTTP/1.1 client for `http://` origins.
#[derive(Debug, Clone)]
pub struct PlainHttp {
    pub user_agent: String,
}

impl Default for PlainHttp {
    fn default() -> Self {
        PlainHttp { user_agent: "Lain".into() }
    }
}

impl PlainHttp {
    pub fn new() -> PlainHttp {
        PlainHttp::default()
    }

    fn once(&self, req: &Request, url: &Url) -> Result<Response, HttpError> {
        let host = url.authority();
        if url.scheme == "https" {
            return Err(HttpError::new(HttpErrorKind::Invalid, format!("{host}: https needs the TLS client")));
        }
        if url.scheme != "http" {
            return Err(HttpError::new(HttpErrorKind::Invalid, format!("{host}: unsupported scheme")));
        }
        let canceled = || req.cancel.as_ref().is_some_and(Cancel::is_canceled);
        let addrs = (url.host.as_str(), url.port)
            .to_socket_addrs()
            .map_err(|_| HttpError::new(HttpErrorKind::Connect, format!("{host}: unknown host")))?;
        let mut stream = None;
        for a in addrs {
            if canceled() {
                return Err(HttpError::new(HttpErrorKind::Canceled, format!("{host}: canceled")));
            }
            if let Ok(s) = TcpStream::connect_timeout(&a, req.timeout) {
                stream = Some(s);
                break;
            }
        }
        let stream = stream.ok_or_else(|| HttpError::new(HttpErrorKind::Connect, format!("{host}: unreachable")))?;
        let _ = stream.set_nodelay(true);
        let tick = if req.cancel.is_some() { Duration::from_millis(100) } else { req.timeout };
        let _ = stream.set_read_timeout(Some(tick.max(Duration::from_millis(1))));
        let _ = stream.set_write_timeout(Some(req.timeout));

        let mut head = format!("{} {} HTTP/1.1\r\nHost: {host}\r\n", req.method, url.path_and_query);
        let has = |n: &str| req.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(n));
        if !has("user-agent") {
            head.push_str(&format!("User-Agent: {}\r\n", self.user_agent));
        }
        head.push_str("Connection: close\r\nAccept-Encoding: identity\r\n");
        for (k, v) in &req.headers {
            if k.contains(['\r', '\n', ':']) || v.contains(['\r', '\n']) {
                return Err(HttpError::new(HttpErrorKind::Invalid, format!("{host}: bad header")));
            }
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        head.push_str("\r\n");
        (&stream).write_all(head.as_bytes()).map_err(|e| HttpError::from_io(&host, &e))?;

        let raw = Patient { stream, cancel: req.cancel.clone(), idle: req.timeout };
        let mut r = BufReader::with_capacity(64 << 10, raw);
        loop {
            let (status, headers) = read_head(&mut r).map_err(|e| HttpError::from_io(&host, &e))?;
            if (100..200).contains(&status) && status != 101 {
                continue;
            }
            let body: Box<dyn Read + Send> = if req.method.eq_ignore_ascii_case("HEAD")
                || status == 204
                || status == 304
                || (100..200).contains(&status)
            {
                Box::new(io::empty())
            } else {
                let resp = Response { status, headers: headers.clone(), body: Box::new(io::empty()) };
                let chunked =
                    resp.header("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
                if chunked {
                    Box::new(Chunked { r, left: 0, done: false })
                } else if let Some(n) = resp.header("content-length") {
                    let n: u64 = n
                        .trim()
                        .parse()
                        .map_err(|_| HttpError::new(HttpErrorKind::Protocol, format!("{host}: bad content-length")))?;
                    Box::new(Sized { r, left: n })
                } else {
                    Box::new(r)
                }
            };
            return Ok(Response { status, headers, body });
        }
    }
}

impl Http for PlainHttp {
    fn send(&self, req: &Request) -> Result<Response, HttpError> {
        let mut url = parse_url(&req.url)
            .ok_or_else(|| HttpError::new(HttpErrorKind::Invalid, "the URL is not an absolute http(s) URL"))?;
        let first = url.host.clone();
        let mut cur = req.clone();
        for _ in 0..=5 {
            let resp = self.once(&cur, &url)?;
            if !cur.follow_redirects || !matches!(resp.status, 301 | 302 | 303 | 307 | 308) {
                return Ok(resp);
            }
            let Some(loc) = resp.header("location").map(str::to_string) else { return Ok(resp) };
            let Some(next) = resolve(&url, loc.trim()) else { return Ok(resp) };
            if !matches!(scheme_of(&next).as_deref(), Some("http" | "https")) {
                return Ok(resp);
            }
            let Some(nu) = parse_url(&next) else { return Ok(resp) };
            if nu.host != first {
                cur.headers
                    .retain(|(k, _)| !k.eq_ignore_ascii_case("authorization") && !k.eq_ignore_ascii_case("cookie"));
            }
            url = nu;
        }
        Err(HttpError::new(HttpErrorKind::Protocol, format!("{}: too many redirects", url.authority())))
    }
}

/// A socket whose reads wait out short timeouts, fail on cancel and give
/// up after `idle` without bytes.
struct Patient {
    stream: TcpStream,
    cancel: Option<Cancel>,
    idle: Duration,
}

impl Read for Patient {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let start = Instant::now();
        loop {
            if self.cancel.as_ref().is_some_and(Cancel::is_canceled) {
                let _ = self.stream.shutdown(std::net::Shutdown::Both);
                return Err(io::Error::new(io::ErrorKind::Interrupted, "canceled"));
            }
            match self.stream.read(buf) {
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                    if start.elapsed() >= self.idle {
                        return Err(io::Error::new(io::ErrorKind::TimedOut, "timed out"));
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                other => return other,
            }
        }
    }
}

const MAX_HEAD: usize = 64 << 10;

fn read_line(r: &mut impl BufRead, budget: &mut usize) -> io::Result<String> {
    let mut line = Vec::new();
    let n = r.take(*budget as u64 + 1).read_until(b'\n', &mut line)?;
    if n == 0 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "unexpected EOF"));
    }
    if n > *budget || !line.ends_with(b"\n") {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "header too long"));
    }
    *budget -= n;
    while matches!(line.last(), Some(b'\n' | b'\r')) {
        line.pop();
    }
    Ok(String::from_utf8_lossy(&line).into_owned())
}

fn read_head(r: &mut impl BufRead) -> io::Result<(u16, Vec<(String, String)>)> {
    let bad = || io::Error::new(io::ErrorKind::InvalidData, "malformed response");
    let mut budget = MAX_HEAD;
    let status_line = read_line(r, &mut budget)?;
    let mut parts = status_line.splitn(3, ' ');
    if !parts.next().is_some_and(|v| v.starts_with("HTTP/1.")) {
        return Err(bad());
    }
    let status: u16 = parts.next().and_then(|s| s.parse().ok()).filter(|s| (100..1000).contains(s)).ok_or_else(bad)?;
    let mut headers = Vec::new();
    loop {
        let line = read_line(r, &mut budget)?;
        if line.is_empty() {
            return Ok((status, headers));
        }
        let (k, v) = line.split_once(':').ok_or_else(bad)?;
        if headers.len() >= 200 {
            return Err(bad());
        }
        headers.push((k.trim().to_string(), v.trim().to_string()));
    }
}

/// A Content-Length body: a short one is an error, not an EOF.
struct Sized<R> {
    r: R,
    left: u64,
}

impl<R: Read> Read for Sized<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.left == 0 || buf.is_empty() {
            return Ok(0);
        }
        let max = buf.len().min(self.left.min(usize::MAX as u64) as usize);
        let n = self.r.read(&mut buf[..max])?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "unexpected EOF"));
        }
        self.left -= n as u64;
        Ok(n)
    }
}

/// A chunked body.
struct Chunked<R> {
    r: R,
    left: u64,
    done: bool,
}

impl<R: BufRead> Read for Chunked<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.done || buf.is_empty() {
            return Ok(0);
        }
        if self.left == 0 {
            let mut budget = 4096;
            let line = read_line(&mut self.r, &mut budget)?;
            let size = line.split(';').next().unwrap_or("").trim();
            let n = u64::from_str_radix(size, 16)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad chunk size"))?;
            if n == 0 {
                let mut budget = MAX_HEAD;
                while !read_line(&mut self.r, &mut budget)?.is_empty() {}
                self.done = true;
                return Ok(0);
            }
            self.left = n;
        }
        let max = buf.len().min(self.left.min(usize::MAX as u64) as usize);
        let n = self.r.read(&mut buf[..max])?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "unexpected EOF"));
        }
        self.left -= n as u64;
        if self.left == 0 {
            let mut budget = 4;
            if !read_line(&mut self.r, &mut budget)?.is_empty() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "bad chunk end"));
            }
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testsrv::{Srv, respond};

    #[test]
    fn urls() {
        let u = parse_url("http://Example.test:8080/a/b?x=1#frag").unwrap();
        assert_eq!(
            (u.scheme.as_str(), u.host.as_str(), u.port, u.path_and_query.as_str()),
            ("http", "Example.test", 8080, "/a/b?x=1")
        );
        assert_eq!(parse_url("HTTPS://h").unwrap().path_and_query, "/");
        assert_eq!(parse_url("http://h?q=1").unwrap().path_and_query, "/?q=1");
        let v6 = parse_url("http://[::1]:9/x").unwrap();
        assert_eq!((v6.host.as_str(), v6.port, v6.authority()), ("::1", 9, "[::1]:9".to_string()));
        for bad in ["", "http://", "/relative", "::bad", "http://h:99999/", "magnet:?xt=urn:btih:x", "http://a b/"] {
            assert!(parse_url(bad).is_none(), "{bad}");
        }
        assert_eq!(scheme_of("magnet:?xt=1").as_deref(), Some("magnet"));
        let base = parse_url("http://h:1/a/b?c").unwrap();
        assert_eq!(resolve(&base, "d").unwrap(), "http://h:1/a/d");
        assert_eq!(resolve(&base, "/d").unwrap(), "http://h:1/d");
        assert_eq!(resolve(&base, "//o/d").unwrap(), "http://o/d");
    }

    #[test]
    fn bodies_and_redirects() {
        let srv = Srv::start(|req, s| match req.path.as_str() {
            "/len" => respond(s, 200, &[("Content-Length", "5")], b"hello"),
            "/chunked" => {
                let _ = s.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3;x=1\r\nabc\r\n2\r\nde\r\n0\r\nT: 1\r\n\r\n");
            }
            "/close" => {
                let _ = s.write_all(b"HTTP/1.1 200 OK\r\n\r\nuntil close");
            }
            "/short" => {
                let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc");
            }
            "/hop" => respond(s, 302, &[("Location", "len")], b""),
            "/magnet" => respond(s, 302, &[("Location", "magnet:?xt=urn:btih:abc")], b""),
            "/loop" => respond(s, 302, &[("Location", "/loop")], b""),
            _ => respond(s, 404, &[], b"no"),
        });
        let h = PlainHttp::new();
        let get = |p: &str| h.send(&Request::get(&format!("{}{p}", srv.url)));
        assert_eq!(get("/len").unwrap().read_all(100).unwrap(), b"hello");
        assert_eq!(get("/chunked").unwrap().read_all(100).unwrap(), b"abcde");
        assert_eq!(get("/close").unwrap().read_all(100).unwrap(), b"until close");
        assert!(get("/short").unwrap().read_all(100).is_err());
        assert_eq!(get("/len").unwrap().read_all(4).unwrap_err().kind, HttpErrorKind::TooLarge);
        let r = get("/hop").unwrap();
        assert_eq!(r.status, 200);
        let m = get("/magnet").unwrap();
        assert_eq!((m.status, m.header("LOCATION")), (302, Some("magnet:?xt=urn:btih:abc")));
        let e = get("/loop").unwrap_err();
        assert!(e.msg.contains("too many redirects") && !e.msg.contains("/loop"), "{e}");
        assert_eq!(get("/nope").unwrap().status, 404);
        let tls = h.send(&Request::get("https://example.test/a?apikey=SECRET")).unwrap_err();
        assert!(tls.msg.contains("https needs the TLS client") && !tls.msg.contains("SECRET"), "{tls}");
    }

    #[test]
    fn cancel_interrupts_a_stalled_body() {
        let srv = Srv::start(|_, s| {
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nab");
            std::thread::sleep(Duration::from_secs(3));
        });
        let c = Cancel::new();
        let mut req = Request::get(&srv.url);
        req.cancel = Some(c.clone());
        let mut r = PlainHttp::new().send(&req).unwrap();
        let mut b = [0u8; 10];
        assert_eq!(r.body.read(&mut b).unwrap(), 2);
        let c2 = c.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            c2.cancel();
        });
        let t = Instant::now();
        assert_eq!(r.body.read(&mut b).unwrap_err().kind(), io::ErrorKind::Interrupted);
        assert!(t.elapsed() < Duration::from_secs(2));
    }
}
