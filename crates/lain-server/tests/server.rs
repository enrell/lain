//! The Rust server end to end: the conformance suites (fresh, v2 and
//! private) run in-process against `lain-rs`, plus the event stream's
//! live token check and the catalog export round trip.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use lain_conformance::cases::{Outcome, Runner};
use lain_conformance::http::{self, Base};
use lain_server::app::{App, Config};

struct Server {
    url: String,
    app: Arc<App>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    rt: Option<tokio::runtime::Runtime>,
}

impl Server {
    fn start(dir: &Path) -> Server {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let mut cfg = Config::new(dir, lain_server::VERSION);
        cfg.keep_alive = Duration::from_millis(300);
        cfg.reload_debounce = Duration::from_millis(100);
        let app = App::open(cfg).unwrap();
        let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let a = app.clone();
        rt.block_on(async { a.start_background() });
        rt.spawn(lain_server::serve(app.clone(), listener, async move {
            let _ = rx.await;
        }));
        Server { url, app, stop: Some(tx), rt: Some(rt) }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        self.app.shutdown();
        if let Some(rt) = self.rt.take() {
            rt.shutdown_timeout(Duration::from_secs(5));
        }
    }
}

fn cases_dir(suite: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/cases").join(suite)
}

fn run(runner: &mut Runner, suite: &str) {
    let reports = runner.run_suite(&cases_dir(suite), &["v1".into(), "v2".into()]).unwrap();
    assert!(!reports.is_empty(), "suite {suite} ran no cases");
    let failed: Vec<String> = reports
        .iter()
        .filter_map(|r| match &r.outcome {
            Outcome::Fail(e) => Some(format!("{} :: {}: {e}", r.file, r.name)),
            Outcome::Skip(e) => Some(format!("{} :: {} skipped: {e}", r.file, r.name)),
            Outcome::Pass => None,
        })
        .collect();
    assert!(failed.is_empty(), "suite {suite}:\n{}", failed.join("\n"));
}

#[test]
fn fresh_and_v2_suites_pass() {
    let dir = lain_store::test_dir("rs-fresh");
    let media = dir.join("media");
    std::fs::create_dir_all(media.join("Movies")).unwrap();
    let data = dir.join("data");
    let server = Server::start(&data);
    let mut runner = Runner::new(Base::parse(&server.url).unwrap());
    runner.vars.insert("media_dir".into(), json!(media.to_string_lossy()));
    runner.vars.insert("media_dir2".into(), json!(media.join("Movies").to_string_lossy()));
    runner.vars.insert("data_dir".into(), json!(data.to_string_lossy()));
    run(&mut runner, "fresh");
    run(&mut runner, "v2");

    // A rotated password or a disabled account ends an open stream at
    // the next keep-alive.
    let base = Base::parse(&server.url).unwrap();
    let bob = runner.vars["bob_token"].as_str().unwrap().to_string();
    let admin = runner.vars["admin_token"].as_str().unwrap().to_string();
    let stream = http::open_sse(&base, &format!("/api/events?token={bob}"), &[]).unwrap();
    assert_eq!(stream.status, 200);
    assert_eq!(stream.events.recv_timeout(Duration::from_secs(5)).unwrap().event, "ready");
    let r = http::request(
        &base,
        "PATCH",
        "/api/users/user-bob",
        &[("Authorization".into(), format!("Bearer {admin}"))],
        Some(br#"{"disabled":true}"#),
    )
    .unwrap();
    assert_eq!(r.status, 200);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match stream.events.recv_timeout(Duration::from_millis(200)) {
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            _ if std::time::Instant::now() > deadline => panic!("the stream outlived the disabled account"),
            _ => {}
        }
    }
    drop(server);
    std::fs::remove_dir_all(dir).ok();
}

/// The private-library suite on a data dir prepared like
/// `conformance/run.sh` does: a `media` root and the fixture catalog.
#[test]
fn private_suite_passes() {
    let root = lain_store::test_dir("rs-private");
    let media = root.join("media");
    std::fs::create_dir_all(media.join("Frieren")).unwrap();
    let data = root.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let roots = json!({"format": "lain.roots@1", "roots": {"media": media.to_string_lossy()}});
    std::fs::write(data.join("roots.json"), serde_json::to_vec(&roots).unwrap()).unwrap();
    let raw =
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/fixtures/catalog.json")).unwrap();
    assert_eq!(lain_server::import(&data, &serde_json::from_slice(&raw).unwrap()).unwrap(), 2);
    let server = Server::start(&data);
    let mut runner = Runner::new(Base::parse(&server.url).unwrap());
    runner.vars.insert("media_root".into(), json!(media.to_string_lossy()));
    runner.vars.insert("data_dir".into(), json!(data.to_string_lossy()));
    run(&mut runner, "private");
    drop(server);

    // The catalog travels as @3 and comes back whole.
    let doc = lain_server::export(&data).unwrap();
    assert_eq!((doc.version, doc.roots.clone(), doc.items.len()), (3, vec!["media".to_string()], 2));
    let file = &doc.items["item-frieren-01"].file_path;
    assert!(file.ends_with("01 [1080p].mkv") && Path::new(file).starts_with(&media), "{file}");
    let other = root.join("other");
    assert_eq!(lain_server::import(&other, &doc).unwrap(), 2);
    let mut old = doc.clone();
    old.version = 2;
    assert!(lain_server::import(&other, &old).unwrap_err().contains("@3"));
    std::fs::remove_dir_all(root).ok();
}
