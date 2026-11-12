//! The HTTP transport: hyper's HTTP/1.1 client connection over a tokio
//! TCP stream, one connection per request (a LAN server answers in
//! milliseconds; pooling can come later without an API change).
//!
//! `https` servers are reached through `lain-http`'s rustls setup
//! (D-277): the webpki-roots trust anchors, plus the roots given to
//! [`trust_roots`] for a private CA or a self-signed certificate. A
//! failed handshake is [`ClientError::Tls`].

use std::sync::Mutex;
use std::time::Duration;

use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::header::{HeaderMap, HeaderValue};
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;

use crate::error::ClientError;

/// lain's default port (D-178), used when a URL has no scheme or port.
pub const DEFAULT_PORT: u16 = 9360;

/// A parsed server address.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerUrl {
    pub tls: bool,
    pub host: String,
    pub port: u16,
    /// A path prefix the server is mounted under (no trailing `/`), for
    /// a reverse proxy; empty for the root.
    pub base: String,
}

impl ServerUrl {
    /// Parses `http://host[:port][/base]`, `https://...`, or a bare
    /// `host[:port]`, which means `http` on port 9360. IPv6 hosts go in
    /// brackets (`http://[::1]:9360`).
    pub fn parse(raw: &str) -> Result<ServerUrl, ClientError> {
        let invalid = |m: &str| ClientError::Invalid { message: format!("server url {raw:?}: {m}") };
        let raw = raw.trim();
        let (tls, rest, default_port) = if let Some(r) = raw.strip_prefix("http://") {
            (false, r, 80)
        } else if let Some(r) = raw.strip_prefix("https://") {
            (true, r, 443)
        } else if raw.contains("://") {
            return Err(invalid("only http and https are supported"));
        } else {
            (false, raw, DEFAULT_PORT)
        };
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        };
        if authority.contains('@') {
            return Err(invalid("credentials do not belong in the url"));
        }
        let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
            let (h, after) = v6.split_once(']').ok_or_else(|| invalid("unclosed ["))?;
            match after.strip_prefix(':') {
                Some(p) => (h.to_string(), p.parse().map_err(|_| invalid("bad port"))?),
                None if after.is_empty() => (h.to_string(), default_port),
                None => return Err(invalid("bad host")),
            }
        } else {
            match authority.rsplit_once(':') {
                Some((h, p)) => (h.to_string(), p.parse().map_err(|_| invalid("bad port"))?),
                None => (authority.to_string(), default_port),
            }
        };
        if host.is_empty() || host.chars().any(|c| c.is_whitespace() || c == '?' || c == '#') {
            return Err(invalid("bad host"));
        }
        if path.contains(['?', '#']) {
            return Err(invalid("no query or fragment"));
        }
        Ok(ServerUrl { tls, host, port, base: path.trim_end_matches('/').to_string() })
    }

    /// The canonical form, which also keys local state (tokens, the
    /// progress queue, offline copies) per server.
    pub fn canonical(&self) -> String {
        let scheme = if self.tls { "https" } else { "http" };
        let host = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        format!("{scheme}://{host}:{}{}", self.port, self.base)
    }

    fn authority(&self) -> String {
        let host = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        format!("{host}:{}", self.port)
    }
}

impl std::fmt::Display for ServerUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.canonical())
    }
}

/// Percent-encodes one path segment or query component.
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// A byte stream hyper can drive: TCP, or TLS over it.
pub(crate) trait Io: hyper::rt::Read + hyper::rt::Write + Unpin + Send {}
impl<T: hyper::rt::Read + hyper::rt::Write + Unpin + Send> Io for T {}

/// The longest wait for a TLS handshake.
const TLS_HANDSHAKE: Duration = Duration::from_secs(10);

/// The process-wide TLS setup: the extra roots, and the connector built
/// from them on first use.
struct Trust {
    roots: Vec<Vec<u8>>,
    connector: Option<lain_http::tls::Connector>,
}

static TRUST: Mutex<Trust> = Mutex::new(Trust { roots: Vec::new(), connector: None });

fn tls_error(e: lain_http::Error) -> ClientError {
    match e.kind {
        lain_http::ErrorKind::Timeout => ClientError::Timeout,
        _ => ClientError::Tls { message: e.to_string() },
    }
}

/// Trusts more root certificates (DER, X.509) for `https` servers, on
/// top of the public roots compiled in: a private CA, or a server's
/// self-signed certificate. Process-wide, for every client made before
/// or after; a certificate that does not parse changes nothing.
pub fn trust_roots(der: Vec<Vec<u8>>) -> Result<(), ClientError> {
    let mut t = TRUST.lock().unwrap_or_else(|e| e.into_inner());
    let mut roots = t.roots.clone();
    for d in der {
        if !roots.contains(&d) {
            roots.push(d);
        }
    }
    let connector = lain_http::tls::Connector::new(&roots)
        .map_err(|e| ClientError::Invalid { message: format!("root certificate: {e}") })?;
    *t = Trust { roots, connector: Some(connector) };
    Ok(())
}

