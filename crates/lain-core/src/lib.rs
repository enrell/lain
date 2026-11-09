//! Lain's trusted base: which provider serves each capability, the
//! generation that fences every swap, recovery (last-good fallback,
//! withdraw) and the module layer above the composition (D-154).
//!
//! It understands providers, contracts and modules. It does not
//! understand anime, seasons or metadata: those are provider policy.
//! This is the Rust port of `internal/core` (D-171).

mod composition;
mod error;
pub mod modules;
mod registry;

pub use composition::{Binding, BindingMode, BindingView, Composition};
pub use error::{Error, codes};
pub use registry::{Deliveries, Event, Outputs, Provider, ProviderError, ProviderInfo, Registry};

/// Seconds since the Unix epoch (the wire's timestamp unit).
pub fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Milliseconds since the Unix epoch.
pub fn now_unix_millis() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}
