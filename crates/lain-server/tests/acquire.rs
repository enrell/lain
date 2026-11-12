//! The acquire module end to end in-process: the conformance suite,
//! a search → grab → import → rescan over a local tracker, a seeding
//! engine and a fake Torznab indexer (nothing leaves loopback), release
//! alternatives from a scripted parser service (D-279), and the privacy
//! gate on the acquire item routes (D-156).

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use lain_conformance::cases::{Outcome, Runner};
use lain_conformance::http::{self, Base};
use lain_server::app::{App, Config};
use lain_torrent::trackertest::Tracker;
use lain_torrent::{Client, SourceFile, Spec, build};

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
        cfg.watch = false;
        cfg.auto_enrich = false;
        cfg.theme_path = Some(dir.join("no-colors.toml"));
        cfg.acquire_listen_port = Some(0);
        cfg.acquire_tick = Duration::from_millis(100);
        // Loopback only: the metadata providers point at a closed port.
        cfg.metadata_endpoints = lain_metadata::Endpoints::under("http://127.0.0.1:9");
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

fn call(base: &Base, method: &str, path: &str, token: &str, body: Option<Value>) -> (u16, Value) {
    let mut headers = vec![("Authorization".to_string(), format!("Bearer {token}"))];
    let raw = body.map(|b| {
        headers.push(("Content-Type".into(), "application/json".into()));
        serde_json::to_vec(&b).unwrap()
    });
    let r = http::request(base, method, path, &headers, raw.as_deref()).unwrap();
    (r.status, serde_json::from_slice(&r.body).unwrap_or(Value::Null))
}

fn admin(base: &Base) -> String {
    let (st, _) = call(base, "POST", "/api/setup", "", Some(json!({"username": "admin", "password": "password123"})));
    assert_eq!(st, 201);
    let (_, v) =
        call(base, "POST", "/api/auth/login", "", Some(json!({"username": "admin", "password": "password123"})));
    v["token"].as_str().unwrap().to_string()
}

fn wait(what: &str, mut f: impl FnMut() -> Option<Value>) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(Instant::now() < deadline, "{what} never happened");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn acquire_suite_passes() {
    let dir = lain_store::test_dir("rs-acquire");
    let media = dir.join("media");
    std::fs::create_dir_all(&media).unwrap();
    let server = Server::start(&dir.join("data"));
    let mut runner = Runner::new(Base::parse(&server.url).unwrap());
    runner.vars.insert("acq_media".into(), json!(media.to_string_lossy()));
    let reports = runner
        .run_suite(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/cases/acquire"),
            &["v1".into(), "v2".into()],
        )
        .unwrap();
    assert!(!reports.is_empty());
    let failed: Vec<String> = reports
        .iter()
        .filter_map(|r| match &r.outcome {
            Outcome::Pass => None,
            o => Some(format!("{} :: {}: {o:?}", r.file, r.name)),
        })
        .collect();
    assert!(failed.is_empty(), "{}", failed.join("\n"));
    drop(server);
    std::fs::remove_dir_all(dir).ok();
}

