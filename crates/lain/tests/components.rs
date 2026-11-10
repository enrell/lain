//! The P4 components end to end: `lain component install` writes a
//! manifest, the provisioner spawns the real `lain component run`
//! process, and the bindings and routes reach the bridged app (a
//! scripted stand-in here; the live tests in `lain-bridge` use the real
//! apps). Health and declines are what decide who answers.
#![cfg(unix)]

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use lain_bridge::fake::{Fake, Handler};
use lain_conformance::http::{self, Base};
use lain_server::app::{App, Config};

const EXE: &str = env!("CARGO_BIN_EXE_lain");

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
        cfg.offline = true;
        cfg.watch = false;
        cfg.auto_enrich = false;
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

    fn wait_healthy(&self, provider: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        let healthy = || self.app.registry.provider_infos().iter().any(|p| p.id == provider && p.healthy);
        while !healthy() {
            assert!(Instant::now() < deadline, "{provider} never became healthy");
            std::thread::sleep(Duration::from_millis(50));
        }
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

/// A scratch dir short enough for the component sockets under
/// `<data-dir>/plugins/.run/` (macOS's $TMPDIR is too deep for sun_path).
fn short_dir(tag: &str) -> std::path::PathBuf {
    let d = std::path::PathBuf::from(format!("/tmp/lp4-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn lain_rs(args: &[&str]) -> std::process::Output {
    std::process::Command::new(EXE).args(args).output().unwrap()
}

fn install(data: &Path, id: &str, opts: &[&str]) {
    let mut args = vec!["component", "install", id, "--data-dir", data.to_str().unwrap()];
    args.extend_from_slice(opts);
    let out = lain_rs(&args);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
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

fn login(base: &Base, user: &str, pw: &str) -> String {
    let (st, v) = call(base, "POST", "/api/auth/login", "", Some(json!({"username": user, "password": pw})));
    assert_eq!(st, 200, "{v}");
    v["token"].as_str().unwrap().to_string()
}

#[test]
fn component_cli_writes_runnable_manifests() {
    let dir = short_dir("cli");
    let out = lain_rs(&["component", "list"]);
    let list = String::from_utf8_lossy(&out.stdout);
    for id in ["lain-release-model", "lain-subtitle-animesubs", "lain-acquire-anything"] {
        assert!(list.contains(id), "{list}");
    }
    install(&dir, "lain-release-model", &["--parser", "unix:///run/none.sock"]);
    let raw = std::fs::read(dir.join("plugins/lain-release-model.json")).unwrap();
    let m: lain_component::Manifest = serde_json::from_slice(&raw).unwrap();
    assert_eq!(m.capabilities, vec!["lain.release.parse@1"]);
    assert_eq!(m.execution.args[..5], ["component", "run", "lain-release-model", "--sock", "{sock}"]);
    assert!(m.execution.args.contains(&"unix:///run/none.sock".to_string()));
    assert!(lain_rs(&["component", "install", "nope", "--data-dir", dir.to_str().unwrap()]).status.code() == Some(1));
    let typo =
        lain_rs(&["component", "install", "lain-acquire-anything", "--data-dir", dir.to_str().unwrap(), "--tokn", "x"]);
    assert!(!typo.status.success(), "unknown options are refused");
    assert!(
        lain_rs(&["component", "uninstall", "lain-release-model", "--data-dir", dir.to_str().unwrap()])
            .status
            .success()
    );
    assert!(!dir.join("plugins/lain-release-model.json").exists());
    std::fs::remove_dir_all(dir).ok();
}

fn parser_daemon(up: Arc<AtomicBool>) -> Handler {
    Arc::new(move |r| match r.path.as_str() {
        "/health" if up.load(Ordering::SeqCst) => (200, json!({"status": "ok"})),
        "/health" => (503, json!({"error": "down"})),
        _ => {
            let name = r.body.as_ref().and_then(|b| b["filename"].as_str()).unwrap_or("").to_string();
            if name.contains("abstain") {
                (200, json!({"record": null}))
            } else {
                (200, json!({"record": {"title": "Model Title", "episodes": [7]}}))
            }
        }
    })
}

#[test]
fn the_model_answers_first_and_the_tokenizer_answers_its_declines() {
    let dir = short_dir("parser");
    // sun_path is short: keep the socket under /tmp.
    let sock = std::path::PathBuf::from(format!("/tmp/lain-p4-parser-{}.sock", std::process::id()));
    let up = Arc::new(AtomicBool::new(true));
    let fake = Fake::unix(&sock, parser_daemon(up.clone()));
    let data = dir.join("data");
    install(&data, "lain-release-model", &["--parser", &fake.url]);
    let server = Server::start(&data);
    server.wait_healthy("lain-release-model");
    let reg = &server.app.registry;
    let accepted = |v: &Value| v["title"].as_str().is_some_and(|t| !t.is_empty());
    let parse = |name: &str| {
        reg.call_first("lain.release.parse@1", "parse", json!({"name": name, "kind": "anime"}), accepted)
            .unwrap()
            .unwrap()
    };
    let (out, by) = parse("[Fansub-A] Show - 07 [1080p].mkv");
    assert_eq!((by.as_str(), out["title"].as_str()), ("lain-release-model", Some("Model Title")));
    assert_eq!(out["resolution"], "1080p", "tokenizer tags merged");
    let (out, by) = parse("[Fansub-A] Show - 07 abstain.mkv");
    assert_eq!(by, "lain-release-tokenizer", "an abstention falls through: {out}");
    // The daemon goes unhealthy: after the health cache the model is skipped.
    up.store(false, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(2300));
    let (_, by) = parse("[Fansub-A] Show - 07 [1080p].mkv");
    assert_eq!(by, "lain-release-tokenizer");
    let info = reg.provider_infos().into_iter().find(|p| p.id == "lain-release-model").unwrap();
    assert!(!info.healthy, "an unhealthy component is reported: {info:?}");
    drop(server);
    drop(fake);
    std::fs::remove_dir_all(dir).ok();
}

const TOKEN: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

fn animesubs() -> Handler {
    Arc::new(|r| {
        if r.path == "/v1/health" {
            return (200, json!({"service": "animesubs", "version": "1", "api": 1}));
        }
        if r.header("authorization") != Some(&format!("Bearer {TOKEN}")) {
            return (401, json!({"error": {"code": "unauthorized", "message": "token"}}));
        }
        let job = |state: &str| {
            json!({"id": "job-1", "state": state, "phase": state, "progress": null,
                "input": "/x", "output": "/y", "target_language": "pt-BR", "created_at_ms": 1})
        };
        match (r.method.as_str(), r.path.as_str()) {
            ("POST", "/v1/jobs") => (202, job("queued")),
            ("GET", "/v1/jobs/job-1") => (200, job("running")),
            ("POST", "/v1/jobs/job-1/cancel") => (200, job("cancelled")),
            _ => (404, json!({"error": {"code": "not_found", "message": "no job"}})),
        }
    })
}

#[test]
fn translation_routes_drive_the_translate_component() {
    let dir = short_dir("translate");
    let show = dir.join("media/Show");
    std::fs::create_dir_all(&show).unwrap();
    std::fs::write(show.join("Show - 01.mkv"), b"x").unwrap();
    std::fs::write(show.join("Show - 01.en.srt"), b"1\n00:00:01,000 --> 00:00:02,000\nHi\n").unwrap();
    let data = dir.join("data");
    let server = Server::start(&data);
    let base = Base::parse(&server.url).unwrap();
    call(&base, "POST", "/api/setup", "", Some(json!({"username": "admin", "password": "password123"})));
    let admin = login(&base, "admin", "password123");
    call(&base, "POST", "/api/users", &admin, Some(json!({"username": "bob", "password": "bobpassword"})));
    let bob = login(&base, "bob", "bobpassword");
    let media = dir.join("media").to_string_lossy().into_owned();
    let (st, _) = call(&base, "POST", "/api/libraries", &admin, Some(json!({"name": "TV", "path": media})));
    assert_eq!(st, 201);
    call(&base, "POST", "/api/library/scan", &admin, None);
    let deadline = Instant::now() + Duration::from_secs(20);
    let id = loop {
        let (_, page) = call(&base, "GET", "/api/catalog", &admin, None);
        if let Some(id) = page["items"][0]["id"].as_str() {
            break id.to_string();
        }
        assert!(Instant::now() < deadline, "scan found nothing");
        std::thread::sleep(Duration::from_millis(50));
    };
    let path = format!("/api/items/{id}/sidecars/0/translate");
    let body = json!({"target_language": "pt-BR"});

    // Nothing installed: the capability has no provider.
    let (st, v) = call(&base, "POST", &path, &admin, Some(body.clone()));
    assert_eq!((st, v["code"].as_str()), (503, Some("dependency-unavailable")), "{v}");

    // AnimeSubs found through its discovery file and token file.
    let fake = Fake::tcp(animesubs());
    let home = dir.join("animesubs-home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("server.json"), json!({"service": "animesubs", "url": fake.url}).to_string()).unwrap();
    std::fs::write(home.join("server-token"), TOKEN).unwrap();
    install(&data, "lain-subtitle-animesubs", &["--home", home.to_str().unwrap(), "--model-kind", "ollama"]);
    server.wait_healthy("lain-subtitle-animesubs");

    assert_eq!(call(&base, "POST", &path, &bob, Some(body.clone())).0, 403, "admin only");
    assert_eq!(call(&base, "POST", &path, &admin, Some(json!({}))).0, 400);
    let (st, _) = call(&base, "POST", &format!("/api/items/{id}/sidecars/9/translate"), &admin, Some(body.clone()));
    assert_eq!(st, 404);
    assert_eq!(call(&base, "POST", "/api/items/nope/sidecars/0/translate", &admin, Some(body.clone())).0, 404);
    let (st, job) = call(&base, "POST", &path, &admin, Some(body.clone()));
    assert_eq!(st, 202, "{job}");
    assert_eq!((job["provider"].as_str(), job["job"].as_str()), (Some("lain-subtitle-animesubs"), Some("job-1")));
    let sent = fake.seen().into_iter().find(|s| s.path == "/v1/jobs").unwrap().body.unwrap();
    assert!(sent["input"].as_str().unwrap().ends_with("Show - 01.en.srt"), "the sidecar by index: {sent}");
    assert_eq!(sent["provider"]["kind"], "ollama");

    let job_path = "/api/subtitle-translations/lain-subtitle-animesubs/job-1";
    assert_eq!(call(&base, "GET", job_path, &admin, None).1["state"], "running");
    assert_eq!(call(&base, "DELETE", job_path, &admin, None).1["state"], "cancelled");
    assert_eq!(call(&base, "GET", "/api/subtitle-translations/lain-subtitle-animesubs/job-9", &admin, None).0, 404);
    assert_eq!(call(&base, "GET", "/api/subtitle-translations/nobody/job-1", &admin, None).0, 404);

    drop(server);
    drop(fake);
    std::fs::remove_dir_all(dir).ok();
}
