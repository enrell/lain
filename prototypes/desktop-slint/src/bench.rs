//! `--bench <out.json>`: Home → scroll the 500-card rail end to end → 4K file
//! 20 s → 10-bit file 20 s → write JSON and exit. Unattended.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use serde_json::{Value as J, json};
use slint::{ComponentHandle, Model};
use slint_interpreter::Value;

use crate::metrics::{self, Frames, Phase};
use crate::{App, with_app};

const SCROLL_PX_PER_S: f64 = 4000.0;
const PLAY_S: f64 = 20.0;

enum Step {
    WaitHome,
    Settle(Instant),
    Prep(Instant),
    Scrolling(Instant, f64),
    Open(usize),
    WaitPlaying(usize, Instant),
    Playing(usize, Instant),
    Done,
}

impl Step {
    fn name(&self) -> &'static str {
        match self {
            Step::WaitHome => "wait-home",
            Step::Settle(_) => "settle",
            Step::Prep(_) => "prep",
            Step::Scrolling(..) => "scrolling",
            Step::Open(_) => "open",
            Step::WaitPlaying(..) => "wait-playing",
            Step::Playing(..) => "playing",
            Step::Done => "done",
        }
    }
}

fn theme_num(a: &App, p: &str) -> f64 {
    match a.inst.get_global_property("Theme", p) {
        Ok(Value::Number(v)) => v,
        _ => 0.0,
    }
}

fn mpv_stats(a: &App) -> J {
    let m = &a.mpv;
    let i = |k: &str| m.get_i64(k).map(J::from).unwrap_or(J::Null);
    let f = |k: &str| m.get_f64(k).map(|v| J::from((v * 100.0).round() / 100.0)).unwrap_or(J::Null);
    let st = |k: &str| m.get_str(k).map(J::from).unwrap_or(J::Null);
    json!({
        "vo": st("current-vo"),
        "ao": st("current-ao"),
        "hwdec-current": st("hwdec-current"),
        "frame-drop-count": i("frame-drop-count"),
        "decoder-frame-drop-count": i("decoder-frame-drop-count"),
        "vo-delayed-frame-count": i("vo-delayed-frame-count"),
        "estimated-vf-fps": f("estimated-vf-fps"),
        "container-fps": f("container-fps"),
        "display-fps": f("display-fps"),
        "video": format!("{}x{} {}", m.get_i64("video-params/w").unwrap_or(0), m.get_i64("video-params/h").unwrap_or(0),
                         m.get_str("video-params/pixelformat").unwrap_or_default()),
        "file": a.playing.as_ref().and_then(|id| a.by_id.get(id)).map(|&i| a.file_for(&a.items[i]).to_string_lossy().into_owned()),
    })
}

