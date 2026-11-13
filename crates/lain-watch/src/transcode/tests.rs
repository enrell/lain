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

fn as_viewer(r: &Rig, action: &str, user: &str, viewer: &str, session: &str) -> V3Request {
    V3Request { user_id: user.into(), viewer: viewer.into(), session: session.into(), ..req(r, action) }
}

#[test]
fn a_shared_session_stops_when_its_last_viewer_leaves() {
    let r = rig("tc-viewers", true);
    let a = r.t.invoke_v3(&as_viewer(&r, ACTION_START, "u1", "tab-a", "")).unwrap();
    let b = r.t.invoke_v3(&as_viewer(&r, ACTION_START, "u1", "tab-b", "")).unwrap();
    assert_eq!(a.session, b.session, "same options, one session");
    // Another account joins it too.
    r.t.invoke_v3(&as_viewer(&r, ACTION_START, "u2", "phone", "")).unwrap();
    let release =
        |user: &str, viewer: &str| r.t.invoke_v3(&as_viewer(&r, ACTION_CANCEL, user, viewer, &a.session)).unwrap();
    let left = release("u1", "tab-a");
    assert!(left.state == QUEUED || left.state == RUNNING, "{}", left.state);
    assert_eq!(left.session, a.session);
    // A release that held nothing changes nothing: twice, or another
    // account naming the same viewer id.
    release("u1", "tab-a");
    release("u2", "tab-b");
    release("u1", "phone");
    assert_eq!(r.t.list_v3().len(), 1);
    release("u2", "phone");
    assert_eq!(r.t.list_v3().len(), 1, "tab-b of u1 still holds it");
    let last = release("u1", "tab-b");
    assert_eq!((last.state.as_str(), last.user_id.as_str()), (IDLE, ""));
    assert!(r.t.list_v3().is_empty());
    r.gate.as_ref().unwrap().send(true).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    // The stopped run never becomes a record.
    assert_eq!(r.t.invoke_v3(&req(&r, ACTION_INSPECT)).unwrap().state, IDLE);
}

#[test]
fn a_client_without_a_viewer_id_holds_the_session_as_before() {
    let r = rig("tc-unnamed", true);
    let a = r.t.invoke_v3(&as_viewer(&r, ACTION_START, "u1", "tab-a", "")).unwrap();
    // An older client joins without naming itself.
    r.t.invoke_v3(&as_viewer(&r, ACTION_START, "u1", "", "")).unwrap();
    let left = r.t.invoke_v3(&as_viewer(&r, ACTION_CANCEL, "u1", "tab-a", &a.session)).unwrap();
    assert_ne!(left.state, IDLE, "a viewer's release never ends an unnamed hold");
    assert_eq!(r.t.list_v3().len(), 1);
    // Its own release (no viewer) stops the session as it always did.
    let stopped = r.t.invoke_v3(&as_viewer(&r, ACTION_CANCEL, "u1", "", &a.session)).unwrap();
    assert_eq!(stopped.state, IDLE);
    assert!(r.t.list_v3().is_empty());
    r.gate.as_ref().unwrap().send(true).unwrap();
}

#[test]
fn a_finished_session_outlives_its_last_viewer() {
    let r = rig("tc-viewer-ready", false);
    let a = r.t.invoke_v3(&as_viewer(&r, ACTION_START, "u1", "tab-a", "")).unwrap();
    wait_state(&r, &a.session, READY);
    let left = r.t.invoke_v3(&as_viewer(&r, ACTION_CANCEL, "u1", "tab-a", &a.session)).unwrap();
    assert_eq!(left.state, READY, "nothing to stop; the replay cache stays");
    assert!(r.t.meta_path(&a.session).exists());
    // Once no job tracks it (pruned, or a restart), a viewer's release
    // answers the cached session and still removes nothing.
    r.t.lock().jobs.clear();
    let left = r.t.invoke_v3(&as_viewer(&r, ACTION_CANCEL, "u1", "tab-b", &a.session)).unwrap();
    assert!(left.state == READY && left.cached);
    assert!(r.t.meta_path(&a.session).exists());
    let gone = r.t.invoke_v3(&as_viewer(&r, ACTION_CANCEL, "u1", "tab-b", &"f".repeat(64))).unwrap();
    assert_eq!(gone.state, IDLE);
    // A release without a viewer still drops it.
    assert_eq!(r.t.invoke_v3(&as_viewer(&r, ACTION_CANCEL, "u1", "", &a.session)).unwrap().state, IDLE);
    assert!(!r.t.meta_path(&a.session).exists());
}

