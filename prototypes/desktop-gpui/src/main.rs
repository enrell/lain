//! lain desktop prototype on GPUI (P2b, DESIGN-v2 §8.2 V-13): Home with a
//! full-bleed hero and rails over the whole catalog, and a Player with
//! libmpv rendering inside the window under GPUI's own controls.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backend;
mod bench;
mod home;
mod images;
mod mpv;
mod player;
mod theme;

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use gpui::prelude::*;
use gpui::{
    App, Application, AssetSource, Bounds, Context, Entity, FocusHandle, KeyDownEvent, SharedString, TitlebarOptions,
    Window, WindowBackgroundAppearance, WindowBounds, WindowOptions, div, px, size,
};

use backend::{Backend, Item};
use images::ImageStore;
use theme::{Palette, Theme};

// ---- assets: icons and the bundled fonts ------------------------------------

struct Assets;

const ICONS: &[(&str, &[u8])] = &[
    ("icons/play.svg", include_bytes!("../assets/icons/play.svg")),
    ("icons/pause.svg", include_bytes!("../assets/icons/pause.svg")),
    ("icons/back.svg", include_bytes!("../assets/icons/back.svg")),
    ("icons/volume.svg", include_bytes!("../assets/icons/volume.svg")),
    ("icons/mute.svg", include_bytes!("../assets/icons/mute.svg")),
    ("icons/subtitles.svg", include_bytes!("../assets/icons/subtitles.svg")),
    ("icons/audio.svg", include_bytes!("../assets/icons/audio.svg")),
    ("icons/fullscreen.svg", include_bytes!("../assets/icons/fullscreen.svg")),
    ("icons/windowed.svg", include_bytes!("../assets/icons/windowed.svg")),
];

const FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/inter-latin-400-normal.ttf"),
    include_bytes!("../assets/fonts/inter-latin-500-normal.ttf"),
    include_bytes!("../assets/fonts/inter-latin-600-normal.ttf"),
    include_bytes!("../assets/fonts/fraunces-latin-400-normal.ttf"),
    include_bytes!("../assets/fonts/fraunces-latin-600-normal.ttf"),
    include_bytes!("../assets/fonts/fraunces-latin-700-normal.ttf"),
    include_bytes!("../assets/fonts/jetbrains-mono-latin-400-normal.ttf"),
];

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS.iter().find(|(p, _)| *p == path).map(|(_, b)| Cow::Borrowed(*b)))
    }
    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(ICONS.iter().filter(|(p, _)| p.starts_with(path)).map(|(p, _)| SharedString::from(*p)).collect())
    }
}

// ---- command line --------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Args {
    pub theme: PathBuf,
    pub shared: PathBuf,
    pub media: PathBuf,
    pub server: Option<String>,
    pub bench: Option<PathBuf>,
}

fn parse_args() -> Args {
    let mut a = Args {
        theme: PathBuf::new(),
        shared: PathBuf::from(r"D:\lain-proto\shared"),
        media: PathBuf::from(r"D:\lain-proto\media"),
        server: None,
        bench: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--theme" => a.theme = it.next().map(PathBuf::from).unwrap_or_default(),
            "--shared" => a.shared = it.next().map(PathBuf::from).unwrap_or(a.shared),
            "--media" => a.media = it.next().map(PathBuf::from).unwrap_or(a.media),
            "--server" => a.server = it.next(),
            "--bench" => a.bench = it.next().map(PathBuf::from),
            other => eprintln!("unknown argument {other}"),
        }
    }
    if a.theme.as_os_str().is_empty() {
        a.theme = a.shared.join("theme.json");
    }
    a
}

// ---- small animation helper ------------------------------------------------------

/// A value easing from `from` to `to` over `dur_ms` with the theme's curve.
#[derive(Debug, Clone, Copy)]
pub struct Anim {
    pub from: f32,
    pub to: f32,
    pub start: Instant,
    pub dur_ms: f32,
}

