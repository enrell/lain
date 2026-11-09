//! Player: mpv draws into a child window under GPUI's transparent surface
//! (see `mpv.rs`); this view only draws the controls over it. While the
//! controls are hidden the view does not redraw at all.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    Bounds, Context, FontWeight, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Window, canvas, div, linear_color_stop, linear_gradient, px, svg,
};

use crate::home::fmt_time;
use crate::mpv::{Mpv, PlayerState, VideoHost};
use crate::{Anim, LainApp, backend};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    Audio,
    Subs,
}

pub struct PlayerSession {
    pub id: u64,
    pub item: usize,
    pub file: PathBuf,
    pub host: Option<VideoHost>,
    pub mpv: Option<Arc<Mpv>>,
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
    pub opened: Instant,
    pub force_controls: bool,
}

impl PlayerSession {
    fn wants_controls(&self, autohide_ms: f32) -> bool {
        self.force_controls
            || self.st.paused
            || self.menu.is_some()
            || self.seek_drag.is_some()
            || self.vol_drag
            || !self.st.has_video
            || self.last_move.elapsed().as_secs_f32() * 1000. < autohide_ms
    }

    fn mpv(&self, args: &[&str]) {
        if let Some(m) = &self.mpv {
            m.command_async(args);
        }
    }
}