fn connector() -> Result<lain_http::tls::Connector, ClientError> {
    let mut t = TRUST.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = &t.connector {
        return Ok(c.clone());
    }
    let c = lain_http::tls::Connector::new(&t.roots).map_err(tls_error)?;
    t.connector = Some(c.clone());
    Ok(c)
}

async fn connect(url: &ServerUrl) -> Result<Box<dyn Io>, ClientError> {
    let tcp = tokio::net::TcpStream::connect((url.host.as_str(), url.port))
        .await
        .map_err(|e| ClientError::Offline { message: format!("{}: {e}", url.authority()) })?;
    let _ = tcp.set_nodelay(true);
    if url.tls {
        let stream = connector()?.connect(&url.host, tcp, TLS_HANDSHAKE).await.map_err(tls_error)?;
        return Ok(Box::new(TokioIo::new(stream)));
    }
    Ok(Box::new(TokioIo::new(tcp)))
}

/// One finished answer.
pub(crate) struct Response {
    pub status: u16,
    pub body: Bytes,
}

/// An answer whose body is still arriving (event streams, downloads).
pub(crate) struct Streaming {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Incoming,
}

fn offline(e: impl std::fmt::Display) -> ClientError {
    ClientError::Offline { message: e.to_string() }
}

/// Sends a request and returns the answer with its headers, its body
/// unread. `path` already carries the query.
pub(crate) async fn open(
    url: &ServerUrl,
    method: Method,
    path: &str,
    headers: &[(&'static str, String)],
    body: Option<(Bytes, &str)>,
) -> Result<Streaming, ClientError> {
    let io = connect(url).await?;
    let (mut sender, conn) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io).await.map_err(offline)?;
    tokio::spawn(async move {
        // Ends when the answer is read or dropped.
        let _ = conn.await;
    });
    let mut req = Request::builder().method(method).uri(format!("{}{path}", url.base));
    let h = req.headers_mut().expect("fresh builder");
    h.insert(hyper::header::HOST, HeaderValue::from_str(&url.authority()).map_err(offline)?);
    for (k, v) in headers {
        let v = HeaderValue::from_str(v).map_err(|e| ClientError::Invalid { message: format!("header {k}: {e}") })?;
        h.insert(*k, v);
    }
    let payload = match body {
        Some((b, content_type)) => {
            h.insert(hyper::header::CONTENT_TYPE, HeaderValue::from_str(content_type).map_err(offline)?);
            b
        }
        None => Bytes::new(),
    };
    let req = req.body(Full::new(payload)).map_err(|e| ClientError::Invalid { message: e.to_string() })?;
    let resp = sender.send_request(req).await.map_err(offline)?;
    let status = resp.status().as_u16();
    let (parts, body) = resp.into_parts();
    Ok(Streaming { status, headers: parts.headers, body })
}

/// Sends a request and reads the whole answer, all within `timeout`.
pub(crate) async fn send(
    url: &ServerUrl,
    method: Method,
    path: &str,
    headers: &[(&'static str, String)],
    body: Option<(Bytes, &str)>,
    timeout: Duration,
) -> Result<Response, ClientError> {
    let work = async {
        let s = open(url, method, path, headers, body).await?;
        let body = s.body.collect().await.map_err(offline)?.to_bytes();
        Ok(Response { status: s.status, body })
    };
    tokio::time::timeout(timeout, work).await.map_err(|_| ClientError::Timeout)?
}

/// The next chunk of a streaming body; `None` at its end. Trailers are
/// skipped.
pub(crate) async fn next_chunk(body: &mut Incoming) -> Result<Option<Bytes>, ClientError> {
    loop {
        match body.frame().await {
            None => return Ok(None),
            Some(Err(e)) => return Err(offline(e)),
            Some(Ok(f)) => {
                if let Ok(data) = f.into_data() {
                    return Ok(Some(data));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_parse() {
        let u = ServerUrl::parse("192.168.0.5").unwrap();
        assert_eq!(u.canonical(), "http://192.168.0.5:9360");
        assert_eq!(ServerUrl::parse("http://lain.lan").unwrap().canonical(), "http://lain.lan:80");
        assert_eq!(ServerUrl::parse("http://h:1234/media/").unwrap().canonical(), "http://h:1234/media");
        let v6 = ServerUrl::parse("http://[::1]:9360").unwrap();
        assert_eq!((v6.host.as_str(), v6.port, v6.canonical()), ("::1", 9360, "http://[::1]:9360".to_string()));
        assert!(ServerUrl::parse("https://h").unwrap().tls);
        for bad in ["ftp://h", "http://", "http://h:x", "http://u:p@h", "http://h/?q", "http://[::1", "h h"] {
            assert!(ServerUrl::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn components_are_encoded() {
        assert_eq!(encode("a b/c?d&e=é"), "a%20b%2Fc%3Fd%26e%3D%C3%A9");
        assert_eq!(encode("item-01.x_~"), "item-01.x_~");
    }
}
