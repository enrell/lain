//! The playback planner (`lain.playback.plan@1`, D-023/D-030/D-058):
//! direct play versus transcode, from the probed streams and what the
//! client says it decodes. It returns references, never bytes.

use lain_api::{MediaInfo, MediaStream, PlaybackPlan};
use serde::{Deserialize, Serialize};

/// What the planner is asked. `file_path` comes from the gateway after
/// the catalog lookup; `media_info` is absent for mpv/desktop clients.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlanInput {
    pub item_id: String,
    #[serde(default)]
    pub client: String,
    #[serde(default)]
    pub network: String,
    /// The client's decode claim (D-058): `None` is unknown, an empty
    /// list claims nothing.
    #[serde(default)]
    pub capabilities: Option<Vec<String>>,
    pub file_path: String,
    #[serde(default)]
    pub media_info: Option<MediaInfo>,
    /// HDR can be tone-mapped right now (enabled and probed).
    #[serde(default)]
    pub tone_map: bool,
}

/// Extensions served directly, mapped to the container token a client
/// reports for them.
fn container_token(ext: &str) -> Option<&'static str> {
    Some(match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
        "mp4" | "m4v" | "mov" | "m4a" => "mp4",
        "mkv" => "mkv",
        "webm" => "webm",
        "ogg" | "ogv" | "oga" | "opus" => "ogg",
        "mp3" => "mp3",
        "flac" => "flac",
        _ => return None,
    })
}

const CONTAINERS: [&str; 6] = ["mp4", "mkv", "webm", "ogg", "mp3", "flac"];

/// ffprobe codec names mapped to the family token a client reports.
fn codec_token(codec: &str) -> Option<&'static str> {
    Some(match codec.to_ascii_lowercase().as_str() {
        "h264" | "avc1" => "h264",
        "hevc" | "h265" => "hevc",
        "vp8" => "vp8",
        "vp9" => "vp9",
        "av1" => "av1",
        "aac" => "aac",
        "mp3" => "mp3",
        "opus" => "opus",
        "vorbis" => "vorbis",
        "flac" => "flac",
        "ac3" => "ac3",
        "eac3" => "eac3",
        "dts" => "dts",
        _ => return None,
    })
}

const FAMILIES: [&str; 13] =
    ["h264", "hevc", "vp8", "vp9", "av1", "aac", "mp3", "opus", "vorbis", "flac", "ac3", "eac3", "dts"];

/// A token inside this server's vocabulary: `mkv` or `mkv/h264`.
pub fn known_capability_token(tok: &str) -> bool {
    let (container, codec) = match tok.split_once('/') {
        Some((c, k)) => (c, Some(k)),
        None => (tok, None),
    };
    CONTAINERS.contains(&container) && codec.is_none_or(|c| FAMILIES.contains(&c))
}

/// Normalizes the `caps` query values (repeated or comma-separated):
/// lowercase, known tokens only, deduplicated and sorted. No value at
/// all is `None` (unknown); present but empty is `Some([])`.
pub fn parse_capabilities(raw: &[String]) -> Option<Vec<String>> {
    if raw.is_empty() {
        return None;
    }
    let mut out: Vec<String> = raw
        .iter()
        .flat_map(|v| v.split(','))
        .map(|t| t.trim().to_ascii_lowercase())
        .filter(|t| !t.is_empty() && known_capability_token(t))
        .collect();
    out.sort();
    out.dedup();
    Some(out)
}

fn ext_of(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match name.rfind('.') {
        Some(i) => name[i + 1..].to_ascii_lowercase(),
        None => String::new(),
    }
}

/// Plans one play.
pub fn plan(input: &PlanInput) -> PlaybackPlan {
    let asset = format!("asset:{}", input.item_id);
    let ext = ext_of(&input.file_path);
    let client = input.client.to_ascii_lowercase();
    let mk = |mode: &str, available: bool, streams: Vec<MediaStream>, reason: &str| PlaybackPlan {
        mode: mode.into(),
        asset: asset.clone(),
        available,
        streams,
        reason: reason.into(),
        ..PlaybackPlan::default()
    };
    if client.contains("mpv") || client.contains("desktop") {
        return mk("direct", true, Vec::new(), "");
    }
    if let Some(info) = &input.media_info {
        let streams = info.streams.clone();
        if has_hdr(info) {
            if input.tone_map {
                return mk("transcode", true, streams, "");
            }
            return mk(
                "transcode-required",
                false,
                Vec::new(),
                "HDR browser playback needs tone mapping, which is unavailable on this server",
            );
        }
        if direct_play(&ext, info, input.capabilities.as_deref()) {
            return mk("direct", true, streams, "");
        }
        return mk("transcode", true, streams, "");
    }
    match ext.as_str() {
        "mp4" | "m4v" | "webm" | "mp3" | "ogg" => mk("direct", true, Vec::new(), ""),
        _ => mk("transcode", true, Vec::new(), ""),
    }
}

