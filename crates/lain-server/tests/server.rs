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
        // The torrent engine binds a free port.
        cfg.acquire_listen_port = Some(0);
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
    // The fixture's files are there, as conformance/run.sh makes them: a
    // file that is not is told `offline` (D-287).
    for n in ["01", "02"] {
        std::fs::write(media.join(format!("Frieren/[Fansub-A] Frieren - {n} [1080p].mkv")), b"").unwrap();
    }
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

/// Waits for the metadata pass (D-302) and returns its status.
fn enrich_to_done(base: &Base, admin: &str) -> serde_json::Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let (_, v) = call(base, "GET", "/api/library/scan", admin, None);
        if v["enrich"]["state"] == "done" || v["enrich"]["state"] == "error" {
            return v["enrich"].clone();
        }
        assert!(std::time::Instant::now() < deadline, "metadata pass did not finish: {v}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// One folder per file, each with the downloader's `.metadata/meta.toml`:
/// the sidecar names the series and the episode, so the episodes join
/// into one title instead of standing as three videos. A file without a
/// sidecar is still named by its own name.
#[test]
fn sidecars_join_one_folder_per_file_episodes() {
    let dir = lain_store::test_dir("rs-sidecar");
    let media = dir.join("media");
    for n in 1..=3 {
        let name = format!("Some Show {n}");
        let folder = media.join(&name);
        std::fs::create_dir_all(folder.join(".metadata/thumbs")).unwrap();
        std::fs::write(folder.join(format!("{name}.mp4")), b"x").unwrap();
        std::fs::write(folder.join(".metadata/thumbs/thumbnail.jpg"), b"x").unwrap();
        let toml = format!(
            "schema_version = 2\nmedia_type = \"video\"\ntitle = \"{name}\"\nseries = \"Some Show\"\nepisode = \"{n}\"\nsource_site = \"site-exemplo\"\n\n[local]\nmetadata_dir = \".metadata\"\n"
        );
        std::fs::write(folder.join(".metadata/meta.toml"), toml).unwrap();
    }
    std::fs::create_dir_all(media.join("Loose Clip")).unwrap();
    std::fs::write(media.join("Loose Clip/Loose Clip.mp4"), b"x").unwrap();
    let server = Server::start_with(&dir.join("data"), |c| c.watch = false);
    let base = Base::parse(&server.url).unwrap();
    call(&base, "POST", "/api/setup", "", Some(json!({"username": "admin", "password": "password123"})));
    let admin = login(&base, "admin", "password123");
    let (st, lib) = call(
        &base,
        "POST",
        "/api/libraries",
        &admin,
        Some(json!({"name": "Shows", "type": "auto", "path": media.to_string_lossy()})),
    );
    assert_eq!(st, 201, "{lib}");
    let st = scan_to_done(&base, &admin);
    assert_eq!(st["stats"]["identified"], 4, "{st}");

    let (_, files) = call(&base, "GET", "/api/catalog?limit=50", &admin, None);
    let mut rows: Vec<(String, String, i64)> = files["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["title"].as_str().unwrap().to_string(),
                i["kind"].as_str().unwrap().to_string(),
                i["episode"].as_i64().unwrap_or(0),
            )
        })
        .collect();
    rows.sort();
    let ep = |n: i64| ("Some Show".to_string(), "episode".to_string(), n);
    assert_eq!(rows, vec![("Loose Clip".to_string(), "video".to_string(), 0), ep(1), ep(2), ep(3)], "{files}");
    // Paged by title: one row for the show, one for the clip.
    let (_, titles) = call(&base, "GET", "/api/catalog?group=title&limit=50", &admin, None);
    assert_eq!(titles["total"], 2, "{titles}");
    drop(server);
    std::fs::remove_dir_all(dir).ok();
}

