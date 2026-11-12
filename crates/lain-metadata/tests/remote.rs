//! The network metadata providers against recorded upstream answers
//! (`conformance/fixtures/upstream`, served on loopback): the Go tests'
//! intent, and the merge-many enrichment across NFO and all four
//! providers. Nothing here reaches the network except the opt-in
//! `live_*` tests (`cargo test -p lain-metadata -- --ignored`).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use lain_api::CatalogItem;
use lain_conformance::upstream::Upstream;
use lain_core::{Binding, BindingMode, Composition, Provider, Registry};
use lain_http::{Client, Config};
use lain_metadata::remote::{AniList, Jikan, Kitsu, TvMaze};
use lain_metadata::{
    ANILIST_ID, CAP_METADATA_ENRICH, CAP_METADATA_RESOLVE, CAP_METADATA_SEARCH, DEFAULT_PROVIDERS, ENRICH_ID,
    Endpoints, Enricher, JIKAN_ID, KITSU_ID, MetadataCandidate, MetadataRecord, NFO_ID, Nfo, Remote, TVMAZE_ID,
};

fn upstream() -> Upstream {
    Upstream::start(&lain_conformance::upstream::dir(), 0).unwrap()
}

fn client() -> Client {
    Client::new(Config { backoff: Duration::from_millis(5), ..Config::default() }).unwrap()
}

fn providers(u: &Upstream) -> BTreeMap<String, Arc<dyn Provider>> {
    let r = Remote { client: client(), endpoints: Endpoints::under(&u.url), pace: false };
    r.providers().into_iter().map(|p| (p.id().to_string(), p)).collect()
}

fn search(p: &dyn Provider, query: &str, kind: &str, limit: i64) -> Vec<MetadataCandidate> {
    let out = p.invoke(CAP_METADATA_SEARCH, "search", json!({"query": query, "kind": kind, "limit": limit})).unwrap();
    serde_json::from_value(out).unwrap()
}

fn resolve(p: &dyn Provider, id: &str) -> Result<MetadataRecord, lain_core::Error> {
    p.invoke(CAP_METADATA_RESOLVE, "resolve", json!({"provider": p.id(), "remote_id": id}))
        .map(|v| serde_json::from_value(v).unwrap())
}

#[test]
fn kitsu_search_and_resolve() {
    let u = upstream();
    let ps = providers(&u);
    let k = &*ps[KITSU_ID];
    let list = search(k, "Frieren", "", 2);
    assert_eq!(list.len(), 2);
    let (a, b) = (&list[0], &list[1]);
    assert_eq!((a.provider.as_str(), a.remote_id.as_str(), a.title.as_str()), (KITSU_ID, "46474", "Sousou no Frieren"));
    assert_eq!((a.year, a.kind.as_str()), (2023, "anime"));
    assert!(a.poster.contains("/poster_image/"), "the original tier: {}", a.poster);
    assert!(a.synonyms.iter().any(|s| s.contains("Frieren")));
    assert_eq!(b.title, "Sousou no Frieren 2nd Season");
    let seen = u.seen();
    assert_eq!(seen[0].param("filter[text]"), Some("Frieren"));
    assert_eq!(seen[0].param("page[limit]"), Some("2"));
    assert_eq!(seen[0].header("user-agent"), Some(lain_http::USER_AGENT));

    let rec = resolve(k, "46474").unwrap();
    assert_eq!((rec.title.as_str(), rec.year, rec.episodes), ("Sousou no Frieren", 2023, 28));
    assert!(!rec.synopsis.is_empty() && !rec.poster.is_empty() && !rec.cover.is_empty(), "{rec:?}");
    assert!(resolve(k, "999").unwrap_err().to_string().contains("status 404"));
}

