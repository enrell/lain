//! The lists module's wire (D-078..D-083): tracking-list entries and
//! linked external accounts. Go: `internal/contracts/list.go` and the
//! `linkView` of `internal/gateway/list.go`.

use serde::{Deserialize, Serialize};

use crate::{is_false, is_zero_i64};

fn is_zero_f64(v: &f64) -> bool {
    *v == 0.0
}

/// Normalized tracking statuses shared by every platform.
pub mod list_status {
    pub const CURRENT: &str = "current";
    pub const PLANNING: &str = "planning";
    pub const COMPLETED: &str = "completed";
    pub const PAUSED: &str = "paused";
    pub const DROPPED: &str = "dropped";
    pub const REPEATING: &str = "repeating";
}

/// Broad media kinds a list entry carries.
pub mod list_media {
    pub const ANIME: &str = "anime";
    pub const MANGA: &str = "manga";
    pub const MOVIE: &str = "movie";
    pub const SERIES: &str = "series";
    pub const COMIC: &str = "comic";
}

/// One tracked entry on a user's list. Entries are platform-owned
/// imports (D-081): a sync replaces a platform's set wholesale.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ListEntry {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub remote_id: String,
    #[serde(default)]
    pub media_type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub format: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cover: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub progress: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub progress_total: i64,
    /// The secondary count manga platforms report next to chapters.
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub progress_volumes: i64,
    #[serde(default, skip_serializing_if = "is_zero_f64")]
    pub score: f64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub repeat: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub started_at: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub completed_at: String,
    #[serde(default)]
    pub updated_at: i64,
}

/// `GET /api/list`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ListEntries {
    pub entries: Vec<ListEntry>,
}

/// A linked account as a client sees it: identity and sync state,
/// never the token (D-079).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub platform: String,
    #[serde(default)]
    pub remote_user_id: String,
    #[serde(default)]
    pub remote_username: String,
    #[serde(default)]
    pub linked_at: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub last_sync_at: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_sync_error: String,
    #[serde(default)]
    pub entry_count: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub token_expires_at: i64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub token_expired: bool,
    #[serde(default)]
    pub scrobble: bool,
}

/// `GET /api/me/links`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Links {
    pub links: Vec<Link>,
}

/// `POST /api/me/links/{platform}/code`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkReply {
    pub link: Link,
}

/// `GET /api/me/links/{platform}/authorize` and `/pin`: the URL the
/// client opens.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkUrl {
    pub url: String,
}

/// `PATCH /api/me/links/{platform}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scrobble: Option<bool>,
}

/// One platform import: entries written and entries removed because the
/// remote no longer lists them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListSyncStats {
    #[serde(default)]
    pub upserted: i64,
    #[serde(default)]
    pub removed: i64,
}

/// `POST /api/me/links/{platform}/sync`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkSyncReply {
    pub status: String,
    pub stats: ListSyncStats,
}

/// `DELETE /api/me/links/{platform}`: the platform-owned entries it
/// removed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnlinkReply {
    pub status: String,
    #[serde(default)]
    pub removed: i64,
}

/// `list-changed` on `/api/events`, to the account's own streams: a
/// link was made, updated, synced (`upserted`/`removed`), failed to sync
/// (`error`, a stable code), was unlinked (`removed`) or scrobbled an
/// entry (`remote_id`, `progress`, `status`). Clients re-read
/// `/api/me/links` and `/api/list`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListChangedEvent {
    pub platform: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub upserted: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub removed: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub remote_id: String,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub progress: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub status: String,
}
