use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::http::PlainHttp;
use crate::testsrv::{Gate, Srv, TestReq, head, respond, serve_content};
use crate::{CODE_DISK_FULL, CODE_HTTP, CODE_QUOTA};

fn payload() -> Vec<u8> {
    b"lain-fixture-0123456789\n".repeat(4096)
}

fn dir(tag: &str) -> PathBuf {
    let d = lain_store::test_dir(&format!("dl-{tag}"));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn settings(tag: &str) -> DownloadSettings {
    DownloadSettings { dir: dir(tag).to_string_lossy().into_owned(), concurrency: 2, ..Default::default() }
}

fn open(data: &Path, s: DownloadSettings) -> Arc<Manager> {
    let store = Arc::new(Store::open(data).unwrap());
    let m = Manager::open(store, s, Arc::new(PlainHttp::new())).unwrap();
    m.set_backoff(|_| Duration::ZERO);
    m
}

fn wait_state(m: &Manager, id: &str, want: &str) -> Job {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let j = m.get(id).unwrap_or_default();
        if j.state == want {
            return j;
        }
        assert!(Instant::now() < deadline, "job {id}: state {} ({} {}), want {want}", j.state, j.code, j.error);
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wait_bytes(m: &Manager, id: &str, at_least: i64) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while m.get(id).map(|j| j.bytes).unwrap_or(0) < at_least {
        assert!(Instant::now() < deadline, "job {id} never reached {at_least} bytes");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Serves the payload with ranges, stalling each response after `after`
/// bytes until the gate opens.
fn gated(after: usize) -> (Arc<Srv>, Gate) {
    let gate = Gate::default();
    let g = gate.clone();
    let body = payload();
    (Srv::start(move |req, s| serve_content(req, s, &body, r#""fixture""#, Some((&g, after)))), gate)
}

#[derive(Clone, Copy, Default)]
struct Step {
    status: u16,
    cut: usize,
    no_tag: bool,
}

/// Serves the payload; `script` decides per request (1-based) whether to
/// answer a status, drop the connection mid-body, or serve.
fn flaky(script: Vec<Step>) -> Arc<Srv> {
    let n = Arc::new(AtomicUsize::new(0));
    let body = payload();
    Srv::start(move |req: &TestReq, s| {
        let i = n.fetch_add(1, Ordering::SeqCst);
        let st = script.get(i).copied().unwrap_or_default();
        if st.status != 0 {
            respond(s, st.status, &[], b"");
            return;
        }
        let rg = req.header("range");
        let start: usize =
            rg.strip_prefix("bytes=").and_then(|v| v.strip_suffix('-')).and_then(|v| v.parse().ok()).unwrap_or(0);
        let total = body.len();
        let cr = format!("bytes {start}-{}/{total}", total - 1);
        let mut hs = vec![("Accept-Ranges", "bytes")];
        if !st.no_tag {
            hs.push(("ETag", r#""v1""#));
        }
        let status = if start > 0 {
            hs.push(("Content-Range", cr.as_str()));
            206
        } else {
            200
        };
        let rest = &body[start..];
        let _ = s.write_all(&head(status, &hs, Some(rest.len())));
        if st.cut > 0 {
            let _ = s.write_all(&rest[..st.cut]);
            let _ = s.flush();
            let _ = s.shutdown(std::net::Shutdown::Both);
            return;
        }
        let _ = s.write_all(rest);
    })
}

#[test]
fn downloads_into_dir_and_reports_done() {
    let (o, _gate) = gated(usize::MAX);
    let m = open(&dir("done-data"), settings("done"));
    let got = Arc::new(Mutex::new(Vec::<Job>::new()));
    let g2 = got.clone();
    m.on_done(Arc::new(move |j| g2.lock().unwrap().push(j.clone())));
    let changes = Arc::new(AtomicUsize::new(0));
    let c2 = changes.clone();
    m.on_change(Arc::new(move |_| {
        c2.fetch_add(1, Ordering::SeqCst);
    }));
    m.start();
    let lib = dir("done-lib");
    let j = m
        .add(AddInput {
            url: format!("{}/media/Frieren%20-%2001.mkv", o.url),
            dir: lib.to_string_lossy().into(),
            library_id: "lib-1".into(),
            created_by: "u1".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!((j.name.as_str(), j.state.as_str()), ("Frieren - 01.mkv", JOB_QUEUED));
    let done = wait_state(&m, &j.id, JOB_DONE);
    let want = lib.join("Frieren - 01.mkv");
    assert_eq!(std::fs::read(&want).unwrap(), payload());
    assert_eq!(done.path, want.to_string_lossy());
    assert!(!part_path(&done).exists(), "the part must be renamed away");
    m.close();
    let got = got.lock().unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].library_id, "lib-1");
    assert!(changes.load(Ordering::SeqCst) >= 3, "queued, running, done");
}

#[test]
fn pause_keeps_bytes_and_resume_continues() {
    let (o, gate) = gated(10_000);
    let m = open(&dir("pause-data"), settings("pause"));
    m.start();
    let j = m.add(AddInput { url: format!("{}/a.cbz", o.url), ..Default::default() }).unwrap();
    wait_bytes(&m, &j.id, 10_000);
    // Pause answers with the settled state, not "running".
    let p = m.pause(&j.id).unwrap();
    assert_eq!(p.state, JOB_PAUSED);
    let p = wait_state(&m, &j.id, JOB_PAUSED);
    assert_eq!((std::fs::metadata(part_path(&p)).unwrap().len(), p.bytes), (10_000, 10_000));
    gate.open();
    m.resume(&j.id).unwrap();
    let d = wait_state(&m, &j.id, JOB_DONE);
    assert_eq!(std::fs::read(&d.path).unwrap(), payload());
    assert_eq!(o.ranges(), ["", "bytes=10000-"]);
    m.close();
}

#[test]
fn cancel_deletes_part() {
    let (o, gate) = gated(5_000);
    let m = open(&dir("cancel-data"), settings("cancel"));
    m.start();
    let j = m.add(AddInput { url: format!("{}/a.mkv", o.url), ..Default::default() }).unwrap();
    wait_bytes(&m, &j.id, 5_000);
    assert_eq!(m.cancel(&j.id).unwrap().state, JOB_CANCELED);
    let c = wait_state(&m, &j.id, JOB_CANCELED);
    assert!(!part_path(&c).exists(), "canceled part must be deleted");
    assert_eq!(m.resume(&j.id).unwrap_err().code, CODE_STATE);
    assert!(m.cancel(&j.id).is_ok(), "cancel is idempotent");
    gate.open();
    m.close();
}

#[test]
fn pause_queued_and_state_errors() {
    let m = open(&dir("states-data"), settings("states")); // not started: jobs stay queued
    let j = m.add(AddInput { url: "http://origin.test/a.mkv".into(), ..Default::default() }).unwrap();
    assert_eq!(m.pause(&j.id).unwrap().state, JOB_PAUSED);
    assert_eq!(m.remove(&j.id).unwrap_err().code, CODE_STATE);
    assert_eq!(m.pause("nope").unwrap_err().code, CODE_NOT_FOUND);
    m.cancel(&j.id).unwrap();
    m.remove(&j.id).unwrap();
    assert!(m.list().is_empty());
}

#[test]
fn restart_resumes_running_job() {
    let (o, gate) = gated(20_000);
    let data = dir("restart-data");
    let s = settings("restart");
    let m = open(&data, s.clone());
    m.start();
    let j = m.add(AddInput { url: format!("{}/a.mkv", o.url), ..Default::default() }).unwrap();
    wait_bytes(&m, &j.id, 20_000);
    m.close(); // shutdown mid-transfer
    drop(m);

    let m = open(&data, s);
    let r = m.get(&j.id).unwrap();
    assert_eq!((r.state.as_str(), r.bytes), (JOB_QUEUED, 20_000));
    gate.open();
    m.start();
    let d = wait_state(&m, &j.id, JOB_DONE);
    assert_eq!(std::fs::read(&d.path).unwrap(), payload());
    assert_eq!(o.ranges().last().unwrap(), "bytes=20000-");
    m.close();
}

#[test]
fn quota_fails_job_and_refuses_when_full() {
    let (o, _g) = gated(usize::MAX);
    let mut s = settings("quota");
    let n = payload().len() as i64;
    s.limits.max_bytes = n + 100;
    let m = open(&dir("quota-data"), s.clone());
    m.start();
    let first = m.add(AddInput { url: format!("{}/a.mkv", o.url), ..Default::default() }).unwrap();
    wait_state(&m, &first.id, JOB_DONE);
    let second = m.add(AddInput { url: format!("{}/b.mkv", o.url), ..Default::default() }).unwrap();
    let f = wait_state(&m, &second.id, JOB_FAILED);
    assert_eq!(f.code, CODE_QUOTA);
    if let Ok(md) = std::fs::metadata(part_path(&f)) {
        assert!(md.len() <= 100, "a refused job wrote {} bytes", md.len());
    }
    // Raising the budget is a setting, and the failed job resumes.
    s.limits.max_bytes = 0;
    m.set_settings(s.clone()).unwrap();
    m.resume(&second.id).unwrap();
    wait_state(&m, &second.id, JOB_DONE);
    // Over budget, add refuses at the door.
    s.limits.max_bytes = 10;
    m.set_settings(s).unwrap();
    assert_eq!(m.add(AddInput { url: format!("{}/c.mkv", o.url), ..Default::default() }).unwrap_err().code, CODE_QUOTA);
    let u = m.usage();
    assert_eq!((u.used_bytes, u.max_bytes), (2 * n, 10));
    m.close();
}

#[test]
fn disk_floor_fails_job() {
    let (o, _g) = gated(usize::MAX);
    let mut s = settings("floor");
    s.limits.min_free_bytes = (1 << 20) - 1000; // less than one payload of headroom
    let m = open(&dir("floor-data"), s);
    m.set_free_space(|_| Some(1 << 20));
    m.start();
    let j = m.add(AddInput { url: format!("{}/a.mkv", o.url), ..Default::default() }).unwrap();
    assert_eq!(wait_state(&m, &j.id, JOB_FAILED).code, CODE_DISK_FULL);
    m.close();
}

#[test]
fn concurrency_bound() {
    let (o, gate) = gated(1_000);
    let mut s = settings("conc");
    s.concurrency = 1;
    let m = open(&dir("conc-data"), s);
    m.start();
    let a = m.add(AddInput { url: format!("{}/a.mkv", o.url), ..Default::default() }).unwrap();
    let b = m.add(AddInput { url: format!("{}/b.mkv", o.url), ..Default::default() }).unwrap();
    wait_bytes(&m, &a.id, 1_000);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(m.get(&b.id).unwrap().state, JOB_QUEUED, "second job must wait");
    gate.open();
    wait_state(&m, &a.id, JOB_DONE);
    wait_state(&m, &b.id, JOB_DONE);
    m.close();
}

#[test]
fn never_overwrites_and_uses_origin_name() {
    let body = payload()[..100].to_vec();
    let srv = Srv::start(move |req, s| {
        let mut hs = vec![];
        if req.path.ends_with("/dl") {
            hs.push(("Content-Disposition", r#"attachment; filename="../Origin Name.cbz""#));
        }
        respond(s, 200, &hs, &body);
    });
    let s = settings("names");
    std::fs::write(Path::new(&s.dir).join("taken.mkv"), b"keep me").unwrap();
    let m = open(&dir("names-data"), s.clone());
    m.start();
    let a = m.add(AddInput { url: format!("{}/x", srv.url), name: "taken.mkv".into(), ..Default::default() }).unwrap();
    let b = m.add(AddInput { url: format!("{}/dl", srv.url), ..Default::default() }).unwrap();
    let (da, db) = (wait_state(&m, &a.id, JOB_DONE), wait_state(&m, &b.id, JOB_DONE));
    assert_eq!(Path::new(&da.path).file_name().unwrap(), "taken (2).mkv");
    assert_eq!(std::fs::read(Path::new(&s.dir).join("taken.mkv")).unwrap(), b"keep me");
    assert_eq!(db.path, Path::new(&s.dir).join("Origin Name.cbz").to_string_lossy());
    m.close();
}

#[test]
fn rejects_bad_input() {
    let m = open(&dir("bad-data"), settings("bad"));
    for u in ["", "ftp://x/a", "file:///etc/passwd", "http://", "/relative"] {
        assert_eq!(m.add(AddInput { url: u.into(), ..Default::default() }).unwrap_err().code, CODE_INVALID, "{u}");
    }
    let base = DownloadSettings { dir: "/x".into(), concurrency: 1, ..Default::default() };
    for s in [
        DownloadSettings { dir: "relative".into(), ..base.clone() },
        DownloadSettings { concurrency: 0, ..base.clone() },
        DownloadSettings { concurrency: 9, ..base.clone() },
        DownloadSettings { limits: Limits { max_bytes: -1, min_free_bytes: 0 }, ..base.clone() },
        DownloadSettings { max_retries: 21, ..base.clone() },
    ] {
        assert_eq!(m.set_settings(s.clone()).unwrap_err().code, CODE_INVALID, "{s:?}");
    }
}

#[test]
fn settings_persist() {
    let data = dir("persist");
    let m = open(&data, default_settings(&data));
    assert_eq!(m.settings().limits.max_bytes, DEFAULT_MAX_BYTES);
    let want = DownloadSettings {
        dir: data.join("srv-dl").to_string_lossy().into(),
        limits: Limits { max_bytes: 1 << 40, min_free_bytes: 1 << 30 },
        concurrency: 3,
        keep_finished_days: 7,
        max_retries: 0,
    };
    m.set_settings(want.clone()).unwrap();
    drop(m);
    assert_eq!(open(&data, default_settings(&data)).settings(), want);
}

#[test]
fn old_settings_gain_retry_default() {
    let data = dir("old");
    let srv = data.join("srv-dl").to_string_lossy().into_owned();
    {
        let store = Store::open(&data).unwrap();
        store
            .write(|tx| {
                tx.put(
                    Bucket::Downloads,
                    KEY_SETTINGS,
                    serde_json::json!({"dir": srv, "max_bytes": 1, "min_free_bytes": 0, "concurrency": 1, "keep_finished_days": 0})
                        .to_string()
                        .as_bytes(),
                )
            })
            .unwrap();
    }
    let m = open(&data, default_settings(&data));
    let s = m.settings();
    assert_eq!((s.max_retries, s.dir.as_str(), s.limits.max_bytes), (DEFAULT_MAX_RETRIES, srv.as_str(), 1));
}

#[test]
fn cleanup() {
    let mut s = settings("cleanup");
    s.keep_finished_days = 1;
    let m = open(&dir("cleanup-data"), s.clone());
    let now = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    m.set_clock(move || now);
    let add = |u: &str| m.add(AddInput { url: u.into(), ..Default::default() }).unwrap();
    let failed = add("http://origin.test/f.mkv");
    let old = add("http://origin.test/o.mkv");
    let kept = add("http://origin.test/k.mkv");
    let finished = Path::new(&s.dir).join("done.mkv");
    std::fs::write(&finished, &payload()[..10]).unwrap();
    {
        let mut g = m.lock();
        std::fs::write(part_path(&g.jobs[&failed.id]), &payload()[..300]).unwrap();
        g.jobs.get_mut(&failed.id).unwrap().bytes = 300;
        m.set_state(&mut g, &failed.id, JOB_FAILED, CODE_HTTP, "boom");
        g.jobs.get_mut(&old.id).unwrap().path = finished.to_string_lossy().into();
        m.set_state(&mut g, &old.id, JOB_DONE, "", "");
        g.jobs.get_mut(&old.id).unwrap().updated_at = 1_800_000_000 - 48 * 3600;
    }
    std::fs::write(Path::new(&s.dir).join(".lain-deadbeef.part"), &payload()[..50]).unwrap();
    let rep = m.cleanup();
    assert_eq!((rep.parts, rep.freed_bytes, rep.records), (2, 350, 1));
    assert!(finished.exists(), "cleanup must never delete a finished file");
    assert_eq!(m.get(&old.id).unwrap_err().code, CODE_NOT_FOUND);
    assert!(m.get(&kept.id).is_ok());
    assert_eq!(m.get(&failed.id).unwrap().bytes, 0);
}

// ---- retries ----------------------------------------------------------

fn retry_manager(tag: &str, retries: i64) -> Arc<Manager> {
    let mut s = settings(tag);
    s.max_retries = retries;
    open(&dir(&format!("{tag}-data")), s)
}

#[test]
fn retry_transient_statuses_then_succeed() {
    let o = flaky(vec![Step { status: 503, ..Default::default() }, Step { status: 429, ..Default::default() }]);
    let m = retry_manager("retry-ok", 3);
    m.start();
    let j = m.add(AddInput { url: format!("{}/a.mkv", o.url), ..Default::default() }).unwrap();
    let d = wait_state(&m, &j.id, JOB_DONE);
    assert_eq!(o.hits(), 3);
    assert!(d.attempts == 0 && d.code.is_empty() && d.error.is_empty(), "{d:?}");
    m.close();
}

#[test]
fn permanent_errors_do_not_retry() {
    let o = flaky(vec![Step { status: 404, ..Default::default() }]);
    let m = retry_manager("retry-404", 5);
    m.start();
    let j = m.add(AddInput { url: format!("{}/a.mkv", o.url), ..Default::default() }).unwrap();
    let f = wait_state(&m, &j.id, JOB_FAILED);
    assert!(f.attempts == 0 && f.code == CODE_HTTP, "{f:?}");
    assert_eq!(f.error, "http-error: origin answered 404");
    assert_eq!(o.hits(), 1);
    m.close();
}

#[test]
fn retries_exhaust_then_fail() {
    let s = |c| Step { status: c, ..Default::default() };
    let o = flaky(vec![s(500), s(502), s(503), s(504)]);
    let m = retry_manager("retry-exhaust", 2);
    m.start();
    let j = m.add(AddInput { url: format!("{}/a.mkv", o.url), ..Default::default() }).unwrap();
    let f = wait_state(&m, &j.id, JOB_FAILED);
    assert_eq!((o.hits(), f.attempts), (3, 2));
    // A manual resume starts a fresh retry budget and succeeds.
    assert_eq!(m.resume(&j.id).unwrap().attempts, 0);
    wait_state(&m, &j.id, JOB_DONE);
    m.close();
}

#[test]
fn interrupted_transfer_resumes_by_range() {
    let o = flaky(vec![Step { cut: 10_000, ..Default::default() }, Step { cut: 5_000, ..Default::default() }]);
    let m = retry_manager("retry-range", 3);
    m.start();
    let j = m.add(AddInput { url: format!("{}/a.mkv", o.url), ..Default::default() }).unwrap();
    let d = wait_state(&m, &j.id, JOB_DONE);
    assert_eq!(std::fs::read(&d.path).unwrap(), payload());
    assert_eq!(o.ranges(), ["", "bytes=10000-", "bytes=15000-"]);
    assert_eq!(o.if_ranges()[1], r#""v1""#);
    m.close();
}

#[test]
fn interrupted_transfer_without_validator_resumes_when_ranges_advertised() {
    let o = flaky(vec![
        Step { cut: 8_000, no_tag: true, ..Default::default() },
        Step { no_tag: true, ..Default::default() },
    ]);
    let m = retry_manager("retry-notag", 3);
    m.start();
    let j = m.add(AddInput { url: format!("{}/a.mkv", o.url), ..Default::default() }).unwrap();
    let d = wait_state(&m, &j.id, JOB_DONE);
    assert_eq!(std::fs::read(&d.path).unwrap(), payload());
    assert_eq!(o.ranges()[1], "bytes=8000-");
    assert_eq!(o.if_ranges()[1], "");
    m.close();
}

#[test]
fn backoff_doubles_with_jitter() {
    for (attempt, base) in [(1, 5), (3, 20), (50, 600)] {
        let d = default_backoff(attempt).as_secs_f64();
        let b = base as f64;
        assert!(d >= b * 0.8 && d <= b * 1.2, "backoff({attempt}) = {d}, want {b} ±20%");
    }
}

#[test]
fn retry_waits_for_retry_at() {
    let o = flaky(vec![Step { status: 503, ..Default::default() }]);
    let m = retry_manager("retry-at", 3);
    m.set_backoff(|_| Duration::from_secs(2));
    m.start();
    let j = m.add(AddInput { url: format!("{}/a.mkv", o.url), ..Default::default() }).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let g = m.get(&j.id).unwrap();
        if g.attempts == 1 && g.state == JOB_QUEUED {
            assert!(g.retry_at > unix(SystemTime::now()) && g.code == CODE_HTTP, "{g:?}");
            break;
        }
        assert!(Instant::now() < deadline, "never queued a retry: {g:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(o.hits(), 1, "retried before retry_at");
    wait_state(&m, &j.id, JOB_DONE); // the retry timer fires on its own
    m.close();
}
