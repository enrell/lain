//! The player. On Windows mpv draws into a child window under GPUI's
//! transparent surface (`mpv.rs`) and this view draws only the controls
//! over it; while they are hidden it does not redraw at all. On Linux
//! mpv runs in its own window (`mpv_ipc.rs`) and this view is its remote.
//!
//! The source comes from the server's playback plan
//! (`/api/items/{id}/playback`): direct play through the Range-capable
//! stream, or HLS/progressive transcode when the plan says so. An offline
//! copy plays first; a server that cannot stream yet (501 not-ported)
//! falls back to the file through the shared-storage path map. Progress
//! goes through `ProgressSync` every 5 s, on close and at the end; the
//! next episode starts after a short countdown.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    Bounds, ClickEvent, Context, FontWeight, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Window, canvas, div, linear_color_stop, linear_gradient, px,
};
use lain_client::ClientError;

use crate::app::{Anim, LainApp};
use crate::backend;
use crate::engine::{Engine, PlayerState};
use crate::screens::home::episode_label;
use crate::ui::fmt_time;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    Audio,
    Subs,
}

pub struct PlayerSession {
    pub id: u64,
    pub item_id: String,
    pub title: String,
    pub label: String,
    pub source_note: String,
    #[cfg(windows)]
    pub host: Option<crate::mpv::VideoHost>,
    pub engine: Option<Arc<dyn Engine>>,
    pub pending: bool,
    pub error: Option<String>,
    pub st: PlayerState,
    last_move: Instant,
    visible: bool,
    fade: Anim,
    pub menu: Option<Menu>,
    seek_drag: Option<f32>,
    last_scrub: Instant,
    vol_drag: bool,
    seek_bounds: Rc<Cell<Bounds<Pixels>>>,
    vol_bounds: Rc<Cell<Bounds<Pixels>>>,
    last_save: Instant,
    saved_pos: f64,
    pub force_controls: bool,
    pub next_id: Option<String>,
    pub next_at: Option<Instant>,
    ended: bool,
    logged_ao: bool,
}

impl PlayerSession {
    fn wants_controls(&self, autohide_ms: f32) -> bool {
        self.force_controls
            || self.is_external()
            || self.st.paused
            || self.menu.is_some()
            || self.seek_drag.is_some()
            || self.vol_drag
            || !self.st.has_video
            || self.next_at.is_some()
            || self.last_move.elapsed().as_secs_f32() * 1000. < autohide_ms
    }

    fn cmd(&self, args: &[&str]) {
        if let Some(e) = &self.engine {
            e.command(args);
        }
    }

    pub fn is_external(&self) -> bool {
        match &self.engine {
            Some(e) => e.external(),
            None => !cfg!(windows),
        }
    }

    pub fn engine_state(&self) -> PlayerState {
        self.engine.as_ref().map(|e| e.snapshot()).unwrap_or_default()
    }
}

/// What the Playback settings row says about the engine.
pub fn engine_status() -> String {
    #[cfg(windows)]
    {
        match crate::mpv::available() {
            Ok(()) => "libmpv embedded in the window (vo=gpu-next, d3d11, hwdec auto-safe)".into(),
            Err(e) => format!("unavailable: {e}"),
        }
    }
    #[cfg(not(windows))]
    {
        format!("external mpv over JSON IPC ({})", std::env::var("LAIN_MPV").unwrap_or_else(|_| "mpv on PATH".into()))
    }
}

/// Where the bytes come from.
struct Resolved {
    url: String,
    note: String,
    start: f64,
    subs: Vec<(String, String, String)>,
}

impl LainApp {
    /// The next item of the same title, in episode order.
    fn next_item(&self, id: &str) -> Option<String> {
        let s = *self.series_of.get(id)?;
        let items = &self.series[s].items;
        let pos = items.iter().position(|&i| self.catalog[i].id == id)?;
        items.get(pos + 1).map(|&i| self.catalog[i].id.clone())
    }

