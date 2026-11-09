//! Offline copies (lain-client `OfflineStore`, D-094's limits): start,
//! pause, resume and remove copies; the Downloads screen lists them with
//! live progress and plays a finished copy without the server.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use gpui::prelude::*;
use gpui::{Context, FontWeight, Window, div, px};
use lain_client::{FetchObserver, OfflineEntry, OfflineState};

use crate::app::{Act, LainApp};
use crate::ui::{Notice, Tone, fmt_bytes};

#[derive(Default)]
pub struct DlState {
    pub entries: Vec<OfflineEntry>,
    /// Live transfer progress: item id → (done, total).
    pub live: Arc<Mutex<HashMap<String, (u64, u64)>>>,
    pub running: Vec<String>,
}

struct Observer {
    id: String,
    live: Arc<Mutex<HashMap<String, (u64, u64)>>>,
}

impl FetchObserver for Observer {
    fn progress(&self, done: u64, total: u64) {
        self.live.lock().unwrap_or_else(|e| e.into_inner()).insert(self.id.clone(), (done, total));
    }
    fn evicted(&self, entry: &OfflineEntry) {
        eprintln!("offline: evicted {} to make room", entry.title);
    }
}

impl LainApp {
    pub fn download(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let Some(store) = b.offline.clone() else {
            self.toast(format!("Offline copies are off: {}", b.offline_error.clone().unwrap_or_default()), true);
            return;
        };
        let Some(item) = self.by_id.get(id).map(|&i| self.catalog[i].clone()) else { return };
        if self.dl.running.iter().any(|r| r == id) {
            return;
        }
        let server = b.client.server();
        if let Err(e) = store.get(&server, id).and_then(|e| match e {
            Some(_) => store.resume(&server, id).map(|_| ()),
            None => store.add(&server, &item).map(|_| ()),
        }) {
            self.toast(crate::backend::reason(&e), true);
            return;
        }
        self.dl.running.push(id.to_string());
        let obs: Arc<dyn FetchObserver> = Arc::new(Observer { id: id.to_string(), live: self.dl.live.clone() });
        let c = b.client.clone();
        let id2 = id.to_string();
        self.toast(format!("Downloading {}", item.title), false);
        self.spawn_ui(cx, async move { store.fetch(&c, &id2, Some(obs)).await }, move |app, r, _| {
            let id = item.id.clone();
            app.dl.running.retain(|x| *x != id);
            app.report(&r, &format!("{} is on this device", item.title));
            app.refresh_offline_entries();
            app.reload_dl_entries();
        });
        self.refresh_offline_entries();
        self.reload_dl_entries();
    }

    pub fn offline_op(&mut self, id: &str, op: &str, _cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let Some(store) = b.offline.clone() else { return };
        let server = b.client.server();
        let r = match op {
            "pause" => store.pause(&server, id).map(|_| ()),
            _ => store.remove(&server, id, true).map(|_| ()),
        };
        self.report(&r, if op == "pause" { "Paused" } else { "Offline copy removed" });
        self.refresh_offline_entries();
        self.reload_dl_entries();
    }

    pub fn reload_dl_entries(&mut self) {
        let Some(b) = self.backend.clone() else { return };
        if let Some(o) = &b.offline {
            self.dl.entries = o.list().unwrap_or_default();
            self.dl.entries.sort_by(|a, b| b.added_at_ms.cmp(&a.added_at_ms));
        }
    }

    pub fn downloads_tick(&mut self, cx: &mut Context<Self>) {
        if !self.dl.running.is_empty() {
            cx.notify();
        }
    }

    /// A finished offline copy of an item, if this device has one.
    pub fn offline_file(&self, id: &str) -> Option<String> {
        let b = self.backend.as_ref()?;
        let o = b.offline.as_ref()?;
        let p = o.local_path(&b.client.server(), id).ok().flatten()?;
        let _ = o.touch(&b.client.server(), id);
        Some(p)
    }