fn payload(n: usize, seed: u8) -> Vec<u8> {
    let mut x = (seed as u32).wrapping_mul(2246822519).wrapping_add(7);
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

/// A Torznab endpoint and the `.torrent` it links, on loopback.
fn indexer(raw: Vec<u8>, name: String, len: i64) -> String {
    let ln = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", ln.local_addr().unwrap());
    let b2 = base.clone();
    std::thread::spawn(move || {
        for conn in ln.incoming().flatten() {
            let mut conn = conn;
            let mut r = BufReader::new(conn.try_clone().unwrap());
            let mut line = String::new();
            let _ = r.read_line(&mut line);
            loop {
                let mut h = String::new();
                if r.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                    break;
                }
            }
            let target = line.split_whitespace().nth(1).unwrap_or("").to_string();
            let body = if target.starts_with("/dl/show.torrent") {
                raw.clone()
            } else if target.contains("t=caps") {
                br#"<caps><searching><search available="yes"/><tv-search available="yes"/></searching></caps>"#.to_vec()
            } else {
                format!(
                    r#"<rss><channel><item><title>{name}</title><link>{b2}/dl/show.torrent</link><size>{len}</size><torznab:attr name="seeders" value="1"/></item></channel></rss>"#
                )
                .into_bytes()
            };
            let _ = write!(conn, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = conn.write_all(&body);
        }
    });
    base
}

/// The whole path through the HTTP API: an indexer, a search, a grab,
/// the download from a local seeder, the import into the library under
/// lain's names, the rescan that catalogs it, and events on the way.
#[test]
fn search_grab_import_rescan() {
    let dir = lain_store::test_dir("rs-acquire-e2e");
    let media = dir.join("media/Anime");
    std::fs::create_dir_all(&media).unwrap();
    let tracker = Tracker::new().unwrap();
    let files = vec![
        SourceFile { path: vec!["[Fansub-A] Frieren - 01 [1080p].mkv".into()], data: payload(70_000, 1) },
        SourceFile { path: vec!["[Fansub-A] Frieren - 02 [1080p].mkv".into()], data: payload(50_000, 2) },
    ];
    let (raw, mi) = build("[Fansub-A] Frieren - 01-02 [1080p]", &files, 16 << 10, &[tracker.http_url()]).unwrap();
    let seed_dir = dir.join("seed");
    for f in &files {
        let p = seed_dir.join(&mi.info.name).join(&f.path[0]);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, &f.data).unwrap();
    }
    let seed = Client::new(lain_torrent::Config {
        listen_addr: Some("127.0.0.1:0".into()),
        min_announce_interval: Duration::from_secs(1),
        ..Default::default()
    })
    .unwrap();
    seed.add(Spec { meta_info: Some(mi.clone()), dir: seed_dir, ..Spec::default() }).unwrap();
    let index = indexer(raw, mi.info.name.clone(), mi.info.length);

    let server = Server::start(&dir.join("data"));
    let base = Base::parse(&server.url).unwrap();
    let tok = admin(&base);
    let stream = http::open_sse(&base, &format!("/api/events?token={tok}"), &[]).unwrap();
    assert_eq!(stream.events.recv_timeout(Duration::from_secs(5)).unwrap().event, "ready");
    let (st, lib) = call(
        &base,
        "POST",
        "/api/libraries",
        &tok,
        Some(json!({"name": "Anime", "type": "anime", "path": media.to_string_lossy()})),
    );
    assert_eq!(st, 201, "{lib}");
    let (st, _) = call(&base, "PUT", "/api/acquire/settings", &tok, Some(json!({"seed_ratio": 0, "seed_minutes": 0})));
    assert_eq!(st, 200);
    let (st, ix) = call(
        &base,
        "POST",
        "/api/acquire/indexers",
        &tok,
        Some(
            json!({"name": "tracker-exemplo", "protocol": "torznab", "url": index, "api_key": "k", "min_interval_ms": 0}),
        ),
    );
    assert_eq!(st, 201, "{ix}");
    let (st, res) = call(&base, "GET", "/api/acquire/search?q=Frieren&kind=anime", &tok, None);
    assert_eq!(st, 200, "{res}");
    let best = res["candidates"][0].clone();
    assert_eq!(best["release"]["title"], "Frieren", "{res}");
    assert!(best.get("rejections").is_none(), "{best}");
    let (st, g) =
        call(&base, "POST", "/api/acquire/grabs", &tok, Some(json!({"result": best, "library_id": lib["id"]})));
    assert_eq!(st, 201, "{g}");
    assert_eq!(g["info_hash"], mi.info_hash.hex());
    let id = g["id"].as_str().unwrap().to_string();
    let done = wait("the import", || {
        let (_, g) = call(&base, "GET", "/api/acquire/grabs", &tok, None);
        let g = g["grabs"][0].clone();
        assert_ne!(g["state"], "failed", "{g}");
        (g["state"] == "done").then_some(g)
    });
    assert_eq!(done["imported"].as_array().unwrap().len(), 2, "{done}");
    assert_eq!(done["data_removed"], true);
    for (i, n) in ["[Fansub-A] Frieren - 01.mkv", "[Fansub-A] Frieren - 02.mkv"].iter().enumerate() {
        assert!(std::fs::read(media.join("Frieren").join(n)).unwrap() == files[i].data, "{n}");
    }
    // The rescan catalogs the imported episodes.
    let items = wait("the rescan", || {
        let (_, v) = call(&base, "GET", "/api/catalog", &tok, None);
        (v["total"] == 2).then_some(v)
    });
    assert_eq!(items["items"][0]["title"], "Frieren");
    // Events: the grab's states, to the admin.
    let mut states = Vec::new();
    while let Ok(ev) = stream.events.recv_timeout(Duration::from_millis(200)) {
        if ev.event == "grab-changed" {
            let v: Value = serde_json::from_str(&ev.data).unwrap();
            assert_eq!(v["id"], id.as_str());
            states.push(v["state"].as_str().unwrap_or("").to_string());
        }
    }
    assert!(
        states.contains(&"importing".to_string()) && states.last().map(String::as_str) == Some("done"),
        "{states:?}"
    );
    drop(server);
    seed.close();
    std::fs::remove_dir_all(dir).ok();
}

