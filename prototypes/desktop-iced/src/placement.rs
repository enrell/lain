//! Window placement on Windows, in physical virtual-desktop pixels.
//!
//! `--window-pos X,Y --window-size WxH`: the window is created hidden,
//! moved/sized here with `SetWindowPos`, and shown with
//! `SW_SHOWNOACTIVATE`, so it never appears on another monitor and never
//! takes focus. iced's own `Position::Specific` is logical and would be
//! converted with the primary monitor's scale before the window exists,
//! which is wrong on a mixed-DPI desk.
//!
//! Fullscreen is done here too, explicitly on one monitor (the one under
//! `--window-pos` when given, else the window's own), because iced's
//! `Mode::Fullscreen` means "the current monitor".

use std::sync::Mutex;

#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

pub fn parse_pair(s: &str, sep: char) -> Option<(i32, i32)> {
    let (a, b) = s.split_once(sep)?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

#[cfg(windows)]
mod win {
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    pub struct R {
        pub l: i32,
        pub t: i32,
        pub r: i32,
        pub b: i32,
    }
    #[repr(C)]
    pub struct MonitorInfo {
        pub cb: u32,
        pub monitor: R,
        pub work: R,
        pub flags: u32,
    }
    #[repr(C)]
    pub struct WindowPlacement {
        pub length: u32,
        pub flags: u32,
        pub show_cmd: u32,
        pub min: Pt,
        pub max: Pt,
        pub normal: R,
    }
    #[repr(C)]
    pub struct Pt {
        pub x: i32,
        pub y: i32,
    }
    unsafe extern "system" {
        pub fn SetWindowPos(hwnd: isize, after: isize, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
        pub fn ShowWindow(hwnd: isize, cmd: i32) -> i32;
        pub fn GetWindowRect(hwnd: isize, r: *mut R) -> i32;
        pub fn GetWindowLongPtrW(hwnd: isize, idx: i32) -> isize;
        pub fn SetWindowLongPtrW(hwnd: isize, idx: i32, v: isize) -> isize;
        pub fn GetDpiForSystem() -> u32;
        pub fn SetWindowPlacement(hwnd: isize, wp: *const WindowPlacement) -> i32;
        pub fn MonitorFromPoint(p: Pt, flags: u32) -> isize;
        pub fn MonitorFromWindow(hwnd: isize, flags: u32) -> isize;
        pub fn GetMonitorInfoW(m: isize, mi: *mut MonitorInfo) -> i32;
    }
    pub const SWP_NOZORDER: u32 = 0x0004;
    pub const SWP_NOACTIVATE: u32 = 0x0010;
    pub const SWP_FRAMECHANGED: u32 = 0x0020;
    pub const SW_SHOWNOACTIVATE: i32 = 4;
    pub const GWL_STYLE: i32 = -16;
    pub const WS_OVERLAPPEDWINDOW: isize = 0x00CF_0000;
    pub const MONITOR_DEFAULTTONEAREST: u32 = 2;
}

#[cfg(windows)]
fn hwnd(window: &dyn iced::window::Window) -> Option<isize> {
    use iced::window::raw_window_handle::RawWindowHandle;
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

#[cfg(windows)]
pub fn place(window: &dyn iced::window::Window, r: Rect, show: bool) {
    use win::*;
    log("place: called");
    let Some(h) = hwnd(window) else {
        log("place: no HWND");
        return;
    };
    unsafe {
        let mut before = R::default();
        GetWindowRect(h, &mut before);
        log(&format!("place: hwnd={h:#x} before=({},{})-({},{}) target={r:?}", before.l, before.t, before.r, before.b));
        // Twice: crossing to a monitor with another DPI makes the window
        // take the system's suggested rect; the second call wins.
        SetWindowPos(h, 0, r.x, r.y, r.w, r.h, SWP_NOZORDER | SWP_NOACTIVATE);
        SetWindowPos(h, 0, r.x, r.y, r.w, r.h, SWP_NOZORDER | SWP_NOACTIVATE);
        if !show {
            log("place: moved while hidden");
            return;
        }
        // Show through SetWindowPlacement, not ShowWindow: a process
        // started with STARTF_USESHOWWINDOW gets its first ShowWindow
        // rewritten to SW_SHOWNORMAL, which restored the window to the
        // default rect on the primary monitor and activated it.
        let wp = WindowPlacement {
            length: std::mem::size_of::<WindowPlacement>() as u32,
            flags: 0,
            show_cmd: SW_SHOWNOACTIVATE as u32,
            min: Pt { x: -1, y: -1 },
            max: Pt { x: -1, y: -1 },
            normal: R { l: r.x, t: r.y, r: r.x + r.w, b: r.y + r.h },
        };
        SetWindowPlacement(h, &wp);
        // The first ShowWindow of a process started with
        // STARTF_USESHOWWINDOW can be rewritten by the shell; pin the rect
        // again once shown.
        SetWindowPos(h, 0, r.x, r.y, r.w, r.h, SWP_NOZORDER | SWP_NOACTIVATE);
        let mut after = R::default();
        GetWindowRect(h, &mut after);
        log(&format!("place: after=({},{})-({},{})", after.l, after.t, after.r, after.b));
    }
}

/// The rect in the logical units winit uses when it positions a new
/// window: it converts with the DPI of the monitor the window is born on
/// (the primary, i.e. the system DPI). Born there already, the window can
/// only ever be visible on the target monitor even if something shows it
/// before `place` runs; `place` then fixes the exact physical rect.
#[cfg(windows)]
pub fn creation_logical(r: Rect) -> (iced::Point, iced::Size) {
    let dpi = unsafe { win::GetDpiForSystem() }.max(96) as f32;
    let k = 96.0 / dpi;
    (iced::Point::new(r.x as f32 * k, r.y as f32 * k), iced::Size::new(r.w as f32 * k, r.h as f32 * k))
}

#[cfg(not(windows))]
pub fn creation_logical(r: Rect) -> (iced::Point, iced::Size) {
    (iced::Point::new(r.x as f32, r.y as f32), iced::Size::new(r.w as f32, r.h as f32))
}

/// Saved (style, rect) while fullscreen.
static SAVED: Mutex<Option<(isize, (i32, i32, i32, i32))>> = Mutex::new(None);

#[cfg(windows)]
pub fn set_fullscreen(window: &dyn iced::window::Window, on: bool, anchor: Option<(i32, i32)>) {
    use win::*;
    let Some(h) = hwnd(window) else { return };
    let mut saved = SAVED.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        if on {
            if saved.is_some() {
                return;
            }
            let style = GetWindowLongPtrW(h, GWL_STYLE);
            let mut wr = R::default();
            GetWindowRect(h, &mut wr);
            *saved = Some((style, (wr.l, wr.t, wr.r - wr.l, wr.b - wr.t)));
            let mon = match anchor {
                Some((x, y)) => MonitorFromPoint(Pt { x, y }, MONITOR_DEFAULTTONEAREST),
                None => MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST),
            };
            let mut mi = MonitorInfo { cb: std::mem::size_of::<MonitorInfo>() as u32, monitor: R::default(), work: R::default(), flags: 0 };
            if GetMonitorInfoW(mon, &mut mi) == 0 {
                return;
            }
            let m = mi.monitor;
            SetWindowLongPtrW(h, GWL_STYLE, style & !WS_OVERLAPPEDWINDOW);
            SetWindowPos(h, 0, m.l, m.t, m.r - m.l, m.b - m.t, SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
        } else if let Some((style, (x, y, w, hh))) = saved.take() {
            SetWindowLongPtrW(h, GWL_STYLE, style);
            SetWindowPos(h, 0, x, y, w, hh, SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
        }
    }
}

#[cfg(not(windows))]
pub fn place(_window: &dyn iced::window::Window, _r: Rect, _show: bool) {}

#[cfg(not(windows))]
pub fn set_fullscreen(_window: &dyn iced::window::Window, _on: bool, _anchor: Option<(i32, i32)>) {
    let _ = &SAVED;
}

/// Append one line to `lain-iced.log` next to the exe (release builds have
/// no console).
pub static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

pub fn log(line: &str) {
    let line = &format!("[{:>6}ms] {line}", START.get_or_init(std::time::Instant::now).elapsed().as_millis());
    use std::io::Write;
    let path = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("lain-iced.log")));
    if let Some(p) = path {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
            let _ = writeln!(f, "{line}");
        }
    }
    eprintln!("{line}");
}

