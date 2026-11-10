//! lain desktop app v1 on GPUI (P3, DESIGN-v2 §10, D-180): the parity
//! checklist of §8.4 over `lain-client`, with themes tier 1 (§8.3).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod backend;
mod config;
mod demo;
mod engine;
mod ext;
mod images;
mod layout;
#[cfg(windows)]
mod mpv;
#[cfg(unix)]
mod mpv_ipc;
mod platform;
mod screens;
mod theme;
mod ui;
mod zip;

use std::borrow::Cow;
use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{
    App, Application, AssetSource, Bounds, SharedString, TitlebarOptions, WindowBackgroundAppearance, WindowBounds,
    WindowOptions, point, px, size,
};

// ---- assets: icons (theme overrides first) and the bundled fonts ----------------

struct Assets;

macro_rules! icons {
    ($($n:literal),* $(,)?) => { &[$((concat!("icons/", $n, ".svg"), include_bytes!(concat!("../assets/icons/", $n, ".svg")) as &[u8])),*] };
}

const ICONS: &[(&str, &[u8])] = icons![
    "alert",
    "audio",
    "back",
    "book",
    "check",
    "chevron",
    "clock",
    "close",
    "command",
    "download",
    "empty",
    "external",
    "eye",
    "folder",
    "fullscreen",
    "home",
    "key",
    "library",
    "lock",
    "mute",
    "next",
    "off",
    "offline",
    "palette",
    "pause",
    "play",
    "plus",
    "refresh",
    "search",
    "server",
    "settings",
    "subtitles",
    "trash",
    "unlock",
    "user",
    "volume",
    "windowed",
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
        // `?g=<n>` is the theme generation (cache busting on reload).
        let path = path.split('?').next().unwrap_or(path);
        if let Some(b) = theme::icon_override(path) {
            return Ok(Some(Cow::Owned(b)));
        }
        Ok(ICONS.iter().find(|(p, _)| *p == path).map(|(_, b)| Cow::Borrowed(*b)))
    }
    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(ICONS.iter().filter(|(p, _)| p.starts_with(path)).map(|(p, _)| SharedString::from(*p)).collect())
    }
}

// ---- command line --------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Args {
    /// Client state: sessions, the client db, offline copies, themes,
    /// `desktop.json` (default: `lain_client::client_dir()`).
    pub client_dir: PathBuf,
    pub server: Option<String>,
    /// A theme folder to use instead of the configured one (theme work).
    pub theme: Option<PathBuf>,
    /// In-app automation for screenshots and tests (see `demo.rs`).
    pub demo: Option<String>,
    /// Write a PNG of this window when the demo script says `shot`.
    pub shot: Option<PathBuf>,
    /// Silent playback (`ao=null`): every automated run passes it.
    pub mute: bool,
    /// Exact window placement in virtual-desktop pixels; the window is
    /// then shown without activation.
    pub window_pos: Option<(i32, i32)>,
    pub window_size: Option<(i32, i32)>,
}

fn usage() -> ! {
    eprintln!(
        "lain-desktop [--server URL] [--client-dir DIR] [--theme DIR] [--mute]
             [--window-pos X,Y] [--window-size WxH] [--log FILE]
             [--demo STEPS] [--shot FILE.png]"
    );
    std::process::exit(2)
}

fn parse_args() -> Args {
    let mut a = Args {
        client_dir: lain_client::client_dir().unwrap_or_else(|| std::env::temp_dir().join("lain-client")),
        server: None,
        theme: None,
        demo: None,
        shot: None,
        mute: false,
        window_pos: None,
        window_size: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--client-dir" => a.client_dir = it.next().map(PathBuf::from).unwrap_or_else(|| usage()),
            "--server" => a.server = it.next(),
            "--theme" => a.theme = it.next().map(PathBuf::from),
            "--demo" => a.demo = it.next(),
            "--shot" => a.shot = it.next().map(PathBuf::from),
            "--mute" => a.mute = true,
            "--log" => {
                if let Some(path) = it.next() {
                    platform::redirect_stderr(&path);
                }
            }
            "--window-pos" => {
                a.window_pos = it.next().and_then(|v| {
                    let (x, y) = v.split_once(',')?;
                    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
                })
            }
            "--window-size" => {
                a.window_size = it.next().and_then(|v| {
                    let (w, h) = v.split_once('x')?;
                    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
                })
            }
            "-h" | "--help" => usage(),
            other => eprintln!("unknown argument {other}"),
        }
    }
    if a.demo.is_some() {
        a.mute = true;
    }
    a
}

fn main() {
    let args = parse_args();
    Application::new().with_assets(Assets).run(move |cx: &mut App| {
        let fonts: Vec<Cow<'static, [u8]>> = FONTS.iter().map(|b| Cow::Borrowed(*b)).collect();
        if let Err(e) = cx.text_system().add_fonts(fonts) {
            eprintln!("bundled fonts: {e}");
        }
        let placed = args.window_pos.is_some();
        let (bounds, display_id) = match args.window_pos {
            // Test runs: exactly where asked, on the display that holds that
            // point, created hidden and then shown without activation.
            Some((x, y)) => {
                let (w, h) = args.window_size.unwrap_or((1440, 860));
                let display = cx
                    .displays()
                    .into_iter()
                    .find(|d| {
                        let b = d.bounds();
                        let (l, t) = (f32::from(b.origin.x), f32::from(b.origin.y));
                        let (r, bt) = (l + f32::from(b.size.width), t + f32::from(b.size.height));
                        (x as f32) >= l && (x as f32) < r && (y as f32) >= t && (y as f32) < bt
                    })
                    .map(|d| d.id());
                (Bounds { origin: point(px(x as f32), px(y as f32)), size: size(px(w as f32), px(h as f32)) }, display)
            }
            None => (Bounds::centered(None, size(px(1440.), px(880.)), cx), None),
        };
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            display_id,
            show: !placed || !cfg!(windows),
            focus: !placed,
            titlebar: Some(TitlebarOptions { title: Some("lain".into()), ..Default::default() }),
            window_background: WindowBackgroundAppearance::Opaque,
            window_min_size: Some(size(px(900.), px(560.))),
            app_id: Some("lain-desktop".into()),
            ..Default::default()
        };
        let args = args.clone();
        cx.open_window(opts, move |window, cx| {
            platform::place_early(window, &args);
            cx.new(|cx| app::LainApp::new(args, window, cx))
        })
        .expect("open window");
        cx.on_window_closed(|cx| cx.quit()).detach();
        if !placed {
            cx.activate(true);
        }
    });
}
