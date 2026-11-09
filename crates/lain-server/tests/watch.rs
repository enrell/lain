//! The watch module end to end on an in-process lain-rs: the
//! conformance watch suite over real clips (needs ffmpeg), byte serving
//! semantics, local playback with a stub player, and the events.
//!
//! Tests that run the host ffmpeg skip without it, unless
//! `LAIN_REQUIRE_FFMPEG=1` (CI) makes that a failure (Go's ffmpeg gate).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

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
        // The fixture catalog is the library: no scan, no network.
        cfg.watch = false;
        cfg.auto_enrich = false;
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

    fn base(&self) -> Base {
        Base::parse(&self.url).unwrap()
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

fn require_ffmpeg() -> bool {
    let ok = lain_probe::exec::look_path("ffmpeg").is_some() && lain_probe::exec::look_path("ffprobe").is_some();
    if !ok {
        assert!(
            std::env::var("LAIN_REQUIRE_FFMPEG").is_err(),
            "LAIN_REQUIRE_FFMPEG is set but ffmpeg/ffprobe are not on PATH"
        );
        eprintln!("skipping: ffmpeg/ffprobe not installed");
    }
    ok
}

fn ffmpeg(args: &[&str]) {
    let out = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// The clips of `conformance/make-clips.sh`, made the same way.
fn make_clips(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    let srt = dir.join("subs.srt");
    std::fs::write(&srt, "1\n00:00:00,000 --> 00:00:01,500\nHello\n").unwrap();
    let p = |n: &str| dir.join(n).to_string_lossy().into_owned();
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc=size=320x240:rate=15:duration=2",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:sample_rate=48000:duration=2",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-b:v",
        "2000k",
        "-c:a",
        "aac",
        "-b:a",
        "128k",
        "-movflags",
        "+faststart",
        "-y",
        &p("Alpha.mp4"),
    ]);
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc=size=160x120:rate=10:duration=2",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=330:sample_rate=48000:duration=2",
        "-i",
        &srt.to_string_lossy(),
        "-map",
        "0:v",
        "-map",
        "1:a",
        "-map",
        "2:s",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-b:a",
        "96k",
        "-c:s",
        "srt",
        "-y",
        &p("Beta.mkv"),
    ]);
    std::fs::remove_file(srt).unwrap();
}

/// A data dir with a `media` root over `root/media` and the watch
/// fixture catalog (items `item-alpha`, `item-beta` in Clips/).
fn data_dir(root: &Path) -> (PathBuf, PathBuf) {
    let media = root.join("media");
    std::fs::create_dir_all(media.join("Clips")).unwrap();
    let data = root.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let roots = json!({"format": "lain.roots@1", "roots": {"media": media.to_string_lossy()}});
    std::fs::write(data.join("roots.json"), serde_json::to_vec(&roots).unwrap()).unwrap();
    let raw =
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/fixtures/watch-catalog.json"))
            .unwrap();
    assert_eq!(lain_server::import(&data, &serde_json::from_slice(&raw).unwrap()).unwrap(), 2);
    (data, media)
}

fn post(base: &Base, path: &str, token: Option<&str>, body: Value) -> http::Response {
    let mut h = vec![("Content-Type".to_string(), "application/json".to_string())];
    if let Some(t) = token {
        h.push(("Authorization".into(), format!("Bearer {t}")));
    }
    http::request(base, "POST", path, &h, Some(&serde_json::to_vec(&body).unwrap())).unwrap()
}

fn get(base: &Base, path: &str, headers: &[(&str, &str)]) -> http::Response {
    let h: Vec<(String, String)> = headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    http::request(base, "GET", path, &h, None).unwrap()
}

fn json_of(r: &http::Response) -> Value {
    serde_json::from_slice(&r.body).unwrap_or(Value::Null)
}

/// Admin and bob on a fresh server, plus the Clips library.
fn accounts(s: &Server) -> (String, String) {
    let b = s.base();
    assert_eq!(post(&b, "/api/setup", None, json!({"username": "admin", "password": "password123"})).status, 201);
    let admin =
        json_of(&post(&b, "/api/auth/login", None, json!({"username": "admin", "password": "password123"})))["token"]
            .as_str()
            .unwrap()
            .to_string();
    assert_eq!(post(&b, "/api/users", Some(&admin), json!({"username": "bob", "password": "bobpassword"})).status, 201);
    let bob =
        json_of(&post(&b, "/api/auth/login", None, json!({"username": "bob", "password": "bobpassword"})))["token"]
            .as_str()
            .unwrap()
            .to_string();
    let lib = post(
        &b,
        "/api/libraries",
        Some(&admin),
        json!({"name": "Clips", "type": "movie", "root": "media", "path": "Clips"}),
    );
    assert_eq!(lib.status, 201, "{}", String::from_utf8_lossy(&lib.body));
    (admin, bob)
}

