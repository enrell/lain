//! Settings → Transcode (admins, the watch module): the transcode policy
//! staged behind one Ctrl+S bar (D-087) and the running sessions with
//! cancel. Only the policy keys an operator changes day to day are
//! fields here; the rest keep their stored values on save.

use gpui::prelude::*;
use gpui::{Context, div, px};
use lain_client::types::{AdminTranscodes, TranscodeSettings, TranscodeSettingsView};

use crate::app::{Act, LainApp, Load};
use crate::ui::{Notice, Tone};

/// (field id, label, hint) of the editable keys, in display order.
const KEYS: &[(&str, &str, &str)] = &[
    ("tc.delivery", "Default delivery", "hls or progressive"),
    ("tc.hw", "Hardware acceleration", "none, nvenc, qsv, vaapi, videotoolbox, amf"),
    ("tc.preset", "Encoder preset", "x264/x265 preset, e.g. veryfast"),
    ("tc.crf", "CRF", "quality for software encodes (lower is better)"),
    ("tc.max", "Concurrent transcodes", "0 means the server's default"),
    ("tc.remote", "Remote bitrate limit (kbps)", "0 means unlimited"),
    ("tc.segment", "HLS segment seconds", "length of each HLS segment"),
    ("tc.subs", "Subtitle mode", "auto, always, never, burn"),
];

#[derive(Debug, Clone, Default)]
pub struct TranscodeState {
    pub view: Option<Load<TranscodeSettingsView>>,
    pub sessions: Option<Load<AdminTranscodes>>,
    /// The values the fields were filled from (to tell staged edits apart).
    pub loaded: Vec<String>,
    pub tone_mapping: bool,
    pub throttle: bool,
}

fn values(s: &TranscodeSettings) -> Vec<String> {
    vec![
        s.default_delivery.clone(),
        s.hardware_acceleration.clone(),
        s.encoder_preset.clone(),
        s.crf.to_string(),
        s.max_concurrent.to_string(),
        s.remote_bitrate_limit_kbps.to_string(),
        s.hls_segment_seconds.to_string(),
        s.subtitle_mode.clone(),
    ]
}

impl LainApp {
    pub fn load_transcode(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let (c, c2) = (b.client.clone(), b.client.clone());
        self.tc.view = Some(Load::Loading);
        self.spawn_ui(cx, async move { c.transcode_settings().await }, |app, r, _| {
            if let Ok(v) = &r {
                app.fill_transcode(&v.settings);
            }
            app.tc.view = Some(Load::from(r));
        });
        self.spawn_ui(cx, async move { c2.admin_transcodes().await }, |app, r, _| {
            app.tc.sessions = Some(Load::from(r))
        });
    }

    fn fill_transcode(&mut self, s: &TranscodeSettings) {
        let v = values(s);
        for ((id, _, _), val) in KEYS.iter().zip(v.iter()) {
            self.set_text(id, val);
        }
        self.tc.loaded = v;
        self.tc.tone_mapping = s.tone_mapping;
        self.tc.throttle = s.throttle;
    }

    /// Whether the fields differ from what was loaded.
    pub fn transcode_dirty(&self) -> bool {
        let Some(Load::Ready(v)) = &self.tc.view else { return false };
        let now: Vec<String> = KEYS.iter().map(|(id, _, _)| self.text(id)).collect();
        now != self.tc.loaded
            || self.tc.tone_mapping != v.settings.tone_mapping
            || self.tc.throttle != v.settings.throttle
    }

