//! The AnimeSubs and Anything bridges against scripted servers (the
//! wire shapes of animesubs#8 and Anything#1), plus opt-in live tests
//! against the real apps.

use std::sync::Arc;

use serde_json::{Value, json};

use lain_bridge::animesubs::{AnimeSubs, CAP_SUBTITLE_TRANSLATE};
use lain_bridge::anything::{Anything, CAP_ACQUIRE_EXTRACT};
use lain_bridge::fake::{Fake, Handler, Seen};
use lain_core::Provider;

/// An absolute path on this OS (`/m/x` is relative on Windows).
fn abs(p: &str) -> String {
    if cfg!(windows) { format!("C:{p}") } else { p.to_string() }
}

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn authed(r: &Seen) -> bool {
    r.header("authorization") == Some(&format!("Bearer {TOKEN}"))
}

fn app_job(state: &str) -> Value {
    json!({"id": "job-1", "kind": "translate_subtitle", "state": state, "phase": state,
        "progress": {"done": 3, "total": 10}, "input": "/m/ep01.ass", "output": "/m/ep01.pt-BR.ass",
        "target_language": "pt-BR", "created_at_ms": 5, "started_at_ms": 6, "finished_at_ms": null, "error": null})
}

fn animesubs_fake() -> Handler {
    Arc::new(|r| {
        if r.path == "/v1/health" {
            return (200, json!({"service": "animesubs", "version": "1.1.0", "api": 1}));
        }
        if !authed(r) {
            return (401, json!({"error": {"code": "unauthorized", "message": "missing or wrong token"}}));
        }
        match (r.method.as_str(), r.path.as_str()) {
            ("POST", "/v1/jobs") => {
                let b = r.body.clone().unwrap_or_default();
                if b["input"].as_str().is_some_and(|i| i.ends_with(".txt")) {
                    return (422, json!({"error": {"code": "unsupported", "message": "not a subtitle file"}}));
                }
                (202, app_job("queued"))
            }
            ("GET", "/v1/jobs/job-1") => (200, app_job("running")),
            ("POST", "/v1/jobs/job-1/cancel") => (200, app_job("cancelled")),
            ("POST", "/v1/jobs/job-2/cancel") => {
                (409, json!({"error": {"code": "not_cancellable", "message": "job job-2 already succeeded"}}))
            }
            _ => (404, json!({"error": {"code": "not_found", "message": "no job"}})),
        }
    })
}

fn animesubs_bridge(url: &str, token: &str) -> AnimeSubs {
    let mut loc = AnimeSubs::locator(None, Some(url.into()));
    loc.token = Some(token.into());
    AnimeSubs::new(
        loc,
        Some(lain_api::TranslateModel { kind: "ollama".into(), model: "m".into(), ..Default::default() }),
    )
}

#[test]
fn animesubs_bridge_drives_jobs() {
    let fake = Fake::tcp(animesubs_fake());
    let b = animesubs_bridge(&fake.url, TOKEN);
    assert!(b.health().is_ok());
    let start = json!({"input": abs("/m/ep01.ass"), "target_language": "pt-BR"});
    let out = b.invoke(CAP_SUBTITLE_TRANSLATE, "start", start).unwrap();
    assert_eq!((out["job"].as_str(), out["state"].as_str()), (Some("job-1"), Some("queued")));
    assert_eq!(out["provider"], "lain-subtitle-animesubs");
    assert_eq!((out["done"].as_u64(), out["total"].as_u64()), (Some(3), Some(10)));
    let sent = fake.seen().into_iter().find(|s| s.method == "POST").unwrap();
    let body = sent.body.clone().unwrap();
    assert_eq!(body["kind"], "translate_subtitle");
    assert_eq!(body["provider"], json!({"kind": "ollama", "model": "m"}), "the configured default model");
    assert!(body.get("output").is_none());
    assert_eq!(sent.header("host").unwrap(), format!("127.0.0.1:{}", fake.port()));

    assert_eq!(b.invoke(CAP_SUBTITLE_TRANSLATE, "status", json!({"job": "job-1"})).unwrap()["state"], "running");
    assert_eq!(b.invoke(CAP_SUBTITLE_TRANSLATE, "cancel", json!({"job": "job-1"})).unwrap()["state"], "cancelled");
    let e = b.invoke(CAP_SUBTITLE_TRANSLATE, "cancel", json!({"job": "job-2"})).unwrap_err();
    assert_eq!(e.code, "not-cancellable");
    assert_eq!(b.invoke(CAP_SUBTITLE_TRANSLATE, "status", json!({"job": "job-9"})).unwrap_err().code, "not-found");
    let e = b
        .invoke(CAP_SUBTITLE_TRANSLATE, "start", json!({"input": abs("/m/x.txt"), "target_language": "pt"}))
        .unwrap_err();
    assert_eq!(e.code, "unsupported-format", "declines so another translator can try");
    let e =
        b.invoke(CAP_SUBTITLE_TRANSLATE, "start", json!({"input": "rel.ass", "target_language": "pt"})).unwrap_err();
    assert_eq!(e.code, "invalid-message");
    assert_eq!(b.invoke(CAP_SUBTITLE_TRANSLATE, "nope", json!({})).unwrap_err().code, "invalid-message");

    // A rotated token is auth-failed, not silently dropped.
    let wrong = animesubs_bridge(&fake.url, "bad");
    let e = wrong.invoke(CAP_SUBTITLE_TRANSLATE, "status", json!({"job": "job-1"})).unwrap_err();
    assert_eq!(e.code, "auth-failed");
    assert!(!e.msg.contains("bad"), "the token never reaches an error");

    drop(fake);
    let down = animesubs_bridge("http://127.0.0.1:1", TOKEN);
    assert_eq!(down.health().unwrap_err().code, "dependency-unavailable");
}

