//! `--bench <out.json>`: an unattended run that logs in, waits for Home
//! with art, scrolls the 500-card rail end to end at a fixed speed, plays
//! the 4K file and the 10-bit file for 20 s each with the controls shown,
//! writes the numbers and exits.
//!
//! Frame times are the intervals between successive GPUI frames (one
//! `render` of the root view per presented frame) while the app keeps
//! requesting animation frames, so a slow frame shows up as a long gap.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use gpui::{AsyncWindowContext, WeakEntity, Window};
use serde_json::{Value, json};

use crate::LainApp;
use crate::home::{BenchScroll, RailKind};

static START: OnceLock<(Instant, f64)> = OnceLock::new();

/// Records `main`'s start and how long the process had existed by then.
pub fn mark_process_start() {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) as f64 / 10_000.;
    let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    let pre = unsafe {
        if GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u).is_ok() {
            (ft(GetSystemTimePreciseAsFileTime()) - ft(c)).max(0.)
        } else {
            0.
        }
    };
    let _ = START.set((Instant::now(), pre));
}

/// Milliseconds since the process was created.
pub fn since_start_ms() -> f64 {
    START.get().map(|(t, pre)| t.elapsed().as_secs_f64() * 1000. + pre).unwrap_or(0.)
}

#[repr(C)]
#[derive(Default)]
struct ProcessMemoryCounters {
    cb: u32,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn K32GetProcessMemoryInfo(process: isize, counters: *mut ProcessMemoryCounters, cb: u32) -> i32;
    fn GetCurrentProcess() -> isize;
}

/// Working set in MB.
pub fn rss_mb() -> f64 {
    let mut c = ProcessMemoryCounters { cb: std::mem::size_of::<ProcessMemoryCounters>() as u32, ..Default::default() };
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
    if ok != 0 { c.working_set_size as f64 / (1024. * 1024.) } else { 0. }
}

#[derive(Default)]
struct Rec {
    last: Option<Instant>,
    intervals: Vec<f64>,
    build: Vec<f64>,
}

pub struct Bench {
    out: Option<PathBuf>,
    pub cold_start_ms: Option<f64>,
    rec: Option<Rec>,
}

