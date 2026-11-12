//! The watch module's wire shapes (D-190): playback plans, the probed
//! streams they carry, transcode sessions and settings, and local
//! playback. Names and `omitempty` behavior follow the Go server's
//! `internal/contracts` and `internal/gateway` exactly.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

fn is_false(v: &bool) -> bool {
    !*v
}

fn is_zero(v: &i64) -> bool {
    *v == 0
}

fn is_zero_f64(v: &f64) -> bool {
    *v == 0.0
}

/// One probed stream, normalized for planner and player policy.
/// `index` is the source-global ffmpeg stream index.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MediaStream {
    pub index: i64,
    #[serde(rename = "type")]
    pub kind: String,
    pub codec: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub profile: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pixel_format: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub width: i64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub height: i64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub channels: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub language: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub default: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub forced: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color_transfer: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color_primaries: String,
    /// Bits per second; 0 when ffprobe reports none.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub bit_rate: i64,
    /// A subtitle track that converts to WebVTT (text, not image).
    #[serde(default, skip_serializing_if = "is_false")]
    pub convertible: bool,
}

/// Bounded technical metadata of one file (`lain.media.probe@1`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MediaInfo {
    pub format: String,
    #[serde(default, skip_serializing_if = "is_zero_f64")]
    pub duration: f64,
    #[serde(default)]
    pub streams: Vec<MediaStream>,
}

/// `GET /api/items/{id}/playback`: how to play an item. `mode` is
/// `direct`, `transcode`, `transcode-required` or `unavailable`; a
/// transcode plan names the side-effect-free session the client may
/// start. `duration_sec` is the probed length, absent when unknown.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaybackPlan {
    pub mode: String,
    #[serde(default)]
    pub asset: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub profile: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub session: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub state: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub streams: Vec<MediaStream>,
    pub available: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_sec: Option<f64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// One quality ladder entry the player can request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscodeQuality {
    pub name: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub max_width: i64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub max_height: i64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub bitrate_kbps: i64,
}

/// The operator's transcode policy (D-045), `GET/PUT
/// /api/admin/settings/transcode`. Zero values decode as "unset"; the
/// server fills them with its defaults before validating.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TranscodeSettings {
    pub default_delivery: String,
    pub hls_segment_seconds: i64,
    pub hls_segment_container: String,
    pub throttle: bool,
    pub throttle_ahead_sec: i64,
    pub segment_deletion: bool,
    pub segment_keep_sec: i64,
    pub idle_timeout_sec: i64,
    pub cache_bytes: i64,
    pub queue_size: i64,
    pub max_concurrent: i64,
    pub thread_count: i64,
    #[serde(rename = "max_muxing_queue_size")]
    pub muxing_queue_size: i64,
    pub transcode_temp_path: String,
    pub remote_bitrate_limit_kbps: i64,
    pub encoder_preset: String,
    pub crf: i64,
    pub h264_preset: String,
    pub h265_preset: String,
    pub av1_preset: String,
    pub h264_crf: i64,
    pub h265_crf: i64,
    pub av1_crf: i64,
    pub allow_hevc: bool,
    pub allow_av1: bool,
    pub qualities: Vec<TranscodeQuality>,
    pub audio_bitrate_kbps: i64,
    pub audio_vbr: bool,
    pub downmix_audio: bool,
    pub downmix_audio_boost: f64,
    pub downmix_stereo_algorithm: String,
    pub subtitle_mode: String,
    /// `None` is the shipped default (on).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_subtitle_extraction: Option<bool>,
    pub fallback_font_path: String,
    pub fallback_font_name: String,
    /// `None` is the shipped default (on).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_font_enabled: Option<bool>,
    pub tone_mapping: bool,
    pub tone_mapping_algorithm: String,
    pub tone_mapping_mode: String,
    pub tone_mapping_peak_nits: i64,
    pub deinterlace: String,
    pub deinterlace_method: String,
    pub deinterlace_double_rate: bool,
    pub hardware_acceleration: String,
    pub hardware_decode_codecs: Vec<String>,
    pub hardware_encode: bool,
    pub hardware_low_power: bool,
    pub hardware_decode_10bit_hevc: bool,
    pub hardware_decode_10bit_vp9: bool,
    pub hardware_device: String,
    pub ffmpeg_path: String,
    pub ffprobe_path: String,
}

/// The path-free probe of the local ffmpeg for the settings UI.
/// `encoders` is `null` when none of the listed encoders exist.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscodeCapabilities {
    pub ffmpeg: String,
    pub encoders: Option<Vec<String>>,
    pub tone_mapping: bool,
    pub tone_mapping_bt2390: bool,
    /// The libplacebo shader path executes (D-274).
    #[serde(default)]
    pub video_effects: bool,
    #[serde(default)]
    pub hardware: BTreeMap<String, bool>,
}

/// `GET/PUT /api/admin/settings/transcode`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TranscodeSettingsView {
    pub settings: TranscodeSettings,
    pub capabilities: TranscodeCapabilities,
}