#[test]
fn viewer_ids_are_checked_and_bounded() {
    let r = rig("tc-viewer-ids", true);
    for bad in ["has space", "../x", "ü", &"v".repeat(65)] {
        let e = r.t.invoke_v3(&as_viewer(&r, ACTION_START, "u1", bad, "")).unwrap_err();
        assert_eq!(e.code, "invalid-message", "{bad}");
        let e = r.t.invoke_v3(&as_viewer(&r, ACTION_CANCEL, "u1", bad, &"f".repeat(64))).unwrap_err();
        assert_eq!(e.code, "invalid-message", "{bad}");
    }
    assert!(valid_viewer_id("0f3c-AB_9") && valid_viewer_id(&"a".repeat(64)) && !valid_viewer_id(""));
    // Past the bound, joins hold the session unnamed: releasing every
    // named viewer no longer stops it.
    let a = r.t.invoke_v3(&as_viewer(&r, ACTION_START, "u1", "v0", "")).unwrap();
    for n in 1..=MAX_VIEWERS {
        r.t.invoke_v3(&as_viewer(&r, ACTION_START, "u1", &format!("v{n}"), "")).unwrap();
    }
    for n in 0..=MAX_VIEWERS {
        r.t.invoke_v3(&as_viewer(&r, ACTION_CANCEL, "u1", &format!("v{n}"), &a.session)).unwrap();
    }
    assert_eq!(r.t.list_v3().len(), 1);
    r.gate.as_ref().unwrap().send(true).unwrap();
}

/// The session's state, unwrapped.
fn state_of(r: &Rig, session: &str) -> String {
    r.t.invoke_v3(&V3Request { session: session.into(), ..req(r, ACTION_STATUS) }).unwrap().state
}

/// Waits until `n` runs and stops are working on `session`'s files.
fn wait_busy(r: &Rig, session: &str, n: u32) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while r.t.lock().busy.get(session).copied().unwrap_or(0) != n {
        assert!(Instant::now() < deadline, "{session} never had {n} runs and stops at work");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A viewer joining just as the last one left gets a fresh job with the
/// same id and paths. Its run must not start while the stopped run is
/// still exiting: that run deletes what it leaves behind, which would
/// be the fresh run's files.
#[test]
fn a_fresh_run_waits_for_the_stopped_run_of_its_session() {
    let r = rig("tc-settle-run", true);
    let gate = r.gate.as_ref().unwrap();
    let a = r.t.invoke_v3(&as_viewer(&r, ACTION_START, "u1", "tab-a", "")).unwrap();
    wait_state(&r, &a.session, RUNNING);
    // The run is held in the gate, as a killed ffmpeg is held until it exits.
    assert_eq!(r.t.invoke_v3(&as_viewer(&r, ACTION_CANCEL, "u1", "tab-a", &a.session)).unwrap().state, IDLE);
    let b = r.t.invoke_v3(&as_viewer(&r, ACTION_START, "u1", "tab-b", "")).unwrap();
    assert_eq!(b.session, a.session);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(state_of(&r, &b.session), QUEUED, "the fresh run started beside the stopped one");
    // The stopped run exits; only then does the fresh one start.
    gate.send(true).unwrap();
    wait_state(&r, &b.session, RUNNING);
    gate.send(true).unwrap();
    wait_state(&r, &b.session, READY);
    assert!(r.t.meta_path(&b.session).exists());
}

/// The same for the stop itself: it removes the session's files after it
/// took the job out of the map, outside the lock.
#[test]
fn a_fresh_run_waits_for_a_stop_still_removing_files() {
    let r = rig("tc-settle-drop", true);
    let gate = r.gate.as_ref().unwrap();
    let a = r.t.invoke_v3(&as_viewer(&r, ACTION_START, "u1", "tab-a", "")).unwrap();
    wait_state(&r, &a.session, RUNNING);
    // A stop between taking the job and removing its files.
    let taken = r.t.take_job(&mut r.t.lock(), &a.session).unwrap();
    gate.send(true).unwrap();
    // The stopped run has exited; only the stop is left at work.
    wait_busy(&r, &a.session, 1);
    let b = r.t.invoke_v3(&as_viewer(&r, ACTION_START, "u1", "tab-b", "")).unwrap();
    assert_eq!(b.session, a.session);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(state_of(&r, &b.session), QUEUED, "the fresh run started before the stop removed the files");
    r.t.drop_taken(&a.session, taken);
    wait_state(&r, &b.session, RUNNING);
    gate.send(true).unwrap();
    wait_state(&r, &b.session, READY);
    let hls_dir = r.t.paths_for(&b.session, &st::defaults()).hls_dir;
    assert!(hls::playable(&hls_dir), "the fresh run's files are in place");
    wait_busy(&r, &b.session, 0);
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

/// A fake ffmpeg for the hardware fallback: a hardware encode writes
/// three segments and a playlist, then fails when the test says so; the
/// software retry announces itself, waits for the test, then writes one
/// segment of its own and ends the playlist. Both write under the init
/// and segment names they are given. POSIX sh (CI's dash).
#[cfg(unix)]
const FALLBACK_FFMPEG: &str = r#"#!/bin/sh
ctl="$LAIN_TEST_CTL"
enc=""
init=""
seg=""
prev=""
for last; do
  case "$prev" in
  -c:v) enc="$last" ;;
  -hls_fmp4_init_filename) init="$last" ;;
  -hls_segment_filename) seg=$(basename "$last") ;;
  esac
  prev="$last"
