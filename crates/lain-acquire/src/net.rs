//! The acquire module's outbound HTTP over the shared `lain-http` client
//! (D-200): indexers, `.torrent` links, subtitle files and HTTP(S)
//! trackers. `lain-http` answers whole bodies, so [`LainHttp`] serves
//! the bounded fetches; the download manager streams plain `http://`
//! through `lain-downloads`' own client and only `https://` through
//! `lain-http`, within [`HTTPS_DOWNLOAD_CAP`] (D-209), via [`Routed`].

use std::io::Cursor;
use std::time::Duration;

use lain_downloads::http::{Http, HttpError, HttpErrorKind, PlainHttp, Request, Response};

/// The largest `https://` download until `lain-http` streams bodies.
pub const HTTPS_DOWNLOAD_CAP: usize = 256 << 20;

/// `lain_downloads::Http` over `lain-http`, with a body cap. Redirects
/// are followed here (up to 5) so a redirect to `magnet:` comes back
/// to the caller as the 3xx answer.
#[derive(Clone)]
pub struct LainHttp {
    pub client: lain_http::Client,
    pub max_body: usize,
}

fn kind(e: &lain_http::Error) -> HttpErrorKind {
    use lain_http::ErrorKind as K;
    match e.kind {
        K::Offline | K::InvalidUrl => HttpErrorKind::Invalid,
        K::Connect | K::Tls => HttpErrorKind::Connect,
        K::Timeout => HttpErrorKind::Timeout,
        K::TooLarge => HttpErrorKind::TooLarge,
        _ => HttpErrorKind::Protocol,
    }
}

impl Http for LainHttp {
    fn send(&self, req: &Request) -> Result<Response, HttpError> {
        let mut url = req.url.clone();
        for _ in 0..=5 {
            if req.cancel.as_ref().is_some_and(|c| c.is_canceled()) {
                return Err(HttpError::new(HttpErrorKind::Canceled, "canceled"));
            }
            let method = lain_http::Method::from_bytes(req.method.as_bytes()).unwrap_or(lain_http::Method::GET);
            let mut r = self.client.request(method, &url).no_redirects().max_body(self.max_body);
            if !req.timeout.is_zero() {
                r = r.timeout(req.timeout.max(Duration::from_secs(1)));
            }
            for (k, v) in &req.headers {
                r = if k.eq_ignore_ascii_case("user-agent") { r.user_agent(v) } else { r.header(k, v) };
            }
            let res = r.send_blocking().map_err(|e| HttpError::new(kind(&e), e.to_string()))?;
            let location = res.header("location").map(str::to_string);
            if req.follow_redirects
                && (300..400).contains(&res.status)
                && let Some(loc) = location
            {
                let base =
                    lain_http::Url::parse(&url).map_err(|e| HttpError::new(HttpErrorKind::Invalid, e.to_string()))?;
                let lower = loc.trim().to_ascii_lowercase();
                let relative = !lower.contains("://") && !lower.starts_with("magnet:");
                if lower.starts_with("http://") || lower.starts_with("https://") || relative {
                    url = base
                        .join(loc.trim())
                        .map_err(|e| HttpError::new(HttpErrorKind::Invalid, e.to_string()))?
                        .to_string();
                    continue;
                }
            }
            return Ok(Response { status: res.status, headers: res.headers, body: Box::new(Cursor::new(res.body)) });
        }
        Err(HttpError::new(HttpErrorKind::Protocol, format!("{}: too many redirects", crate::util::url_host(&req.url))))
    }
}

/// The download manager's client: plain `http://` streams through
/// `lain-downloads`, `https://` goes through `lain-http` (capped).
pub struct Routed {
    pub plain: PlainHttp,
    pub tls: LainHttp,
    /// Refuse everything (tests and air-gapped servers), like `lain-http`.
    pub offline: bool,
}

impl Http for Routed {
    fn send(&self, req: &Request) -> Result<Response, HttpError> {
        if self.offline {
            return Err(HttpError::new(
                HttpErrorKind::Invalid,
                format!("{}: offline", crate::util::url_host(&req.url)),
            ));
        }
        if req.url.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("https://")) {
            self.tls.send(req)
        } else {
            self.plain.send(req)
        }
    }
}

/// HTTP(S) tracker announces through `lain-http` (blocking: the engine
/// calls it on its blocking pool).
pub struct TrackerHttp(pub lain_http::Client);

impl lain_torrent::HttpGet for TrackerHttp {
    fn get(&self, url: &str, timeout: Duration) -> Result<(u16, Vec<u8>), String> {
        self.0
            .get(url)
            .timeout(timeout.max(Duration::from_secs(1)))
            .max_body(1 << 20)
            .send_blocking()
            .map(|r| (r.status, r.body))
            .map_err(|e| e.to_string())
    }
}
