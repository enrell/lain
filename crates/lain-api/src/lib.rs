//! The JSON shapes of lain's HTTP API (`/api/*`, D-169), shared by the
//! server crates and `lain-client` (D-179) so the wire is defined once.
//!
//! Field names, `omitempty` behavior and number types follow the Go
//! server's `internal/contracts`, because the web client and
//! lain-desktop read them. Decoding is tolerant: unknown fields are
//! ignored, except on [`Library`] and [`CatalogItem`], which keep them
//! in `extra` so stored records round-trip additive fields.
//!
//! Every type is plain owned data (no lifetimes, no generics), so a
//! foreign-language binding can mirror it field by field.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

mod lists;
mod social;
mod watch;
pub use lists::*;
pub use social::*;
pub use watch::*;
pub mod acquire;
pub mod downloads;

fn is_false(v: &bool) -> bool {
    !*v
}

fn is_zero_i64(v: &i64) -> bool {
    *v == 0
}

fn is_zero_u64(v: &u64) -> bool {
    *v == 0
}

// ---- accounts ---------------------------------------------------------------

/// Per-user playback limits (D-042). Unset means unrestricted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaybackPolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_video_transcode: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_audio_transcode: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_remux: Option<bool>,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub max_bitrate_kbps: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub max_streams: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subtitle_mode: String,
}

/// The avatar shown for an account.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Avatar {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mascot: String,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub version: i64,
}

/// What other people see of an account.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub display_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub bio: String,
    #[serde(default)]
    pub avatar: Avatar,
}

/// An account as `/api/me`, `/api/users` and `/api/setup` answer it.
/// The server also sends `"pass_hash": null`, which this type drops.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
    pub role: String,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub pwd_ver: u64,
    #[serde(default)]
    pub created_at: i64,
    /// D-071's one language, kept for older clients: since D-280 the
    /// subtitle language when one is set, else the audio language.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub preferred_language: String,
    /// D-280: ISO 639-2; absent plays the file's default audio.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub preferred_audio_language: String,
    /// D-280: ISO 639-2; absent when none is preferred.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub preferred_subtitle_language: String,
    /// D-280: `always`, `forced` or `off`; absent is `auto`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub preferred_subtitle_mode: String,
    #[serde(default)]
    pub playback: PlaybackPolicy,
    #[serde(default)]
    pub profile: Profile,
}

impl User {
    /// How this account wants tracks chosen. With none of the D-280
    /// fields (a server before D-280) `preferred_language` is both
    /// languages with subtitles on `auto`, which is D-071.
    pub fn track_preferences(&self) -> TrackPreferences {
        TrackPreferences::read(
            &self.preferred_language,
            &self.preferred_audio_language,
            &self.preferred_subtitle_language,
            &self.preferred_subtitle_mode,
        )
    }
}

/// The subtitle modes of D-280.
pub const SUBTITLE_MODES: [&str; 4] = ["auto", "always", "forced", "off"];

/// How a viewer wants audio and subtitle tracks chosen (D-280): the
/// resolved account values every player applies.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackPreferences {
    /// ISO 639-2; empty plays the file's default audio.
    #[serde(default)]
    pub audio_language: String,
    /// ISO 639-2; empty when none is preferred.
    #[serde(default)]
    pub subtitle_language: String,
    /// `auto` (subtitles when the audio is not in the subtitle language),
    /// `always`, `forced` (forced tracks only) or `off`; empty is `auto`.
    #[serde(default)]
    pub subtitle_mode: String,
}

impl TrackPreferences {
    /// Reads an account's fields (D-280's wire and stored shape): a
    /// record without any of the three split fields is one written
    /// before D-280, and its one language is both; an empty mode is
    /// `auto`.
    pub fn read(language: &str, audio: &str, subtitle: &str, mode: &str) -> Self {
        if audio.is_empty() && subtitle.is_empty() && mode.is_empty() {
            return TrackPreferences::legacy(language);
        }
        TrackPreferences {
            audio_language: audio.to_string(),
            subtitle_language: subtitle.to_string(),
            subtitle_mode: if mode.is_empty() { "auto".into() } else { mode.to_string() },
        }
    }

