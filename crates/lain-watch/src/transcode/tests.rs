//! The session lifecycle with an injected probe, capability set and run
//! (no ffmpeg): Go's `v3_test.go`, `idle_test.go` and
//! `lifecycle_model_test.go` intents. Real ffmpeg runs are covered by
//! the server's gated end-to-end tests.

use std::sync::atomic::{AtomicI64, Ordering};
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