#[test]
fn watch_suite_passes() {
    if !require_ffmpeg() {
        return;
    }
    // Server logs (ffmpeg's error text) show with a failure.
    let _ = tracing_subscriber::fmt().with_test_writer().with_ansi(false).try_init();
    let root = lain_store::test_dir("rs-watch");
    let (data, media) = data_dir(&root);
    make_clips(&media.join("Clips"));
    let server = Server::start(&data);
    let mut runner = Runner::new(server.base());
    runner.vars.insert("clips".into(), json!("1"));
    runner.vars.insert("media_root".into(), json!(media.to_string_lossy()));
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/cases/watch");
    let reports = runner.run_suite(&dir, &["v1".into(), "v2".into()]).unwrap();
    let failed: Vec<String> = reports
        .iter()
        .filter_map(|r| match &r.outcome {
            Outcome::Fail(e) => Some(format!("{} :: {}: {e}", r.file, r.name)),
            _ => None,
        })
        .collect();
    assert!(failed.is_empty(), "watch suite:\n{}", failed.join("\n"));
    assert!(reports.iter().filter(|r| r.outcome == Outcome::Pass).count() >= 30);

    // A session's terminal state reaches its owner's event stream.
    let bob = runner.vars["w_bob"].as_str().unwrap().to_string();
    let stream = http::open_sse(&server.base(), &format!("/api/events?token={bob}"), &[]).unwrap();
    assert_eq!(stream.events.recv_timeout(Duration::from_secs(5)).unwrap().event, "ready");
    let r = post(
        &server.base(),
        "/api/items/item-beta/transcode",
        Some(&bob),
        json!({"delivery": "progressive", "quality": "360p"}),
    );
    assert!(r.status == 200 || r.status == 202, "{}", String::from_utf8_lossy(&r.body));
    let session = json_of(&r)["session"].as_str().unwrap().to_string();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let ev = stream.events.recv_timeout(Duration::from_secs(30)).expect("transcode-changed");
        if ev.event == "transcode-changed" {
            let d: Value = serde_json::from_str(&ev.data).unwrap();
            assert_eq!(
                (d["session"].as_str(), d["state"].as_str(), d["item_id"].as_str()),
                (Some(session.as_str()), Some("ready"), Some("item-beta"))
            );
            break;
        }
        assert!(Instant::now() < deadline);
    }
    drop(server);
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn bytes_are_served_like_go() {
    let root = lain_store::test_dir("rs-watch-bytes");
    let (data, media) = data_dir(&root);
    let bytes: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(media.join("Clips/Alpha.mp4"), &bytes).unwrap();
    let server = Server::start(&data);
    let (_admin, bob) = accounts(&server);
    let b = server.base();
    let path = format!("/api/items/item-alpha/stream?token={bob}");
    let full = get(&b, &path, &[]);
    assert_eq!((full.status, full.body.len()), (200, bytes.len()));
    assert_eq!(full.body, bytes);
    assert_eq!(full.header("content-type"), Some("video/mp4"));
    let lm = full.header("last-modified").unwrap().to_string();
    // Ranges.
    let r = get(&b, &path, &[("Range", "bytes=100-199")]);
    assert_eq!((r.status, r.header("content-range")), (206, Some("bytes 100-199/10000")));
    assert_eq!(r.body, bytes[100..200]);
    let r = get(&b, &path, &[("Range", "bytes=9990-")]);
    assert_eq!(r.body, bytes[9990..]);
    let r = get(&b, &path, &[("Range", "bytes=10000-")]);
    assert_eq!((r.status, r.header("content-range")), (416, Some("bytes */10000")));
    let r = get(&b, &path, &[("Range", "bytes=0-1,5-6")]);
    assert_eq!(r.status, 206);
    let ct = r.header("content-type").unwrap().to_string();
    let boundary = ct.strip_prefix("multipart/byteranges; boundary=").unwrap();
    let body = String::from_utf8_lossy(&r.body).into_owned();
    assert!(
        body.starts_with(&format!("--{boundary}\r\nContent-Range: bytes 0-1/10000\r\nContent-Type: video/mp4\r\n\r\n")),
        "{body:?}"
    );
    assert!(body.ends_with(&format!("\r\n--{boundary}--\r\n")));
    assert_eq!(r.header("content-length").unwrap().parse::<usize>().unwrap(), r.body.len());
    // Conditional requests: If-Range by date, If-Modified-Since.
    let r = get(&b, &path, &[("Range", "bytes=0-9"), ("If-Range", &lm)]);
    assert_eq!(r.status, 206);
    let r = get(&b, &path, &[("Range", "bytes=0-9"), ("If-Range", "Thu, 01 Jan 1970 00:00:00 GMT")]);
    assert_eq!((r.status, r.body.len()), (200, bytes.len()), "a stale If-Range gets the whole file");
    let r = get(&b, &path, &[("Range", "bytes=0-9"), ("If-Range", "\"some-etag\"")]);
    assert_eq!(r.status, 200);
    let r = get(&b, &path, &[("If-Modified-Since", &lm)]);
    assert_eq!((r.status, r.body.len()), (304, 0));
    let r = get(&b, &path, &[("If-None-Match", "*")]);
    assert_eq!(r.status, 304);
    let r = get(&b, &path, &[("If-Match", "\"x\"")]);
    assert_eq!(r.status, 412);
    // HEAD carries the length and no body.
    let h = http::request(&b, "HEAD", &path, &[], None).unwrap();
    assert_eq!((h.status, h.header("content-length")), (200, Some("10000")));
    // A row whose file vanished plans as unavailable; its stream 404s.
    let r = get(&b, "/api/items/item-beta/playback?client=web", &[("Authorization", &format!("Bearer {bob}"))]);
    assert_eq!(
        json_of(&r),
        json!({"mode": "unavailable", "asset": "", "available": false, "reason": "the file is no longer on disk"})
    );
    let r = get(&b, &format!("/api/items/item-beta/stream?token={bob}"), &[]);
    assert_eq!((r.status, json_of(&r)), (404, json!({"error": "file unavailable"})));
    drop(server);
    std::fs::remove_dir_all(root).ok();
}