    /// D-071's single language: audio in it, else subtitles in it.
    pub fn legacy(language: &str) -> Self {
        TrackPreferences {
            audio_language: language.to_string(),
            subtitle_language: language.to_string(),
            subtitle_mode: "auto".into(),
        }
    }

    /// Nothing to apply: the player's own defaults stand.
    pub fn is_empty(&self) -> bool {
        self.audio_language.is_empty()
            && self.subtitle_language.is_empty()
            && matches!(self.subtitle_mode.as_str(), "" | "auto")
    }

    /// The one language an older client understands (D-071): the
    /// subtitle language, the one the viewer reads, else the audio one.
    pub fn legacy_language(&self) -> &str {
        if self.subtitle_language.is_empty() { &self.audio_language } else { &self.subtitle_language }
    }
}

/// `PATCH /api/me/preferences` (D-071, D-280): only the fields present
/// apply. `preferred_language` (an older client) sets both languages;
/// the others then override it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreferencesPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_audio_language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_subtitle_language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_subtitle_mode: Option<String>,
}

impl PreferencesPatch {
    pub fn is_empty(&self) -> bool {
        self.preferred_language.is_none()
            && self.preferred_audio_language.is_none()
            && self.preferred_subtitle_language.is_none()
            && self.preferred_subtitle_mode.is_none()
    }
}

/// `PATCH /api/me/profile`: only the fields present apply; `mascot: ""`
/// clears the avatar.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfilePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mascot: Option<String>,
}

/// `PATCH /api/users/{id}` (admin): any subset, applied in this order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playback: Option<PlaybackPolicy>,
}

/// `GET /health`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Health {
    pub status: String,
    pub version: String,
}

/// `GET /api/setup/status`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupStatus {
    pub setup_required: bool,
}

/// `POST /api/auth/login`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginReply {
    pub token: String,
}

impl std::fmt::Debug for LoginReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LoginReply { token: <redacted> }")
    }
}

/// `{"status": "ok"}` answers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusReply {
    pub status: String,
}

/// `GET|PUT|DELETE /api/me/pin`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinState {
    pub set: bool,
}

/// `GET /api/profile/mascots`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mascots {
    pub mascots: Vec<String>,
}

// ---- libraries and roots ----------------------------------------------------

/// A configured media root.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Library {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub path: String,
    pub source: String,
    pub created_at: i64,
    /// D-156: hidden from everyone who has not unlocked it this session.
    #[serde(default, skip_serializing_if = "is_false")]
    pub private: bool,
    /// The user who made it private; always a member.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub owner: String,
    /// Other users who may unlock it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<String>,
    /// D-157: the named root it lives in, and its path inside that root.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub root: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub root_path: String,
    /// Responses only, on private libraries: whether this session has
    /// not unlocked it. Stored records never carry it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Library {
    pub fn is_member(&self, user_id: &str) -> bool {
        self.owner == user_id || self.members.iter().any(|m| m == user_id)
    }
}

/// `POST /api/libraries` (admin): `path` alone, or `root` + `path`
/// inside it (D-157).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewLibrary {
    pub name: String,
    #[serde(rename = "type", default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub root: String,
}

/// `PATCH /api/libraries/{id}` (admin).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub members: Option<Vec<String>>,
}

/// `DELETE /api/libraries/{id}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryDeleted {
    pub status: String,
    pub items_removed: i64,
}

/// One named root of `GET /api/roots`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootView {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub available: bool,
}

/// `GET /api/roots` (admin): `lain.roots@1` as this machine maps it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootsView {
    pub format: String,
    pub roots: Vec<RootView>,
}

// ---- catalog and progress ---------------------------------------------------