/// `GET /api/playback/options`: what the player's quality menu offers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaybackOptions {
    pub default_delivery: String,
    pub deliveries: Vec<String>,
    pub qualities: Vec<TranscodeQuality>,
    /// Video effects this server can render into a session (D-274);
    /// empty when its ffmpeg cannot.
    #[serde(default)]
    pub effects: Vec<String>,
}

/// One video's length (`GET /api/durations`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ItemDuration {
    pub item_id: String,
    pub duration_sec: f64,
}

/// The envelope of `GET /api/durations`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ItemDurations {
    #[serde(default)]
    pub items: Vec<ItemDuration>,
}

/// How an item's storyboard is laid out (`GET /api/items/{id}/storyboard`):
/// tile `i`, row by row, is the frame near `i * interval_sec`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryboardLayout {
    pub interval_sec: i64,
    pub count: i64,
    pub columns: i64,
    pub rows: i64,
    pub tile_width: i64,
    pub tile_height: i64,
}

/// A transcode session as clients see it: never a filesystem path.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TranscodeStatus {
    pub session: String,
    pub state: String,
    pub profile: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub delivery: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub method: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub cached: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub playable: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub encoder: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hardware: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fallback: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub video_codec: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub audio_codec: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub width: i64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub height: i64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub bitrate_kbps: i64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub video_direct: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub audio_direct: bool,
    #[serde(default, skip_serializing_if = "is_zero_f64")]
    pub fps: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub output_bitrate_kbps: i64,
    #[serde(default, skip_serializing_if = "is_zero_f64")]
    pub progress: f64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_subtitle: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error_code: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub queued_at: i64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub started_at: i64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub finished_at: i64,
}

/// One session in the operator's list: the public view plus its owner.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AdminTranscodeStatus {
    #[serde(flatten)]
    pub status: TranscodeStatus,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub user_id: String,
}

/// `GET /api/admin/transcodes`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AdminTranscodes {
    pub sessions: Vec<AdminTranscodeStatus>,
}

/// `POST /api/items/{id}/transcode`: the client's selection. Every
/// field is optional; the server's settings fill the rest.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TranscodeSelection {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub delivery: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub quality: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub video_codec: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub audio_codec: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_stream: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle_stream: Option<i64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subtitle_mode: String,
    /// The source position to begin at (resume or seek).
    #[serde(default, skip_serializing_if = "is_zero_f64")]
    pub start_sec: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_video_stream_copy: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_audio_stream_copy: Option<bool>,
    /// Video codecs beyond H.264 the client decodes in fMP4 (D-273):
    /// a source in one of them is copied instead of re-encoded. Only
    /// `hevc` is known; other values are ignored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub video_copy_codecs: Vec<String>,
    /// A video effect the server renders into the session (D-274): one
    /// of `PlaybackOptions.effects`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub video_effect: String,
}

/// `GET /api/localplay`: players that can open on the server's own
/// screen. `null` for a browser on another machine.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalPlayers {
    pub players: Option<Vec<String>>,
}

/// `POST /api/items/{id}/play-local`; an empty `player` picks the first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayLocalRequest {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub player: String,
}

/// The local session that started: `count` items queued.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayLocalReply {
    pub status: String,
    pub player: String,
    pub count: i64,
}

/// `transcode-changed` (D-193): a session reached `ready` or `failed`,
/// or was cancelled (`idle`). Sent to its owner and to admins.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscodeChangedEvent {
    pub session: String,
    pub state: String,
    #[serde(default)]
    pub item_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_status_omits_empty_fields() {
        let s =
            TranscodeStatus { session: "s".into(), state: "ready".into(), profile: "p".into(), ..Default::default() };
        assert_eq!(serde_json::to_string(&s).unwrap(), r#"{"session":"s","state":"ready","profile":"p"}"#);
        let a = AdminTranscodeStatus { status: s, user_id: "u".into() };
        assert_eq!(serde_json::to_value(&a).unwrap()["user_id"], "u");
    }

    #[test]
    fn plan_keeps_go_shape() {
        let raw = r#"{"mode":"direct","asset":"asset:x","streams":[{"index":0,"type":"video","codec":"h264","width":320}],"available":true,"duration_sec":2.0}"#;
        let p: PlaybackPlan = serde_json::from_str(raw).unwrap();
        assert_eq!(p.streams[0].kind, "video");
        assert_eq!(serde_json::to_value(&p).unwrap(), serde_json::from_str::<Value>(raw).unwrap());
    }

    #[test]
    fn settings_decode_partially() {
        let s: TranscodeSettings = serde_json::from_str(r#"{"crf":20,"allow_subtitle_extraction":false}"#).unwrap();
        assert_eq!((s.crf, s.allow_subtitle_extraction, s.queue_size), (20, Some(false), 0));
        assert!(serde_json::to_value(TranscodeSettings::default()).unwrap().get("fallback_font_enabled").is_none());
    }
}
