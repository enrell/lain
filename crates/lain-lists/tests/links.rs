//! The gateway logic of linking through a real registry and the AniList
//! connector against a loopback fixture (Go: `internal/gateway/list_test.go`
//! and `scrobble_test.go`).

mod support {
    pub mod fixture;
}

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use lain_admin::integrations::IntegrationSettings;
use lain_api::ListEntryPatch;
use lain_core::{Composition, Registry};
use lain_lists::anilist::Endpoints;
use lain_lists::links::{Links, code_from_paste};
use lain_lists::scrobble::Finished;
use lain_lists::{LinkedAccount, ListEntry};
use lain_store::Store;
use support::fixture::{AniListState, Fixture, al_entry, anilist, token};

struct Env {
    links: Arc<Links>,
    remote: Arc<Mutex<AniListState>>,
    fixture: Fixture,
    events: Arc<Mutex<Vec<(String, Value)>>>,
    dir: std::path::PathBuf,
}

impl Drop for Env {
    fn drop(&mut self) {
        self.links.stop();
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

fn env() -> Env {
    let dir = lain_store::test_dir("links");
    let store = Arc::new(Store::open(&dir).unwrap());
    let remote = Arc::new(Mutex::new(AniListState { anime: json!([]), manga: json!([]), token_ok: true }));
    let fixture = Fixture::start(anilist(remote.clone()));
    let mut bindings = BTreeMap::new();
    for (cap, b) in lain_lists::bindings() {
        bindings.insert(cap.to_string(), b);
    }
    let registry = Arc::new(Registry::new(Composition { version: 8, bindings }));
    let endpoints = Endpoints { graphql: format!("{}/graphql", fixture.url), oauth: format!("{}/oauth", fixture.url) };
    lain_lists::register(
        &registry,
        store.clone(),
        endpoints,
        lain_http::Client::new(lain_http::Config::default()).unwrap(),
    );
    let links = Arc::new(Links::open(&store, registry).unwrap());
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    links.on_change(Arc::new(move |u: &str, d: Value| sink.lock().unwrap().push((u.to_string(), d))));
    Env { links, remote, fixture, events, dir }
}

fn set_anime(e: &Env, entries: Vec<Value>) {
    e.remote.lock().unwrap().anime = json!([{"name": "Watching", "isCustomList": false, "entries": entries}]);
}

fn link(e: &Env, user: &str) -> LinkedAccount {
    let id = e.links.exchange_pin("anilist", "good").unwrap();
    e.links.link(user, "anilist", id, "test")
}

#[test]
fn state_round_trip() {
    let e = env();
    let st = e.links.sign_state("u1", "anilist");
    assert_eq!(e.links.verify_state(&st, "anilist").as_deref(), Some("u1"));
    assert_eq!(e.links.verify_state(&st, "mal"), None, "bound to its platform");
    assert_eq!(e.links.verify_state(&format!("{st}x"), "anilist"), None, "tampered");
    for bad in ["", "x", "x.y", "x.y.z"] {
        assert_eq!(e.links.verify_state(bad, "anilist"), None, "{bad:?}");
    }
    let expired = e.links.sign_state_at("u1", "anilist", lain_core::now_unix() - 1);
    assert_eq!(e.links.verify_state(&expired, "anilist"), None, "expired");
}

#[test]
fn code_from_paste_accepts_urls_and_fragments() {
    assert_eq!(code_from_paste("  abc  "), "abc");
    assert_eq!(code_from_paste("https://anilist.co/api/v2/oauth/pin?code=def&x=1"), "def");
    assert_eq!(code_from_paste("https://anilist.co/api/v2/oauth/pin#access_token=ghi&token_type=Bearer"), "ghi");
    assert_eq!(code_from_paste("code=a%2Bb"), "a+b");
    assert_eq!(code_from_paste(""), "");
}

#[test]
fn integrations_decide_the_client() {
    let e = env();
    assert_eq!(e.links.client("anilist"), None);
    e.links
        .save_integrations(&IntegrationSettings {
            anilist_client_id: "12345".into(),
            anilist_client_secret: "sekret".into(),
        })
        .unwrap();
    assert_eq!(e.links.client("anilist"), Some(("12345".into(), "sekret".into())));
    assert_eq!(e.links.client("mal"), None);
    let url = e.links.authorize("u1", "anilist", "12345", "http://h/api/auth/anilist/callback").unwrap();
    assert!(
        url.starts_with(&format!("{}/oauth/authorize?client_id=12345&redirect_uri=http%3A%2F%2Fh%2F", e.fixture.url)),
        "{url}"
    );
    let state = lain_lists::links::parse_query(url.split_once('?').unwrap().1)
        .into_iter()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1;
    assert_eq!(e.links.verify_state(&state, "anilist").as_deref(), Some("u1"));
    assert_eq!(e.links.authorize("u1", "trakt", "1", "x").unwrap_err().code, "unsupported-platform");
    let pin = e.links.pin_url("anilist").unwrap();
    assert_eq!(
        pin,
        "https://anilist.co/api/v2/oauth/authorize?client_id=52227&redirect_uri=https%3A%2F%2Fanilist.co%2Fapi%2Fv2%2Foauth%2Fpin&response_type=code"
    );
    assert_eq!(e.links.pin_url("mal"), None);
}

#[test]
fn link_imports_and_syncs_replace_the_platform_set() {
    let e = env();
    set_anime(&e, vec![al_entry(1, "ANIME", "Frieren", "CURRENT", 12, 28)]);
    e.remote.lock().unwrap().manga = json!([{"name": "Reading", "isCustomList": false, "entries": [al_entry(2, "MANGA", "Solo Quest", "PLANNING", 0, 0)]}]);
    let a = link(&e, "u1");
    assert_eq!((a.remote_user_id.as_str(), a.remote_username.as_str()), ("42", "watcher"));
    let stored = e.links.account("u1", "anilist").unwrap();
    assert_eq!((stored.entry_count, stored.last_sync_error.as_str(), stored.token.clone()), (2, "", token()));
    assert_eq!(e.links.list("u1", "", "").unwrap().len(), 2);
    assert_eq!(e.links.list("u1", "manga", "").unwrap()[0].title, "Solo Quest");
    let actions: Vec<String> =
        e.events.lock().unwrap().iter().map(|(_, d)| d["action"].as_str().unwrap().to_string()).collect();
    assert_eq!(actions, ["linked", "synced"]);

    // The remote shrank: the missing entry is replaced away (D-081).
    set_anime(&e, vec![al_entry(9, "ANIME", "New Pick", "CURRENT", 1, 12)]);
    e.remote.lock().unwrap().manga = json!([]);
    let st = e.links.sync(e.links.account("u1", "anilist").unwrap()).unwrap();
    assert_eq!((st.upserted, st.removed), (1, 2));

    // A revoked token: typed, recorded, entries kept.
    e.remote.lock().unwrap().token_ok = false;
    let err = e.links.sync(e.links.account("u1", "anilist").unwrap()).unwrap_err();
    assert_eq!(err.code, "token-invalid");
    assert_eq!(e.links.account("u1", "anilist").unwrap().last_sync_error, "token-invalid");
    assert_eq!(e.links.list("u1", "", "").unwrap()[0].remote_id, "9");
    assert_eq!(e.events.lock().unwrap().last().unwrap().1["action"], "sync-failed");

    // Unlink removes the account and its entries.
    assert_eq!(e.links.unlink("u1", "anilist").unwrap(), Some(1));
    assert!(e.links.list("u1", "", "").unwrap().is_empty());
    assert_eq!(e.links.unlink("u1", "anilist").unwrap(), None);
}

#[test]
fn a_bad_paste_never_links() {
    let e = env();
    let err = e.links.exchange_pin("anilist", "bad").unwrap_err();
    assert_eq!(err.code, "invalid-grant");
    assert!(e.links.accounts("u1").unwrap().is_empty());
}

#[test]
fn scheduled_sync_covers_accounts_and_short_circuits_expired_tokens() {
    let e = env();
    set_anime(&e, vec![al_entry(1, "ANIME", "Frieren", "CURRENT", 1, 28)]);
    link(&e, "u1");
    link(&e, "u2");
    set_anime(&e, vec![al_entry(7, "ANIME", "Catch Up", "CURRENT", 1, 12)]);
    e.links.sync_all();
    for u in ["u1", "u2"] {
        assert_eq!(e.links.list(u, "", "").unwrap()[0].remote_id, "7", "{u}");
    }
    let mut a = e.links.account("u1", "anilist").unwrap();
    a.token_expires_at = lain_core::now_unix() - 1;
    e.links.put_account(&a);
    e.fixture.taken();
    e.links.sync_all();
    assert_eq!(e.links.account("u1", "anilist").unwrap().last_sync_error, "token-expired");
    assert_eq!(e.fixture.taken().len(), 2, "only u2 reached the remote (two collections)");
    // The ticker runs a catch-up pass at once.
    set_anime(&e, vec![al_entry(8, "ANIME", "Ticker", "CURRENT", 1, 12)]);
    e.links.start(std::time::Duration::from_secs(3600));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while e.links.list("u2", "", "").unwrap()[0].remote_id != "8" {
        assert!(std::time::Instant::now() < deadline, "the catch-up pass never ran");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn ep(n: i64) -> Finished {
    Finished { kind: "episode".into(), title: "Frieren".into(), season: 1, episode: n, ..Finished::default() }
}

#[test]
fn scrobble_is_on_for_a_new_link_and_updates_the_local_copy() {
    let e = env();
    set_anime(&e, vec![al_entry(154587, "ANIME", "Frieren", "CURRENT", 3, 28)]);
    let a = link(&e, "u1");
    assert!(a.scrobble, "a new link updates the platform (D-288)");
    assert!(!e.links.set_scrobble("u1", "anilist", false).unwrap().scrobble);
    e.fixture.taken();
    e.links.scrobble("u1", &ep(4));
    assert!(e.fixture.taken().is_empty(), "the switch turns it off");
    // A relink keeps the owner's choice.
    link(&e, "u1");
    assert!(!e.links.account("u1", "anilist").unwrap().scrobble, "a relink keeps the switch");
    assert!(e.links.set_scrobble("u1", "anilist", true).unwrap().scrobble);
    assert!(e.links.set_scrobble("u1", "mal", true).is_none());
    e.fixture.taken();
    e.links.scrobble("u1", &ep(4));
    let pushes = e.fixture.taken();
    assert_eq!(pushes.len(), 1);
    assert_eq!(pushes[0].json()["variables"], json!({"mediaId": 154587, "progress": 4, "status": "CURRENT"}));
    let entry = e.links.list("u1", "", "").unwrap().remove(0);
    assert_eq!((entry.progress, entry.status.as_str()), (4, "current"), "the local copy follows");
    assert!(entry.pushed_at > 0, "the entry says when lain wrote it");
    let last = e.links.account("u1", "anilist").unwrap().last_push.expect("the last write is recorded");
    assert_eq!(
        (last.title.as_str(), last.progress, last.via.as_str(), last.error.as_str()),
        ("Frieren", 4, "scrobble", "")
    );
    e.links.scrobble("u1", &ep(4));
    assert!(e.fixture.taken().is_empty(), "the same episode is not pushed twice");
    e.links.scrobble("u1", &ep(28));
    assert_eq!(e.fixture.taken()[0].json()["variables"]["status"], "COMPLETED");
    assert_eq!(e.links.list("u1", "", "").unwrap()[0].status, "completed");
    assert_eq!(e.events.lock().unwrap().last().unwrap().1["action"], "scrobbled");
}

/// A failed write is said on the account and on the entry, and the
/// next import clears the entry's mark.
#[test]
fn a_failed_scrobble_is_visible() {
    let e = env();
    set_anime(&e, vec![al_entry(154587, "ANIME", "Frieren", "CURRENT", 3, 28)]);
    link(&e, "u1");
    // A sync that started before the write must not drop its record.
    let before = e.links.account("u1", "anilist").unwrap();
    e.fixture.set(|_| (429, "{}".into()));
    e.links.scrobble("u1", &ep(4));
    let last = e.links.account("u1", "anilist").unwrap().last_push.unwrap();
    assert_eq!((last.error.as_str(), last.progress), ("rate-limited", 4));
    let entry = e.links.list("u1", "", "").unwrap().remove(0);
    assert_eq!((entry.progress, entry.push_error.as_str()), (3, "rate-limited"), "the local copy does not move");
    assert_eq!(e.events.lock().unwrap().last().unwrap().1["action"], "push-failed");
    e.fixture.set(anilist(e.remote.clone()));
    e.links.sync(before).unwrap();
    assert_eq!(e.links.list("u1", "", "").unwrap()[0].push_error, "", "an import is the truth");
    assert_eq!(e.links.account("u1", "anilist").unwrap().last_push.unwrap().error, "rate-limited", "kept by the sync");
}

/// The worker gathers finishes: a season marked watched is one write
/// per entry, completing it.
#[test]
fn queued_finishes_become_one_write() {
    let e = env();
    set_anime(&e, vec![al_entry(154587, "ANIME", "Frieren", "PLANNING", 0, 3)]);
    link(&e, "u1");
    e.links.set_debounce(std::time::Duration::from_millis(200));
    e.fixture.taken();
    for n in [1, 2, 3] {
        e.links.enqueue("u1", ep(n));
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while e.links.list("u1", "", "").unwrap()[0].status != "completed" {
        assert!(std::time::Instant::now() < deadline, "the worker never wrote");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let pushes: Vec<_> = e.fixture.taken().into_iter().filter(|r| r.body.contains("SaveMediaListEntry")).collect();
    assert_eq!(pushes.len(), 1, "one write for three finishes");
    assert_eq!(pushes[0].json()["variables"], json!({"mediaId": 154587, "progress": 3, "status": "COMPLETED"}));
}

#[test]
fn chapters_scrobble_to_manga() {
    let e = env();
    e.remote.lock().unwrap().manga = json!([{"name": "Reading", "isCustomList": false, "entries": [al_entry(30013, "MANGA", "One Piece", "CURRENT", 10, 0)]}]);
    link(&e, "u1");
    e.fixture.taken();
    let ch =
        Finished { kind: "manga".into(), title: "One Piece".into(), season: 2, episode: 11, ..Finished::default() };
    e.links.scrobble("u1", &ch);
    let pushes = e.fixture.taken();
    assert_eq!(
        pushes[0].json()["variables"],
        json!({"mediaId": 30013, "progress": 11, "progressVolumes": 2, "status": "CURRENT"})
    );
    let entry = e.links.list("u1", "manga", "").unwrap().remove(0);
    assert_eq!((entry.progress, entry.progress_volumes), (11, 2));
}

/// A title page's edits: they write to AniList whether or not the
/// switch is on (an explicit action), store the platform's copy, and
/// add a work the list did not hold.
#[test]
fn edits_write_back_and_add_entries() {
    let e = env();
    set_anime(&e, vec![al_entry(154587, "ANIME", "Frieren", "CURRENT", 3, 28)]);
    link(&e, "u1");
    e.links.set_scrobble("u1", "anilist", false);
    e.fixture.taken();
    let patch = |status: Option<&str>, score: Option<f64>| ListEntryPatch {
        status: status.map(str::to_string),
        score,
        ..ListEntryPatch::default()
    };
    let got = e.links.save_entry("u1", "anilist", "154587", &patch(None, Some(9.0)), None).unwrap();
    assert_eq!((got.score, got.progress, got.status.as_str()), (9.0, 3, "current"));
    assert_eq!(e.fixture.taken()[0].json()["variables"], json!({"mediaId": 154587, "scoreRaw": 90}));
    let got = e.links.save_entry("u1", "anilist", "154587", &patch(Some("completed"), None), None).unwrap();
    assert_eq!((got.status.as_str(), got.progress), ("completed", 28), "completing fills the progress");
    let last = e.links.account("u1", "anilist").unwrap().last_push.unwrap();
    assert_eq!((last.via.as_str(), last.status.as_str(), last.title.as_str()), ("edit", "completed", "Frieren"));
    assert_eq!(e.events.lock().unwrap().last().unwrap().1["action"], "saved");

    // A work the list does not hold: added as planning, read back whole.
    let fallback = ListEntry { title: "Local Name".into(), media_type: "anime".into(), ..ListEntry::default() };
    let got = e.links.save_entry("u1", "anilist", "777", &patch(Some("planning"), None), Some(fallback)).unwrap();
    assert_eq!((got.remote_id.as_str(), got.title.as_str(), got.status.as_str()), ("777", "Media 777", "planning"));
    assert_eq!(e.links.list("u1", "", "").unwrap().len(), 2);

    for (bad, code) in [
        (patch(Some("watching"), None), "invalid-message"),
        (patch(None, Some(11.0)), "invalid-message"),
        (ListEntryPatch::default(), "invalid-message"),
    ] {
        assert_eq!(e.links.save_entry("u1", "anilist", "154587", &bad, None).unwrap_err().code, code, "{bad:?}");
    }
    assert_eq!(
        e.links.save_entry("u2", "anilist", "154587", &patch(None, Some(1.0)), None).unwrap_err().code,
        "not-found"
    );
    e.remote.lock().unwrap().token_ok = false;
    let err = e.links.save_entry("u1", "anilist", "154587", &patch(None, Some(1.0)), None).unwrap_err();
    assert_eq!(err.code, "token-invalid");
    assert_eq!(e.links.account("u1", "anilist").unwrap().last_push.unwrap().error, "token-invalid");
}