/// One catalog item: canonical identity plus provenance.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CatalogItem {
    pub id: String,
    pub library_id: String,
    pub kind: String,
    pub title: String,
    pub season: i64,
    pub episode: i64,
    pub year: i64,
    #[serde(default)]
    pub file_path: String,
    pub size: i64,
    pub confidence: f64,
    pub origin: String,
    pub provenance: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub missing: bool,
    /// D-157: `root` + `/`-separated `rel_path`. A rooted item stores no
    /// `file_path`: responses and exports fill it with this machine's
    /// resolution of the root.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub root: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rel_path: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// The paged envelope of catalog reads (default limit 50, cap 500).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CatalogPage {
    pub items: Vec<CatalogItem>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

/// `GET /api/catalog/{id}/episodes`: one title's files.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Episodes {
    pub items: Vec<CatalogItem>,
}

/// One user's progress on one item. `updated_at` is the server's clock.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    #[serde(default)]
    pub item_id: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub position_sec: f64,
    #[serde(default)]
    pub duration_sec: f64,
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub updated_at: i64,
}

/// `PUT /api/items/{id}/progress`: the item and user come from the
/// route and the token; the time is the server's. Reading uses the same
/// record with `position_sec` as the 1-based page and `duration_sec` as
/// the page count.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ProgressUpdate {
    pub position_sec: f64,
    pub duration_sec: f64,
    pub completed: bool,
}

// ---- read ---------------------------------------------------------------

/// The metadata an archive declares about itself (`ComicInfo.xml`).
/// Every field is optional; it decorates and never changes identity.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComicInfo {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub series: String,
    /// The issue or chapter, may be `12.5`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub number: String,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub volume: i64,
    /// Issues in the series.
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub count: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub year: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub writer: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub artist: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub publisher: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub language: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub age_rating: String,
    /// `rtl` or `ltr` when the file declares it, else empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub direction: String,
}

/// One page of a comic or manga archive.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComicPage {
    pub index: i64,
    pub mime: String,
    pub size: i64,
    pub width: i64,
    pub height: i64,
}

/// `GET /api/items/{id}/pages` (`lain.comic.pages@1`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComicPages {
    pub kind: String,
    pub format: String,
    /// The archive's own declaration (`info.direction`), else `rtl`
    /// for manga and `ltr` for comics.
    pub direction: String,
    pub pages: Vec<ComicPage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<ComicInfo>,
}

// ---- server administration ------------------------------------------------

/// `GET /api/theme`: the web UI palette (`lain.ui.theme@1`), already
/// semantic `#rrggbb` roles. `source` is `default` or `omarchy`, `mode`
/// `dark` or `light`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Palette {
    pub source: String,
    pub mode: String,
    pub background: String,
    pub surface: String,
    pub surface_hover: String,
    pub surface_active: String,
    pub foreground: String,
    pub muted: String,
    pub line: String,
    pub accent: String,
    pub accent_hover: String,
    pub accent_foreground: String,
    pub danger: String,
    pub danger_foreground: String,
    pub success: String,
    pub warning: String,
}

/// One platform of `GET /api/admin/settings/integrations`. The secret
/// itself is never returned, only whether one is stored.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrationPlatform {
    pub client_id: String,
    pub secret_set: bool,
    /// The OAuth redirect URL to register with the platform.
    pub callback_url: String,
}

/// `GET/PUT /api/admin/settings/integrations` (admin).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrationsView {
    pub platforms: BTreeMap<String, IntegrationPlatform>,
}

/// The `PUT /api/admin/settings/integrations` body: absent fields keep
/// their value, and an empty secret keeps the stored one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrationsUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anilist_client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anilist_client_secret: Option<String>,
}

/// One subdirectory of `GET /api/browse`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowseDir {
    pub name: String,
    pub path: String,
}

/// `GET /api/browse` (admin): the library folder picker.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowseResult {
    pub path: String,
    pub parent: String,
    /// The path was auto-detected (no `path` asked).
    pub detected: bool,
    pub dirs: Vec<BrowseDir>,
}

