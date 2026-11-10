//! The metadata module (D-155): `lain-metadata-enrich` serves
//! `lain.metadata.enrich@1` (search fan-out, pick, resolve, overlay and
//! caches) over the merge-many `lain.metadata.search@1` and
//! `lain.metadata.resolve@1` providers; `lain-metadata-nfo` is the
//! offline one. Every provider call goes through the registry, so
//! swapping or withdrawing providers changes enrichment. The network
//! providers (Kitsu, AniList, Jikan, TVMaze, [`remote`]) run in-process
//! over the shared `lain-http` client (D-201).
//! A port of `internal/plugins/metadata`.

pub mod enrich;
pub mod merge;
pub mod nfo;
pub mod remote;
pub mod store;
pub mod types;

pub use enrich::{CAP_METADATA_ENRICH, ENRICH_ID, Enricher};
pub use nfo::{CAP_METADATA_RESOLVE, CAP_METADATA_SEARCH, NFO_ID, Nfo};
pub use remote::{ANILIST_ID, Endpoints, JIKAN_ID, KITSU_ID, Remote, TVMAZE_ID};
pub use types::{MetadataCandidate, MetadataRecord, MetadataResolveInput, MetadataSearchInput};

/// The component that imports what a downloader left next to its
/// downloads (`lain-bridge`'s `library::ID`; a component, so this crate
/// only knows its name).
pub const LIBRARY_ID: &str = "lain-library-anything";

/// The providers that read files next to the media and never the
/// network.
pub const LOCAL_PROVIDERS: &[&str] = &[NFO_ID, LIBRARY_ID];

/// The default precedence of the merge-many metadata bindings: local
/// first, then the remote providers Go ships.
pub const DEFAULT_PROVIDERS: &[&str] = &[NFO_ID, LIBRARY_ID, KITSU_ID, ANILIST_ID, JIKAN_ID, TVMAZE_ID];