    pub fn render_downloads(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        if self.dl.entries.is_empty() {
            self.reload_dl_entries();
        }
        let th = self.theme.clone();
        let pal = self.pal.clone();
        let margin = th.sp(6);
        let mut root = div().size_full().flex().flex_col().pt(px(84.)).px(px(margin)).gap(px(th.sp(3)));
        let used: u64 = self.dl.entries.iter().map(|e| e.bytes).sum();
        let cap = self.config.offline_max_gib << 30;
        root = root
            .child(
                div()
                    .font_family(self.fonts.display.clone())
                    .text_size(px(th.ts(4)))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Downloads"),
            )
            .child(self.label(format!(
                "{} of {} used · offline copies play without the server",
                fmt_bytes(used),
                fmt_bytes(cap)
            )));
        let Some(b) = self.backend.clone() else {
            return root.child(self.notice(
                Notice::Offline,
                "No server yet",
                "Connect to a server to download copies.",
            ));
        };
        if b.offline.is_none() {
            return root.child(self.notice(
                Notice::Error,
                "Offline copies are off",
                &b.offline_error.clone().unwrap_or_default(),
            ));
        }
        if self.dl.entries.is_empty() {
            return root.child(self.notice(
                Notice::Empty,
                "No offline copies",
                "Open a title and press D to keep an episode on this device.",
            ));
        }
        let live = self.dl.live.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let entries = self.dl.entries.clone();
        let mut list =
            div().id("dl.list").flex().flex_col().gap(px(th.sp(1))).overflow_y_scroll().track_scroll(&self.scroll);
        for (i, e) in entries.iter().enumerate() {
            self.cur_row = i;
            let (done, total) = live.get(&e.item_id).copied().unwrap_or((e.bytes, e.size));
            let frac = if total > 0 { (done as f32 / total as f32).clamp(0., 1.) } else { 0. };
            let running = self.dl.running.contains(&e.item_id);
            let (state, color) = match e.state {
                OfflineState::Done => ("on device".to_string(), pal.ok),
                OfflineState::Queued if running => (format!("{:.0}%", frac * 100.), pal.accent),
                OfflineState::Queued => ("queued".into(), pal.muted),
                OfflineState::Paused => ("paused".into(), pal.warn),
                OfflineState::Failed => {
                    (format!("failed · {}", if e.code.is_empty() { &e.error } else { &e.code }), pal.danger)
                }
            };
            let mut actions = div().flex().gap(px(th.sp(1)));
            let base = format!("dl.{i}");
            if e.state == OfflineState::Done {
                actions = actions.child(self.btn(
                    &format!("{base}.play"),
                    "Play",
                    None,
                    Tone::Ghost,
                    Act::Play(e.item_id.clone()),
                    cx,
                ));
            } else if running {
                actions = actions.child(self.btn(
                    &format!("{base}.pause"),
                    "Pause",
                    None,
                    Tone::Quiet,
                    Act::PauseDownload(e.item_id.clone()),
                    cx,
                ));
            } else {
                actions = actions.child(self.btn(
                    &format!("{base}.resume"),
                    "Resume",
                    None,
                    Tone::Ghost,
                    Act::ResumeDownload(e.item_id.clone()),
                    cx,
                ));
            }
            actions = actions.child(self.btn(
                &format!("{base}.rm"),
                "Remove",
                None,
                Tone::Quiet,
                Act::RemoveDownload(e.item_id.clone()),
                cx,
            ));
            for id in [format!("{base}.play"), format!("{base}.pause"), format!("{base}.resume"), format!("{base}.rm")]
            {
                self.row_of.insert(id.into(), i);
            }
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(th.sp(3)))
                    .py(px(th.sp(2)))
                    .border_b_1()
                    .border_color(pal.line)
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(div().font_weight(FontWeight::MEDIUM).child(format!(
                                "{} · {}",
                                e.title,
                                crate::screens::home::episode_label(e.season, e.episode)
                            )))
                            .child(
                                div().h(px(3.)).w_full().rounded_full().bg(pal.text.opacity(0.08)).child(
                                    div()
                                        .h_full()
                                        .w(gpui::relative(if e.state == OfflineState::Done { 1. } else { frac }))
                                        .bg(color),
                                ),
                            ),
                    )
                    .child(div().w(px(160.)).text_size(px(th.ts(-1))).text_color(color).child(state))
                    .child(
                        div()
                            .w(px(90.))
                            .text_size(px(th.ts(-1)))
                            .text_color(pal.muted)
                            .child(fmt_bytes(total.max(e.size))),
                    )
                    .child(actions),
            );
        }
        root.child(list)
    }
}
