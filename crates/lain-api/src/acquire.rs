//! `/api/acquire/*`: the acquire module's records (Go `internal/acquire`
//! and `internal/contracts/indexer.go`). Credentials (`api_key`) live on
//! the stored records and travel in contract input; every response
//! clears them and reports `has_api_key` instead.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Release;

fn is_false(v: &bool) -> bool {
    !*v
}

fn is_zero_i64(v: &i64) -> bool {
    *v == 0
}

// ---- lain.indexer@1 ---------------------------------------------------------

pub const CAP_INDEXER: &str = "lain.indexer@1";
pub const INDEXER_TORZNAB: &str = "torznab";
pub const INDEXER_NEWZNAB: &str = "newznab";
/// Search kinds map onto Newznab functions: tv → `t=tvsearch`, movie →
/// `t=movie`, book → `t=book`, anything else → `t=search`.
pub const SEARCH_GENERIC: &str = "";
pub const SEARCH_TV: &str = "tv";
pub const SEARCH_MOVIE: &str = "movie";
pub const SEARCH_BOOK: &str = "book";
pub const PROTOCOL_TORRENT: &str = "torrent";
pub const PROTOCOL_USENET: &str = "usenet";

/// One configured indexer.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Indexer {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key: String,
    #[serde(default)]
    pub categories: Vec<i64>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub priority: i64,
    /// The minimum time between two requests (A-8).
    #[serde(default)]
    pub min_interval_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caps: Option<IndexerCaps>,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub last_check_at: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_error: String,
    #[serde(default)]
    pub created_at: i64,
}

impl Indexer {
    /// The client-safe view.
    pub fn public(&self) -> Indexer {
        Indexer { api_key: String::new(), ..self.clone() }
    }
}

impl std::fmt::Debug for Indexer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Indexer")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("protocol", &self.protocol)
            .field("has_api_key", &!self.api_key.is_empty())
            .finish_non_exhaustive()
    }
}

/// What a `t=caps` probe reported.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexerCaps {
    #[serde(default)]
    pub search: bool,
    #[serde(default)]
    pub tv_search: bool,
    #[serde(default)]
    pub movie_search: bool,
    #[serde(default)]
    pub book_search: bool,
    #[serde(default)]
    pub categories: Vec<IndexerCategory>,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub limit: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexerCategory {
    pub id: i64,
    pub name: String,
}

/// `caps` op input: probes an indexer (the health check).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexerCapsInput {
    pub indexer: Indexer,
}

/// `search` op input: one query against one indexer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexerSearchInput {
    pub indexer: Indexer,
    #[serde(default)]
    pub query: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub season: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub episode: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub year: i64,
    /// Overrides the indexer's own list when non-empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<i64>,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub limit: i64,
}

/// One release an indexer offered.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResult {
    #[serde(default)]
    pub indexer_id: String,
    #[serde(default)]
    pub protocol: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub guid: String,
    /// The `.torrent` or `.nzb` download URL.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub link: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub magnet: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub info_hash: String,
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub seeders: i64,
    #[serde(default)]
    pub peers: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub published_at: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<i64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comments_url: String,
    /// A download volume factor of 0.
    #[serde(default, skip_serializing_if = "is_false")]
    pub freeleech: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexerSearchOutput {
    pub results: Vec<SearchResult>,
}

// ---- settings and indexers --------------------------------------------------

pub const IMPORT_HARDLINK: &str = "hardlink";
pub const IMPORT_COPY: &str = "copy";
pub const IMPORT_MOVE: &str = "move";