    pub fn open_player(&mut self, id: &str, start_at: Option<f64>, cx: &mut Context<Self>) {
        if self.player.is_some() {
            return;
        }
        self.ext_event(crate::ext::Event::PlaybackStarted(id.to_string()));
        #[cfg(windows)]
        if self.hwnd == 0 {
            return;
        }
        let Some(b) = self.backend.clone() else { return };
        let item = self.by_id.get(id).map(|&i| self.catalog[i].clone());
        let title = item.as_ref().map(|i| i.title.clone()).unwrap_or_else(|| id.to_string());
        let label = item.as_ref().map(|i| episode_label(i.season, i.episode)).unwrap_or_default();
        #[cfg(windows)]
        let host = match crate::mpv::VideoHost::new(self.hwnd) {
            Ok(h) => Some(h),
            Err(e) => {
                self.toast(format!("video host: {e}"), true);
                return;
            }
        };
        #[cfg(windows)]
        let wid = host.as_ref().map(|h| h.wid()).unwrap_or(0);
        let sid = self.next_session;
        self.next_session += 1;
        let force = self.demo.enabled() && self.demo.force_controls();
        self.player = Some(PlayerSession {
            id: sid,
            item_id: id.to_string(),
            title: title.clone(),
            label: label.clone(),
            source_note: String::new(),
            #[cfg(windows)]
            host,
            engine: None,
            pending: true,
            error: None,
            st: PlayerState::default(),
            last_move: Instant::now(),
            visible: true,
            fade: Anim::still(1.),
            menu: None,
            seek_drag: None,
            last_scrub: Instant::now(),
            vol_drag: false,
            seek_bounds: Rc::new(Cell::new(Bounds::default())),
            vol_bounds: Rc::new(Cell::new(Bounds::default())),
            last_save: Instant::now(),
            saved_pos: 0.,
            force_controls: force,
            next_id: self.next_item(id),
            next_at: None,
            ended: false,
            logged_ao: false,
        });

        // Local sources are checked here (cheap file checks); the plan,
        // progress and sidecars are asked of the server off the UI thread.
        let offline = self.offline_file(id);
        let shared =
            item.as_ref().and_then(|i| self.config.local_file(&i.file_path)).map(|p| p.to_string_lossy().into_owned());
        let c = b.client.clone();
        let sync = b.sync.clone();
        let want = id.to_string();
        let resolve = async move {
            let p = match &sync {
                Some(s) => s.fetch(&want).await.ok(),
                None => c.progress(&want).await.ok(),
            };
            let start = match start_at {
                Some(t) => t,
                None => p.filter(|p| !p.completed && p.position_sec > 3.).map(|p| p.position_sec).unwrap_or(0.),
            };
            let subs: Vec<(String, String, String)> = match c.sidecars(&want).await {
                Ok(list) => list
                    .into_iter()
                    .filter_map(|s| c.sidecar_url(&want, s.index).ok().map(|u| (u, s.name, s.language)))
                    .collect(),
                Err(_) => Vec::new(),
            };
            if let Some(path) = offline {
                return Ok(Resolved { url: path, note: "offline copy".into(), start, subs });
            }
            match c.playback_plan(&want, "desktop").await {
                Ok(plan) if !plan.available => Err(if plan.reason.is_empty() { format!("not playable ({})", plan.mode) } else { plan.reason }),
                Ok(plan) => {
                    let (url, note) = if plan.mode == "transcode" {
                        // mpv decodes nearly everything, so this is rare: the
                        // server (or this account's policy) asked for it.
                        let session = if plan.session.is_empty() {
                            match c.start_transcode(&want, Default::default()).await {
                                Ok(st) => st.session,
                                Err(e) => return Err(backend::reason(&e)),
                            }
                        } else {
                            plan.session.clone()
                        };
                        let profile = if plan.profile.is_empty() { String::new() } else { format!(" · {}", plan.profile) };
                        (c.hls_url(&want, &session), format!("transcode · HLS{profile}"))
                    } else {
                        (c.stream_url(&want), "direct play".to_string())
                    };
                    url.map(|u| Resolved { url: u, note, start, subs }).map_err(|e| backend::reason(&e))
                }
                Err(e @ (ClientError::NotPorted { .. } | ClientError::ModuleDisabled { .. })) => match shared {
                    Some(path) => Ok(Resolved { url: path, note: "shared storage (the server cannot stream yet)".into(), start, subs }),
                    None => Err(match e {
                        ClientError::ModuleDisabled { .. } => "The watch module is off on this server.".to_string(),
                        _ => "Playback is not available on this server yet (its stream route is not ported), and the file is not reachable from this device. Set a shared-storage path map in Settings → Playback.".to_string(),
                    }),
                },
                Err(e) => match shared {
                    Some(path) if e.is_offline() => Ok(Resolved { url: path, note: "shared storage (server offline)".into(), start, subs }),
                    _ => Err(backend::reason(&e)),
                },
            }
        };
        let task = b.spawn(resolve);
        let mute = self.args.mute;
        let slang = self.me.as_ref().map(|u| u.preferred_language.clone()).unwrap_or_default();
        let title_full = format!("{title} · {label}");
        cx.spawn(async move |this, cx| {
            let resolved = match task.await {
                Ok(r) => r,
                Err(e) => Err(e.to_string()),
            };
            let (resolved, note) = match resolved {
                Ok(r) => {
                    let note = r.note.clone();
                    (r, note)
                }
                Err(e) => {
                    let _ = this.update(cx, |app, cx| app.player_started(sid, Err(e), String::new(), cx));
                    return;
                }
            };
            // mpv_initialize creates mpv's window, which talks to our host
            // window: never block the UI thread on it.
            let init = cx
                .background_executor()
                .spawn(async move {
                    start_engine(
                        resolved,
                        mute,
                        &slang,
                        &title_full,
                        #[cfg(windows)]
                        wid,
                    )
                })
                .await;
            let _ = this.update(cx, |app, cx| app.player_started(sid, init, note, cx));
        })
        .detach();
        cx.notify();
    }