fn has_hdr(info: &MediaInfo) -> bool {
    info.streams.iter().any(|s| {
        s.kind == "video" && matches!(s.color_transfer.to_ascii_lowercase().as_str(), "smpte2084" | "arib-std-b67")
    })
}

fn direct_play(ext: &str, info: &MediaInfo, caps: Option<&[String]>) -> bool {
    match caps {
        None => browser_direct(ext, info),
        Some(caps) => reported_direct(ext, info, caps),
    }
}

/// The streams a player uses: the first video, the default audio (the
/// first when none is marked).
fn select_tracks(info: &MediaInfo) -> (Option<&MediaStream>, Option<&MediaStream>) {
    let mut video = None;
    let mut audio: Option<&MediaStream> = None;
    for s in &info.streams {
        match s.kind.as_str() {
            "video" if video.is_none() => video = Some(s),
            "audio" if audio.is_none() || (!audio.is_some_and(|a| a.default) && s.default) => audio = Some(s),
            _ => {}
        }
    }
    (video, audio)
}

/// A client's claim applied to the probed tracks: the container must
/// open, and each used track must decode in that container.
fn reported_direct(ext: &str, info: &MediaInfo, caps: &[String]) -> bool {
    let has = |t: &str| caps.iter().any(|c| c == t);
    let Some(container) = container_token(ext) else { return false };
    if !has(container) {
        return false;
    }
    let (video, audio) = select_tracks(info);
    if video.is_none() && audio.is_none() {
        return false;
    }
    if let Some(v) = video {
        match codec_token(&v.codec) {
            Some(f) if has(&format!("{container}/{f}")) && video_decodable(v) => {}
            _ => return false,
        }
    }
    if let Some(a) = audio {
        match codec_token(&a.codec) {
            Some(f) if has(&format!("{container}/{f}")) => {}
            _ => return false,
        }
    }
    true
}

/// H.264 High 10, 4:2:2 and 4:4:4 decode nowhere in a browser, whatever
/// a client claims.
fn video_decodable(v: &MediaStream) -> bool {
    if !v.codec.eq_ignore_ascii_case("h264") {
        return true;
    }
    matches!(v.pixel_format.to_ascii_lowercase().as_str(), "yuv420p" | "yuvj420p")
}

