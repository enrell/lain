//! Typed acquisition errors: messages are written for people, codes are
//! stable (the HTTP layer maps them to statuses).

use std::fmt;

pub const CODE_INVALID: &str = "invalid-request";
pub const CODE_NOT_FOUND: &str = "not-found";
pub const CODE_STATE: &str = "invalid-state";
pub const CODE_QUOTA: &str = "quota-exceeded";
pub const CODE_DISK_FULL: &str = "disk-full";
pub const CODE_UNSUPPORTED: &str = "unsupported-protocol";
pub const CODE_FETCH: &str = "fetch-failed";
pub const CODE_IMPORT: &str = "import-failed";
pub const CODE_NO_LIBRARY: &str = "no-library";
pub const CODE_STALLED: &str = "stalled";
/// A subtitle failed the sync check (A-33).
pub const CODE_SUBTITLE_MISMATCH: &str = "subtitle-mismatch";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcqError {
    pub code: String,
    pub msg: String,
}

impl AcqError {
    pub fn new(code: &str, msg: impl Into<String>) -> AcqError {
        AcqError { code: code.into(), msg: msg.into() }
    }
    /// The HTTP status of the code (Go `writeAcqErr`): an indexer or
    /// tracker failure is a bad gateway.
    pub fn status(&self) -> u16 {
        match self.code.as_str() {
            CODE_INVALID | CODE_NO_LIBRARY => 400,
            CODE_NOT_FOUND => 404,
            CODE_STATE => 409,
            CODE_QUOTA | CODE_DISK_FULL => 507,
            CODE_UNSUPPORTED | CODE_SUBTITLE_MISMATCH => 422,
            CODE_IMPORT => 500,
            _ => 502,
        }
    }
}

impl fmt::Display for AcqError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.msg)
    }
}

impl std::error::Error for AcqError {}

impl From<lain_store::StoreError> for AcqError {
    fn from(e: lain_store::StoreError) -> AcqError {
        AcqError::new("store-error", e.to_string())
    }
}

impl From<lain_core::Error> for AcqError {
    fn from(e: lain_core::Error) -> AcqError {
        AcqError { code: e.code.clone(), msg: e.msg.clone() }
    }
}

pub type Result<T> = std::result::Result<T, AcqError>;

/// `AcqError::new` with `format!` arguments.
#[macro_export]
macro_rules! errf {
    ($code:expr, $($arg:tt)*) => {
        $crate::error::AcqError::new($code, format!($($arg)*))
    };
}