#[cfg(unix)]
#[test]
fn local_play_reports_progress_to_the_user() {
    use std::os::unix::fs::PermissionsExt;
    let root = lain_store::test_dir("rs-watch-local");
    let (data, media) = data_dir(&root);
    std::fs::write(media.join("Clips/Alpha.mp4"), b"video").unwrap();
    let server = Server::start(&data);
    let (_admin, bob) = accounts(&server);
    let bin = root.join("mpv");
    std::fs::write(
        &bin,
        "#!/bin/sh\nstate=\"\"\nfor arg in \"$@\"; do\n  case \"$arg\" in\n    --script-opt=lain-state_dir=*) state=\"${arg#--script-opt=lain-state_dir=}\" ;;\n  esac\ndone\nprintf '{\"position_sec\":42,\"duration_sec\":120}' > \"$state/0.json\"\nsleep 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let m = &server.app.media.local.manager;
    let b2 = bin.clone();
    m.set_look_path(move |n| (n == "mpv").then(|| b2.clone()));
    m.set_getenv(|k| if k == "DISPLAY" { ":0".into() } else { String::new() });
    let b = server.base();
    let auth = format!("Bearer {bob}");
    assert_eq!(json_of(&get(&b, "/api/localplay", &[("Authorization", &auth)])), json!({"players": ["mpv"]}));
    let stream = http::open_sse(&b, &format!("/api/events?token={bob}"), &[]).unwrap();
    assert_eq!(stream.events.recv_timeout(Duration::from_secs(5)).unwrap().event, "ready");
    let r = post(&b, "/api/items/item-alpha/play-local", Some(&bob), json!({"player": "vlc"}));
    assert_eq!((r.status, json_of(&r)), (400, json!({"error": "invalid-message: player must be one of: mpv"})));
    let r = post(&b, "/api/items/item-alpha/play-local", Some(&bob), json!({}));
    assert_eq!((r.status, json_of(&r)), (200, json!({"status": "playing", "player": "mpv", "count": 1})));
    let r = post(&b, "/api/items/item-alpha/play-local", Some(&bob), json!({}));
    assert_eq!(r.status, 409, "one screen, one player");
    let ev = loop {
        let ev = stream.events.recv_timeout(Duration::from_secs(10)).expect("progress-changed");
        if ev.event == "progress-changed" {
            break ev;
        }
    };
    let d: Value = serde_json::from_str(&ev.data).unwrap();
    assert_eq!((d["item_id"].as_str(), d["progress"]["position_sec"].as_f64()), (Some("item-alpha"), Some(42.0)));
    let p = json_of(&get(&b, "/api/items/item-alpha/progress", &[("Authorization", &auth)]));
    assert_eq!(p["position_sec"], 42.0);
    // A vanished file is a conflict, not a crash (D-068).
    let r = post(&b, "/api/items/item-beta/play-local", Some(&bob), json!({}));
    assert_eq!((r.status, json_of(&r)), (409, json!({"error": "not-found: the file is no longer on disk"})));
    // No graphical session: 503.
    m.set_getenv(|_| String::new());
    let r = post(&b, "/api/items/item-alpha/play-local", Some(&bob), json!({}));
    assert_eq!(r.status, 503);
    drop(server);
    std::fs::remove_dir_all(root).ok();
}