/// One library root a scan could not read fully.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnreadableRoot {
    pub library_id: String,
    pub name: String,
    pub path: String,
    pub reason: String,
}

/// The counts of a scan (`lain.ingest.scan@1`). `missing` counts files
/// that vanished (marked, never deleted, D-068), `restored` files that
/// came back, `migrated` moves and renames that kept their id (D-019).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScanStats {
    pub libraries: i64,
    pub candidates: i64,
    pub identified: i64,
    pub unidentified: i64,
    pub errors: i64,
    pub pruned: i64,
    pub missing: i64,
    pub restored: i64,
    pub migrated: i64,
    pub enriched: i64,
    pub walk_errors: i64,
    /// The roots behind `walk_errors` and any inaccessible root, in
    /// library order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unreadable: Vec<UnreadableRoot>,
    #[serde(default)]
    pub dirs: i64,
    #[serde(default)]
    pub started_at: i64,
    #[serde(default)]
    pub finished_at: i64,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Where a running scan is (`ScanStatus.progress`). `phase` is the
/// furthest step reached: `walk` (roots are being listed), `identify`
/// (`files_done` of `files`; `files` grows as roots finish their walk,
/// `libraries_walked` of `libraries`), `commit` (`libraries_done` of
/// `libraries`) or `enrich` (`enrich_done` of `enrich_total`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanProgress {
    pub phase: String,
    pub libraries: i64,
    pub libraries_walked: i64,
    pub libraries_done: i64,
    pub files: i64,
    pub files_done: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub enrich_total: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub enrich_done: i64,
}

/// `GET /api/library/scan`: `state` is `idle`, `running`, `done` or
/// `error`; `trigger` is `manual` or `watch`; `progress` is present
/// while the scan runs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScanStatus {
    pub state: String,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub started_at: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub finished_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<ScanStats>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<ScanProgress>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub trigger: String,
}

// ---- metadata and sidecars --------------------------------------------------

/// The metadata overlay of one catalog item (`lain.metadata.enrich@1`).
/// Identity stays in the catalog; this only decorates it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Enrichment {
    pub item_id: String,
    pub provider: String,
    pub remote_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub year: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub synopsis: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub poster: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cover: String,
    pub fetched_at: i64,
}

/// `GET /api/enrichments?ids=`: the overlays that exist, in id order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Enrichments {
    pub items: Vec<Enrichment>,
}

/// One subtitle file next to an item's media file, addressed by
/// `index` (never by path). `language` is ISO 639-2/T, `und` when the
/// name carries none.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sidecar {
    pub index: i64,
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

/// `GET /api/items/{id}/sidecars`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sidecars {
    pub sidecars: Vec<Sidecar>,
}

// ---- subtitle providers (lain.subtitle@1, D-137) ----------------------------

/// The OpenSubtitles.com provider kind.
pub const SUBTITLE_OPENSUBTITLES: &str = "opensubtitles";

/// One configured subtitle provider account. `api_key` and `password`
/// are credentials: stored server-side, carried in contract input,
/// never logged (`Debug` redacts them) and cleared by [`Self::public`].
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtitleProvider {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: String,
    /// Empty means the provider's public API.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub base_url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub username: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub password: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub priority: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub last_check_at: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_error: String,
    #[serde(default)]
    pub created_at: i64,
}

impl SubtitleProvider {
    /// The client-safe view: credentials cleared.
    pub fn public(&self) -> SubtitleProvider {
        SubtitleProvider { api_key: String::new(), password: String::new(), ..self.clone() }
    }
}

impl std::fmt::Debug for SubtitleProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let redact = |s: &str| if s.is_empty() { "" } else { "<redacted>" };
        f.debug_struct("SubtitleProvider")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("base_url", &self.base_url)
            .field("api_key", &redact(&self.api_key))
            .field("username", &self.username)
            .field("password", &redact(&self.password))
            .field("enabled", &self.enabled)
            .field("priority", &self.priority)
            .finish_non_exhaustive()
    }
}