/// Deleting from the title page (D-301): the file goes with what only
/// existed for it, the folders it leaves empty go too, the answer says
/// what happened, and a title with no file left is off the lists while
/// its rows stay. Nothing is deleted from a library that is not there.
#[test]
fn deleting_files_leaves_nothing_behind_and_says_what_it_did() {
    let dir = lain_store::test_dir("rs-delete");
    let media = dir.join("media");
    let show = media.join("Show");
    std::fs::create_dir_all(&show).unwrap();
    std::fs::create_dir_all(media.join("Other")).unwrap();
    std::fs::write(show.join("[Fansub-A] Show - 01 [1080p].mkv"), b"12345").unwrap();
    std::fs::write(show.join("[Fansub-A] Show - 01 [1080p].en.srt"), b"123").unwrap();
    std::fs::write(show.join("[Fansub-A] Show - 01 [1080p]-thumb.jpg"), b"12").unwrap();
    std::fs::write(show.join("[Fansub-A] Show - 02 [1080p].mkv"), b"1234567").unwrap();
    std::fs::write(media.join("Other/[Fansub-A] Other - 01 [1080p].mkv"), b"1").unwrap();
    std::fs::write(media.join("Other/notes.txt"), b"mine").unwrap();
    let server = Server::start_with(&dir.join("data"), |c| c.watch = false);
    let base = Base::parse(&server.url).unwrap();
    call(&base, "POST", "/api/setup", "", Some(json!({"username": "admin", "password": "password123"})));
    let admin = login(&base, "admin", "password123");
    let (st, lib) = call(
        &base,
        "POST",
        "/api/libraries",
        &admin,
        Some(json!({"name": "Shows", "type": "anime", "path": media.to_string_lossy()})),
    );
    assert_eq!(st, 201, "{lib}");
    scan_to_done(&base, &admin);
    let (_, files) = call(&base, "GET", "/api/catalog?limit=50", &admin, None);
    let id_of = |name: &str| {
        let it = files["items"].as_array().unwrap().iter().find(|i| i["file_path"].as_str().unwrap().ends_with(name));
        it.unwrap_or_else(|| panic!("{name} not in {files}"))["id"].as_str().unwrap().to_string()
    };
    let (one, two, other) = (
        id_of("[Fansub-A] Show - 01 [1080p].mkv"),
        id_of("[Fansub-A] Show - 02 [1080p].mkv"),
        id_of("[Fansub-A] Other - 01 [1080p].mkv"),
    );
    let titles =
        |q: &str| call(&base, "GET", &format!("/api/{q}group=title&limit=50"), &admin, None).1["total"].clone();
    assert_eq!(titles("catalog?"), 2);

    // The library's folder is away: nothing is deleted, and it is said.
    let away = dir.join("media.away");
    std::fs::rename(&media, &away).unwrap();
    let (st, v) = call(&base, "DELETE", &format!("/api/items/{one}"), &admin, None);
    assert_eq!(
        (st, v["error"].as_str()),
        (409, Some("the library's folder cannot be reached, so nothing was deleted"))
    );
    std::fs::rename(&away, &media).unwrap();
    let (_, v) = call(&base, "GET", &format!("/api/catalog/{one}"), &admin, None);
    assert!(v["missing"].is_null(), "{v}");

    // One episode: its subtitle and still go with it, the folder stays for the other.
    let (st, v) = call(&base, "DELETE", &format!("/api/items/{one}"), &admin, None);
    assert_eq!(st, 200, "{v}");
    assert_eq!(
        v["removal"],
        json!({"file": true, "freed_bytes": 10, "sidecars": 2, "folders": 0, "downloads": 0,
               "not_wanted": false, "still_linked": false})
    );
    assert_eq!(std::fs::read_dir(&show).unwrap().count(), 1, "only the other episode is left");
    assert_eq!(titles("catalog?"), 2, "a title with a file left keeps its card");
    let (_, v) = call(&base, "GET", "/api/search?q=show&limit=50", &admin, None);
    assert_eq!(v["total"], 2, "and its missing episode: {v}");

    // The last one: the folder goes, and the title is off the lists.
    let (st, v) = call(&base, "DELETE", &format!("/api/items/{two}"), &admin, None);
    assert_eq!((st, &v["removal"]["folders"], &v["removal"]["freed_bytes"]), (200, &json!(1), &json!(7)), "{v}");
    assert!(!show.exists());
    assert_eq!(titles("catalog?"), 1);
    assert_eq!(titles("search?q=show&"), 0);
    let (_, v) = call(&base, "GET", "/api/search?q=show&limit=50", &admin, None);
    assert_eq!(v["total"], 0, "{v}");
    // The rows stay (D-068): by id, and as the title's episodes.
    let (st, v) = call(&base, "GET", &format!("/api/catalog/{two}/episodes"), &admin, None);
    assert_eq!((st, v["items"].as_array().map(Vec::len)), (200, Some(2)));
    // Again changes nothing and still answers.
    let (st, v) = call(&base, "DELETE", &format!("/api/items/{two}"), &admin, None);
    assert_eq!((st, &v["removal"]["file"]), (200, &json!(false)), "{v}");

    // A folder holding something else stays, and so does the library's own.
    #[cfg(unix)]
    std::fs::hard_link(media.join("Other/[Fansub-A] Other - 01 [1080p].mkv"), dir.join("elsewhere.mkv")).unwrap();
    let (st, v) = call(&base, "DELETE", &format!("/api/items/{other}"), &admin, None);
    assert_eq!((st, &v["removal"]["folders"]), (200, &json!(0)), "{v}");
    #[cfg(unix)]
    assert_eq!((&v["removal"]["still_linked"], &v["removal"]["freed_bytes"]), (&json!(true), &json!(0)), "{v}");
    assert!(media.join("Other/notes.txt").exists() && !media.join("Other/[Fansub-A] Other - 01 [1080p].mkv").exists());
    assert_eq!(titles("catalog?"), 0);
    // A rescan agrees: nothing comes back.
    scan_to_done(&base, &admin);
    assert_eq!(titles("catalog?"), 0);
    drop(server);
    std::fs::remove_dir_all(dir).ok();
}

