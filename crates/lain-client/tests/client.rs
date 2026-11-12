//! `lain-client` against an in-process `lain-rs`: auth and sessions,
//! libraries (private unlock/lock), catalog paging, progress, the event
//! stream, the offline queue and its conflict rules, and the offline
//! store's limits and LRU eviction.

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use lain_client::types::{
    CatalogPage, ComicPages, IntegrationsUpdate, IntegrationsView, Library, LibraryPatch, ModulesView, NewLibrary,
    Palette, PluginsView, ProfilePatch, Progress, ProgressUpdate, RootsView, TranscodeSelection, User, UserPatch,
};
use lain_client::{
    CatalogQuery, Client, ClientConfig, ClientDb, ClientError, ConflictRule, DiskSpace, EventStream, FetchObserver,
    FileTokenStore, OfflineConfig, OfflineEntry, OfflineState, OfflineStore, ProgressSync, RecordOutcome, Resolution,
    SearchQuery, ServerEvent, Session, StreamConfig, StreamItem, TokenStore, Unlock,
};
use support::*;

fn upd(position_sec: f64, completed: bool) -> ProgressUpdate {
    ProgressUpdate { position_sec, duration_sec: 1440.0, completed }
}

const ITEM1: &str = "item-frieren-01";
const ITEM2: &str = "item-frieren-02";
const LIB: &str = "lib-e8bb66fd";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auth_session_and_account() {
    let root = temp("auth");
    let server = Server::start(&root.join("data")).await;
    let tokens = Arc::new(FileTokenStore::new(root.join("cfg/sessions.json")));
    let c = Client::new(ClientConfig::new(&server.url), tokens.clone()).unwrap();

    assert!(c.setup_status().await.unwrap().setup_required);
    assert_eq!(c.me().await.unwrap_err(), ClientError::Unauthorized);
    assert!(matches!(c.setup("admin", &pw(0)[..4]).await.unwrap_err(), ClientError::Api { status: 400, .. }));
    let admin = c.setup("admin", &pw(0)).await.unwrap();
    assert_eq!((admin.id.as_str(), admin.role.as_str()), ("user-admin", "admin"));
    assert!(!c.setup_status().await.unwrap().setup_required);
    assert_eq!(
        c.login("admin", &pw(4)).await.unwrap_err(),
        ClientError::LoginFailed { message: "invalid credentials".into() }
    );
    let me = c.login("admin", &pw(0)).await.unwrap();
    assert_eq!(me.username, "admin");
    let s = c.session().unwrap();
    assert_eq!((s.server.as_str(), s.user_id.as_str()), (server.url.as_str(), "user-admin"));
    assert_eq!(c.health().await.unwrap().status, "ok");

    // The session survives a restart of the client through the file.
    let again = Client::new(ClientConfig::new(&server.url), tokens.clone()).unwrap();
    assert_eq!(again.me().await.unwrap().id, "user-admin");
    let raw = raw_get(&server.url, &s.token, "/api/me").await;
    assert_mirrors::<User>(&raw);

    // The account routes.
    let u = c
        .update_profile(ProfilePatch {
            display_name: Some("  The   Admin ".into()),
            mascot: Some("moth".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!((u.profile.display_name.as_str(), u.profile.avatar.mascot.as_str()), ("The Admin", "moth"));
    assert_eq!(c.set_preferred_language(" JPN ").await.unwrap().preferred_language, "jpn");
    // D-280: the one language is both; the split fields change apart.
    let both = c.me().await.unwrap().track_preferences();
    assert_eq!(
        (both.audio_language.as_str(), both.subtitle_language.as_str(), both.subtitle_mode.as_str()),
        ("jpn", "jpn", "auto")
    );
    let u = c
        .update_preferences(&lain_api::PreferencesPatch {
            preferred_subtitle_language: Some("por".into()),
            preferred_subtitle_mode: Some("always".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let t = u.track_preferences();
    assert_eq!(
        (t.audio_language.as_str(), t.subtitle_language.as_str(), t.subtitle_mode.as_str()),
        ("jpn", "por", "always")
    );
    assert_eq!(u.preferred_language, "por", "older clients read the language the viewer reads");
    assert_mirrors::<User>(&raw_get(&server.url, &s.token, "/api/me").await);
    let refused = c
        .update_preferences(&lain_api::PreferencesPatch {
            preferred_subtitle_mode: Some("sometimes".into()),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("auto, always, forced or off"), "{refused}");
    assert!(c.mascots().await.unwrap().contains(&"moth".to_string()));
    assert!(!c.pin_status().await.unwrap());
    c.set_pin(&pin(0), &pw(0)).await.unwrap();
    assert!(c.pin_status().await.unwrap());
    c.clear_pin(&pw(0)).await.unwrap();
    assert!(!c.pin_status().await.unwrap());

    // Users (admin).
    let bob = c.create_user("bob", &pw(2), "").await.unwrap();
    assert_eq!(bob.role, "user");
    let users = raw_get(&server.url, &s.token, "/api/users").await;
    assert_eq!(assert_mirrors::<Vec<User>>(&users).len(), 2);
    let b = c.update_user("user-bob", UserPatch { disabled: Some(true), ..Default::default() }).await.unwrap();
    assert!(b.disabled);
    let bc = client(&server.url);
    assert_eq!(
        bc.login("bob", &pw(2)).await.unwrap_err(),
        ClientError::LoginFailed { message: "account disabled".into() }
    );
    c.update_user("user-bob", UserPatch { disabled: Some(false), ..Default::default() }).await.unwrap();
    bc.login("bob", &pw(2)).await.unwrap();
    assert!(matches!(bc.users().await.unwrap_err(), ClientError::Forbidden { .. }));

    // A password change ends the session; logging in again works.
    bc.change_password(&pw(2), &pw(3)).await.unwrap();
    assert_eq!(bc.me().await.unwrap_err(), ClientError::Unauthorized);
    bc.login("bob", &pw(3)).await.unwrap();
    assert_eq!(bc.me().await.unwrap().pwd_ver, 2);

    // Logout forgets the session here and in the file.
    c.logout().unwrap();
    assert_eq!(c.me().await.unwrap_err(), ClientError::Unauthorized);
    assert!(tokens.load(&server.url).unwrap().is_none());
    drop(server);
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn token_file_store() {
    let root = temp("tokens");
    let path = root.join("nested/dir/sessions.json");
    let store = FileTokenStore::new(&path);
    assert!(store.load("http://a:9360").unwrap().is_none());
    let s = |server: &str, token: &str| Session {
        server: server.into(),
        token: token.into(),
        user_id: "u".into(),
        username: "n".into(),
    };
    store.save(&s("http://a:9360", "tok-a")).unwrap();
    store.save(&s("http://b:9360", "tok-b")).unwrap();
    store.save(&s("http://a:9360", "tok-a2")).unwrap();
    // A second handle (another process) reads the same file.
    let other = FileTokenStore::new(&path);
    assert_eq!(other.load("http://a:9360").unwrap().unwrap().token, "tok-a2");
    assert_eq!(other.load("http://b:9360").unwrap().unwrap().token, "tok-b");
    assert!(!format!("{:?}", other.load("http://a:9360").unwrap()).contains("tok-a2"));
    other.clear("http://a:9360").unwrap();
    other.clear("http://nowhere").unwrap();
    assert!(store.load("http://a:9360").unwrap().is_none());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
        // A file opened up by hand is tightened on the next read.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        store.load("http://b:9360").unwrap();
        assert_eq!(mode(&path), 0o600);
    }
    // Corrupt or foreign files are reported, never overwritten silently.
    std::fs::write(&path, b"{\"format\":\"something-else\"}").unwrap();
    assert!(matches!(store.load("http://b:9360").unwrap_err(), ClientError::Store { .. }));
    assert!(store.save(&s("http://b:9360", "x")).is_err());
    std::fs::remove_dir_all(root).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn libraries_catalog_and_private() {
    let root = temp("libs");
    let server = Server::with_fixture(&root).await;
    let (admin, bob) = admin_and_bob(&server.url).await;
    let token = admin.session().unwrap().token;

    // The rooted fixture library and a path library.
    let lib = admin
        .create_library(NewLibrary {
            name: "Anime".into(),
            root: "media".into(),
            path: "Frieren".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!((lib.id.as_str(), lib.root_path.as_str()), (LIB, "Frieren"));
    let other = admin
        .create_library(NewLibrary {
            name: "Other".into(),
            path: root.join("media").to_string_lossy().into(),
            kind: "movie".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(other.kind, "movie");
    let raw = raw_get(&server.url, &token, "/api/libraries").await;
    assert_eq!(assert_mirrors::<Vec<Library>>(&raw).len(), 2);
    assert_eq!(admin.delete_library(&other.id).await.unwrap().items_removed, 0);
    assert!(matches!(admin.delete_library("lib-nope").await.unwrap_err(), ClientError::NotFound { .. }));
    assert!(matches!(bob.create_library(NewLibrary::default()).await.unwrap_err(), ClientError::Forbidden { .. }));
    let roots = raw_get(&server.url, &token, "/api/roots").await;
    assert_eq!(assert_mirrors::<RootsView>(&roots).roots[0].name, "media");
    assert!(admin.roots().await.unwrap().roots[0].available);

    // Catalog paging.
    let page = admin.catalog(CatalogQuery::default()).await.unwrap();
    assert_eq!((page.total, page.limit, page.offset, page.items.len()), (2, 50, 0, 2));
    assert_mirrors::<CatalogPage>(&raw_get(&server.url, &token, "/api/catalog").await);
    let p = admin.catalog(CatalogQuery { limit: 1, offset: 1, ..Default::default() }).await.unwrap();
    assert_eq!((p.total, p.limit, p.items[0].id.as_str()), (2, 1, ITEM2));
    assert!(admin.catalog(CatalogQuery { offset: 5, ..Default::default() }).await.unwrap().items.is_empty());
    let recent = admin.catalog(CatalogQuery { sort: "recent".into(), ..Default::default() }).await.unwrap();
    assert_eq!(recent.items[0].id, ITEM2);
    assert_eq!(
        admin.catalog(CatalogQuery { library_id: "lib-other".into(), ..Default::default() }).await.unwrap().total,
        0
    );
    let clamped = admin.catalog(CatalogQuery { limit: 900, offset: -5, ..Default::default() }).await.unwrap();
    assert_eq!((clamped.limit, clamped.offset), (500, 0));
    let item = admin.catalog_item(ITEM1).await.unwrap();
    assert_eq!((item.episode, item.root.as_str()), (1, "media"));
    assert!(item.file_path.ends_with("01 [1080p].mkv"));
    assert_eq!(admin.episodes(ITEM2).await.unwrap().iter().map(|i| i.episode).collect::<Vec<_>>(), vec![1, 2]);
    assert_eq!(admin.search(SearchQuery { q: "frieren".into(), ..Default::default() }).await.unwrap().total, 2);
    // A page of titles (D-283): one title, both of its files.
    let titles = admin
        .search(SearchQuery { q: "frieren".into(), group: "title".into(), limit: 1, ..Default::default() })
        .await
        .unwrap();
    assert_eq!((titles.total, titles.items.len(), titles.group.as_str()), (1, 2, "title"));
    assert_mirrors::<CatalogPage>(&raw_get(&server.url, &token, "/api/catalog?group=title").await);
    let titles = admin.catalog(CatalogQuery { group: "title".into(), ..Default::default() }).await.unwrap();
    assert_eq!((titles.total, titles.items.len(), titles.group.as_str()), (1, 2, "title"));
    // An episode's own picture (D-284): the show's poster kept beside the
    // first episode is its cover, never its still.
    let (dir, name) = (root.join("media/Frieren"), "[Fansub-A] Frieren - 01 [1080p]");
    std::fs::write(dir.join(format!("{name}-poster.png")), b"\x89PNG\r\n\x1a\n").unwrap();
    std::fs::write(dir.join(format!("{name}-thumb.jpg")), b"GIF89a-thumb").unwrap();
    assert!(admin.thumbnail(ITEM1).await.unwrap().starts_with(b"\x89PNG"));
    assert_eq!(admin.still(ITEM1).await.unwrap(), b"GIF89a-thumb");
    assert!(matches!(admin.catalog_item("no-such-item").await.unwrap_err(), ClientError::NotFound { .. }));

    // Private: hidden from bob (not a member) and locked for the admin.
    admin.put_progress(ITEM1, upd(30.0, false)).await.unwrap();
    let lib = admin.update_library(LIB, LibraryPatch { private: Some(true), ..Default::default() }).await.unwrap();
    assert_eq!((lib.private, lib.owner.as_str(), lib.locked), (true, "user-admin", Some(true)));
    assert!(bob.libraries().await.unwrap().is_empty());
    assert_eq!(bob.catalog(CatalogQuery::default()).await.unwrap().total, 0);
    assert!(matches!(
        bob.unlock_library(LIB, Unlock::Password(pw(2))).await.unwrap_err(),
        ClientError::NotFound { .. }
    ));
    assert_eq!(admin.libraries().await.unwrap()[0].locked, Some(true));
    assert_eq!(admin.catalog(CatalogQuery::default()).await.unwrap().total, 0);
    assert!(matches!(admin.catalog_item(ITEM1).await.unwrap_err(), ClientError::NotFound { .. }));
    assert!(admin.continue_watching().await.unwrap().is_empty());
    assert!(matches!(
        admin.unlock_library(LIB, Unlock::Password(pw(4))).await.unwrap_err(),
        ClientError::Forbidden { .. }
    ));
    let lib = admin.unlock_library(LIB, Unlock::Password(pw(0))).await.unwrap();
    assert_eq!(lib.locked, Some(false));
    assert_eq!(admin.catalog(CatalogQuery::default()).await.unwrap().total, 2);
    assert_eq!(admin.continue_watching().await.unwrap()[0].position_sec, 30.0);
    assert_eq!(admin.lock_library(LIB).await.unwrap().locked, Some(true));
    assert_eq!(admin.catalog(CatalogQuery::default()).await.unwrap().total, 0);
    // A PIN unlocks too; repeated failures are rate limited.
    admin.set_pin(&pin(0), &pw(0)).await.unwrap();
    admin.unlock_library(LIB, Unlock::Pin(pin(0))).await.unwrap();
    assert_eq!(admin.catalog(CatalogQuery::default()).await.unwrap().total, 2);
    admin.lock_library(LIB).await.unwrap();
    for n in 1..=5 {
        assert!(matches!(
            admin.unlock_library(LIB, Unlock::Pin(pin(n))).await.unwrap_err(),
            ClientError::Forbidden { .. }
        ));
    }
    assert!(matches!(
        admin.unlock_library(LIB, Unlock::Pin(pin(0))).await.unwrap_err(),
        ClientError::RateLimited { .. }
    ));
    // Made public again, everyone sees it.
    let lib = admin.update_library(LIB, LibraryPatch { private: Some(false), ..Default::default() }).await.unwrap();
    assert_eq!((lib.private, lib.locked), (false, None));
    assert_eq!(bob.catalog(CatalogQuery::default()).await.unwrap().total, 2);
    drop(server);
    std::fs::remove_dir_all(root).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn not_ported_and_module_disabled() {
    let root = temp("ported");
    let server = Server::with_fixture(&root).await;
    let (admin, _bob) = admin_and_bob(&server.url).await;
    admin
        .create_library(NewLibrary {
            name: "Anime".into(),
            root: "media".into(),
            path: "Frieren".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let url = admin.stream_url(ITEM1).unwrap();
    assert!(url.starts_with(&format!("{}/api/items/{ITEM1}/stream?token=", server.url)));

    // The registry views mirror the wire.
    let token = admin.session().unwrap().token;
    let modules = assert_mirrors::<ModulesView>(&raw_get(&server.url, &token, "/api/modules").await);
    assert_eq!(modules.format, "lain.modules@1");
    assert_eq!(modules, admin.modules().await.unwrap());
    let plugins = assert_mirrors::<PluginsView>(&raw_get(&server.url, &token, "/api/plugins").await);
    assert!(plugins.composition.iter().any(|b| b.capability == "lain.catalog.read@1"));
    let swap = admin
        .swap_providers(lain_client::types::SwapRequest {
            capability: "lain.catalog.read@1".into(),
            providers: vec!["x".into()],
            generation: 999,
        })
        .await
        .unwrap_err();
    assert!(matches!(swap, ClientError::Api { status: 409, ref code, .. } if code == "stale-generation"), "{swap:?}");

    // A disabled module's routes answer ModuleDisabled.
    std::fs::write(
        server.dir.join("modules.json"),
        br#"{"format":"lain.modules@1","modules":[{"id":"watch","enabled":false}]}"#,
    )
    .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while admin.modules().await.unwrap().modules.iter().any(|m| m.id == "watch" && m.enabled) {
        assert!(std::time::Instant::now() < deadline, "modules.json was not picked up");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        admin.playback_plan(ITEM1, "mpv").await.unwrap_err(),
        ClientError::ModuleDisabled { module: "watch".into() }
    );
    drop(server);
    std::fs::remove_dir_all(root).ok();
}

/// Overlays from the network metadata providers (D-201) through the
/// typed client, against the recorded upstream answers: merge-many
/// skips the failing and the unrelated providers, the exact TVMaze
/// title wins; one named provider without a hit is a 404.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn enrich_from_network_providers() {
    let root = temp("remote-enrich");
    let media = root.join("media/Show");
    std::fs::create_dir_all(&media).unwrap();
    std::fs::write(media.join("Show.Name.S01E02.1080p.WEB-DL.mkv"), b"x").unwrap();
    let upstream = lain_conformance::upstream::Upstream::start(&lain_conformance::upstream::dir(), 0).unwrap();
    let endpoints = lain_metadata::Endpoints::under(&upstream.url);
    let server = Server::start_with(&root.join("data"), move |c| {
        c.offline = false;
        c.auto_enrich = false;
        c.watch = false;
        c.metadata_endpoints = endpoints;
    })
    .await;
    let (admin, bob) = admin_and_bob(&server.url).await;
    admin
        .create_library(NewLibrary {
            name: "Shows".into(),
            path: root.join("media").to_string_lossy().into(),
            ..Default::default()
        })
        .await
        .unwrap();
    admin.start_scan().await.unwrap();
    for _ in 0..200 {
        if admin.scan_status().await.unwrap().state == "done" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let item = bob.catalog(CatalogQuery::default()).await.unwrap().items[0].clone();
    assert_eq!(item.title, "Show Name");
    let miss = admin.enrich(&item.id, "lain-metadata-kitsu").await.unwrap_err();
    assert!(matches!(miss, ClientError::NotFound { .. }), "{miss:?}");
    let e = admin.enrich(&item.id, "").await.unwrap();
    assert_eq!(
        (e.provider.as_str(), e.remote_id.as_str(), e.title.as_str(), e.year),
        ("lain-metadata-tvmaze", "9001", "Show Name", 2021)
    );
    assert_eq!(e.synopsis, "A keeper returns to the lighthouse.");
    assert_eq!(bob.enrichment(&item.id).await.unwrap(), e);
    assert_eq!(upstream.hits("/tvmaze/shows/9001"), 1);
    assert_eq!(upstream.hits("/jikan/"), 6, "both enrichments asked Jikan, retried its 503 twice, then skipped it");
    drop(server);
    std::fs::remove_dir_all(root).ok();
}

/// Scan, identified catalog, sidecars, overlays and item delete through
/// the typed client, with the scan's events on the stream.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn catalog_population() {
    let root = temp("population");
    let media = root.join("media/Frieren");
    std::fs::create_dir_all(&media).unwrap();
    let ep = media.join("[Fansub-A] Frieren - 01 [1080p].mkv");
    std::fs::write(&ep, b"x").unwrap();
    std::fs::write(media.join("[Fansub-A] Frieren - 01 [1080p].en.srt"), b"1\n00:00:01,000 --> 00:00:02,000\nHi\n")
        .unwrap();
    std::fs::write(media.join("tvshow.nfo"), b"<tvshow><title>Sousou no Frieren</title><year>2023</year></tvshow>")
        .unwrap();
    let server = Server::start(&root.join("data")).await;
    let (admin, bob) = admin_and_bob(&server.url).await;
    let lib = admin
        .create_library(NewLibrary {
            name: "Anime".into(),
            path: root.join("media").to_string_lossy().into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let stream = EventStream::new(bob.clone(), StreamConfig::default());
    next_matching(&stream, "ready", |i| matches!(i, StreamItem::Connected(_))).await;

    assert_eq!(bob.scan_status().await.unwrap().state, "idle");
    assert!(matches!(bob.start_scan().await.unwrap_err(), ClientError::Forbidden { .. }));
    admin.start_scan().await.unwrap();
    match next_matching(
        &stream,
        "scan-state done",
        |i| matches!(i, StreamItem::Event(ServerEvent::ScanState(s)) if s.state == "done"),
    )
    .await
    {
        StreamItem::Event(ServerEvent::ScanState(s)) => {
            assert_eq!(s.stats.as_ref().map(|s| s.identified), Some(1));
        }
        _ => unreachable!(),
    }
    let st = bob.scan_status().await.unwrap();
    assert_eq!((st.state.as_str(), st.trigger.as_str()), ("done", "manual"));
    let stats = st.stats.unwrap();
    assert_eq!((stats.candidates, stats.identified, stats.dirs), (1, 1, 2));
    let token = admin.session().unwrap().token;
    assert_mirrors::<lain_client::types::ScanStatus>(&raw_get(&server.url, &token, "/api/library/scan").await);

    let page = bob.catalog(CatalogQuery::default()).await.unwrap();
    let item = page.items[0].clone();
    assert_eq!(
        (item.title.as_str(), item.kind.as_str(), item.episode, item.library_id.as_str()),
        ("Frieren", "episode", 1, lib.id.as_str())
    );
    let sc = bob.sidecars(&item.id).await.unwrap();
    assert_eq!((sc.len(), sc[0].language.as_str(), sc[0].index), (1, "eng", 0));
    assert_eq!(bob.sidecar_vtt(&item.id, 0).await.unwrap(), "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nHi\n\n");
    assert!(matches!(bob.sidecar_vtt(&item.id, 3).await.unwrap_err(), ClientError::NotFound { .. }));

    // Offline (the harness default), only the NFO provider answers.
    let e = admin.enrich(&item.id, "").await.unwrap();
    assert_eq!((e.title.as_str(), e.year, e.provider.as_str()), ("Sousou no Frieren", 2023, "lain-metadata-nfo"));
    assert_eq!(bob.enrichment(&item.id).await.unwrap(), e);
    assert_eq!(bob.enrichments(&[item.id.clone(), "nope".into()]).await.unwrap(), vec![e.clone()]);
    assert!(matches!(bob.enrich(&item.id, "").await.unwrap_err(), ClientError::Forbidden { .. }));
    admin.delete_enrichment(&item.id).await.unwrap();
    assert!(matches!(bob.enrichment(&item.id).await.unwrap_err(), ClientError::NotFound { .. }));

    let dirs = admin.browse(&root.join("media").to_string_lossy()).await.unwrap();
    assert_eq!(dirs.dirs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), vec!["Frieren"]);
    let gone = admin.delete_item(&item.id).await.unwrap();
    assert!(gone.missing && !ep.exists());
    assert!(bob.catalog_item(&item.id).await.unwrap().missing);
    drop(stream);
    drop(server);
    std::fs::remove_dir_all(root).ok();
}

async fn next_matching(stream: &EventStream, what: &str, f: impl Fn(&StreamItem) -> bool) -> StreamItem {
    let wait = async {
        loop {
            let item = stream.next().await.expect("stream closed");
            if f(&item) {
                return item;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(10), wait).await.unwrap_or_else(|_| panic!("no {what} within 10s"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn progress_and_events() {
    let root = temp("events");
    let server = Server::start(&root.join("data")).await;
    let (admin, bob) = admin_and_bob(&server.url).await;

    // Progress put/get/continue.
    assert_eq!(admin.progress("item-one").await.unwrap(), Progress::default());
    let p = admin
        .put_progress("item-one", ProgressUpdate { position_sec: 12.5, duration_sec: 1440.0, completed: false })
        .await
        .unwrap();
    assert_eq!((p.item_id.as_str(), p.user_id.as_str(), p.position_sec), ("item-one", "user-admin", 12.5));
    assert!(p.updated_at > 0);
    assert_eq!(admin.progress("item-one").await.unwrap(), p);
    assert_eq!(bob.progress("item-one").await.unwrap(), Progress::default());
    admin.put_progress("item-two", upd(1.0, false)).await.unwrap();
    let cont = admin.continue_watching().await.unwrap();
    assert_eq!(cont.iter().map(|p| p.item_id.as_str()).collect::<Vec<_>>(), vec!["item-one", "item-two"]);

    // The stream: ready, then this user's progress (and not bob's).
    let proxy = Proxy::start(server.addr).await;
    let via = client(&proxy.url);
    via.login("admin", &pw(0)).await.unwrap();
    let cfg = StreamConfig { initial_backoff_ms: 50, max_backoff_ms: 400, idle_timeout_ms: 2_000 };
    let stream = EventStream::new(via.clone(), cfg);
    match next_matching(&stream, "ready", |i| matches!(i, StreamItem::Connected(_))).await {
        StreamItem::Connected(r) => assert_eq!(r.server, "lain-rs"),
        _ => unreachable!(),
    }
    bob.put_progress("item-bob", upd(7.0, false)).await.unwrap();
    admin.put_progress("item-admin", upd(5.0, false)).await.unwrap();
    match next_matching(&stream, "progress-changed", |i| {
        matches!(i, StreamItem::Event(ServerEvent::ProgressChanged(_)))
    })
    .await
    {
        StreamItem::Event(ServerEvent::ProgressChanged(e)) => {
            assert_eq!(
                (e.item_id.as_str(), e.progress.position_sec, e.progress.user_id.as_str()),
                ("item-admin", 5.0, "user-admin")
            )
        }
        _ => unreachable!(),
    }
    let lib = admin
        .create_library(NewLibrary { name: "Ev".into(), path: root.to_string_lossy().into(), ..Default::default() })
        .await
        .unwrap();
    match next_matching(&stream, "library-changed", |i| matches!(i, StreamItem::Event(ServerEvent::LibraryChanged(_))))
        .await
    {
        StreamItem::Event(ServerEvent::LibraryChanged(e)) => {
            assert_eq!((e.library_id, e.action.as_str()), (lib.id, "created"))
        }
        _ => unreachable!(),
    }

    // The network drops: the stream says so, retries, and reconnects.
    proxy.set_online(false);
    next_matching(&stream, "reconnecting", |i| matches!(i, StreamItem::Reconnecting { .. })).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    proxy.set_online(true);
    next_matching(&stream, "ready again", |i| matches!(i, StreamItem::Connected(_))).await;
    admin.put_progress("item-admin", upd(6.0, false)).await.unwrap();
    next_matching(
        &stream,
        "progress after reconnect",
        |i| matches!(i, StreamItem::Event(ServerEvent::ProgressChanged(e)) if e.progress.position_sec == 6.0),
    )
    .await;

    // The password changes elsewhere: the server ends the stream at the
    // next keep-alive, the client asks for a login, and a fresh login
    // through the shared session resumes it.
    admin.change_password(&pw(0), &pw(1)).await.unwrap();
    next_matching(&stream, "auth required", |i| matches!(i, StreamItem::AuthRequired)).await;
    via.login("admin", &pw(1)).await.unwrap();
    next_matching(&stream, "ready after login", |i| matches!(i, StreamItem::Connected(_))).await;
    stream.close().await;
    assert!(stream.next().await.is_none());
    drop(server);
    std::fs::remove_dir_all(root).ok();
}

struct Sync {
    sync: ProgressSync,
    proxy: Proxy,
    other: Client,
}

/// Device A (through a proxy that can go offline) and device B (direct),
/// both logged in as admin.
async fn two_devices(server: &Server, root: &std::path::Path) -> Sync {
    let admin = client(&server.url);
    admin.setup("admin", &pw(0)).await.unwrap();
    admin.login("admin", &pw(0)).await.unwrap();
    let proxy = Proxy::start(server.addr).await;
    let a = client(&proxy.url);
    a.login("admin", &pw(0)).await.unwrap();
    let db = Arc::new(ClientDb::open(&root.join("a/client.redb")).unwrap());
    Sync { sync: ProgressSync::new(a, db), proxy, other: admin }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offline_queue_flushes_when_online() {
    let root = temp("queue");
    let server = Server::start(&root.join("data")).await;
    let d = two_devices(&server, &root).await;
    let s = &d.sync;

    // Online: straight to the server.
    assert!(matches!(s.record("ep", upd(10.0, false)).await.unwrap(), RecordOutcome::Synced { .. }));
    assert!(s.queued().unwrap().is_empty());

    // Offline: queued, coalesced (R6), and readable locally.
    d.proxy.set_online(false);
    assert!(matches!(s.record("ep", upd(20.0, false)).await.unwrap(), RecordOutcome::Queued { .. }));
    assert!(matches!(s.record("ep", upd(1440.0, true)).await.unwrap(), RecordOutcome::Queued { .. }));
    assert!(matches!(s.record("ep", upd(30.0, false)).await.unwrap(), RecordOutcome::Queued { .. }));
    assert!(matches!(s.record("ep2", upd(5.0, false)).await.unwrap(), RecordOutcome::Queued { .. }));
    let q = s.queued().unwrap();
    assert_eq!(q.len(), 2);
    assert_eq!((q[0].item_id.as_str(), q[0].update), ("ep", upd(30.0, true)));
    assert_eq!(q[0].base.as_ref().unwrap().position_sec, 10.0);
    assert_eq!(s.fetch("ep").await.unwrap().position_sec, 30.0);
    let r = s.flush().await.unwrap();
    assert!(r.items.is_empty() && r.remaining == 2 && r.stopped.as_ref().is_some_and(|e| e.is_offline()), "{r:?}");
    assert!(s.record("ep", upd(f64::NAN, false)).await.is_err());

    // Back online: the queue drains.
    d.proxy.set_online(true);
    let r = s.flush().await.unwrap();
    assert_eq!((r.items.len(), r.remaining, r.stopped.clone()), (2, 0, None), "{r:?}");
    assert!(r.items.iter().all(|i| i.rule == ConflictRule::FastForward && i.resolution == Resolution::Pushed));
    let srv = d.other.progress("ep").await.unwrap();
    assert_eq!((srv.position_sec, srv.completed), (30.0, true));
    assert_eq!(d.other.progress("ep2").await.unwrap().position_sec, 5.0);

    // With a record queued, a new online write goes through the rules.
    d.proxy.set_online(false);
    s.record("ep3", upd(1.0, false)).await.unwrap();
    d.proxy.set_online(true);
    assert!(matches!(s.record("ep3", upd(2.0, false)).await.unwrap(), RecordOutcome::Synced { .. }));
    assert!(s.queued().unwrap().is_empty());
    assert_eq!(d.other.progress("ep3").await.unwrap().position_sec, 2.0);

    // A logged-out device queues too and flushes after a login.
    d.proxy.set_online(false);
    s.record("ep4", upd(9.0, false)).await.unwrap();
    d.proxy.set_online(true);
    d.other.change_password(&pw(0), &pw(1)).await.unwrap();
    let r = s.flush().await.unwrap();
    assert_eq!((r.remaining, r.stopped), (1, Some(ClientError::Unauthorized)));
    drop(server);
    std::fs::remove_dir_all(root).ok();
}

/// One conflict scenario: A records `base` online (or only reads), goes
/// offline, B writes `theirs` (unless None), A records `mine`, A comes
/// back and flushes.
async fn scenario(
    d: &Sync,
    item: &'static str,
    base: Option<ProgressUpdate>,
    theirs: Option<ProgressUpdate>,
    mine: ProgressUpdate,
) -> (ConflictRule, Resolution, Progress) {
    let (s, b) = (&d.sync, &d.other);
    if let Some(bp) = base {
        s.record(item, bp).await.unwrap();
    } else {
        s.fetch(item).await.unwrap();
    }
    d.proxy.set_online(false);
    if let Some(t) = theirs {
        b.put_progress(item, t).await.unwrap();
    }
    assert!(matches!(s.record(item, mine).await.unwrap(), RecordOutcome::Queued { .. }));
    d.proxy.set_online(true);
    let r = s.flush().await.unwrap();
    assert_eq!(r.items.len(), 1, "{r:?}");
    let it = r.items[0].clone();
    (it.rule, it.resolution, b.progress(item).await.unwrap())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn conflict_rules() {
    let root = temp("conflicts");
    let server = Server::start(&root.join("data")).await;
    let d = two_devices(&server, &root).await;
    let (s, b) = (&d.sync, &d.other);

    // R1: nobody else wrote; even a rewind is pushed.
    let (rule, res, p) = scenario(&d, "r1", Some(upd(50.0, false)), None, upd(5.0, false)).await;
    assert_eq!((rule, res, p.position_sec), (ConflictRule::FastForward, Resolution::Pushed, 5.0));
    // R2: a completion beats a partial position, either way round.
    let (rule, res, p) = scenario(&d, "r2a", Some(upd(10.0, false)), Some(upd(70.0, false)), upd(1440.0, true)).await;
    assert_eq!((rule, res, p.completed), (ConflictRule::CompletionWins, Resolution::Pushed, true));
    let (rule, res, p) = scenario(&d, "r2b", Some(upd(10.0, false)), Some(upd(1440.0, true)), upd(99.0, false)).await;
    assert_eq!(
        (rule, res, p.completed, p.position_sec),
        (ConflictRule::CompletionWins, Resolution::KeptServer, true, 1440.0)
    );
    // R3: the further position wins; a tie keeps the server's.
    let (rule, res, p) = scenario(&d, "r3a", Some(upd(10.0, false)), Some(upd(40.0, false)), upd(80.0, false)).await;
    assert_eq!((rule, res, p.position_sec), (ConflictRule::FurtherPositionWins, Resolution::Pushed, 80.0));
    let (rule, res, p) = scenario(&d, "r3b", Some(upd(10.0, false)), Some(upd(40.0, false)), upd(20.0, false)).await;
    assert_eq!((rule, res, p.position_sec), (ConflictRule::FurtherPositionWins, Resolution::KeptServer, 40.0));
    let (_, res, _) = scenario(&d, "r3c", Some(upd(10.0, false)), Some(upd(40.0, false)), upd(40.0, false)).await;
    assert_eq!(res, Resolution::KeptServer);
    // R4: both completed keeps the server's record.
    let (rule, res, p) = scenario(&d, "r4", Some(upd(10.0, false)), Some(upd(1400.0, true)), upd(1440.0, true)).await;
    assert_eq!((rule, res, p.position_sec), (ConflictRule::BothCompleted, Resolution::KeptServer, 1400.0));
    // R5: a pushed record without a duration keeps the server's.
    let (_, res, p) = scenario(
        &d,
        "r5",
        Some(upd(10.0, false)),
        None,
        ProgressUpdate { position_sec: 20.0, duration_sec: 0.0, completed: false },
    )
    .await;
    assert_eq!((res, p.duration_sec), (Resolution::Pushed, 1440.0));
    // No base at all and a record on the server: treated as concurrent.
    b.put_progress("nobase", upd(40.0, false)).await.unwrap();
    d.proxy.set_online(false);
    s.record("nobase", upd(20.0, false)).await.unwrap();
    d.proxy.set_online(true);
    let r = s.flush().await.unwrap();
    assert_eq!((r.items[0].rule, r.items[0].resolution), (ConflictRule::FurtherPositionWins, Resolution::KeptServer));

    // A `progress-changed` event moves the base: B's write was seen, so
    // A's later offline rewind counts as a fast-forward over it.
    s.record("seen", upd(10.0, false)).await.unwrap();
    let theirs = b.put_progress("seen", upd(40.0, false)).await.unwrap();
    s.observe(&ServerEvent::ProgressChanged(lain_client::types::ProgressChangedEvent {
        item_id: "seen".into(),
        progress: theirs,
    }))
    .unwrap();
    d.proxy.set_online(false);
    s.record("seen", upd(20.0, false)).await.unwrap();
    d.proxy.set_online(true);
    let r = s.flush().await.unwrap();
    assert_eq!((r.items[0].rule, r.items[0].resolution), (ConflictRule::FastForward, Resolution::Pushed));
    assert_eq!(b.progress("seen").await.unwrap().position_sec, 20.0);
    drop(server);
    std::fs::remove_dir_all(root).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_record_is_dropped() {
    // R7, against a stub whose progress writes can fail on demand.
    let root = temp("refused");
    let stub = stub(Vec::new()).await;
    let s = ProgressSync::new(stub_client(&stub.url), Arc::new(ClientDb::open(&root.join("client.redb")).unwrap()));
    stub.put_status.store(400, std::sync::atomic::Ordering::SeqCst);
    // Not queued: a refusal of a direct write is the caller's error.
    assert!(matches!(s.record("x", upd(1.0, false)).await.unwrap_err(), ClientError::Api { status: 400, .. }));
    assert!(s.queued().unwrap().is_empty());
    // A 5xx queues; a later refusal drops the record and reports it.
    stub.put_status.store(503, std::sync::atomic::Ordering::SeqCst);
    assert!(matches!(s.record("x", upd(2.0, false)).await.unwrap(), RecordOutcome::Queued { .. }));
    assert!(matches!(s.record("y", upd(3.0, false)).await.unwrap(), RecordOutcome::Queued { .. }));
    let r = s.flush().await.unwrap();
    assert!(matches!(r.stopped, Some(ClientError::Api { status: 503, .. })) && r.remaining == 2, "{r:?}");
    stub.put_status.store(400, std::sync::atomic::Ordering::SeqCst);
    let r = s.flush().await.unwrap();
    assert_eq!(r.items.len(), 2, "{r:?}");
    assert!(r.items.iter().all(|i| i.rule == ConflictRule::Refused && i.resolution == Resolution::Refused));
    assert_eq!((r.remaining, r.stopped), (0, None));
    std::fs::remove_dir_all(root).ok();
}

struct FakeDisk(Mutex<Option<u64>>);

impl DiskSpace for FakeDisk {
    fn free_bytes(&self, _: &std::path::Path) -> Option<u64> {
        *self.0.lock().unwrap()
    }
}

#[derive(Default)]
struct Seen {
    evicted: Mutex<Vec<String>>,
    last: Mutex<(u64, u64)>,
}

impl FetchObserver for Seen {
    fn progress(&self, done: u64, total: u64) {
        *self.last.lock().unwrap() = (done, total);
    }
    fn evicted(&self, e: &OfflineEntry) {
        self.evicted.lock().unwrap().push(e.item_id.clone());
    }
}

fn catalog_item(id: &str) -> lain_client::types::CatalogItem {
    lain_client::types::CatalogItem {
        id: id.into(),
        title: format!("Title {id}"),
        rel_path: format!("Show/{id}.mkv"),
        kind: "episode".into(),
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offline_store_limits_and_lru() {
    let root = temp("lru");
    let files: Vec<StubFile> = ["a", "b", "c", "d", "e", "f"]
        .iter()
        .map(|id| StubFile { item: id.to_string(), bytes: vec![id.as_bytes()[0]; 1000], cut_after: None })
        .collect();
    let stub = stub(files).await;
    let c = stub_client(&stub.url);
    let server = c.server();
    let db = Arc::new(ClientDb::open(&root.join("client.redb")).unwrap());
    let disk = Arc::new(FakeDisk(Mutex::new(None)));
    let mut cfg = OfflineConfig::new(&root.join("offline").to_string_lossy());
    cfg.max_bytes = 3000;
    assert_eq!((OfflineConfig::new("/x").max_bytes, OfflineConfig::new("/x").min_free_bytes), (10 << 30, 5 << 30));
    assert!(OfflineConfig::new("/x").evict_watched);
    let store = OfflineStore::with_disk(cfg.clone(), db.clone(), disk.clone()).unwrap();
    assert!(OfflineStore::open(OfflineConfig::new("relative/dir"), db.clone()).is_err());

    let seen = Arc::new(Seen::default());
    for id in ["a", "b", "c"] {
        store.add(&server, &catalog_item(id)).unwrap();
        let e = store.fetch(&c, id, Some(seen.clone())).await.unwrap();
        assert_eq!((e.state, e.bytes, e.size), (OfflineState::Done, 1000, 1000));
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(*seen.last.lock().unwrap(), (1000, 1000));
    assert_eq!(store.used().unwrap(), 3000);
    let pa = store.local_path(&server, "a").unwrap().unwrap();
    assert!(pa.ends_with("a.mkv") && std::fs::read(&pa).unwrap() == vec![b'a'; 1000]);
    // Re-adding returns the entry; names stay unique.
    assert_eq!(store.add(&server, &catalog_item("a")).unwrap().state, OfflineState::Done);

    // Full and nothing watched: refused with a stable code, no eviction.
    store.add(&server, &catalog_item("d")).unwrap();
    let err = store.fetch(&c, "d", None).await.unwrap_err();
    assert!(matches!(err, ClientError::NoRoom { ref code, .. } if code == "quota-exceeded"), "{err:?}");
    assert_eq!(store.get(&server, "d").unwrap().unwrap().code, "quota-exceeded");

    // a and b watched, a used more recently: b goes first (LRU).
    store.set_watched(&server, "a", true).unwrap();
    store.set_watched(&server, "b", true).unwrap();
    tokio::time::sleep(Duration::from_millis(5)).await;
    store.touch(&server, "a").unwrap();
    store.resume(&server, "d").unwrap();
    let e = store.fetch(&c, "d", Some(seen.clone())).await.unwrap();
    assert_eq!(e.state, OfflineState::Done);
    assert_eq!(*seen.evicted.lock().unwrap(), vec!["b".to_string()]);
    assert!(store.get(&server, "b").unwrap().is_none());
    assert!(store.local_path(&server, "a").unwrap().is_some());

    // Unsynced progress protects a watched copy: a gets a queued
    // record (the server answers 503), so c goes instead, though a is
    // older.
    store.set_watched(&server, "c", true).unwrap();
    tokio::time::sleep(Duration::from_millis(5)).await;
    store.touch(&server, "c").unwrap();
    let sync = ProgressSync::new(c.clone(), db.clone());
    stub.put_status.store(503, std::sync::atomic::Ordering::SeqCst);
    assert!(matches!(sync.record("a", upd(10.0, false)).await.unwrap(), RecordOutcome::Queued { .. }));
    store.add(&server, &catalog_item("e")).unwrap();
    store.fetch(&c, "e", Some(seen.clone())).await.unwrap();
    assert_eq!(*seen.evicted.lock().unwrap(), vec!["b".to_string(), "c".to_string()]);
    assert!(store.local_path(&server, "a").unwrap().is_some());
    assert!(matches!(store.remove(&server, "a", false).unwrap_err(), ClientError::Invalid { .. }));

    // Eviction off: refused even with a watched copy around.
    store.set_watched(&server, "d", true).unwrap();
    let strict =
        OfflineStore::with_disk(OfflineConfig { evict_watched: false, ..cfg.clone() }, db.clone(), disk.clone())
            .unwrap();
    strict.add(&server, &catalog_item("f")).unwrap();
    let err = strict.fetch(&c, "f", None).await.unwrap_err();
    assert!(matches!(err, ClientError::NoRoom { ref code, .. } if code == "quota-exceeded"), "{err:?}");
    assert!(store.local_path(&server, "d").unwrap().is_some());

    // The free-space floor, with no byte cap.
    let floor = OfflineStore::with_disk(
        OfflineConfig { max_bytes: 0, evict_watched: false, ..cfg.clone() },
        db.clone(),
        disk.clone(),
    )
    .unwrap();
    *disk.0.lock().unwrap() = Some(cfg.min_free_bytes + 500);
    floor.resume(&server, "f").unwrap();
    let err = floor.fetch(&c, "f", None).await.unwrap_err();
    assert!(matches!(err, ClientError::NoRoom { ref code, .. } if code == "disk-full"), "{err:?}");
    *disk.0.lock().unwrap() = None;
    floor.resume(&server, "f").unwrap();
    assert_eq!(floor.fetch(&c, "f", None).await.unwrap().state, OfflineState::Done);

    // The server says nothing is completed: watched flags follow, except
    // on a copy with unsynced progress.
    assert_eq!(store.refresh_watched(&c).await.unwrap(), 3);
    assert!(!store.get(&server, "d").unwrap().unwrap().watched);
    assert!(store.get(&server, "a").unwrap().unwrap().watched);

    // Cleanup: an orphan part, a finished file deleted by hand, and
    // watched copies on request (a keeps its unsynced progress).
    std::fs::write(root.join("offline/.parts/stray.part"), b"xx").unwrap();
    std::fs::remove_file(store.local_path(&server, "d").unwrap().unwrap()).unwrap();
    let rep = store.gc(true).unwrap();
    assert_eq!((rep.orphan_parts, rep.vanished, rep.evicted.len()), (1, 1, 0), "{rep:?}");
    assert!(store.get(&server, "a").unwrap().is_some());
    assert_eq!(store.remove(&server, "a", true).unwrap(), 1000);
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offline_fetch_resumes() {
    let root = temp("resume");
    let full: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
    let stub = stub(vec![StubFile { item: "r".into(), bytes: full.clone(), cut_after: Some(400) }]).await;
    let c = stub_client(&stub.url);
    let db = Arc::new(ClientDb::open(&root.join("client.redb")).unwrap());
    let store = OfflineStore::open(OfflineConfig::new(&root.join("offline").to_string_lossy()), db).unwrap();
    store.add(&c.server(), &catalog_item("r")).unwrap();
    // The connection drops after 400 bytes: paused, part kept.
    assert!(store.fetch(&c, "r", None).await.unwrap_err().is_offline());
    let e = store.get(&c.server(), "r").unwrap().unwrap();
    assert_eq!((e.state, e.bytes, e.validator.as_str()), (OfflineState::Paused, 400, "\"v-r\""));
    // The next fetch asks for the rest (If-Range) and completes.
    let e = store.fetch(&c, "r", None).await.unwrap();
    assert_eq!((e.state, e.size), (OfflineState::Done, 1000));
    assert_eq!(std::fs::read(store.local_path(&c.server(), "r").unwrap().unwrap()).unwrap(), full);
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn core_admin_and_read() {
    let root = temp("admin-read");
    let server = Server::with_read_fixture(&root).await;
    let (admin, bob) = admin_and_bob(&server.url).await;
    let token = admin.session().unwrap().token;

    // The theme needs no session.
    let anon = client(&server.url);
    let palette = anon.theme().await.unwrap();
    assert!(palette.background.starts_with('#') && !palette.source.is_empty());
    assert_eq!(palette, assert_mirrors::<Palette>(&raw_get(&server.url, &token, "/api/theme").await));

    // Integrations: the secret is stored, never returned.
    let v = admin
        .set_integrations(IntegrationsUpdate {
            anilist_client_id: Some("4242".into()),
            anilist_client_secret: Some("s3cret".into()),
        })
        .await
        .unwrap();
    let a = &v.platforms["anilist"];
    assert_eq!((a.client_id.as_str(), a.secret_set), ("4242", true));
    assert!(a.callback_url.ends_with("/api/auth/anilist/callback"));
    let v = admin
        .set_integrations(IntegrationsUpdate { anilist_client_secret: Some(String::new()), ..Default::default() })
        .await
        .unwrap();
    assert!(v.platforms["anilist"].secret_set, "an empty secret keeps the stored one");
    assert_eq!(
        v,
        assert_mirrors::<IntegrationsView>(&raw_get(&server.url, &token, "/api/admin/settings/integrations").await)
    );
    assert!(matches!(bob.integrations().await.unwrap_err(), ClientError::Forbidden { .. }));

    // Avatars.
    let png = lain_read::image::test_png(16, 16);
    let me = admin.set_avatar(png.clone(), "image/png").await.unwrap();
    assert_eq!(me.profile.avatar.kind, "upload");
    assert_eq!(bob.avatar("user-admin").await.unwrap(), png);
    assert!(matches!(
        admin.set_avatar(b"<svg/>".to_vec(), "image/svg+xml").await.unwrap_err(),
        ClientError::Api { status: 415, .. }
    ));
    assert_eq!(admin.clear_avatar().await.unwrap().profile.avatar.kind, "");
    assert!(matches!(bob.avatar("user-admin").await.unwrap_err(), ClientError::NotFound { .. }));

    // Read: the page index and page bytes.
    let pages = bob.comic_pages("item-read-manga").await.unwrap();
    assert_eq!((pages.kind.as_str(), pages.direction.as_str(), pages.pages.len()), ("manga", "rtl", 3));
    assert_eq!((pages.pages[0].width, pages.pages[0].height), (16, 24));
    assert_eq!(
        pages,
        assert_mirrors::<ComicPages>(&raw_get(&server.url, &token, "/api/items/item-read-manga/pages").await)
    );
    let comic = bob.comic_pages("item-read-comic").await.unwrap();
    assert_eq!((comic.direction.as_str(), comic.info.unwrap().series.as_str()), ("rtl", "Tiny Blade"));
    let page = bob.comic_page("item-read-manga", 2).await.unwrap();
    assert_eq!(lain_read::image::decode_config(&page), Some(("png", 16, 24)));
    assert!(matches!(bob.comic_page("item-read-manga", 3).await.unwrap_err(), ClientError::NotFound { .. }));
    assert!(matches!(bob.comic_pages("item-read-video").await.unwrap_err(), ClientError::Api { status: 400, .. }));

    // A backup is a tar the admin can read back.
    let tar = admin.backup().await.unwrap();
    let names: Vec<String> = lain_admin::backup::read_tar(&tar).unwrap().into_iter().map(|(n, _)| n).collect();
    assert!(names.contains(&"lain.redb".to_string()) && names.contains(&"manifest.json".to_string()), "{names:?}");
    assert!(matches!(bob.backup().await.unwrap_err(), ClientError::Forbidden { .. }));
    drop(server);
    std::fs::remove_dir_all(root).ok();
}

/// The watch module's typed routes (D-190) against lain-rs, without
/// ffmpeg: plans, options, settings, sessions, local play, and an
/// offline copy through the Range-capable stream.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watch_routes() {
    let root = temp("watch");
    let server = Server::with_fixture(&root).await;
    let bytes: Vec<u8> = (0..5000u32).map(|i| (i % 253) as u8).collect();
    let media = root.join("media/Frieren");
    std::fs::write(media.join("[Fansub-A] Frieren - 01 [1080p].mkv"), &bytes).unwrap();
    let (admin, bob) = admin_and_bob(&server.url).await;
    admin
        .create_library(NewLibrary {
            name: "Anime".into(),
            root: "media".into(),
            path: "Frieren".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let plan = bob.playback_plan(ITEM1, "mpv").await.unwrap();
    assert_eq!((plan.mode.as_str(), plan.available, plan.asset.as_str()), ("direct", true, "asset:item-frieren-01"));
    let gone = bob.playback_plan(ITEM2, "web").await.unwrap();
    assert_eq!((gone.mode.as_str(), gone.reason.as_str()), ("unavailable", "the file is no longer on disk"));
    assert!(bob.playback_plan_for(ITEM1, "web", Some(vec!["mkv".into(), "mkv/h264".into()])).await.is_ok());
    let opts = bob.playback_options().await.unwrap();
    assert_eq!((opts.default_delivery.as_str(), opts.qualities.len()), ("hls", 5));

    // Settings round trip (admin only).
    let mut view = admin.transcode_settings().await.unwrap();
    assert_eq!(view.settings.crf, 23);
    view.settings.crf = 21;
    assert_eq!(admin.update_transcode_settings(view.settings.clone()).await.unwrap().settings.crf, 21);
    view.settings.hls_segment_seconds = 99;
    let e = admin.update_transcode_settings(view.settings).await.unwrap_err();
    assert!(matches!(e, ClientError::Api { status: 400, ref code, .. } if code == "invalid-message"), "{e:?}");
    assert!(matches!(bob.transcode_settings().await.unwrap_err(), ClientError::Forbidden { .. }));

    // Sessions: a file ffprobe cannot read is an honest 503; unknown
    // sessions are 404.
    let e = bob.start_transcode(ITEM1, TranscodeSelection::default()).await.unwrap_err();
    assert!(matches!(e, ClientError::Api { status: 503, ref code, .. } if code == "dependency-unavailable"), "{e:?}");
    let e = bob.transcode_status(ITEM1, &"f".repeat(64)).await.unwrap_err();
    assert!(matches!(e, ClientError::NotFound { ref message } if message == "unknown transcode session"), "{e:?}");
    assert_eq!(bob.cancel_transcode(ITEM1, &"f".repeat(64)).await.unwrap().state, "idle");
    assert_eq!(bob.release_transcode(ITEM1, &"f".repeat(64), "tab-1").await.unwrap().state, "idle");
    let e = bob.release_transcode(ITEM1, &"f".repeat(64), "not a viewer").await.unwrap_err();
    assert!(matches!(e, ClientError::Api { status: 400, ref code, .. } if code == "invalid-message"), "{e:?}");
    assert!(admin.admin_transcodes().await.unwrap().sessions.is_empty());
    assert!(matches!(bob.admin_transcodes().await.unwrap_err(), ClientError::Forbidden { .. }));
    let hls = bob.hls_url(ITEM1, "s1").unwrap();
    assert!(hls.starts_with(&format!("{}/api/items/{ITEM1}/transcode/hls/index.m3u8?session=s1&token=", server.url)));
    assert!(bob.subtitles_url(ITEM1, 2).unwrap().contains("/subtitles?stream=2&token="));
    assert!(bob.transcode_url(ITEM1, "s1").unwrap().contains("/transcode?session=s1&token="));
    // Local play answers (players depend on the host; never launched here).
    bob.local_players().await.unwrap();

    // An offline copy now comes from the real stream route.
    let db = Arc::new(ClientDb::open(&root.join("client/client.redb")).unwrap());
    let store = OfflineStore::open(OfflineConfig::new(&root.join("offline").to_string_lossy()), db).unwrap();
    let item = bob.catalog_item(ITEM1).await.unwrap();
    store.add(&bob.server(), &item).unwrap();
    let e = store.fetch(&bob, ITEM1, None).await.unwrap();
    assert_eq!((e.state, e.size), (OfflineState::Done, 5000));
    assert!(e.validator.ends_with(" GMT"), "Last-Modified is the resume validator: {}", e.validator);
    assert_eq!(std::fs::read(store.local_path(&bob.server(), ITEM1).unwrap().unwrap()).unwrap(), bytes);
    drop(server);
    std::fs::remove_dir_all(root).ok();
}

/// The lists and social modules through the typed client: a pin link
/// against the recorded AniList answers, the list, sync, scrobble
/// preference and unlink; friends, a rating of a catalog item, the work
/// view, a shared collection and notifications, with `list-changed` and
/// `social-notification` on the stream.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lists_and_social() {
    use lain_client::types::{
        CollectionItemInput, CollectionShareInput, MarkRead, NewCollection, RatingInput, ShareInput,
        SocialSettingsPatch, WorkRef,
    };
    let root = temp("lists-social");
    let upstream = lain_conformance::upstream::Upstream::start(&lain_conformance::upstream::dir(), 0).unwrap();
    let base = upstream.url.clone();
    let server = Server::start_with(&root.join("data"), move |c| {
        c.offline = false;
        c.list_sync = false;
        c.anilist_endpoints = lain_lists::anilist::Endpoints {
            graphql: format!("{base}/anilist-list/graphql"),
            oauth: format!("{base}/anilist-list/oauth"),
        };
    })
    .await;
    let (admin, bob) = admin_and_bob(&server.url).await;
    let stream = EventStream::new(bob.clone(), StreamConfig::default());
    next_matching(&stream, "ready", |i| matches!(i, StreamItem::Connected(_))).await;

    // Lists.
    assert!(bob.links().await.unwrap().is_empty());
    assert!(bob.link_pin_url("anilist").await.unwrap().contains("client_id=52227"));
    let err = bob.link_authorize_url("anilist").await.unwrap_err();
    assert!(matches!(err, ClientError::Api { status: 503, ref code, .. } if code == "not-configured"), "{err:?}");
    let err = bob.link_with_code("anilist", "bad-code").await.unwrap_err();
    assert!(matches!(err, ClientError::Api { status: 400, ref code, .. } if code == "invalid-grant"), "{err:?}");
    let link = bob.link_with_code("anilist", "good-code").await.unwrap();
    assert_eq!((link.platform.as_str(), link.remote_username.as_str()), ("anilist", "watcher"));
    let ev = next_matching(
        &stream,
        "list-changed synced",
        |i| matches!(i, StreamItem::Event(ServerEvent::ListChanged(e)) if e.action == "synced"),
    )
    .await;
    let StreamItem::Event(ServerEvent::ListChanged(ev)) = ev else { unreachable!() };
    assert_eq!((ev.platform.as_str(), ev.upserted, ev.removed), ("anilist", 2, 0));
    assert_eq!(bob.links().await.unwrap()[0].entry_count, 2);
    let manga = bob.list("manga", "").await.unwrap();
    assert_eq!((manga.len(), manga[0].title.as_str(), manga[0].progress_volumes), (1, "One Piece", 7));
    assert_eq!(bob.list("", "").await.unwrap().len(), 2);
    let stats = bob.sync_link("anilist").await.unwrap();
    assert_eq!((stats.upserted, stats.removed), (2, 0));
    assert!(bob.set_link_scrobble("anilist", true).await.unwrap().scrobble);
    assert_eq!(bob.unlink("anilist").await.unwrap(), 2);
    assert!(matches!(bob.unlink("anilist").await.unwrap_err(), ClientError::NotFound { .. }));
    assert!(bob.list("", "").await.unwrap().is_empty());

    // Social.
    let st = bob
        .update_social_settings(SocialSettingsPatch { ratings: Some("public".into()), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(st.ratings, "public");
    assert_eq!(admin.find_users("bo").await.unwrap()[0].user.username, "bob");
    assert_eq!(admin.befriend("user-bob").await.unwrap().relation, "outgoing");
    let ev = next_matching(&stream, "social-notification", |i| {
        matches!(i, StreamItem::Event(ServerEvent::SocialNotification(_)))
    })
    .await;
    let StreamItem::Event(ServerEvent::SocialNotification(n)) = ev else { unreachable!() };
    assert_eq!((n.kind.as_str(), n.from_user_id.as_str()), ("friend_request", "user-admin"));
    assert_eq!(bob.befriend("user-admin").await.unwrap().relation, "friend");
    assert_eq!(bob.friends().await.unwrap().friends[0].user.id, "user-admin");
    let frieren = WorkRef { kind: "manga".into(), title: "Frieren".into(), ..Default::default() };
    let r =
        bob.rate(RatingInput { work: frieren.clone(), score: 9, review: "kind".into(), spoiler: false }).await.unwrap();
    assert_eq!(r.score, 9);
    let view = admin.work(&frieren).await.unwrap();
    assert_eq!((view.mine, view.ratings.len(), view.average, view.scored), (None, 1, 9.0, 1));
    assert!(view.users.contains_key("user-bob"));
    let c = admin
        .comment(lain_client::types::CommentInput {
            work: frieren.clone(),
            body: "agreed".into(),
            reply_to: String::new(),
        })
        .await
        .unwrap();
    admin.delete_comment(&c.id).await.unwrap();
    assert_eq!(admin.feed(0, 0).await.unwrap().items[0].kind, "rated");
    assert_eq!(admin.user_ratings("user-bob").await.unwrap().len(), 1);
    assert_eq!(admin.user_activity("user-bob", 0, 10).await.unwrap().items.len(), 1);
    assert_eq!(admin.social_profile("user-bob").await.unwrap().relation, "friend");
    let col =
        bob.create_collection(NewCollection { name: "Rainy".into(), ..Default::default() }).await.unwrap().collection;
    bob.add_to_collection(&col.id, CollectionItemInput { work: frieren.clone(), note: "start".into() }).await.unwrap();
    assert_eq!(
        bob.share_collection(&col.id, CollectionShareInput { to: vec!["user-admin".into()], message: String::new() })
            .await
            .unwrap(),
        ["user-admin"]
    );
    assert_eq!(admin.collection(&col.id).await.unwrap().collection.items.len(), 1);
    assert_eq!(admin.collections().await.unwrap().collections.len(), 1);
    assert_eq!(
        bob.share_work(ShareInput { to: vec!["user-admin".into()], work: frieren.clone(), message: "read".into() })
            .await
            .unwrap(),
        ["user-admin"]
    );
    let inbox = admin.notifications(0).await.unwrap();
    assert!(inbox.unread >= 2 && inbox.users.contains_key("user-bob"), "{inbox:?}");
    assert!(admin.mark_notifications_read(MarkRead { all: true, ..Default::default() }).await.unwrap() >= 2);
    assert!(bob.remove_from_collection(&col.id, &frieren).await.unwrap().collection.items.is_empty());
    bob.unshare_collection(&col.id, "user-admin").await.unwrap();
    bob.delete_collection(&col.id).await.unwrap();
    assert!(bob.delete_rating(&frieren).await.unwrap());
    assert_eq!(bob.block("user-admin").await.unwrap().relation, "blocking");
    assert!(matches!(admin.social_profile("user-bob").await.unwrap_err(), ClientError::NotFound { .. }));
    assert_eq!(bob.unblock("user-admin").await.unwrap().relation, "none");
    assert_eq!(admin.befriend("user-bob").await.unwrap().relation, "outgoing");
    assert_eq!(bob.decline_friend("user-admin").await.unwrap().relation, "none");
    assert_eq!(bob.unfriend("user-admin").await.unwrap().relation, "none");
    drop(stream);
    drop(server);
    std::fs::remove_dir_all(root).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn acquire_and_downloads() {
    use lain_client::acquire::ReleaseSearch;
    use lain_client::types::acquire::{
        AcquireView, GrabInput, GrabList, IndexerInput, Monitored, Profile, SubtitleProviderInput, Wanted,
        WantedOverview,
    };
    use lain_client::types::downloads::{DownloadAdd, DownloadsView};

    let root = temp("acquire");
    let media = root.join("media/Anime");
    std::fs::create_dir_all(&media).unwrap();
    let server = Server::start(&root.join("data")).await;
    let (admin, bob) = admin_and_bob(&server.url).await;
    let token = admin.session().unwrap().token;
    let lib = admin
        .create_library(NewLibrary {
            name: "Anime".into(),
            kind: "anime".into(),
            path: media.to_string_lossy().into(),
            ..Default::default()
        })
        .await
        .unwrap();

    // Admin only.
    assert!(matches!(bob.acquire_settings().await.unwrap_err(), ClientError::Forbidden { .. }));
    let view = admin.acquire_settings().await.unwrap();
    assert_eq!(view, assert_mirrors::<AcquireView>(&raw_get(&server.url, &token, "/api/acquire/settings").await));
    let view = admin.update_acquire_settings(serde_json::json!({"seed_ratio": 0.5})).await.unwrap();
    assert_eq!((view.settings.seed_ratio, view.settings.max_active), (0.5, 3));

    // Indexers keep their key to themselves.
    let ix = admin
        .create_indexer(IndexerInput {
            name: Some("tracker-exemplo".into()),
            protocol: Some("torznab".into()),
            url: Some("http://127.0.0.1:9/api".into()),
            api_key: Some("secret".into()),
            min_interval_ms: Some(0),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(ix.has_api_key && ix.indexer.api_key.is_empty());
    let test = admin.test_indexer(&ix.indexer.id).await.unwrap();
    assert!(!test.ok && !test.error.is_empty());
    let res = admin
        .search_releases(ReleaseSearch { query: "Frieren".into(), kind: "anime".into(), ..Default::default() })
        .await
        .unwrap();
    assert!(res.candidates.is_empty() && res.failures.len() == 1);
    assert_eq!(admin.indexers().await.unwrap().len(), 1);
    admin.delete_indexer(&ix.indexer.id).await.unwrap();
    let r = admin.parse_release("[Fansub-A] Frieren - 05 [1080p]", "anime").await.unwrap();
    assert_eq!((r.title.as_str(), r.episodes.as_slice()), ("Frieren", &[5][..]));

    // A magnet whose tracker never answers: stored, paused, removed.
    let magnet = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Frieren%20-%2003&tr=http%3A%2F%2F127.0.0.1%3A9%2Fa";
    let g = admin
        .grab(GrabInput { magnet: magnet.into(), library_id: lib.id.clone(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(g.state, "metadata");
    let list = admin.grabs().await.unwrap();
    assert_eq!(list, assert_mirrors::<GrabList>(&raw_get(&server.url, &token, "/api/acquire/grabs").await));
    assert_eq!(admin.grab_action(&g.id, "pause").await.unwrap().state, "paused");
    admin.remove_grab(&g.id, true).await.unwrap();
    assert!(admin.grabs().await.unwrap().grabs.is_empty());

    // Profiles and monitored titles.
    let p = admin
        .create_quality_profile(Profile { name: "4K".into(), resolutions: vec!["2160p".into()], ..Default::default() })
        .await
        .unwrap();
    assert_eq!(admin.quality_profiles().await.unwrap().len(), 2);
    let m = admin
        .create_monitored(Monitored {
            library_id: lib.id.clone(),
            kind: "anime".into(),
            title: "Frieren".into(),
            profile_id: p.id.clone(),
            to: 2,
            ..Default::default()
        })
        .await
        .unwrap();
    let w = admin.wanted_for(&m.id).await.unwrap();
    assert_eq!(w.missing.len(), 2);
    assert_eq!(
        w,
        assert_mirrors::<Wanted>(
            &raw_get(&server.url, &token, &format!("/api/acquire/monitored/{}/wanted", m.id)).await
        )
    );
    let all = admin.wanted().await.unwrap();
    assert_eq!(all, assert_mirrors::<WantedOverview>(&raw_get(&server.url, &token, "/api/acquire/wanted").await));
    assert!(admin.search_monitored(&m.id).await.unwrap().grabbed.is_empty());
    assert!(!admin.automation().await.unwrap().enabled);
    assert!(matches!(admin.delete_quality_profile(&p.id).await.unwrap_err(), ClientError::Api { status: 409, .. }));
    admin.delete_monitored(&m.id).await.unwrap();
    admin.delete_quality_profile(&p.id).await.unwrap();
    assert!(admin.blocklist().await.unwrap().is_empty());
    assert_eq!(admin.purge_replaced().await.unwrap(), 0);

    // Subtitle accounts and records.
    let sp = admin
        .create_subtitle_provider(SubtitleProviderInput {
            name: Some("os".into()),
            kind: Some("opensubtitles".into()),
            api_key: Some("k".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(sp.has_api_key && !sp.has_password && sp.provider.api_key.is_empty());
    admin.delete_subtitle_provider(&sp.provider.id).await.unwrap();
    assert!(admin.subtitle_ledger().await.unwrap().is_empty());
    assert_eq!(admin.clear_subtitle_refusals().await.unwrap(), 0);

    // Downloads (the server is offline: a job fails, and is removable).
    let job =
        admin.add_download(DownloadAdd { url: "http://127.0.0.1:9/x.mkv".into(), ..Default::default() }).await.unwrap();
    assert_eq!(job.name, "x.mkv");
    let view = admin.downloads().await.unwrap();
    let raw = raw_get(&server.url, &token, "/api/downloads").await;
    assert_eq!(view.settings, assert_mirrors::<DownloadsView>(&raw).settings);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while admin.download(&job.id).await.unwrap().state != "failed" {
        assert!(std::time::Instant::now() < deadline, "never failed");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(admin.download_action(&job.id, "cancel").await.unwrap().state, "canceled");
    admin.remove_download(&job.id).await.unwrap();
    let s = admin.download_settings().await.unwrap();
    assert_eq!(admin.update_download_settings(s.clone()).await.unwrap(), s);
    assert_eq!(admin.cleanup_downloads().await.unwrap().records, 0);
    drop(server);
    std::fs::remove_dir_all(root).ok();
}

/// One file of `lain-http`'s committed test CA (`tests/tls/`: a root
/// and a certificate for `localhost` and 127.0.0.1, valid until 2126).
fn tls_file(name: &str) -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../lain-http/tests/tls").join(name)).unwrap()
}

/// Terminates TLS with the test certificate in front of `target` and
/// returns its `https://localhost:<port>` address.
async fn tls_front(target: std::net::SocketAddr) -> String {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    let cfg = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(tls_file("server.der"))],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(tls_file("server.key.der"))),
        )
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(cfg));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("https://localhost:{}", listener.local_addr().unwrap().port());
    tokio::spawn(async move {
        while let Ok((down, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(mut down) = acceptor.accept(down).await else { return };
                let Ok(mut up) = tokio::net::TcpStream::connect(target).await else { return };
                let _ = tokio::io::copy_bidirectional(&mut down, &mut up).await;
            });
        }
    });
    url
}

/// `https` (D-277): a certificate from an unknown CA is refused as
/// `Tls`, which counts as unreachable; once its root is trusted the
/// same client logs in and reads through TLS. A port that does not
/// speak TLS is `Tls` too, never a hang.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn https_with_a_trusted_root() {
    let root = temp("https");
    let server = Server::start(&root.join("data")).await;
    let url = tls_front(server.addr).await;
    let c = client(&url);
    assert_eq!(c.server(), url);

    let e = c.health().await.unwrap_err();
    assert!(matches!(e, ClientError::Tls { .. }) && e.is_offline(), "{e:?}");

    assert!(matches!(lain_client::trust_roots(vec![b"not a certificate".to_vec()]), Err(ClientError::Invalid { .. })));
    assert!(matches!(c.health().await.unwrap_err(), ClientError::Tls { .. }));

    lain_client::trust_roots(vec![tls_file("ca.der")]).unwrap();
    assert_eq!(c.health().await.unwrap().status, "ok");
    c.setup("admin", &pw(0)).await.unwrap();
    assert_eq!(c.login("admin", &pw(0)).await.unwrap().username, "admin");
    assert_eq!(c.session().unwrap().server, url);
    assert_eq!(c.me().await.unwrap().username, "admin");
    assert!(c.libraries().await.unwrap().is_empty());

    let plain = client(&format!("https://localhost:{}", server.addr.port()));
    assert!(matches!(plain.health().await.unwrap_err(), ClientError::Tls { .. }));
}
