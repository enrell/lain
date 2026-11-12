//! Where a play comes from (D-246), in the desktop's order (D-182): a
//! finished offline copy; else the server's plan for `client=android`
//! with the device's decode claim (D-058): `direct` streams the file,
//! `transcode` starts (or joins) an HLS session at the resume point and
//! waits until its playlist is playable; a plan that is not available
//! is [`LainError::Unplayable`] with the server's reason.
//!
//! An HLS session carries one audio stream (D-280): the one the app asks
//! for (`audio_stream`, or the stream in the title's `audio_language`),
//! else the server's choice by the account's audio language. Another
//! audio track is another session, as another quality is.

use std::sync::Arc;
use std::time::Duration;

use lain_api::{MediaStream, PlaybackPlan, TranscodeSelection};
use lain_client::Client;

use crate::LainCore;
use crate::core::Backend;
use crate::error::LainError;
use crate::types::{AudioTrack, PlaySource, QualityInfo, SourceKind, StoryboardSheet, SubtitleTrack};

/// The plan's `client` value for this app.
pub const CLIENT: &str = "android";

/// How long to wait for a new HLS session's first segments.
const HLS_READY_WAIT: Duration = Duration::from_secs(45);

fn typeset(codec_or_format: &str) -> bool {
    matches!(codec_or_format.to_ascii_lowercase().as_str(), "ass" | "ssa")
}

fn label(language: &str, title: &str, extra: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if !language.is_empty() && language != "und" {
        parts.push(language);
    }
    if !title.is_empty() {
        parts.push(title);
    }
    if !extra.is_empty() {
        parts.push(extra);
    }
    if parts.is_empty() { "Unknown".into() } else { parts.join(" · ") }
}

/// Sidecars and the plan's convertible embedded tracks, as WebVTT URLs.
/// A failure to list sidecars leaves them out (logged), never the play.
async fn subtitles(c: &Client, item_id: &str, streams: &[MediaStream]) -> Vec<SubtitleTrack> {
    let mut out = Vec::new();
    match c.sidecars(item_id).await {
        Ok(list) => {
            for s in list {
                let Ok(url) = c.sidecar_url(item_id, s.index) else { continue };
                let mut extra = s.format.to_ascii_uppercase();
                if s.hi {
                    extra.push_str(" · SDH");
                }
                out.push(SubtitleTrack {
                    url,
                    label: label(&s.language, &s.tag, &extra),
                    language: s.language,
                    origin: "sidecar".into(),
                    forced: s.forced,
                    typeset: typeset(&s.format),
                    stream: -1,
                });
            }
        }
        Err(e) => eprintln!("lain-ffi: sidecars of {item_id}: {e}"),
    }
    for s in streams.iter().filter(|s| s.kind == "subtitle" && s.convertible) {
        let Ok(url) = c.subtitles_url(item_id, s.index) else { continue };
        out.push(SubtitleTrack {
            url,
            label: label(&s.language, &s.title, &s.codec.to_ascii_uppercase()),
            language: if s.language.is_empty() { "und".into() } else { s.language.clone() },
            origin: "embedded".into(),
            forced: s.forced,
            typeset: typeset(&s.codec),
            stream: s.index,
        });
    }
    out
}

/// The file's audio streams as the player lists them.
fn audio_tracks(streams: &[MediaStream]) -> Vec<AudioTrack> {
    streams
        .iter()
        .filter(|s| s.kind == "audio")
        .map(|s| AudioTrack {
            stream: s.index,
            label: label(&s.language, &s.title, &s.codec.to_ascii_uppercase()),
            language: if s.language.is_empty() { "und".into() } else { s.language.clone() },
            default: s.default,
        })
        .collect()
}

/// ISO 639-2 bibliographic codes and their terminology twins, as
/// lain-watch's `tracks.rs` folds them.
const BIBLIOGRAPHIC: [(&str, &str); 16] = [
    ("fre", "fra"),
    ("ger", "deu"),
    ("chi", "zho"),
    ("dut", "nld"),
    ("gre", "ell"),
    ("rum", "ron"),
    ("cze", "ces"),
    ("slo", "slk"),
    ("per", "fas"),
    ("may", "msa"),
    ("alb", "sqi"),
    ("arm", "hye"),
    ("baq", "eus"),
    ("bur", "mya"),
    ("ice", "isl"),
    ("mac", "mkd"),
];