/// What the acquisition side keeps about a private library follows its
/// lock (D-293): its monitored titles and downloads are absent from every
/// list and id lookup, and from the events, while it is locked in this
/// session, admins included; unlocked, they are back.
#[test]
fn acquisition_follows_the_library_lock() {
    let dir = lain_store::test_dir("rs-acq-lock");
    let media = dir.join("media");
    std::fs::create_dir_all(&media).unwrap();
    let server = Server::start_with(&dir.join("data"), |c| c.watch = false);
    let base = Base::parse(&server.url).unwrap();
    call(&base, "POST", "/api/setup", "", Some(json!({"username": "admin", "password": "password123"})));
    let admin = login(&base, "admin", "password123");
    let (st, lib) = call(
        &base,
        "POST",
        "/api/libraries",
        &admin,
        Some(json!({"name": "Late", "type": "anime", "path": media.to_string_lossy()})),
    );
    assert_eq!(st, 201, "{lib}");
    let id = lib["id"].as_str().unwrap().to_string();
    let (st, _) = call(&base, "PATCH", &format!("/api/libraries/{id}"), &admin, Some(json!({"private": true})));
    assert_eq!(st, 200);
    let unlock = || {
        let r = call(
            &base,
            "POST",
            &format!("/api/libraries/{id}/unlock"),
            &admin,
            Some(json!({"password": "password123"})),
        );
        assert_eq!(r.0, 200, "{}", r.1);
    };
    let lock = || assert_eq!(call(&base, "POST", &format!("/api/libraries/{id}/lock"), &admin, None).0, 200);

    // A public library next to it, whose records stay visible throughout.
    let other = dir.join("other");
    std::fs::create_dir_all(&other).unwrap();
    let (_, open) = call(
        &base,
        "POST",
        "/api/libraries",
        &admin,
        Some(json!({"name": "Plain", "type": "anime", "path": other.to_string_lossy(), "audience": "everyone"})),
    );
    let open_id = open["id"].as_str().unwrap().to_string();

    unlock();
    let mon = |library: &str, title: &str| {
        call(
            &base,
            "POST",
            "/api/acquire/monitored",
            &admin,
            Some(json!({"library_id": library, "kind": "anime", "title": title})),
        )
    };
    let (st, m1) = mon(&id, "Hidden Show");
    assert_eq!(st, 201, "{m1}");
    let m1 = m1["id"].as_str().unwrap().to_string();
    let (st, m2) = mon(&open_id, "Plain Show");
    assert_eq!(st, 201, "{m2}");
    let m2 = m2["id"].as_str().unwrap().to_string();
    let (st, job) = call(
        &base,
        "POST",
        "/api/downloads",
        &admin,
        Some(json!({"url": "http://127.0.0.1:9/hidden.bin", "library_id": id})),
    );
    assert_eq!(st, 201, "{job}");
    let job = job["id"].as_str().unwrap().to_string();
    let titles = |path: &str, key: &str, field: &str| -> Vec<String> {
        let (st, v) = call(&base, "GET", path, &admin, None);
        assert_eq!(st, 200, "{v}");
        v[key].as_array().unwrap().iter().map(|x| x[field].as_str().unwrap_or("").to_string()).collect()
    };
    assert_eq!(titles("/api/acquire/monitored", "monitored", "title").len(), 2);
    assert_eq!(titles("/api/downloads", "jobs", "id"), vec![job.clone()]);

    lock();
    let stream = http::open_sse(&base, &format!("/api/events?token={admin}"), &[]).unwrap();
    assert_eq!(stream.events.recv_timeout(Duration::from_secs(5)).unwrap().event, "ready");
    assert_eq!(titles("/api/acquire/monitored", "monitored", "title"), vec!["Plain Show"]);
    assert_eq!(titles("/api/acquire/wanted", "titles", "monitored").len(), 1);
    assert!(titles("/api/downloads", "jobs", "id").is_empty());
    // Every way in by id answers like an id that does not exist.
    for (method, path) in [
        ("GET", format!("/api/acquire/monitored/{m1}")),
        ("PUT", format!("/api/acquire/monitored/{m1}")),
        ("DELETE", format!("/api/acquire/monitored/{m1}")),
        ("GET", format!("/api/acquire/monitored/{m1}/wanted")),
        ("POST", format!("/api/acquire/monitored/{m1}/search")),
        ("GET", format!("/api/acquire/monitored/{m1}/subtitles")),
        ("POST", format!("/api/acquire/monitored/{m1}/subtitles")),
        ("GET", format!("/api/downloads/{job}")),
        ("POST", format!("/api/downloads/{job}/pause")),
        ("DELETE", format!("/api/downloads/{job}")),
    ] {
        let body = (method == "PUT").then(|| json!({"library_id": id, "kind": "anime", "title": "x"}));
        let (st, v) = call(&base, method, &path, &admin, body);
        assert_eq!(st, 404, "{method} {path}: {v}");
    }
    // The folder picker does not list its folder, nor open it (D-293).
    let browse = |path: &std::path::Path| {
        call(&base, "GET", &format!("/api/browse?path={}", path.to_string_lossy()), &admin, None)
    };
    let names = |v: &serde_json::Value| -> Vec<String> {
        v["dirs"].as_array().unwrap().iter().map(|d| d["name"].as_str().unwrap().to_string()).collect()
    };
    let (st, v) = browse(&dir);
    assert_eq!(st, 200, "{v}");
    assert!(names(&v).contains(&"other".to_string()) && !names(&v).contains(&"media".to_string()), "{v}");
    assert_eq!(browse(&media).0, 400);
    assert_eq!(browse(&media.join("deeper/..")).0, 400, "a path that walks back in is the same folder");
    // Nor can a record be made in it, nor a title moved into it.
    let (st, v) = mon(&id, "Another");
    assert_eq!(st, 400, "{v}");
    let (st, v) = call(
        &base,
        "PUT",
        &format!("/api/acquire/monitored/{m2}"),
        &admin,
        Some(json!({"library_id": id, "kind": "anime", "title": "Plain Show"})),
    );
    assert_eq!(st, 400, "{v}");
    let (st, v) = call(
        &base,
        "POST",
        "/api/downloads",
        &admin,
        Some(json!({"url": "http://127.0.0.1:9/x.bin", "library_id": id})),
    );
    assert_eq!(st, 404, "{v}");
    // Its downloads' events do not reach a locked session; a public one's do.
    let (st, j2) = call(
        &base,
        "POST",
        "/api/downloads",
        &admin,
        Some(json!({"url": "http://127.0.0.1:9/plain.bin", "library_id": open_id})),
    );
    assert_eq!(st, 201, "{j2}");
    std::thread::sleep(Duration::from_millis(600));
    let mut seen = Vec::new();
    while let Ok(ev) = stream.events.recv_timeout(Duration::from_millis(400)) {
        if ev.event == "download-changed" {
            let d: serde_json::Value = serde_json::from_str(&ev.data).unwrap();
            seen.push(d["id"].as_str().unwrap_or("").to_string());
        }
    }
    assert!(!seen.is_empty() && !seen.contains(&job), "{seen:?} (hidden job {job})");
    assert!(seen.contains(&j2["id"].as_str().unwrap().to_string()), "{seen:?}");

    unlock();
    assert!(names(&browse(&dir).1).contains(&"media".to_string()), "unlocked, its folder is listed again");
    assert_eq!(browse(&media).0, 200);
    assert_eq!(titles("/api/acquire/monitored", "monitored", "title").len(), 2);
    assert!(titles("/api/downloads", "jobs", "id").contains(&job));
    assert_eq!(call(&base, "GET", &format!("/api/acquire/monitored/{m1}"), &admin, None).0, 200);
    drop(stream);
    drop(server);
    std::fs::remove_dir_all(dir).ok();
}

