//! `lain-client`: the client core every native lain client shares
//! (DESIGN-v2 §8.1, D-179). It holds the logic, not the UI:
//!
//! - [`Client`]: the typed API for every route a client uses, with the
//!   server's own `lain-api` types, and typed errors ([`ClientError`];
//!   a route lain-rs does not serve yet is [`ClientError::NotPorted`]);
//! - auth: setup, login, logout and `/api/me`, with sessions kept by a
//!   [`TokenStore`] ([`FileTokenStore`], [`MemoryTokenStore`]);
//! - discovery behind [`Discovery`] ([`ManualDiscovery`]);
//! - `https` servers through rustls, with [`trust_roots`] for a private
//!   CA (D-277);
//! - offline copies ([`OfflineStore`]), mirroring D-094's CLI store, on
//!   redb ([`ClientDb`]);
//! - progress sync with an offline queue and explicit conflict rules
//!   ([`ProgressSync`]; the rules are in [`sync`]);
//! - the event stream with reconnect and re-auth ([`EventStream`]).
//!
//! The public surface is owned data, `Arc`-shared objects and object-safe
//! traits (no lifetimes or generics in signatures), so Android can wrap
//! it with UniFFI later (P5). Async methods run on tokio.

pub mod acquire;
pub mod api;
pub mod db;
pub mod discovery;
pub mod error;
pub mod events;
pub mod http;
pub mod offline;
pub mod sync;
pub mod token;

pub use api::{CatalogQuery, Client, ClientConfig, SearchQuery, Unlock};
pub use db::ClientDb;
pub use discovery::{Discovery, DiscoverySource, ManualDiscovery, ServerCandidate};
pub use error::ClientError;
pub use events::{EventStream, ServerEvent, StreamConfig, StreamItem};
pub use http::{DEFAULT_PORT, ServerUrl, trust_roots};
pub use offline::{DiskSpace, FetchObserver, GcReport, OfflineConfig, OfflineEntry, OfflineState, OfflineStore};
pub use sync::{ConflictRule, FlushItem, FlushReport, ProgressSync, QueuedProgress, RecordOutcome, Resolution};
pub use token::{FileTokenStore, MemoryTokenStore, Session, TokenStore};

/// Re-exported wire types, so a client needs only this crate.
pub use lain_api as types;

/// The per-user directory for client state (`sessions.json`,
/// `client.redb`, `offline/`): `$LAIN_CLIENT_HOME`, else
/// `%LOCALAPPDATA%\lain-client` on Windows, `~/Library/Application
/// Support/lain-client` on macOS and `$XDG_DATA_HOME/lain-client` (or
/// `~/.local/share/lain-client`) elsewhere. It is apart from the
/// server's data dir (`lain`, D-164), which a server on the same machine
/// locks. `None` when the OS gives no home (Android: pass paths).
pub fn client_dir() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(d) = env("LAIN_CLIENT_HOME") {
        return Some(d);
    }
    if cfg!(windows) {
        return env("LOCALAPPDATA").map(|d| d.join("lain-client"));
    }
    if cfg!(target_os = "macos") {
        return env("HOME").map(|h| h.join("Library/Application Support/lain-client"));
    }
    env("XDG_DATA_HOME").or_else(|| env("HOME").map(|h| h.join(".local/share"))).map(|d| d.join("lain-client"))
}