impl Anim {
    pub fn still(v: f32) -> Anim {
        Anim { from: v, to: v, start: Instant::now(), dur_ms: 0. }
    }
    fn t(&self) -> f32 {
        if self.dur_ms <= 0. { 1. } else { (self.start.elapsed().as_secs_f32() * 1000. / self.dur_ms).min(1.) }
    }
    pub fn value(&self, th: &Theme) -> f32 {
        self.from + (self.to - self.from) * th.ease(self.t())
    }
    pub fn running(&self) -> bool {
        self.t() < 1.
    }
    /// Retargets from the current value (keeps motion continuous).
    pub fn go(&mut self, to: f32, dur_ms: f32, th: &Theme) {
        let now = self.value(th);
        *self = Anim { from: now, to, start: Instant::now(), dur_ms };
    }
    pub fn set(&mut self, v: f32) {
        *self = Anim::still(v);
    }
}

// ---- the app -------------------------------------------------------------------

pub enum Status {
    Loading,
    Error(String),
    Ready,
}

/// Font families actually used (the theme's, or a system fallback).
pub struct FontSet {
    pub ui: SharedString,
    pub display: SharedString,
    pub mono: SharedString,
    pub notes: Vec<String>,
}

pub struct LainApp {
    pub args: Args,
    pub theme: Theme,
    pub pal: Palette,
    pub fonts: FontSet,
    theme_mtime: Option<SystemTime>,
    pub theme_reloads: u32,
    pub backend: Arc<Backend>,
    pub images: Entity<ImageStore>,
    pub focus: FocusHandle,
    pub status: Status,
    pub items: Vec<Item>,
    pub by_id: HashMap<String, usize>,
    pub series_title: HashMap<String, String>,
    pub continues: Vec<(usize, lain_client::types::Progress)>,
    pub library: String,
    pub username: String,
    pub home: home::HomeState,
    pub player: Option<player::PlayerSession>,
    /// Sessions closed while mpv was still starting; torn down when it is up.
    pub orphans: Vec<player::PlayerSession>,
    pub next_session: u64,
    pub hwnd: isize,
    pub bench: bench::Bench,
}

fn resolve_fonts(th: &Theme, cx: &App) -> FontSet {
    let names = cx.text_system().all_font_names();
    if std::env::var_os("LAIN_DEBUG_FONTS").is_some() {
        eprintln!("fonts: {:?}", names.iter().filter(|n| n.contains("Fraun") || n.contains("Inter") || n.contains("JetBrains")).collect::<Vec<_>>());
    }
    let mut notes = Vec::new();
    let mut pick = |want: &str, fallback: &str, role: &str| -> SharedString {
        if names.iter().any(|n| n == want) {
            SharedString::from(want.to_string())
        } else if let Some(n) = names.iter().find(|n| n.starts_with(want)) {
            // DirectWrite may group a family under a longer WWS name
            // (the bundled Fraunces shows up as "Fraunces NonWonky").
            notes.push(format!("{role} font {want:?} resolved to {n:?}"));
            SharedString::from(n.to_string())
        } else {
            notes.push(format!("{role} font {want:?} not found; using {fallback:?}"));
            SharedString::from(fallback.to_string())
        }
    };
    let ui = pick(&th.fonts.ui, "Segoe UI", "ui");
    let display = pick(&th.fonts.display, "Georgia", "display");
    let mono = pick(&th.fonts.mono, "Cascadia Mono", "mono");
    FontSet { ui, display, mono, notes }
}

fn read_series(shared: &Path) -> HashMap<String, String> {
    #[derive(serde::Deserialize)]
    struct S {
        slug: String,
        title: String,
    }
    let raw = std::fs::read(shared.join("series.json")).unwrap_or_default();
    let list: Vec<S> = serde_json::from_slice(&raw).unwrap_or_default();
    list.into_iter().map(|s| (s.title, s.slug)).collect()
}