#[test]
fn discovery_files_are_read_each_time() {
    let home = lain_store::test_dir("bridge-discovery");
    let b = AnimeSubs::new(AnimeSubs::locator(Some(home.clone()), None), None);
    let e = b.health().unwrap_err();
    assert!(e.msg.contains("not running"), "{e}");
    let fake = Fake::tcp(animesubs_fake());
    std::fs::write(home.join("server.json"), json!({"service": "animesubs", "url": fake.url}).to_string()).unwrap();
    std::fs::write(home.join("server-token"), format!("{TOKEN}\n")).unwrap();
    let b = AnimeSubs::new(AnimeSubs::locator(Some(home.clone()), None), None);
    assert!(b.health().is_ok());
    let e = b
        .invoke(CAP_SUBTITLE_TRANSLATE, "start", json!({"input": abs("/m/a.ass"), "target_language": "pt"}))
        .unwrap_err();
    assert_eq!(e.code, "invalid-message", "no model given and none configured");
    std::fs::write(home.join("server.json"), json!({"service": "anything", "url": fake.url}).to_string()).unwrap();
    let b = AnimeSubs::new(AnimeSubs::locator(Some(home.clone()), None), None);
    assert!(b.health().is_err(), "a discovery file of another service is ignored");
    std::fs::remove_dir_all(home).ok();
}

fn anything_fake() -> Handler {
    Arc::new(|r| {
        if r.path == "/health" {
            return (200, json!({"service": "anything", "version": "0.1.0", "api": 1}));
        }
        if !authed(r) {
            return (401, json!({"error": "unauthorized"}));
        }
        match (r.method.as_str(), r.path.as_str()) {
            ("POST", "/extract") => (
                200,
                json!({
                "id": "x1", "title": "A Clip", "webpage_url": "https://video.example/watch/1", "duration": 61.5,
                "site": "generic", "content_class": "live-action", "thumbnail": "https://img.example/t.jpg",
                "streams": [{"id": "hls-1080", "url": "https://cdn.example/1080.m3u8?sig=abc", "type": "hls",
                    "quality": "1080p", "width": 1920, "height": 1080, "is_audio_only": false, "is_video_only": false}],
                "subtitles": [{"language": "en", "url": "https://cdn.example/en.vtt"}]}),
            ),
            ("POST", "/queue") => (201, json!({"id": 4, "url": "https://video.example/watch/1", "status": "pending"})),
            ("GET", "/queue/4") => (
                200,
                json!({"id": 4, "url": "https://video.example/watch/1", "status": "running",
                "title": "A Clip", "downloaded_bytes": 10, "total_bytes": 100}),
            ),
            ("GET", "/queue/5") => (200, json!({"id": 5, "status": "completed", "output_path": "/lib/a.mp4"})),
            ("DELETE", "/queue/4") => (200, json!({"success": true})),
            _ => (404, json!({"error": "Not found"})),
        }
    })
}

#[test]
fn anything_bridge_extracts_and_queues() {
    let fake = Fake::tcp(anything_fake());
    let mut loc = Anything::locator(None, Some(fake.url.clone()));
    loc.token = Some(TOKEN.into());
    let b = Anything::new(loc);
    assert!(b.health().is_ok());
    let out = b.invoke(CAP_ACQUIRE_EXTRACT, "extract", json!({"url": "https://video.example/watch/1"})).unwrap();
    assert_eq!(out["title"], "A Clip");
    assert_eq!(out["candidates"][0]["id"], "hls-1080");
    assert_eq!(out["subtitles"], json!(["en"]));
    assert!(!out.to_string().contains("sig=abc"), "signed stream urls never leave the component");

    let job = b
        .invoke(
            CAP_ACQUIRE_EXTRACT,
            "enqueue",
            json!({"url": "https://video.example/watch/1", "format": "1080p", "output_dir": abs("/lib/videos")}),
        )
        .unwrap();
    assert_eq!((job["job"].as_str(), job["state"].as_str()), (Some("4"), Some("queued")));
    let sent = fake.seen().into_iter().find(|s| s.path == "/queue").unwrap().body.unwrap();
    assert_eq!(sent["options"], json!({"format": "1080p", "output_dir": abs("/lib/videos")}));

    let st = b.invoke(CAP_ACQUIRE_EXTRACT, "status", json!({"job": "4"})).unwrap();
    assert_eq!((st["state"].as_str(), st["total_bytes"].as_i64()), (Some("running"), Some(100)));
    assert_eq!(b.invoke(CAP_ACQUIRE_EXTRACT, "cancel", json!({"job": "4"})).unwrap()["state"], "cancelled");
    assert_eq!(b.invoke(CAP_ACQUIRE_EXTRACT, "cancel", json!({"job": "5"})).unwrap_err().code, "not-cancellable");
    assert_eq!(b.invoke(CAP_ACQUIRE_EXTRACT, "status", json!({"job": "9"})).unwrap_err().code, "not-found");
    let e = b.invoke(CAP_ACQUIRE_EXTRACT, "extract", json!({"url": "file:///etc/hosts"})).unwrap_err();
    assert_eq!(e.code, "unsupported-url");
    let e = b.invoke(CAP_ACQUIRE_EXTRACT, "enqueue", json!({"url": "https://v.example/1", "output_dir": "rel"}));
    assert_eq!(e.unwrap_err().code, "invalid-message");
}

