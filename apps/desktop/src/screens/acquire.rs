//! Acquire (admins, the acquire module): the server's torrent grabs and
//! HTTP downloads with live state, pause/resume/remove, and the wanted
//! overview of monitored titles. Searching indexers and editing profiles
//! stay in the web UI for now.

use gpui::prelude::*;
use gpui::{Context, FontWeight, Window, div, px};
use lain_client::types::acquire::{GrabList, WantedOverview};
use lain_client::types::downloads::DownloadsView;

use crate::app::{Act, LainApp, Load};
use crate::ui::{Notice, Tone, fmt_bytes};

#[derive(Debug, Clone, Default)]
pub struct AcquireState {
    pub grabs: Option<Load<GrabList>>,
    pub downloads: Option<Load<DownloadsView>>,
    pub wanted: Option<Load<WantedOverview>>,
}

impl LainApp {
    pub fn load_acquire(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let (c1, c2, c3) = (b.client.clone(), b.client.clone(), b.client.clone());
        if self.acq.grabs.is_none() {
            self.acq.grabs = Some(Load::Loading);
        }
        self.spawn_ui(cx, async move { c1.grabs().await }, |app, r, _| app.acq.grabs = Some(Load::from(r)));
        self.spawn_ui(cx, async move { c2.downloads().await }, |app, r, _| app.acq.downloads = Some(Load::from(r)));
        self.spawn_ui(cx, async move { c3.wanted().await }, |app, r, _| app.acq.wanted = Some(Load::from(r)));
    }

    /// `kind` is `grab` or `download`; `action` pause, resume, cancel or remove.
    pub fn acquire_op(&mut self, kind: &'static str, id: String, action: &'static str, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let c = b.client.clone();
        self.spawn_ui(
            cx,
            async move {
                match (kind, action) {
                    ("grab", "remove") => c.remove_grab(&id, false).await,
                    ("grab", a) => c.grab_action(&id, a).await.map(|_| ()),
                    (_, "remove") => c.remove_download(&id).await,
                    (_, a) => c.download_action(&id, a).await.map(|_| ()),
                }
            },
            move |app, r, cx| {
                app.report(&r, "");
                app.load_acquire(cx);
            },
        );
    }

