//! The bindings end to end against the real server in-process (the
//! P5 exit criteria without a device): log in, navigate, resume, make
//! offline copies, lose the network, watch and read from the copies,
//! queue progress, and push it when the network comes back.
//!
//! The harness is lain-client's own (a server, a proxy that drops the
//! network on demand, run-time passwords).

#[path = "../../../../crates/lain-client/tests/support/mod.rs"]
mod support;

use std::sync::{Arc, Mutex};

use lain_client::types::NewLibrary;
use lain_ffi::{CopyState, DownloadObserver, LainCore, LainError, SaveOutcome, SourceKind};
use support::*;

const ITEM1: &str = "item-frieren-01";

/// The workspace's conformance fixtures (the harness's own loaders
/// resolve them from lain-client's manifest dir, not this one).
fn fixtures() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../conformance/fixtures")
}

fn import(data: &std::path::Path, media: &std::path::Path, catalog: &str, n: usize) {
    std::fs::create_dir_all(data).unwrap();
    let roots = serde_json::json!({"format": "lain.roots@1", "roots": {"media": media.to_string_lossy()}});
    std::fs::write(data.join("roots.json"), serde_json::to_vec(&roots).unwrap()).unwrap();
    let raw = std::fs::read(fixtures().join(catalog)).unwrap();
    assert_eq!(lain_server::import(data, &serde_json::from_slice(&raw).unwrap()).unwrap(), n);
}

/// Two Frieren episodes in library `lib-e8bb66fd`, root `media`.
async fn watch_server(root: &std::path::Path) -> Server {
    let media = root.join("media");
    std::fs::create_dir_all(media.join("Frieren")).unwrap();
    import(&root.join("data"), &media, "catalog.json", 2);
    Server::start(&root.join("data")).await
}

/// The read fixture: comics and manga under `conformance/fixtures/read`.
async fn read_server(root: &std::path::Path) -> Server {
    import(&root.join("data"), &fixtures().join("read"), "read-catalog.json", 8);
    Server::start_with(&root.join("data"), |c| {
        c.watch = false;
        c.auto_enrich = false;
    })
    .await
}

fn core(root: &std::path::Path) -> Arc<LainCore> {
    LainCore::new(root.join("app").to_string_lossy().into(), root.join("copies").to_string_lossy().into()).unwrap()
}

#[derive(Default)]
struct Seen(Mutex<Vec<(String, i64, i64)>>);