#[test]
fn anilist_search_and_resolve_read_capitalized_fields() {
    let u = upstream();
    let ps = providers(&u);
    let a = &*ps[ANILIST_ID];
    let list = search(a, "Frieren", "anime", 7);
    assert!(!list.is_empty());
    assert_eq!(list[0].remote_id, "154587");
    assert!(list[0].title.starts_with("Frieren: Beyond Journey"), "english title first: {}", list[0].title);
    assert_eq!(list[0].year, 2023);
    let body: Value = serde_json::from_str(&u.seen()[0].body).unwrap();
    assert_eq!(body["variables"], json!({"search": "Frieren", "perPage": 7}));
    assert!(body["query"].as_str().unwrap().contains("Page(page: 1, perPage: $perPage)"));

    let rec = resolve(a, "154587").unwrap();
    assert_eq!((rec.remote_id.as_str(), rec.episodes), ("154587", 28));
    assert!(!rec.synopsis.contains('<'), "html stripped: {}", rec.synopsis);
    assert!(!rec.genres.is_empty() && !rec.poster.is_empty());
    let e = resolve(a, "abc").unwrap_err();
    assert_eq!((e.code.as_str(), e.msg.as_str()), ("invalid-message", "numeric id required"));
}

#[test]
fn jikan_search_resolve_and_failures() {
    let u = upstream();
    let ps = providers(&u);
    let j = &*ps[JIKAN_ID];
    let list = search(j, "Frieren", "", 0);
    assert_eq!(list.len(), 1);
    assert_eq!((list[0].remote_id.as_str(), list[0].year), ("52991", 2023));
    assert_eq!(list[0].title, "Frieren: Beyond Journey's End");
    assert_eq!(list[0].synonyms, vec!["Sousou no Frieren", "葬送のフリーレン"]);
    let q = &u.seen()[0];
    assert_eq!((q.param("q"), q.param("limit"), q.param("order_by")), (Some("Frieren"), Some("5"), Some("members")));
    assert_eq!((q.param("sort"), q.param("sfw")), (Some("desc"), Some("true")));
    let rec = resolve(j, "52991").unwrap();
    assert_eq!(rec.genres, vec!["Adventure", "Drama", "Fantasy"]);
    assert!(rec.poster.ends_with("138006l.jpg"));
    assert_eq!(resolve(j, "not-a-number").unwrap_err().code, "invalid-message");

    // 503 is retried (two retries), then the provider fails its call.
    let before = u.hits("/jikan/v4/anime");
    let e = j.invoke(CAP_METADATA_SEARCH, "search", json!({"query": "Show Name"})).unwrap_err();
    assert_eq!(e.code, "dependency-unavailable");
    assert!(e.msg.contains("status 503"), "{e}");
    assert_eq!(u.hits("/jikan/v4/anime") - before, 3);
    // Garbage answers fail too.
    let g = Jikan::new(client(), &format!("{}/garbage", u.url));
    assert_eq!(
        g.invoke(CAP_METADATA_SEARCH, "search", json!({"query": "x"})).unwrap_err().code,
        "dependency-unavailable"
    );
}

#[test]
fn tvmaze_skips_nameless_and_resolves() {
    let u = upstream();
    let ps = providers(&u);
    let t = &*ps[TVMAZE_ID];
    let list = search(t, "Show Name", "series", 0);
    assert_eq!(list.len(), 1, "the nameless hit is dropped: {list:?}");
    let c = &list[0];
    assert_eq!(
        (c.remote_id.as_str(), c.title.as_str(), c.year, c.kind.as_str()),
        ("9001", "Show Name", 2021, "series")
    );
    assert_eq!(c.poster, "https://static.example.invalid/o/9001.jpg");
    let rec = resolve(t, "9001").unwrap();
    assert_eq!(rec.synopsis, "A keeper returns to the lighthouse.");
    assert_eq!((rec.genres.len(), rec.cover.as_str(), rec.episodes), (2, "", 0));
    let imageless = resolve(t, "69956").unwrap();
    assert_eq!((imageless.poster.as_str(), imageless.synopsis.as_str()), ("", "An elf mage."));
    let frieren = search(t, "Frieren", "", 2);
    assert_eq!(frieren.len(), 2, "the limit applies after the search");
}

