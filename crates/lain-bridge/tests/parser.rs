//! The lain-parser bridge against a scripted daemon on a Unix socket.
#![cfg(unix)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::json;

use lain_bridge::fake::{Fake, Handler};
use lain_bridge::http1::Endpoint;
use lain_bridge::parser::{MODEL_ID, ReleaseModel};
use lain_core::Provider;
use lain_identify::CAP_RELEASE_PARSE;

fn daemon(healthy: Arc<AtomicBool>) -> Handler {
    Arc::new(move |r| match (r.method.as_str(), r.path.as_str()) {
        ("GET", "/health") if healthy.load(Ordering::SeqCst) => (200, json!({"status": "ok", "artifact": "e5-s2"})),
        ("GET", "/health") => (503, json!({"error": "loading"})),
        ("POST", "/parse") => {
            let name = r.body.as_ref().and_then(|b| b["filename"].as_str()).unwrap_or("").to_string();
            if name.contains("abstain") {
                (200, json!({"artifact": "e5-s2", "record": null, "abstain_reason": null}))
            } else if name.contains("boom") {
                (500, json!({"error": "internal"}))
            } else if name.contains("Quiet") {
                // Service 1.3.0: other identities, when the request asks for them.
                let asked = r.body.as_ref().and_then(|b| b["alternatives"].as_i64()).unwrap_or(0);
                let mut answer = json!({"artifact": "v6-s11", "record": {"title": "Quiet Harbor 2",
                    "release_group": "Fansub-A", "resolution": "720p"}});
                if asked > 0 {
                    answer["alternatives"] = json!([
                        {"title": "Quiet Harbor", "episodes": [2]},
                        {"title": " quiet  harbor 2 "},
                        {"title": "Quiet Harbor", "season": 2},
                        {"title": ""},
                        {"title": "Quiet Harbor", "episodes": [2]},
                        {"title": "Quiet"}, {"title": "Harbor"}, {"title": "Harbor 2"}, {"title": "Q"}
                    ]);
                }
                (200, answer)
            } else {
                (
                    200,
                    json!({"artifact": "e5-s2", "record": {"title": "Frieren", "season": null, "episodes": [5],
                    "release_group": "Fansub-A", "resolution": "1080p"}}),
                )
            }
        }
        _ => (404, json!({"error": "not-found"})),
    })
}

#[test]
fn answers_declines_and_failures() {
    // macOS's $TMPDIR is too deep for sun_path; /tmp is short everywhere.
    let sock = std::path::PathBuf::from(format!("/tmp/lain-bridge-parser-{}.sock", std::process::id()));
    let healthy = Arc::new(AtomicBool::new(true));
    let fake = Fake::unix(&sock, daemon(healthy.clone()));
    let m = ReleaseModel::new(Endpoint::parse(&fake.url).unwrap());
    assert!(m.health().is_ok());

    let out =
        m.invoke(CAP_RELEASE_PARSE, "parse", json!({"name": "[Fansub-A] Frieren - 05 [1080p][HEVC].mkv"})).unwrap();
    assert_eq!(out["parser"], MODEL_ID);
    assert_eq!(out["title"], "Frieren");
    assert_eq!(out["episodes"], json!([5]));
    assert_eq!(out["absolute"], true);
    assert_eq!(out["group"], "Fansub-A");
    let tok = lain_identify::tokenize("[Fansub-A] Frieren - 05 [1080p][HEVC].mkv", "");
    assert!(!tok.codec.is_empty());
    assert_eq!(out["codec"], json!(tok.codec), "the tokenizer's tag survives the merge");

    assert!(out.get("alternatives").is_none(), "a daemon that sends none (service < 1.3.0) adds none");

    let out = m.invoke(CAP_RELEASE_PARSE, "parse", json!({"name": "[Fansub-A] Quiet Harbor 2 [720p].mkv"})).unwrap();
    assert_eq!(out["title"], "Quiet Harbor 2");
    assert_eq!(
        out["alternatives"],
        json!([
            {"title": "Quiet Harbor", "episodes": [2], "absolute": true},
            {"title": "Quiet Harbor", "season": 2},
            {"title": "Quiet"},
            {"title": "Harbor"},
            {"title": "Harbor 2"}
        ]),
        "cleaned like the answer, without it, without repeats, at most five"
    );

    let out = m.invoke(CAP_RELEASE_PARSE, "", json!({"name": "abstain me"})).unwrap();
    assert_eq!(out["title"], "", "an abstention is the zero Release");

    let e = m.invoke(CAP_RELEASE_PARSE, "", json!({"name": "boom"})).unwrap_err();
    assert_eq!(e.code, "dependency-unavailable");

    let seen = fake.seen();
    let parse = seen.iter().find(|s| s.path == "/parse").unwrap();
    assert_eq!(parse.body.as_ref().unwrap()["filename"], "[Fansub-A] Frieren - 05 [1080p][HEVC].mkv");
    assert_eq!(parse.body.as_ref().unwrap()["alternatives"], 5, "every parse asks for the other identities");
    assert_eq!(parse.header("host"), Some("localhost"));

    // Unhealthy daemon: health fails (after the cache expires).
    healthy.store(false, Ordering::SeqCst);
    let fresh = ReleaseModel::new(Endpoint::parse(&fake.url).unwrap());
    assert_eq!(fresh.health().unwrap_err().code, "dependency-unavailable");
    drop(fake);
    let gone = ReleaseModel::new(Endpoint::Unix(sock));
    assert_eq!(gone.health().unwrap_err().msg, "parser socket not found");
}

/// Opt-in: a real lain-parser daemon (`LAIN_PARSER_LIVE=unix:///…/parser.sock`
/// or `http://127.0.0.1:9377`).
#[test]
#[ignore = "needs a running lain-parser daemon (LAIN_PARSER_LIVE)"]
fn live_lain_parser() {
    let url = std::env::var("LAIN_PARSER_LIVE").expect("LAIN_PARSER_LIVE");
    let m = ReleaseModel::new(Endpoint::parse(&url).unwrap());
    m.health().expect("healthy daemon");
    let out = m.invoke(CAP_RELEASE_PARSE, "parse", json!({"name": "[Fansub-A] Frieren - 05 [1080p].mkv"})).unwrap();
    assert_eq!(out["parser"], MODEL_ID);
    assert_eq!(out["title"], "Frieren");
    assert_eq!(out["episodes"], json!([5]));
}
