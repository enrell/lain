//! Outbound HTTP for lain (D-200): one shared client that every network
//! provider uses, so politeness and safety live in one place.
//!
//! - HTTPS through rustls with the `ring` provider and the webpki-roots
//!   trust anchors (no OS store, no OS TLS library); plain HTTP for
//!   local endpoints and tests.
//! - hyper's HTTP/1.1 client connection with a small keep-alive pool.
//! - A timeout per attempt, a connect timeout, a response size cap.
//! - Retries with exponential backoff: connection failures, and for
//!   idempotent requests timeouts and 502/503/504; a 429 is retried
//!   only when `Retry-After` names a short wait, as Go's provider
//!   client does.
//! - Per-host pacing (a minimum interval between request starts).
//! - Redirects for GET and HEAD, never from `https` down to `http`;
//!   `Authorization` is dropped when the host changes.
//! - `User-Agent: lain/<version>` unless a request names its own.
//! - Errors name the host only, never a path or query (which may carry
//!   keys).
//!
//! The client owns a one-worker tokio runtime, so it serves blocking
//! callers (registry providers run on blocking threads) through
//! [`Request::send_blocking`] and async callers through
//! [`Request::send`] alike. It holds no server types, so `lain-client`
//! uses its TLS setup ([`tls::Connector`]) for `https` servers (D-274).

mod conn;
mod pace;
pub mod tls;
pub mod url;

use std::sync::Arc;
use std::time::Duration;

use http_body_util::{BodyExt, Full};
pub use hyper::Method;
use hyper::body::Bytes;
use hyper::header::{HeaderName, HeaderValue};
use serde::Serialize;
use serde::de::DeserializeOwned;

pub use url::{Url, parse_query, path_escape, query_escape, query_unescape};

/// Client-wide settings. Each request may override the timeout and the
/// size cap.
#[derive(Debug, Clone)]
pub struct Config {
    /// Sent unless a request sets its own.
    pub user_agent: String,
    pub connect_timeout: Duration,
    /// Per attempt: connect, request, headers and the whole body.
    pub timeout: Duration,
    /// Response body cap in bytes.
    pub max_body: usize,
    /// Extra attempts after the first.
    pub retries: u32,
    /// First backoff; it doubles per retry.
    pub backoff: Duration,
    /// The longest `Retry-After` honored; longer ones fail at once.
    pub max_retry_after: Duration,
    pub max_redirects: u32,
    /// Refuse every request without touching the network (tests and
    /// air-gapped servers).
    pub offline: bool,
    /// Extra trust anchors (DER certificates) on top of webpki-roots.
    pub extra_roots: Vec<Vec<u8>>,
}

/// `lain/<version>` for this crate's (the workspace's) version.
pub const USER_AGENT: &str = concat!("lain/", env!("CARGO_PKG_VERSION"));

impl Default for Config {
    fn default() -> Self {
        Config {
            user_agent: USER_AGENT.into(),
            connect_timeout: Duration::from_secs(10),
            timeout: Duration::from_secs(15),
            max_body: 8 << 20,
            retries: 2,
            backoff: Duration::from_millis(250),
            max_retry_after: Duration::from_secs(30),
            max_redirects: 5,
            offline: false,
            extra_roots: Vec::new(),
        }
    }
}

/// What went wrong, for callers that map errors to their own codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Offline,
    InvalidUrl,
    Connect,
    Tls,
    Timeout,
    Protocol,
    TooLarge,
    /// A non-2xx answer, from [`Response::ok`].
    Status(u16),
    Decode,
}

/// An outbound call's failure. `Display` is `<host>: <message>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    pub host: String,
    pub message: String,
}

impl Error {
    fn new(kind: ErrorKind, host: &str, message: impl Into<String>) -> Error {
        Error { kind, host: host.to_string(), message: message.into() }
    }

    fn invalid_url(m: &str) -> Error {
        Error::new(ErrorKind::InvalidUrl, "", format!("invalid url: {m}"))
    }

