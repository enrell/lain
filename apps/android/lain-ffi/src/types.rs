//! The records the bindings hand to Kotlin: owned, flat, and only the
//! fields the app reads. Numbers are `i64`/`f64` (Kotlin `Long`/
//! `Double`), never unsigned, so Kotlin needs no `ULong` arithmetic.

use lain_api::{CatalogItem, Progress, User};
use lain_client::{OfflineEntry, OfflineState, Session};

/// The login on the current server (never the token).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SessionInfo {
    pub server: String,
    pub user_id: String,
    pub username: String,
}

impl From<Session> for SessionInfo {
    fn from(s: Session) -> Self {
        SessionInfo { server: s.server, user_id: s.user_id, username: s.username }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct UserInfo {
    pub id: String,
    pub username: String,
    pub role: String,
    pub display_name: String,
    /// ISO 639-2/T, empty when unset: the default subtitle language.
    pub preferred_language: String,
}

impl From<User> for UserInfo {
    fn from(u: User) -> Self {
        UserInfo {
            id: u.id,
            username: u.username,
            role: u.role,
            display_name: u.profile.display_name,
            preferred_language: u.preferred_language,
        }
    }
}

/// One client destination of a module (`/api/modules`, D-154).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DestinationInfo {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ModuleInfo {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// `disabled`, `unavailable`, `degraded` or `active`.
    pub state: String,
    pub destinations: Vec<DestinationInfo>,
}

/// One catalog item.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Item {
    pub id: String,
    pub library_id: String,
    /// `episode`, `movie`, `comic`, `manga`, ...
    pub kind: String,
    pub title: String,
    pub season: i64,
    pub episode: i64,
    pub year: i64,
    pub size: i64,
    pub updated_at: i64,
    /// The file is gone (D-068): keep it off the shelves.
    pub missing: bool,
    /// The media file's base name (shown on episode rows).
    pub file_name: String,
}

impl From<&CatalogItem> for Item {
    fn from(i: &CatalogItem) -> Self {
        let source = if i.rel_path.is_empty() { &i.file_path } else { &i.rel_path };
        Item {
            id: i.id.clone(),
            library_id: i.library_id.clone(),
            kind: i.kind.clone(),
            title: i.title.clone(),
            season: i.season,
            episode: i.episode,
            year: i.year,
            size: i.size,
            updated_at: i.updated_at,
            missing: i.missing,
            file_name: source.rsplit(['/', '\\']).next().unwrap_or("").to_string(),
        }
    }
}

/// The whole catalog. Offline it is the last copy this device saw.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CatalogSnapshot {
    pub items: Vec<Item>,
    pub from_cache: bool,
    /// When the server answered it (client clock, ms); 0 when unknown.
    pub fetched_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct LibraryInfo {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub private: bool,
    pub locked: bool,
}

/// An item's metadata overlay: what the catalog entry does not carry.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Overlay {
    pub item_id: String,
    pub title: String,
    pub year: i64,
    pub genres: Vec<String>,
    pub synopsis: String,
}

/// One progress record. For reading, `position_sec` is the 1-based
/// page and `duration_sec` the page count.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ProgressInfo {
    pub item_id: String,
    pub position_sec: f64,
    pub duration_sec: f64,
    pub completed: bool,
    pub updated_at: i64,
}

impl From<Progress> for ProgressInfo {
    fn from(p: Progress) -> Self {
        ProgressInfo {
            item_id: p.item_id,
            position_sec: p.position_sec,
            duration_sec: p.duration_sec,
            completed: p.completed,
            updated_at: p.updated_at,
        }
    }
}

/// What saving a position did (the sync rules, `lain-client` R0..R7).
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum SaveOutcome {
    Synced {
        progress: ProgressInfo,
    },
    /// Kept on the device until the server answers again.
    Queued {
        reason: String,
    },
    /// A queued record lost to a newer one on the server.
    KeptServer {
        progress: ProgressInfo,
    },
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FlushSummary {
    pub pushed: i64,
    pub kept_server: i64,
    pub dropped: i64,
    pub remaining: i64,
    /// Why the pass stopped early, if it did.
    pub stopped: Option<String>,
}

/// Where a play comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SourceKind {
    /// A finished offline copy: a local file path.
    Offline,
    /// The original file over `/api/items/{id}/stream`.
    Direct,
    /// An HLS transcode session (`.../transcode/hls/index.m3u8`).
    Hls,
}

/// A text subtitle the player side-loads (WebVTT from the server).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SubtitleTrack {
    pub url: String,
    pub label: String,
    /// ISO 639-2/T or `und`.
    pub language: String,
    /// `sidecar` (a file next to the media) or `embedded` (a stream).
    pub origin: String,
    pub forced: bool,
    /// The source is ASS/SSA: the WebVTT drops its typesetting, so the
    /// player offers burn-in instead (D-247).
    pub typeset: bool,
    /// The embedded stream index (for burn-in), -1 for a sidecar.
    pub stream: i64,
}

