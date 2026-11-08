//! Windows stand-ins: Rust's standard library has no Unix sockets
//! there yet, so component mode reports itself unavailable instead of
//! failing the build or the boot.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use lain_core::{Error, Registry};

use crate::manifest::Manifest;

#[derive(Debug, Clone, Copy, Default)]
pub struct Timeouts {
    pub invoke: Duration,
    pub health: Duration,
    pub handshake: Duration,
    pub backoff_start: Duration,
    pub backoff_max: Duration,
}

pub struct Process;

impl Process {
    pub fn spawn(_m: Manifest, _dir: &Path) -> Result<Process, Error> {
        Err(Error::unavailable("component mode needs Unix sockets; not available on this platform yet"))
    }
}

pub struct Provisioner;

impl Provisioner {
    pub fn new(_dir: &Path, _reg: Arc<Registry>) -> Self {
        Provisioner
    }
    pub fn start(&self) -> Result<(), String> {
        Err("component mode needs Unix sockets; not available on this platform yet".into())
    }
    pub fn installed(&self) -> Vec<String> {
        Vec::new()
    }
    pub fn on_change(&self, _f: impl Fn(&crate::Change) + Send + Sync + 'static) {}
    pub fn close(&self) {}
}
