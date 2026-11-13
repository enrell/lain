//! The acquire module (Go `internal/acquire`, `internal/plugins/indexer`):
//! indexers through `lain.indexer@1` (the Torznab provider is in-tree),
//! grabs on the in-house torrent engine, import into libraries, the
//! seeding policy, Phase 2 automation (profiles, monitored titles, RSS,
//! searches, upgrades, the blocklist) and Phase 3 subtitles through
//! `lain.subtitle@1`. It moves bytes and owns threads, so it is a core
//! crate beside `lain-downloads`, not a provider; parsing, indexers,
//! the probe and subtitle providers are capabilities it calls through
//! injected functions.

pub mod decide;
pub mod error;
pub mod importer;
pub mod monitored;
pub mod util;

pub use error::{AcqError, Result};
pub mod aggregators;
pub mod automation;
pub mod client;
pub mod manager;
pub mod net;
pub mod store;
pub mod subtitles;
pub mod torznab;
mod xml;

#[cfg(test)]
mod testsupport;

pub use manager::{Deps, GrabRequest, Manager, SearchQuery, default_settings, validate_settings};
pub use torznab::{TORZNAB_ID, Torznab};
#[cfg(test)]
mod subtitle_tests;
#[cfg(test)]
mod world_tests;
