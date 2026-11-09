//! The metadata module (D-155): `lain-metadata-enrich` serves
//! `lain.metadata.enrich@1` (search fan-out, pick, resolve, overlay and
//! caches) over the merge-many `lain.metadata.search@1` and
//! `lain.metadata.resolve@1` providers; `lain-metadata-nfo` is the
//! offline one. Every provider call goes through the registry, so
//! swapping or withdrawing providers changes enrichment. The network
//! providers (Kitsu, AniList, Jikan, TVMaze) need HTTPS and stay
//! components (`lain plugin-run`) until the Rust server has TLS.
//! A port of `internal/plugins/metadata`.

pub mod enrich;
pub mod merge;
pub mod nfo;
pub mod store;
pub mod types;

pub use enrich::{CAP_METADATA_ENRICH, ENRICH_ID, Enricher};
pub use nfo::{CAP_METADATA_RESOLVE, CAP_METADATA_SEARCH, NFO_ID, Nfo};
pub use types::{MetadataCandidate, MetadataRecord, MetadataResolveInput, MetadataSearchInput};

/// The default precedence of the merge-many metadata bindings: local
/// first, then the remote providers Go ships.
pub const DEFAULT_PROVIDERS: &[&str] =
    &[NFO_ID, "lain-metadata-kitsu", "lain-metadata-anilist", "lain-metadata-jikan", "lain-metadata-tvmaze"];