/// One search (op `search`): by file hash and/or by title. Languages
/// are ISO 639-2/T codes (D-071's form).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtitleSearchInput {
    #[serde(default)]
    pub provider: SubtitleProvider,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hash: String,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub size: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub query: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub movie: bool,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub year: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub season: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub episode: i64,
    #[serde(default)]
    pub languages: Vec<String>,
}

/// One subtitle a provider offers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtitleCandidate {
    pub provider_id: String,
    pub file_id: String,
    /// ISO 639-2/T.
    pub language: String,
    /// The provider's tag when it carries a region (`pt-BR`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub region: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub release: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub file_name: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub hash_match: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub hi: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub forced: bool,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub downloads: i64,
    /// The provider's own numbering, when given.
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub season: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub episode: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtitleSearchOutput {
    pub candidates: Vec<SubtitleCandidate>,
}

/// A download link request (op `download`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtitleDownloadInput {
    #[serde(default)]
    pub provider: SubtitleProvider,
    #[serde(default)]
    pub file_id: String,
}

/// A short-lived link the core fetches (D-150: at most 5 MiB).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtitleDownloadOutput {
    pub link: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub file_name: String,
    /// Downloads left today; -1 = unknown.
    pub remaining: i64,
}

// ---- component jobs (lain.subtitle.translate@1, lain.acquire.extract@1) ------

/// A typed failure of a component job (`error` of [`TranslateJob`] and
/// [`ExtractJob`]): the app's own code and message, never a credential.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobError {
    pub code: String,
    pub msg: String,
}

/// Names one job of a component capability (ops `status` and `cancel`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRef {
    pub job: String,
}

/// The translation model a job uses (`lain.subtitle.translate@1`). The
/// key is a credential: carried in contract input only, never logged
/// (`Debug` redacts it) and never returned.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranslateModel {
    /// The app's provider kind (`ollama`, `openai`, `openrouter`, …).
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub base_url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key: String,
}

impl std::fmt::Debug for TranslateModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TranslateModel")
            .field("kind", &self.kind)
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .field("api_key", &if self.api_key.is_empty() { "" } else { "<redacted>" })
            .finish()
    }
}

/// Op `start` of `lain.subtitle.translate@1`: translate one subtitle
/// file. Paths are absolute on the host both sides run on; the
/// component reads and writes the files, lain never sends their bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranslateStartInput {
    pub input: String,
    /// Empty: the app's default (`<stem>.<lang>.<ext>` next to `input`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub output: String,
    /// BCP 47 (`pt-BR`).
    pub target_language: String,
    /// `None`: the app's configured default model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<TranslateModel>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub overwrite: bool,
}

/// One translation job. `state` is `queued`, `running`, `succeeded`,
/// `failed` or `cancelled` (terminal: the last three); `phase` is the
/// app's finer step and clients accept unknown phases. `done`/`total`
/// count cues (`total` 0 = not known yet).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranslateJob {
    pub job: String,
    /// The provider serving it; `status` and `cancel` go to this one.
    pub provider: String,
    pub state: String,
    #[serde(default)]
    pub phase: String,
    #[serde(default)]
    pub done: u64,
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub input: String,
    #[serde(default)]
    pub output: String,
    #[serde(default)]
    pub target_language: String,
    #[serde(default)]
    pub created_at_ms: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub finished_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JobError>,
}

/// Op `extract` of `lain.acquire.extract@1`: what a web page offers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractInput {
    pub url: String,
}

/// One downloadable rendition found on a page. Stream URLs stay inside
/// the component (they are often signed and short-lived); a job names
/// what it wants by `format`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExtractCandidate {
    pub id: String,
    /// `video` (with audio), `video-only` or `audio`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub quality: String,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub width: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub height: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub codec: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mime_type: String,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub filesize: i64,
}