/// D-294: an unlock is a lease on the session's open event stream. The
/// laptop unlocks; the TV, signed in to the same account, stays locked
/// the whole time. Closing the laptop's stream (the tab) locks it on
/// the server after the grace, with no request from the client, and a
/// new stream does not bring the unlock back.
#[test]
fn an_unlock_ends_when_the_tab_closes() {
    let dir = lain_store::test_dir("rs-lease");
    let media = dir.join("media");
    std::fs::create_dir_all(&media).unwrap();
    let server = Server::start_with(&dir.join("data"), |c| {
        c.watch = false;
        c.unlock_grace = Duration::from_millis(400);
    });
    let base = Base::parse(&server.url).unwrap();
    call(&base, "POST", "/api/setup", "", Some(json!({"username": "admin", "password": "password123"})));
    let laptop = login(&base, "admin", "password123");
    let tv = login(&base, "admin", "password123");
    let (st, lib) =
        call(&base, "POST", "/api/libraries", &laptop, Some(json!({"name": "Late", "path": media.to_string_lossy()})));
    assert_eq!(st, 201, "{lib}");
    let id = lib["id"].as_str().unwrap().to_string();
    assert_eq!(call(&base, "PATCH", &format!("/api/libraries/{id}"), &laptop, Some(json!({"private": true}))).0, 200);
    let locked = |token: &str| {
        let (_, v) = call(&base, "GET", "/api/libraries", token, None);
        (v[0]["locked"].as_bool().unwrap(), v[0]["name"].as_str().unwrap().to_string())
    };
    let open_stream = |token: &str| {
        let s = http::open_sse(&base, &format!("/api/events?token={token}"), &[]).unwrap();
        assert_eq!(s.events.recv_timeout(Duration::from_secs(5)).unwrap().event, "ready");
        s
    };
    let wait = || std::thread::sleep(Duration::from_millis(900));

    let tab = open_stream(&laptop);
    let tv_stream = open_stream(&tv);
    let unlock = json!({"password": "password123"});
    assert_eq!(call(&base, "POST", &format!("/api/libraries/{id}/unlock"), &laptop, Some(unlock.clone())).0, 200);
    wait();
    assert_eq!(locked(&laptop), (false, "Late".to_string()), "held by the open stream");
    assert_eq!(locked(&tv), (true, String::new()), "the TV's own login never unlocks with it");

    drop(tab);
    wait();
    assert_eq!(locked(&laptop), (true, String::new()), "the tab closed: locked on the server");
    let _tab = open_stream(&laptop);
    assert!(locked(&laptop).0, "coming back does not unlock");

    // Without any stream an unlock lasts the grace only.
    assert_eq!(call(&base, "POST", &format!("/api/libraries/{id}/unlock"), &tv, Some(unlock)).0, 200);
    drop(tv_stream);
    assert!(!locked(&tv).0);
    wait();
    assert!(locked(&tv).0);
    drop(server);
    std::fs::remove_dir_all(dir).ok();
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
    assert_eq!(st["stats"]["enriched"], 0, "the scan itself fetches no metadata (D-302): {st}");
    let e = enrich_to_done(&base, &admin);
    assert_eq!((e["state"].as_str(), e["enriched"].as_i64()), (Some("done"), Some(1)), "the post-scan NFO pass: {e}");
    let (code, v) = call(&base, "POST", "/api/library/enrich", &admin, Some(json!({"retry": true})));
    assert_eq!((code, v["state"].as_str()), (202, Some("running")), "{v}");
    let (code, _) = call(&base, "POST", "/api/library/enrich", &bob, None);
    assert_eq!(code, 403, "a user cannot start the metadata pass");
    let e = enrich_to_done(&base, &admin);
    assert_eq!(e["enriched"], 0, "nothing left without an overlay: {e}");
    assert_eq!(st["stats"]["unreadable"].as_array().unwrap().len(), 1);
    let (_, page) = call(&base, "GET", "/api/catalog", &admin, None);
    let item = page["items"][0].clone();
    let id = item["id"].as_str().unwrap().to_string();
    assert_eq!(item["root"], "media");
    assert_eq!(item["rel_path"], "anime/Frieren/[Fansub-A] Frieren - 01 [1080p].mkv");
    assert!(item["file_path"].as_str().unwrap().ends_with("01 [1080p].mkv"));
    // Stored without a machine path; the id is the location's.
    drop(server);
    // Windows releases the store's file lock a moment after the server's
    // threads exit; retry instead of racing it.
    let doc = {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            match lain_server::export(&data) {
                Ok(doc) => break doc,
                Err(e) if std::time::Instant::now() < deadline && e.contains("already open") => {
                    std::thread::sleep(std::time::Duration::from_millis(100))
                }
                Err(e) => panic!("export: {e}"),
            }
        }
    };
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
    // The admin has not unlocked it: it is told something is unreadable and
    // not which library, nor where (D-293).
    let row = &st["stats"]["unreadable"][0];
    assert_eq!((row["name"].as_str(), row["path"].as_str()), (Some(""), Some("")), "{row}");
    assert_eq!(row["library_id"], gone_id);
    let (_, admin_view) = call(&base, "GET", "/api/library/scan", &admin, None);
    assert_eq!(admin_view["stats"]["unreadable"][0]["path"], "", "{admin_view}");
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
    let id = page["items"][0]["id"].as_str().unwrap().to_string();
    std::fs::remove_file(&file).unwrap();
    // Its only file is gone: the title is off the list (D-301), the row stays.
    wait(&|p| p["total"] == 0);
    let (_, row) = call(&base, "GET", &format!("/api/catalog/{id}"), &admin, None);
    assert_eq!(row["missing"], true, "{row}");
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