/// A tag as one lowercase ISO 639-2/T code (`fre` and `fra-CA` are `fra`).
fn language_code(tag: &str) -> String {
    let primary = tag.trim().split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
    BIBLIOGRAPHIC.iter().find(|(b, _)| *b == primary).map(|(_, t)| t.to_string()).unwrap_or(primary)
}

/// The audio stream in `language` (the app sends ISO 639-2, the code the
/// server tags streams with); `None` for no language, `und`, or a
/// language the file does not have, which leaves the server's choice.
fn audio_in(streams: &[MediaStream], language: &str) -> Option<i64> {
    let want = language_code(language);
    if want.is_empty() || want == "und" {
        return None;
    }
    streams.iter().find(|s| s.kind == "audio" && language_code(&s.language) == want).map(|s| s.index)
}

/// The selection for a transcode session: from `start`, at `quality`
/// (empty: the server's choice), with the embedded subtitle `burn`
/// burned in and the `audio` stream (none: the server's choice).
fn selection(start: f64, quality: &str, burn: Option<i64>, audio: Option<i64>) -> TranscodeSelection {
    TranscodeSelection {
        quality: quality.into(),
        audio_stream: audio,
        subtitle_stream: burn,
        subtitle_mode: if burn.is_some() { "burn".into() } else { String::new() },
        start_sec: start.max(0.),
        ..Default::default()
    }
}