fn stats(v: &[f64]) -> Value {
    if v.is_empty() {
        return json!({"frames": 0});
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let q = |p: f64| s[((s.len() - 1) as f64 * p).round() as usize];
    let r = |x: f64| (x * 100.).round() / 100.;
    json!({"p50": r(q(0.5)), "p95": r(q(0.95)), "p99": r(q(0.99)), "max": r(*s.last().unwrap()), "frames": s.len()})
}

impl Bench {
    pub fn new(out: Option<PathBuf>) -> Bench {
        Bench { out, cold_start_ms: None, rec: None }
    }

    pub fn enabled(&self) -> bool {
        self.out.is_some()
    }

    pub fn home_art_ready(&mut self) {
        if self.cold_start_ms.is_none() {
            self.cold_start_ms = Some(since_start_ms());
        }
    }

    fn start(&mut self) {
        self.rec = Some(Rec::default());
    }

    fn stop(&mut self) -> Rec {
        self.rec.take().unwrap_or_default()
    }

    /// Called at the end of every root render.
    pub fn on_frame(&mut self, started: Instant, window: &mut Window) {
        if let Some(r) = self.rec.as_mut() {
            let now = Instant::now();
            if let Some(last) = r.last {
                r.intervals.push((now - last).as_secs_f64() * 1000.);
                r.build.push((now - started).as_secs_f64() * 1000.);
            }
            r.last = Some(now);
            window.request_animation_frame();
        }
    }
}

async fn sleep(cx: &mut AsyncWindowContext, ms: u64) {
    cx.background_executor().timer(Duration::from_millis(ms)).await;
}

/// Waits until `f` holds, polling every 50 ms, for at most `secs`.
async fn wait_for(
    this: &WeakEntity<LainApp>,
    cx: &mut AsyncWindowContext,
    secs: u64,
    f: impl Fn(&LainApp) -> bool,
) -> bool {
    let until = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < until {
        if this.update(cx, |app, _| f(app)).unwrap_or(false) {
            return true;
        }
        sleep(cx, 50).await;
    }
    false
}

pub async fn run(this: WeakEntity<LainApp>, cx: &mut AsyncWindowContext) {
    let mut out = json!({"framework": "gpui", "gpui": "0.2.2 (crates.io)", "renderer": "DirectX 11 + DirectComposition"});
    let mut errors: Vec<String> = Vec::new();

    if !wait_for(&this, cx, 90, |a| a.home.art_ready).await {
        errors.push("home art never became ready".into());
    }
    let cold = this.update(cx, |a, _| a.bench.cold_start_ms).ok().flatten();
    out["cold_start_ms"] = json!(cold.map(|c| c.round()));
    sleep(cx, 1500).await;
    out["rss_mb_home"] = json!((rss_mb() * 10.).round() / 10.);

    // ---- scroll the 500-card rail
    let speed = 2400.;
    let started = this
        .update(cx, |app, cx| {
            let rail = app.home.rails.iter().position(|r| r.kind == RailKind::Posters && r.cards.len() >= 500)?;
            app.home.bench_scroll = Some(BenchScroll { rail, start: Instant::now(), px_per_s: speed, done: false });
            app.bench.start();
            cx.notify();
            Some(app.home.rails[rail].cards.len())
        })
        .ok()
        .flatten();
    match started {
        Some(n) => {
            wait_for(&this, cx, 120, |a| a.home.bench_scroll.as_ref().map(|b| b.done).unwrap_or(true)).await;
            let rec = this
                .update(cx, |app, _| {
                    app.home.bench_scroll = None;
                    app.bench.stop()
                })
                .unwrap_or_default();
            out["scroll_frame_ms"] = stats(&rec.intervals);
            out["scroll_build_ms"] = stats(&rec.build);
            out["scroll"] = json!({"cards": n, "px_per_s": speed});
        }
        None => errors.push("no 500-card rail".into()),
    }
    sleep(cx, 500).await;

    // ---- the two files
    let mut mpv_stats = json!({});
    let mut ui = json!({});
    let mut all_ui: Vec<f64> = Vec::new();
    for (label, want_even) in [("4k", true), ("10bit", false)] {
        let item = this
            .update(cx, |app, _| app.items.iter().position(|i| (i.episode % 2 == 0) == want_even))
            .ok()
            .flatten();
        let Some(item) = item else {
            errors.push(format!("no item for {label}"));
            continue;
        };
        let _ = this.update(cx, |app, cx| app.open_player(item, cx));
        let up = wait_for(&this, cx, 30, |a| a.player.as_ref().map(|p| p.st.has_video || p.error.is_some()).unwrap_or(false)).await;
        if !up {
            errors.push(format!("{label}: video never started"));
        }
        let _ = this.update(cx, |app, _| app.bench.start());
        let t0 = Instant::now();
        let mut rss_4k = None;
        while t0.elapsed() < Duration::from_secs(20) {
            sleep(cx, 250).await;
            if label == "4k" && rss_4k.is_none() && t0.elapsed() > Duration::from_secs(15) {
                rss_4k = Some(rss_mb());
            }
        }
        let (rec, st) = this
            .update(cx, |app, _| {
                let rec = app.bench.stop();
                let st = app.player.as_ref().and_then(|p| p.mpv.clone()).map(|m| {
                    json!({
                        "vo": m.get_string("current-vo"),
                        "gpu-api": m.get_string("current-gpu-context"),
                        "hwdec-current": m.get_string("hwdec-current"),
                        "frame-drop-count": m.get_i64("frame-drop-count"),
                        "decoder-frame-drop-count": m.get_i64("decoder-frame-drop-count"),
                        "vo-delayed-frame-count": m.get_i64("vo-delayed-frame-count"),
                        "estimated-vf-fps": m.get_f64("estimated-vf-fps"),
                        "container-fps": m.get_f64("container-fps"),
                        "video-params": m.get_string("video-params/pixelformat"),
                        "size": format!("{}x{}", m.get_i64("width").unwrap_or(0), m.get_i64("height").unwrap_or(0)),
                        "file": m.get_string("filename"),
                    })
                });
                (rec, st)
            })
            .unwrap_or_default();
        mpv_stats[label] = st.unwrap_or(Value::Null);
        ui[label] = stats(&rec.intervals);
        all_ui.extend(&rec.intervals);
        if let Some(r) = rss_4k {
            out["rss_mb_player_4k"] = json!((r * 10.).round() / 10.);
        }
        let _ = this.update(cx, |app, cx| app.close_player(cx));
        sleep(cx, 1500).await;
    }
    // Both files together for the headline number.
    out["mpv"] = mpv_stats;
    let both = stats(&all_ui);
    out["ui_frame_ms_during_video"] = json!({"p50": both["p50"], "p95": both["p95"], "per_file": ui});
    out["errors"] = json!(errors);

    let path = this.update(cx, |app, _| app.bench.out.clone()).ok().flatten();
    if let Some(path) = path {
        let text = serde_json::to_string_pretty(&out).unwrap_or_default();
        if let Err(e) = std::fs::write(&path, text) {
            eprintln!("bench: {}: {e}", path.display());
        }
    }
    let _ = cx.update(|_, cx| cx.quit());
}