#[test]
fn kinds_gate_before_any_request() {
    let u = upstream();
    let ps = providers(&u);
    for (id, accepts) in [
        (KITSU_ID, vec!["", "anime", "episode", "video"]),
        (ANILIST_ID, vec!["", "anime", "episode", "video"]),
        (JIKAN_ID, vec!["", "anime", "episode", "video"]),
        (TVMAZE_ID, vec!["", "anime", "series", "episode", "video"]),
    ] {
        let p = &*ps[id];
        for kind in accepts {
            let before = u.seen().len();
            let _ = p.invoke(CAP_METADATA_SEARCH, "search", json!({"query": "Frieren", "kind": kind}));
            assert!(u.seen().len() > before, "{id} kind {kind:?} must reach the server");
        }
        let before = u.seen().len();
        assert!(search(p, "Frieren", "movie", 0).is_empty());
        assert_eq!(u.seen().len(), before, "{id}: movies never reach the server");
        // Typed input and capability errors.
        for (cap, input) in
            [("lain.bogus@1", json!({})), (CAP_METADATA_SEARCH, json!("nope")), (CAP_METADATA_RESOLVE, json!(42))]
        {
            assert_eq!(p.invoke(cap, "", input).unwrap_err().code, "invalid-message", "{id} {cap}");
        }
        assert_eq!(p.invoke(CAP_METADATA_SEARCH, "explode", json!({})).unwrap_err().code, "invalid-message");
        assert!(p.health().is_ok());
    }
}

#[test]
fn offline_providers_fail_without_connecting() {
    let r = Remote::offline();
    for p in r.providers() {
        let e = p.invoke(CAP_METADATA_SEARCH, "search", json!({"query": "Frieren"})).unwrap_err();
        assert_eq!(e.code, "dependency-unavailable");
        assert!(e.msg.contains("outbound HTTP is off"), "{e}");
    }
}

fn registry(u: &Upstream, store: Arc<lain_store::Store>) -> Arc<Registry> {
    let mut bindings = BTreeMap::new();
    bindings.insert(CAP_METADATA_SEARCH.into(), Binding::new(BindingMode::MergeMany, DEFAULT_PROVIDERS));
    bindings.insert(CAP_METADATA_RESOLVE.into(), Binding::new(BindingMode::MergeMany, DEFAULT_PROVIDERS));
    bindings.insert(CAP_METADATA_ENRICH.into(), Binding::new(BindingMode::ExactlyOne, &[ENRICH_ID]));
    let reg = Arc::new(Registry::new(Composition { version: 8, bindings }));
    reg.register(Arc::new(Nfo));
    for p in providers(u).into_values() {
        reg.register(p);
    }
    reg.register(Arc::new(Enricher::new(&reg, store)));
    reg
}

/// Merge-many as in Go: NFO first, then Kitsu, AniList, Jikan, TVMaze;
/// a failing provider (Jikan's 503) and an implausible hit (AniList's
/// "Showa Era Stories") are skipped, the exact TVMaze title wins, and
/// the overlay comes from TVMaze's resolve.
#[test]
fn enrich_merges_every_provider() {
    let u = upstream();
    let dir = lain_store::test_dir("meta-remote");
    let store = Arc::new(lain_store::Store::open(&dir).unwrap());
    let reg = registry(&u, store);
    let item =
        CatalogItem { id: "item-1".into(), kind: "episode".into(), title: "Show Name".into(), ..Default::default() };
    let out = reg.invoke_provider(CAP_METADATA_ENRICH, ENRICH_ID, "enrich", json!({"item": item})).unwrap();
    assert_eq!(out["provider"], TVMAZE_ID);
    assert_eq!(out["remote_id"], "9001");
    assert_eq!(out["title"], "Show Name");
    assert_eq!(out["year"], 2021);
    assert_eq!(out["synopsis"], "A keeper returns to the lighthouse.");
    assert_eq!(u.hits("/kitsu/"), 1);
    assert_eq!(u.hits("/anilist"), 1);
    assert_eq!(u.hits("/tvmaze/search/shows"), 1);
    assert_eq!(u.hits("/tvmaze/shows/9001"), 1, "only the winner resolves");

    // Cached: a second enrichment of the same title asks nobody.
    let before = u.seen().len();
    let again = CatalogItem { id: "item-2".into(), ..item };
    reg.invoke_provider(CAP_METADATA_ENRICH, ENRICH_ID, "enrich", json!({"item": again})).unwrap();
    assert_eq!(u.seen().len(), before);

    // A movie: every network provider stays silent.
    let movie = CatalogItem { id: "m".into(), kind: "movie".into(), title: "Akira".into(), ..Default::default() };
    let e = reg.invoke_provider(CAP_METADATA_ENRICH, ENRICH_ID, "enrich", json!({"item": movie})).unwrap_err();
    assert_eq!(e.code, "not-found");
    assert_eq!(u.seen().len(), before);
    std::fs::remove_dir_all(dir).ok();
}