fn session_file() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("lain-proto-gpui").join("sessions.json")
}

impl LainApp {
    fn new(args: Args, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (theme, theme_err) = match Theme::load(&args.theme) {
            Ok(t) => (t, None),
            Err(e) => (Theme::default(), Some(e)),
        };
        let theme_mtime = std::fs::metadata(&args.theme).and_then(|m| m.modified()).ok();
        let fonts = resolve_fonts(&theme, cx);
        for n in &fonts.notes {
            eprintln!("{n}");
        }
        let account = backend::read_account(&args.shared.join("conta.txt"));
        let server = args
            .server
            .clone()
            .or_else(|| account.as_ref().ok().map(|a| a.server.clone()))
            .unwrap_or_else(|| "http://127.0.0.1:9360".into());
        let backend = Backend::new(&server, session_file()).expect("client");
        let images = cx.new(|_| ImageStore::default());
        cx.observe(&images, |_, _, cx| cx.notify()).detach();

        let series = read_series(&args.shared);
        let series_title = series.iter().map(|(t, s)| (s.clone(), t.clone())).collect();
        let focus = cx.focus_handle();
        window.focus(&focus);

        let bench = bench::Bench::new(args.bench.clone());
        let mut app = LainApp {
            pal: theme.palette(),
            theme,
            fonts,
            theme_mtime,
            theme_reloads: 0,
            backend: backend.clone(),
            images,
            focus,
            status: Status::Loading,
            items: Vec::new(),
            by_id: HashMap::new(),
            series_title,
            continues: Vec::new(),
            library: String::new(),
            username: String::new(),
            home: home::HomeState::default(),
            player: None,
            orphans: Vec::new(),
            next_session: 1,
            hwnd: 0,
            bench,
            args,
        };
        if let Some(e) = theme_err {
            eprintln!("theme: {e}; using built-in defaults");
        }
        if let Some(hwnd) = window_hwnd(window) {
            app.hwnd = hwnd;
        }

        // Login + catalog.
        match account {
            Ok(account) => {
                let task = backend.spawn(backend::load_all(backend.client.clone(), account, series));
                cx.spawn(async move |this, cx| {
                    let r = task.await.map_err(|e| e.to_string()).and_then(|r| r);
                    let _ = this.update(cx, |app, cx| {
                        match r {
                            Ok(l) => app.loaded(l),
                            Err(e) => app.status = Status::Error(e),
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            Err(e) => app.status = Status::Error(e),
        }

        // Server events: refetch the Continue row on progress changes.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        backend.spawn(backend::watch_events(backend.client.clone(), tx));
        cx.spawn(async move |this, cx| {
            while rx.recv().await.is_some() {
                // Coalesce bursts.
                cx.background_executor().timer(Duration::from_millis(300)).await;
                while rx.try_recv().is_ok() {}
                if this.update(cx, |app, cx| app.refresh_continues(cx)).is_err() {
                    return;
                }
            }
        })
        .detach();

        // The UI tick: player state, progress saves, theme live reload.
        cx.spawn(async move |this, cx| {
            let mut n: u64 = 0;
            loop {
                cx.background_executor().timer(Duration::from_millis(33)).await;
                n += 1;
                if this.update(cx, |app, cx| app.tick(n, cx)).is_err() {
                    return;
                }
            }
        })
        .detach();

        if app.bench.enabled() {
            cx.spawn_in(window, async move |this, cx| bench::run(this, cx).await).detach();
        }
        app
    }

    fn loaded(&mut self, l: backend::Loaded) {
        self.items = l.items;
        self.by_id = self.items.iter().enumerate().map(|(i, it)| (it.id.clone(), i)).collect();
        self.library = l.library;
        self.username = l.username;
        self.set_continues(l.continues);
        self.home.build(&self.items, &self.continues);
        self.status = Status::Ready;
    }

    fn set_continues(&mut self, list: Vec<lain_client::types::Progress>) {
        self.continues = list
            .into_iter()
            .filter(|p| !p.completed)
            .filter_map(|p| self.by_id.get(&p.item_id).map(|&i| (i, p)))
            .collect();
    }

    pub fn refresh_continues(&mut self, cx: &mut Context<Self>) {
        let task = self.backend.spawn(backend::continues(self.backend.client.clone()));
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(list)) = task.await {
                let _ = this.update(cx, |app, cx| {
                    app.set_continues(list);
                    app.home.rebuild_continue(&app.continues);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn tick(&mut self, n: u64, cx: &mut Context<Self>) {
        if n % 15 == 0 {
            self.check_theme(cx);
        }
        if self.player.is_some() {
            self.player_tick(cx);
        }
    }

    fn check_theme(&mut self, cx: &mut Context<Self>) {
        let m = std::fs::metadata(&self.args.theme).and_then(|m| m.modified()).ok();
        if m.is_some() && m != self.theme_mtime {
            self.theme_mtime = m;
            match Theme::load(&self.args.theme) {
                Ok(t) => {
                    self.pal = t.palette();
                    self.theme = t;
                    self.fonts = resolve_fonts(&self.theme, cx);
                    self.theme_reloads += 1;
                    eprintln!("theme reloaded ({})", self.theme.name);
                    cx.notify();
                }
                Err(e) => eprintln!("theme reload: {e}"),
            }
        }
    }

    /// Art for an item's series: `<shared>/art/<slug>-poster.jpg` or `-backdrop.jpg`.
    pub fn art(&self, slug: &str, kind: &str) -> PathBuf {
        self.args.shared.join("art").join(format!("{slug}-{kind}.jpg"))
    }

    /// Even episodes play the 4K file, odd ones the 10-bit file.
    pub fn media_for(&self, item: &Item) -> PathBuf {
        if item.episode % 2 == 0 {
            self.args.media.join("teste-4k-hevc-60fps.mkv")
        } else {
            let subs = self.args.media.join("teste-1080p-hevc-10bit-legendas.mkv");
            if subs.exists() { subs } else { self.args.media.join("teste-1080p-hevc-10bit.mkv") }
        }
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.player.is_some() {
            self.player_key(ev, window, cx);
        } else {
            self.home_key(ev, window, cx);
        }
    }
}

pub fn window_hwnd(window: &Window) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

impl Render for LainApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let started = Instant::now();
        if self.hwnd == 0 {
            self.hwnd = window_hwnd(window).unwrap_or(0);
        }
        let body = if self.player.is_some() {
            self.render_player(window, cx).into_any_element()
        } else {
            self.render_home(window, cx).into_any_element()
        };
        self.bench.on_frame(started, window);
        div()
            .id("root")
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .font_family(self.fonts.ui.clone())
            .text_size(px(self.theme.fonts.size_base))
            .text_color(self.pal.text)
            .child(body)
    }
}

fn main() {
    let args = parse_args();
    bench::mark_process_start();
    Application::new().with_assets(Assets).run(move |cx: &mut App| {
        let fonts: Vec<Cow<'static, [u8]>> = FONTS.iter().map(|b| Cow::Borrowed(*b)).collect();
        if let Err(e) = cx.text_system().add_fonts(fonts) {
            eprintln!("bundled fonts: {e}");
        }
        let bounds = Bounds::centered(None, size(px(1440.), px(860.)), cx);
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some("lain".into()), ..Default::default() }),
            window_background: WindowBackgroundAppearance::Opaque,
            window_min_size: Some(size(px(800.), px(500.))),
            ..Default::default()
        };
        let args = args.clone();
        cx.open_window(opts, move |window, cx| cx.new(|cx| LainApp::new(args, window, cx))).expect("open window");
        cx.on_window_closed(|cx| cx.quit()).detach();
        cx.activate(true);
    });
}
