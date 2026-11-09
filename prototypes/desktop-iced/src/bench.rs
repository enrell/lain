//! `--bench <out.json>`: the unattended run the coordinator compares.
//! Frame times come from iced's own `window::frames()` (the
//! `RedrawRequested` instants the runtime hands the app), so they are the
//! intervals between real redraws, vsync included.

use std::path::PathBuf;
use std::time::Instant;

use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    /// Waiting for Home with art.
    Home,
    /// Home is up; settle, then sample memory.
    Settle(Instant),
    /// Auto-scrolling the 500-card rail.
    Scroll(Instant),
    /// Playing one file with controls pinned (0 = 4K, 1 = 10-bit).
    Video(usize, Instant),
    /// Between files: the next one is opening.
    Opening(usize),
    Done,
}

pub struct Bench {
    pub out: PathBuf,
    pub phase: Phase,
    pub cold_start_ms: Option<f64>,
    pub rss_home: Option<f64>,
    pub rss_4k: Option<f64>,
    pub scroll_frames: Vec<f64>,
    pub video_frames: Vec<f64>,
    pub mpv: Vec<Value>,
    pub last_frame: Option<Instant>,
    pub notes: Vec<String>,
    pub px_per_s: f32,
    pub last_log: Option<Instant>,
}

impl Bench {
    pub fn new(out: PathBuf) -> Self {
        Bench {
            out,
            phase: Phase::Home,
            cold_start_ms: None,
            rss_home: None,
            rss_4k: None,
            scroll_frames: Vec::new(),
            video_frames: Vec::new(),
            mpv: Vec::new(),
            last_frame: None,
            notes: Vec::new(),
            px_per_s: 4000.0,
            last_log: None,
        }
    }

    /// One redraw instant; files it under the phase that is running.
    pub fn frame(&mut self, at: Instant) {
        let dt = self.last_frame.map(|l| at.saturating_duration_since(l).as_secs_f64() * 1000.0);
        self.last_frame = Some(at);
        let Some(dt) = dt else { return };
        match self.phase {
            Phase::Scroll(_) => self.scroll_frames.push(dt),
            Phase::Video(_, start) if at.duration_since(start).as_secs_f32() > 1.0 => self.video_frames.push(dt),
            _ => {}
        }
    }

    pub fn write(&self) -> Result<(), String> {
        let v = json!({
            "framework": "iced 0.14 (wgpu 27, DX12)",
            "cold_start_ms": self.cold_start_ms,
            "rss_mb_home": self.rss_home,
            "rss_mb_player_4k": self.rss_4k,
            "scroll_frame_ms": stats(&self.scroll_frames, true),
            "mpv": self.mpv,
            "ui_frame_ms_during_video": stats(&self.video_frames, false),
            "notes": self.notes,
        });
        let s = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
        std::fs::write(&self.out, s).map_err(|e| format!("{}: {e}", self.out.display()))
    }
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let i = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

fn stats(v: &[f64], full: bool) -> Value {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let r = |x: f64| (x * 100.0).round() / 100.0;
    if full {
        json!({"p50": r(pct(&s, 0.5)), "p95": r(pct(&s, 0.95)), "p99": r(pct(&s, 0.99)), "max": r(s.last().copied().unwrap_or(0.0)), "frames": s.len()})
    } else {
        json!({"p50": r(pct(&s, 0.5)), "p95": r(pct(&s, 0.95)), "frames": s.len()})
    }
}

// ---- process numbers (Windows) -------------------------------------------------

#[cfg(windows)]
mod sys {
    #[repr(C)]
    #[derive(Default)]
    struct Pmc {
        cb: u32,
        page_fault_count: u32,
        peak_working_set: usize,
        working_set: usize,
        quota_peak_paged: usize,
        quota_paged: usize,
        quota_peak_nonpaged: usize,
        quota_nonpaged: usize,
        pagefile: usize,
        peak_pagefile: usize,
    }

    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(process: isize, counters: *mut Pmc, cb: u32) -> i32;
        fn GetProcessTimes(process: isize, creation: *mut u64, exit: *mut u64, kernel: *mut u64, user: *mut u64) -> i32;
        fn GetSystemTimePreciseAsFileTime(out: *mut u64);
    }

    /// Working set, in MiB (what Task Manager calls memory).
    pub fn rss_mb() -> Option<f64> {
        let mut c = Pmc { cb: std::mem::size_of::<Pmc>() as u32, ..Default::default() };
        let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
        (ok != 0).then(|| c.working_set as f64 / (1024.0 * 1024.0))
    }

    /// Milliseconds since the OS created this process.
    pub fn since_process_start_ms() -> Option<f64> {
        let (mut cr, mut ex, mut k, mut u, mut now) = (0u64, 0u64, 0u64, 0u64, 0u64);
        let ok = unsafe { GetProcessTimes(GetCurrentProcess(), &mut cr, &mut ex, &mut k, &mut u) };
        if ok == 0 {
            return None;
        }
        unsafe { GetSystemTimePreciseAsFileTime(&mut now) };
        Some(now.saturating_sub(cr) as f64 / 10_000.0)
    }
}

#[cfg(not(windows))]
mod sys {
    pub fn rss_mb() -> Option<f64> {
        None
    }
    pub fn since_process_start_ms() -> Option<f64> {
        None
    }
}

pub use sys::{rss_mb, since_process_start_ms};
