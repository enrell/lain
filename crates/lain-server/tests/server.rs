//! The Rust server end to end: the conformance suites (fresh, v2,
//! private and read) run in-process against `lain-rs`, plus the event
//! stream's live token check, the catalog export round trip, avatar
//! size limits and the backup archive.

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
        Self::start_with(dir, |_| {})
    }

    fn start_with(dir: &Path, tune: impl FnOnce(&mut Config)) -> Server {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let mut cfg = Config::new(dir, lain_server::VERSION);
        cfg.keep_alive = Duration::from_millis(300);
        cfg.reload_debounce = Duration::from_millis(100);
        // No test reaches the network: a test that wants the network
        // providers points them at the upstream fixture server.
        cfg.offline = true;
        tune(&mut cfg);
        // Never the host's own Omarchy colors.
        cfg.theme_path = Some(dir.join("no-colors.toml"));
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

/// The scan suite (catalog population) on a fresh server with library
/// watching and the post-scan metadata pass off, like `run.sh` runs it;
/// the network metadata providers talk to the upstream fixture server.
#[test]
fn scan_suite_passes() {
    let dir = lain_store::test_dir("rs-scan");
    let media = dir.join("media");
    std::fs::create_dir_all(&media).unwrap();
    let upstream = lain_conformance::upstream::Upstream::start(&lain_conformance::upstream::dir(), 0).unwrap();
    let server = Server::start_with(&dir.join("data"), |c| {
        c.watch = false;
        c.auto_enrich = false;
        c.offline = false;
        c.metadata_endpoints = lain_metadata::Endpoints::under(&upstream.url);
    });
    let mut runner = Runner::new(Base::parse(&server.url).unwrap());
    runner.vars.insert("scan_media".into(), json!(media.to_string_lossy()));
    runner.vars.insert("upstream".into(), json!(upstream.url));
    run(&mut runner, "scan");
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

fn call(
    base: &Base,
    method: &str,
    path: &str,
    token: &str,
    body: Option<serde_json::Value>,
) -> (u16, serde_json::Value) {
    let mut headers = vec![("Authorization".to_string(), format!("Bearer {token}"))];
    let raw = body.map(|b| {
        headers.push(("Content-Type".into(), "application/json".into()));
        serde_json::to_vec(&b).unwrap()
    });
    let r = http::request(base, method, path, &headers, raw.as_deref()).unwrap();
    (r.status, serde_json::from_slice(&r.body).unwrap_or(serde_json::Value::Null))
}

fn login(base: &Base, user: &str, pw: &str) -> String {
    let (st, v) = call(base, "POST", "/api/auth/login", "", Some(json!({"username": user, "password": pw})));
    assert_eq!(st, 200, "{v}");
    v["token"].as_str().unwrap().to_string()
}

fn scan_to_done(base: &Base, admin: &str) -> serde_json::Value {
    let (st, _) = call(base, "POST", "/api/library/scan", admin, None);
    assert_eq!(st, 202);
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let (_, v) = call(base, "GET", "/api/library/scan", admin, None);
        if v["state"] == "done" {
            return v;
        }
        assert!(std::time::Instant::now() < deadline, "scan did not finish: {v}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A rooted library (D-157) scans into `root` + `rel_path`, every new
/// read path keeps a private library's items from another user
/// (D-156), and a scan's events and unreadable roots follow suit.
#[test]
fn rooted_private_scans_leak_nothing() {
    let dir = lain_store::test_dir("rs-scan-private");
    let media = dir.join("media");
    let show = media.join("anime/Frieren");
    std::fs::create_dir_all(&show).unwrap();
    std::fs::write(show.join("[Fansub-A] Frieren - 01 [1080p].mkv"), b"x").unwrap();
    std::fs::write(show.join("[Fansub-A] Frieren - 01 [1080p].en.srt"), b"1\n00:00:01,000 --> 00:00:02,000\nHi\n")
        .unwrap();
    std::fs::write(show.join("tvshow.nfo"), b"<tvshow><title>Sousou no Frieren</title></tvshow>").unwrap();
    let data = dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let roots = json!({"format": "lain.roots@1", "roots": {"media": media.to_string_lossy()}});
    std::fs::write(data.join("roots.json"), serde_json::to_vec(&roots).unwrap()).unwrap();
    let server = Server::start_with(&data, |c| {
        c.watch = false;
        c.auto_enrich = true;
    });
    let base = Base::parse(&server.url).unwrap();
    call(&base, "POST", "/api/setup", "", Some(json!({"username": "admin", "password": "password123"})));
    let admin = login(&base, "admin", "password123");
    call(&base, "POST", "/api/users", &admin, Some(json!({"username": "bob", "password": "bobpassword"})));
    let bob = login(&base, "bob", "bobpassword");
    let (st, lib) =
        call(&base, "POST", "/api/libraries", &admin, Some(json!({"name": "Anime", "path": "anime", "root": "media"})));
    assert_eq!(st, 201, "{lib}");
    let lib_id = lib["id"].as_str().unwrap().to_string();
    // A second, public library whose root is gone: an unreadable root.
    let gone = dir.join("gone");
    std::fs::create_dir_all(&gone).unwrap();
    call(&base, "POST", "/api/libraries", &admin, Some(json!({"name": "Gone", "path": gone.to_string_lossy()})));
    std::fs::remove_dir_all(&gone).unwrap();

    let st = scan_to_done(&base, &admin);
    assert_eq!(st["stats"]["identified"], 1, "{st}");
    assert_eq!(st["stats"]["enriched"], 1, "the post-scan NFO pass: {st}");
    assert_eq!(st["stats"]["unreadable"].as_array().unwrap().len(), 1);
    let (_, page) = call(&base, "GET", "/api/catalog", &admin, None);
    let item = page["items"][0].clone();
    let id = item["id"].as_str().unwrap().to_string();
    assert_eq!(item["root"], "media");
    assert_eq!(item["rel_path"], "anime/Frieren/[Fansub-A] Frieren - 01 [1080p].mkv");
    assert!(item["file_path"].as_str().unwrap().ends_with("01 [1080p].mkv"));
    // Stored without a machine path; the id is the location's.
    drop(server);
    let doc = lain_server::export(&data).unwrap();
    assert_eq!(doc.items[&id].rel_path, item["rel_path"]);
    assert_eq!(id, lain_ingest::item_id(&lib_id, "anime/Frieren/[Fansub-A] Frieren - 01 [1080p].mkv"));
    let server = Server::start_with(&data, |c| c.watch = false);
    let base = Base::parse(&server.url).unwrap();
    let admin = login(&base, "admin", "password123");
    let bob = {
        let _ = bob;
        login(&base, "bob", "bobpassword")
    };
    let (st, sc) = call(&base, "GET", &format!("/api/items/{id}/sidecars"), &bob, None);
    assert_eq!((st, sc["sidecars"][0]["language"].as_str()), (200, Some("eng")), "rooted sidecars resolve");

    // Private: bob sees nothing on any new path.
    let (st, _) = call(&base, "PATCH", &format!("/api/libraries/{lib_id}"), &admin, Some(json!({"private": true})));
    assert_eq!(st, 200);
    let admin2 = login(&base, "admin", "password123");
    let hidden = [
        format!("/api/items/{id}/sidecars"),
        format!("/api/items/{id}/sidecars/0"),
        format!("/api/catalog/{id}/enrich"),
    ];
    for p in &hidden {
        for t in [&bob, &admin2] {
            let (st, v) = call(&base, "GET", p, t, None);
            assert_eq!((st, v["error"].as_str()), (404, Some("unknown item")), "{p}");
        }
    }
    let (_, v) = call(&base, "GET", &format!("/api/enrichments?ids={id}"), &bob, None);
    assert_eq!(v, json!({"items": []}), "batch overlays of a private item");
    let (_, v) = call(&base, "GET", &format!("/api/enrichments?ids={id}"), &admin, None);
    assert_eq!(v, json!({"items": []}), "still locked in this session");
    let (st, _) = call(
        &base,
        "POST",
        &format!("/api/libraries/{lib_id}/unlock"),
        &admin,
        Some(json!({"password": "password123"})),
    );
    assert_eq!(st, 200);
    let (_, v) = call(&base, "GET", &format!("/api/enrichments?ids={id}"), &admin, None);
    assert_eq!(v["items"][0]["title"], "Sousou no Frieren");
    let (st, _) = call(&base, "POST", &format!("/api/catalog/{id}/enrich"), &admin2, None);
    assert_eq!(st, 404, "a locked session cannot enrich it either");
    let (st, _) = call(&base, "DELETE", &format!("/api/items/{id}"), &admin2, None);
    assert_eq!(st, 404, "nor delete its file");
    assert!(show.join("[Fansub-A] Frieren - 01 [1080p].mkv").exists());

    // Events: bob hears the scan but not the private library.
    let stream = http::open_sse(&base, &format!("/api/events?token={bob}"), &[]).unwrap();
    assert_eq!(stream.events.recv_timeout(Duration::from_secs(5)).unwrap().event, "ready");
    let st = scan_to_done(&base, &admin);
    let (_, bob_view) = call(&base, "GET", "/api/library/scan", &bob, None);
    assert_eq!(st["stats"]["unreadable"].as_array().unwrap().len(), 1);
    // Make the unreadable library private too: admins still see it, bob does not.
    let gone_id = call(&base, "GET", "/api/libraries", &admin, None)
        .1
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["name"] == "Gone")
        .unwrap()["id"]
        .clone();
    call(
        &base,
        "PATCH",
        &format!("/api/libraries/{}", gone_id.as_str().unwrap()),
        &admin,
        Some(json!({"private": true})),
    );
    let _ = bob_view;
    let st = scan_to_done(&base, &admin);
    assert_eq!(st["stats"]["unreadable"].as_array().unwrap().len(), 1);
    let (_, bob_view) = call(&base, "GET", "/api/library/scan", &bob, None);
    assert!(bob_view["stats"].get("unreadable").is_none(), "{bob_view}");
    let mut names = Vec::new();
    let mut last_done = serde_json::Value::Null;
    while let Ok(ev) = stream.events.recv_timeout(Duration::from_millis(500)) {
        if ev.event == "scan-state" {
            let d: serde_json::Value = serde_json::from_str(&ev.data).unwrap();
            if d["state"] == "done" {
                last_done = d;
            }
        }
        if ev.event == "library-changed" {
            let d: serde_json::Value = serde_json::from_str(&ev.data).unwrap();
            assert_ne!(d["library_id"], lib_id, "a private library's scan reached bob");
        }
        names.push(ev.event);
    }
    assert!(names.iter().filter(|n| *n == "scan-state").count() >= 4, "{names:?}");
    assert_eq!(last_done["stats"]["candidates"], 1, "{last_done}");
    assert!(last_done["stats"].get("unreadable").is_none(), "a private root reached bob: {last_done}");
    drop(stream);
    drop(server);
    std::fs::remove_dir_all(dir).ok();
}

/// File events rescan the library on their own (D-068): a new file
/// lands, a deleted one reads missing.
#[test]
fn watched_libraries_rescan_on_file_events() {
    let dir = lain_store::test_dir("rs-watch");
    let media = dir.join("media");
    std::fs::create_dir_all(media.join("Show")).unwrap();
    let server = Server::start_with(&dir.join("data"), |c| {
        c.auto_enrich = false;
        c.watch_debounce = Duration::from_millis(200);
    });
    let base = Base::parse(&server.url).unwrap();
    call(&base, "POST", "/api/setup", "", Some(json!({"username": "admin", "password": "password123"})));
    let admin = login(&base, "admin", "password123");
    call(
        &base,
        "POST",
        "/api/libraries",
        &admin,
        Some(json!({"name": "TV", "type": "show", "path": media.to_string_lossy()})),
    );
    let wait = |want: &dyn Fn(&serde_json::Value) -> bool| {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            let (_, page) = call(&base, "GET", "/api/catalog", &admin, None);
            if want(&page) {
                return page;
            }
            assert!(std::time::Instant::now() < deadline, "the watcher did not rescan: {page}");
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    // The watch loop picks the new library up on its next poll.
    std::thread::sleep(Duration::from_millis(800));
    let file = media.join("Show/Show.Name.S01E01.1080p.mkv");
    std::fs::write(&file, b"x").unwrap();
    let page = wait(&|p| p["total"] == 1);
    assert_eq!(page["items"][0]["title"], "Show Name");
    let (_, st) = call(&base, "GET", "/api/library/scan", &admin, None);
    assert_eq!(st["trigger"], "watch");
    std::fs::remove_file(&file).unwrap();
    wait(&|p| p["items"][0]["missing"] == true);
    drop(server);
    std::fs::remove_dir_all(dir).ok();
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/fixtures")
}

/// The read suite: root `media` at the committed fixture archives and
/// the read fixture catalog, as `conformance/run.sh` prepares it.
#[test]
fn read_suite_passes() {
    let root = lain_store::test_dir("rs-read");
    let media = fixtures().join("read");
    let data = root.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let roots = json!({"format": "lain.roots@1", "roots": {"media": media.to_string_lossy()}});
    std::fs::write(data.join("roots.json"), serde_json::to_vec(&roots).unwrap()).unwrap();
    let raw = std::fs::read(fixtures().join("read-catalog.json")).unwrap();
    assert_eq!(lain_server::import(&data, &serde_json::from_slice(&raw).unwrap()).unwrap(), 5);
    // The fixture catalog stays as imported: no watch, no scan.
    let server = Server::start_with(&data, |c| {
        c.watch = false;
        c.auto_enrich = false;
    });
    let mut runner = Runner::new(Base::parse(&server.url).unwrap());
    runner.vars.insert("media_root".into(), json!(media.to_string_lossy()));
    runner.vars.insert("data_dir".into(), json!(data.to_string_lossy()));
    run(&mut runner, "read");
    drop(server);
    std::fs::remove_dir_all(root).ok();
}

/// What the case files cannot carry: a 2 MiB + 1 upload, and the
/// backup archive's contents.
#[test]
fn avatar_limit_and_backup_archive() {
    let root = lain_store::test_dir("rs-admin");
    let data = root.join("data");
    let server = Server::start(&data);
    let base = Base::parse(&server.url).unwrap();
    let setup = serde_json::to_vec(&json!({"username": "admin", "password": "password123"})).unwrap();
    assert_eq!(http::request(&base, "POST", "/api/setup", &[], Some(&setup)).unwrap().status, 201);
    let auth = vec![("Authorization".to_string(), format!("Bearer {}", login(&base, "admin", "password123")))];

    let big = vec![0u8; lain_admin::avatars::MAX_BYTES + 1];
    let r = http::request(&base, "PUT", "/api/me/avatar", &auth, Some(&big)).unwrap();
    assert_eq!(r.status, 413, "{}", String::from_utf8_lossy(&r.body));
    assert!(String::from_utf8_lossy(&r.body).contains("the image must be 2 MB or smaller"));
    let png = lain_read::image::test_png(32, 32);
    assert_eq!(http::request(&base, "PUT", "/api/me/avatar", &auth, Some(&png)).unwrap().status, 200);

    let r = http::request(&base, "GET", "/api/admin/backup", &auth, None).unwrap();
    assert_eq!(r.status, 200);
    let disposition = r.header("content-disposition").unwrap().to_string();
    assert!(disposition.starts_with("attachment; filename=\"lain-backup-") && disposition.ends_with(".tar\""));
    assert_eq!(r.header("content-length").unwrap(), r.body.len().to_string());
    let entries = lain_admin::backup::read_tar(&r.body).unwrap();
    let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["manifest.json", "lain.redb", "composition.json", "avatars/user-admin"]);
    let manifest: serde_json::Value = serde_json::from_slice(&entries[0].1).unwrap();
    assert_eq!(manifest["format"], "lain.backup@1");
    let comp: serde_json::Value = serde_json::from_slice(&entries[2].1).unwrap();
    assert_eq!(comp["bindings"]["lain.comic.pages@1"]["providers"][0], "lain-comic-archive");
    assert_eq!(entries[3].1, png);
    // The snapshot is a store that opens on its own.
    let restored = root.join("restored");
    std::fs::create_dir_all(&restored).unwrap();
    std::fs::write(restored.join("lain.redb"), &entries[1].1).unwrap();
    let store = lain_store::Store::open(&restored).unwrap();
    assert!(store.read(|tx| tx.get(lain_store::Bucket::Users, b"user-admin")).unwrap().is_some());
    drop(store);
    drop(server);
    std::fs::remove_dir_all(root).ok();
}