    fn player_started(&mut self, id: u64, init: Result<Arc<dyn Engine>, String>, note: String, cx: &mut Context<Self>) {
        if let Err(e) = &init {
            eprintln!("player: session {id}: {e}");
        }
        match self.player.as_mut() {
            Some(p) if p.id == id => {
                p.pending = false;
                p.source_note = note;
                match init {
                    Ok(m) => {
                        p.saved_pos = m.snapshot().time;
                        p.engine = Some(m);
                    }
                    Err(e) => p.error = Some(e),
                }
                cx.notify();
            }
            _ => {
                // Closed while starting.
                if let Some(pos) = self.orphans.iter().position(|o| o.id == id) {
                    let mut s = self.orphans.remove(pos);
                    s.engine = init.ok();
                    self.teardown(s, cx);
                }
            }
        }
    }

    fn teardown(&mut self, mut s: PlayerSession, cx: &mut Context<Self>) {
        #[cfg(windows)]
        if let Some(h) = &s.host {
            h.hide();
        }
        let engine = s.engine.take();
        #[cfg(windows)]
        let host = s.host.take();
        // Dropping mpv joins its threads, and mpv's window is a child of
        // the host: keep the UI thread pumping messages meanwhile.
        let gone = cx.background_executor().spawn(async move { drop(engine) });
        cx.spawn(async move |_, _| {
            gone.await;
            #[cfg(windows)]
            drop(host);
        })
        .detach();
    }

