//! `lain-probe` (D-190): technical facts about a media file from the
//! external `ffprobe` (never linked; DESIGN-v2 §13). One run of
//! `ffprobe -show_format -show_streams` yields a [`RawReport`] with
//! every field the planner, the transcoder and subtitle discovery need;
//! [`media_info`] bounds it into the wire's [`MediaInfo`] (the Go
//! `lain-probe-ffprobe` provider's shape). Media bytes are never read
//! here, only ffprobe's JSON.
//!
//! Calls block: run them on a blocking pool or a worker thread.

pub mod exec;

use std::path::Path;
use std::process::Command;
use std::time::Duration;

pub use lain_api::{MediaInfo, MediaStream};
use serde::{Deserialize, Serialize};

/// How long one probe may take (Go: 20 s).
pub const TIMEOUT: Duration = Duration::from_secs(20);

/// Stream tags ffprobe reports.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tags {
    pub language: String,
    pub title: String,
}

/// Stream disposition flags (0 or 1).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Disposition {
    pub default: i64,
    pub forced: i64,
}

/// One stream as ffprobe reports it (the fields lain reads).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RawStream {
    pub index: i64,
    pub codec_type: String,
    pub codec_name: String,
    pub profile: String,
    pub pix_fmt: String,
    pub color_transfer: String,
    pub color_primaries: String,
    pub color_space: String,
    pub field_order: String,
    pub width: i64,
    pub height: i64,
    pub channels: i64,
    pub bit_rate: String,
    pub r_frame_rate: String,
    pub bits_per_raw_sample: String,
    pub tags: Tags,
    pub disposition: Disposition,
}

/// The container facts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RawFormat {
    pub format_name: String,
    pub duration: String,
    pub bit_rate: String,
}

/// ffprobe's whole answer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RawReport {
    pub streams: Vec<RawStream>,
    pub format: RawFormat,
}

impl RawStream {
    pub fn is_default(&self) -> bool {
        self.disposition.default != 0
    }

    /// More than 8 bits per sample (decides hardware decoding).
    pub fn is_10bit(&self) -> bool {
        if self.bits_per_raw_sample.trim().parse::<i64>().is_ok_and(|b| b > 8) {
            return true;
        }
        let p = self.pix_fmt.to_ascii_lowercase();
        p.contains("10le") || p.contains("10be") || p.starts_with("p010") || p.starts_with("p410")
    }

    /// ffprobe's `num/den` rate; 0 when unknown.
    pub fn frame_rate(&self) -> f64 {
        let Some((n, d)) = self.r_frame_rate.split_once('/') else { return 0.0 };
        match (n.trim().parse::<f64>(), d.trim().parse::<f64>()) {
            (Ok(n), Ok(d)) if n > 0.0 && d > 0.0 && n.is_finite() && d.is_finite() => n / d,
            _ => 0.0,
        }
    }

    /// The stream bitrate in bits per second; 0 when absent or `N/A`.
    pub fn bit_rate_bps(&self) -> i64 {
        self.bit_rate.trim().parse::<i64>().ok().filter(|n| *n >= 0).unwrap_or(0)
    }

    /// HDR transfer (PQ or HLG).
    pub fn is_hdr(&self) -> bool {
        matches!(self.color_transfer.to_ascii_lowercase().as_str(), "smpte2084" | "arib-std-b67")
    }

    /// An interlaced field order.
    pub fn is_interlaced(&self) -> bool {
        matches!(self.field_order.to_ascii_lowercase().as_str(), "tt" | "bb" | "tb" | "bt")
    }
}

impl RawReport {
    /// The probed duration; 0 when unknown (never invented).
    pub fn duration_sec(&self) -> f64 {
        match self.format.duration.trim().parse::<f64>() {
            Ok(d) if d > 0.0 && d.is_finite() => d,
            _ => 0.0,
        }
    }

    pub fn first_video(&self) -> Option<&RawStream> {
        self.streams.iter().find(|s| s.codec_type == "video")
    }
}

