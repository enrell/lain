//! The AniList connector against a loopback fixture (Go:
//! `internal/plugins/listlink/anilist_test.go`).

mod support {
    pub mod fixture;
}

use std::sync::{Arc, Mutex};

use serde_json::json;

use lain_core::Provider;
use lain_lists::CAP_LIST_LINK;
use lain_lists::anilist::{AniList, Endpoints};
use lain_lists::link::{AuthorizeInput, ExchangeInput, FetchInput, PushInput};
use lain_lists::links::parse_query;
use support::fixture::{AniListState, Fixture, al_entry, anilist, token};

fn connector(f: &Fixture) -> AniList {
    AniList::new(
        Endpoints { graphql: format!("{}/graphql", f.url), oauth: format!("{}/oauth", f.url) },
        lain_http::Client::new(lain_http::Config::default()).unwrap(),
    )
}

fn state() -> Arc<Mutex<AniListState>> {
    Arc::new(Mutex::new(AniListState { anime: json!([]), manga: json!([]), token_ok: true }))
}

#[test]
fn authorize_url_carries_client_redirect_and_state() {
    let a = AniList::new(Endpoints::default(), lain_http::Client::new(lain_http::Config::default()).unwrap());
    let out = a
        .authorize(&AuthorizeInput {
            platform: "anilist".into(),
            client_id: "42".into(),
            redirect_uri: "http://lain.local:9360/api/auth/anilist/callback".into(),
            state: "abc.def".into(),
        })
        .unwrap();
    let (base, q) = out.url.split_once('?').unwrap();
    assert_eq!(base, "https://anilist.co/api/v2/oauth/authorize");
    let q = parse_query(q);
    assert_eq!(
        q,
        vec![
            ("client_id".to_string(), "42".to_string()),
            ("redirect_uri".into(), "http://lain.local:9360/api/auth/anilist/callback".into()),
            ("response_type".into(), "code".into()),
            ("state".into(), "abc.def".into()),
        ]
    );
    assert_eq!(a.authorize(&AuthorizeInput::default()).unwrap_err().code, "invalid-message");
}

#[test]
fn other_platforms_are_declined() {
    let a = AniList::new(Endpoints::default(), lain_http::Client::new(lain_http::Config::default()).unwrap());
    for op in ["authorize", "exchange", "fetch", "push"] {
        assert_eq!(a.invoke(CAP_LIST_LINK, op, json!({"platform": "trakt"})).unwrap_err().code, "unsupported-platform");
    }
    assert_eq!(a.invoke(CAP_LIST_LINK, "poke", json!({"platform": "anilist"})).unwrap_err().code, "invalid-message");
}