    pub fn save_transcode(&mut self, cx: &mut Context<Self>) {
        let Some(Load::Ready(v)) = self.tc.view.clone() else { return };
        let Some(b) = self.backend.clone() else { return };
        let mut s = v.settings.clone();
        let num = |id: &'static str, app: &LainApp| -> Result<i64, String> {
            app.text(id).trim().parse::<i64>().map_err(|_| format!("{id}: a whole number"))
        };
        let parsed = (|| -> Result<(), String> {
            s.default_delivery = self.text("tc.delivery").trim().to_string();
            s.hardware_acceleration = self.text("tc.hw").trim().to_string();
            s.encoder_preset = self.text("tc.preset").trim().to_string();
            s.crf = num("tc.crf", self)?;
            s.max_concurrent = num("tc.max", self)?;
            s.remote_bitrate_limit_kbps = num("tc.remote", self)?;
            s.hls_segment_seconds = num("tc.segment", self)?;
            s.subtitle_mode = self.text("tc.subs").trim().to_string();
            Ok(())
        })();
        if let Err(e) = parsed {
            self.toast(e, true);
            return;
        }
        s.tone_mapping = self.tc.tone_mapping;
        s.throttle = self.tc.throttle;
        let c = b.client.clone();
        self.spawn_ui(cx, async move { c.update_transcode_settings(s).await }, |app, r, _| {
            if app.report(&r, "Transcode settings saved") {
                if let Ok(v) = &r {
                    app.fill_transcode(&v.settings);
                }
                app.tc.view = Some(Load::from(r));
            }
        });
    }

    pub fn cancel_session(&mut self, session: String, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let c = b.client.clone();
        self.spawn_ui(cx, async move { c.admin_cancel_transcode(&session).await }, |app, r, cx| {
            app.report(&r, "Transcode cancelled");
            app.load_transcode(cx);
        });
    }

    /// The transcode part of Settings → Transcode & integrations.
    pub fn transcode_section(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let th = self.theme.clone();
        let pal = self.pal.clone();
        let mut col = div().flex().flex_col();
        match self.tc.view.clone() {
            Some(Load::Ready(v)) => {
                let caps = &v.capabilities;
                let hw: Vec<String> = caps.hardware.iter().filter(|(_, ok)| **ok).map(|(k, _)| k.clone()).collect();
                col = col.child(div().py(px(th.sp(2))).text_color(pal.muted).child(format!(
                    "ffmpeg {} · hardware: {} · tone mapping {}",
                    if caps.ffmpeg.is_empty() { "not found" } else { &caps.ffmpeg },
                    if hw.is_empty() { "none".to_string() } else { hw.join(", ") },
                    if caps.tone_mapping { "available" } else { "unavailable" }
                )));
                for (id, label, hint) in KEYS {
                    let f = self.field(id, "", false, Act::SaveTranscode, cx);
                    col = col.child(self.row(label, hint, div().w(px(260.)).child(f)));
                }
                let tm = self.btn(
                    "tc.tonemap",
                    if self.tc.tone_mapping { "On" } else { "Off" },
                    None,
                    Tone::Ghost,
                    Act::ToggleTranscode("tone"),
                    cx,
                );
                let thr = self.btn(
                    "tc.throttle",
                    if self.tc.throttle { "On" } else { "Off" },
                    None,
                    Tone::Ghost,
                    Act::ToggleTranscode("throttle"),
                    cx,
                );
                col = col
                    .child(self.row("Tone mapping", "HDR to SDR when a client cannot show HDR", tm))
                    .child(self.row("Throttle", "Pause encoding when far ahead of the player", thr));
                if self.transcode_dirty() {
                    let save = self.btn("tc.save", "Save", Some("Ctrl S"), Tone::Primary, Act::SaveTranscode, cx);
                    let discard = self.btn("tc.discard", "Discard", None, Tone::Quiet, Act::DiscardTranscode, cx);
                    col = col.child(
                        self.panel()
                            .mt(px(th.sp(3)))
                            .px(px(th.sp(3)))
                            .py(px(th.sp(2)))
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(div().text_color(pal.warn).child("Unsaved server settings"))
                            .child(div().flex().gap(px(th.sp(1))).child(discard).child(save)),
                    );
                }
            }
            Some(Load::Failed(e)) => col = col.child(self.error_notice("Transcode settings", &e)),
            _ => col = col.child(self.notice(Notice::Loading, "Loading…", "")),
        }
        col = col.child(div().pt(px(th.sp(5))).child(self.label("running transcodes")));
        match self.tc.sessions.clone() {
            Some(Load::Ready(list)) if list.sessions.is_empty() => {
                col = col.child(div().py(px(th.sp(2))).text_color(pal.muted).child("No transcode is running."));
            }
            Some(Load::Ready(list)) => {
                for (i, s) in list.sessions.iter().enumerate() {
                    let st = &s.status;
                    let b = self.btn(
                        &format!("tc.cancel.{i}"),
                        "Cancel",
                        None,
                        Tone::Quiet,
                        Act::CancelTranscode(st.session.clone()),
                        cx,
                    );
                    let what = format!(
                        "{} · {} · {}{} · {:.0}%{}",
                        st.state,
                        st.delivery,
                        st.encoder,
                        if st.hardware.is_empty() { String::new() } else { format!(" ({})", st.hardware) },
                        st.progress * 100.,
                        if s.user_id.is_empty() { String::new() } else { format!(" · user {}", s.user_id) }
                    );
                    col = col.child(self.row(&st.session, &what, b));
                }
            }
            Some(Load::Failed(e)) => col = col.child(div().text_color(pal.muted).child(crate::backend::reason(&e))),
            _ => {}
        }
        col
    }
}