/// The answer of op `extract`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExtractOutput {
    /// The page the extraction settled on.
    pub url: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub site: String,
    #[serde(default)]
    pub duration_s: f64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub thumbnail: String,
    /// The app's content class (`anime-2d`, `live-action`, …).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content_class: String,
    pub candidates: Vec<ExtractCandidate>,
    /// Subtitle languages offered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subtitles: Vec<String>,
    /// The provider that answered.
    #[serde(default)]
    pub provider: String,
}

/// Op `enqueue`: download one page's media.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractEnqueueInput {
    pub url: String,
    /// The app's selection (`best`, a quality such as `1080p`); empty:
    /// its default.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub format: String,
    /// Absolute directory to download into (a library folder); empty:
    /// the app's own routing.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub output_dir: String,
}

/// One download job. `state` is `queued`, `running`, `paused`,
/// `succeeded`, `failed` or `cancelled` (terminal: the last three).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractJob {
    pub job: String,
    pub provider: String,
    pub state: String,
    #[serde(default)]
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// The downloaded file, once known.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub output_path: String,
    #[serde(default)]
    pub downloaded_bytes: i64,
    /// 0 = not known.
    #[serde(default)]
    pub total_bytes: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JobError>,
}

/// A parsed release name (`lain.release.parse@1`). Zero numbers mean
/// "not stated"; `episodes` lists every episode a pack covers and
/// `absolute` marks absolute numbering (anime with no season marker).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub episode_title: String,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub year: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub season: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub episodes: Vec<i64>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub absolute: bool,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub volume: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub chapter: i64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub season_pack: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub group: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub resolution: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub codec: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub proper: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub repack: bool,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub version: i64,
    /// The provider that produced the record.
    #[serde(default)]
    pub parser: String,
}

// ---- composition and modules ------------------------------------------------

/// A stable sorted view of one binding for inspection endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindingView {
    pub capability: String,
    pub mode: String,
    pub providers: Vec<String>,
    pub generation: u64,
}

/// One registered provider for operator surfaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub id: String,
    pub capabilities: Vec<String>,
    pub healthy: bool,
}

/// A composition change or recovery action, kept for diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryEvent {
    pub at: i64,
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub capability: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub provider: String,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub generation: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

/// `GET /api/plugins` (admin).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginsView {
    pub composition: Vec<BindingView>,
    pub providers: Vec<String>,
    pub provider_info: Vec<ProviderInfo>,
    pub events: Vec<RegistryEvent>,
}

/// `POST /api/plugins/swap` (admin).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwapRequest {
    pub capability: String,
    pub providers: Vec<String>,
    /// The generation the caller saw; a stale one answers 409.
    pub generation: u64,
}

/// The answer of a successful swap.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwapReply {
    pub generation: u64,
    pub composition: Vec<BindingView>,
}

/// The answer of a successful withdraw.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WithdrawReply {
    pub composition: Vec<BindingView>,
}

/// One top-level client destination of a module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Destination {
    pub id: String,
    pub label: String,
    /// The web client's route for it (other clients map the id).
    pub path: String,
}

/// What a module shows in each client.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiSurfaces {
    #[serde(default)]
    pub destinations: Vec<Destination>,
    /// Settings section ids.
    #[serde(default)]
    pub settings: Vec<String>,
    /// Ctrl+K command ids.
    #[serde(default)]
    pub commands: Vec<String>,
}

/// One capability of a module as this server serves it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityView {
    pub capability: String,
    pub bound: bool,
    pub healthy: bool,
    pub providers: Vec<String>,
}

/// One module of `GET /api/modules` (D-154). `state` is `disabled`,
/// `unavailable`, `degraded` or `active`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleView {
    pub id: String,
    pub name: String,
    pub required: bool,
    pub enabled: bool,
    pub state: String,
    pub capabilities: Vec<CapabilityView>,
    pub routes: Vec<String>,
    pub library_kinds: Vec<String>,
    pub ui: UiSurfaces,
    pub components: Vec<String>,
}