impl LainApp {
    pub fn open_player(&mut self, item: usize, cx: &mut Context<Self>) {
        if self.player.is_some() || self.hwnd == 0 || item >= self.items.len() {
            return;
        }
        let host = match VideoHost::new(self.hwnd) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("video host: {e}");
                return;
            }
        };
        let wid = host.wid();
        let id = self.next_session;
        self.next_session += 1;
        let file = self.media_for(&self.items[item]);
        let file_s = file.to_string_lossy().into_owned();
        let force = self.bench.enabled();
        let mute = self.args.mute;
        // Bench runs always start at 0 so every run plays the same frames.
        let start_at = if force { Some(0.) } else { self.start_override.take() };
        self.player = Some(PlayerSession {
            id,
            item,
            file,
            host: Some(host),
            mpv: None,
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
            opened: Instant::now(),
            force_controls: force,
        });
        let ptask = self.backend.spawn(backend::progress(self.backend.client.clone(), self.items[item].id.clone()));
        cx.spawn(async move |this, cx| {
            let p = ptask.await.ok().flatten();
            let start = match start_at {
                Some(t) => t,
                None => p.filter(|p| !p.completed && p.position_sec > 3.).map(|p| p.position_sec).unwrap_or(0.),
            };
            // mpv_initialize creates mpv's window, which talks to our host
            // window: never block the UI thread on it.
            let init = cx
                .background_executor()
                .spawn(async move {
                    let start_s = format!("{start:.3}");
                    let mut opts = vec![("start", start_s.as_str())];
                    if mute {
                        opts.push(("ao", "null"));
                        opts.push(("mute", "yes"));
                    }
                    let m = Mpv::new(wid, &opts)?;
                    m.command(&["loadfile", &file_s])?;
                    Ok::<_, String>(Arc::new(m))
                })
                .await;
            let _ = this.update(cx, |app, cx| app.player_started(id, init, start, cx));
        })
        .detach();
        cx.notify();
    }

    fn player_started(&mut self, id: u64, init: Result<Arc<Mpv>, String>, start: f64, cx: &mut Context<Self>) {
        if let Err(e) = &init {
            eprintln!("player: session {id}: {e}");
        }
        match self.player.as_mut() {
            Some(p) if p.id == id => {
                p.pending = false;
                p.saved_pos = start;
                match init {
                    Ok(m) => p.mpv = Some(m),
                    Err(e) => p.error = Some(e),
                }
                cx.notify();
            }
            _ => {
                // Closed while starting.
                if let Some(pos) = self.orphans.iter().position(|o| o.id == id) {
                    let mut s = self.orphans.remove(pos);
                    s.mpv = init.ok();
                    self.teardown(s, cx);
                }
            }
        }
    }

    fn teardown(&mut self, mut s: PlayerSession, cx: &mut Context<Self>) {
        if let Some(h) = &s.host {
            h.hide();
        }
        let mpv = s.mpv.take();
        let host = s.host.take();
        // Dropping mpv joins its threads, and mpv's window is a child of
        // the host: keep the UI thread pumping messages meanwhile.
        let gone = cx.background_executor().spawn(async move { drop(mpv) });
        cx.spawn(async move |_, _| {
            gone.await;
            drop(host);
        })
        .detach();
    }

    pub fn close_player(&mut self, cx: &mut Context<Self>) {
        let Some(s) = self.player.take() else { return };
        if let Some(m) = &s.mpv {
            let st = m.snapshot();
            if st.duration > 0. {
                let id = self.items[s.item].id.clone();
                self.backend.spawn(backend::save_progress(self.backend.client.clone(), id, st.time, st.duration));
            }
        }
        if s.pending {
            if let Some(h) = &s.host {
                h.hide();
            }
            self.orphans.push(s);
        } else {
            self.teardown(s, cx);
        }
        // The event stream also pokes us; this makes the row current at once.
        let me = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(Duration::from_millis(400)).await;
            let _ = me.update(cx, |app, cx| app.refresh_continues(cx));
        })
        .detach();
        cx.notify();
    }

    /// Saves progress synchronously (window close).
    pub fn save_now(&mut self) {
        if let Some(p) = &self.player
            && let Some(m) = &p.mpv
        {
            let st = m.snapshot();
            if st.duration > 0. {
                let id = self.items[p.item].id.clone();
                let client = self.backend.client.clone();
                let _ = self.backend.rt.block_on(async {
                    tokio::time::timeout(Duration::from_secs(2), backend::save_progress(client, id, st.time, st.duration)).await
                });
            }
        }
    }

    pub fn player_tick(&mut self, cx: &mut Context<Self>) {
        let autohide = self.theme.player.autohide_ms;
        let (fast, normal) = (self.theme.motion.fast_ms, self.theme.motion.normal_ms);
        let th = self.theme.clone();
        let Some(p) = self.player.as_mut() else { return };
        if let Some(h) = p.host.as_mut() {
            h.sync();
        }
        let Some(m) = p.mpv.clone() else { return };
        let st = m.snapshot();
        if st.has_video && !p.st.has_video {
            eprintln!("mpv: current-ao={} current-vo={} hwdec={}", m.get_string("current-ao").unwrap_or_else(|| "none".into()),
                m.get_string("current-vo").unwrap_or_default(), m.get_string("hwdec-current").unwrap_or_default());
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
        // Progress: every 5 s of playback, and on close.
        if p.last_save.elapsed() > Duration::from_secs(5) && p.st.duration > 0. && (p.st.time - p.saved_pos).abs() > 0.5 {
            p.last_save = Instant::now();
            p.saved_pos = p.st.time;
            let id = self.items[p.item].id.clone();
            let (t, d) = (p.st.time, p.st.duration);
            self.backend.spawn(backend::save_progress(self.backend.client.clone(), id, t, d));
        }
        if redraw {
            cx.notify();
        }
    }

    pub fn player_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.player.as_mut() else { return };
        p.last_move = Instant::now();
        match ev.keystroke.key.as_str() {
            "space" | "k" => p.mpv(&["cycle", "pause"]),
            "left" => p.mpv(&["seek", "-5", "relative"]),
            "right" => p.mpv(&["seek", "5", "relative"]),
            "up" => p.mpv(&["add", "volume", "5"]),
            "down" => p.mpv(&["add", "volume", "-5"]),
            "m" => p.mpv(&["cycle", "mute"]),
            "f" => window.toggle_fullscreen(),
            "escape" => {
                if p.menu.is_some() {
                    p.menu = None;
                } else if window.is_fullscreen() {
                    window.toggle_fullscreen();
                } else {
                    self.close_player(cx);
                    return;
                }
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
            let b = p.seek_bounds.get();
            let f = frac(x, b);
            p.seek_drag = Some(f);
            if p.last_scrub.elapsed() > Duration::from_millis(90) && p.st.duration > 0. {
                p.last_scrub = Instant::now();
                let t = format!("{:.3}", f as f64 * p.st.duration);
                p.mpv(&["seek", &t, "absolute+keyframes"]);
            }
            cx.notify();
        } else if p.vol_drag {
            let f = frac(x, p.vol_bounds.get());
            p.mpv(&["set", "volume", &format!("{:.0}", f * 100.)]);
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
        let in_seek = pos.x >= sb.left() && pos.x <= sb.right() && pos.y >= sb.top() - grow && pos.y <= sb.bottom() + grow;
        let vb = p.vol_bounds.get();
        let in_vol = pos.x >= vb.left() - px(4.) && pos.x <= vb.right() + px(4.) && pos.y >= vb.top() - grow && pos.y <= vb.bottom() + grow;
        let y = f32::from(pos.y);
        if p.visible && in_seek {
            p.seek_drag = Some(frac(pos.x, sb));
        } else if p.visible && in_vol {
            p.vol_drag = true;
            let f = frac(pos.x, vb);
            p.mpv(&["set", "volume", &format!("{:.0}", f * 100.)]);
        } else if p.visible && (y < 110. || y > vh - 120.) {
            // the control bars: their buttons handle clicks
        } else if p.menu.is_some() {
            p.menu = None;
        } else if ev.click_count == 2 {
            window.toggle_fullscreen();
        } else {
            p.mpv(&["cycle", "pause"]);
        }
        cx.notify();
    }

    fn player_mouse_up(&mut self, _ev: &MouseUpEvent, _w: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.player.as_mut() else { return };
        if let Some(f) = p.seek_drag.take() {
            if p.st.duration > 0. {
                let t = format!("{:.3}", f as f64 * p.st.duration);
                p.mpv(&["seek", &t, "absolute"]);
                p.st.time = f as f64 * p.st.duration;
            }
            cx.notify();
        }
        p.vol_drag = false;
    }

    pub fn render_player(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let th = self.theme.clone();
        let pal = self.pal.clone();
        let vs = window.viewport_size();
        let (vw, vh) = (f32::from(vs.width), f32::from(vs.height));
        let fullscreen = window.is_fullscreen();
        let item = self.items[self.player.as_ref().map(|p| p.item).unwrap_or(0)].clone();
        let series = self.series_title.get(&item.slug).cloned().unwrap_or(item.title.clone());
        let display = self.fonts.display.clone();
        let mono = self.fonts.mono.clone();
        let p = self.player.as_mut().expect("player");
        if let Some(h) = p.host.as_mut() {
            h.sync();
        }
        let st = p.st.clone();
        let opacity = p.fade.value(&th);
        if p.fade.running() {
            window.request_animation_frame();
        }
        let m = th.sp(5);
        let fmt = if p.file.to_string_lossy().contains("4k") { "2160p HEVC · 60 fps" } else { "1080p HEVC 10-bit · ASS subtitles" };

        let mut root = div()
            .id("player")
            .size_full()
            .relative()
            .overflow_hidden()
            .on_mouse_move(cx.listener(Self::player_mouse_move))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::player_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::player_mouse_up));
        // Until mpv's first frame the host shows nothing (the GPUI window has
        // no redirection bitmap), so paint black ourselves.
        if !st.has_video {
            root = root.bg(gpui::black());
        }

        let center_msg = if let Some(e) = p.error.clone().or(st.error.clone()) {
            Some((format!("Playback failed: {e}"), pal.danger))
        } else if !st.has_video {
            Some((format!("Opening {}…", p.file.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()), pal.muted))
        } else {
            None
        };
        if let Some((msg, c)) = center_msg {
            root = root.child(div().absolute().inset_0().flex().items_center().justify_center().text_color(c).child(msg));
        }

        if opacity <= 0.001 {
            return root;
        }

        let btn = |id: &'static str, icon: &'static str, size: f32| {
            div()
                .id(id)
                .size(px(size))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(th.radius.button))
                .cursor_pointer()
                .hover(|s| s.bg(gpui::white().opacity(0.12)))
                .child(svg().path(icon).size(px(size * 0.5)).text_color(pal.text))
        };

        // ---- top: back + title
        root = root.child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .w(px(vw))
                .h(px(130.))
                .opacity(opacity)
                .bg(linear_gradient(180., linear_color_stop(pal.controls_bg, 0.), linear_color_stop(pal.controls_bg.opacity(0.), 1.)))
                .flex()
                .items_start()
                .gap(px(th.sp(3)))
                .px(px(m))
                .pt(px(th.sp(4)))
                .child(btn("back", "icons/back.svg", 44.).on_click(cx.listener(|app, _, _, cx| app.close_player(cx))))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(div().font_family(display).text_size(px(th.fonts.size_base * 1.8)).font_weight(FontWeight::SEMIBOLD).child(series))
                        .child(div().text_color(pal.text.opacity(0.7)).child(format!("{} · {}", item.label(), fmt))),
                ),
        );

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
        let seek = div()
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
                    .bg(pal.text.opacity(0.18))
                    .child(div().absolute().left_0().top_0().h_full().w(px(seek_w * buf_f)).rounded_full().bg(pal.text.opacity(0.32)))
                    .child(div().absolute().left_0().top_0().h_full().w(px(seek_w * pos_f)).rounded_full().bg(pal.accent)),
            )
            .child(
                div()
                    .absolute()
                    .left(px(seek_w * pos_f - knob / 2.))
                    .top_0()
                    .size(px(knob))
                    .rounded_full()
                    .bg(pal.text),
            );

        let menu = p.menu;
        let vol = (st.volume / 100.).clamp(0., 1.) as f32;
        let vol_w = 96.;
        let playing_icon = if st.paused { "icons/play.svg" } else { "icons/pause.svg" };
        let controls = div()
            .flex()
            .items_center()
            .gap(px(th.sp(2)))
            .w(px(seek_w))
            .child(btn("pp", playing_icon, 44.).on_click(cx.listener(|app, _, _, cx| {
                if let Some(p) = &app.player {
                    p.mpv(&["cycle", "pause"]);
                }
                cx.notify();
            })))
            .child(
                div()
                    .font_family(mono)
                    .text_size(px(th.fonts.size_base * 0.9))
                    .child(format!("{} / {}", fmt_time(shown_t), fmt_time(dur))),
            )
            .child(div().flex_1())
            .child(btn("mute", if st.muted { "icons/mute.svg" } else { "icons/volume.svg" }, 40.).on_click(cx.listener(|app, _, _, cx| {
                if let Some(p) = &app.player {
                    p.mpv(&["cycle", "mute"]);
                }
                cx.notify();
            })))
            .child(
                div()
                    .relative()
                    .w(px(vol_w))
                    .h(px(18.))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .child(canvas(move |b, _, _| vol_cell.set(b), |_, _, _, _| {}).absolute().inset_0())
                    .child(
                        div()
                            .w_full()
                            .h(px(seek_h))
                            .rounded_full()
                            .bg(pal.text.opacity(0.18))
                            .child(div().h_full().w(px(vol_w * if st.muted { 0. } else { vol })).rounded_full().bg(pal.text)),
                    ),
            )
            .child(btn("audio", "icons/audio.svg", 40.).on_click(cx.listener(|app, _, _, cx| {
                if let Some(p) = app.player.as_mut() {
                    p.menu = if p.menu == Some(Menu::Audio) { None } else { Some(Menu::Audio) };
                }
                cx.notify();
            })))
            .child(btn("subs", "icons/subtitles.svg", 40.).on_click(cx.listener(|app, _, _, cx| {
                if let Some(p) = app.player.as_mut() {
                    p.menu = if p.menu == Some(Menu::Subs) { None } else { Some(Menu::Subs) };
                }
                cx.notify();
            })))
            .child(
                btn("fs", if fullscreen { "icons/windowed.svg" } else { "icons/fullscreen.svg" }, 40.)
                    .on_click(cx.listener(|_, _, window, _| window.toggle_fullscreen())),
            );

        root = root.child(
            div()
                .absolute()
                .left_0()
                .bottom_0()
                .w(px(vw))
                .h(px(170.))
                .opacity(opacity)
                .bg(linear_gradient(180., linear_color_stop(pal.controls_bg.opacity(0.), 0.), linear_color_stop(pal.controls_bg, 1.)))
                .flex()
                .flex_col()
                .justify_end()
                .items_center()
                .gap(px(th.sp(1)))
                .pb(px(th.sp(4)))
                .child(seek)
                .child(controls),
        );

        // ---- track menu
        if let Some(menu) = menu {
            let kind = if menu == Menu::Audio { "audio" } else { "sub" };
            let mut list = div()
                .id("menu")
                .absolute()
                .right(px(m))
                .bottom(px(110.))
                .w(px(340.))
                .max_h(px(vh * 0.5))
                .overflow_y_scroll()
                .opacity(opacity)
                .p(px(th.sp(1)))
                .rounded(px(th.radius.card))
                .bg(pal.surface.opacity(0.96))
                .shadow_lg()
                .flex()
                .flex_col()
                .child(
                    div()
                        .px(px(th.sp(3)))
                        .py(px(th.sp(2)))
                        .text_color(pal.muted)
                        .text_size(px(th.fonts.size_base * 0.8))
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
                        .child(div().size(px(8.)).rounded_full().bg(if selected { pal.accent } else { pal.accent.opacity(0.) }))
                        .child(div().truncate().child(label))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            if let Some(p) = app.player.as_mut() {
                                p.mpv(&["set", prop, &value]);
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

fn frac(x: Pixels, b: Bounds<Pixels>) -> f32 {
    let w = f32::from(b.size.width).max(1.);
    ((f32::from(x) - f32::from(b.origin.x)) / w).clamp(0., 1.)
}
