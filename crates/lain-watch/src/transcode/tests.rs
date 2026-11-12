//! The session lifecycle with an injected probe, capability set and run
//! (no ffmpeg): Go's `v3_test.go`, `idle_test.go` and
//! `lifecycle_model_test.go` intents. Real ffmpeg runs are covered by
//! the server's gated end-to-end tests.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc;

use super::*;

fn report(video: &str) -> RawReport {
    let mut r = RawReport::default();
    r.streams.push(RawStream {
        index: 0,
        codec_type: "video".into(),
        codec_name: video.into(),
        pix_fmt: "yuv420p".into(),
        width: 320,
        height: 240,
        r_frame_rate: "10/1".into(),
        ..Default::default()
    });
    r.streams.push(RawStream {
        index: 1,
        codec_type: "audio".into(),
        codec_name: "aac".into(),
        channels: 2,
        ..Default::default()
    });
    r.format.duration = "2".into();
    r
}

struct Rig {
    t: Transcoder,
    dir: PathBuf,
    src: String,
    clock: Arc<AtomicI64>,
    /// Each run waits for one message (`true` succeeds, `false` fails).
    gate: Option<mpsc::Sender<bool>>,
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.gate.take();
        self.t.close();
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

fn rig(tag: &str, gated: bool) -> Rig {
    let dir = lain_store::test_dir(tag);
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("Show.mkv");
    std::fs::write(&src, b"not really media").unwrap();
    let clock = Arc::new(AtomicI64::new(1_000_000));
    let (tx, rx) = mpsc::channel::<bool>();
    let rx = Arc::new(Mutex::new(rx));
    let c = clock.clone();
    let hooks = Hooks {
        probe: Some(Arc::new(|_: &Path| Ok(report("h264")))),
        caps: Some(Arc::new(|_: &TranscodeSettings| {
            let mut c = Capabilities::default();
            c.encoders.insert("libx264".into());
            c
        })),
        run: Some(Arc::new(move |ctx: &RunCtx| {
            if gated {
                let ok = rx.lock().unwrap().recv().unwrap_or(false);
                if !ok {
                    return Err("boom".into());
                }
            }
            if ctx.delivery == st::DELIVERY_HLS {
                std::fs::create_dir_all(&ctx.paths.hls_dir).unwrap();
                let raw = "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:2.0,\nseg00000.m4s\n#EXT-X-ENDLIST\n";
                std::fs::write(ctx.paths.hls_dir.join(hls::RAW_PLAYLIST), raw).unwrap();
                std::fs::write(ctx.paths.hls_dir.join("init.mp4"), b"init").unwrap();
                std::fs::write(ctx.paths.hls_dir.join("seg00000.m4s"), b"seg").unwrap();
            } else {
                std::fs::write(&ctx.paths.media, b"mp4 bytes").unwrap();
            }
            Ok(())
        })),
        clock: Some(Arc::new(move || c.load(Ordering::SeqCst))),
    };
    let t = Transcoder::with_hooks(&dir.join("transcodes"), hooks);
    Rig { t, dir, src: src.to_string_lossy().into_owned(), clock, gate: gated.then_some(tx) }
}

fn req(r: &Rig, action: &str) -> V3Request {
    V3Request { action: action.into(), file_path: r.src.clone(), settings: st::defaults(), ..V3Request::default() }
}

fn wait_state(r: &Rig, session: &str, want: &str) -> V3Status {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let s = r.t.invoke_v3(&V3Request { session: session.into(), ..req(r, ACTION_STATUS) }).unwrap();
        if s.state == want {
            return s;
        }
        assert!(Instant::now() < deadline, "state {} never became {want}", s.state);
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn inspect_start_status_lifecycle() {
    let r = rig("tc-life", false);
    let idle = r.t.invoke_v3(&req(&r, ACTION_INSPECT)).unwrap();
    assert_eq!((idle.state.as_str(), idle.delivery.as_str()), (IDLE, "hls"));
    assert!(valid_session_id(&idle.session));
    // Inspect is side-effect free.
    assert!(r.t.list_v3().is_empty());
    let started = r.t.invoke_v3(&req(&r, ACTION_START)).unwrap();
    assert_eq!(started.session, idle.session);
    assert!(started.state == QUEUED || started.state == RUNNING || started.state == READY);
    let ready = wait_state(&r, &started.session, READY);
    assert!(ready.playable && !ready.playlist_path.is_empty());
    assert_eq!(ready.method, "remux");
    assert!(ready.public().profile.starts_with("web-mp4-v3-"));
    // The record makes the next start instant and cached.
    let again = r.t.invoke_v3(&req(&r, ACTION_START)).unwrap();
    assert!(again.cached && again.state == READY);
    let resolved = r.t.invoke_v3(&V3Request { session: ready.session.clone(), ..req(&r, ACTION_RESOLVE) }).unwrap();
    assert_eq!(resolved.playlist_path, ready.playlist_path);
    // A session id that does not match the options is refused.
    let mut wrong = req(&r, ACTION_START);
    wrong.session = "0".repeat(64);
    assert_eq!(r.t.invoke_v3(&wrong).unwrap_err().code, "invalid-message");
    let unknown = V3Request { session: "f".repeat(64), ..req(&r, ACTION_STATUS) };
    assert_eq!(r.t.invoke_v3(&unknown).unwrap_err().code, "not-found");
    let bad = V3Request { session: "../x".into(), ..req(&r, ACTION_STATUS) };
    assert_eq!(r.t.invoke_v3(&bad).unwrap_err().code, "not-found");
}

#[test]
fn duplicate_start_joins_and_selection_changes_session() {
    let r = rig("tc-join", true);
    let a = r.t.invoke_v3(&req(&r, ACTION_START)).unwrap();
    let b = r.t.invoke_v3(&req(&r, ACTION_START)).unwrap();
    assert_eq!(a.session, b.session);
    assert_eq!(r.t.list_v3().len(), 1);
    let mut other = req(&r, ACTION_START);
    other.delivery = "progressive".into();
    let c = r.t.invoke_v3(&other).unwrap();
    assert_ne!(c.session, a.session);
    let gate = r.gate.as_ref().unwrap();
    gate.send(true).unwrap();
    gate.send(true).unwrap();
    wait_state(&r, &a.session, READY);
    let p = wait_state(&r, &c.session, READY);
    assert!(p.path.ends_with(".mp4") && p.playable);
}

#[test]
fn failure_is_reported_and_a_post_retries() {
    let r = rig("tc-fail", true);
    let a = r.t.invoke_v3(&req(&r, ACTION_START)).unwrap();
    r.gate.as_ref().unwrap().send(false).unwrap();
    let f = wait_state(&r, &a.session, FAILED);
    assert_eq!((f.error_code.as_str(), f.error.as_str()), ("dependency-unavailable", "transcode failed"));
    let again = r.t.invoke_v3(&req(&r, ACTION_START)).unwrap();
    assert!(again.state == QUEUED || again.state == RUNNING);
    r.gate.as_ref().unwrap().send(true).unwrap();
    wait_state(&r, &a.session, READY);
}

#[test]
fn cancel_stops_and_only_the_owner_may() {
    let r = rig("tc-cancel", true);
    let mut start = req(&r, ACTION_START);
    start.user_id = "u1".into();
    let a = r.t.invoke_v3(&start).unwrap();
    let mut other = V3Request { session: a.session.clone(), ..req(&r, ACTION_CANCEL) };
    other.user_id = "u2".into();
    r.t.invoke_v3(&other).unwrap();
    assert_eq!(r.t.list_v3().len(), 1, "another account cannot cancel");
    let mut mine = other.clone();
    mine.user_id = "u1".into();
    let st = r.t.invoke_v3(&mine).unwrap();
    assert_eq!(st.state, IDLE);
    assert!(r.t.list_v3().is_empty());
    r.gate.as_ref().unwrap().send(true).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    // The cancelled run never becomes a record.
    let after = r.t.invoke_v3(&req(&r, ACTION_INSPECT)).unwrap();
    assert_eq!(after.state, IDLE);
}

#[test]
fn queue_and_stream_limits() {
    let r = rig("tc-limits", true);
    let mut s = st::defaults();
    s.queue_size = 1;
    s.max_concurrent = 1;
    let mk = |delivery: &str, quality: &str| V3Request {
        delivery: delivery.into(),
        quality: quality.into(),
        settings: s.clone(),
        user_id: "u1".into(),
        policy: Some(Policy {
            allow_video_transcode: true,
            allow_audio_transcode: true,
            allow_remux: true,
            max_streams: 2,
        }),
        ..req(&r, ACTION_START)
    };
    let a = r.t.invoke_v3(&mk("hls", "")).unwrap();
    // The first job leaves the queue for the only slot.
    let deadline = Instant::now() + Duration::from_secs(5);
    while r.t.invoke_v3(&V3Request { session: a.session.clone(), ..req(&r, ACTION_STATUS) }).unwrap().state != RUNNING {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    r.t.invoke_v3(&mk("progressive", "")).unwrap();
    let e = r.t.invoke_v3(&mk("hls", "720p")).unwrap_err();
    assert_eq!(e.code, "queue-full");
    let mut roomy = mk("hls", "480p");
    roomy.settings.queue_size = 8;
    let e = r.t.invoke_v3(&roomy).unwrap_err();
    assert_eq!(e.code, "too-many-streams", "two active streams of u1");
    // Another account is not counted against u1.
    roomy.user_id = "u2".into();
    r.t.invoke_v3(&roomy).unwrap();
    for _ in 0..3 {
        r.gate.as_ref().unwrap().send(true).unwrap();
    }
}

#[test]
fn position_and_idle_timeout() {
    let r = rig("tc-idle", true);
    let a = r.t.invoke_v3(&req(&r, ACTION_START)).unwrap();
    wait_state(&r, &a.session, RUNNING);
    let p =
        r.t.invoke_v3(&V3Request { session: a.session.clone(), segment_index: 3, ..req(&r, ACTION_POSITION) }).unwrap();
    assert_eq!(p.state, RUNNING);
    // A status poll is not a touch; only media fetches and positions are.
    r.clock.fetch_add(121, Ordering::SeqCst);
    r.t.invoke_v3(&V3Request { session: a.session.clone(), ..req(&r, ACTION_STATUS) }).unwrap();
    r.t.tick();
    let f = r.t.invoke_v3(&V3Request { session: a.session.clone(), ..req(&r, ACTION_STATUS) }).unwrap();
    assert_eq!((f.state.as_str(), f.error_code.as_str()), (FAILED, "idle-timeout"));
    r.gate.as_ref().unwrap().send(true).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let f = r.t.invoke_v3(&V3Request { session: a.session.clone(), ..req(&r, ACTION_STATUS) }).unwrap();
    assert_eq!(f.state, FAILED, "a stop is terminal; a late finish never resurrects it");
    let unknown = V3Request { session: "f".repeat(64), ..req(&r, ACTION_POSITION) };
    assert_eq!(r.t.invoke_v3(&unknown).unwrap_err().code, "not-found");
}

#[test]
fn restricted_users_never_get_a_forbidden_derivative() {
    let r = rig("tc-policy", false);
    let a = r.t.invoke_v3(&req(&r, ACTION_START)).unwrap();
    wait_state(&r, &a.session, READY);
    let mut limited = req(&r, ACTION_START);
    limited.policy =
        Some(Policy { allow_video_transcode: true, allow_audio_transcode: true, allow_remux: false, max_streams: 0 });
    assert_eq!(r.t.invoke_v3(&limited).unwrap_err().code, "forbidden");
}

#[test]
fn lru_eviction_spares_recent_sessions() {
    let r = rig("tc-lru", false);
    let mut s = st::defaults();
    s.cache_bytes = 1 << 30;
    let mut ids = Vec::new();
    for q in ["", "720p", "480p"] {
        let a =
            r.t.invoke_v3(&V3Request {
                quality: q.into(),
                delivery: "progressive".into(),
                settings: s.clone(),
                ..req(&r, ACTION_START)
            })
            .unwrap();
        ids.push(wait_state(&r, &a.session, READY).session);
    }
    assert!(ids.iter().all(|id| r.t.meta_path(id).exists()));
    // Over budget with every entry old: the least recently used goes.
    r.t.lock().max_cache_bytes = 10;
    r.clock.fetch_add(3600, Ordering::SeqCst);
    r.t.touch(&read_entry(&r.t.meta_path(&ids[0])).unwrap());
    r.t.cleanup(false, &Default::default()).unwrap();
    assert!(r.t.meta_path(&ids[0]).exists(), "touched within the grace window");
    assert!(!r.t.meta_path(&ids[1]).exists() && !r.t.meta_path(&ids[2]).exists());
    // A record whose source changed is stale.
    std::fs::write(&r.src, b"a different file now").unwrap();
    r.t.cleanup(false, &Default::default()).unwrap();
    assert!(!r.t.meta_path(&ids[0]).exists());
}

#[test]
fn subtitle_extraction_policy() {
    let r = rig("tc-subs", false);
    let mut s = req(&r, ACTION_SUBTITLES);
    assert_eq!(r.t.invoke_v3(&s).unwrap_err().msg, "subtitle_stream required");
    s.subtitle_stream = Some(1);
    s.settings.allow_subtitle_extraction = Some(false);
    assert_eq!(r.t.invoke_v3(&s).unwrap_err().code, "forbidden");
    s.settings.allow_subtitle_extraction = None;
    assert_eq!(r.t.invoke_v3(&s).unwrap_err().msg, "subtitle_stream is not a subtitle track");
}

#[test]
fn legacy_prepare_waits_for_the_file() {
    let r = rig("tc-v1", false);
    let out = r.t.prepare_sync(&r.src).unwrap();
    assert!(out.path.ends_with(".mp4") && !out.cached);
    assert!(r.t.prepare_sync(&r.src).unwrap().cached);
    assert_eq!(r.t.prepare_sync("").unwrap_err().code, "invalid-message");
    assert_eq!(r.t.prepare_sync(&format!("{}.gone", r.src)).unwrap_err().msg, "source file unavailable");
}

#[test]
fn closing_fails_queued_work() {
    let r = rig("tc-close", true);
    let mut s = st::defaults();
    s.max_concurrent = 1;
    let a = r.t.invoke_v3(&V3Request { settings: s.clone(), ..req(&r, ACTION_START) }).unwrap();
    wait_state(&r, &a.session, RUNNING);
    let b = r.t.invoke_v3(&V3Request { settings: s, delivery: "progressive".into(), ..req(&r, ACTION_START) }).unwrap();
    r.t.close();
    let st = r.t.list_v3().into_iter().find(|x| x.session == b.session).unwrap();
    assert_eq!((st.state.as_str(), st.error.as_str()), (FAILED, "transcoder stopped"));
    assert_eq!(r.t.invoke_v3(&req(&r, ACTION_START)).unwrap_err().msg, "transcoder stopped");
}

// ---- capabilities: single flight, stale-while-revalidate, warm-up -------

/// A transcoder whose capability probe counts its runs, answers with its
/// run number as the ffmpeg (`probe-<n>`) and `effects` as the video
/// effects, and lands only when the test lets one through
/// ([`CapsRig::let_one`]).
struct CapsRig {
    t: Transcoder,
    dir: PathBuf,
    runs: Arc<AtomicI64>,
    effects: Arc<AtomicBool>,
    gate: Option<mpsc::Sender<()>>,
}

impl Drop for CapsRig {
    fn drop(&mut self) {
        // A closed gate lets every probe still waiting land.
        self.gate.take();
        self.t.close();
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

impl CapsRig {
    fn let_one(&self) {
        self.gate.as_ref().unwrap().send(()).unwrap();
    }

    fn runs(&self) -> i64 {
        self.runs.load(Ordering::SeqCst)
    }

    fn wait_runs(&self, n: i64) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.runs() < n {
            assert!(Instant::now() < deadline, "{} probes started, want {n}", self.runs());
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// `capabilities` for a caller that must not wait: fails instead of
    /// hanging when it does.
    fn caps_now(&self, s: &TranscodeSettings) -> Capabilities {
        let (t, s) = (self.t.clone(), s.clone());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(t.capabilities(&s));
        });
        rx.recv_timeout(Duration::from_secs(5)).expect("a caller with a set waits for no probe")
    }

    fn wait_for(&self, s: &TranscodeSettings, ffmpeg: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.caps_now(s).ffmpeg != ffmpeg {
            assert!(Instant::now() < deadline, "{ffmpeg} never landed");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn set_fresh(&self, d: Duration) {
        self.t.inner.caps.lock().unwrap().fresh = d;
    }

    fn set_recheck(&self, d: Duration) {
        self.t.inner.caps.lock().unwrap().recheck = d;
    }
}

fn caps_rig(tag: &str) -> CapsRig {
    let dir = lain_store::test_dir(tag);
    std::fs::create_dir_all(&dir).unwrap();
    let runs = Arc::new(AtomicI64::new(0));
    let (tx, rx) = mpsc::channel::<()>();
    let rx = Mutex::new(rx);
    let effects = Arc::new(AtomicBool::new(false));
    let (counter, fx) = (runs.clone(), effects.clone());
    let hooks = Hooks {
        caps: Some(Arc::new(move |_: &TranscodeSettings| {
            let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
            let _ = rx.lock().unwrap().recv();
            Capabilities { ffmpeg: format!("probe-{n}"), effects: fx.load(Ordering::SeqCst), ..Capabilities::default() }
        })),
        ..Hooks::default()
    };
    CapsRig { t: Transcoder::with_hooks(&dir.join("transcodes"), hooks), dir, runs, effects, gate: Some(tx) }
}

#[test]
fn concurrent_callers_share_one_probe() {
    let r = caps_rig("tc-caps-flight");
    let s = st::defaults();
    let start = Arc::new(std::sync::Barrier::new(9));
    let callers: Vec<_> = (0..8)
        .map(|_| {
            let (t, s, start) = (r.t.clone(), s.clone(), start.clone());
            std::thread::spawn(move || {
                start.wait();
                t.capabilities(&s)
            })
        })
        .collect();
    start.wait();
    r.wait_runs(1);
    // Every caller is in by now, waiting on that one probe.
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(r.runs(), 1, "eight callers, one probe");
    assert!(callers.iter().all(|c| !c.is_finished()), "no set yet: callers wait for the probe");
    r.let_one();
    for c in callers {
        assert_eq!(c.join().unwrap().ffmpeg, "probe-1");
    }
    assert_eq!(r.runs(), 1);
}

#[test]
fn a_stale_set_comes_back_at_once_while_one_probe_refreshes_it() {
    let r = caps_rig("tc-caps-stale");
    let s = st::defaults();
    r.let_one();
    assert_eq!(r.t.capabilities(&s).ffmpeg, "probe-1");
    assert_eq!(r.caps_now(&s).ffmpeg, "probe-1");
    assert_eq!(r.runs(), 1, "a current set is not probed again");
    r.set_fresh(Duration::ZERO);
    for _ in 0..5 {
        assert_eq!(r.caps_now(&s).ffmpeg, "probe-1", "the stale set, without waiting");
    }
    r.wait_runs(2);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(r.runs(), 2, "one refresh at a time");
    r.set_fresh(CAP_FRESH);
    r.let_one();
    r.wait_for(&s, "probe-2");
    assert_eq!(r.runs(), 2);
}

#[test]
fn reprobe_forces_a_fresh_probe_nobody_waits_for() {
    let r = caps_rig("tc-caps-reprobe");
    let s = st::defaults();
    r.let_one();
    assert_eq!(r.t.capabilities(&s).ffmpeg, "probe-1");
    r.t.reprobe(&s);
    r.wait_runs(2);
    // The one running may have started before what prompted these (a
    // driver installed, then the settings saved): one more runs after
    // it, however many were asked for, and nobody waits for either.
    r.t.reprobe(&s);
    r.t.reprobe(&s);
    assert_eq!(r.caps_now(&s).ffmpeg, "probe-1");
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(r.runs(), 2, "single flight");
    r.let_one();
    r.wait_runs(3);
    assert_eq!(r.caps_now(&s).ffmpeg, "probe-2");
    r.let_one();
    r.wait_for(&s, "probe-3");
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(r.runs(), 3);
}

#[test]
fn the_warm_up_and_a_new_ffmpeg_each_probe_once() {
    let r = caps_rig("tc-caps-key");
    let s = st::defaults();
    // The server's warm-up returns at once; a caller arriving before it
    // lands waits for it instead of starting a probe of its own.
    r.t.warm(&s);
    r.wait_runs(1);
    let (t, s2) = (r.t.clone(), s.clone());
    let first = std::thread::spawn(move || t.capabilities(&s2));
    std::thread::sleep(Duration::from_millis(50));
    assert!(!first.is_finished(), "no set yet: the caller waits");
    assert_eq!(r.runs(), 1);
    r.let_one();
    assert_eq!(first.join().unwrap().ffmpeg, "probe-1");
    r.t.warm(&s);
    // `ffmpeg` is the default binary spelled out: the same set.
    let same = TranscodeSettings { ffmpeg_path: "ffmpeg".into(), crf: 18, ..s.clone() };
    assert_eq!(r.caps_now(&same).ffmpeg, "probe-1");
    assert_eq!(r.runs(), 1, "a warm-up of a current set and other settings probe nothing");
    // Another ffmpeg is another set, probed once while the first stays.
    let other = TranscodeSettings { ffmpeg_path: "/opt/ffmpeg/bin/ffmpeg".into(), ..s.clone() };
    let (t, o) = (r.t.clone(), other.clone());
    let waiting = std::thread::spawn(move || t.capabilities(&o));
    r.wait_runs(2);
    assert_eq!(r.caps_now(&s).ffmpeg, "probe-1");
    r.let_one();
    assert_eq!(waiting.join().unwrap().ffmpeg, "probe-2");
    assert_eq!(r.caps_now(&other).ffmpeg, "probe-2");
    assert_eq!(r.runs(), 2);
}

#[test]
fn a_set_that_lost_a_capability_is_probed_again_sooner() {
    let r = caps_rig("tc-caps-recheck");
    let s = st::defaults();
    r.effects.store(true, Ordering::SeqCst);
    r.let_one();
    assert!(r.t.capabilities(&s).effects);
    // A refresh on a busy machine misses the effects probe: the answer is
    // served as probed, current for CAP_RECHECK instead of CAP_FRESH.
    r.effects.store(false, Ordering::SeqCst);
    r.t.reprobe(&s);
    r.let_one();
    r.wait_for(&s, "probe-2");
    assert!(!r.caps_now(&s).effects);
    assert_eq!(r.runs(), 2, "current for CAP_RECHECK");
    r.set_recheck(Duration::ZERO);
    r.effects.store(true, Ordering::SeqCst);
    assert!(!r.caps_now(&s).effects, "the next caller still waits for nothing");
    r.wait_runs(3);
    r.let_one();
    r.wait_for(&s, "probe-3");
    assert!(r.caps_now(&s).effects);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(r.runs(), 3, "a gain is current for CAP_FRESH");
    // A loss the next probe confirms is then current as usual.
    r.effects.store(false, Ordering::SeqCst);
    r.t.reprobe(&s);
    r.let_one();
    r.wait_for(&s, "probe-4");
    r.wait_runs(5);
    r.let_one();
    r.wait_for(&s, "probe-5");
    assert!(!r.caps_now(&s).effects);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(r.runs(), 5);
}

/// The first answer for a key has nothing to lose against, and it is
/// probed while the server starts and the machine is busiest: one where
/// a check timed out is current only for CAP_RECHECK, every time.
#[test]
fn an_answer_with_a_timed_out_check_is_probed_again_sooner() {
    let mut c =
        CapsCache { slots: HashMap::new(), fresh: CAP_FRESH, recheck: CAP_RECHECK, limit: policy::PROBE_TIMEOUT };
    let caps = Capabilities { tone_map: true, ..Capabilities::default() };
    assert!(c.claim("k", false));
    assert!(c.store("k", caps.clone(), true).doubtful, "a first answer with a timed-out check");
    assert!(!c.claim("k", false), "current for CAP_RECHECK");
    c.recheck = Duration::ZERO;
    assert!(c.claim("k", false), "then probed again");
    assert!(c.store("k", caps.clone(), true).doubtful, "the same answer, timed out again: still in doubt");
    assert!(c.claim("k", false));
    assert!(!c.store("k", caps, false).doubtful, "every run ended");
    assert!(!c.claim("k", false), "current for CAP_FRESH");
}

/// The transcoder's own probe (no `caps` hook) over a stub ffmpeg whose
/// video effects check hangs: the answer counts it as a no and is in
/// doubt; once the check answers again, the next answer is not.
#[cfg(unix)]
#[test]
fn the_real_probe_marks_a_timed_out_check_in_doubt() {
    use std::os::unix::fs::PermissionsExt;
    let dir = lain_store::test_dir("tc-caps-timeout");
    std::fs::create_dir_all(&dir).unwrap();
    let (bin, slow) = (dir.join("ffmpeg"), dir.join("slow"));
    let script = r#"#!/bin/sh
case "$2" in
  -encoders) printf ' V..... = Video\n V....D libx264 H.264\n'; exit 0 ;;
  -filters) printf ' ... libplacebo VV->V GPU filters\n'; exit 0 ;;
  -hwaccels) exit 0 ;;
esac
case "$*" in
  *libplacebo=w=*) [ -e 'SLOW' ] && exec sleep 30; exit 0 ;;
esac
exit 1
"#
    .replace("SLOW", &slow.to_string_lossy());
    std::fs::write(&bin, script).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(&slow, b"").unwrap();
    let t = Transcoder::with_hooks(&dir.join("transcodes"), Hooks::default());
    t.inner.caps.lock().unwrap().limit = Duration::from_secs(3);
    let s = TranscodeSettings { ffmpeg_path: bin.to_string_lossy().into_owned(), ..st::defaults() };
    let doubtful = || t.inner.caps.lock().unwrap().slots[&caps_key(&s)].doubtful;
    assert!(!t.capabilities(&s).effects);
    assert!(doubtful(), "the effects check timed out");
    std::fs::remove_file(&slow).unwrap();
    t.reprobe(&s);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !t.capabilities(&s).effects {
        assert!(Instant::now() < deadline, "the second probe never landed");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!doubtful(), "every run ended");
    t.close();
    std::fs::remove_dir_all(&dir).ok();
}
