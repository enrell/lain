//! `lain-subtitle-opensubtitles` against the recorded API answers in
//! `conformance/fixtures/upstream/opensubtitles` (loopback only): the
//! intent of the Go provider's tests.

use std::time::Duration;

use serde_json::{Value, json};

use lain_api::{SUBTITLE_OPENSUBTITLES, SubtitleDownloadOutput, SubtitleProvider, SubtitleSearchOutput};
use lain_conformance::upstream::Upstream;
use lain_core::Provider;
use lain_http::{Client, Config};
use lain_subtitle::{CAP_SUBTITLE, OpenSubtitles};

fn upstream() -> Upstream {
    Upstream::start(&lain_conformance::upstream::dir(), 0).unwrap()
}

fn provider() -> OpenSubtitles {
    OpenSubtitles::new(Client::new(Config { backoff: Duration::from_millis(5), ..Config::default() }).unwrap())
}

fn account(u: &Upstream, key: &str) -> SubtitleProvider {
    SubtitleProvider {
        id: "p1".into(),
        kind: SUBTITLE_OPENSUBTITLES.into(),
        base_url: format!("{}/opensubtitles/api/v1", u.url),
        api_key: key.into(),
        enabled: true,
        ..SubtitleProvider::default()
    }
}

fn call(o: &OpenSubtitles, op: &str, input: Value) -> Result<Value, lain_core::Error> {
    o.invoke(CAP_SUBTITLE, op, input)
}

#[test]
fn search_by_hash_and_episode() {
    let u = upstream();
    let out = call(
        &provider(),
        "search",
        json!({"provider": account(&u, "key-1"), "hash": "8e245d9679d31e12", "size": 12909756, "query": "Show",
               "season": 1, "episode": 5, "languages": ["eng", "por"]}),
    )
    .unwrap();
    let cands = serde_json::from_value::<SubtitleSearchOutput>(out).unwrap().candidates;
    assert_eq!(cands.len(), 2, "the unknown language and the fileless entry are dropped: {cands:?}");
    let (a, b) = (&cands[0], &cands[1]);
    assert_eq!((a.file_id.as_str(), a.language.as_str(), a.hash_match, a.downloads), ("111", "eng", true, 900));
    assert_eq!((a.provider_id.as_str(), a.season, a.episode, a.region.as_str()), ("p1", 1, 5, ""));
    assert_eq!((b.language.as_str(), b.region.as_str(), b.hi), ("por", "pt-BR", true));
    let seen = u.seen();
    let q = &seen[0];
    // Lower-case, alphabetically sorted parameters, as the API asks.
    assert_eq!(
        q.raw_query,
        "episode_number=5&languages=en%2Cpt-br%2Cpt-pt&moviehash=8e245d9679d31e12&query=show&season_number=1&type=episode"
    );
    assert_eq!(q.header("api-key"), Some("key-1"));
    assert!(q.header("user-agent").unwrap().starts_with("Lain v"), "the API wants <App> v<version>");
    assert_eq!(q.header("accept"), Some("application/json"));
}

#[test]
fn movie_search_shape() {
    let u = upstream();
    let p = account(&u, "key-1");
    call(
        &provider(),
        "search",
        json!({"provider": p, "query": "  Film  ", "movie": true, "year": 2001, "languages": []}),
    )
    .unwrap();
    assert_eq!(u.seen()[0].raw_query, "query=film&type=movie&year=2001");
}

#[test]
fn download_logs_in_when_an_account_is_given() {
    let u = upstream();
    let o = provider();
    let mut p = account(&u, "key-1");
    let out: SubtitleDownloadOutput =
        serde_json::from_value(call(&o, "download", json!({"provider": p, "file_id": "111"})).unwrap()).unwrap();
    assert_eq!(out.link, format!("{}/opensubtitles/files/111.srt", u.url));
    assert_eq!((out.remaining, out.file_name.as_str()), (4, "Show.05.en.srt"));
    let downloads = |u: &Upstream| -> Vec<Option<String>> {
        u.seen()
            .iter()
            .filter(|s| s.path.ends_with("/download"))
            .map(|s| s.header("authorization").map(String::from))
            .collect()
    };
    assert_eq!(downloads(&u), vec![None], "anonymous: no login, no bearer");
    assert_eq!(u.hits("/opensubtitles/api/v1/login"), 0);
    assert_eq!(serde_json::from_str::<Value>(&u.seen()[0].body).unwrap(), json!({"file_id": 111}));

    p.username = "lain".into();
    p.password = "secret-pass".into();
    call(&o, "download", json!({"provider": p, "file_id": "111"})).unwrap();
    assert_eq!(u.hits("/opensubtitles/api/v1/login"), 1);
    assert_eq!(downloads(&u)[1].as_deref(), Some("Bearer jwt-1"));
    // The token is cached per account.
    call(&o, "download", json!({"provider": p, "file_id": "111"})).unwrap();
    assert_eq!(u.hits("/opensubtitles/api/v1/login"), 1);

    // The link is fetchable with the shared client under the 5 MiB cap
    // the core applies (D-150).
    let c = Client::new(Config::default()).unwrap();
    let srt = c.get(&out.link).max_body(lain_subtitle::MAX_BYTES).send_blocking().unwrap().ok().unwrap();
    assert!(!lain_subtitle::parse(&srt.body).unwrap().is_empty());
}