/// Everything the player needs to start.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PlaySource {
    pub kind: SourceKind,
    /// A URL (carries the token: never log it) or a local path.
    pub url: String,
    /// A short human note: `offline copy`, `direct play`, `transcode`.
    pub note: String,
    /// Where to resume, in source seconds.
    pub start_sec: f64,
    /// Source seconds at player position 0: an HLS session that starts
    /// at the resume point begins its timeline there.
    pub offset_sec: f64,
    pub duration_sec: Option<f64>,
    pub subtitles: Vec<SubtitleTrack>,
    /// The transcode session, empty for direct and offline plays.
    pub session: String,
}

/// A work kept as a favourite: its kind and title, and the catalog item it
/// points at (empty when that item is no longer visible).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FavoriteInfo {
    pub item_id: String,
    pub kind: String,
    pub title: String,
}

/// One video's length, in seconds.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DurationInfo {
    pub item_id: String,
    pub duration_sec: f64,
}

/// A quality the server can transcode to (the player's quality menu).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct QualityInfo {
    pub name: String,
    pub max_height: i64,
    pub bitrate_kbps: i64,
}

/// One picture of small frames of a video, for the seek bar's preview:
/// tile `i` (row by row, `columns` a row) is the frame near
/// `i * interval_sec`.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct StoryboardSheet {
    pub image: Vec<u8>,
    pub interval_sec: i64,
    pub count: i64,
    pub columns: i64,
    pub tile_width: i64,
    pub tile_height: i64,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PageInfo {
    pub index: i64,
    pub width: i64,
    pub height: i64,
    pub mime: String,
}

/// A comic or manga ready to read.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ComicBook {
    pub item_id: String,
    /// `ltr` or `rtl`.
    pub direction: String,
    pub pages: Vec<PageInfo>,
    /// Read from this device's offline copy.
    pub offline: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CopyState {
    Queued,
    Paused,
    Done,
    Failed,
}

impl From<OfflineState> for CopyState {
    fn from(s: OfflineState) -> Self {
        match s {
            OfflineState::Queued => CopyState::Queued,
            OfflineState::Paused => CopyState::Paused,
            OfflineState::Done => CopyState::Done,
            OfflineState::Failed => CopyState::Failed,
        }
    }
}

/// One offline copy (or one in progress) of the current server.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct OfflineCopy {
    pub item_id: String,
    pub title: String,
    pub kind: String,
    pub season: i64,
    pub episode: i64,
    pub size: i64,
    pub bytes: i64,
    pub state: CopyState,
    pub code: String,
    pub error: String,
    pub watched: bool,
}

impl From<OfflineEntry> for OfflineCopy {
    fn from(e: OfflineEntry) -> Self {
        OfflineCopy {
            item_id: e.item_id,
            title: e.title,
            kind: e.kind,
            season: e.season,
            episode: e.episode,
            size: e.size as i64,
            bytes: e.bytes as i64,
            state: e.state.into(),
            code: e.code,
            error: e.error,
            watched: e.watched,
        }
    }
}

/// The device budget (D-094's limits; the app's Settings change them).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct OfflinePolicy {
    /// Finished copies plus partials; 0 means no cap.
    pub max_bytes: i64,
    /// Free space to keep after a write; 0 disables the floor.
    pub min_free_bytes: i64,
    /// Let the store delete watched, synced copies (LRU) for room.
    pub evict_watched: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct OfflineUsage {
    pub used_bytes: i64,
    pub max_bytes: i64,
    pub min_free_bytes: i64,
    /// Free space where copies live, `None` when unknown.
    pub free_bytes: Option<i64>,
}

/// A download run over the queue.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct DownloadRun {
    pub done: i64,
    pub failed: i64,
    /// Why the run stopped before the queue was empty.
    pub stopped: Option<String>,
}

/// Told about a transfer as it runs (at most every 250 ms).
#[uniffi::export(with_foreign)]
pub trait DownloadObserver: Send + Sync {
    fn progress(&self, item_id: String, done: i64, total: i64);
    /// A copy was removed to make room.
    fn evicted(&self, item_id: String, title: String);
}

/// What the server's event stream tells the app.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum CoreEvent {
    /// (Re)connected; queued progress was pushed.
    Connected {
        pushed: i64,
    },
    /// The connection dropped; the next call retries after the wait.
    Reconnecting {
        attempt: i64,
        delay_ms: i64,
    },
    /// The session was refused: log in again.
    AuthRequired,
    ProgressChanged {
        item_id: String,
    },
    LibraryChanged,
    ModulesChanged,
}
