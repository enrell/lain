//! The one error type of the bindings. It mirrors [`ClientError`]
//! variant for variant (Kotlin matches on the class, never on text) and
//! adds the cases only the bindings meet. Fields are `detail`, not
//! `message`, so the Kotlin exception keeps `Throwable.message`.

use lain_client::ClientError;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum LainError {
    /// The server could not be reached: queued work stays queued.
    Offline {
        detail: String,
    },
    Timeout,
    /// No session, or it stopped verifying: log in again.
    Unauthorized,
    LoginFailed {
        detail: String,
    },
    Forbidden {
        detail: String,
    },
    NotFound {
        detail: String,
    },
    /// The module that owns the route is off (D-154).
    ModuleDisabled {
        module: String,
    },
    /// The route exists in the API but this server does not serve it.
    NotPorted {
        route: String,
    },
    RateLimited {
        detail: String,
    },
    Api {
        status: u16,
        code: String,
        detail: String,
    },
    Decode {
        detail: String,
    },
    Invalid {
        detail: String,
    },
    Unsupported {
        detail: String,
    },
    /// An `https` server whose TLS handshake failed (a certificate that
    /// is not trusted, expired or for another name).
    Tls {
        detail: String,
    },
    Store {
        detail: String,
    },
    /// The offline store has no room (`quota-exceeded`, `disk-full`).
    NoRoom {
        code: String,
        detail: String,
    },
    /// No server chosen yet ([`crate::LainCore::set_server`]).
    NoServer,
    /// The server says the item cannot be played (the plan's reason).
    Unplayable {
        detail: String,
    },
    /// A bug or a runtime failure inside the bindings.
    Internal {
        detail: String,
    },
}

impl LainError {
    /// Whether the failure means "not reachable right now", the same
    /// answer as [`ClientError::is_offline`]: a failed TLS handshake
    /// counts too (D-277).
    pub fn is_offline(&self) -> bool {
        matches!(self, LainError::Offline { .. } | LainError::Timeout | LainError::Tls { .. })
    }
}

impl std::fmt::Display for LainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LainError::Offline { detail } => write!(f, "server unreachable: {detail}"),
            LainError::Timeout => f.write_str("the server did not answer in time"),
            LainError::Unauthorized => f.write_str("not logged in"),
            LainError::LoginFailed { detail }
            | LainError::Forbidden { detail }
            | LainError::NotFound { detail }
            | LainError::RateLimited { detail }
            | LainError::Decode { detail }
            | LainError::Invalid { detail }
            | LainError::Unsupported { detail }
            | LainError::Tls { detail }
            | LainError::Store { detail }
            | LainError::Unplayable { detail }
            | LainError::Internal { detail } => f.write_str(detail),
            LainError::ModuleDisabled { module } => write!(f, "the {module} module is disabled"),
            LainError::NotPorted { route } => write!(f, "{route} is not served by this server"),
            LainError::Api { status, code, detail } => write!(f, "{status} {code}: {detail}"),
            LainError::NoRoom { code, detail } => write!(f, "{code}: {detail}"),
            LainError::NoServer => f.write_str("no server chosen"),
        }
    }
}

impl std::error::Error for LainError {}

impl From<ClientError> for LainError {
    fn from(e: ClientError) -> Self {
        match e {
            ClientError::Offline { message } => LainError::Offline { detail: message },
            ClientError::Timeout => LainError::Timeout,
            ClientError::Unauthorized => LainError::Unauthorized,
            ClientError::LoginFailed { message } => LainError::LoginFailed { detail: message },
            ClientError::Forbidden { message } => LainError::Forbidden { detail: message },
            ClientError::NotFound { message } => LainError::NotFound { detail: message },
            ClientError::ModuleDisabled { module } => LainError::ModuleDisabled { module },
            ClientError::NotPorted { route } => LainError::NotPorted { route },
            ClientError::RateLimited { message } => LainError::RateLimited { detail: message },
            ClientError::Api { status, code, message } => LainError::Api { status, code, detail: message },
            ClientError::Decode { message } => LainError::Decode { detail: message },
            ClientError::Invalid { message } => LainError::Invalid { detail: message },
            ClientError::Unsupported { message } => LainError::Unsupported { detail: message },
            ClientError::Tls { message } => LainError::Tls { detail: message },
            ClientError::Store { message } => LainError::Store { detail: message },
            ClientError::NoRoom { code, message } => LainError::NoRoom { code, detail: message },
        }
    }
}

impl From<lain_core::Error> for LainError {
    fn from(e: lain_core::Error) -> Self {
        match e.code.as_str() {
            "not-found" => LainError::NotFound { detail: e.msg },
            "unsupported-format" => LainError::Unsupported { detail: e.msg },
            _ => LainError::Store { detail: e.to_string() },
        }
    }
}

pub(crate) fn io(e: std::io::Error) -> LainError {
    LainError::Store { detail: e.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bindings' error says "unreachable" exactly when the client's
    /// does, a failed TLS handshake included (D-277).
    #[test]
    fn offline_matches_the_client() {
        let m = || "x".to_string();
        for e in [
            ClientError::Offline { message: m() },
            ClientError::Timeout,
            ClientError::Tls { message: m() },
            ClientError::Unauthorized,
            ClientError::Unsupported { message: m() },
            ClientError::Invalid { message: m() },
            ClientError::Api { status: 502, code: m(), message: m() },
        ] {
            assert_eq!(LainError::from(e.clone()).is_offline(), e.is_offline(), "{e:?}");
        }
        assert!(LainError::Tls { detail: m() }.is_offline());
    }
}