    pub fn close_player(&mut self, cx: &mut Context<Self>) {
        let Some(s) = self.player.take() else { return };
        self.ext_event(crate::ext::Event::PlaybackStopped(s.item_id.clone()));
        let st = s.engine_state();
        if st.duration > 0.
            && let Some(b) = self.backend.clone()
        {
            let done = st.eof || st.time / st.duration > 0.92;
            let id = s.item_id.clone();
            self.progress.insert(
                id.clone(),
                lain_client::types::Progress {
                    item_id: id.clone(),
                    position_sec: st.time,
                    duration_sec: st.duration,
                    completed: done,
                    ..Default::default()
                },
            );
            let b2 = b.clone();
            b.spawn(async move {
                if let Err(e) = backend::save_progress(b2, id, st.time, st.duration, done).await {
                    eprintln!("progress: {e}");
                }
            });
        }
        if s.pending {
            #[cfg(windows)]
            if let Some(h) = &s.host {
                h.hide();
            }
            self.orphans.push(s);
        } else {
            self.teardown(s, cx);
        }
        let me = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(Duration::from_millis(400)).await;
            let _ = me.update(cx, |app, cx| {
                app.refresh_continues(cx);
                app.item.dirty = true;
            });
        })
        .detach();
        cx.notify();
    }

    pub fn player_next(&mut self, cx: &mut Context<Self>) {
        let Some(next) = self.player.as_ref().and_then(|p| p.next_id.clone()) else { return };
        self.close_player(cx);
        // Let the old engine go before the new host window is made.
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(150)).await;
            let _ = this.update(cx, |app, cx| app.open_player(&next, Some(0.), cx));
        })
        .detach();
    }

    pub fn player_tick(&mut self, cx: &mut Context<Self>) {
        let autohide = self.theme.player.autohide_ms;
        let (fast, normal) = (self.theme.motion.fast_ms, self.theme.motion.normal_ms);
        let th = self.theme.clone();
        let autoplay = self.config.autoplay_next;
        let Some(p) = self.player.as_mut() else { return };
        #[cfg(windows)]
        if let Some(h) = p.host.as_mut() {
            h.sync();
        }
        let Some(m) = p.engine.clone() else { return };
        let st = m.snapshot();
        if st.has_video && !p.logged_ao {
            p.logged_ao = true;
            eprintln!(
                "mpv: current-ao={} current-vo={} hwdec={} source={}",
                m.get_string("current-ao").unwrap_or_else(|| "none".into()),
                m.get_string("current-vo").unwrap_or_default(),
                m.get_string("hwdec-current").unwrap_or_default(),
                p.source_note
            );
        }
        if st.exited {
            self.close_player(cx);
            return;
        }
        let structural = st.has_video != p.st.has_video
            || st.paused != p.st.paused
            || st.error != p.st.error
            || st.tracks != p.st.tracks
            || st.eof != p.st.eof;
        let changed = st != p.st;
        p.st = st;
        let want = p.wants_controls(autohide);
        let mut redraw = structural || (changed && (p.visible || p.fade.running()));
        if want != p.visible {
            p.visible = want;
            p.fade.go(if want { 1. } else { 0. }, if want { fast } else { normal }, &th);
            redraw = true;
        }
        // The end: save as watched once, then count down to the next one.
        if p.st.eof && !p.ended {
            p.ended = true;
            if autoplay && p.next_id.is_some() {
                p.next_at = Some(Instant::now() + Duration::from_secs(5));
            }
            if let Some(b) = self.backend.clone() {
                let (id, d) = (p.item_id.clone(), p.st.duration);
                let b2 = b.clone();
                b.spawn(async move {
                    let _ = backend::save_progress(b2, id, d, d, true).await;
                });
            }
            redraw = true;
        }
        if p.next_at.is_some() {
            redraw = true;
        }
        if p.next_at.is_some_and(|t| Instant::now() >= t) {
            self.player_next(cx);
            return;
        }
        // Progress: every 5 s of playback.
        if p.last_save.elapsed() > Duration::from_secs(5)
            && p.st.duration > 0.
            && (p.st.time - p.saved_pos).abs() > 0.5
            && !p.ended
        {
            p.last_save = Instant::now();
            p.saved_pos = p.st.time;
            if let Some(b) = self.backend.clone() {
                let (id, t, d) = (p.item_id.clone(), p.st.time, p.st.duration);
                let done = t / d > 0.92;
                let b2 = b.clone();
                b.spawn(async move {
                    if let Err(e) = backend::save_progress(b2, id, t, d, done).await {
                        eprintln!("progress: {e}");
                    }
                });
            }
        }
        if redraw {
            cx.notify();
        }
    }

    pub fn player_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.player.as_mut() else { return };
        p.last_move = Instant::now();
        let shift = ev.keystroke.modifiers.shift;
        match ev.keystroke.key.as_str() {
            "space" | "k" => p.cmd(&["cycle", "pause"]),
            "left" => p.cmd(&["seek", if shift { "-30" } else { "-5" }, "relative"]),
            "right" => p.cmd(&["seek", if shift { "30" } else { "5" }, "relative"]),
            "j" => p.cmd(&["seek", "-10", "relative"]),
            "l" => p.cmd(&["seek", "10", "relative"]),
            "up" => p.cmd(&["add", "volume", "5"]),
            "down" => p.cmd(&["add", "volume", "-5"]),
            "m" => p.cmd(&["cycle", "mute"]),
            "s" => p.cmd(&["cycle", "sub"]),
            "a" => p.cmd(&["cycle", "audio"]),
            "n" => {
                self.player_next(cx);
                return;
            }
            "enter" if p.next_at.is_some() => {
                self.player_next(cx);
                return;
            }
            "f" => window.toggle_fullscreen(),
            "escape" => {
                if p.next_at.is_some() {
                    p.next_at = None;
                } else if p.menu.is_some() {
                    p.menu = None;
                } else if window.is_fullscreen() {
                    window.toggle_fullscreen();
                } else {
                    self.close_player(cx);
                    return;
                }
            }
            d if d.len() == 1 && d.chars().all(|c| c.is_ascii_digit()) => {
                // 0..9: jump to 0%..90%.
                let pct = d.parse::<f64>().unwrap_or(0.) * 10.;
                p.cmd(&["seek", &format!("{pct}"), "absolute-percent"]);
            }
            _ => {}
        }
        cx.notify();
    }

    fn player_mouse_move(&mut self, ev: &MouseMoveEvent, _w: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.player.as_mut() else { return };
        p.last_move = Instant::now();
        let x = ev.position.x;
        if p.seek_drag.is_some() {
            let f = frac(x, p.seek_bounds.get());
            p.seek_drag = Some(f);
            if p.last_scrub.elapsed() > Duration::from_millis(90) && p.st.duration > 0. {
                p.last_scrub = Instant::now();
                let t = format!("{:.3}", f as f64 * p.st.duration);
                p.cmd(&["seek", &t, "absolute+keyframes"]);
            }
            cx.notify();
        } else if p.vol_drag {
            let f = frac(x, p.vol_bounds.get());
            p.cmd(&["set", "volume", &format!("{:.0}", f * 100.)]);
            p.st.volume = f as f64 * 100.;
            cx.notify();
        } else if !p.visible {
            cx.notify();
        }
    }

    fn player_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let vh = f32::from(window.viewport_size().height);
        let Some(p) = self.player.as_mut() else { return };
        p.last_move = Instant::now();
        let pos = ev.position;
        let sb = p.seek_bounds.get();
        let grow = px(8.);
        let in_seek =
            pos.x >= sb.left() && pos.x <= sb.right() && pos.y >= sb.top() - grow && pos.y <= sb.bottom() + grow;
        let vb = p.vol_bounds.get();
        let in_vol = pos.x >= vb.left() - px(4.)
            && pos.x <= vb.right() + px(4.)
            && pos.y >= vb.top() - grow
            && pos.y <= vb.bottom() + grow;
        let y = f32::from(pos.y);
        if p.visible && in_seek {
            p.seek_drag = Some(frac(pos.x, sb));
        } else if p.visible && in_vol {
            p.vol_drag = true;
            let f = frac(pos.x, vb);
            p.cmd(&["set", "volume", &format!("{:.0}", f * 100.)]);
        } else if p.visible && (y < 110. || y > vh - 130.) {
            // the control bars: their buttons handle clicks
        } else if p.menu.is_some() {
            p.menu = None;
        } else if ev.click_count == 2 {
            window.toggle_fullscreen();
        } else if !p.is_external() {
            p.cmd(&["cycle", "pause"]);
        }
        cx.notify();
    }

    fn player_mouse_up(&mut self, _ev: &MouseUpEvent, _w: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.player.as_mut() else { return };
        if let Some(f) = p.seek_drag.take() {
            if p.st.duration > 0. {
                let t = format!("{:.3}", f as f64 * p.st.duration);
                p.cmd(&["seek", &t, "absolute"]);
                p.st.time = f as f64 * p.st.duration;
            }
            cx.notify();
        }
        p.vol_drag = false;
    }

    pub fn render_player(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let th = self.theme.clone();
        let pal = self.pal.clone();
        let (vw, vh) = crate::ui::viewport(window);
        let fullscreen = window.is_fullscreen();
        let display = self.fonts.display.clone();
        let mono = self.fonts.mono.clone();
        let next_label = self.player.as_ref().and_then(|p| p.next_id.clone()).and_then(|id| {
            self.by_id.get(&id).map(|&i| episode_label(self.catalog[i].season, self.catalog[i].episode))
        });
        let tg = self.theme_gen;
        let lay = self.layout.clone();
        let p = self.player.as_mut().expect("player");
        #[cfg(windows)]
        if let Some(h) = p.host.as_mut() {
            h.sync();
        }
        let external = p.is_external();
        let st = p.st.clone();
        let opacity = p.fade.value(&th);
        if p.fade.running() {
            window.request_animation_frame();
        }
        // The player's chrome sits on video: light text on dark scrims,
        // whatever the theme's scheme.
        let fg = gpui::white().opacity(0.94);
        let m = th.sp(5);

        let mut root = div()
            .id("player")
            .size_full()
            .relative()
            .overflow_hidden()
            .on_mouse_move(cx.listener(Self::player_mouse_move))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::player_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::player_mouse_up));
        if external {
            root = root.absolute().inset_0().bg(pal.bg.opacity(0.97));
        } else if !st.has_video {
            // Until mpv's first frame the host shows nothing (the GPUI window
            // has no redirection bitmap), so paint black ourselves.
            root = root.bg(gpui::black());
        }

        let center_msg = if let Some(e) = p.error.clone().or(st.error.clone()) {
            Some((format!("Playback failed: {e}"), pal.danger))
        } else if p.pending {
            Some(("Asking the server how to play this…".to_string(), fg.opacity(0.6)))
        } else if external && st.has_video {
            Some(("Playing in the mpv window. These controls drive it.".to_string(), fg.opacity(0.7)))
        } else if !st.has_video {
            Some((format!("Opening · {}", p.source_note), fg.opacity(0.6)))
        } else {
            None
        };
        if let Some((msg, c)) = center_msg {
            root = root.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(m * 2.))
                    .text_color(c)
                    .child(msg),
            );
        }

        // next-episode countdown
        if let Some(at) = p.next_at {
            let left = at.saturating_duration_since(Instant::now()).as_secs_f32().ceil() as i64;
            root = root.child(
                div()
                    .absolute()
                    .right(px(m))
                    .bottom(px(150.))
                    .p(px(th.sp(4)))
                    .rounded(px(th.radius.panel))
                    .bg(gpui::black().opacity(0.72))
                    .text_color(fg)
                    .flex()
                    .flex_col()
                    .gap(px(th.sp(2)))
                    .child(
                        div()
                            .font_family(mono.clone())
                            .text_size(px(th.ts(-2)))
                            .text_color(fg.opacity(0.6))
                            .child("UP NEXT"),
                    )
                    .child(
                        div()
                            .font_family(display.clone())
                            .text_size(px(th.ts(2)))
                            .child(format!("{} in {left} s", next_label.clone().unwrap_or_default())),
                    )
                    .child(
                        div().text_size(px(th.ts(-1))).text_color(fg.opacity(0.7)).child("Enter: play now · Esc: stay"),
                    ),
            );
        }

        if opacity <= 0.001 {
            return root;
        }

        let radius = th.radius.button;
        let btn = |id: &'static str, icon: SharedIcon, size: f32| {
            div()
                .id(id)
                .size(px(size))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(radius))
                .cursor_pointer()
                .hover(|s| s.bg(gpui::white().opacity(0.12)))
                .child(gpui::svg().path(icon.0).size(px(size * 0.5)).text_color(fg))
        };
        let ip = |n: &str| SharedIcon(gpui::SharedString::from(format!("icons/{n}.svg?g={tg}")));

        // ---- the title block: top (default), above the seek bar, or hidden
        // (layout slots, `layout.rs`)
        let title_block = div()
            .flex()
            .flex_col()
            .text_color(fg)
            .child(
                div()
                    .font_family(display)
                    .text_size(px(th.ts(3)))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(p.title.clone()),
            )
            .child(div().text_color(fg.opacity(0.7)).child(format!("{} · {}", p.label, p.source_note)));
        let mut title_block = Some(title_block);
        let mut top = div()
            .absolute()
            .top_0()
            .left_0()
            .w(px(vw))
            .h(px(130.))
            .opacity(opacity)
            .bg(linear_gradient(
                180.,
                linear_color_stop(pal.controls_bg, 0.),
                linear_color_stop(pal.controls_bg.opacity(0.), 1.),
            ))
            .flex()
            .items_start()
            .gap(px(th.sp(3)))
            .px(px(m))
            .pt(px(th.sp(4)))
            .child(
                btn("back", ip("back"), 44.).on_click(cx.listener(|app, _: &ClickEvent, _, cx| app.close_player(cx))),
            );
        if lay.player_title == crate::layout::Place::Top {
            top = top.children(title_block.take());
        }
        root = root.child(top);

        // ---- bottom: seek bar + controls
        let dur = st.duration.max(0.);
        let shown_t = p.seek_drag.map(|f| f as f64 * dur).unwrap_or(st.time);
        let pos_f = if dur > 0. { (shown_t / dur).clamp(0., 1.) as f32 } else { 0. };
        let buf_f = if dur > 0. { (st.cache_end / dur).clamp(0., 1.) as f32 } else { 0. };
        let seek_h = th.player.seek_h.max(1.);
        let seek_w = vw - 2. * m;
        let seek_cell = p.seek_bounds.clone();
        let vol_cell = p.vol_bounds.clone();
        let active = p.seek_drag.is_some();
        let knob = seek_h * 3.;
        let mut seek = div()
            .relative()
            .w(px(seek_w))
            .h(px(knob))
            .flex()
            .items_center()
            .cursor_pointer()
            .child(canvas(move |b, _, _| seek_cell.set(b), |_, _, _, _| {}).absolute().inset_0())
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(if active { seek_h * 1.5 } else { seek_h }))
                    .rounded_full()
                    .bg(fg.opacity(0.18))
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top_0()
                            .h_full()
                            .w(px(seek_w * buf_f))
                            .rounded_full()
                            .bg(fg.opacity(0.32)),
                    )
                    .child(
                        div().absolute().left_0().top_0().h_full().w(px(seek_w * pos_f)).rounded_full().bg(pal.accent),
                    ),
            )
            .child(div().absolute().left(px(seek_w * pos_f - knob / 2.)).top_0().size(px(knob)).rounded_full().bg(fg));
        if let Some(f) = p.seek_drag {
            // The time under the pointer while scrubbing (the server has no
            // trick-play thumbnails yet).
            seek = seek.child(
                div()
                    .absolute()
                    .left(px((seek_w * f - 30.).clamp(0., seek_w - 60.)))
                    .bottom(px(knob + 6.))
                    .px(px(6.))
                    .py(px(2.))
                    .rounded(px(4.))
                    .bg(gpui::black().opacity(0.8))
                    .font_family(mono.clone())
                    .text_size(px(th.ts(-1)))
                    .text_color(fg)
                    .child(fmt_time(f as f64 * dur)),
            );
        }

        let menu = p.menu;
        let vol = (st.volume / 100.).clamp(0., 1.) as f32;
        let vol_w = 96.;
        let has_next = p.next_id.is_some();
        let playing_icon = if st.paused { "play" } else { "pause" };
        let mut controls = div().flex().items_center().gap(px(th.sp(2))).w(px(seek_w)).text_color(fg);
        for c in &lay.controls {
            use crate::layout::Control as C;
            controls = match c {
                C::Play => controls.child(btn("pp", ip(playing_icon), 44.).on_click(cx.listener(
                    |app, _: &ClickEvent, _, cx| {
                        if let Some(p) = &app.player {
                            p.cmd(&["cycle", "pause"]);
                        }
                        cx.notify();
                    },
                ))),
                C::Next if has_next => {
                    controls.child(btn("next", ip("next"), 40.).on_click(cx.listener(|app, _: &ClickEvent, _, cx| {
                        app.player_next(cx);
                        cx.notify();
                    })))
                }
                C::Next => controls,
                C::Time => controls.child(div().font_family(mono.clone()).text_size(px(th.ts(-1))).child(format!(
                    "{} / {}",
                    fmt_time(shown_t),
                    fmt_time(dur)
                ))),
                C::Spacer => controls.child(div().flex_1()),
                C::Mute => controls.child(btn("mute", ip(if st.muted { "mute" } else { "volume" }), 40.).on_click(
                    cx.listener(|app, _: &ClickEvent, _, cx| {
                        if let Some(p) = &app.player {
                            p.cmd(&["cycle", "mute"]);
                        }
                        cx.notify();
                    }),
                )),
                C::Volume => {
                    let vc = vol_cell.clone();
                    controls.child(
                        div()
                            .relative()
                            .w(px(vol_w))
                            .h(px(18.))
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .child(canvas(move |b, _, _| vc.set(b), |_, _, _, _| {}).absolute().inset_0())
                            .child(div().w_full().h(px(seek_h)).rounded_full().bg(fg.opacity(0.18)).child(
                                div().h_full().w(px(vol_w * if st.muted { 0. } else { vol })).rounded_full().bg(fg),
                            )),
                    )
                }
                C::Audio => controls.child(btn("audio", ip("audio"), 40.).on_click(cx.listener(
                    |app, _: &ClickEvent, _, cx| {
                        if let Some(p) = app.player.as_mut() {
                            p.menu = if p.menu == Some(Menu::Audio) { None } else { Some(Menu::Audio) };
                        }
                        cx.notify();
                    },
                ))),
                C::Subs => controls.child(btn("subs", ip("subtitles"), 40.).on_click(cx.listener(
                    |app, _: &ClickEvent, _, cx| {
                        if let Some(p) = app.player.as_mut() {
                            p.menu = if p.menu == Some(Menu::Subs) { None } else { Some(Menu::Subs) };
                        }
                        cx.notify();
                    },
                ))),
                C::Fullscreen => controls.child(
                    btn("fs", ip(if fullscreen { "windowed" } else { "fullscreen" }), 40.)
                        .on_click(cx.listener(|_, _: &ClickEvent, window, _| window.toggle_fullscreen())),
                ),
            };
        }

        let mut bottom = div()
            .absolute()
            .left_0()
            .bottom_0()
            .w(px(vw))
            .h(px(if title_block.is_some() && lay.player_title == crate::layout::Place::Bottom { 240. } else { 170. }))
            .opacity(opacity)
            .bg(linear_gradient(
                180.,
                linear_color_stop(pal.controls_bg.opacity(0.), 0.),
                linear_color_stop(pal.controls_bg, 1.),
            ))
            .flex()
            .flex_col()
            .justify_end()
            .items_center()
            .gap(px(th.sp(1)))
            .pb(px(th.sp(4)));
        if lay.player_title == crate::layout::Place::Bottom
            && let Some(t) = title_block.take()
        {
            bottom = bottom.child(div().w(px(seek_w)).pb(px(th.sp(2))).child(t));
        }
        bottom = if lay.seek_above { bottom.child(seek).child(controls) } else { bottom.child(controls).child(seek) };
        if lay.hints {
            bottom = bottom.child(
                div()
                    .w(px(seek_w))
                    .flex()
                    .gap(px(th.sp(3)))
                    .text_size(px(th.ts(-3)))
                    .text_color(fg.opacity(0.5))
                    .child("Space pause · ←/→ 5 s · J/L 10 s · 0–9 jump · A audio · S subs · N next · F fullscreen · Esc back"),
            );
        }
        root = root.child(bottom);

        // ---- track menu
        if let Some(menu) = menu {
            let kind = if menu == Menu::Audio { "audio" } else { "sub" };
            let mut list = div()
                .id("menu")
                .absolute()
                .right(px(m))
                .bottom(px(120.))
                .w(px(360.))
                .max_h(px(vh * 0.5))
                .overflow_y_scroll()
                .opacity(opacity)
                .p(px(th.sp(1)))
                .rounded(px(th.radius.card))
                .bg(gpui::black().opacity(0.85))
                .text_color(fg)
                .shadow_lg()
                .flex()
                .flex_col()
                .child(
                    div()
                        .px(px(th.sp(3)))
                        .py(px(th.sp(2)))
                        .text_color(fg.opacity(0.6))
                        .text_size(px(th.ts(-2)))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(if menu == Menu::Audio { "AUDIO" } else { "SUBTITLES" }),
                );
            let mut entries: Vec<(String, String, bool)> = Vec::new();
            if menu == Menu::Subs {
                let none = !st.tracks.iter().any(|t| t.kind == "sub" && t.selected);
                entries.push(("Off".into(), "no".into(), none));
            }
            for t in st.tracks.iter().filter(|t| t.kind == kind) {
                entries.push((t.label.clone(), t.id.to_string(), t.selected));
            }
            for (i, (label, value, selected)) in entries.into_iter().enumerate() {
                let prop = if menu == Menu::Audio { "aid" } else { "sid" };
                list = list.child(
                    div()
                        .id(("track", i))
                        .flex()
                        .items_center()
                        .gap(px(th.sp(2)))
                        .px(px(th.sp(3)))
                        .py(px(th.sp(2)))
                        .rounded(px(th.radius.button))
                        .cursor_pointer()
                        .hover(|s| s.bg(gpui::white().opacity(0.08)))
                        .child(div().size(px(8.)).rounded_full().bg(if selected {
                            pal.accent
                        } else {
                            pal.accent.opacity(0.)
                        }))
                        .child(div().truncate().child(label))
                        .on_click(cx.listener(move |app, _: &ClickEvent, _, cx| {
                            if let Some(p) = app.player.as_mut() {
                                p.cmd(&["set", prop, &value]);
                                p.menu = None;
                            }
                            cx.notify();
                        })),
                );
            }
            root = root.child(list);
        }
        root
    }
}