/// The conservative per-container rules (D-030) for clients that report
/// nothing.
fn browser_direct(ext: &str, info: &MediaInfo) -> bool {
    let (video, audio) = select_tracks(info);
    let Some(video) = video else { return ext == "mp3" || ext == "ogg" };
    let audio_codec = audio.map(|a| a.codec.to_ascii_lowercase()).unwrap_or_default();
    match ext {
        "mp4" | "m4v" => {
            let pixel = video.pixel_format.to_ascii_lowercase();
            let video_ok = video.codec.eq_ignore_ascii_case("h264") && (pixel == "yuv420p" || pixel == "yuvj420p");
            video_ok && (audio.is_none() || matches!(audio_codec.as_str(), "aac" | "mp3"))
        }
        "webm" => {
            let codec = video.codec.to_ascii_lowercase();
            let video_ok = matches!(codec.as_str(), "vp8" | "vp9" | "av1");
            video_ok && (audio.is_none() || matches!(audio_codec.as_str(), "opus" | "vorbis"))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(kind: &str, codec: &str) -> MediaStream {
        MediaStream { kind: kind.into(), codec: codec.into(), ..Default::default() }
    }

    fn info(streams: Vec<MediaStream>) -> MediaInfo {
        MediaInfo { format: "x".into(), duration: 10.0, streams }
    }

    fn h264() -> MediaStream {
        MediaStream { pixel_format: "yuv420p".into(), ..stream("video", "h264") }
    }

    fn input(path: &str, mi: Option<MediaInfo>, caps: Option<Vec<&str>>) -> PlanInput {
        PlanInput {
            item_id: "i".into(),
            client: "web".into(),
            file_path: path.into(),
            media_info: mi,
            capabilities: caps.map(|c| c.into_iter().map(String::from).collect()),
            ..Default::default()
        }
    }

    #[test]
    fn desktop_always_direct() {
        let mut i = input("/m/a.mkv", None, None);
        i.client = "lain-desktop".into();
        let p = plan(&i);
        assert_eq!((p.mode.as_str(), p.available, p.asset.as_str()), ("direct", true, "asset:i"));
    }

    #[test]
    fn container_guess_without_probe() {
        assert_eq!(plan(&input("/m/a.mp4", None, None)).mode, "direct");
        assert_eq!(plan(&input("/m/a.mkv", None, None)).mode, "transcode");
    }

    #[test]
    fn browser_rules_with_probe() {
        let mp4 = info(vec![h264(), stream("audio", "aac")]);
        assert_eq!(plan(&input("/m/a.mp4", Some(mp4.clone()), None)).mode, "direct");
        assert_eq!(plan(&input("/m/a.mkv", Some(mp4), None)).mode, "transcode");
        let ac3 = info(vec![h264(), stream("audio", "ac3")]);
        assert_eq!(plan(&input("/m/a.mp4", Some(ac3), None)).mode, "transcode", "AC-3 is not web-safe");
        let hi10 = info(vec![MediaStream { pixel_format: "yuv420p10le".into(), ..stream("video", "h264") }]);
        assert_eq!(plan(&input("/m/a.mp4", Some(hi10), None)).mode, "transcode");
    }

    #[test]
    fn hdr_needs_tone_mapping() {
        let hdr = info(vec![MediaStream { color_transfer: "smpte2084".into(), ..stream("video", "hevc") }]);
        let p = plan(&input("/m/a.mkv", Some(hdr.clone()), None));
        assert_eq!((p.mode.as_str(), p.available), ("transcode-required", false));
        let mut i = input("/m/a.mkv", Some(hdr), None);
        i.tone_map = true;
        assert_eq!(plan(&i).mode, "transcode");
    }

    #[test]
    fn reported_capabilities_pair_codec_and_container() {
        let mkv = info(vec![h264(), stream("audio", "aac"), stream("audio", "ac3")]);
        let ok = Some(vec!["mkv", "mkv/h264", "mkv/aac"]);
        assert_eq!(plan(&input("/m/a.mkv", Some(mkv.clone()), ok)).mode, "direct");
        let mp4_only = Some(vec!["mkv", "mp4/h264", "mp4/aac"]);
        assert_eq!(plan(&input("/m/a.mkv", Some(mkv.clone()), mp4_only)).mode, "transcode");
        assert_eq!(plan(&input("/m/a.mkv", Some(mkv), Some(vec![]))).mode, "transcode", "an empty claim");
        let hi10 = info(vec![MediaStream { pixel_format: "yuv422p".into(), ..stream("video", "h264") }]);
        assert_eq!(plan(&input("/m/a.mkv", Some(hi10), Some(vec!["mkv", "mkv/h264"]))).mode, "transcode");
        let avi = info(vec![h264()]);
        assert_eq!(plan(&input("/m/a.avi", Some(avi), Some(vec!["mkv", "mkv/h264"]))).mode, "transcode");
    }

    #[test]
    fn default_audio_track_wins() {
        let mut ac3 = stream("audio", "ac3");
        ac3.default = true;
        let mkv = info(vec![h264(), stream("audio", "aac"), ac3]);
        assert_eq!(plan(&input("/m/a.mkv", Some(mkv), Some(vec!["mkv", "mkv/h264", "mkv/aac"]))).mode, "transcode");
    }

    #[test]
    fn capability_parsing() {
        assert_eq!(parse_capabilities(&[]), None);
        assert_eq!(parse_capabilities(&["".into()]), Some(vec![]));
        assert_eq!(
            parse_capabilities(&["MKV/h264, mkv,zzz,mkv/zzz".into(), "mkv".into()]),
            Some(vec!["mkv".to_string(), "mkv/h264".to_string()])
        );
    }
}