#[test]
fn exchange_and_invalid_grant() {
    let st = state();
    let f = Fixture::start(anilist(st.clone()));
    let a = connector(&f);
    let ex = |code: &str| ExchangeInput {
        platform: "anilist".into(),
        client_id: "1".into(),
        client_secret: "s".into(),
        code: code.into(),
        redirect_uri: "http://x/cb".into(),
    };
    let id = a.exchange(&ex("good")).unwrap();
    assert_eq!((id.token.clone(), id.remote_user_id.as_str(), id.remote_username.as_str()), (token(), "42", "watcher"));
    assert_eq!(id.token_expires_at, 4_102_444_800);
    let reqs = f.taken();
    let tok = reqs[0].json();
    assert_eq!((tok["code"].as_str(), tok["grant_type"].as_str()), (Some("good"), Some("authorization_code")));
    assert_eq!(reqs[0].header("content-type"), "application/json");
    assert_eq!(reqs[1].header("authorization"), format!("Bearer {}", token()));
    let e = a.exchange(&ex("bad")).unwrap_err();
    assert_eq!((e.code.as_str(), e.msg.as_str()), ("invalid-grant", "The authorization code is invalid"));
    f.set(|_| (200, r#"{"token_type":"Bearer"}"#.into()));
    assert_eq!(a.exchange(&ex("good")).unwrap_err().msg, "token response carried no access_token");
    assert_eq!(a.exchange(&ExchangeInput::default()).unwrap_err().code, "invalid-message");
}

#[test]
fn fetch_maps_entries_and_folds_custom_lists() {
    let st = state();
    {
        let mut s = st.lock().unwrap();
        let mut custom = al_entry(154587, "ANIME", "Frieren: Beyond Journey's End", "CURRENT", 12, 28);
        custom["notes"] = json!("dup");
        s.anime = json!([
            {"name": "Custom", "isCustomList": true, "entries": [custom, al_entry(9, "ANIME", "Only Custom", "PLANNING", 0, 12)]},
            {"name": "Watching", "isCustomList": false, "entries": [al_entry(154587, "ANIME", "Frieren: Beyond Journey's End", "CURRENT", 12, 28)]},
        ]);
        let mut op = al_entry(30013, "MANGA", "One Piece", "CURRENT", 60, 0);
        op["progressVolumes"] = json!(7);
        op["media"]["coverImage"] = json!({"extraLarge": "", "large": "https://x/op.jpg"});
        s.manga = json!([{"name": "Reading", "isCustomList": false, "entries": [op]}]);
    }
    let f = Fixture::start(anilist(st.clone()));
    let a = connector(&f);
    let out = a.fetch(&FetchInput { platform: "anilist".into(), token: token(), remote_user_id: "42".into() }).unwrap();
    assert_eq!(out.entries.len(), 3, "the custom-list copy folds into the standard one: {:?}", out.entries);
    let frieren = out.entries.iter().find(|e| e.remote_id == "154587").unwrap();
    assert_eq!(frieren.title, "Frieren: Beyond Journey's End");
    assert_eq!((frieren.progress, frieren.progress_total, frieren.status.as_str()), (12, 28, "current"));
    assert_eq!((frieren.cover.as_str(), frieren.started_at.as_str()), ("https://img.example/154587.jpg", "2024-01-05"));
    assert_eq!((frieren.score, frieren.format.as_str(), frieren.notes.as_str()), (8.5, "TV", ""));
    assert_eq!(frieren.completed_at, "");
    let op = out.entries.iter().find(|e| e.remote_id == "30013").unwrap();
    assert_eq!((op.media_type.as_str(), op.progress_volumes, op.cover.as_str()), ("manga", 7, "https://x/op.jpg"));
    assert_eq!(out.entries.iter().find(|e| e.remote_id == "9").unwrap().status, "planning");
    let vars: Vec<_> = f.taken().iter().map(|r| r.json()["variables"].clone()).collect();
    assert_eq!(vars, vec![json!({"userId": 42, "type": "ANIME"}), json!({"userId": 42, "type": "MANGA"})]);
    for (tok, uid) in [("", "42"), ("t", ""), ("t", "abc"), ("t", "0")] {
        let e = a
            .fetch(&FetchInput { platform: "anilist".into(), token: tok.into(), remote_user_id: uid.into() })
            .unwrap_err();
        assert_eq!(e.code, "invalid-message", "{tok:?} {uid:?}");
    }
}

#[test]
fn a_rejected_token_is_typed() {
    let st = state();
    st.lock().unwrap().token_ok = false;
    let f = Fixture::start(anilist(st));
    let e = connector(&f)
        .fetch(&FetchInput { platform: "anilist".into(), token: token(), remote_user_id: "42".into() })
        .unwrap_err();
    assert_eq!(e.code, "token-invalid");
    f.set(|_| (500, "boom".into()));
    let e = connector(&f)
        .fetch(&FetchInput { platform: "anilist".into(), token: token(), remote_user_id: "42".into() })
        .unwrap_err();
    assert_eq!((e.code.as_str(), e.msg.as_str()), ("sync-failed", "anilist graphql: status 500: boom"));
    f.set(|_| (429, "{}".into()));
    let e = connector(&f)
        .fetch(&FetchInput { platform: "anilist".into(), token: token(), remote_user_id: "42".into() })
        .unwrap_err();
    assert_eq!(e.msg, "rate limited (429)");
}

#[test]
fn push_writes_progress_and_status() {
    let st = state();
    let f = Fixture::start(anilist(st));
    let a = connector(&f);
    let push = |status: &str| PushInput {
        platform: "anilist".into(),
        token: token(),
        remote_id: "154587".into(),
        progress: 13,
        status: status.into(),
    };
    let out = a.push(&push("completed")).unwrap();
    assert_eq!((out.progress, out.status.as_str()), (13, "completed"));
    let v = f.taken()[0].json();
    assert!(v["query"].as_str().unwrap().starts_with("mutation"));
    assert_eq!(v["variables"], json!({"mediaId": 154587, "progress": 13, "status": "COMPLETED"}));
    a.push(&push("")).unwrap();
    assert_eq!(f.taken()[0].json()["variables"], json!({"mediaId": 154587, "progress": 13}), "no status when empty");
    f.set(|_| (200, r#"{"errors":[{"message":"validation"}],"data":null}"#.into()));
    assert_eq!(a.push(&push("")).unwrap_err().msg, "anilist rejected the entry: validation");
    for bad in [
        PushInput { token: String::new(), ..push("") },
        PushInput { remote_id: "x".into(), ..push("") },
        PushInput { progress: -1, ..push("") },
    ] {
        assert_eq!(a.push(&bad).unwrap_err().code, "invalid-message");
    }
}