/// An icon path, typed so the button helper closure stays `Copy`-free.
struct SharedIcon(gpui::SharedString);

/// Builds and starts the engine for a resolved source.
fn start_engine(
    r: Resolved,
    mute: bool,
    slang: &str,
    title: &str,
    #[cfg(windows)] wid: isize,
) -> Result<Arc<dyn Engine>, String> {
    #[cfg(windows)]
    {
        let _ = title;
        let start_s = format!("{:.3}", r.start);
        let mut opts = vec![("start", start_s.as_str())];
        if mute {
            opts.push(("ao", "null"));
            opts.push(("mute", "yes"));
        }
        if !slang.is_empty() {
            opts.push(("slang", slang));
        }
        let m = crate::mpv::Mpv::new(wid, &opts)?;
        m.command(&["loadfile", &r.url])?;
        for (url, name, lang) in &r.subs {
            m.command_async(&["sub-add", url, "auto", name, lang]);
        }
        Ok(Arc::new(m))
    }
    #[cfg(unix)]
    {
        let o = crate::mpv_ipc::ExtOptions {
            url: r.url,
            title: title.to_string(),
            start: r.start,
            mute,
            alang: String::new(),
            slang: slang.to_string(),
            sub_files: r.subs.into_iter().map(|s| s.0).collect(),
        };
        Ok(Arc::new(crate::mpv_ipc::ExtMpv::spawn(o)?))
    }
}

fn frac(x: Pixels, b: Bounds<Pixels>) -> f32 {
    let w = f32::from(b.size.width).max(1.);
    ((f32::from(x) - f32::from(b.origin.x)) / w).clamp(0., 1.)
}