#[test]
fn errors_map_to_codes_and_never_leak_secrets() {
    let u = upstream();
    let o = provider();
    let e = call(&o, "search", json!({"provider": account(&u, "key-wrong"), "query": "x", "languages": ["eng"]}))
        .unwrap_err();
    assert_eq!(e.code, "auth-failed");
    assert!(!e.msg.contains("key-wrong"), "{e}");
    assert!(e.msg.starts_with("127.0.0.1:"), "errors name the host: {e}");
    let mut p = account(&u, "key-1");
    p.username = "lain".into();
    p.password = "wrong-pass".into();
    let e = call(&o, "download", json!({"provider": p, "file_id": "111"})).unwrap_err();
    assert_eq!(e.code, "auth-failed");
    assert!(!e.to_string().contains("wrong-pass") && !e.to_string().contains("secret-pass"), "{e}");
    let p = account(&u, "key-1");
    let e = call(&o, "download", json!({"provider": p, "file_id": "222"})).unwrap_err();
    assert_eq!((e.code.as_str(), e.msg.ends_with("no such subtitle")), ("not-found", true));
    let e = call(&o, "download", json!({"provider": p, "file_id": "333"})).unwrap_err();
    assert_eq!(e.code, "rate-limited");
    let mut dead = p.clone();
    dead.base_url = format!("{}/nowhere", u.url);
    assert_eq!(call(&o, "search", json!({"provider": dead, "query": "x"})).unwrap_err().code, "not-found");
    // The secrets are not in Debug either.
    let mut s = account(&u, "key-1");
    s.password = "pw".into();
    let dbg = format!("{s:?}");
    assert!(!dbg.contains("key-1") && !dbg.contains("\"pw\""), "Debug must redact the api key and password");
    assert_eq!((s.public().api_key.as_str(), s.public().password.as_str()), ("", ""));
}

#[test]
fn rejects_bad_input() {
    let o = provider();
    let os = SUBTITLE_OPENSUBTITLES;
    for (op, input) in [
        ("search", json!({"provider": {"kind": "other"}})),
        ("search", json!({"provider": {"kind": os, "api_key": "k", "base_url": "file:///etc"}, "query": "x"})),
        ("search", json!({"provider": {"kind": os, "api_key": "k"}})),
        ("search", json!({"provider": {"kind": os, "api_key": " "}, "query": "x"})),
        ("search", json!({"provider": {"kind": os, "api_key": "k"}, "hash": "123", "query": "x"})),
        ("download", json!({"provider": {"kind": os, "api_key": "k"}, "file_id": "../x"})),
        ("download", json!({"provider": {"kind": os, "api_key": "k"}, "file_id": "0"})),
        ("search", json!("nope")),
        ("explode", json!({})),
    ] {
        let e = call(&o, op, input.clone()).unwrap_err();
        assert!(e.code == "invalid-message" || e.code == "unsupported-provider", "{input}: {e}");
    }
    let e = call(&o, "search", json!({"provider": {"kind": "other"}})).unwrap_err();
    assert_eq!(e.code, "unsupported-provider", "another kind declines");
    assert_eq!(o.invoke("lain.bogus@1", "search", json!({})).unwrap_err().code, "invalid-message");
    assert!(o.health().is_ok());
}

/// The real API, opt-in: `LAIN_OPENSUBTITLES_KEY=<key> cargo test -p
/// lain-subtitle -- --ignored`.
#[test]
#[ignore = "reaches the network and needs an API key"]
fn live_opensubtitles() {
    let Ok(key) = std::env::var("LAIN_OPENSUBTITLES_KEY") else { return };
    let p =
        SubtitleProvider { id: "live".into(), kind: SUBTITLE_OPENSUBTITLES.into(), api_key: key, ..Default::default() };
    let out = call(&provider(), "search", json!({"provider": p, "query": "Frieren", "languages": ["eng"]})).unwrap();
    assert!(!serde_json::from_value::<SubtitleSearchOutput>(out).unwrap().candidates.is_empty());
}