impl DownloadObserver for Seen {
    fn progress(&self, item_id: String, done: i64, total: i64) {
        self.0.lock().unwrap().push((item_id, done, total));
    }
    fn evicted(&self, _item_id: String, _title: String) {}
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watch_offline_and_sync_back() {
    let root = temp("ffi-watch");
    let server = watch_server(&root).await;
    let bytes: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(root.join("media/Frieren/[Fansub-A] Frieren - 01 [1080p].mkv"), &bytes).unwrap();
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
    let proxy = Proxy::start(server.addr).await;

    let core = core(&root);
    assert!(matches!(core.catalog().await.unwrap_err(), LainError::NoServer));
    let canonical = core.set_server(proxy.url.clone()).unwrap();
    assert_eq!(core.server().as_deref(), Some(canonical.as_str()));
    assert!(core.session().is_none());
    assert!(!core.setup_required().await.unwrap());
    assert!(matches!(core.login("bob".into(), pw(9)).await, Err(LainError::LoginFailed { .. })));
    let me = core.login("bob".into(), pw(2)).await.unwrap();
    assert_eq!((me.username.as_str(), me.role.as_str()), ("bob", "user"));
    assert_eq!(core.session().unwrap().username, "bob");

    // Navigation comes from the modules; the catalog is cached.
    let modules = core.modules().await.unwrap();
    let dests: Vec<String> = modules.iter().flat_map(|m| m.destinations.iter().map(|d| d.id.clone())).collect();
    assert!(dests.iter().any(|d| d == "home") && dests.iter().any(|d| d == "library"), "{dests:?}");
    let snap = core.catalog().await.unwrap();
    assert!(!snap.from_cache);
    assert_eq!(snap.items.len(), 2);
    let ep1 = snap.items.iter().find(|i| i.id == ITEM1).unwrap();
    assert_eq!(
        (ep1.title.as_str(), ep1.episode, ep1.file_name.as_str()),
        ("Frieren", 1, "[Fansub-A] Frieren - 01 [1080p].mkv")
    );
    assert_eq!(core.search("frier".into(), 10).await.unwrap().len(), 2);

    // The account's list: a favourite is the work, so the second episode
    // names the same one, and taking it out by either empties the list.
    assert!(core.favorites().await.unwrap().is_empty());
    let favs = core.set_favorite(ITEM1.into(), true).await.unwrap();
    assert_eq!(favs.iter().map(|f| f.title.as_str()).collect::<Vec<_>>(), ["Frieren"]);
    let ep2 = snap.items.iter().find(|i| i.id != ITEM1).unwrap().id.clone();
    assert_eq!(core.set_favorite(ep2, true).await.unwrap().len(), 1, "one work, whichever episode named it");
    assert!(core.set_favorite(ITEM1.into(), false).await.unwrap().is_empty());

    // The account's language: set, read back, cleared.
    assert_eq!(core.set_preferred_language("jpn".into()).await.unwrap().preferred_language, "jpn");
    assert_eq!(core.me().await.unwrap().preferred_language, "jpn");
    assert_eq!(core.set_preferred_language(String::new()).await.unwrap().preferred_language, "");
    // D-280: the one language reads as both; the three are set apart.
    let both = core.set_preferred_language("jpn".into()).await.unwrap();
    assert_eq!(
        (
            both.preferred_audio_language.as_str(),
            both.preferred_subtitle_language.as_str(),
            both.preferred_subtitle_mode.as_str()
        ),
        ("jpn", "jpn", "auto")
    );
    let split = core.set_track_preferences("jpn".into(), " POR ".into(), "always".into()).await.unwrap();
    assert_eq!(
        (
            split.preferred_audio_language.as_str(),
            split.preferred_subtitle_language.as_str(),
            split.preferred_subtitle_mode.as_str(),
            split.preferred_language.as_str()
        ),
        ("jpn", "por", "always", "por")
    );
    assert_eq!(core.me().await.unwrap(), split);
    assert!(core.set_track_preferences("jpn".into(), "por".into(), "sometimes".into()).await.is_err());
    let cleared = core.set_track_preferences(String::new(), String::new(), String::new()).await.unwrap();
    assert_eq!((cleared.preferred_language.as_str(), cleared.preferred_subtitle_mode.as_str()), ("", "auto"));

    // A score: given, seen in the average, changed, and taken back.
    let unrated = core.rating(ITEM1.into()).await.unwrap();
    assert_eq!((unrated.mine, unrated.scored), (0, 0));
    let rated = core.rate(ITEM1.into(), 8).await.unwrap();
    assert_eq!((rated.mine, rated.scored, rated.average), (8, 1, 8.0));
    assert_eq!(core.rate(ITEM1.into(), 10).await.unwrap().mine, 10);
    let cleared = core.rate(ITEM1.into(), 0).await.unwrap();
    assert_eq!((cleared.mine, cleared.scored), (0, 0));

    // Resume: a position saved online is the next play's start.
    let saved = core.save_progress(ITEM1.into(), 600., 1440., false).await.unwrap();
    assert!(matches!(saved, SaveOutcome::Synced { .. }), "{saved:?}");
    assert_eq!(core.continue_watching().await.unwrap()[0].item_id, ITEM1);
    assert_eq!(core.all_progress().await.unwrap()[0].item_id, ITEM1);
    // Online, an mkv this server cannot probe needs a transcode, which
    // needs ffmpeg: an honest error, never a panic.
    match core.play_source(ITEM1.into(), vec!["mp4".into(), "mp4/h264".into()], String::new()).await {
        Ok(src) => assert_eq!((src.kind, src.start_sec), (SourceKind::Hls, 600.)),
        Err(e) => assert!(!matches!(e, LainError::Internal { .. }), "{e:?}"),
    }

    // An offline copy, with progress reports.
    let added = core.offline_add(ITEM1.into()).await.unwrap();
    assert_eq!(added.state, CopyState::Queued);
    let seen = Arc::new(Seen::default());
    let run = core.offline_run(Some(seen.clone())).await.unwrap();
    assert_eq!((run.done, run.failed, run.stopped.clone()), (1, 0, None));
    let copy = core.offline_copy(ITEM1.into()).unwrap().unwrap();
    assert_eq!((copy.state, copy.bytes), (CopyState::Done, 20_000));
    assert_eq!(seen.0.lock().unwrap().last().unwrap(), &(ITEM1.to_string(), 20_000, 20_000));
    assert_eq!(core.offline_copies().unwrap().len(), 1);
    assert_eq!(core.offline_usage().unwrap().used_bytes, 20_000);

    // The network goes away: the copy plays, from the saved position.
    proxy.set_online(false);
    let src = core.play_source(ITEM1.into(), Vec::new(), String::new()).await.unwrap();
    assert_eq!((src.kind, src.start_sec, src.offset_sec), (SourceKind::Offline, 600., 0.));
    assert_eq!(std::fs::read(&src.url).unwrap(), bytes);
    let offline = core.catalog().await.unwrap();
    assert!(offline.from_cache && offline.items.len() == 2);
    assert_eq!(core.search("FRIEREN".into(), 1).await.unwrap().len(), 1);
    assert!(core.health().await.unwrap_err().is_offline());
    let q = core.save_progress(ITEM1.into(), 1300., 1440., false).await.unwrap();
    assert!(matches!(q, SaveOutcome::Queued { .. }), "{q:?}");
    assert_eq!(core.queued_progress().unwrap(), 1);
    assert_eq!(core.progress(ITEM1.into()).await.unwrap().position_sec, 1300.);

    // Back online: the queued position reaches the server.
    proxy.set_online(true);
    let f = core.flush_progress().await.unwrap();
    assert_eq!((f.pushed, f.remaining), (1, 0), "{f:?}");
    let direct = client(&server.url);
    direct.login("bob", &pw(2)).await.unwrap();
    assert_eq!(direct.progress(ITEM1).await.unwrap().position_sec, 1300.);

    // Removing the copy frees its bytes.
    assert_eq!(core.offline_remove(ITEM1.into()).unwrap(), 20_000);
    assert!(core.offline_copy(ITEM1.into()).unwrap().is_none());
    core.logout().unwrap();
    assert!(core.session().is_none());
    drop(core);
    drop(server);
    std::fs::remove_dir_all(root).ok();
}

/// ffmpeg and ffprobe on PATH, or a skip (a failure under
/// `LAIN_REQUIRE_FFMPEG`, as the server's watch tests do).
fn require_ffmpeg() -> bool {
    let ok = ["ffmpeg", "ffprobe"]
        .iter()
        .all(|t| std::process::Command::new(t).arg("-version").output().map(|o| o.status.success()).unwrap_or(false));
    if !ok {
        assert!(
            std::env::var("LAIN_REQUIRE_FFMPEG").is_err(),
            "LAIN_REQUIRE_FFMPEG is set but ffmpeg/ffprobe are not on PATH"
        );
        eprintln!("skipping: ffmpeg/ffprobe not installed");
    }
    ok
}

fn ffmpeg(args: &[&str]) -> std::process::Output {
    let out = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    out
}

/// The tone of a played file's audio, in Hz, from its zero crossings: the
/// clip's English track is 440 Hz and its Japanese one 660 Hz.
fn tone(file: &std::path::Path) -> f64 {
    let f = file.to_string_lossy();
    let out = ffmpeg(&["-i", &f, "-map", "0:a:0", "-ac", "1", "-ar", "8000", "-t", "1", "-f", "s16le", "-"]);
    let s: Vec<i16> = out.stdout.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
    let crossings = s.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
    crossings as f64 / 2. / (s.len() as f64 / 8000.)
}

/// The first seconds an HLS session made (its init and first segment),
/// fetched over its playlist URL into `out`.
async fn hls_head(url: &str, out: &std::path::Path) {
    let get = |u: String| async move {
        let (base, path) = u.split_at(u.find("/api/").unwrap());
        let (base, path) = (lain_conformance::http::Base::parse(base).unwrap(), path.to_string());
        let r = tokio::task::spawn_blocking(move || lain_conformance::http::request(&base, "GET", &path, &[], None))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        r.body
    };
    let dir = &url[..url.rfind('/').unwrap() + 1];
    let playlist = String::from_utf8(get(url.to_string()).await).unwrap();
    let init = playlist.split("URI=\"").nth(1).and_then(|r| r.split('"').next()).expect("an fMP4 init segment");
    let first = playlist.lines().find(|l| !l.is_empty() && !l.starts_with('#')).expect("a segment");
    let mut bytes = get(format!("{dir}{init}")).await;
    bytes.extend(get(format!("{dir}{first}")).await);
    std::fs::write(out, bytes).unwrap();
}

/// Probes the server's encoders with a patient client before a transcode
/// call: on a busy machine the first probe can outlast the app's 20 s
/// request timeout, and the server keeps its answer for 30 s.
async fn warm(admin: &lain_client::Client) {
    admin.transcode_settings().await.unwrap();
}

/// D-280 on a transcode: the session carries one audio stream, the one the
/// app asks for, by the title's language or by its index.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_transcode_plays_the_audio_asked_for() {
    if !require_ffmpeg() {
        return;
    }
    let root = temp("ffi-audio");
    let server = watch_server(&root).await;
    // English (default, 440 Hz) and Japanese (660 Hz) over a picture in a
    // codec no claim names, so the plan transcodes.
    let file = root.join("media/Frieren/[Fansub-A] Frieren - 01 [1080p].mkv");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc=size=160x120:rate=10:duration=8",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:sample_rate=48000:duration=8",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=660:sample_rate=48000:duration=8",
        "-map",
        "0:v",
        "-map",
        "1:a",
        "-map",
        "2:a",
        "-c:v",
        "mpeg4",
        "-c:a",
        "aac",
        "-metadata:s:a:0",
        "language=eng",
        "-metadata:s:a:1",
        "language=jpn",
        "-disposition:a:0",
        "default",
        "-disposition:a:1",
        "0",
        "-y",
        &file.to_string_lossy(),
    ]);
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
    let patient = lain_client::Client::new(
        lain_client::ClientConfig { server: server.url.clone(), timeout_ms: 180_000 },
        Arc::new(lain_client::MemoryTokenStore::new()),
    )
    .unwrap();
    patient.login("admin", &pw(0)).await.unwrap();
    let core = core(&root);
    core.set_server(server.url.clone()).unwrap();
    core.login("bob".into(), pw(2)).await.unwrap();
    let claim = vec!["mkv".into(), "mkv/h264".into(), "mkv/aac".into()];

    // No language: the server's choice, the file's default.
    warm(&patient).await;
    let plain = core.play_source(ITEM1.into(), claim.clone(), String::new()).await.unwrap();
    assert_eq!((plain.kind, plain.audio_stream), (SourceKind::Hls, None));
    let listed: Vec<(i64, &str, bool)> =
        plain.audio.iter().map(|a| (a.stream, a.language.as_str(), a.default)).collect();
    assert_eq!(listed, [(1, "eng", true), (2, "jpn", false)]);
    let head = root.join("head.mp4");
    hls_head(&plain.url, &head).await;
    assert!((tone(&head) - 440.).abs() < 30., "default audio: {} Hz", tone(&head));

    // The title's language: its stream, from the start. (Session ids are
    // not compared: the server's profile can change once it has run its
    // first encode, and the id hashes it.)
    warm(&patient).await;
    let japanese = core.play_source(ITEM1.into(), claim.clone(), "jpn".into()).await.unwrap();
    assert_eq!(japanese.audio_stream, Some(2));
    hls_head(&japanese.url, &head).await;
    assert!((tone(&head) - 660.).abs() < 30., "Japanese audio: {} Hz", tone(&head));
    // A language the file lacks leaves the server's choice.
    warm(&patient).await;
    let korean = core.play_source(ITEM1.into(), claim.clone(), "kor".into()).await.unwrap();
    assert_eq!(korean.audio_stream, None);
    hls_head(&korean.url, &head).await;
    assert!((tone(&head) - 440.).abs() < 30., "no Korean, the default: {} Hz", tone(&head));

    // A pick in the player names the stream, from where the viewer is.
    warm(&patient).await;
    let picked = core.play_transcoded(ITEM1.into(), 2., String::new(), None, Some(2)).await.unwrap();
    assert_eq!((picked.audio_stream, picked.offset_sec), (Some(2), 2.));
    hls_head(&picked.url, &head).await;
    assert!((tone(&head) - 660.).abs() < 30., "picked Japanese: {} Hz", tone(&head));
    warm(&patient).await;
    let back = core.play_transcoded(ITEM1.into(), 2., String::new(), None, Some(1)).await.unwrap();
    hls_head(&back.url, &head).await;
    assert!((tone(&head) - 440.).abs() < 30., "English again: {} Hz", tone(&head));
    // A stream that is not audio is the server's refusal, not a panic.
    warm(&patient).await;
    let refused = core.play_transcoded(ITEM1.into(), 0., String::new(), None, Some(9)).await.unwrap_err();
    assert!(!matches!(refused, LainError::Internal { .. }), "{refused:?}");
    for s in [plain, japanese, korean, picked, back] {
        core.stop_transcode(ITEM1.into(), s.session).await.ok();
    }
    drop(core);
    drop(server);
    std::fs::remove_dir_all(root).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn read_offline_copy() {
    let root = temp("ffi-read");
    let server = read_server(&root).await;
    let (_admin, _bob) = admin_and_bob(&server.url).await;
    let proxy = Proxy::start(server.addr).await;
    let core = core(&root);
    core.set_server(proxy.url.clone()).unwrap();
    core.login("bob".into(), pw(2)).await.unwrap();

    let online = core.comic("item-read-manga".into()).await.unwrap();
    assert_eq!((online.direction.as_str(), online.pages.len(), online.offline), ("rtl", 3, false));
    let page2 = core.comic_page("item-read-manga".into(), 2).await.unwrap();

    core.offline_add("item-read-manga".into()).await.unwrap();
    let copy = core.offline_fetch("item-read-manga".into(), None).await.unwrap();
    assert_eq!(copy.state, CopyState::Done);
    core.save_progress("item-read-manga".into(), 2., 3., false).await.unwrap();

    proxy.set_online(false);
    let local = core.comic("item-read-manga".into()).await.unwrap();
    assert!(local.offline);
    assert_eq!((local.direction.as_str(), local.pages.len()), ("rtl", 3));
    assert_eq!((local.pages[0].width, local.pages[0].height), (16, 24));
    assert_eq!(core.comic_page("item-read-manga".into(), 2).await.unwrap(), page2);
    assert!(matches!(core.comic_page("item-read-manga".into(), 9).await, Err(LainError::NotFound { .. })));
    assert_eq!(core.progress("item-read-manga".into()).await.unwrap().position_sec, 2.);
    // A book with no copy needs the server.
    assert!(core.comic("item-read-comic".into()).await.unwrap_err().is_offline());
    drop(core);
    drop(server);
    std::fs::remove_dir_all(root).ok();
}