/// The operator's acquisition policy (A-5). Disk limits are the
/// download manager's, shared (A-4).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AcquireSettings {
    #[serde(default)]
    pub dir: String,
    #[serde(default)]
    pub listen_port: i64,
    #[serde(default)]
    pub max_active: i64,
    #[serde(default)]
    pub max_peers: i64,
    #[serde(default)]
    pub upload_kbps: i64,
    #[serde(default)]
    pub download_kbps: i64,
    #[serde(default)]
    pub seed_ratio: f64,
    #[serde(default)]
    pub seed_minutes: i64,
    #[serde(default)]
    pub remove_after_seeding: bool,
    #[serde(default)]
    pub import_mode: String,
    #[serde(default)]
    pub automation: bool,
    #[serde(default)]
    pub rss_minutes: i64,
    #[serde(default)]
    pub search_hours: i64,
    #[serde(default)]
    pub stall_hours: i64,
    #[serde(default)]
    pub subtitle_hours: i64,
}

/// Acquisition's footprint against the shared limits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcquireUsage {
    /// Torrent data on disk.
    pub used_bytes: i64,
    /// The HTTP download manager's share.
    pub downloads_bytes: i64,
    pub max_bytes: i64,
    pub min_free_bytes: i64,
}

/// `GET`/`PUT /api/acquire/settings`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AcquireView {
    pub settings: AcquireSettings,
    pub usage: AcquireUsage,
    pub port: u16,
    /// Whether the lain-parser model answers.
    pub parser: bool,
}

/// Creates or patches an indexer; absent fields stay. An empty
/// `api_key` keeps the stored key, `"-"` clears it.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexerInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<i64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_interval_ms: Option<i64>,
}

impl std::fmt::Debug for IndexerInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexerInput").field("name", &self.name).field("url", &self.url).finish_non_exhaustive()
    }
}

/// An indexer as clients see it: no key, `has_api_key` instead.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexerView {
    #[serde(flatten)]
    pub indexer: Indexer,
    pub has_api_key: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexerList {
    pub indexers: Vec<IndexerView>,
}

/// `POST /api/acquire/indexers/{id}/test`: 200 either way.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexerTest {
    pub indexer: IndexerView,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
}

// ---- search -----------------------------------------------------------------

/// A search result with its parse, rank and the reasons it does not fit
/// (rejected results stay visible for manual grabs).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    #[serde(flatten)]
    pub result: SearchResult,
    pub release: Release,
    pub score: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejections: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexerFailure {
    pub indexer_id: String,
    pub name: String,
    pub error: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResults {
    pub candidates: Vec<Candidate>,
    pub failures: Vec<IndexerFailure>,
}

// ---- grabs ------------------------------------------------------------------

pub const GRAB_QUEUED: &str = "queued";
pub const GRAB_METADATA: &str = "metadata";
pub const GRAB_DOWNLOADING: &str = "downloading";
pub const GRAB_IMPORTING: &str = "importing";
pub const GRAB_SEEDING: &str = "seeding";
pub const GRAB_DONE: &str = "done";
pub const GRAB_PAUSED: &str = "paused";
pub const GRAB_FAILED: &str = "failed";

/// `POST /api/acquire/grabs`: a search result, a `.torrent` URL or a
/// magnet, into a library.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrabInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<SearchResult>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub magnet: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default)]
    pub library_id: String,
}

/// One wanted thing: an episode (season 0 = absolute), a chapter or
/// volume (season 0) or a movie (`{0,1}`). Number 0 with a season is a
/// whole-season pack.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Unit {
    pub season: i64,
    pub number: i64,
}

/// One release taken from an indexer (or a pasted magnet).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grab {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub indexer_id: String,
    /// `torrent-url` or `magnet`.
    pub source: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub magnet: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub info_hash: String,
    pub release: Release,
    pub library_id: String,
    /// The grab's own folder for torrent data.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub dir: String,
    /// The library type.
    pub kind: String,
    pub state: String,
    pub size: i64,
    pub downloaded: i64,
    pub uploaded: i64,
    pub completed: i64,
    pub peers: i64,
    pub down_rate: i64,
    pub up_rate: i64,
    /// Library paths the import created.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imported: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub code: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
    /// Download complete.
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub finished_at: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub seeding_at: i64,
    /// The torrent copy is gone from `dir`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub data_removed: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub monitored_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub units: Vec<Unit>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub upgrade: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replace_from: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replaced: Vec<String>,
}

