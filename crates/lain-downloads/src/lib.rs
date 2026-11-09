//! Moving bytes from an HTTP origin onto local disk (a port of Go
//! `internal/downloads`): the outbound HTTP seam ([`http`]), a resumable
//! fetcher ([`fetch()`]), the shared byte budget ([`check`], [`full`]),
//! safe file naming and the server download queue ([`Manager`]). It is
//! not a provider: it owns threads and writes media bytes, which
//! providers never do.
//!
//! The wire records are `lain_api::downloads`, re-exported here.

pub mod fetch;
pub mod http;
mod limits;
mod manager;
mod names;

#[cfg(test)]
mod testsrv;

use std::fmt;
use std::time::Duration;

pub use fetch::{FetchRequest, FetchResult, fetch};
pub use http::{Cancel, Http, HttpError, PlainHttp, Request, Response};
pub use lain_api::downloads::{
    CleanupReport, DownloadAdd, DownloadSettings, DownloadUsage, DownloadsView, JOB_CANCELED, JOB_DONE, JOB_FAILED,
    JOB_PAUSED, JOB_QUEUED, JOB_RUNNING, Job, Limits,
};
pub use limits::{FreeSpace, check, check_with, disk_free, full, full_with, human_bytes};
pub use manager::{
    AddInput, DEFAULT_CONCURRENCY, DEFAULT_KEEP_DAYS, DEFAULT_MAX_BYTES, DEFAULT_MAX_RETRIES, DEFAULT_MIN_FREE_BYTES,
    Hook, Manager, default_backoff, default_settings, part_path, validate_settings,
};
pub use names::{free_path, name_from_url, safe_name};

/// Stable error codes (the HTTP API returns them).
pub const CODE_QUOTA: &str = "quota-exceeded";
pub const CODE_DISK_FULL: &str = "disk-full";
pub const CODE_HTTP: &str = "http-error";
pub const CODE_IO: &str = "io-error";
pub const CODE_INVALID: &str = "invalid-request";
pub const CODE_NOT_FOUND: &str = "not-found";
pub const CODE_STATE: &str = "invalid-state";
/// A fetch stopped by its [`Cancel`]; never stored on a job.
pub const CODE_CANCELED: &str = "canceled";

/// A typed failure with a stable code. `retry` marks a failure worth
/// trying again (network trouble, 5xx, 408/429, an interrupted body);
/// `retry_after` is the origin's own Retry-After hint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub code: String,
    pub msg: String,
    pub retry: bool,
    pub retry_after: Duration,
}

impl Error {
    pub fn new(code: &str, msg: impl Into<String>) -> Error {
        Error { code: code.into(), msg: msg.into(), retry: false, retry_after: Duration::ZERO }
    }

    /// A retryable transfer failure.
    pub fn transient(msg: impl Into<String>) -> Error {
        Error { retry: true, ..Error::new(CODE_HTTP, msg) }
    }

    pub fn canceled() -> Error {
        Error::new(CODE_CANCELED, "context canceled")
    }

    pub fn is_canceled(&self) -> bool {
        self.code == CODE_CANCELED
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.msg)
    }
}

impl std::error::Error for Error {}

/// The code of an optional error (`""` for none), like Go's `CodeOf`.
pub fn code_of(e: &Option<Error>) -> &str {
    e.as_ref().map(|e| e.code.as_str()).unwrap_or("")
}

/// Unix seconds of a `SystemTime` (0 before the epoch).
pub(crate) fn unix(t: std::time::SystemTime) -> i64 {
    t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