    pub fn render_acquire(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let th = self.theme.clone();
        let pal = self.pal.clone();
        let margin = th.sp(6);
        let mut col = div().flex().flex_col().max_w(px(1100.)).child(
            div()
                .font_family(self.fonts.display.clone())
                .text_size(px(th.ts(4)))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Acquire"),
        );
        if !self.is_admin() {
            col = col.child(self.notice(
                Notice::Disabled,
                "Acquire is for admins",
                "An admin manages torrents, downloads and wanted titles.",
            ));
            return div().id("acquire").size_full().pt(px(84.)).px(px(margin)).child(col);
        }
        // ---- torrents
        col = col.child(div().pt(px(th.sp(4))).child(self.label("torrents")));
        match self.acq.grabs.clone() {
            Some(Load::Ready(g)) if g.grabs.is_empty() => {
                col = col.child(
                    div()
                        .py(px(th.sp(2)))
                        .text_color(pal.muted)
                        .child("No torrent. Grab releases from Wanted, or in the web UI."),
                );
            }
            Some(Load::Ready(g)) => {
                for (i, x) in g.grabs.iter().enumerate() {
                    let frac = if x.size > 0 { (x.downloaded as f32 / x.size as f32).clamp(0., 1.) } else { 0. };
                    let running = !matches!(x.state.as_str(), "paused" | "done" | "seeding" | "imported" | "failed");
                    let toggle = if x.state == "paused" {
                        self.btn(
                            &format!("acq.g.{i}.r"),
                            "Resume",
                            None,
                            Tone::Ghost,
                            Act::AcquireOp("grab", x.id.clone(), "resume"),
                            cx,
                        )
                    } else {
                        self.btn(
                            &format!("acq.g.{i}.p"),
                            "Pause",
                            None,
                            Tone::Quiet,
                            Act::AcquireOp("grab", x.id.clone(), "pause"),
                            cx,
                        )
                    };
                    let rm = self.btn(
                        &format!("acq.g.{i}.x"),
                        "Remove",
                        None,
                        Tone::Quiet,
                        Act::AcquireOp("grab", x.id.clone(), "remove"),
                        cx,
                    );
                    let hint = format!(
                        "{} · {:.0}% of {} · {} peers{}",
                        x.state,
                        frac * 100.,
                        fmt_bytes(x.size.max(0) as u64),
                        x.peers,
                        if running && x.down_rate > 0 {
                            format!(" · {}/s", fmt_bytes(x.down_rate as u64))
                        } else {
                            String::new()
                        }
                    );
                    col = col
                        .child(self.row(&x.title, &hint, div().flex().gap(px(th.sp(1))).child(toggle).child(rm)))
                        .child(
                            div()
                                .h(px(2.))
                                .w_full()
                                .bg(pal.text.opacity(0.06))
                                .child(div().h_full().w(gpui::relative(frac)).bg(pal.accent)),
                        );
                }
            }
            Some(Load::Failed(e)) => col = col.child(self.error_notice("Torrents", &e)),
            _ => col = col.child(self.notice(Notice::Loading, "Loading…", "")),
        }
        // ---- downloads
        col = col.child(div().pt(px(th.sp(5))).child(self.label("downloads")));
        match self.acq.downloads.clone() {
            Some(Load::Ready(d)) if d.jobs.is_empty() => {
                col = col.child(div().py(px(th.sp(2))).text_color(pal.muted).child("No HTTP download."));
            }
            Some(Load::Ready(d)) => {
                for (i, j) in d.jobs.iter().enumerate() {
                    let frac = if j.total > 0 { (j.bytes as f32 / j.total as f32).clamp(0., 1.) } else { 0. };
                    let (label, act) = match j.state.as_str() {
                        "paused" => ("Resume", Act::AcquireOp("download", j.id.clone(), "resume")),
                        "done" | "failed" | "canceled" => {
                            ("Remove", Act::AcquireOp("download", j.id.clone(), "remove"))
                        }
                        _ => ("Pause", Act::AcquireOp("download", j.id.clone(), "pause")),
                    };
                    let b = self.btn(&format!("acq.d.{i}"), label, None, Tone::Quiet, act, cx);
                    col = col.child(self.row(
                        &j.name,
                        &format!("{} · {:.0}% of {}", j.state, frac * 100., fmt_bytes(j.total.max(0) as u64)),
                        b,
                    ));
                }
            }
            Some(Load::Failed(e)) => {
                col = col.child(div().py(px(th.sp(2))).text_color(pal.muted).child(crate::backend::reason(&e)))
            }
            _ => {}
        }
        // ---- wanted
        col = col.child(div().pt(px(th.sp(5))).child(self.label("wanted")));
        match self.acq.wanted.clone() {
            Some(Load::Ready(w)) if w.titles.is_empty() => {
                col = col.child(div().py(px(th.sp(2))).text_color(pal.muted).child("No monitored title."));
            }
            Some(Load::Ready(w)) => {
                for row in &w.titles {
                    col = col.child(self.row(
                        &row.monitored.title,
                        &format!("{} missing · {} upgrades", row.missing_count, row.upgrade_count),
                        self.pill(&row.monitored.kind, pal.muted),
                    ));
                }
            }
            Some(Load::Failed(e)) => {
                col = col.child(div().py(px(th.sp(2))).text_color(pal.muted).child(crate::backend::reason(&e)))
            }
            _ => {}
        }
        div()
            .id("acquire")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .pt(px(84.))
            .px(px(margin))
            .pb(px(th.sp(6)))
            .child(col)
    }
}
