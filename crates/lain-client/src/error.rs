//! Typed client failures. Every server answer maps to one variant by
//! status and stable `code`, so a client never matches on message text.

use lain_api::{ApiError, CODE_MODULE_DISABLED, CODE_NOT_PORTED};

/// A `lain-client` failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError {
    /// The server could not be reached or the connection broke: the
    /// offline case. Queued work stays queued.
    Offline { message: String },
    /// No answer within the configured time.
    Timeout,
    /// 401: no token, or the token stopped verifying (expired, password
    /// rotated, account disabled). Log in again.
    Unauthorized,
    /// Login refused: `invalid credentials` or `account disabled`.
    LoginFailed { message: String },
    /// 403: authenticated but not allowed (admin required, wrong PIN).
    Forbidden { message: String },
    /// 404: unknown item, library, user or route. Items of a locked
    /// private library read as unknown too (D-156).
    NotFound { message: String },
    /// 404 `module-disabled`: the module owning the route is off (D-154).
    ModuleDisabled { module: String },
    /// 501 `not-ported`: the route exists in the API but this server
    /// (lain-rs) does not serve it yet.
    NotPorted { route: String },
    /// 429: too many attempts (library unlock).
    RateLimited { message: String },
    /// Any other refusal: 400 validation errors, 409 `stale-generation`,
    /// 422, and 5xx.
    Api { status: u16, code: String, message: String },
    /// The answer did not decode as the expected type.
    Decode { message: String },
    /// Bad input caught before any request.
    Invalid { message: String },
    /// Not supported by this build.
    Unsupported { message: String },
    /// The TLS handshake with an `https` server failed: a certificate
    /// that is not trusted, expired or for another name, or a server
    /// that does not speak TLS on that port.
    Tls { message: String },
    /// The local store (tokens, offline copies, the progress queue).
    Store { message: String },
    /// The offline store has no room: `code` is `quota-exceeded` or
    /// `disk-full`, like D-094's CLI store.
    NoRoom { code: String, message: String },
}

impl ClientError {
    /// Whether the failure means "not reachable right now", the cue to
    /// queue work and retry later.
    pub fn is_offline(&self) -> bool {
        matches!(self, ClientError::Offline { .. } | ClientError::Timeout | ClientError::Tls { .. })
    }

    /// Maps a non-2xx answer.
    pub(crate) fn from_response(status: u16, route: &str, body: &[u8]) -> ClientError {
        let e: ApiError = serde_json::from_slice(body).unwrap_or_else(|_| ApiError {
            error: String::from_utf8_lossy(body).trim().to_string(),
            ..ApiError::default()
        });
        match (status, e.code.as_str()) {
            (501, CODE_NOT_PORTED) => ClientError::NotPorted { route: route.to_string() },
            (404, CODE_MODULE_DISABLED) => ClientError::ModuleDisabled { module: e.module },
            (401, _) => ClientError::Unauthorized,
            (403, _) => ClientError::Forbidden { message: e.error },
            (404, _) => ClientError::NotFound { message: e.error },
            (429, _) => ClientError::RateLimited { message: e.error },
            _ => ClientError::Api { status, code: e.code, message: e.error },
        }
    }
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Offline { message } => write!(f, "server unreachable: {message}"),
            ClientError::Timeout => f.write_str("the server did not answer in time"),
            ClientError::Unauthorized => f.write_str("not logged in, or the session ended"),
            ClientError::LoginFailed { message } => write!(f, "login failed: {message}"),
            ClientError::Forbidden { message } => write!(f, "forbidden: {message}"),
            ClientError::NotFound { message } => write!(f, "not found: {message}"),
            ClientError::ModuleDisabled { module } => write!(f, "module {module} is disabled"),
            ClientError::NotPorted { route } => write!(f, "{route} is not served by this server yet"),
            ClientError::RateLimited { message } => write!(f, "rate limited: {message}"),
            ClientError::Api { status, code, message } if code.is_empty() => write!(f, "{status}: {message}"),
            ClientError::Api { status, code, message } => write!(f, "{status} {code}: {message}"),
            ClientError::Decode { message } => write!(f, "unexpected answer: {message}"),
            ClientError::Invalid { message } => write!(f, "invalid input: {message}"),
            ClientError::Unsupported { message } => write!(f, "unsupported: {message}"),
            ClientError::Tls { message } => write!(f, "no secure connection: {message}"),
            ClientError::Store { message } => write!(f, "local store: {message}"),
            ClientError::NoRoom { message, .. } => f.write_str(message),
        }
    }
}

impl std::error::Error for ClientError {}

pub(crate) fn store_err(e: impl std::fmt::Display) -> ClientError {
    ClientError::Store { message: e.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_map_by_status_and_code() {
        let r = |s, b: &str| ClientError::from_response(s, "GET /x", b.as_bytes());
        assert_eq!(
            r(501, r#"{"error":"not served by lain-rs yet","code":"not-ported"}"#),
            ClientError::NotPorted { route: "GET /x".into() }
        );
        assert_eq!(
            r(404, r#"{"error":"module watch is disabled","code":"module-disabled","module":"watch"}"#),
            ClientError::ModuleDisabled { module: "watch".into() }
        );
        assert_eq!(r(404, r#"{"error":"unknown item"}"#), ClientError::NotFound { message: "unknown item".into() });
        assert_eq!(r(401, "garbage"), ClientError::Unauthorized);
        assert_eq!(
            r(409, r#"{"error":"stale","code":"stale-generation","generation":4}"#),
            ClientError::Api { status: 409, code: "stale-generation".into(), message: "stale".into() }
        );
        assert_eq!(
            r(502, "bad gateway\n"),
            ClientError::Api { status: 502, code: "".into(), message: "bad gateway".into() }
        );
        assert!(ClientError::Timeout.is_offline() && !ClientError::Unauthorized.is_offline());
    }
}