/// A private library's titles stay on the machine: its items are
/// enriched from the files next to them, and no network provider is
/// asked, not even for a title one of them knows exactly, nor when a
/// caller names one.
#[test]
fn a_private_library_is_enriched_from_local_files_only() {
    let u = upstream();
    let dir = lain_store::test_dir("meta-private");
    let store = Arc::new(lain_store::Store::open(&dir).unwrap());
    let lib = lain_api::Library { id: "lib-p".into(), private: true, owner: "u".into(), ..Default::default() };
    store.write(|tx| tx.put_json(lain_store::Bucket::Libraries, lib.id.as_bytes(), &lib)).unwrap();
    let reg = registry(&u, store);

    let media = dir.join("media").join("Show Name");
    std::fs::create_dir_all(&media).unwrap();
    std::fs::write(
        media.join("tvshow.nfo"),
        "<tvshow><title>Show Name (my cut)</title><year>2019</year><genre>Drama</genre><plot>From the folder.</plot></tvshow>",
    )
    .unwrap();
    let item = CatalogItem {
        id: "item-p".into(),
        library_id: "lib-p".into(),
        kind: "episode".into(),
        title: "Show Name".into(),
        file_path: media.join("Show Name.mkv").to_string_lossy().into(),
        ..Default::default()
    };
    let out = reg.invoke_provider(CAP_METADATA_ENRICH, ENRICH_ID, "enrich", json!({"item": item})).unwrap();
    assert_eq!(out["provider"], NFO_ID);
    assert_eq!((out["title"].as_str(), out["year"].as_i64()), (Some("Show Name (my cut)"), Some(2019)));
    assert_eq!(out["genres"], json!(["Drama"]));
    assert_eq!(out["synopsis"], "From the folder.");

    // Nothing next to the file: no overlay, rather than a network answer.
    let bare = CatalogItem {
        id: "item-q".into(),
        file_path: dir.join("Show Name.mkv").to_string_lossy().into(),
        ..item.clone()
    };
    let e = reg.invoke_provider(CAP_METADATA_ENRICH, ENRICH_ID, "enrich", json!({"item": bare})).unwrap_err();
    assert_eq!(e.code, "not-found");
    let e = reg
        .invoke_provider(CAP_METADATA_ENRICH, ENRICH_ID, "enrich", json!({"item": item, "only": TVMAZE_ID}))
        .unwrap_err();
    assert_eq!(e.code, "not-found");
    assert!(u.seen().is_empty(), "a private title reached the network: {:?}", u.seen());
    std::fs::remove_dir_all(dir).ok();
}

// ---- opt-in live smoke tests: one per provider ------------------------------

fn live(p: &dyn Provider, query: &str) {
    let list = search(p, query, "", 3);
    assert!(!list.is_empty(), "{}: no hits", p.id());
    let rec = resolve(p, &list[0].remote_id).unwrap();
    assert!(!rec.title.is_empty(), "{}: {rec:?}", p.id());
}

#[test]
#[ignore = "reaches the network"]
fn live_kitsu() {
    live(&Kitsu::new(client(), &Endpoints::public().kitsu), "Frieren");
}

#[test]
#[ignore = "reaches the network"]
fn live_anilist() {
    live(&AniList::new(client(), &Endpoints::public().anilist), "Frieren");
}

#[test]
#[ignore = "reaches the network"]
fn live_jikan() {
    live(&Jikan::new(client(), &Endpoints::public().jikan), "Frieren");
}

#[test]
#[ignore = "reaches the network"]
fn live_tvmaze() {
    live(&TvMaze::new(client(), &Endpoints::public().tvmaze), "Severance");
}