/// The native book reader's calls (D-306): the manifest, a chapter
/// ready for a WebView (absolute resources with the token, chapter links
/// on `lain-chapter://`), and progress on the shared virtual pages.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn read_a_reflowable_book() {
    let root = temp("ffi-book");
    let server = read_server(&root).await;
    let (_admin, _bob) = admin_and_bob(&server.url).await;
    let proxy = Proxy::start(server.addr).await;
    let core = core(&root);
    let base = core.set_server(proxy.url.clone()).unwrap();
    core.login("bob".into(), pw(2)).await.unwrap();

    let book = core.book("item-read-epub".into()).await.unwrap();
    assert_eq!(
        (book.layout.as_str(), book.title.as_str(), book.direction.as_str()),
        ("reflowable", "Tiny Book", "ltr")
    );
    assert_eq!(book.authors, ["A. Writer"]);
    assert_eq!((book.chapters.len(), book.chapters[1], book.pages), (2, 7, 1));
    let toc: Vec<_> = book.toc.iter().map(|t| (t.title.as_str(), t.chapter, t.anchor.as_str(), t.depth)).collect();
    assert_eq!(toc, [("One", 0, "", 0), ("Two", 1, "end", 0)]);
    let pdf = core.book("item-read-pdf".into()).await.unwrap();
    assert_eq!((pdf.layout.as_str(), pdf.pages), ("pages", 2));
    assert!(
        matches!(core.book("item-read-manga".into()).await, Err(LainError::Api { status: 400, ref code, .. }) if code == "not-a-book")
    );

    let html = core.book_chapter("item-read-epub".into(), 0).await.unwrap();
    assert!(html.contains("href=\"lain-chapter://1#end\" data-lain-chapter=\"1\""), "{html}");
    assert!(!html.contains("steal"));
    let token = core_token(&root, &base);
    let cover = format!("{}/api/items/item-read-epub/book/res/OEBPS/cover.png?token=", base.trim_end_matches('/'));
    let at = html.find(&cover).unwrap_or_else(|| panic!("{html}"));
    let url = &html[at..at + html[at..].find('"').unwrap()];
    assert!(url.ends_with(&lain_client::http::encode(&token)), "{url}");
    // The URL loads on its own, as a WebView would ask for it.
    let r = lain_http::Client::new(lain_http::Config::default()).unwrap().get(url).send().await.unwrap();
    assert_eq!((r.status, r.header("content-type")), (200, Some("image/png")));
    assert!(matches!(core.book_chapter("item-read-epub".into(), 2).await, Err(LainError::NotFound { .. })));
    assert!(matches!(core.book_chapter("item-read-epub".into(), -1).await, Err(LainError::Invalid { .. })));
    // Anchors are placed by their share of the chapter's text (no script).
    assert_eq!(core.book_anchor_fraction("item-read-epub".into(), 1, "end".into()).await.unwrap(), 0.0);
    assert_eq!(core.book_anchor_fraction("item-read-epub".into(), 1, "nowhere".into()).await.unwrap(), 0.0);
    let two = core.book_chapter("item-read-epub".into(), 1).await.unwrap();
    assert_eq!(lain_ffi::book_anchor_fraction_in(two, "end".into()), 0.0);
    assert!(matches!(
        core.book_anchor_fraction("item-read-epub".into(), 2, "end".into()).await,
        Err(LainError::NotFound { .. })
    ));
    assert!(matches!(
        core.book_anchor_fraction("item-read-epub".into(), -1, "end".into()).await,
        Err(LainError::Invalid { .. })
    ));

    // Progress: the end of the last chapter finishes the book, and the
    // place comes back from the saved record.
    let p = lain_ffi::book_progress_at(book.chapters.clone(), 0, 0.5);
    assert_eq!(p.duration, book.pages as f64);
    core.save_progress("item-read-epub".into(), p.position, p.duration, p.completed).await.unwrap();
    let saved = core.progress("item-read-epub".into()).await.unwrap();
    let place = lain_ffi::book_position_of(book.chapters.clone(), saved.position_sec, saved.completed);
    assert_eq!(place.chapter, 0);
    assert!(lain_ffi::book_progress_at(book.chapters.clone(), 1, 1.0).completed);

    proxy.set_online(false);
    assert!(core.book_chapter("item-read-epub".into(), 1).await.unwrap_err().is_offline());
    drop(core);
    drop(server);
    std::fs::remove_dir_all(root).ok();
}

