//! libmpv, embedded with `wid` into a child HWND of the GPUI window.
//!
//! GPUI's Windows renderer draws through DirectComposition with a visual
//! tree that is *topmost* over the window's child windows
//! (`CreateTargetForHwnd(hwnd, true)`) and a premultiplied-alpha swap
//! chain cleared to transparent. So a child window placed under it shows
//! through wherever GPUI paints nothing: mpv renders its own D3D11
//! flip-model swap chain into that child, and GPUI's controls are
//! composited over the video by DWM, with real alpha. No copy of video
//! frames passes through the UI thread.
//!
//! The host window is created disabled: hit testing skips disabled child
//! windows, so mouse input over the video still reaches GPUI's window.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{BLACK_BRUSH, GetStockObject, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, EnumChildWindows, GetClientRect,
    HTTRANSPARENT, HWND_BOTTOM, RegisterClassW, SW_HIDE, SW_SHOWNA, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos,
    ShowWindow, WM_NCHITTEST, WNDCLASSW, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_DISABLED,
    WS_EX_NOACTIVATE,
};
use windows::core::{BOOL, PCWSTR, w};

// ---- FFI (client.h, API 2.x) ----------------------------------------------

#[repr(C)]
struct MpvHandle {
    _p: [u8; 0],
}

#[repr(C)]
struct MpvEvent {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

#[repr(C)]
struct MpvEventProperty {
    name: *const c_char,
    format: c_int,
    data: *mut c_void,
}

#[repr(C)]
struct MpvEventEndFile {
    reason: c_int,
    error: c_int,
}

const FMT_NONE: c_int = 0;
const FMT_STRING: c_int = 1;
const FMT_FLAG: c_int = 3;
const FMT_INT64: c_int = 4;
const FMT_DOUBLE: c_int = 5;

const EV_SHUTDOWN: c_int = 1;
const EV_END_FILE: c_int = 7;
const EV_FILE_LOADED: c_int = 8;
const EV_VIDEO_RECONFIG: c_int = 17;
const EV_PROPERTY_CHANGE: c_int = 22;

#[link(name = "mpv")]
unsafe extern "C" {
    fn mpv_create() -> *mut MpvHandle;
    fn mpv_initialize(ctx: *mut MpvHandle) -> c_int;
    fn mpv_terminate_destroy(ctx: *mut MpvHandle);
    fn mpv_set_option_string(ctx: *mut MpvHandle, name: *const c_char, data: *const c_char) -> c_int;
    fn mpv_command(ctx: *mut MpvHandle, args: *mut *const c_char) -> c_int;
    fn mpv_command_async(ctx: *mut MpvHandle, reply: u64, args: *mut *const c_char) -> c_int;
    fn mpv_set_property_string(ctx: *mut MpvHandle, name: *const c_char, data: *const c_char) -> c_int;
    fn mpv_get_property(ctx: *mut MpvHandle, name: *const c_char, format: c_int, data: *mut c_void) -> c_int;
    fn mpv_get_property_string(ctx: *mut MpvHandle, name: *const c_char) -> *mut c_char;
    fn mpv_observe_property(ctx: *mut MpvHandle, reply: u64, name: *const c_char, format: c_int) -> c_int;
    fn mpv_wait_event(ctx: *mut MpvHandle, timeout: f64) -> *mut MpvEvent;
    fn mpv_free(data: *mut c_void);
    fn mpv_error_string(error: c_int) -> *const c_char;
}

// ---- state shared with the UI ---------------------------------------------

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Track {
    pub id: i64,
    pub kind: String,
    pub label: String,
    pub selected: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlayerState {
    pub time: f64,
    pub duration: f64,
    pub paused: bool,
    pub volume: f64,
    pub muted: bool,
    /// Seconds of demuxed data ahead of the start (`demuxer-cache-time`).
    pub cache_end: f64,
    pub tracks: Vec<Track>,
    pub loaded: bool,
    /// The VO is configured: the first frame is (about to be) on screen.
    pub has_video: bool,
    pub eof: bool,
    pub error: Option<String>,
}

pub struct Mpv {
    h: *mut MpvHandle,
    pub state: Arc<Mutex<PlayerState>>,
    events: Option<JoinHandle<()>>,
}

// libmpv's client API is thread-safe.
unsafe impl Send for Mpv {}
unsafe impl Sync for Mpv {}

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

fn err_str(code: c_int) -> String {
    unsafe { CStr::from_ptr(mpv_error_string(code)).to_string_lossy().into_owned() }
}

struct RawHandle(*mut MpvHandle);
unsafe impl Send for RawHandle {}

impl Mpv {
    /// Creates a player rendering into `wid` (an HWND), with `options`
    /// applied before initialization.
    pub fn new(wid: isize, options: &[(&str, &str)]) -> Result<Mpv, String> {
        let h = unsafe { mpv_create() };
        if h.is_null() {
            return Err("mpv_create failed".into());
        }
        let wid = wid.to_string();
        let base: &[(&str, &str)] = &[
            ("wid", &wid),
            ("vo", "gpu-next"),
            ("gpu-api", "d3d11"),
            ("hwdec", "auto-safe"),
            ("keep-open", "yes"),
            ("idle", "yes"),
            ("force-window", "yes"),
            ("osc", "no"),
            ("osd-level", "0"),
            ("input-default-bindings", "no"),
            ("input-vo-keyboard", "no"),
            ("input-cursor", "no"),
            ("cursor-autohide", "no"),
            ("terminal", "no"),
            ("background-color", "#000000"),
        ];
        for (k, v) in base.iter().chain(options.iter()) {
            let r = unsafe { mpv_set_option_string(h, cstr(k).as_ptr(), cstr(v).as_ptr()) };
            if r < 0 {
                eprintln!("mpv option {k}={v}: {}", err_str(r));
            }
        }
        let r = unsafe { mpv_initialize(h) };
        if r < 0 {
            unsafe { mpv_terminate_destroy(h) };
            return Err(format!("mpv_initialize: {}", err_str(r)));
        }
        let props: &[(&str, c_int)] = &[
            ("time-pos", FMT_DOUBLE),
            ("duration", FMT_DOUBLE),
            ("pause", FMT_FLAG),
            ("volume", FMT_DOUBLE),
            ("mute", FMT_FLAG),
            ("demuxer-cache-time", FMT_DOUBLE),
            ("track-list", FMT_NONE),
            ("eof-reached", FMT_FLAG),
            ("vo-configured", FMT_FLAG),
        ];
        for (i, (name, fmt)) in props.iter().enumerate() {
            unsafe { mpv_observe_property(h, i as u64 + 1, cstr(name).as_ptr(), *fmt) };
        }
        let state = Arc::new(Mutex::new(PlayerState { volume: 100.0, ..Default::default() }));
        let st = state.clone();
        let raw = RawHandle(h);
        let events = std::thread::Builder::new()
            .name("mpv-events".into())
            .spawn(move || {
                let raw = raw;
                event_loop(raw.0, &st)
            })
            .map_err(|e| e.to_string())?;
        Ok(Mpv { h, state, events: Some(events) })
    }

    pub fn command(&self, args: &[&str]) -> Result<(), String> {
        let owned: Vec<CString> = args.iter().map(|a| cstr(a)).collect();
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        let r = unsafe { mpv_command(self.h, ptrs.as_mut_ptr()) };
        if r < 0 { Err(err_str(r)) } else { Ok(()) }
    }

    /// Fire-and-forget command (seeks: never block the UI thread).
    pub fn command_async(&self, args: &[&str]) {
        let owned: Vec<CString> = args.iter().map(|a| cstr(a)).collect();
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        unsafe { mpv_command_async(self.h, 0, ptrs.as_mut_ptr()) };
    }

    pub fn set(&self, name: &str, value: &str) {
        unsafe { mpv_set_property_string(self.h, cstr(name).as_ptr(), cstr(value).as_ptr()) };
    }

    pub fn get_string(&self, name: &str) -> Option<String> {
        get_string(self.h, name)
    }

    pub fn get_f64(&self, name: &str) -> Option<f64> {
        let mut v = 0f64;
        let r = unsafe { mpv_get_property(self.h, cstr(name).as_ptr(), FMT_DOUBLE, &mut v as *mut f64 as *mut c_void) };
        (r >= 0).then_some(v)
    }

    pub fn get_i64(&self, name: &str) -> Option<i64> {
        let mut v = 0i64;
        let r = unsafe { mpv_get_property(self.h, cstr(name).as_ptr(), FMT_INT64, &mut v as *mut i64 as *mut c_void) };
        (r >= 0).then_some(v)
    }

    pub fn snapshot(&self) -> PlayerState {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        // The event thread leaves on MPV_EVENT_SHUTDOWN; only then may the
        // handle be destroyed (mpv_wait_event must not race the destroy).
        let _ = self.command(&["quit"]);
        if let Some(t) = self.events.take() {
            let _ = t.join();
        }
        unsafe { mpv_terminate_destroy(self.h) };
    }
}

fn get_string(h: *mut MpvHandle, name: &str) -> Option<String> {
    let p = unsafe { mpv_get_property_string(h, cstr(name).as_ptr()) };
    if p.is_null() {
        return None;
    }
    let s = unsafe { CStr::from_ptr(p).to_string_lossy().into_owned() };
    unsafe { mpv_free(p as *mut c_void) };
    Some(s)
}

fn read_tracks(h: *mut MpvHandle) -> Vec<Track> {
    let n: i64 = get_string(h, "track-list/count").and_then(|s| s.parse().ok()).unwrap_or(0);
    (0..n)
        .map(|i| {
            let g = |k: &str| get_string(h, &format!("track-list/{i}/{k}")).unwrap_or_default();
            let (title, lang, codec) = (g("title"), g("lang"), g("codec"));
            let mut label = match (title.is_empty(), lang.is_empty()) {
                (false, false) => format!("{title} ({lang})"),
                (false, true) => title,
                (true, false) => lang,
                (true, true) => format!("Track {}", g("id")),
            };
            if !codec.is_empty() {
                label = format!("{label} · {codec}");
            }
            Track { id: g("id").parse().unwrap_or(0), kind: g("type"), label, selected: g("selected") == "yes" }
        })
        .collect()
}

fn event_loop(h: *mut MpvHandle, state: &Mutex<PlayerState>) {
    loop {
        let ev = unsafe { &*mpv_wait_event(h, 1.0) };
        let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
        match ev.event_id {
            EV_SHUTDOWN => return,
            EV_FILE_LOADED => {
                st.loaded = true;
                st.eof = false;
                st.error = None;
            }
            EV_VIDEO_RECONFIG => st.has_video = true,
            EV_END_FILE => {
                let ef = unsafe { &*(ev.data as *const MpvEventEndFile) };
                if ef.reason == 4 {
                    st.error = Some(err_str(ef.error));
                }
            }
            EV_PROPERTY_CHANGE => {
                let p = unsafe { &*(ev.data as *const MpvEventProperty) };
                let name = unsafe { CStr::from_ptr(p.name) }.to_bytes();
                let dbl = || if p.format == FMT_DOUBLE { Some(unsafe { *(p.data as *const f64) }) } else { None };
                let flag = || if p.format == FMT_FLAG { Some(unsafe { *(p.data as *const c_int) } != 0) } else { None };
                match name {
                    b"time-pos" => st.time = dbl().unwrap_or(st.time),
                    b"duration" => st.duration = dbl().unwrap_or(st.duration),
                    b"pause" => st.paused = flag().unwrap_or(st.paused),
                    b"volume" => st.volume = dbl().unwrap_or(st.volume),
                    b"mute" => st.muted = flag().unwrap_or(st.muted),
                    b"demuxer-cache-time" => st.cache_end = dbl().unwrap_or(0.0),
                    b"eof-reached" => st.eof = flag().unwrap_or(false),
                    b"vo-configured" => st.has_video = flag().unwrap_or(st.has_video),
                    b"track-list" => {
                        drop(st);
                        let tracks = read_tracks(h);
                        state.lock().unwrap_or_else(|e| e.into_inner()).tracks = tracks;
                        continue;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

// Silence an unused-format warning; strings are read with get_property_string.
const _: c_int = FMT_STRING;

// ---- the child window mpv renders into ------------------------------------

unsafe extern "system" fn host_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_NCHITTEST {
        return LRESULT(HTTRANSPARENT as isize);
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

/// A disabled, non-activating child window covering the parent's client
/// area, at the bottom of the child z-order.
pub struct VideoHost {
    pub hwnd: HWND,
    parent: HWND,
    last: (i32, i32),
}

impl VideoHost {
    pub fn new(parent: isize) -> Result<VideoHost, String> {
        let parent = HWND(parent as *mut c_void);
        unsafe {
            let hinst = GetModuleHandleW(PCWSTR::null()).map_err(|e| e.to_string())?;
            let class = w!("lain-video-host");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(host_proc),
                hInstance: hinst.into(),
                lpszClassName: class,
                hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
                ..Default::default()
            };
            RegisterClassW(&wc); // fails harmlessly when already registered
            let mut rc = RECT::default();
            let _ = GetClientRect(parent, &mut rc);
            let hwnd = CreateWindowExW(
                WS_EX_NOACTIVATE,
                class,
                w!(""),
                WS_CHILD | WS_CLIPSIBLINGS | WS_CLIPCHILDREN | WS_DISABLED,
                0,
                0,
                rc.right.max(1),
                rc.bottom.max(1),
                Some(parent),
                None,
                Some(hinst.into()),
                None,
            )
            .map_err(|e| e.to_string())?;
            let _ = SetWindowPos(hwnd, Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOACTIVATE | windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE | windows::Win32::UI::WindowsAndMessaging::SWP_NOSIZE);
            let _ = ShowWindow(hwnd, SW_SHOWNA);
            Ok(VideoHost { hwnd, parent, last: (rc.right, rc.bottom) })
        }
    }

    pub fn wid(&self) -> isize {
        self.hwnd.0 as isize
    }

    /// Keeps the host the size of the parent's client area and disables
    /// mpv's own window inside it (mpv creates it on its thread).
    pub fn sync(&mut self) {
        unsafe {
            let mut rc = RECT::default();
            let _ = GetClientRect(self.parent, &mut rc);
            let size = (rc.right.max(1), rc.bottom.max(1));
            if size != self.last {
                self.last = size;
                let _ = SetWindowPos(self.hwnd, None, 0, 0, size.0, size.1, SWP_NOACTIVATE | SWP_NOZORDER);
            }
            let _ = EnumChildWindows(Some(self.hwnd), Some(disable_child), LPARAM(0));
        }
    }

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }
}

unsafe extern "system" fn disable_child(hwnd: HWND, _: LPARAM) -> BOOL {
    unsafe {
        let _ = EnableWindow(hwnd, false);
    }
    BOOL(1)
}

impl Drop for VideoHost {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