/// D-279 end to end on the Rust side: the lain-parser bridge, registered
/// where its component would be, against a scripted service 1.3.0. The
/// parse route answers the alternatives as the provider gave them; a
/// search settles them against its query and returns none; a grab made
/// by hand keeps the answer and stores none.
#[test]
fn release_alternatives_through_the_api() {
    use lain_bridge::fake::Fake;
    use lain_bridge::http1::Endpoint;
    use lain_bridge::parser::{MODEL_ID, ReleaseModel};

    let dir = lain_store::test_dir("rs-acquire-alts");
    let media = dir.join("media/Anime");
    std::fs::create_dir_all(&media).unwrap();
    // "Quiet Harbor 2 - 03": episode 3 of a sequel, or of season 2.
    const NAME: &str = "[Fansub-A] Quiet Harbor 2 - 03 [1080p]";
    let files = vec![SourceFile { path: vec![format!("{NAME}.mkv")], data: payload(20_000, 4) }];
    let (raw, mi) = build(NAME, &files, 16 << 10, &["http://127.0.0.1:9/announce".to_string()]).unwrap();
    let index = indexer(raw, NAME.to_string(), mi.info.length);
    let service = Fake::tcp(Arc::new(|r| match (r.method.as_str(), r.path.as_str()) {
        ("GET", "/health") => (200, json!({"status": "ok", "artifact": "v6-s11"})),
        ("POST", "/parse") if r.body.as_ref().is_some_and(|b| b["filename"].as_str() == Some(NAME)) => (
            200,
            json!({"record": {"title": "Quiet Harbor 2", "episodes": [3], "release_group": "Fansub-A"},
                "alternatives": [{"title": "Quiet Harbor", "season": 2, "episodes": [3]}, {"title": "Quiet"}]}),
        ),
        _ => (200, json!({"record": null})),
    }));

    let server = Server::start(&dir.join("data"));
    server.app.registry.register(Arc::new(ReleaseModel::new(Endpoint::parse(&service.url).unwrap())));
    let base = Base::parse(&server.url).unwrap();
    let tok = admin(&base);
    let (_, lib) = call(
        &base,
        "POST",
        "/api/libraries",
        &tok,
        Some(json!({"name": "Anime", "type": "anime", "path": media.to_string_lossy()})),
    );

    let (st, r) = call(
        &base,
        "GET",
        "/api/acquire/parse?name=%5BFansub-A%5D%20Quiet%20Harbor%202%20-%2003%20%5B1080p%5D&kind=anime",
        &tok,
        None,
    );
    assert_eq!(st, 200, "{r}");
    assert_eq!((&r["title"], &r["parser"]), (&json!("Quiet Harbor 2"), &json!(MODEL_ID)), "{r}");
    assert_eq!(
        r["alternatives"],
        json!([{"title": "Quiet Harbor", "season": 2, "episodes": [3]}, {"title": "Quiet"}]),
        "as the provider gave them"
    );

    let (st, ix) = call(
        &base,
        "POST",
        "/api/acquire/indexers",
        &tok,
        Some(
            json!({"name": "tracker-exemplo", "protocol": "torznab", "url": index, "api_key": "k", "min_interval_ms": 0}),
        ),
    );
    assert_eq!(st, 201, "{ix}");
    let (st, res) = call(&base, "GET", "/api/acquire/search?q=Quiet%20Harbor&kind=anime", &tok, None);
    assert_eq!(st, 200, "{res}");
    let best = res["candidates"][0].clone();
    let rel = &best["release"];
    assert_eq!((&rel["title"], &rel["season"], &rel["episodes"]), (&json!("Quiet Harbor"), &json!(2), &json!([3])));
    assert!(rel.get("alternatives").is_none() && best.get("rejections").is_none(), "{best}");
    let (_, res) = call(&base, "GET", "/api/acquire/search?q=Quiet%20Harbor%202&kind=anime", &tok, None);
    let rel = &res["candidates"][0]["release"];
    assert_eq!((&rel["title"], rel.get("season")), (&json!("Quiet Harbor 2"), None), "{res}");
    assert!(rel.get("alternatives").is_none(), "{res}");

    let (st, g) =
        call(&base, "POST", "/api/acquire/grabs", &tok, Some(json!({"result": best, "library_id": lib["id"]})));
    assert_eq!(st, 201, "{g}");
    assert_eq!(g["release"]["title"], "Quiet Harbor 2", "a grab by hand names no title: {g}");
    assert!(g["release"].get("alternatives").is_none(), "{g}");
    let (_, list) = call(&base, "GET", "/api/acquire/grabs", &tok, None);
    assert!(list["grabs"][0]["release"].get("alternatives").is_none(), "{list}");
    drop(server);
    drop(service);
    std::fs::remove_dir_all(dir).ok();
}

