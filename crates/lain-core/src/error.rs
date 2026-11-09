use std::fmt;

/// Stable wire codes. They have registry meaning (docs/PLUGIN.md); a
/// new one is documented before it is used.
pub mod codes {
    pub const INVALID_MESSAGE: &str = "invalid-message";
    pub const NOT_FOUND: &str = "not-found";
    pub const DEPENDENCY_UNAVAILABLE: &str = "dependency-unavailable";
    pub const STALE_GENERATION: &str = "stale-generation";
    pub const OUTCOME_UNKNOWN: &str = "outcome-unknown";
    pub const INTERNAL: &str = "internal";
    /// A capability or route that belongs to a disabled module (D-154).
    pub const MODULE_DISABLED: &str = "module-disabled";
}

/// A typed error with a stable wire code, the Rust form of `core.Error`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub code: String,
    pub msg: String,
}

impl Error {
    pub fn new(code: &str, msg: impl Into<String>) -> Self {
        Error { code: code.to_string(), msg: msg.into() }
    }
    pub fn invalid(msg: impl Into<String>) -> Self {
        Self::new(codes::INVALID_MESSAGE, msg)
    }
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::new(codes::NOT_FOUND, msg)
    }
    pub fn unavailable(msg: impl Into<String>) -> Self {
        Self::new(codes::DEPENDENCY_UNAVAILABLE, msg)
    }
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::new(codes::INTERNAL, msg)
    }
    pub fn is(&self, code: &str) -> bool {
        self.code == code
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.msg)
    }
}

impl std::error::Error for Error {}
