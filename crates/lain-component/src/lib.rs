//! Component mode (D-074, D-077):
//! a [`lain_core::Provider`] backed by a supervised child process that
//! speaks the lain component protocol over a Unix socket (a named pipe on Windows), the server
//! side for components written in Rust ([`serve`]), and the
//! [`Provisioner`] that installs, reloads and removes components from
//! manifests dropped into `<data-dir>/plugins/`.
//!
//! The wire is the language-neutral component protocol frozen in
//! `conformance/component/` (see [`protocol`]), so a component may be
//! written in any language. Payloads are bounded JSON; media bytes
//! never cross it.
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