done
dir=$(dirname "$last")
seg0=$(printf '%s' "$seg" | sed 's/%05d/00000/')
seg1=$(printf '%s' "$seg" | sed 's/%05d/00001/')
seg2=$(printf '%s' "$seg" | sed 's/%05d/00002/')
case "$enc" in
*_nvenc)
  printf 'hw-init' > "$dir/$init"
  for f in "$seg0" "$seg1" "$seg2"; do printf 'hw' > "$dir/$f"; done
  printf '#EXTM3U\n#EXT-X-MAP:URI="%s"\n#EXTINF:2.0,\n%s\n#EXTINF:2.0,\n%s\n#EXTINF:2.0,\n%s\n' "$init" "$seg0" "$seg1" "$seg2" > "$dir/raw.m3u8"
  while [ ! -e "$ctl/fail" ]; do sleep 0.05; done
  echo "the device went away" >&2
  exit 1
  ;;
*)
  : > "$ctl/software"
  while [ ! -e "$ctl/go" ]; do sleep 0.05; done
  printf 'sw-init' > "$dir/$init"
  printf 'sw' > "$dir/$seg0"
  printf '#EXTM3U\n#EXT-X-MAP:URI="%s"\n#EXTINF:2.0,\n%s\n#EXT-X-ENDLIST\n' "$init" "$seg0" > "$dir/raw.m3u8"
  ;;
esac
"#;

/// The HLS files in a session directory, sorted.
#[cfg(unix)]
fn hls_files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    names.retain(|n| !n.ends_with(".tmp"));
    names.sort();
    names
}