pub fn start(
    out: PathBuf,
    shots: Option<PathBuf>,
    frames: Rc<RefCell<Frames>>,
    cold_start: Rc<Cell<Option<f64>>>,
    scroll: Rc<Cell<Option<(Instant, f64, f64)>>>,
    gl_info: Rc<RefCell<String>>,
    compile_ms: f64,
) -> slint::Timer {
    let timer = slint::Timer::default();
    let t_begin = Instant::now();
    let mut step = Step::WaitHome;
    let mut report = json!({
        "framework": "slint 1.18.1 (winit + femtovg/OpenGL, slint-interpreter)",
        "ui_compile_ms": (compile_ms * 10.0).round() / 10.0,
        "mpv": {},
    });
    let mut step_started = Instant::now();
    let mut written = false;
    let (mut shot10, mut shot_scroll) = (false, false);
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(20), move || {
        with_app(|a| {
            let now = Instant::now();
            let prev = std::mem::discriminant(&step);
            let files = [Phase::Video4k, Phase::Video10];
            let mut fail: Option<String> = None;
            step = match std::mem::replace(&mut step, Step::Done) {
                Step::WaitHome => {
                    if cold_start.get().is_some() && a.rails.row_count() > 0 && a.posters_done >= a.series.len() {
                        Step::Settle(now)
                    } else {
                        Step::WaitHome
                    }
                }
                Step::Settle(t) if now - t > Duration::from_millis(1500) => {
                    report["cold_start_ms"] = J::from(cold_start.get().unwrap_or(0.0).round());
                    report["rss_mb_home"] = J::from(metrics::rss_mb());
                    let row = a.all_rail_row();
                    let _ = a.inst.set_global_property("Home", "bench-rail", Value::Number(row as f64));
                    let _ = a.inst.set_global_property("Home", "focus-row", Value::Number(row as f64));
                    Step::Prep(now)
                }
                Step::Prep(t) if now - t > Duration::from_millis(1000) => {
                    let w = a.inst.window();
                    let width = w.size().to_logical(w.scale_factor()).width as f64;
                    let step_px = theme_num(a, "card-w") + theme_num(a, "gap");
                    let content = theme_num(a, "s6") * 2.0 + 500.0 * step_px - theme_num(a, "gap");
                    let max = (content - width).max(0.0);
                    report["scroll"] = json!({"px_per_s": SCROLL_PX_PER_S, "distance_px": max.round(), "cards": 500});
                    frames.borrow_mut().set_phase(Some(Phase::Scroll));
                    scroll.set(Some((now, SCROLL_PX_PER_S, max)));
                    w.request_redraw();
                    Step::Scrolling(now, max)
                }
                Step::Scrolling(t, max) => {
                    if !shot_scroll && (now - t).as_secs_f64() > 3.0 {
                        shot_scroll = true;
                        if let Some(d) = &shots {
                            crate::snapshot(&a.inst, &d.join("home-scrolling.png"));
                        }
                    }
                    if (now - t).as_secs_f64() * SCROLL_PX_PER_S >= max + 200.0 {
                        scroll.set(None);
                        frames.borrow_mut().set_phase(None);
                        let f = frames.borrow();
                        report["scroll_frame_ms"] = metrics::stats(&f.of(Phase::Scroll, false));
                        report["scroll_cpu_ms"] = metrics::stats(&f.of(Phase::Scroll, true));
                        Step::Open(0)
                    } else {
                        Step::Scrolling(t, max)
                    }
                }
                Step::Open(k) => {
                    let want_even = k == 0;
                    let id = a.items.iter().find(|i| (i.episode % 2 == 0) == want_even).map(|i| i.id.clone());
                    match id {
                        Some(id) => {
                            a.open(&id);
                            Step::WaitPlaying(k, now)
                        }
                        None => {
                            fail = Some("no item to play".into());
                            Step::Done
                        }
                    }
                }
                Step::WaitPlaying(k, t) => {
                    if a.mpv.get_f64("time-pos").unwrap_or(0.0) > 0.5 {
                        frames.borrow_mut().set_phase(Some(files[k]));
                        Step::Playing(k, now)
                    } else if now - t > Duration::from_secs(30) {
                        fail = Some(format!("file {k} did not start playing"));
                        Step::Done
                    } else {
                        Step::WaitPlaying(k, t)
                    }
                }
                Step::Playing(k, t) => {
                    let el = (now - t).as_secs_f64();
                    if k == 0 && report.get("rss_mb_player_4k").is_none() && el > PLAY_S / 2.0 {
                        report["rss_mb_player_4k"] = J::from(metrics::rss_mb());
                        if let Some(d) = &shots {
                            crate::snapshot(&a.inst, &d.join("player-4k.png"));
                        }
                    }
                    if k == 1 && !shot10 && el > PLAY_S / 2.0 {
                        shot10 = true;
                        if let Some(d) = &shots {
                            crate::snapshot(&a.inst, &d.join("player-10bit.png"));
                        }
                    }
                    if el >= PLAY_S {
                        let key = if k == 0 { "4k_hevc" } else { "1080p_hevc10" };
                        report["mpv"][key] = mpv_stats(a);
                        frames.borrow_mut().set_phase(None);
                        if k == 0 { Step::Open(1) } else { Step::Done }
                    } else {
                        Step::Playing(k, t)
                    }
                }
                s @ (Step::Settle(_) | Step::Prep(_)) => s,
                Step::Done => Step::Done,
            };
            if std::mem::discriminant(&step) != prev {
                eprintln!("bench: step {} at {:.1} s", step.name(), t_begin.elapsed().as_secs_f64());
                step_started = now;
            } else if now - step_started > Duration::from_secs(90) && !matches!(step, Step::Done) {
                fail = Some("bench step timed out".into());
                step = Step::Done;
            }
            if let Some(f) = fail {
                report["error"] = J::from(f);
            }
            if matches!(step, Step::Done) && !written {
                written = true;
                let f = frames.borrow();
                let mut v: Vec<f64> = f.of(Phase::Video4k, false);
                v.extend(f.of(Phase::Video10, false));
                report["ui_frame_ms_during_video"] = metrics::stats(&v);
                report["ui_frame_ms_during_video_4k"] = metrics::stats(&f.of(Phase::Video4k, false));
                report["ui_frame_ms_during_video_10bit"] = metrics::stats(&f.of(Phase::Video10, false));
                report["ui_cpu_ms_during_video"] = metrics::stats(&{
                    let mut c = f.of(Phase::Video4k, true);
                    c.extend(f.of(Phase::Video10, true));
                    c
                });
                report["gl"] = J::from(gl_info.borrow().clone());
                report["total_s"] = J::from((t_begin.elapsed().as_secs_f64() * 10.0).round() / 10.0);
                let txt = serde_json::to_string_pretty(&report).unwrap_or_default();
                if let Err(e) = std::fs::write(&out, &txt) {
                    eprintln!("bench: write {}: {e}", out.display());
                }
                eprintln!("{txt}");
                a.close_player();
                let _ = slint::quit_event_loop();
            }
        });
    });
    timer
}