impl Grab {
    /// Whether the torrent data counts against the budget (A-4).
    pub fn on_disk(&self) -> bool {
        !self.data_removed && self.state != GRAB_FAILED
    }
}

/// `GET /api/acquire/grabs`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrabList {
    pub grabs: Vec<Grab>,
    pub usage: AcquireUsage,
}

// ---- profiles and monitored titles -------------------------------------------

pub const HI_INCLUDE: &str = "include";
pub const HI_PREFER: &str = "prefer";
pub const HI_EXCLUDE: &str = "exclude";

/// A quality policy (A-19).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Allowed resolutions, best first.
    #[serde(default)]
    pub resolutions: Vec<String>,
    /// `bluray`, `web`, `hdtv`, `dvd`; empty allows any.
    #[serde(default)]
    pub sources: Vec<String>,
    /// The resolution at which upgrades stop.
    #[serde(default)]
    pub cutoff: String,
    #[serde(default)]
    pub preferred_groups: Vec<String>,
    #[serde(default)]
    pub blocked_groups: Vec<String>,
    #[serde(default)]
    pub blocked_words: Vec<String>,
    #[serde(default)]
    pub min_seeders: i64,
    /// Per unit (episode, chapter or movie); 0 disables a bound.
    #[serde(default)]
    pub min_size_mb: i64,
    #[serde(default)]
    pub max_size_mb: i64,
    #[serde(default)]
    pub prefer_proper: bool,
    /// Wanted subtitle languages (ISO 639-2/T, ordered).
    #[serde(default)]
    pub subtitle_languages: Vec<String>,
    #[serde(default)]
    pub subtitle_even_with_audio: bool,
    /// `include` (default), `prefer` or `exclude`.
    #[serde(default)]
    pub subtitle_hi: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileList {
    pub profiles: Vec<Profile>,
}

pub const NUMBER_ABSOLUTE: &str = "absolute";
pub const NUMBER_SEASONAL: &str = "seasonal";
pub const NUMBER_CHAPTER: &str = "chapter";
pub const NUMBER_VOLUME: &str = "volume";
pub const NUMBER_MOVIE: &str = "movie";

/// Season `season` begins at absolute episode `first` (A-24).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeasonStart {
    pub season: i64,
    pub first: i64,
}

/// One monitored season: episodes `from..=to` (`to` 0 = ongoing).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeasonWant {
    pub season: i64,
    #[serde(default)]
    pub from: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub to: i64,
}

/// A title automation keeps complete (A-17).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Monitored {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub library_id: String,
    /// The library type.
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub year: i64,
    #[serde(default)]
    pub profile_id: String,
    #[serde(default)]
    pub numbering: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub season_map: Vec<SeasonStart>,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub from: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub to: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub seasons: Vec<SeasonWant>,
    /// The provider's episode total for an absolute title (A-18).
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub metadata_episodes: i64,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub last_search_at: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub last_subtitle_at: i64,
    #[serde(default)]
    pub created_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitoredList {
    pub monitored: Vec<Monitored>,
}

/// A present file below the profile cutoff.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpgradeUnit {
    pub unit: Unit,
    pub quality: String,
    pub path: String,
}

/// What a monitored title still needs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wanted {
    pub missing: Vec<Unit>,
    /// Ongoing absolute/chapter/volume titles: every number from here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_from: Option<Unit>,
    /// Ongoing seasons: episodes from this number on.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub open_seasons: BTreeMap<i64, i64>,
    pub upgrades: Vec<UpgradeUnit>,
    pub present: i64,
}

/// One monitored title in the Wanted overview.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WantedRow {
    #[serde(flatten)]
    pub monitored: Monitored,
    pub wanted: Wanted,
    pub missing_count: i64,
    pub upgrade_count: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WantedOverview {
    pub titles: Vec<WantedRow>,
    pub automation: AutomationState,
}