/// `GET /api/modules`: `lain.modules@1`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModulesView {
    pub format: String,
    pub modules: Vec<ModuleView>,
}

// ---- errors and events ------------------------------------------------------

/// An error body: `{"error": msg}`, plus a stable `code` on typed
/// failures (`not-ported`, `module-disabled`, `stale-generation`, ...).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub code: String,
    /// The disabled module (`module-disabled`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub module: String,
    /// The current generation (`stale-generation`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
}

/// The stable error code of routes lain-rs does not serve yet (501).
pub const CODE_NOT_PORTED: &str = "not-ported";
/// The stable error code of routes of a disabled module (404).
pub const CODE_MODULE_DISABLED: &str = "module-disabled";

/// `ready`, the first frame of `GET /api/events`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadyEvent {
    pub server: String,
    pub version: String,
}

/// `module-state`: a module was enabled or disabled.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleStateEvent {
    pub module: String,
    pub enabled: bool,
    pub state: String,
}

/// `library-changed`: `action` is `created`, `updated`, `deleted`,
/// `scanned` (a scan committed the library: the counts say what
/// changed) or `item-deleted` (`item_id`'s file was removed).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryChangedEvent {
    pub library_id: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items_removed: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restored: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migrated: Option<i64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub item_id: String,
}

/// `scan-state` (everyone): a scan started, finished one library or
/// ended. `libraries_done`/`libraries` count the walked roots; `stats`
/// is present once the scan ended.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScanStateEvent {
    pub state: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub trigger: String,
    pub libraries: i64,
    pub libraries_done: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<ScanStats>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
}

/// `progress-changed`: only the user it belongs to receives it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProgressChangedEvent {
    pub item_id: String,
    pub progress: Progress,
}

/// `plugin-changed` (admins): `action` is `installed`, `reloaded` or
/// `removed`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginChangedEvent {
    pub provider: String,
    pub action: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_round_trips() {
        let raw = r#"{"id":"e4622164393ba829","library_id":"lib-1","kind":"episode","title":"Frieren","season":0,"episode":1,"year":0,"file_path":"/m/a.mkv","size":0,"confidence":0.8799999999999999,"origin":"lain-identify-anime","provenance":"identify:lain-identify-anime","updated_at":1794173709}"#;
        let it: CatalogItem = serde_json::from_str(raw).unwrap();
        let back = serde_json::to_value(&it).unwrap();
        assert_eq!(back, serde_json::from_str::<Value>(raw).unwrap());
        let with_future: CatalogItem =
            serde_json::from_str(&raw.replace("}", r#","root":"anime","rel_path":"a.mkv"}"#)).unwrap();
        assert_eq!(with_future.root, "anime");
        assert_eq!(serde_json::to_value(&with_future).unwrap()["rel_path"], "a.mkv");
        let with_unknown: CatalogItem = serde_json::from_str(&raw.replace("}", r#","future":1}"#)).unwrap();
        assert_eq!(serde_json::to_value(&with_unknown).unwrap()["future"], 1);
    }

    #[test]
    fn library_locked_is_a_response_field() {
        let raw = r#"{"id":"lib-1","name":"A","type":"anime","path":"/m","source":"s","created_at":1,"private":true,"owner":"u","locked":true}"#;
        let lib: Library = serde_json::from_str(raw).unwrap();
        assert_eq!(lib.locked, Some(true));
        assert!(lib.extra.is_empty());
        let stored = Library { locked: None, ..lib };
        assert!(serde_json::to_value(&stored).unwrap().get("locked").is_none());
    }

    #[test]
    fn login_reply_hides_the_token() {
        assert!(!format!("{:?}", LoginReply { token: "secret".into() }).contains("secret"));
    }
}