/// D-156: the acquire item routes answer 404 for an item of a locked
/// private library, admin included, as for one that does not exist.
#[test]
fn private_items_are_unknown_to_acquire_routes() {
    let dir = lain_store::test_dir("rs-acquire-private");
    let media: PathBuf = dir.join("media/Anime");
    std::fs::create_dir_all(media.join("Frieren")).unwrap();
    std::fs::write(media.join("Frieren/[Fansub-A] Frieren - 01 [1080p].mkv"), b"x").unwrap();
    let server = Server::start(&dir.join("data"));
    let base = Base::parse(&server.url).unwrap();
    let tok = admin(&base);
    let (_, lib) = call(
        &base,
        "POST",
        "/api/libraries",
        &tok,
        Some(json!({"name": "Anime", "type": "anime", "path": media.to_string_lossy()})),
    );
    let lib_id = lib["id"].as_str().unwrap().to_string();
    let (st, _) = call(&base, "POST", "/api/library/scan", &tok, None);
    assert_eq!(st, 202);
    let items = wait("the scan", || {
        let (_, v) = call(&base, "GET", "/api/catalog", &tok, None);
        (v["total"] == 1).then_some(v)
    });
    let id = items["items"][0]["id"].as_str().unwrap().to_string();
    let paths = [
        ("GET", format!("/api/acquire/items/{id}/subtitles/status")),
        ("GET", format!("/api/acquire/items/{id}/subtitles?languages=en")),
        ("POST", format!("/api/acquire/items/{id}/subtitles")),
        ("DELETE", format!("/api/acquire/items/{id}/sidecars/x.srt")),
    ];
    let (st, v) = call(&base, "GET", &paths[0].1, &tok, None);
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["file"]["item_id"], id.as_str());
    let (st, _) = call(&base, "PATCH", &format!("/api/libraries/{lib_id}"), &tok, Some(json!({"private": true})));
    assert_eq!(st, 200);
    let (_, fresh) =
        call(&base, "POST", "/api/auth/login", "", Some(json!({"username": "admin", "password": "password123"})));
    let locked = fresh["token"].as_str().unwrap().to_string();
    for (m, p) in &paths {
        let (st, v) = call(&base, m, p, &locked, Some(json!({})));
        assert_eq!((st, v["error"].as_str()), (404, Some("unknown item")), "{m} {p}");
    }
    let (st, _) = call(
        &base,
        "POST",
        &format!("/api/libraries/{lib_id}/unlock"),
        &locked,
        Some(json!({"password": "password123"})),
    );
    assert_eq!(st, 200);
    let (st, _) = call(&base, "GET", &paths[0].1, &locked, None);
    assert_eq!(st, 200, "unlocked in this session");
    drop(server);
    std::fs::remove_dir_all(dir).ok();
}