/// Downloaded books read with no server (D-306): the manifest and a
/// chapter from the device's copy (resources inlined as `data:` URIs),
/// and a PDF's pages drawn on the device for the page reader.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn read_books_offline() {
    let root = temp("ffi-book-offline");
    let server = read_server(&root).await;
    let (_admin, _bob) = admin_and_bob(&server.url).await;
    let proxy = Proxy::start(server.addr).await;
    let core = core(&root);
    core.set_server(proxy.url.clone()).unwrap();
    core.login("bob".into(), pw(2)).await.unwrap();

    let online = core.book("item-read-epub".into()).await.unwrap();
    let online_pdf = core.comic("item-read-pdf".into()).await.unwrap();
    assert!(!online_pdf.offline);
    for id in ["item-read-epub", "item-read-pdf"] {
        core.offline_add(id.into()).await.unwrap();
        assert_eq!(core.offline_fetch(id.into(), None).await.unwrap().state, CopyState::Done);
    }
    let p = lain_ffi::book_progress_at(online.chapters.clone(), 0, 0.5);

    proxy.set_online(false);
    assert!(core.health().await.unwrap_err().is_offline());
    let book = core.book("item-read-epub".into()).await.unwrap();
    assert_eq!(
        (book.title.as_str(), book.layout.as_str(), &book.chapters, &book.authors),
        ("Tiny Book", "reflowable", &online.chapters, &online.authors)
    );
    assert_eq!(book.toc.len(), online.toc.len());
    let html = core.book_chapter("item-read-epub".into(), 0).await.unwrap();
    assert!(html.contains("src=\"data:image/png;base64,iVBORw0KGgo"), "{html}");
    assert!(!html.contains("/book/res/OEBPS/cover.png"), "{html}");
    assert!(html.contains("href=\"lain-chapter://1#end\" data-lain-chapter=\"1\""), "{html}");
    assert!(!html.contains("steal"));
    assert!(!core.book_chapter("item-read-epub".into(), 1).await.unwrap().is_empty());
    assert_eq!(core.book_anchor_fraction("item-read-epub".into(), 1, "end".into()).await.unwrap(), 0.0);
    assert!(matches!(
        core.book_anchor_fraction("item-read-epub".into(), 2, "end".into()).await,
        Err(LainError::NotFound { .. })
    ));
    assert!(matches!(core.book_chapter("item-read-epub".into(), 2).await, Err(LainError::NotFound { .. })));
    // Progress keeps going through the offline queue.
    let q = core.save_progress("item-read-epub".into(), p.position, p.duration, p.completed).await.unwrap();
    assert!(matches!(q, SaveOutcome::Queued { .. }), "{q:?}");

    // The PDF: its manifest says pages, and the page reader draws them.
    let pdf = core.book("item-read-pdf".into()).await.unwrap();
    assert_eq!((pdf.layout.as_str(), pdf.pages), ("pages", 2));
    let pages = core.comic("item-read-pdf".into()).await.unwrap();
    assert!(pages.offline);
    let shape = |b: &lain_ffi::ComicBook| b.pages.iter().map(|p| (p.index, p.width, p.height)).collect::<Vec<_>>();
    assert_eq!(shape(&pages), shape(&online_pdf));
    let page = core.comic_page("item-read-pdf".into(), 1).await.unwrap();
    assert!(page.starts_with(b"\x89PNG") || page.starts_with(b"\xff\xd8"), "an image");
    assert!(matches!(core.comic_page("item-read-pdf".into(), 2).await, Err(LainError::NotFound { .. })));
    assert!(matches!(core.comic_page("item-read-pdf".into(), -1).await, Err(LainError::NotFound { .. })));
    drop(core);
    drop(server);
    std::fs::remove_dir_all(root).ok();
}