/// What one automation pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub grabbed: Vec<String>,
    pub considered: i64,
    pub rejected: i64,
    /// Set when the budget stopped the pass (A-25).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub paused: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<IndexerFailure>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationState {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub paused: String,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub last_rss_at: i64,
}

/// A release automation must not grab again (A-22).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockEntry {
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub info_hash: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub indexer_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub monitored_id: String,
    pub reason: String,
    pub at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blocklist {
    pub blocklist: Vec<BlockEntry>,
}

/// A library file an upgrade (or a sidecar removal) moved to the
/// holding folder. Never deleted automatically.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeldFile {
    pub grab_id: String,
    pub path: String,
    pub size: i64,
    pub at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeldFiles {
    pub files: Vec<HeldFile>,
}

/// `{"removed": true}`, `{"deleted": n}` and `{"cleared": n}` answers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Removed {
    pub removed: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deleted {
    pub deleted: i64,
}

// ---- subtitles (Phase 3) ------------------------------------------------------

/// The `lain.subtitle@1` contract types (served by the subtitles module).
pub use crate::{
    SUBTITLE_OPENSUBTITLES, SubtitleCandidate, SubtitleDownloadInput, SubtitleDownloadOutput, SubtitleProvider,
    SubtitleSearchInput, SubtitleSearchOutput,
};

pub const CAP_SUBTITLE: &str = "lain.subtitle@1";

/// Creates or patches an account; absent fields stay. An empty secret
/// keeps the stored one, `"-"` clears it.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtitleProviderInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
}

impl std::fmt::Debug for SubtitleProviderInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubtitleProviderInput").field("name", &self.name).finish_non_exhaustive()
    }
}

/// An account as clients see it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtitleProviderView {
    #[serde(flatten)]
    pub provider: SubtitleProvider,
    pub has_api_key: bool,
    pub has_password: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtitleProviderList {
    pub providers: Vec<SubtitleProviderView>,
}

/// A candidate with its verdict (rejected ones show their reasons).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtitleChoice {
    #[serde(flatten)]
    pub candidate: SubtitleCandidate,
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejections: Vec<String>,
    pub score: i64,
}

/// `GET /api/acquire/items/{id}/subtitles`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtitleSearch {
    pub choices: Vec<SubtitleChoice>,
    pub languages: Vec<String>,
}

/// `POST /api/acquire/items/{id}/subtitles`: a candidate to place.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtitleTake {
    #[serde(flatten)]
    pub candidate: SubtitleCandidate,
    #[serde(default)]
    pub replace: bool,
}

/// A sidecar lain wrote (the A-35 ledger).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtitleRecord {
    pub path: String,
    pub media_path: String,
    pub provider_id: String,
    pub file_id: String,
    pub language: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub hash_match: bool,
    pub at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtitleLedger {
    pub subtitles: Vec<SubtitleRecord>,
}

/// A provider file the sync check refused (A-40).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtitleRefusal {
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub file_id: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub media_path: String,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtitleRefusals {
    pub refusals: Vec<SubtitleRefusal>,
}

/// One video file and its subtitle state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSubtitles {
    pub item_id: String,
    pub path: String,
    pub missing: Vec<String>,
    pub sidecars: Vec<SidecarFile>,
}

/// A sidecar as the acquire routes list it (Go `subtitle.Sidecar`: no
/// index, unlike the player's `Sidecar`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidecarFile {
    pub name: String,
    pub language: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tag: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub forced: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub hi: bool,
    pub format: String,
}

/// `GET /api/acquire/monitored/{id}/subtitles`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitoredSubtitles {
    pub files: Vec<FileSubtitles>,
}

/// `GET /api/acquire/items/{id}/subtitles/status`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemSubtitleStatus {
    pub file: FileSubtitles,
    pub languages: Vec<String>,
}