/// Starts or joins an HLS session and waits until it can be played.
async fn hls(c: &Client, item_id: &str, sel: TranscodeSelection) -> Result<String, LainError> {
    let mut st = c.start_transcode(item_id, TranscodeSelection { delivery: "hls".into(), ..sel }).await?;
    let deadline = tokio::time::Instant::now() + HLS_READY_WAIT;
    while !(st.playable || st.state == "ready") {
        if matches!(st.state.as_str(), "failed" | "cancelled") {
            let why = st.reasons.join("; ");
            return Err(LainError::Unplayable {
                detail: if why.is_empty() { format!("the transcode {}", st.state) } else { why },
            });
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(LainError::Timeout);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        st = c.transcode_status(item_id, &st.session).await?;
    }
    Ok(st.session)
}

pub(crate) async fn resolve(
    b: Arc<Backend>,
    item_id: String,
    caps: Vec<String>,
    audio_language: String,
) -> Result<PlaySource, LainError> {
    let c = &b.client;
    let resume = b.sync.fetch(&item_id).await.ok().filter(|p| !p.completed && p.position_sec > 3.);
    let start = resume.as_ref().map(|p| p.position_sec).unwrap_or(0.);
    let known = resume.as_ref().map(|p| p.duration_sec).filter(|d| *d > 0.);
    let server = b.server();
    if let Some(path) = b.offline.local_path(&server, &item_id)? {
        b.offline.touch(&server, &item_id)?;
        // Sidecars need the server; offline the file's own tracks remain.
        let subs = match c.health().await {
            Ok(_) => subtitles(c, &item_id, &[]).await,
            Err(_) => Vec::new(),
        };
        return Ok(PlaySource {
            kind: SourceKind::Offline,
            url: path,
            note: "offline copy".into(),
            start_sec: start,
            offset_sec: 0.,
            duration_sec: known,
            subtitles: subs,
            session: String::new(),
            audio: Vec::new(),
            audio_stream: None,
        });
    }
    let plan: PlaybackPlan = c.playback_plan_for(&item_id, CLIENT, Some(caps)).await?;
    if !plan.available {
        let detail = if plan.reason.is_empty() { format!("not playable ({})", plan.mode) } else { plan.reason };
        return Err(LainError::Unplayable { detail });
    }
    let subs = subtitles(c, &item_id, &plan.streams).await;
    let audio = audio_tracks(&plan.streams);
    let duration = plan.duration_sec.or(known);
    if plan.mode == "transcode" {
        let audio_stream = audio_in(&plan.streams, &audio_language);
        let session = hls(c, &item_id, selection(start, "", None, audio_stream)).await?;
        let note = if plan.profile.is_empty() {
            "transcode · HLS".into()
        } else {
            format!("transcode · HLS · {}", plan.profile)
        };
        return Ok(PlaySource {
            kind: SourceKind::Hls,
            url: c.hls_url(&item_id, &session)?,
            note,
            start_sec: start,
            offset_sec: start,
            duration_sec: duration,
            subtitles: subs,
            session,
            audio,
            audio_stream,
        });
    }
    // The file holds every audio stream: the player switches by itself.
    Ok(PlaySource {
        kind: SourceKind::Direct,
        url: c.stream_url(&item_id)?,
        note: "direct play".into(),
        start_sec: start,
        offset_sec: 0.,
        duration_sec: duration,
        subtitles: subs,
        session: String::new(),
        audio,
        audio_stream: None,
    })
}

#[uniffi::export]
impl LainCore {
    /// How to play an item; `caps` is the device's decode claim
    /// (`mp4`, `mkv/h264`, ...; D-058). `audio_language` (ISO 639-2) is
    /// the audio a transcode should carry, the one this title was last
    /// played in: its stream is asked for when the file has it; empty
    /// leaves the server's choice. A direct play ignores it (the player
    /// picks among the file's own streams).
    pub async fn play_source(
        &self,
        item_id: String,
        caps: Vec<String>,
        audio_language: String,
    ) -> Result<PlaySource, LainError> {
        let b = self.backend()?;
        self.run(resolve(b, item_id, caps, audio_language)).await
    }

    /// The ASS fallback (D-247): an HLS session with the embedded
    /// subtitle `stream` burned in by the server, from `start_sec`.
    pub async fn play_burned_in(&self, item_id: String, stream: i64, start_sec: f64) -> Result<PlaySource, LainError> {
        let b = self.backend()?;
        self.run(async move {
            let c = &b.client;
            let session = hls(c, &item_id, selection(start_sec, "", Some(stream), None)).await?;
            Ok(PlaySource {
                kind: SourceKind::Hls,
                url: c.hls_url(&item_id, &session)?,
                note: "transcode · HLS · subtitles burned in".into(),
                start_sec: start_sec.max(0.),
                offset_sec: start_sec.max(0.),
                duration_sec: None,
                subtitles: Vec::new(),
                session,
                audio: Vec::new(),
                audio_stream: None,
            })
        })
        .await
    }

    /// A transcode session from `start_sec`, at `quality` (empty: the
    /// server's choice), optionally with the embedded subtitle
    /// `burn_stream` burned in, playing the file's `audio_stream` (one of
    /// [`PlaySource::audio`]; `None`: the server's choice). A seek outside
    /// the range a session holds, a quality change and another audio
    /// track are all this: a new session whose timeline begins where the
    /// viewer is.
    pub async fn play_transcoded(
        &self,
        item_id: String,
        start_sec: f64,
        quality: String,
        burn_stream: Option<i64>,
        audio_stream: Option<i64>,
    ) -> Result<PlaySource, LainError> {
        let b = self.backend()?;
        self.run(async move {
            let c = &b.client;
            let start = start_sec.max(0.);
            let session = hls(c, &item_id, selection(start, &quality, burn_stream, audio_stream)).await?;
            let mut note = String::from("transcode · HLS");
            if !quality.is_empty() {
                note.push_str(" · ");
                note.push_str(&quality);
            }
            if burn_stream.is_some() {
                note.push_str(" · subtitles burned in");
            }
            Ok(PlaySource {
                kind: SourceKind::Hls,
                url: c.hls_url(&item_id, &session)?,
                note,
                start_sec: start,
                offset_sec: start,
                duration_sec: None,
                subtitles: Vec::new(),
                session,
                audio: Vec::new(),
                audio_stream,
            })
        })
        .await
    }

    /// The qualities the server offers for a transcode.
    pub async fn play_qualities(&self) -> Result<Vec<QualityInfo>, LainError> {
        let b = self.backend()?;
        self.run(async move {
            Ok(b.client
                .playback_options()
                .await?
                .qualities
                .into_iter()
                .map(|q| QualityInfo { name: q.name, max_height: q.max_height, bitrate_kbps: q.bitrate_kbps })
                .collect())
        })
        .await
    }

    /// The item's storyboard: its layout and the picture. The first call
    /// for an item waits while the server makes the picture.
    pub async fn storyboard(&self, item_id: String) -> Result<StoryboardSheet, LainError> {
        let b = self.backend()?;
        self.run(async move {
            let l = b.client.storyboard_layout(&item_id).await?;
            let image = b.client.storyboard_image(&item_id).await?;
            Ok(StoryboardSheet {
                image,
                interval_sec: l.interval_sec,
                count: l.count,
                columns: l.columns,
                tile_width: l.tile_width,
                tile_height: l.tile_height,
            })
        })
        .await
    }

    /// Stops a transcode session this app started (player closed).
    pub async fn stop_transcode(&self, item_id: String, session: String) -> Result<(), LainError> {
        let b = self.backend()?;
        self.run(async move {
            b.client.cancel_transcode(&item_id, &session).await?;
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_typeset() {
        assert_eq!(label("jpn", "Signs", "ASS"), "jpn · Signs · ASS");
        assert_eq!(label("und", "", ""), "Unknown");
        assert!(typeset("ASS") && typeset("ssa") && !typeset("subrip") && !typeset("vtt"));
    }

    fn stream(index: i64, kind: &str, language: &str, default: bool) -> MediaStream {
        MediaStream {
            index,
            kind: kind.into(),
            codec: "aac".into(),
            language: language.into(),
            default,
            ..Default::default()
        }
    }

    /// An anime release: English dub first and default, Japanese second,
    /// a French commentary tagged with the bibliographic code, an
    /// untagged one, and Japanese subtitles.
    fn anime() -> Vec<MediaStream> {
        vec![
            stream(0, "video", "", false),
            stream(1, "audio", "eng", true),
            stream(2, "audio", "jpn", false),
            stream(3, "audio", "fre", false),
            stream(4, "audio", "", false),
            stream(5, "subtitle", "jpn", false),
        ]
    }

    #[test]
    fn the_audio_a_title_remembers_names_its_stream() {
        let s = anime();
        assert_eq!(audio_in(&s, "jpn"), Some(2));
        assert_eq!(audio_in(&s, " JPN "), Some(2));
        // Folded both ways: the app sends the terminology code.
        assert_eq!(audio_in(&s, "fra"), Some(3));
        assert_eq!(audio_in(&s, "fre"), Some(3));
        assert_eq!(audio_in(&s, "fra-CA"), Some(3));
        // A subtitle in the language is not audio; nothing asked or missing is the server's choice.
        assert_eq!(audio_in(&[stream(1, "audio", "eng", true), stream(5, "subtitle", "jpn", false)], "jpn"), None);
        assert_eq!(audio_in(&s, "kor"), None);
        assert_eq!(audio_in(&s, ""), None);
        assert_eq!(audio_in(&s, "und"), None);
    }

    #[test]
    fn audio_tracks_list_the_files_audio_streams() {
        let a = audio_tracks(&anime());
        assert_eq!(a.iter().map(|t| t.stream).collect::<Vec<_>>(), [1, 2, 3, 4]);
        assert_eq!((a[0].label.as_str(), a[0].language.as_str(), a[0].default), ("eng · AAC", "eng", true));
        assert_eq!((a[3].language.as_str(), a[3].default), ("und", false));
    }

    #[test]
    fn a_session_asks_for_its_audio_stream() {
        let s = selection(-4., "720p", None, Some(2));
        assert_eq!((s.audio_stream, s.quality.as_str(), s.start_sec), (Some(2), "720p", 0.));
        assert_eq!((s.subtitle_stream, s.subtitle_mode.as_str()), (None, ""));
        // On the wire: the field the server reads, and nothing when the server chooses.
        assert_eq!(serde_json::to_value(&s).unwrap()["audio_stream"], 2);
        let burned = selection(90., "", Some(5), None);
        assert_eq!((burned.subtitle_stream, burned.subtitle_mode.as_str()), (Some(5), "burn"));
        assert!(serde_json::to_value(&burned).unwrap().get("audio_stream").is_none());
    }
}
