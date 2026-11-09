//! The conformance runner: a JSON matcher, a dependency-free HTTP/1.1
//! client (with an SSE reader), the case-file interpreter and the
//! component fixture loader, and the upstream fixture server that
//! stands in for the remote APIs of the network providers. The cases themselves are data in
//! `conformance/cases/` so any language can run them; this crate is
//! one runner.

pub mod cases;
pub mod fixtures;
pub mod http;
pub mod matcher;
pub mod upstream;