/// Opt-in: a real AnimeSubs server binary (`ANIMESUBS_LIVE_BIN=…/animesubs-cli`).
/// Starts it on port 0 in a scratch home, finds it by its discovery file,
/// runs a job against a model endpoint that refuses, and cancels another.
#[test]
#[ignore = "needs the animesubs-cli binary (ANIMESUBS_LIVE_BIN)"]
fn live_animesubs() {
    let bin = std::env::var("ANIMESUBS_LIVE_BIN").expect("ANIMESUBS_LIVE_BIN");
    let home = lain_store::test_dir("live-animesubs");
    let mut child = std::process::Command::new(bin)
        .args(["serve", "--port", "0", "--home"])
        .arg(&home)
        .stdin(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let llm = Fake::tcp(Arc::new(|_| (500, json!({"error": "refused"}))));
    let b = AnimeSubs::new(
        AnimeSubs::locator(Some(home.clone()), None),
        Some(lain_api::TranslateModel {
            kind: "openai".into(),
            model: "m".into(),
            base_url: llm.url.clone(),
            api_key: "k".into(),
        }),
    );
    let mut ok = false;
    for _ in 0..100 {
        if b.health().is_ok() {
            ok = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(ok, "server never answered: {:?}", b.health());
    let sub = home.join("ep01.srt");
    std::fs::write(&sub, "1\n00:00:01,000 --> 00:00:02,000\nHello\n\n2\n00:00:03,000 --> 00:00:04,000\nBye\n").unwrap();
    let start = json!({"input": sub, "target_language": "pt-BR"});
    let first = b.invoke(CAP_SUBTITLE_TRANSLATE, "start", start.clone()).unwrap();
    let second = b.invoke(CAP_SUBTITLE_TRANSLATE, "start", json!({"input": sub, "target_language": "es"})).unwrap();
    let c = b.invoke(CAP_SUBTITLE_TRANSLATE, "cancel", json!({"job": second["job"]})).unwrap();
    assert_eq!(c["state"], "cancelled");
    let mut last = Value::Null;
    for _ in 0..300 {
        last = b.invoke(CAP_SUBTITLE_TRANSLATE, "status", json!({"job": first["job"]})).unwrap();
        if ["succeeded", "failed", "cancelled"].contains(&last["state"].as_str().unwrap()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(last["state"], "failed", "{last}");
    assert!(!last.to_string().contains("\"k\""), "the key is never echoed");
    let e = b.invoke(CAP_SUBTITLE_TRANSLATE, "cancel", json!({"job": first["job"]})).unwrap_err();
    assert_eq!(e.code, "not-cancellable");
    let _ = child.kill();
    let _ = child.wait();
    std::fs::remove_dir_all(home).ok();
}

/// Opt-in: a running Anything daemon with token auth, found by its
/// discovery file in `ANYTHING_LIVE_HOME` (a scratch `ANYTHING_CONFIG_DIR`).
/// Uses only fictional, unresolvable URLs.
#[test]
#[ignore = "needs a running Anything daemon (ANYTHING_LIVE_HOME)"]
fn live_anything() {
    let home = std::env::var("ANYTHING_LIVE_HOME").expect("ANYTHING_LIVE_HOME");
    let b = Anything::new(Anything::locator(Some(home.into()), None));
    b.health().expect("healthy daemon");
    let e = b.invoke(CAP_ACQUIRE_EXTRACT, "extract", json!({"url": "https://video.invalid/watch/1"})).unwrap_err();
    assert!(["extract-failed", "unsupported-url"].contains(&e.code.as_str()), "{e}");
    let job = b.invoke(CAP_ACQUIRE_EXTRACT, "enqueue", json!({"url": "https://video.invalid/watch/2"})).unwrap();
    let id = job["job"].clone();
    let st = b.invoke(CAP_ACQUIRE_EXTRACT, "status", json!({"job": id})).unwrap();
    assert!(["queued", "running", "failed"].contains(&st["state"].as_str().unwrap()), "{st}");
    if st["state"] != "failed" {
        assert_eq!(b.invoke(CAP_ACQUIRE_EXTRACT, "cancel", json!({"job": id})).unwrap()["state"], "cancelled");
    }
}
