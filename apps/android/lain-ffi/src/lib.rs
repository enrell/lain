//! `lain-ffi`: the Android app's bindings over `lain-client` (DESIGN-v2
//! §8.5, D-245). UniFFI generates the Kotlin side from these exports.
//!
//! The surface is deliberately thin: one object, [`LainCore`], owns the
//! client database, the session store, a small tokio runtime and the
//! current server's [`lain_client::Client`], [`lain_client::ProgressSync`]
//! and [`lain_client::OfflineStore`]. Every call takes and returns owned
//! records ([`types`]); async methods become Kotlin `suspend` functions,
//! and cancelling the coroutine aborts the Rust task. Errors keep the
//! client's stable variants ([`LainError`]).
//!
//! What lives here rather than in Kotlin is logic two clients would
//! otherwise write twice: the play-source order (offline copy, then the
//! server's plan, D-246), the offline reading of CBZ copies (D-248), the
//! catalog and art snapshot for offline browsing, and the
//! `lain.ui.theme@1` parser (D-249).

uniffi::setup_scaffolding!();

mod cache;
mod core;
mod error;
mod play;
mod read;
mod task;
pub mod theme;
pub mod types;

pub use crate::core::LainCore;
pub use error::LainError;
pub use types::*;

/// This library's version (the app shows it next to the server's).
#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