/// The session token the core keeps for `server` (its token store file).
fn core_token(root: &std::path::Path, server: &str) -> String {
    let store = lain_client::FileTokenStore::new(root.join("app/sessions.json"));
    lain_client::TokenStore::load(&store, server).unwrap().expect("a session").token
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn events_push_queued_progress_on_connect() {
    let root = temp("ffi-events");
    let server = watch_server(&root).await;
    let (_admin, _bob) = admin_and_bob(&server.url).await;
    let proxy = Proxy::start(server.addr).await;
    let core = core(&root);
    core.set_server(proxy.url.clone()).unwrap();
    core.login("bob".into(), pw(2)).await.unwrap();

    proxy.set_online(false);
    assert!(matches!(core.save_progress(ITEM1.into(), 42., 1440., false).await.unwrap(), SaveOutcome::Queued { .. }));
    proxy.set_online(true);
    let ev = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            match core.next_event().await {
                Some(lain_ffi::CoreEvent::Connected { pushed }) => return pushed,
                Some(_) => continue,
                None => panic!("stream closed"),
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(ev, 1);
    assert_eq!(core.queued_progress().unwrap(), 0);
    // Stopping ends a waiting call instead of hanging it.
    let waiter = {
        let core = core.clone();
        tokio::spawn(async move {
            let mut n = 0;
            while core.next_event().await.is_some() {
                n += 1;
            }
            n
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    core.stop_events();
    let n = tokio::time::timeout(std::time::Duration::from_secs(5), waiter).await.unwrap().unwrap();
    assert!(n <= 1, "only the flush's own progress-changed can be pending: {n}");
    drop(core);
    drop(server);
    std::fs::remove_dir_all(root).ok();
}
