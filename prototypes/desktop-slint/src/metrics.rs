//! Frame timing from Slint's rendering notifier, process age and RSS
//! (Win32, hand-declared).

use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Scroll,
    Video4k,
    Video10,
}

#[derive(Default)]
pub struct Frames {
    pub phase: Option<Phase>,
    last_end: Option<Instant>,
    begin: Option<Instant>,
    /// (phase, interval since the previous frame, CPU time of this frame) in ms
    pub samples: Vec<(Phase, f64, f64)>,
    pub count: u64,
}

impl Frames {
    pub fn before(&mut self) {
        self.begin = Some(Instant::now());
    }
    pub fn after(&mut self) {
        let now = Instant::now();
        self.count += 1;
        if let (Some(p), Some(last), Some(b)) = (self.phase, self.last_end, self.begin) {
            self.samples.push((p, (now - last).as_secs_f64() * 1000.0, (now - b).as_secs_f64() * 1000.0));
        }
        self.last_end = Some(now);
    }
    pub fn set_phase(&mut self, p: Option<Phase>) {
        self.phase = p;
        self.last_end = None;
    }
    pub fn of(&self, p: Phase, cpu: bool) -> Vec<f64> {
        self.samples.iter().filter(|s| s.0 == p).map(|s| if cpu { s.2 } else { s.1 }).collect()
    }
}

pub fn pct(v: &[f64], q: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    let i = ((s.len() - 1) as f64 * q).round() as usize;
    (s[i] * 100.0).round() / 100.0
}