/// winit makes its hidden "Thread Event Target" helper window
/// `WS_VISIBLE` (layered, fully transparent, 16x16 at 0,0 of the primary
/// monitor). Nobody sees it, but it counts as a visible top-level window
/// on the primary screen (launchers and tiling WMs count it). This thread
/// hides it (SW_HIDE) as
/// soon as it exists (polled every millisecond for the first seconds,
/// since it is created inside iced's event-loop setup), and with
/// `trace` logs every visible window of the process.
#[cfg(windows)]
pub fn guard_windows(target: Option<Rect>, trace: bool) {
    unsafe extern "system" {
        fn EnumWindows(f: unsafe extern "system" fn(isize, isize) -> i32, l: isize) -> i32;
        fn GetWindowThreadProcessId(h: isize, pid: *mut u32) -> u32;
        fn IsWindowVisible(h: isize) -> i32;
        fn GetClassNameW(h: isize, buf: *mut u16, n: i32) -> i32;
        fn GetCurrentProcessId() -> u32;
    }
    struct Ctx {
        target: Option<Rect>,
        trace: bool,
    }
    unsafe extern "system" fn cb(h: isize, l: isize) -> i32 {
        let ctx = unsafe { &*(l as *const Ctx) };
        let mut pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(h, &mut pid);
            if pid != GetCurrentProcessId() || IsWindowVisible(h) == 0 {
                return 1;
            }
            let mut buf = [0u16; 128];
            let n = GetClassNameW(h, buf.as_mut_ptr(), 128);
            let class = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
            let mut r = win::R::default();
            win::GetWindowRect(h, &mut r);
            if class.contains("Thread Event Target") {
                // It only exists to receive thread messages; hidden it still does.
                win::ShowWindow(h, 0);
                log(&format!("guard: hid winit helper window {h:#x} at ({},{})", r.l, r.t));
                let _ = ctx.target;
            }
            if ctx.trace {
                log(&format!("trace: t={}ms visible hwnd={h:#x} class={class} rect=({},{})-({},{})", START.get_or_init(std::time::Instant::now).elapsed().as_millis(), r.l, r.t, r.r, r.b));
            }
        }
        1
    }
    std::thread::spawn(move || {
        let ctx = Ctx { target, trace };
        let start = std::time::Instant::now();
        let mut last_trace = start;
        while start.elapsed() < std::time::Duration::from_secs(5) {
            let tr = ctx.trace && last_trace.elapsed() > std::time::Duration::from_millis(5);
            if tr {
                last_trace = std::time::Instant::now();
            }
            let c = Ctx { target: ctx.target, trace: tr };
            unsafe { EnumWindows(cb, &c as *const Ctx as isize) };
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });
}

#[cfg(not(windows))]
pub fn guard_windows(_target: Option<Rect>, _trace: bool) {}
