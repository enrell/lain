//! Lain's in-house BitTorrent engine (DESIGN-v2 §13, a port of Go's
//! `internal/torrent`): v1 metainfo and magnets, HTTP and UDP trackers,
//! the peer wire protocol with ut_metadata (BEP 9/10), piece
//! verification, resume and seeding. No DHT, PEX, uTP or encryption.
//!
//! The public API is synchronous: a [`Client`] owns its own tokio
//! runtime and drives peers and trackers as tasks. Methods that wait
//! (`pause`, `remove`, `close`) block the calling thread, so they must
//! not be called from an async task.

pub mod bencode;
mod bitfield;
mod client;
mod error;
mod http;
mod limiter;
mod metainfo;
mod peer;
mod session;
mod storage;
mod sync;
mod tracker;
pub mod trackertest;
mod wire;

pub use bitfield::Bitfield;
pub use client::{Client, Config};
pub use error::Error;
pub use http::{HttpGet, PlainHttp};
pub use limiter::Limiter;
pub use metainfo::{
    File, Info, InfoHash, Magnet, MetaInfo, SourceFile, build, parse_info, parse_magnet, parse_metainfo,
};
pub use session::{Callback, FileInfo, Spec, State, Stats, Torrent};
pub use tracker::{
    AnnounceRequest, AnnounceResponse, EVENT_COMPLETED, EVENT_NONE, EVENT_STARTED, EVENT_STOPPED, announce,
    compact_peers, encode_compact_peers, parse_http_announce,
};

/// A `.torrent` document holding only `info_raw` (how a magnet's
/// fetched info dictionary is saved for resume).
pub fn torrent_file(info_raw: &[u8]) -> Vec<u8> {
    let mut out = b"d4:info".to_vec();
    out.extend_from_slice(info_raw);
    out.push(b'e');
    out
}
