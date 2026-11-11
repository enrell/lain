//! Where a play comes from (D-246), in the desktop's order (D-182): a
//! finished offline copy; else the server's plan for `client=android`
//! with the device's decode claim (D-058): `direct` streams the file,
//! `transcode` starts (or joins) an HLS session at the resume point and
//! waits until its playlist is playable; a plan that is not available
//! is [`LainError::Unplayable`] with the server's reason.

use std::sync::Arc;
use std::time::Duration;

use lain_api::{MediaStream, PlaybackPlan, TranscodeSelection};
use lain_client::Client;

use crate::LainCore;
use crate::core::Backend;
use crate::error::LainError;
use crate::types::{PlaySource, QualityInfo, SourceKind, StoryboardSheet, SubtitleTrack};

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

pub(crate) async fn resolve(b: Arc<Backend>, item_id: String, caps: Vec<String>) -> Result<PlaySource, LainError> {
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
        });
    }
    let plan: PlaybackPlan = c.playback_plan_for(&item_id, CLIENT, Some(caps)).await?;
    if !plan.available {
        let detail = if plan.reason.is_empty() { format!("not playable ({})", plan.mode) } else { plan.reason };
        return Err(LainError::Unplayable { detail });
    }
    let subs = subtitles(c, &item_id, &plan.streams).await;
    let duration = plan.duration_sec.or(known);
    if plan.mode == "transcode" {
        let session = hls(c, &item_id, TranscodeSelection { start_sec: start, ..Default::default() }).await?;
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
        });
    }
    Ok(PlaySource {
        kind: SourceKind::Direct,
        url: c.stream_url(&item_id)?,
        note: "direct play".into(),
        start_sec: start,
        offset_sec: 0.,
        duration_sec: duration,
        subtitles: subs,
        session: String::new(),
    })
}

#[uniffi::export]
impl LainCore {
    /// How to play an item; `caps` is the device's decode claim
    /// (`mp4`, `mkv/h264`, ...; D-058).
    pub async fn play_source(&self, item_id: String, caps: Vec<String>) -> Result<PlaySource, LainError> {
        let b = self.backend()?;
        self.run(resolve(b, item_id, caps)).await
    }

    /// The ASS fallback (D-247): an HLS session with the embedded
    /// subtitle `stream` burned in by the server, from `start_sec`.
    pub async fn play_burned_in(&self, item_id: String, stream: i64, start_sec: f64) -> Result<PlaySource, LainError> {
        let b = self.backend()?;
        self.run(async move {
            let c = &b.client;
            let sel = TranscodeSelection {
                subtitle_stream: Some(stream),
                subtitle_mode: "burn".into(),
                start_sec: start_sec.max(0.),
                ..Default::default()
            };
            let session = hls(c, &item_id, sel).await?;
            Ok(PlaySource {
                kind: SourceKind::Hls,
                url: c.hls_url(&item_id, &session)?,
                note: "transcode · HLS · subtitles burned in".into(),
                start_sec: start_sec.max(0.),
                offset_sec: start_sec.max(0.),
                duration_sec: None,
                subtitles: Vec::new(),
                session,
            })
        })
        .await
    }

    /// A transcode session from `start_sec`, at `quality` (empty: the
    /// server's choice), optionally with the embedded subtitle
    /// `burn_stream` burned in. A seek outside the range a session holds,
    /// and a quality change, are both this: a new session whose timeline
    /// begins where the viewer is.
    pub async fn play_transcoded(
        &self,
        item_id: String,
        start_sec: f64,
        quality: String,
        burn_stream: Option<i64>,
    ) -> Result<PlaySource, LainError> {
        let b = self.backend()?;
        self.run(async move {
            let c = &b.client;
            let start = start_sec.max(0.);
            let sel = TranscodeSelection {
                quality: quality.clone(),
                subtitle_stream: burn_stream,
                subtitle_mode: if burn_stream.is_some() { "burn".into() } else { String::new() },
                start_sec: start,
                ..Default::default()
            };
            let session = hls(c, &item_id, sel).await?;
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
}