#[cfg(unix)]
#[test]
fn a_software_retry_starts_from_a_clean_hls_directory() {
    use std::os::unix::fs::PermissionsExt;
    let dir = lain_store::test_dir("tc-hw-retry");
    let ctl = dir.join("ctl");
    std::fs::create_dir_all(&ctl).unwrap();
    // Whatever happens, the fake's waits end with the test.
    struct Release(PathBuf);
    impl Drop for Release {
        fn drop(&mut self) {
            for f in ["fail", "go"] {
                let _ = std::fs::write(self.0.join(f), b"");
            }
        }
    }
    let _release = Release(ctl.clone());
    let ffmpeg = dir.join("ffmpeg");
    std::fs::write(&ffmpeg, FALLBACK_FFMPEG.replace("$LAIN_TEST_CTL", &ctl.to_string_lossy())).unwrap();
    std::fs::set_permissions(&ffmpeg, std::fs::Permissions::from_mode(0o755)).unwrap();
    let src = dir.join("Show.mkv");
    std::fs::write(&src, b"not really media").unwrap();
    let hooks = Hooks {
        // MPEG-4 Part 2 is never copied, so the session encodes.
        probe: Some(Arc::new(|_: &Path| Ok(report("mpeg4")))),
        caps: Some(Arc::new(|_: &TranscodeSettings| {
            let mut c = Capabilities::default();
            c.encoders.extend(["libx264".to_string(), "h264_nvenc".to_string()]);
            c.hardware.insert(st::HW_NVENC.into(), true);
            c
        })),
        run: None,
        clock: None,
    };
    let t = Transcoder::with_hooks(&dir.join("transcodes"), hooks);
    let mut settings = st::defaults();
    settings.ffmpeg_path = ffmpeg.to_string_lossy().into_owned();
    settings.hardware_acceleration = st::HW_NVENC.into();
    settings.hardware_encode = true;
    settings.throttle = false;
    let start = V3Request {
        action: ACTION_START.into(),
        file_path: src.to_string_lossy().into_owned(),
        settings: settings.clone(),
        ..V3Request::default()
    };
    let a = t.invoke_v3(&start).unwrap();
    assert_eq!(a.encoder, "h264_nvenc");
    let hls_dir = t.paths_for(&a.session, &settings).hls_dir;
    let status = || {
        t.invoke_v3(&V3Request { action: ACTION_STATUS.into(), session: a.session.clone(), ..start.clone() }).unwrap()
    };
    let wait = |what: &str, ok: &dyn Fn() -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ok() {
            assert!(Instant::now() < deadline, "{what}: {:?}, files {:?}", status(), hls_files(&hls_dir));
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    // The hardware attempt is published (a viewer may be polling it)...
    wait("the hardware attempt's playlist is served", &|| status().playable);
    assert!(hls_dir.join(hls::INDEX_PLAYLIST).exists());
    // ...and then fails.
    std::fs::write(ctl.join("fail"), b"").unwrap();
    wait("the software retry starts", &|| ctl.join("software").exists());
    // Nothing of the failed attempt is left to be served or mixed into
    // the retry's playlist, and the session is not playable until the
    // retry has a segment.
    let between = status();
    assert_eq!((between.state.as_str(), between.playable), (RUNNING, false));
    assert_eq!(hls_files(&hls_dir), Vec::<String>::new(), "the failed attempt's files are gone");
    std::fs::write(ctl.join("go"), b"").unwrap();
    wait("the retry finishes", &|| status().state == READY);
    let done = status();
    assert_eq!((done.encoder.as_str(), done.hardware.as_str()), ("libx264", ""));
    assert!(done.fallback.contains("hardware encode failed; used software"), "{}", done.fallback);
    // The retry's init segment and segments have names (so URLs) of
    // their own: a viewer of the failed attempt keeps that attempt's
    // init segment, and its browser caches the segments it fetched.
    assert_eq!(hls_files(&hls_dir), ["index.m3u8", "raw.m3u8", "sw-init.mp4", "sw-seg00000.m4s"]);
    assert_eq!(std::fs::read(hls_dir.join("sw-init.mp4")).unwrap(), b"sw-init");
    let index = std::fs::read_to_string(hls_dir.join(hls::INDEX_PLAYLIST)).unwrap();
    assert!(index.contains("#EXT-X-MAP:URI=\"sw-init.mp4\""), "{index}");
    assert!(index.contains("\nsw-seg00000.m4s\n#EXT-X-ENDLIST"), "{index}");
    assert!(!index.contains("\nseg0000") && !index.contains("\"init.mp4\""), "{index}");
    t.close();
    drop(_release);
    std::fs::remove_dir_all(&dir).ok();
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
            Capabilities {
                ffmpeg: format!("probe-{n}"),
                encoders: ["libx264".to_string()].into(),
                effects: fx.load(Ordering::SeqCst),
                ..Capabilities::default()
            }
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
    let caps = Capabilities { encoders: ["libx264".to_string()].into(), tone_map: true, ..Capabilities::default() };
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

/// An answer with no ffmpeg there is in doubt: the operator may install
/// it any time (the old 30 s cache found it then), and probing for a
/// missing binary costs nothing.
#[test]
fn an_answer_with_no_ffmpeg_is_probed_again_sooner() {
    let mut c =
        CapsCache { slots: HashMap::new(), fresh: CAP_FRESH, recheck: Duration::ZERO, limit: policy::PROBE_TIMEOUT };
    assert!(c.claim("k", false));
    assert!(c.store("k", Capabilities::default(), false).doubtful);
    assert!(c.claim("k", false), "probed again after CAP_RECHECK");
    let found = Capabilities { encoders: ["libx264".to_string()].into(), ..Capabilities::default() };
    assert!(!c.store("k", found, false).doubtful);
    assert!(!c.claim("k", false), "current for CAP_FRESH");
}

/// The transcoder's own probe (no `caps` hook) over a stub ffmpeg whose
/// video effects check hangs: the answer counts it as a no and is in
/// doubt; once the check answers again, the next answer is not.
#[cfg(unix)]
#[test]
fn the_real_probe_marks_a_timed_out_check_in_doubt() {
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
    crate::transcode::write_stub(&bin, &script);
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

/// A probe that panics leaves no waiter behind and no lock poisoned: the
/// callers waiting for the first answer get what an ffmpeg that does not
/// run would answer, in doubt, and the next caller after CAP_RECHECK
/// probes again.
#[test]
fn a_probe_that_panics_answers_nothing_available_and_is_probed_again() {
    let dir = lain_store::test_dir("tc-caps-panic");
    std::fs::create_dir_all(&dir).unwrap();
    let runs = Arc::new(AtomicI64::new(0));
    let counter = runs.clone();
    let hooks = Hooks {
        caps: Some(Arc::new(move |_: &TranscodeSettings| {
            if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                std::thread::sleep(Duration::from_millis(100));
                panic!("probe hook panics on purpose");
            }
            Capabilities { tone_map: true, ..Capabilities::default() }
        })),
        ..Hooks::default()
    };
    let t = Transcoder::with_hooks(&dir.join("transcodes"), hooks);
    let s = st::defaults();
    let (tx, rx) = mpsc::channel();
    for _ in 0..4 {
        let (t, s, tx) = (t.clone(), s.clone(), tx.clone());
        std::thread::spawn(move || tx.send(t.capabilities(&s)).unwrap());
    }
    for _ in 0..4 {
        let caps = rx.recv_timeout(Duration::from_secs(10)).expect("no waiter is left behind");
        assert!(!caps.tone_map && caps.hardware.values().all(|ok| !ok), "{caps:?}");
    }
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    assert!(!t.inner.caps.is_poisoned());
    assert!(t.inner.caps.lock().unwrap().slots[&caps_key(&s)].doubtful);
    t.inner.caps.lock().unwrap().recheck = Duration::ZERO;
    assert!(!t.capabilities(&s).tone_map, "the answer in doubt comes back at once");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !t.capabilities(&s).tone_map {
        assert!(Instant::now() < deadline, "no probe after the panic");
        std::thread::sleep(Duration::from_millis(5));
    }
    t.close();
    std::fs::remove_dir_all(&dir).ok();
}

/// A refresh of the old ffmpeg still running when the operator moves
/// ffmpeg lands on the old set only: a caller of the new one waits for
/// the new one's own probe, never takes the old answer.
#[test]
fn a_refresh_of_the_old_ffmpeg_never_answers_for_the_new_one() {
    let r = caps_rig("tc-caps-race");
    let old = st::defaults();
    r.let_one();
    assert_eq!(r.t.capabilities(&old).ffmpeg, "probe-1");
    r.t.reprobe(&old);
    r.wait_runs(2);
    // Let the old refresh reach the gate first, so it lands first.
    std::thread::sleep(Duration::from_millis(50));
    let new = TranscodeSettings { ffmpeg_path: "/opt/ffmpeg/bin/ffmpeg".into(), ..old.clone() };
    let (t, n) = (r.t.clone(), new.clone());
    let waiting = std::thread::spawn(move || t.capabilities(&n));
    r.wait_runs(3);
    r.let_one();
    r.wait_for(&old, "probe-2");
    std::thread::sleep(Duration::from_millis(50));
    assert!(!waiting.is_finished(), "the new ffmpeg's caller waits for its own probe");
    r.let_one();
    assert_eq!(waiting.join().unwrap().ffmpeg, "probe-3");
    assert_eq!(r.caps_now(&old).ffmpeg, "probe-2");
    assert_eq!(r.runs(), 3);
}

#[test]
fn a_webvtt_sidecar_drops_ass_escapes() {
    let raw = "WEBVTT\n\n00:01.000 --> 00:03.000\n\\h\\hEssa escola é enorme.\\N{\\i1}só que não{\\i0} há dúvidas\n\n00:04.000 --> 00:05.000\n{nota} C:\\pasta\n";
    let clean = super::clean_webvtt(raw);
    assert_eq!(
        clean,
        "WEBVTT\n\n00:01.000 --> 00:03.000\n\u{a0}\u{a0}Essa escola é enorme.\nsó que não há dúvidas\n\n00:04.000 --> 00:05.000\n{nota} C:\\pasta\n"
    );
    // Text with nothing to clean comes back as it was.
    assert_eq!(
        super::clean_webvtt("WEBVTT\n\n00:01.000 --> 00:02.000\nOlá.\n"),
        "WEBVTT\n\n00:01.000 --> 00:02.000\nOlá.\n"
    );
}
