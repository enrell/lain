//! lain desktop prototype on iced (P2b, DESIGN-v2 §8.2 V-13).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod bench;
mod data;
mod home;
mod placement;
mod player;
mod seekbar;
mod strip;
mod theme;
mod ui;
mod video;

use std::path::PathBuf;

use app::{App, Args};
use data::Paths;

/// Optimus / PowerXpress: run on the discrete GPU (see build.rs).
#[cfg(windows)]
#[unsafe(no_mangle)]
#[used]
pub static NvOptimusEnablement: u32 = 1;
#[cfg(windows)]
#[unsafe(no_mangle)]
#[used]
pub static AmdPowerXpressRequestHighPerformance: u32 = 1;

const FONTS: [&[u8]; 5] = [
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../assets/fonts/Fraunces-Regular.ttf"),
    include_bytes!("../assets/fonts/Fraunces-SemiBold.ttf"),
];

fn usage() -> ! {
    eprintln!(
        "lain-iced [--theme <file>] [--server <url>] [--shared <dir>] [--media <dir>] [--bench <out.json>] [--shots <dir>] [--mute] [--window-pos X,Y] [--window-size WxH]"
    );
    std::process::exit(2)
}

fn parse_args() -> Args {
    let mut shared = PathBuf::from(r"D:\lain-proto\shared");
    let mut media = PathBuf::from(r"D:\lain-proto\media");
    let mut server = "http://127.0.0.1:9360".to_string();
    let mut theme = None;
    let mut bench = None;
    let mut shots = None;
    let mut pos = None;
    let mut size = None;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut val = || it.next().unwrap_or_else(|| usage());
        match a.as_str() {
            "--theme" => theme = Some(PathBuf::from(val())),
            "--server" => server = val(),
            "--shared" => shared = PathBuf::from(val()),
            "--media" => media = PathBuf::from(val()),
            "--bench" => bench = Some(PathBuf::from(val())),
            "--shots" => shots = Some(PathBuf::from(val())),
            "--mute" => player::MUTE.store(true, std::sync::atomic::Ordering::Relaxed),
            "--window-pos" => pos = Some(placement::parse_pair(&val(), ',').unwrap_or_else(|| usage())),
            "--window-size" => size = Some(placement::parse_pair(&val(), 'x').unwrap_or_else(|| usage())),
            _ => usage(),
        }
    }
    let window = match (pos, size) {
        (Some((x, y)), Some((w, h))) => Some(placement::Rect { x, y, w, h }),
        (Some((x, y)), None) => Some(placement::Rect { x, y, w: 1600, h: 900 }),
        _ => None,
    };
    let theme = theme.unwrap_or_else(|| shared.join("theme.json"));
    Args { paths: Paths { server, shared, media, theme }, bench, shots, window }
}

fn main() -> iced::Result {
    let args = parse_args();
    placement::log(&format!("start: args={:?} window={:?}", std::env::args().skip(1).collect::<Vec<_>>(), args.window));
    // The zero-copy video path needs wgpu on D3D12 (shared NT handles are
    // opened with ID3D12Device::OpenSharedHandle).
    if std::env::var_os("WGPU_BACKEND").is_none() {
        // SAFETY: single-threaded at this point.
        unsafe { std::env::set_var("WGPU_BACKEND", "dx12") };
    }
    let trace = std::env::var_os("LAIN_WINDOW_TRACE").is_some();
    placement::guard_windows(args.window, trace);
    let hidden = args.window.is_some();
    let (position, size) = match args.window {
        Some(r) => {
            let (p, s) = placement::creation_logical(r);
            (iced::window::Position::Specific(p), s)
        }
        None => (iced::window::Position::Default, iced::Size::new(1600.0, 900.0)),
    };
    let mut application = iced::application(move || App::boot(take_args()), App::update, App::view)
        .title(App::title)
        .subscription(App::subscription)
        .window(iced::window::Settings {
            size,
            position,
            min_size: Some(iced::Size::new(800.0, 500.0)),
            // With --window, created hidden and shown in place (placement.rs).
            visible: !hidden,
            ..Default::default()
        })
        .antialiasing(true)
        .default_font(iced::Font::with_name("Inter"));
    for f in FONTS {
        application = application.font(f);
    }
    ARGS.with(|a| *a.borrow_mut() = Some(args));
    application.run()
}

thread_local! {
    static ARGS: std::cell::RefCell<Option<Args>> = const { std::cell::RefCell::new(None) };
}

fn take_args() -> Args {
    ARGS.with(|a| a.borrow_mut().take()).expect("boot runs once")
}