pub fn stats(v: &[f64]) -> serde_json::Value {
    serde_json::json!({
        "p50": pct(v, 0.5), "p95": pct(v, 0.95), "p99": pct(v, 0.99),
        "max": pct(v, 1.0), "frames": v.len(),
    })
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    #[repr(C)]
    #[derive(Default)]
    pub struct FileTime {
        pub lo: u32,
        pub hi: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    pub struct MemCounters {
        pub cb: u32,
        pub page_fault_count: u32,
        pub peak_working_set: usize,
        pub working_set: usize,
        pub quota_peak_paged: usize,
        pub quota_paged: usize,
        pub quota_peak_nonpaged: usize,
        pub quota_nonpaged: usize,
        pub pagefile: usize,
        pub peak_pagefile: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn GetCurrentProcess() -> *mut c_void;
        pub fn GetProcessTimes(
            p: *mut c_void,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
        pub fn GetSystemTimePreciseAsFileTime(t: *mut FileTime);
        pub fn SetStdHandle(which: u32, h: *mut c_void) -> i32;
        pub fn K32GetProcessMemoryInfo(p: *mut c_void, c: *mut MemCounters, cb: u32) -> i32;
    }
    pub fn ft(f: &FileTime) -> u64 {
        ((f.hi as u64) << 32) | f.lo as u64
    }
}

/// Milliseconds since the OS created this process.
#[cfg(windows)]
pub fn process_age_ms() -> f64 {
    use win::*;
    unsafe {
        let (mut c, mut e, mut k, mut u, mut now) =
            (FileTime::default(), FileTime::default(), FileTime::default(), FileTime::default(), FileTime::default());
        GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u);
        GetSystemTimePreciseAsFileTime(&mut now);
        (ft(&now).saturating_sub(ft(&c))) as f64 / 10_000.0
    }
}

#[cfg(windows)]
pub fn rss_mb() -> f64 {
    use win::*;
    unsafe {
        let mut m = MemCounters { cb: std::mem::size_of::<MemCounters>() as u32, ..Default::default() };
        K32GetProcessMemoryInfo(GetCurrentProcess(), &mut m, m.cb);
        (m.working_set as f64 / 1048576.0 * 10.0).round() / 10.0
    }
}

/// winit's per-thread message window ("Winit Thread Event Target") is
/// created WS_VISIBLE at 0,0 (16x16, on the primary monitor). It draws
/// nothing, but window-placement checks see it; hide it.
#[cfg(windows)]
pub fn hide_winit_event_target() {
    use std::ffi::c_void;
    #[link(name = "user32")]
    unsafe extern "system" {
        fn EnumThreadWindows(tid: u32, f: unsafe extern "system" fn(*mut c_void, isize) -> i32, l: isize) -> i32;
        fn GetClassNameW(h: *mut c_void, s: *mut u16, n: i32) -> i32;
        fn ShowWindow(h: *mut c_void, cmd: i32) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThreadId() -> u32;
    }
    unsafe extern "system" fn each(h: *mut c_void, _: isize) -> i32 {
        let mut buf = [0u16; 64];
        let n = unsafe { GetClassNameW(h, buf.as_mut_ptr(), 64) };
        if String::from_utf16_lossy(&buf[..n.max(0) as usize]) == "Winit Thread Event Target" {
            unsafe { ShowWindow(h, 0) };
        }
        1
    }
    unsafe { EnumThreadWindows(GetCurrentThreadId(), each, 0) };
}
#[cfg(not(windows))]
pub fn hide_winit_event_target() {}

/// Scale factor (DPI / 96) of the monitor holding the physical point.
#[cfg(windows)]
pub fn monitor_scale_at(x: i32, y: i32) -> f32 {
    use std::ffi::c_void;
    #[repr(C)]
    struct Pt {
        x: i32,
        y: i32,
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MonitorFromPoint(p: Pt, flags: u32) -> *mut c_void;
    }
    #[link(name = "shcore")]
    unsafe extern "system" {
        fn GetDpiForMonitor(m: *mut c_void, kind: i32, dx: *mut u32, dy: *mut u32) -> i32;
    }
    unsafe {
        let m = MonitorFromPoint(Pt { x, y }, 2);
        let (mut dx, mut dy) = (96u32, 96u32);
        if GetDpiForMonitor(m, 0, &mut dx, &mut dy) == 0 { dx as f32 / 96.0 } else { 1.0 }
    }
}
#[cfg(not(windows))]
pub fn monitor_scale_at(_: i32, _: i32) -> f32 {
    1.0
}

/// Sets the window's normal (restored) placement, so the first
/// ShowWindow cannot put it anywhere else.
#[cfg(windows)]
pub fn set_normal_rect(hwnd: isize, x: i32, y: i32, w: i32, h: i32) {
    use std::ffi::c_void;
    #[repr(C)]
    struct Placement {
        len: u32,
        flags: u32,
        show: u32,
        min: [i32; 2],
        max: [i32; 2],
        normal: [i32; 4],
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetWindowPlacement(h: *mut c_void, p: *mut Placement) -> i32;
        fn SetWindowPlacement(h: *mut c_void, p: *const Placement) -> i32;
    }
    let mut p = Placement { len: std::mem::size_of::<Placement>() as u32, flags: 0, show: 0, min: [0; 2], max: [0; 2], normal: [0; 4] };
    unsafe {
        let hw = hwnd as *mut c_void;
        GetWindowPlacement(hw, &mut p);
        p.normal = [x, y, x + w, y + h];
        SetWindowPlacement(hw, &p);
    }
}
#[cfg(not(windows))]
pub fn set_normal_rect(_: isize, _: i32, _: i32, _: i32, _: i32) {}

/// Points this process's stderr (eprintln!) at a file.
#[cfg(windows)]
pub fn redirect_stderr(path: &std::path::Path) {
    use std::os::windows::io::IntoRawHandle;
    if let Ok(f) = std::fs::File::create(path) {
        let h = f.into_raw_handle();
        unsafe { win::SetStdHandle(-12i32 as u32, h) };
    }
}
#[cfg(not(windows))]
pub fn redirect_stderr(_: &std::path::Path) {}

#[cfg(not(windows))]
pub fn process_age_ms() -> f64 {
    0.0
}
#[cfg(not(windows))]
pub fn rss_mb() -> f64 {
    0.0
}