/// A text subtitle codec that converts to WebVTT.
pub fn text_subtitle_codec(codec: &str) -> bool {
    matches!(codec.to_ascii_lowercase().as_str(), "subrip" | "srt" | "ass" | "ssa" | "webvtt" | "mov_text")
}

/// An image subtitle codec (burn-in only).
pub fn image_subtitle_codec(codec: &str) -> bool {
    matches!(codec.to_ascii_lowercase().as_str(), "hdmv_pgs_subtitle" | "dvd_subtitle" | "dvb_subtitle" | "xsub")
}

/// Why a probe failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeError {
    /// ffprobe could not run, timed out or rejected the file.
    Failed(String),
    /// ffprobe answered something that is not its JSON.
    Invalid(String),
}

impl std::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProbeError::Failed(e) => write!(f, "ffprobe failed: {e}"),
            ProbeError::Invalid(e) => write!(f, "invalid ffprobe output: {e}"),
        }
    }
}

impl std::error::Error for ProbeError {}

/// The ffprobe binary for a configured path: empty means `ffprobe` on
/// `PATH`.
pub fn binary(configured: &str) -> &str {
    if configured.trim().is_empty() { "ffprobe" } else { configured }
}

/// Parses ffprobe's `-of json` output.
pub fn parse(raw: &[u8]) -> Result<RawReport, ProbeError> {
    serde_json::from_slice(raw).map_err(|e| ProbeError::Invalid(e.to_string()))
}

/// Runs `ffprobe` (`binary`, see [`binary`]) on one file.
pub fn probe(binary: &str, path: &Path, timeout: Duration) -> Result<RawReport, ProbeError> {
    let mut cmd = Command::new(binary);
    cmd.args(["-v", "error", "-show_format", "-show_streams", "-of", "json"]).arg(path);
    let out = exec::run(cmd, None, timeout).map_err(|e| ProbeError::Failed(e.to_string()))?;
    if !out.status.success() {
        return Err(ProbeError::Failed(out.describe()));
    }
    parse(&out.stdout)
}

/// Bounds a report into the wire's [`MediaInfo`]: video, audio and
/// subtitle streams only, lowercased codecs, malformed geometry and
/// durations clamped to "unknown". A file with no such stream is an
/// error (it is not media).
pub fn media_info(r: &RawReport) -> Result<MediaInfo, ProbeError> {
    let duration = match r.format.duration.trim().parse::<f64>() {
        Ok(d) if d >= 0.0 && d.is_finite() => d,
        _ => 0.0,
    };
    let streams: Vec<MediaStream> = r
        .streams
        .iter()
        .filter(|s| matches!(s.codec_type.as_str(), "video" | "audio" | "subtitle"))
        .map(|s| MediaStream {
            index: s.index,
            kind: s.codec_type.clone(),
            codec: s.codec_name.to_ascii_lowercase(),
            profile: s.profile.clone(),
            pixel_format: s.pix_fmt.to_ascii_lowercase(),
            width: s.width.max(0),
            height: s.height.max(0),
            channels: s.channels.max(0),
            language: s.tags.language.to_ascii_lowercase(),
            title: s.tags.title.clone(),
            default: s.disposition.default != 0,
            forced: s.disposition.forced != 0,
            color_transfer: s.color_transfer.to_ascii_lowercase(),
            color_primaries: s.color_primaries.to_ascii_lowercase(),
            bit_rate: s.bit_rate_bps(),
            convertible: s.codec_type == "subtitle" && text_subtitle_codec(&s.codec_name),
        })
        .collect();
    if streams.is_empty() {
        return Err(ProbeError::Invalid("no media streams".into()));
    }
    Ok(MediaInfo { format: r.format.format_name.clone(), duration, streams })
}

