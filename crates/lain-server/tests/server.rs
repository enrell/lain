//! The Rust server end to end: the conformance suites (fresh, v2 and
//! seeded-after-import) run in-process against `lain-rs`, plus the
//! import's read-only guarantees and the event stream's live token
//! check.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

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

/// The fixture written by the Go server (crates/lain-store/testdata),
/// as a Go data dir.
fn go_fixture_dir(root: &Path) -> PathBuf {
    let go = root.join("go");
    std::fs::create_dir_all(go.join("plugins")).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../lain-store/testdata/go-server.bbolt"),
        go.join("lain.db"),
    )
    .unwrap();
    std::fs::write(go.join("composition.json"), br#"{"version":8,"bindings":{}}"#).unwrap();
    go
}

fn sha_tree(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out: Vec<(PathBuf, Vec<u8>)> = lain_server::import::fingerprint(dir)
        .into_keys()
        .map(|p| {
            let bytes = std::fs::read(&p).unwrap();
            (p, lain_auth::crypto::sha256(&bytes).to_vec())
        })
        .collect();
    out.sort();
    out
}

#[test]
fn seeded_suite_passes_after_import() {
    let root = lain_store::test_dir("rs-import");
    let go = go_fixture_dir(&root);
    let before = sha_tree(&go);
    let data = root.join("data");
    let report = lain_server::import::run(&go, &data, None).unwrap();
    assert_eq!(sha_tree(&go), before, "the Go data dir changed");
    let counts = report.to_json()["counts"].clone();
    assert_eq!(
        (counts["users"].as_u64(), counts["items"].as_u64(), counts["progress"].as_u64()),
        (Some(2), Some(2), Some(2))
    );
    assert!(report.skipped.iter().any(|s| s.starts_with("composition.json")));
    assert!(data.join("import-report.json").exists());
    // A second import into the same target is refused.
    assert!(lain_server::import::run(&go, &data, None).unwrap_err().contains("fresh data dir"));

    // The seeded suite's variables come from the fixture itself.
    let f = lain_store::bbolt::BoltFile::from_bytes(std::fs::read(go.join("lain.db")).unwrap()).unwrap();
    let lib: Value = serde_json::from_slice(&f.pairs("libraries").unwrap()[0].1).unwrap();
    let mut items: Vec<Value> =
        f.pairs("items").unwrap().iter().map(|(_, v)| serde_json::from_slice(v).unwrap()).collect();
    items.sort_by_key(|i| i["episode"].as_i64());
    let server = Server::start(&data);
    let mut runner = Runner::new(Base::parse(&server.url).unwrap());
    runner.vars.insert("lib_id".into(), lib["id"].clone());
    runner.vars.insert("media_dir".into(), lib["path"].clone());
    runner.vars.insert("item1".into(), items[0]["id"].clone());
    runner.vars.insert("item2".into(), items[1]["id"].clone());
    // A token minted with the imported secret, exactly as the Go server
    // mints one (the conformance run proves a real Go token verifies).
    let auth = server.app.auth();
    let admin = auth.get_public("user-admin").unwrap();
    runner.vars.insert("go_token".into(), json!(auth.mint(&admin, lain_core::now_unix())));
    run(&mut runner, "seeded");
    drop(server);
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn import_refuses_unsafe_sources_and_targets() {
    let root = lain_store::test_dir("rs-import-guard");
    let go = go_fixture_dir(&root);
    // Same dir, and a target inside the Go dir.
    assert!(lain_server::import::run(&go, &go, None).is_err());
    assert!(lain_server::import::run(&go, &go.join("inner"), None).is_err());
    // A running Go server holds bbolt's exclusive lock.
    let f = std::fs::File::open(go.join("lain.db")).unwrap();
    f.lock().unwrap();
    let e = lain_server::import::run(&go, &root.join("data"), None).unwrap_err();
    assert!(e.contains("locked"), "{e}");
    f.unlock().unwrap();
    // The Rust server never opens a Go data dir.
    let e = App::open(Config::new(&go, "t")).err().unwrap();
    assert!(e.contains("lain-rs import"), "{e}");
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn import_reads_v01_json_documents() {
    let root = lain_store::test_dir("rs-import-json");
    let go = root.join("go");
    std::fs::create_dir_all(&go).unwrap();
    let hash = lain_auth::hash_password("password123").unwrap();
    let users = json!([{"id": "user-admin", "username": "admin", "pass_hash": lain_auth::crypto::b64_std(hash.as_bytes()), "created_at": 1}]);
    std::fs::write(go.join("users.json"), users.to_string()).unwrap();
    std::fs::write(go.join("secret.json"), "\"c2VjcmV0LXNlY3JldC1zZWNyZXQtc2VjcmV0LXNlY3JldA\"").unwrap();
    std::fs::write(
        go.join("libraries.json"),
        r#"[{"id":"lib-1","name":"A","type":"anime","path":"/m","source":"lain-source-filesystem","created_at":1}]"#,
    )
    .unwrap();
    std::fs::write(
        go.join("catalog.json"),
        r#"{"i1":{"id":"i1","library_id":"lib-1","kind":"episode","title":"T","season":0,"episode":1,"year":0,"file_path":"/m/a.mkv","size":0,"confidence":0.5,"origin":"x","provenance":"y","updated_at":1}}"#,
    )
    .unwrap();
    std::fs::write(go.join("userstate.json"), "{\"user-admin\\u0000i1\":{\"item_id\":\"i1\",\"user_id\":\"user-admin\",\"position_sec\":3,\"duration_sec\":0,\"completed\":false,\"updated_at\":1}}").unwrap();
    let data = root.join("data");
    let report = lain_server::import::run(&go, &data, None).unwrap();
    assert_eq!(report.format, "v0.1 JSON documents");
    let app = App::open(Config::new(&data, "t")).unwrap();
    let auth = app.auth();
    let tok = auth.login("admin", "password123").unwrap();
    assert_eq!(auth.verify(&tok).unwrap().role, "admin");
    let (page, _) =
        app.registry.call_one("lain.catalog.read@1", "page", json!({"page": {"Limit": 50, "Sort": "title"}})).unwrap();
    assert_eq!(page["total"], 1);
    let (p, _) = app
        .registry
        .call_one("lain.userstate.progress@1", "get", json!({"user_id": "user-admin", "item_id": "i1"}))
        .unwrap();
    assert_eq!(p["position_sec"], 3.0);
    drop(app);
    std::fs::remove_dir_all(root).ok();
}