/// The lists and social suites on fresh servers, the OAuth flows
/// against the upstream fixture server, as `run.sh` runs them.
#[test]
fn lists_and_social_suites_pass() {
    let upstream = lain_conformance::upstream::Upstream::start(&lain_conformance::upstream::dir(), 0).unwrap();
    for suite in ["lists", "social"] {
        let dir = lain_store::test_dir(&format!("rs-{suite}"));
        let server = Server::start_with(&dir, |c| {
            c.offline = false;
            c.list_sync = false;
            c.anilist_endpoints = lain_lists::anilist::Endpoints {
                graphql: format!("{}/anilist-list/graphql", upstream.url),
                oauth: format!("{}/anilist-list/oauth", upstream.url),
            };
        });
        let mut runner = Runner::new(Base::parse(&server.url).unwrap());
        runner.vars.insert("upstream".into(), json!(upstream.url));
        run(&mut runner, suite);
        drop(server);
        std::fs::remove_dir_all(dir).ok();
    }
    assert!(upstream.hits("/anilist-list/") > 0, "the OAuth cases reached the fixture");
}

#[path = "../../lain-lists/tests/support/fixture.rs"]
mod fixture;

fn next_event(stream: &http::SseStream, name: &str) -> serde_json::Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if let Ok(ev) = stream.events.recv_timeout(Duration::from_millis(200))
            && ev.event == name
        {
            return serde_json::from_str(&ev.data).unwrap();
        }
    }
    panic!("no {name} event");
}