    /// The HTTP status of a [`ErrorKind::Status`] error.
    pub fn status(&self) -> Option<u16> {
        match self.kind {
            ErrorKind::Status(s) => Some(s),
            _ => None,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.host.is_empty() { f.write_str(&self.message) } else { write!(f, "{}: {}", self.host, self.message) }
    }
}

impl std::error::Error for Error {}

struct Shared {
    cfg: Config,
    tls: Arc<rustls::ClientConfig>,
    pool: conn::Pool,
    pacer: pace::Pacer,
}

struct Inner {
    shared: Arc<Shared>,
    handle: tokio::runtime::Handle,
    rt: Option<tokio::runtime::Runtime>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        // Never blocks, so the last handle may drop anywhere, even on
        // an async thread.
        if let Some(rt) = self.rt.take() {
            rt.shutdown_background();
        }
    }
}

/// The shared client. Cheap to clone; clones share the pool, the pacing
/// and the runtime.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client").field("user_agent", &self.inner.shared.cfg.user_agent).finish_non_exhaustive()
    }
}

impl Client {
    pub fn new(cfg: Config) -> Result<Client, Error> {
        let tls = conn::tls_config(&cfg.extra_roots)?;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("lain-http")
            .enable_io()
            .enable_time()
            .build()
            .map_err(|e| Error::new(ErrorKind::Protocol, "", format!("http runtime: {e}")))?;
        let shared = Arc::new(Shared { cfg, tls, pool: conn::Pool::default(), pacer: pace::Pacer::default() });
        Ok(Client { inner: Arc::new(Inner { shared, handle: rt.handle().clone(), rt: Some(rt) }) })
    }

    /// A client that refuses every request (`ErrorKind::Offline`).
    pub fn offline() -> Client {
        Client::new(Config { offline: true, ..Config::default() }).expect("an offline client always builds")
    }

    pub fn config(&self) -> &Config {
        &self.inner.shared.cfg
    }

    /// Paces requests to the host of `url`: at least `gap` between two
    /// request starts (zero removes the limit). Retries are paced too.
    pub fn pace(&self, url: &str, gap: Duration) {
        if let Ok(u) = Url::parse(url) {
            self.inner.shared.pacer.set(&u.host_key(), gap);
        }
    }

    pub fn request(&self, method: Method, url: &str) -> Request {
        let idempotent = method != Method::POST && method != Method::PATCH;
        Request {
            client: self.clone(),
            spec: Spec {
                method,
                url: url.to_string(),
                headers: Vec::new(),
                body: None,
                timeout: None,
                max_body: None,
                idempotent,
                user_agent: None,
                redirects: true,
            },
        }
    }

    pub fn get(&self, url: &str) -> Request {
        self.request(Method::GET, url)
    }

    pub fn post(&self, url: &str) -> Request {
        self.request(Method::POST, url)
    }
}

struct Spec {
    method: Method,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<(Bytes, String)>,
    timeout: Option<Duration>,
    max_body: Option<usize>,
    idempotent: bool,
    user_agent: Option<String>,
    redirects: bool,
}

/// One request being built.
#[must_use]
pub struct Request {
    client: Client,
    spec: Spec,
}

impl Request {
    pub fn header(mut self, name: &str, value: &str) -> Request {
        self.spec.headers.push((name.to_string(), value.to_string()));
        self
    }

    pub fn bearer(self, token: &str) -> Request {
        self.header("Authorization", &format!("Bearer {token}"))
    }

    pub fn user_agent(mut self, ua: &str) -> Request {
        self.spec.user_agent = Some(ua.to_string());
        self
    }

    pub fn body(mut self, bytes: Vec<u8>, content_type: &str) -> Request {
        self.spec.body = Some((Bytes::from(bytes), content_type.to_string()));
        self
    }

    /// A JSON body (`Content-Type: application/json`).
    pub fn json(self, v: &impl Serialize) -> Request {
        let raw = serde_json::to_vec(v).unwrap_or_default();
        self.body(raw, "application/json")
    }

    pub fn timeout(mut self, t: Duration) -> Request {
        self.spec.timeout = Some(t);
        self
    }

    pub fn max_body(mut self, n: usize) -> Request {
        self.spec.max_body = Some(n);
        self
    }

    /// Marks the request safe to repeat (a GraphQL query sent as POST).
    pub fn idempotent(mut self, yes: bool) -> Request {
        self.spec.idempotent = yes;
        self
    }

    pub fn no_redirects(mut self) -> Request {
        self.spec.redirects = false;
        self
    }