/// The subtitle tracks of a probed file, in stream order: what a player
/// lists (embedded tracks; `convertible` ones extract to WebVTT).
pub fn subtitle_tracks(info: &MediaInfo) -> Vec<MediaStream> {
    info.streams.iter().filter(|s| s.kind == "subtitle").cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"streams":[
        {"index":0,"codec_type":"video","codec_name":"H264","pix_fmt":"YUV420P","width":1920,"height":1080,
         "r_frame_rate":"24000/1001","bit_rate":"4000000","disposition":{"default":1,"forced":0}},
        {"index":1,"codec_type":"audio","codec_name":"aac","channels":6,"tags":{"language":"JPN","title":"Main"},
         "disposition":{"default":1}},
        {"index":2,"codec_type":"subtitle","codec_name":"subrip","tags":{"language":"eng"},"disposition":{"forced":1}},
        {"index":3,"codec_type":"subtitle","codec_name":"hdmv_pgs_subtitle"},
        {"index":4,"codec_type":"attachment","codec_name":"ttf"}],
        "format":{"format_name":"matroska,webm","duration":"1440.5","bit_rate":"5000000"}}"#;

    #[test]
    fn bounds_the_report() {
        let r = parse(SAMPLE.as_bytes()).unwrap();
        let info = media_info(&r).unwrap();
        assert_eq!(info.format, "matroska,webm");
        assert_eq!(info.duration, 1440.5);
        assert_eq!(info.streams.len(), 4, "attachments are not media");
        let v = &info.streams[0];
        assert_eq!(
            (v.codec.as_str(), v.pixel_format.as_str(), v.bit_rate, v.default),
            ("h264", "yuv420p", 4_000_000, true)
        );
        let a = &info.streams[1];
        assert_eq!((a.language.as_str(), a.title.as_str(), a.channels), ("jpn", "Main", 6));
        assert!(info.streams[2].convertible && info.streams[2].forced);
        assert!(!info.streams[3].convertible);
        assert_eq!(subtitle_tracks(&info).iter().map(|s| s.index).collect::<Vec<_>>(), vec![2, 3]);
        assert!((r.streams[0].frame_rate() - 23.976).abs() < 0.01);
        assert_eq!(r.duration_sec(), 1440.5);
    }

    #[test]
    fn malformed_values_degrade_to_unknown() {
        for d in ["nan", "inf", "-5", "x"] {
            let raw = format!(
                r#"{{"streams":[{{"index":0,"codec_type":"video","codec_name":"h264","width":-4,"bit_rate":"N/A"}}],"format":{{"duration":"{d}"}}}}"#
            );
            let r = parse(raw.as_bytes()).unwrap();
            let info = media_info(&r).unwrap();
            assert_eq!(info.duration, 0.0, "{d}");
            assert_eq!((info.streams[0].width, info.streams[0].bit_rate), (0, 0));
            assert_eq!(r.duration_sec(), 0.0);
        }
        assert!(media_info(&parse(br#"{"streams":[]}"#).unwrap()).is_err());
        assert!(parse(b"not json").is_err());
    }

    #[test]
    fn stream_facts() {
        let s = RawStream { pix_fmt: "yuv420p10le".into(), ..Default::default() };
        assert!(s.is_10bit());
        let s = RawStream { bits_per_raw_sample: "10".into(), ..Default::default() };
        assert!(s.is_10bit());
        assert!(!RawStream::default().is_10bit());
        assert_eq!(RawStream { r_frame_rate: "0/0".into(), ..Default::default() }.frame_rate(), 0.0);
        assert!(RawStream { color_transfer: "SMPTE2084".into(), ..Default::default() }.is_hdr());
        assert!(RawStream { field_order: "tt".into(), ..Default::default() }.is_interlaced());
        assert!(!RawStream { field_order: "progressive".into(), ..Default::default() }.is_interlaced());
        assert!(
            text_subtitle_codec("ASS") && !text_subtitle_codec("dvd_subtitle") && image_subtitle_codec("dvd_subtitle")
        );
        assert_eq!(binary(""), "ffprobe");
        assert_eq!(binary("/opt/ffprobe"), "/opt/ffprobe");
    }

    #[test]
    fn missing_ffprobe_fails_cleanly() {
        let e = probe("lain-no-such-ffprobe", Path::new("x.mkv"), Duration::from_secs(1)).unwrap_err();
        assert!(matches!(e, ProbeError::Failed(_)));
    }
}