/// The list joined with the library (D-288, D-289): an entry names the
/// title it is, the title page reads its entry from the stored copy and
/// writes edits back, and a private library's titles stay out of it.
#[test]
fn tracking_joins_the_list_with_library_titles() {
    let root = lain_store::test_dir("rs-tracking");
    let media = root.join("media");
    std::fs::create_dir_all(media.join("Frieren")).unwrap();
    let data = root.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let roots = json!({"format": "lain.roots@1", "roots": {"media": media.to_string_lossy()}});
    std::fs::write(data.join("roots.json"), serde_json::to_vec(&roots).unwrap()).unwrap();
    let raw = std::fs::read(fixtures().join("catalog.json")).unwrap();
    lain_server::import(&data, &serde_json::from_slice(&raw).unwrap()).unwrap();
    let remote = std::sync::Arc::new(std::sync::Mutex::new(fixture::AniListState {
        anime: json!([{"name": "Watching", "isCustomList": false, "entries": [
            fixture::al_entry(154587, "ANIME", "Frieren", "CURRENT", 1, 28),
            fixture::al_entry(21, "ANIME", "Not Here", "PLANNING", 0, 12)
        ]}]),
        manga: json!([]),
        token_ok: true,
    }));
    let remote_srv = fixture::Fixture::start(fixture::anilist(remote.clone()));
    let server = Server::start_with(&data, |c| {
        c.offline = false;
        c.list_sync = false;
        c.anilist_endpoints = lain_lists::anilist::Endpoints {
            graphql: format!("{}/graphql", remote_srv.url),
            oauth: format!("{}/oauth", remote_srv.url),
        };
    });
    let base = Base::parse(&server.url).unwrap();
    call(&base, "POST", "/api/setup", "", Some(json!({"username": "admin", "password": "password123"})));
    let admin = login(&base, "admin", "password123");
    let (st, lib) = call(
        &base,
        "POST",
        "/api/libraries",
        &admin,
        Some(json!({"name": "Anime", "root": "media", "path": "Frieren"})),
    );
    assert_eq!(st, 201, "{lib}");
    let lib_id = lib["id"].as_str().unwrap().to_string();

    // Nothing linked: no tracks.
    let (st, v) = call(&base, "GET", "/api/items/item-frieren-02/tracking", &admin, None);
    assert_eq!((st, v), (200, json!({"tracks": []})));
    let (st, v) = call(&base, "POST", "/api/me/links/anilist/code", &admin, Some(json!({"code": "good"})));
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["link"]["scrobble"], true, "a new link updates AniList");

    // The list names the library title it is (by name: no metadata yet).
    let (_, v) = call(&base, "GET", "/api/list", &admin, None);
    let by_id = |v: &serde_json::Value, rid: &str| {
        v["entries"].as_array().unwrap().iter().find(|e| e["remote_id"] == rid).cloned().unwrap()
    };
    let frieren = by_id(&v, "154587");
    assert_eq!((frieren["item_id"].as_str(), frieren["matched_by"].as_str()), (Some("item-frieren-01"), Some("title")));
    assert!(by_id(&v, "21").get("item_id").is_none(), "an entry the library lacks has no title");

    // A title page, from any of its files, reads the entry.
    let (st, v) = call(&base, "GET", "/api/items/item-frieren-02/tracking", &admin, None);
    assert_eq!(st, 200, "{v}");
    let t = &v["tracks"][0];
    assert_eq!(
        (t["platform"].as_str(), t["remote_id"].as_str(), t["scrobble"].as_bool()),
        (Some("anilist"), Some("154587"), Some(true))
    );
    assert_eq!((t["entry"]["progress"].as_i64(), t["entry"]["item_id"].as_str()), (Some(1), Some("item-frieren-01")));
    let before = remote_srv.taken().len();
    assert!(before > 0);
    // ... from the stored copy: the page never waits on AniList.
    call(&base, "GET", "/api/items/item-frieren-01/tracking", &admin, None);
    call(&base, "GET", "/api/list", &admin, None);
    assert!(remote_srv.taken().is_empty(), "a read reached AniList");

    // Quick actions write back.
    let (st, v) = call(&base, "PUT", "/api/list/anilist/154587", &admin, Some(json!({"score": 8, "status": "paused"})));
    assert_eq!(st, 200, "{v}");
    assert_eq!((v["entry"]["score"].as_f64(), v["entry"]["status"].as_str()), (Some(8.0), Some("paused")));
    assert_eq!(v["entry"]["item_id"], "item-frieren-01", "the answer is joined too");
    let sent = remote_srv.taken();
    assert_eq!(sent[0].json()["variables"], json!({"mediaId": 154587, "status": "PAUSED", "scoreRaw": 80}));
    let (_, links) = call(&base, "GET", "/api/me/links", &admin, None);
    assert_eq!(links["links"][0]["last_push"]["via"], "edit");

    // A private library's titles never meet the outside list.
    let (st, _) = call(&base, "PATCH", &format!("/api/libraries/{lib_id}"), &admin, Some(json!({"private": true})));
    assert_eq!(st, 200);
    call(&base, "POST", &format!("/api/libraries/{lib_id}/unlock"), &admin, Some(json!({"password": "password123"})));
    let (_, v) = call(&base, "GET", "/api/list", &admin, None);
    assert!(by_id(&v, "154587").get("item_id").is_none(), "{v}");
    let (st, v) = call(&base, "GET", "/api/items/item-frieren-02/tracking", &admin, None);
    assert_eq!((st, v), (200, json!({"tracks": []})));
    let (st, v) =
        call(&base, "PUT", "/api/list/anilist/154587", &admin, Some(json!({"score": 8, "item_id": "item-frieren-01"})));
    assert_eq!((st, v["code"].as_str()), (403, Some("private-library")), "{v}");
    assert!(remote_srv.taken().is_empty());
    drop(server);
    std::fs::remove_dir_all(root).ok();
}