    /// Sends from async code; the work runs on the client's runtime.
    pub async fn send(self) -> Result<Response, Error> {
        let shared = self.client.inner.shared.clone();
        let task = self.client.inner.handle.spawn(execute(shared, self.spec));
        task.await.map_err(|e| Error::new(ErrorKind::Protocol, "", format!("request task: {e}")))?
    }

    /// Sends and waits on the calling thread. Never call it from an
    /// async task: it blocks that thread until the answer is in.
    pub fn send_blocking(self) -> Result<Response, Error> {
        let shared = self.client.inner.shared.clone();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        self.client.inner.handle.spawn(async move {
            let _ = tx.send(execute(shared, self.spec).await);
        });
        rx.recv().unwrap_or_else(|_| Err(Error::new(ErrorKind::Protocol, "", "the http runtime stopped")))
    }
}

/// A complete answer.
#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    /// Header names in lower case, in arrival order.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// The host that answered (after redirects).
    pub host: String,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The answer when it is 2xx, else an [`ErrorKind::Status`] error
    /// (`rate limited (429)` or `status <n>`).
    pub fn ok(self) -> Result<Response, Error> {
        if self.is_success() {
            return Ok(self);
        }
        let msg = if self.status == 429 { "rate limited (429)".to_string() } else { format!("status {}", self.status) };
        Err(Error::new(ErrorKind::Status(self.status), &self.host, msg))
    }

    pub fn json<T: DeserializeOwned>(&self) -> Result<T, Error> {
        serde_json::from_slice(&self.body)
            .map_err(|e| Error::new(ErrorKind::Decode, &self.host, format!("response is not the expected JSON: {e}")))
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// `Retry-After` in whole seconds (Go's provider client reads only
/// that form); zero or unreadable means none.
pub fn parse_retry_after(v: Option<&str>) -> Option<Duration> {
    let secs: u64 = v?.trim().parse().ok()?;
    (secs > 0).then(|| Duration::from_secs(secs))
}

fn retryable_status(s: u16) -> bool {
    matches!(s, 502..=504)
}

async fn execute(shared: Arc<Shared>, spec: Spec) -> Result<Response, Error> {
    let cfg = &shared.cfg;
    if cfg.offline {
        let host = Url::parse(&spec.url).map(|u| u.host).unwrap_or_default();
        return Err(Error::new(ErrorKind::Offline, &host, "outbound HTTP is off"));
    }
    let mut url = Url::parse(&spec.url)?;
    let mut strip_auth = false;
    let (mut attempt, mut redirects) = (0u32, 0u32);
    let timeout = spec.timeout.unwrap_or(cfg.timeout);
    loop {
        let wait = shared.pacer.reserve(&url.host_key());
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        let result = match tokio::time::timeout(timeout, once(&shared, &url, &spec, strip_auth)).await {
            Ok(r) => r,
            Err(_) => Err(Error::new(ErrorKind::Timeout, &url.host, "timed out")),
        };
        let backoff = cfg.backoff.saturating_mul(1 << attempt.min(16));
        match result {
            Ok(resp) => {
                let redirect = matches!(resp.status, 301 | 302 | 303 | 307 | 308)
                    && spec.redirects
                    && (spec.method == Method::GET || spec.method == Method::HEAD);
                if let Some(loc) = resp.header("location").filter(|_| redirect) {
                    if redirects >= cfg.max_redirects {
                        return Err(Error::new(ErrorKind::Protocol, &url.host, "too many redirects"));
                    }
                    let next = url.join(loc)?;
                    if url.tls && !next.tls {
                        return Err(Error::new(
                            ErrorKind::Protocol,
                            &url.host,
                            "refused a redirect from https to http",
                        ));
                    }
                    strip_auth |= next.host != url.host;
                    redirects += 1;
                    url = next;
                    continue;
                }
                if resp.status == 429 && attempt < cfg.retries {
                    match parse_retry_after(resp.header("retry-after")) {
                        Some(d) if d <= cfg.max_retry_after => {
                            tokio::time::sleep(d).await;
                            attempt += 1;
                            continue;
                        }
                        _ => return Ok(resp),
                    }
                }
                if retryable_status(resp.status) && spec.idempotent && attempt < cfg.retries {
                    tracing::debug!(host = %url.host, status = resp.status, attempt, "retrying");
                    tokio::time::sleep(backoff).await;
                    attempt += 1;
                    continue;
                }
                return Ok(resp);
            }
            Err(e) => {
                // A connection that never opened never saw the request,
                // so any method may go again.
                let again = match e.kind {
                    ErrorKind::Connect => true,
                    ErrorKind::Timeout | ErrorKind::Protocol => spec.idempotent,
                    _ => false,
                };
                if again && attempt < cfg.retries {
                    tracing::debug!(host = %url.host, err = %e, attempt, "retrying");
                    tokio::time::sleep(backoff).await;
                    attempt += 1;
                    continue;
                }
                return Err(e);
            }
        }
    }
}

fn build(url: &Url, spec: &Spec, ua: &str, strip_auth: bool) -> Result<hyper::Request<Full<Bytes>>, Error> {
    let bad = |m: String| Error::new(ErrorKind::InvalidUrl, &url.host, m);
    let mut req = hyper::Request::builder().method(spec.method.clone()).uri(url.target.as_str());
    let h = req.headers_mut().ok_or_else(|| bad("bad request target".into()))?;
    h.insert(hyper::header::HOST, HeaderValue::from_str(&url.authority()).map_err(|e| bad(e.to_string()))?);
    h.insert(hyper::header::USER_AGENT, HeaderValue::from_str(ua).map_err(|e| bad(format!("user agent: {e}")))?);
    for (k, v) in &spec.headers {
        if strip_auth && k.eq_ignore_ascii_case("authorization") {
            continue;
        }
        let name = HeaderName::from_bytes(k.as_bytes()).map_err(|e| bad(format!("header {k}: {e}")))?;
        let value = HeaderValue::from_str(v).map_err(|_| bad(format!("header {k}: invalid value")))?;
        h.insert(name, value);
    }
    let body = match &spec.body {
        Some((b, ct)) => {
            h.insert(hyper::header::CONTENT_TYPE, HeaderValue::from_str(ct).map_err(|e| bad(e.to_string()))?);
            b.clone()
        }
        None => Bytes::new(),
    };
    req.body(Full::new(body)).map_err(|e| bad(e.to_string()))
}

/// One attempt: a pooled or new connection, the request, the whole
/// body under the size cap.
async fn once(shared: &Shared, url: &Url, spec: &Spec, strip_auth: bool) -> Result<Response, Error> {
    let cfg = &shared.cfg;
    let ua = spec.user_agent.as_deref().unwrap_or(&cfg.user_agent);
    let proto = |e: hyper::Error| Error::new(ErrorKind::Protocol, &url.host, e.to_string());
    let mut sender = None;
    if let Some(mut s) = shared.pool.take(url)
        && s.ready().await.is_ok()
    {
        sender = Some(s);
    }
    let reused = sender.is_some();
    let mut sender = match sender {
        Some(s) => s,
        None => conn::open(url, &shared.tls, cfg.connect_timeout).await?,
    };
    let resp = match sender.send_request(build(url, spec, ua, strip_auth)?).await {
        Ok(r) => r,
        // A pooled connection the server closed meanwhile: once more on
        // a fresh one.
        Err(_) if reused => {
            sender = conn::open(url, &shared.tls, cfg.connect_timeout).await?;
            sender.send_request(build(url, spec, ua, strip_auth)?).await.map_err(proto)?
        }
        Err(e) => return Err(proto(e)),
    };
    let status = resp.status().as_u16();
    let headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned()))
        .collect();
    let cap = spec.max_body.unwrap_or(cfg.max_body);
    let too_large = || Error::new(ErrorKind::TooLarge, &url.host, format!("response over {cap} bytes"));
    let declared = resp.headers().get(hyper::header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()?.parse::<u64>().ok());
    if declared.is_some_and(|n| n > cap as u64) {
        return Err(too_large());
    }
    let mut body = resp.into_body();
    let mut buf = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(proto)?;
        if let Ok(data) = frame.into_data() {
            if buf.len() + data.len() > cap {
                return Err(too_large());
            }
            buf.extend_from_slice(&data);
        }
    }
    shared.pool.put(url, sender);
    Ok(Response { status, headers, body: buf, host: url.host.clone() })
}
