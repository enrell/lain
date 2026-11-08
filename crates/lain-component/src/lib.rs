//! Component mode (D-074, D-077), the Rust port of `internal/component`:
//! a [`lain_core::Provider`] backed by a supervised child process that
//! speaks the lain component protocol over a Unix socket, the server
//! side for components written in Rust ([`serve`]), and the
//! [`Provisioner`] that installs, reloads and removes components from
//! manifests dropped into `<data-dir>/plugins/`.
//!
//! The wire is the Go server's, byte for byte (see [`protocol`] and
//! `conformance/component/`), so a Go `lain plugin-run` component runs
//! under this host unchanged and a Rust component runs under the Go
//! host. Payloads are bounded JSON; media bytes never cross it.
//!
//! Unix only for now: Rust's standard library has no `AF_UNIX` on
//! Windows. On Windows, spawning answers `dependency-unavailable` and
//! the provisioner logs that component mode is unavailable.

pub mod manifest;

/// What a provisioner reconcile pass did, for callers that publish events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Installed(String),
    Removed(String),
}

pub mod protocol;

#[cfg(unix)]
mod host;
#[cfg(unix)]
mod provision;
#[cfg(unix)]
mod serve;

pub use manifest::Manifest;
#[cfg(unix)]
pub use host::{Process, Timeouts};
#[cfg(unix)]
pub use provision::Provisioner;
#[cfg(unix)]
pub use serve::serve;

#[cfg(not(unix))]
mod unsupported;
#[cfg(not(unix))]
pub use unsupported::{Process, Provisioner, Timeouts};