/// A finished episode scrobbles to an opted-in account (and announces
/// `list-changed`), but an item of a private library never leaves the
/// server (D-156); progress feeds the social activity log either way,
/// and friend requests announce `social-notification`.
#[test]
fn scrobbles_skip_private_libraries_and_events_flow() {
    let root = lain_store::test_dir("rs-scrobble");
    let media = root.join("media");
    std::fs::create_dir_all(media.join("Frieren")).unwrap();
    let data = root.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let roots = json!({"format": "lain.roots@1", "roots": {"media": media.to_string_lossy()}});
    std::fs::write(data.join("roots.json"), serde_json::to_vec(&roots).unwrap()).unwrap();
    let raw = std::fs::read(fixtures().join("catalog.json")).unwrap();
    lain_server::import(&data, &serde_json::from_slice(&raw).unwrap()).unwrap();
    let remote = std::sync::Arc::new(std::sync::Mutex::new(fixture::AniListState {
        anime: json!([{"name": "Watching", "isCustomList": false, "entries": [fixture::al_entry(154587, "ANIME", "Frieren", "CURRENT", 0, 28)]}]),
        manga: json!([]),
        token_ok: true,
    }));
    let remote_srv = fixture::Fixture::start(fixture::anilist(remote));
    let server = Server::start_with(&data, |c| {
        c.offline = false;
        c.list_sync = false;
        c.anilist_endpoints = lain_lists::anilist::Endpoints {
            graphql: format!("{}/graphql", remote_srv.url),
            oauth: format!("{}/oauth", remote_srv.url),
        };
    });
    let base = Base::parse(&server.url).unwrap();
    assert_eq!(
        call(&base, "POST", "/api/setup", "", Some(json!({"username": "admin", "password": "password123"}))).0,
        201
    );
    let admin = login(&base, "admin", "password123");
    call(&base, "POST", "/api/users", &admin, Some(json!({"username": "bob", "password": "bobpassword"})));
    let bob = login(&base, "bob", "bobpassword");
    let (st, lib) = call(
        &base,
        "POST",
        "/api/libraries",
        &admin,
        Some(json!({"name": "Anime", "root": "media", "path": "Frieren"})),
    );
    assert_eq!(st, 201, "{lib}");
    let lib_id = lib["id"].as_str().unwrap().to_string();

    let stream = http::open_sse(&base, &format!("/api/events?token={admin}"), &[]).unwrap();
    assert_eq!(next_event(&stream, "ready")["server"], "lain-rs");
    let (st, v) = call(&base, "POST", "/api/me/links/anilist/code", &admin, Some(json!({"code": "good"})));
    assert_eq!(st, 200, "{v}");
    assert_eq!(next_event(&stream, "list-changed"), json!({"platform": "anilist", "action": "linked"}));
    assert_eq!(
        next_event(&stream, "list-changed"),
        json!({"platform": "anilist", "action": "synced", "upserted": 1, "removed": 0})
    );
    assert_eq!(call(&base, "PATCH", "/api/me/links/anilist", &admin, Some(json!({"scrobble": true}))).0, 200);
    assert_eq!(next_event(&stream, "list-changed")["action"], "updated");
    remote_srv.taken();

    // A public library's episode scrobbles.
    let done = json!({"position_sec": 1440, "duration_sec": 1440, "completed": true});
    assert_eq!(call(&base, "PUT", "/api/items/item-frieren-01/progress", &admin, Some(done.clone())).0, 200);
    let ev = next_event(&stream, "list-changed");
    assert_eq!((ev["action"].as_str(), ev["progress"].as_i64()), (Some("scrobbled"), Some(1)), "{ev}");
    let pushes: Vec<_> = remote_srv.taken().into_iter().filter(|r| r.body.contains("SaveMediaListEntry")).collect();
    assert_eq!(pushes.len(), 1);
    // Re-sending the final position is not a new finish.
    assert_eq!(call(&base, "PUT", "/api/items/item-frieren-01/progress", &admin, Some(done.clone())).0, 200);

    // A private library's episode never leaves the server.
    let (st, v) = call(&base, "PATCH", &format!("/api/libraries/{lib_id}"), &admin, Some(json!({"private": true})));
    assert_eq!(st, 200, "{v}");
    assert_eq!(call(&base, "PUT", "/api/items/item-frieren-02/progress", &admin, Some(done)).0, 200);
    std::thread::sleep(Duration::from_millis(500));
    assert!(remote_srv.taken().iter().all(|r| !r.body.contains("SaveMediaListEntry")), "a private item was scrobbled");

    // Progress fed the activity log; a locked session sees none of it.
    let (_, act) = call(&base, "GET", "/api/social/users/user-admin/activity", &admin, None);
    assert_eq!(act["items"], json!([]), "{act}");
    let (st, _) = call(
        &base,
        "POST",
        &format!("/api/libraries/{lib_id}/unlock"),
        &admin,
        Some(json!({"password": "password123"})),
    );
    assert_eq!(st, 200);
    let (_, act) = call(&base, "GET", "/api/social/users/user-admin/activity", &admin, None);
    let kinds: Vec<&str> = act["items"].as_array().unwrap().iter().map(|a| a["type"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["completed", "completed"], "{act}");

    // Friend requests are announced to the recipient only.
    assert_eq!(call(&base, "POST", "/api/social/friends/user-admin", &bob, None).0, 200);
    let ev = next_event(&stream, "social-notification");
    assert_eq!((ev["type"].as_str(), ev["from_user_id"].as_str()), (Some("friend_request"), Some("user-bob")));
    assert!(ev["id"].is_string());

    // Disabling the social module unmounts its routes; progress still
    // writes.
    std::fs::write(
        data.join("modules.json"),
        br#"{"format":"lain.modules@1","modules":[{"id":"social","enabled":false}]}"#,
    )
    .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let (st, v) = call(&base, "GET", "/api/social/feed", &admin, None);
        if st == 404 {
            assert_eq!(v["code"], "module-disabled");
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the social module never went down");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(
        call(&base, "PUT", "/api/items/item-frieren-01/progress", &admin, Some(json!({"position_sec": 5}))).0,
        200
    );
    drop(server);
    std::fs::remove_dir_all(root).ok();
}
