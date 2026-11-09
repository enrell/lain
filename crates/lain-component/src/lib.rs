//! Component mode (D-074, D-077), the Rust port of `internal/component`:
//! a [`lain_core::Provider`] backed by a supervised child process that
//! speaks the lain component protocol over a Unix socket (a named pipe on Windows), the server
//! side for components written in Rust ([`serve`]), and the
//! [`Provisioner`] that installs, reloads and removes components from
//! manifests dropped into `<data-dir>/plugins/`.
//!
//! The wire is the Go server's, byte for byte (see [`protocol`] and
//! `conformance/component/`), so a Go `lain plugin-run` component runs
//! under this host unchanged and a Rust component runs under the Go
//! host. Payloads are bounded JSON; media bytes never cross it.
//!
//! The transport is a Unix socket on Unix and a named pipe on Windows
//! (D-174); the frames are the same on both ([`transport`]).

pub mod manifest;

/// What a provisioner reconcile pass did, for callers that publish events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Installed(String),
    Removed(String),
}

pub mod protocol;
pub mod transport;

mod host;
mod provision;
mod serve;

pub use host::{Process, Timeouts};
pub use manifest::Manifest;
pub use provision::Provisioner;
pub use serve::serve;
