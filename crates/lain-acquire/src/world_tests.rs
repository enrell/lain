//! End-to-end tests of the manager (Go `manager_test.go`,
//! `automation_test.go`, `cleanup_test.go`, `metadata_race_test.go`).
//! Every network peer is in-process on loopback (A-13): a test tracker,
//! a seeding engine and a fake Torznab indexer.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lain_api::acquire::*;
use lain_api::downloads::Limits;
use lain_api::{CatalogItem, Library};
use lain_downloads::http::PlainHttp;
use lain_store::Store;
use lain_torrent::trackertest::Tracker;
use lain_torrent::{Client, Config, MetaInfo, SourceFile, Spec, build};

use crate::client::{AddRequest, ClientStatus, DownloadClient};
use crate::error::*;
use crate::importer::File;
use crate::manager::{Deps, GrabRequest, Manager, SearchQuery, default_settings};
use crate::testsupport::{Fixture, Reply, serve};
use crate::torznab::Torznab;
use crate::util::base_name;

pub(crate) fn payload(n: usize, seed: u8) -> Vec<u8> {
    let mut x = (seed as u32).wrapping_mul(2246822519).wrapping_add(7);
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

fn tmp(tag: &str) -> PathBuf {
    let d = lain_store::test_dir(tag);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn engine() -> Config {
    Config { min_announce_interval: Duration::from_secs(1), ..Config::default() }
}

/// Turns a hang into a failure: aborts the test binary, naming the
/// test, when a world outlives `LIMIT`.
pub(crate) struct Watchdog(Arc<std::sync::atomic::AtomicBool>);

impl Watchdog {
    const LIMIT: Duration = Duration::from_secs(90);

    pub fn new() -> Watchdog {
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let d2 = done.clone();
        let name = std::thread::current().name().unwrap_or("?").to_string();
        std::thread::spawn(move || {
            let start = Instant::now();
            while start.elapsed() < Self::LIMIT {
                if d2.load(std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            eprintln!("test {name} hung for {:?}; aborting", Self::LIMIT);
            std::process::abort();
        });
        Watchdog(done)
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

pub(crate) struct World {
    pub _watchdog: Watchdog,
    /// Kept alive for the swarm.
    pub _tracker: Tracker,
    pub seed: Option<Client>,
    pub mi: MetaInfo,
    pub files: Vec<SourceFile>,
    pub index: Fixture,
    pub lib: Library,
    pub rescans: Arc<Mutex<Vec<String>>>,
    pub limits: Arc<Mutex<Limits>>,
}

impl World {
    #[allow(clippy::type_complexity)]
    pub fn new() -> World {
        let tracker = Tracker::new().unwrap();
        let files = vec![
            SourceFile { path: vec!["[Fansub-A] Show - 01 [1080p].mkv".into()], data: payload(120_000, 1) },
            SourceFile { path: vec!["[Fansub-A] Show - 02 [1080p].mkv".into()], data: payload(90_000, 2) },
            SourceFile { path: vec!["sample.mkv".into()], data: payload(500, 3) },
            SourceFile { path: vec!["info.nfo".into()], data: b"placeholder".to_vec() },
        ];
        let (raw, mi) = build("[Fansub-A] Show - 01-02 [1080p]", &files, 32 << 10, &[tracker.http_url()]).unwrap();
        let seed_dir = tmp("acq-seed");
        for f in &files {
            let p = seed_dir.join(&mi.info.name).join(f.path.join("/"));
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, &f.data).unwrap();
        }
        let seed = Client::new(Config { listen_addr: Some("127.0.0.1:0".into()), ..engine() }).unwrap();
        seed.add(Spec { meta_info: Some(mi.clone()), dir: seed_dir, ..Spec::default() }).unwrap();
        let shared: Arc<Mutex<(Vec<u8>, String, String, i64)>> =
            Arc::new(Mutex::new((raw.clone(), mi.info_hash.hex(), mi.info.name.clone(), mi.info.length)));
        let tr_url = tracker.http_url();
        let base: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
        let (sh, b2) = (shared.clone(), base.clone());
        let index = serve(move |s| {
            let (raw, ih, name, len) = sh.lock().unwrap().clone();
            let base = b2.lock().unwrap().clone();
            if s.path == "/dl/show.torrent" {
                return Reply::ok(raw);
            }
            if s.path == "/dl/magnet" {
                return Reply::redirect(&format!("magnet:?xt=urn:btih:{ih}&tr={tr_url}"));
            }
            if s.param("apikey") != "k" {
                return Reply::ok(r#"<error code="100" description="bad key"/>"#);
            }
            if s.param("t") == "caps" {
                return Reply::ok(
                    r#"<caps><searching><search available="yes"/><tv-search available="yes"/></searching><categories><category id="5070" name="Anime"/></categories></caps>"#,
                );
            }
            Reply::ok(format!(
                r#"<rss><channel>
<item><title>{name}</title><link>{base}/dl/show.torrent</link><size>{len}</size><torznab:attr name="seeders" value="1"/></item>
<item><title>[Fansub-B] Other Show - 01 [720p]</title><link>{base}/dl/none.torrent</link><torznab:attr name="seeders" value="99"/></item>
</channel></rss>"#
            ))
        });
        *base.lock().unwrap() = index.url.clone();
        let lib = Library {
            id: "lib-anime".into(),
            name: "Anime".into(),
            kind: "anime".into(),
            path: tmp("acq-lib").to_string_lossy().into(),
            ..Default::default()
        };
        World {
            _watchdog: Watchdog::new(),
            _tracker: tracker,
            seed: Some(seed),
            mi,
            files,
            index,
            lib,
            rescans: Arc::default(),
            limits: Arc::new(Mutex::new(Limits { max_bytes: 50 << 20, min_free_bytes: 0 })),
        }
    }

    /// The catalog: every `.mkv` under the library root, read with the
    /// release tokenizer, as a rescan would record it.
    pub fn library_items(lib: &Library) -> Vec<CatalogItem> {
        let mut out = Vec::new();
        let mut stack = vec![PathBuf::from(&lib.path)];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                let ps = p.to_string_lossy().to_string();
                if !ps.ends_with(".mkv") {
                    continue;
                }
                let r = lain_identify::tokenize(&base_name(&ps), &lib.kind);
                out.push(CatalogItem {
                    id: ps.clone(),
                    library_id: lib.id.clone(),
                    title: r.title,
                    season: r.season,
                    episode: r.episodes.first().copied().unwrap_or(0),
                    file_path: ps,
                    ..Default::default()
                });
            }
        }
        out
    }

    pub fn deps(&self, data: &Path, s: AcquireSettings) -> Deps {
        let store = Arc::new(Store::open(data).unwrap());
        let http: Arc<PlainHttp> = Arc::new(PlainHttp::new());
        let mut d = Deps::new(store, data, http.clone());
        let tz = Arc::new(Torznab::new(http));
        let tz2 = tz.clone();
        d.search = Arc::new(move |i: &IndexerSearchInput| Ok(tz.search(i)?.results));
        d.caps = Arc::new(move |ix: &Indexer| Ok(tz2.caps(ix)?));
        let lim = self.limits.clone();
        d.budget = Some(Arc::new(move || (*lim.lock().unwrap(), 0)));
        let lib = self.lib.clone();
        let l2 = lib.clone();
        d.library = Arc::new(move |id: &str| (id == lib.id).then(|| lib.clone()));
        let l3 = l2.clone();
        d.libraries = Arc::new(move || vec![l2.clone()]);
        d.items = Some(Arc::new(move |_: &str| World::library_items(&l3)));
        d.titles = Some(Arc::new(|_: &str| vec!["SHOW".to_string()]));
        let rs = self.rescans.clone();
        d.rescan = Some(Arc::new(move |id: &str| rs.lock().unwrap().push(id.into())));
        d.tick = Duration::from_millis(100);
        d.listen_host = "127.0.0.1".into();
        d.engine = Some(engine());
        let mut s = s;
        s.dir = data.join("acquire").to_string_lossy().into();
        d.initial = Some(s);
        d
    }

    pub fn manager_with(&self, data: &Path, s: AcquireSettings, tweak: impl FnOnce(&mut Deps)) -> Arc<Manager> {
        let mut d = self.deps(data, s);
        tweak(&mut d);
        let m = Manager::open(d).unwrap();
        m.start();
        m
    }

    pub fn manager(&self, data: &Path, s: AcquireSettings) -> Arc<Manager> {
        self.manager_with(data, s, |_| {})
    }

    pub fn torrent_url(&self) -> String {
        format!("{}/dl/show.torrent", self.index.url)
    }

    pub fn magnet_url(&self) -> String {
        format!("{}/dl/magnet", self.index.url)
    }

    pub fn assert_imported(&self) {
        let show = Path::new(&self.lib.path).join("SHOW");
        for (i, name) in ["[Fansub-A] SHOW - 01.mkv", "[Fansub-A] SHOW - 02.mkv"].iter().enumerate() {
            let got = std::fs::read(show.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(got == self.files[i].data, "{name} differs");
        }
        let n = std::fs::read_dir(&show).unwrap().count();
        assert_eq!(n, 2, "library holds {n} files, want 2 (sample and nfo skipped)");
        assert_eq!(self.rescans.lock().unwrap().first().map(String::as_str), Some(self.lib.id.as_str()));
    }
}

pub(crate) fn settings(ratio: f64) -> AcquireSettings {
    AcquireSettings { listen_port: 0, seed_ratio: ratio, seed_minutes: 0, ..default_settings(Path::new("/unused")) }
}

pub(crate) fn url_grab(url: &str, lib: &str) -> GrabRequest {
    GrabRequest {
        input: GrabInput { url: url.into(), library_id: lib.into(), ..Default::default() },
        ..Default::default()
    }
}

pub(crate) fn wait_grab(m: &Manager, id: &str, states: &[&str]) -> Grab {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        let g = m.get(id).unwrap();
        if states.contains(&g.state.as_str()) {
            return g;
        }
        assert!(g.state != GRAB_FAILED, "grab failed: {} {}", g.code, g.error);
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("grab state {:?}, want {states:?}", m.get(id).unwrap());
}

pub(crate) fn wait_failed(m: &Manager, id: &str) -> Grab {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        let g = m.get(id).unwrap();
        if g.state == GRAB_FAILED {
            return g;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("never failed: {:?}", m.get(id).unwrap());
}

pub(crate) fn add_indexer(m: &Manager, w: &World) -> Indexer {
    let ix = m
        .create_indexer(&IndexerInput {
            name: Some("tracker-exemplo".into()),
            protocol: Some(INDEXER_TORZNAB.into()),
            url: Some(w.index.url.clone()),
            api_key: Some("k".into()),
            ..Default::default()
        })
        .unwrap();
    m.update_indexer(&ix.id, &IndexerInput { min_interval_ms: Some(0), ..Default::default() }).unwrap();
    ix
}

pub(crate) fn monitor(m: &Manager, w: &World, mon: Monitored) -> Monitored {
    m.create_monitored(Monitored { library_id: w.lib.id.clone(), kind: w.lib.kind.clone(), ..mon }).unwrap()
}

fn mon(title: &str, from: i64, to: i64) -> Monitored {
    Monitored { title: title.into(), from, to, ..Default::default() }
}

#[cfg(unix)]
fn nlink(p: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(p).unwrap().nlink()
}

// ---- manager -------------------------------------------------------------

#[test]
fn search_grab_import_without_seeding() {
    let w = World::new();
    let m = w.manager(&tmp("acq-m1"), settings(0.0));
    let ix = m
        .create_indexer(&IndexerInput {
            name: Some("tracker-exemplo".into()),
            protocol: Some(INDEXER_TORZNAB.into()),
            url: Some(w.index.url.clone()),
            api_key: Some("k".into()),
            ..Default::default()
        })
        .unwrap();
    assert!(ix.api_key.is_empty() && m.has_api_key(&ix.id), "the key must be stored but never returned");
    let (tested, err) = m.test_indexer(&ix.id);
    assert!(err.is_none(), "{err:?}");
    assert!(tested.unwrap().caps.is_some_and(|c| c.tv_search));
    let res = m.search(&SearchQuery { query: "Show".into(), kind: "anime".into(), ..Default::default() }).unwrap();
    assert!(res.failures.is_empty() && res.candidates.len() == 2, "{res:?}");
    let best = res.candidates[0].clone();
    assert_eq!(best.release.title, "Show");
    assert!(best.rejections.is_empty() && best.release.episodes.len() == 2, "{:?}", res.candidates);
    assert!(!res.candidates[1].rejections.is_empty(), "another show must be marked as not matching");

    let g = m
        .grab(GrabRequest {
            input: GrabInput { result: Some(best.result), library_id: w.lib.id.clone(), ..Default::default() },
            ..Default::default()
        })
        .unwrap();
    assert_eq!((g.size, g.info_hash.as_str()), (w.mi.info.length, w.mi.info_hash.hex().as_str()));
    let done = wait_grab(&m, &g.id, &[GRAB_DONE]);
    w.assert_imported();
    assert!(done.data_removed && done.imported.len() == 2, "{done:?}");
    assert!(!Path::new(&m.settings().dir).join(&g.id).join(&w.mi.info.name).exists(), "torrent data left behind");
    assert_eq!(m.usage().used_bytes, 0);
    m.close();
}

#[test]
fn seeding_stops_at_ratio() {
    let mut w = World::new();
    let m = w.manager(&tmp("acq-m2"), settings(1.0));
    let g = m.grab(url_grab(&w.torrent_url(), &w.lib.id)).unwrap();
    wait_grab(&m, &g.id, &[GRAB_SEEDING]);
    w.assert_imported();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let a = std::fs::metadata(Path::new(&w.lib.path).join("SHOW/[Fansub-A] SHOW - 01.mkv")).unwrap();
        let b =
            std::fs::metadata(Path::new(&m.settings().dir).join(&g.id).join(&w.mi.info.name).join(&w.files[0].path[0]))
                .unwrap();
        assert_eq!(a.ino(), b.ino(), "import must hardlink on the same filesystem");
    }
    assert_eq!(m.usage().used_bytes, w.mi.info.length, "seeding data counts against the budget");
    // The original seeder leaves; a fresh leecher finds lain through the
    // tracker.
    w.seed.take().unwrap().close();
    let leech = Client::new(Config { listen_addr: Some("127.0.0.1:0".into()), ..engine() }).unwrap();
    leech.add(Spec { meta_info: Some(w.mi.clone()), dir: tmp("acq-leech"), ..Spec::default() }).unwrap();
    let fin = wait_grab(&m, &g.id, &[GRAB_DONE]);
    assert!(fin.uploaded >= fin.size && fin.data_removed, "{fin:?}");
    assert!(Path::new(&w.lib.path).join("SHOW/[Fansub-A] SHOW - 01.mkv").exists());
    leech.close();
    m.close();
}

#[test]
fn magnet_redirect_from_indexer() {
    let w = World::new();
    let m = w.manager(&tmp("acq-m3"), settings(0.0));
    let g = m.grab(url_grab(&w.magnet_url(), &w.lib.id)).unwrap();
    assert_eq!((g.source.as_str(), g.state.as_str()), ("magnet", GRAB_METADATA));
    let done = wait_grab(&m, &g.id, &[GRAB_DONE]);
    assert_eq!(done.size, w.mi.info.length);
    w.assert_imported();
    m.close();
}

#[test]
fn budget_refuses_big_grabs() {
    let w = World::new();
    w.limits.lock().unwrap().max_bytes = 1000;
    let m = w.manager(&tmp("acq-m4"), settings(0.0));
    assert_eq!(m.grab(url_grab(&w.torrent_url(), &w.lib.id)).unwrap_err().code, CODE_QUOTA);
    assert!(m.grabs().is_empty(), "a refused grab must not be recorded");
    // Magnets are refused only when the store is already full; the
    // metadata check fails them once their size is known.
    let g = m.grab(url_grab(&w.magnet_url(), &w.lib.id)).unwrap();
    let failed = wait_failed(&m, &g.id);
    assert!(failed.code == CODE_QUOTA && failed.data_removed, "{failed:?}");
    m.close();
}

#[test]
fn grab_validation() {
    let w = World::new();
    let m = w.manager(&tmp("acq-m5"), settings(0.0));
    let ih = w.mi.info_hash.hex();
    let lib = w.lib.id.clone();
    let cases = [
        (url_grab(&w.torrent_url(), "nope"), CODE_NO_LIBRARY),
        (url_grab("", &lib), CODE_INVALID),
        (
            GrabRequest {
                input: GrabInput {
                    magnet: format!("magnet:?xt=urn:btih:{ih}"),
                    library_id: lib.clone(),
                    ..Default::default()
                },
                ..Default::default()
            },
            CODE_UNSUPPORTED,
        ),
        (url_grab("file:///etc/passwd", &lib), CODE_INVALID),
        (url_grab(&format!("{}/dl/none.torrent", w.index.url), &lib), CODE_FETCH),
        (
            GrabRequest {
                input: GrabInput {
                    result: Some(SearchResult {
                        protocol: PROTOCOL_USENET.into(),
                        link: "http://x/y.nzb".into(),
                        ..Default::default()
                    }),
                    library_id: lib.clone(),
                    ..Default::default()
                },
                ..Default::default()
            },
            CODE_UNSUPPORTED,
        ),
    ];
    for (input, code) in cases {
        assert_eq!(m.grab(input.clone()).unwrap_err().code, code, "{:?}", input.input);
    }
    m.grab(url_grab(&w.torrent_url(), &lib)).unwrap();
    assert_eq!(m.grab(url_grab(&w.torrent_url(), &lib)).unwrap_err().code, CODE_STATE, "duplicate");
    m.close();
}

#[test]
fn restart_resumes_a_paused_grab() {
    let w = World::new();
    // Hold the seeder back: a fast local swarm could finish before the
    // pause, and a finished download imports.
    let seeding = w.seed.as_ref().unwrap().torrent(&w.mi.info_hash).unwrap();
    seeding.pause();
    let data = tmp("acq-m6");
    let m1 = w.manager(&data, settings(0.0));
    let g = m1.grab(url_grab(&w.torrent_url(), &w.lib.id)).unwrap();
    m1.pause(&g.id).unwrap();
    m1.close();
    drop(m1);
    let m2 = w.manager(&data, settings(0.0));
    assert_eq!(m2.get(&g.id).unwrap().state, GRAB_PAUSED);
    seeding.resume();
    m2.resume(&g.id).unwrap();
    wait_grab(&m2, &g.id, &[GRAB_DONE]);
    w.assert_imported();
    m2.close();
}

#[test]
fn indexer_crud_and_keys() {
    let w = World::new();
    let m = w.manager(&tmp("acq-m7"), settings(0.0));
    let input = |url: &str, key: Option<&str>| IndexerInput {
        name: Some("x".into()),
        protocol: Some(INDEXER_TORZNAB.into()),
        url: Some(url.into()),
        api_key: key.map(Into::into),
        ..Default::default()
    };
    for u in ["ftp://x", "http://user:pass@x/", "not a url"] {
        assert_eq!(m.create_indexer(&input(u, None)).unwrap_err().code, CODE_INVALID, "{u}");
    }
    let ix = m.create_indexer(&input(&w.index.url, Some("first"))).unwrap();
    m.update_indexer(&ix.id, &IndexerInput { api_key: Some(String::new()), ..Default::default() }).unwrap();
    assert!(m.has_api_key(&ix.id), "an empty key in a patch keeps the stored key");
    m.update_indexer(&ix.id, &IndexerInput { api_key: Some("-".into()), ..Default::default() }).unwrap();
    assert!(!m.has_api_key(&ix.id), r#""-" clears the key"#);
    // The test answers the probe's error and records it.
    let (got, err) = m.test_indexer(&ix.id);
    assert_eq!(err.unwrap().code, "auth-failed");
    assert!(got.unwrap().last_error.contains("auth-failed"));
    assert_eq!(m.test_indexer("nope").0.unwrap_err().code, CODE_NOT_FOUND);
    m.delete_indexer(&ix.id).unwrap();
    assert_eq!(m.delete_indexer(&ix.id).unwrap_err().code, CODE_NOT_FOUND);
    assert_eq!(m.search(&SearchQuery { query: "Show".into(), ..Default::default() }).unwrap_err().code, CODE_NOT_FOUND);
    assert_eq!(m.search(&SearchQuery::default()).unwrap_err().code, CODE_INVALID);
    m.close();
}

// ---- automation ------------------------------------------------------------

#[test]
fn search_monitored_grabs_until_complete() {
    let w = World::new();
    let m = w.manager(&tmp("acq-a1"), settings(0.0));
    add_indexer(&m, &w);
    let mo = monitor(&m, &w, mon("Show", 1, 2));
    assert_eq!(m.wanted_for(&mo.id).unwrap().missing.len(), 2);
    let rep = m.search_monitored(&mo.id).unwrap();
    assert_eq!(rep.grabbed.len(), 1, "{rep:?}");
    let g = wait_grab(&m, &rep.grabbed[0], &[GRAB_DONE]);
    assert!(g.monitored_id == mo.id && g.units.len() == 2, "{g:?}");
    assert!(m.wanted_for(&mo.id).unwrap().missing.is_empty());
    assert!(m.search_monitored(&mo.id).unwrap().grabbed.is_empty(), "a complete title must not grab again");
    // The quality ledger outlives grabs: clearing the queue must not make
    // the imported files look upgradable.
    m.remove(&g.id, true).unwrap();
    assert!(m.search_monitored(&mo.id).unwrap().grabbed.is_empty());
    m.close();
}

#[test]
fn search_refreshes_the_metadata_count() {
    let w = World::new();
    let asked = Arc::new(Mutex::new(String::new()));
    let a2 = asked.clone();
    let m = w.manager_with(&tmp("acq-a2"), settings(0.0), |d| {
        d.episode_count = Some(Arc::new(move |title: &str, kind: &str| {
            *a2.lock().unwrap() = format!("{kind}:{title}");
            4
        }))
    });
    add_indexer(&m, &w);
    let mo = monitor(&m, &w, mon("Show", 0, 0));
    m.search_monitored(&mo.id).unwrap();
    assert_eq!(m.monitored(&mo.id).unwrap().metadata_episodes, 4);
    assert_eq!(*asked.lock().unwrap(), "anime:Show");
    m.close();
}

#[test]
fn search_skips_units_already_being_grabbed() {
    let w = World::new();
    let m = w.manager(&tmp("acq-a3"), AcquireSettings { max_active: 1, ..settings(0.0) });
    add_indexer(&m, &w);
    let mo = monitor(&m, &w, mon("Show", 1, 2));
    let first = m.search_monitored(&mo.id).unwrap();
    let second = m.search_monitored(&mo.id).unwrap();
    assert!(first.grabbed.len() == 1 && second.grabbed.is_empty(), "{first:?} then {second:?}");
    m.close();
}

/// A parser that reads the world's release as the title "Show 01" and
/// offers the right identity only as an alternative (D-279).
fn misreading(name: &str, kind: &str) -> lain_api::Release {
    let mut r = lain_identify::tokenize(name, kind);
    if r.title == "Show" && !r.episodes.is_empty() {
        r.alternatives = vec![
            lain_api::ReleaseIdentity { title: "Show 01 02".into(), ..Default::default() },
            lain_api::ReleaseIdentity {
                title: "Show".into(),
                episodes: std::mem::take(&mut r.episodes),
                absolute: true,
                ..Default::default()
            },
        ];
        r.title = "Show 01".into();
    }
    r
}

#[test]
fn a_search_reads_a_release_as_the_title_it_asked_for() {
    let w = World::new();
    let m = w.manager_with(&tmp("acq-alt1"), settings(0.0), |d| d.parse = Arc::new(misreading));
    add_indexer(&m, &w);
    let res = m.search(&SearchQuery { query: "Show".into(), kind: "anime".into(), ..Default::default() }).unwrap();
    let best = &res.candidates[0];
    assert_eq!((best.release.title.as_str(), best.release.episodes.len()), ("Show", 2), "{:?}", res.candidates);
    assert!(best.rejections.is_empty() && best.release.alternatives.is_empty(), "{best:?}");
    assert!(!res.candidates[1].rejections.is_empty(), "another show still does not match");
    // Asked for another title, no reading fits: the answer stands and is refused.
    let res =
        m.search(&SearchQuery { query: "Silver Tide".into(), kind: "anime".into(), ..Default::default() }).unwrap();
    assert!(res.candidates.iter().all(|c| c.rejections.iter().any(|r| r == "title does not match")));
    assert!(res.candidates.iter().any(|c| c.release.title == "Show 01"), "{:?}", res.candidates);
    m.close();
}

#[test]
fn a_monitored_title_decides_between_readings() {
    let w = World::new();
    let m = w.manager_with(&tmp("acq-alt2"), settings(0.0), |d| d.parse = Arc::new(misreading));
    add_indexer(&m, &w);
    let mo = monitor(&m, &w, mon("Show", 0, 0));
    let rep = m.rss_sync().unwrap();
    assert_eq!(rep.grabbed.len(), 1, "{rep:?}");
    let g = wait_grab(&m, &rep.grabbed[0], &[GRAB_DONE]);
    assert_eq!(g.release.title, "Show", "the grab is stored as the monitored title");
    assert!(g.release.alternatives.is_empty());
    w.assert_imported();
    assert_eq!(m.wanted_for(&mo.id).unwrap().open_from, Some(Unit { season: 0, number: 3 }));
    m.close();
}

/// A grab made by hand names no title (`GrabInput` does not carry the
/// search query), so the parser's answer stands, its advice is not
/// stored, and the library's titles do not pull the files into a folder
/// the release did not name.
#[test]
fn a_manual_grab_settles_nothing() {
    let w = World::new();
    let m = w.manager_with(&tmp("acq-alt3"), settings(0.0), |d| d.parse = Arc::new(misreading));
    add_indexer(&m, &w);
    let res = m.search(&SearchQuery { query: "Show".into(), kind: "anime".into(), ..Default::default() }).unwrap();
    let best = res.candidates[0].clone();
    assert_eq!(best.release.title, "Show", "the search reads it as asked");
    let g = m
        .grab(GrabRequest {
            input: GrabInput { result: Some(best.result), library_id: w.lib.id.clone(), ..Default::default() },
            ..Default::default()
        })
        .unwrap();
    assert_eq!((g.release.title.as_str(), g.release.episodes.len()), ("Show 01", 0), "{:?}", g.release);
    assert!(g.release.alternatives.is_empty(), "advice is not stored");
    let g = wait_grab(&m, &g.id, &[GRAB_DONE]);
    assert_eq!(g.release.title, "Show 01");
    let lib = Path::new(&w.lib.path);
    assert!(lib.join("Show 01/[Fansub-A] Show - 01 [1080p].mkv").is_file(), "filed under the answer");
    assert!(!lib.join("SHOW").exists(), "not under the library's title the release did not name");
    m.close();
}

/// A parser that reads the world's release as episodes 1-2 of the
/// sequel "Show 2" (2017), and offers "Show", season 2, as its other
/// reading (D-279).
fn sequel_reading(name: &str, kind: &str) -> lain_api::Release {
    let mut r = lain_identify::tokenize(name, kind);
    if r.title == "Show" && !r.episodes.is_empty() {
        r.alternatives = vec![lain_api::ReleaseIdentity {
            title: "Show".into(),
            year: 2017,
            season: 2,
            episodes: r.episodes.clone(),
            ..Default::default()
        }];
        r.title = "Show 2".into();
        r.year = 2017;
    }
    r
}

/// "Show" with its second season numbered from 13, so that S2E1-2 is a
/// unit it wants.
fn show_with_two_seasons() -> Monitored {
    let map = vec![SeasonStart { season: 1, first: 1 }, SeasonStart { season: 2, first: 13 }];
    Monitored { season_map: map, ..mon("Show", 0, 0) }
}

/// Original and sequel both monitored: the release the parser answers as
/// the sequel is the sequel's, and the original never reads it as its
/// season 2 (D-279).
#[test]
fn a_monitored_sequel_keeps_its_release() {
    let w = World::new();
    let m = w.manager_with(&tmp("acq-alt4"), settings(0.0), |d| d.parse = Arc::new(sequel_reading));
    add_indexer(&m, &w);
    let show = monitor(&m, &w, show_with_two_seasons());
    let sequel = monitor(&m, &w, mon("Show 2", 0, 0));
    assert!(m.search_monitored(&show.id).unwrap().grabbed.is_empty(), "the original took the sequel's release");
    let rep = m.rss_sync().unwrap();
    assert_eq!(rep.grabbed.len(), 1, "{rep:?}");
    let g = m.get(&rep.grabbed[0]).unwrap();
    assert_eq!(g.monitored_id, sequel.id, "grabbed for {:?}", g.monitored_id);
    let r = &g.release;
    assert_eq!((r.title.as_str(), r.season, r.episodes.clone()), ("Show 2", 0, vec![1, 2]), "{r:?}");
    m.close();
}

/// The same rule on the grab path: a grab made for "Show" settles its
/// release against "Show" only while no monitored title is named by the
/// parser's answer.
#[test]
fn a_grab_for_a_monitored_title_respects_the_others() {
    for sequel_monitored in [false, true] {
        let w = World::new();
        let m = w.manager_with(&tmp("acq-alt5"), settings(0.0), |d| d.parse = Arc::new(sequel_reading));
        let show = monitor(&m, &w, show_with_two_seasons());
        if sequel_monitored {
            monitor(&m, &w, mon("Show 2", 0, 0));
        }
        let req = GrabRequest { monitored_id: show.id.clone(), ..url_grab(&w.torrent_url(), &w.lib.id) };
        let r = m.grab(req).unwrap().release;
        let want = if sequel_monitored { ("Show 2", 0) } else { ("Show", 2) };
        assert_eq!((r.title.as_str(), r.season), want, "sequel monitored: {sequel_monitored}");
        assert_eq!(r.episodes, vec![1, 2]);
        assert!(r.alternatives.is_empty());
        m.close();
    }
}

/// A search's year decides between readings and rejects another year
/// (D-279): one either way is the same release, an unknown one never
/// rejects.
#[test]
fn a_search_compares_years() {
    let w = World::new();
    let m = w.manager_with(&tmp("acq-alt6"), settings(0.0), |d| d.parse = Arc::new(sequel_reading));
    add_indexer(&m, &w);
    let ours = |query: &str, year: i64| {
        let q = SearchQuery { query: query.into(), kind: "anime".into(), year, ..Default::default() };
        let res = m.search(&q).unwrap();
        let other = res.candidates.iter().find(|c| c.result.title != w.mi.info.name).unwrap();
        assert!(!other.rejections.iter().any(|r| r.starts_with("year")), "an unknown year: {other:?}");
        let c = res.candidates.into_iter().find(|c| c.result.title == w.mi.info.name).unwrap();
        (c.release.title, c.release.season, c.rejections)
    };
    let no: Vec<String> = Vec::new();
    let year = || "year does not match".to_string();
    assert_eq!(ours("Show", 0), ("Show".into(), 2, no.clone()));
    assert_eq!(ours("Show", 2016), ("Show".into(), 2, no.clone()));
    assert_eq!(ours("Show", 2010), ("Show 2".into(), 0, vec!["title does not match".into(), year()]));
    assert_eq!(ours("Show 2", 2018), ("Show 2".into(), 0, no.clone()));
    assert_eq!(ours("Show 2", 2010), ("Show 2".into(), 0, vec![year()]));
    m.close();
}

#[test]
fn rss_sync_feeds_monitored_titles() {
    let w = World::new();
    let m = w.manager(&tmp("acq-a4"), settings(0.0));
    add_indexer(&m, &w);
    let mo = monitor(&m, &w, mon("Show", 0, 0));
    let rep = m.rss_sync().unwrap();
    assert_eq!(rep.grabbed.len(), 1, "{rep:?}");
    wait_grab(&m, &rep.grabbed[0], &[GRAB_DONE]);
    w.assert_imported();
    assert_eq!(m.wanted_for(&mo.id).unwrap().open_from, Some(Unit { season: 0, number: 3 }));
    assert!(m.automation_status().last_rss_at > 0);
    m.close();
}

#[test]
fn stalled_grab_is_blocklisted_and_not_retried() {
    let mut w = World::new();
    w.seed.take().unwrap().close(); // nobody seeds: the download never moves
    let m = w.manager_with(&tmp("acq-a5"), settings(0.0), |d| d.stall_after = Some(Duration::from_millis(400)));
    add_indexer(&m, &w);
    let mo = monitor(&m, &w, mon("Show", 1, 2));
    let rep = m.search_monitored(&mo.id).unwrap();
    assert_eq!(rep.grabbed.len(), 1);
    let g = wait_failed(&m, &rep.grabbed[0]);
    assert!(g.code == CODE_STALLED && g.data_removed, "{g:?}");
    assert_eq!(g.error, "no progress for 400ms");
    let bl = m.blocklist();
    assert!(bl.len() == 1 && bl[0].info_hash == w.mi.info_hash.hex() && bl[0].monitored_id == mo.id, "{bl:?}");
    assert!(m.search_monitored(&mo.id).unwrap().grabbed.is_empty(), "a blocklisted release was grabbed again");
    m.unblock(&bl[0].id).unwrap();
    assert!(m.blocklist().is_empty());
    m.close();
}

#[test]
fn upgrade_replaces_below_cutoff_and_keeps_the_old_file() {
    let w = World::new();
    let mut old = Vec::new();
    for n in ["[Fansub-B] SHOW - 01 [720p].mkv", "[Fansub-B] SHOW - 02 [720p].mkv"] {
        let p = Path::new(&w.lib.path).join("SHOW").join(n);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, format!("old {n}")).unwrap();
        old.push(p);
    }
    let m = w.manager(&tmp("acq-a6"), settings(0.0));
    add_indexer(&m, &w);
    let mo = monitor(&m, &w, mon("Show", 1, 2));
    let wanted = m.wanted_for(&mo.id).unwrap();
    assert!(wanted.missing.is_empty() && wanted.upgrades.len() == 2, "{wanted:?}");
    let rep = m.search_monitored(&mo.id).unwrap();
    assert_eq!(rep.grabbed.len(), 1, "{rep:?}");
    let g = wait_grab(&m, &rep.grabbed[0], &[GRAB_DONE]);
    assert!(g.upgrade && g.replaced.len() == 2, "{g:?}");
    w.assert_imported();
    for p in &old {
        assert!(!p.exists(), "{} should have left the library", p.display());
    }
    let held = m.replaced();
    assert_eq!(held.len(), 2, "{held:?}");
    for h in &held {
        assert!(std::fs::read_to_string(&h.path).unwrap().starts_with("old "));
        assert_eq!(h.grab_id, g.id);
    }
    assert!(m.wanted_for(&mo.id).unwrap().upgrades.is_empty(), "1080p meets the cutoff");
    assert_eq!(m.purge_replaced().unwrap(), 2);
    assert!(m.replaced().is_empty());
    m.close();
}

#[test]
fn automation_pauses_on_a_full_budget() {
    let w = World::new();
    w.limits.lock().unwrap().max_bytes = 1000;
    let m = w.manager(&tmp("acq-a7"), settings(0.0));
    add_indexer(&m, &w);
    let mo = monitor(&m, &w, mon("Show", 1, 2));
    let rep = m.search_monitored(&mo.id).unwrap();
    assert!(rep.grabbed.is_empty() && !rep.paused.is_empty(), "{rep:?}");
    assert!(m.automation_status().paused.contains("budget"));
    m.close();
}

#[test]
fn monitored_bookkeeping() {
    let w = World::new();
    let m = w.manager(&tmp("acq-a8"), settings(0.0));
    let film =
        Monitored { library_id: w.lib.id.clone(), kind: "movie".into(), title: "Film".into(), ..Default::default() };
    assert_eq!(m.create_monitored(film).unwrap_err().code, CODE_INVALID, "kind must match the library type");
    let mut mo = monitor(&m, &w, mon("Show", 0, 0));
    assert!(mo.profile_id == "default" && mo.enabled, "{mo:?}");
    let dup =
        Monitored { library_id: w.lib.id.clone(), kind: "anime".into(), title: "SHOW!".into(), ..Default::default() };
    assert_eq!(m.create_monitored(dup).unwrap_err().code, CODE_STATE);
    let bad = Monitored {
        library_id: w.lib.id.clone(),
        kind: "anime".into(),
        title: "Other".into(),
        profile_id: "nope".into(),
        ..Default::default()
    };
    assert_eq!(m.create_monitored(bad).unwrap_err().code, CODE_NOT_FOUND);
    let p = m
        .create_profile(Profile { name: "4K only".into(), resolutions: vec!["2160p".into()], ..Default::default() })
        .unwrap();
    mo.profile_id = p.id.clone();
    m.update_monitored(&mo.id, mo.clone()).unwrap();
    assert_eq!(m.delete_profile(&p.id).unwrap_err().code, CODE_STATE);
    assert_eq!(m.delete_profile("default").unwrap_err().code, CODE_STATE);
    m.delete_monitored(&mo.id).unwrap();
    m.delete_profile(&p.id).unwrap();
    assert_eq!(m.profiles().len(), 1);
    m.close();
}

#[test]
fn automation_schedule() {
    use crate::automation::{rss_due, search_due};
    let mut s = default_settings(Path::new("/d"));
    let now = 1_800_000_000;
    assert!(!rss_due(&s, None, now), "automation is off by default");
    s.automation = true;
    assert!(rss_due(&s, None, now) && !rss_due(&s, Some(now - 60), now) && rss_due(&s, Some(now - 31 * 60), now));
    let mut mo = Monitored { enabled: true, last_search_at: now - 13 * 3600, ..Default::default() };
    assert!(search_due(&s, &mo, now));
    mo.last_search_at = now - 3600;
    assert!(!search_due(&s, &mo, now));
    s.search_hours = 0;
    mo.last_search_at = 0;
    assert!(!search_due(&s, &mo, now), "search_hours 0 means on demand only");
}

#[test]
fn import_failure_is_blocklisted_but_keeps_data() {
    let mut w = World::new();
    w.lib.kind = "manga".into(); // a video release cannot be imported here
    let m = w.manager(&tmp("acq-a9"), settings(0.0));
    let g = m.grab(url_grab(&w.torrent_url(), &w.lib.id)).unwrap();
    let failed = wait_failed(&m, &g.id);
    assert!(failed.code == CODE_IMPORT && !failed.data_removed, "{failed:?}");
    let bl = m.blocklist();
    assert!(bl.len() == 1 && bl[0].info_hash == w.mi.info_hash.hex(), "{bl:?}");
    // A manual retry is allowed (and fails the same way here).
    m.retry_import(&g.id).unwrap();
    wait_failed(&m, &g.id);
    m.close();
}

// ---- cleanup (D-112) ----------------------------------------------------------

fn library_files(w: &World) -> Vec<PathBuf> {
    ["[Fansub-A] SHOW - 01.mkv", "[Fansub-A] SHOW - 02.mkv"]
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let p = Path::new(&w.lib.path).join("SHOW").join(n);
            assert!(std::fs::read(&p).unwrap() == w.files[i].data, "library file {} changed", p.display());
            p
        })
        .collect()
}

#[test]
fn cleanup_after_seeding_keeps_hardlinked_library_files() {
    let w = World::new();
    let m = w.manager(&tmp("acq-c1"), AcquireSettings { seed_minutes: 1, ..settings(0.0) });
    let g = m.grab(url_grab(&w.torrent_url(), &w.lib.id)).unwrap();
    wait_grab(&m, &g.id, &[GRAB_SEEDING]);
    let lib = library_files(&w);
    let copy = Path::new(&m.settings().dir).join(&g.id).join(&w.mi.info.name).join(&w.files[0].path[0]);
    #[cfg(unix)]
    for p in &lib {
        assert_eq!(nlink(p), 2, "{} should be hardlinked to the seeding copy", p.display());
    }
    // Pretend the seeding window is over.
    let mut got = m.st.grab(&g.id).unwrap();
    got.seeding_at = crate::util::now() - 120;
    m.st.put_grab(&mut got).unwrap();
    let done = wait_grab(&m, &g.id, &[GRAB_DONE]);
    assert!(done.data_removed, "{done:?}");
    assert!(!copy.exists(), "torrent copy survived");
    #[cfg(unix)]
    for p in library_files(&w) {
        assert_eq!(nlink(&p), 1, "the library name must be the one that remains");
    }
    let _ = lib;
    m.close();
}

#[test]
fn cleanup_after_move_import_keeps_library_files() {
    let w = World::new();
    let m = w.manager(&tmp("acq-c2"), AcquireSettings { import_mode: IMPORT_MOVE.into(), ..settings(0.0) });
    let g = m.grab(url_grab(&w.torrent_url(), &w.lib.id)).unwrap();
    let done = wait_grab(&m, &g.id, &[GRAB_DONE]);
    library_files(&w);
    assert!(done.data_removed, "{done:?}");
    assert!(!Path::new(&m.settings().dir).join(&g.id).exists(), "grab folder survived cleanup");
    m.remove(&g.id, true).unwrap();
    library_files(&w);
    m.close();
}

#[test]
fn cleanup_never_deletes_inside_a_library_root() {
    let mut w = World::new();
    let data = tmp("acq-c3");
    // A hostile layout: the library root contains the acquisition folder.
    w.lib.path = data.to_string_lossy().into();
    let m = w.manager(&data, settings(0.0));
    let g = m.grab(url_grab(&w.torrent_url(), &w.lib.id)).unwrap();
    let done = wait_grab(&m, &g.id, &[GRAB_DONE]);
    library_files(&w);
    let copy = Path::new(&m.settings().dir).join(&g.id).join(&w.mi.info.name).join(&w.files[0].path[0]);
    assert!(copy.exists(), "a file inside a library root must never be deleted");
    assert!(!done.data_removed);
    m.remove(&g.id, true).unwrap();
    assert!(copy.exists(), "removing the grab deleted a file inside a library");
    m.close();
}

#[test]
fn settings_refuse_a_download_folder_inside_a_library() {
    let w = World::new();
    let m = w.manager(&tmp("acq-c4"), settings(0.0));
    let lib = PathBuf::from(&w.lib.path);
    for dir in [lib.join("downloads"), lib.parent().unwrap().to_path_buf(), lib.clone()] {
        let s = AcquireSettings { dir: dir.to_string_lossy().into(), ..m.settings() };
        assert_eq!(m.set_settings(s).unwrap_err().code, CODE_INVALID, "{}", dir.display());
    }
    // Rates apply live; a new port restarts the engine on another port.
    let before = m.port();
    let s = AcquireSettings { upload_kbps: 10, max_peers: 41, ..m.settings() };
    let s = m.set_settings(s).unwrap();
    assert_eq!(m.settings(), s);
    assert!(m.port() > 0 && before > 0);
    m.close();
}

#[test]
fn failed_download_cleanup_spares_libraries() {
    let w = World::new();
    w.limits.lock().unwrap().max_bytes = 1000;
    let m = w.manager(&tmp("acq-c5"), settings(0.0));
    let keep = Path::new(&w.lib.path).join("SHOW/keep.mkv");
    std::fs::create_dir_all(keep.parent().unwrap()).unwrap();
    std::fs::write(&keep, b"mine").unwrap();
    let g = m.grab(url_grab(&w.magnet_url(), &w.lib.id)).unwrap();
    wait_failed(&m, &g.id);
    assert_eq!(std::fs::read(&keep).unwrap(), b"mine");
    m.close();
}

// ---- the metadata race --------------------------------------------------------

/// Reports a magnet's metadata while `add` is still running, before the
/// grab is stored: a fast local swarm does this on CI.
struct EarlyClient {
    m: std::sync::Weak<Manager>,
}

const EARLY_HASH: &str = "a1134d724672bffc94d1c361edb4a04ff3fdf61a";

impl DownloadClient for EarlyClient {
    fn name(&self) -> &str {
        "early"
    }
    fn add(&self, _: AddRequest) -> Result<(String, i64)> {
        let m = self.m.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            if let Some(m) = m.upgrade() {
                m.on_metadata(EARLY_HASH, 1 << 30);
            }
            let _ = tx.send(());
        });
        // Unfixed, the event finds no grab and returns at once; fixed, it
        // waits for the stored record.
        let _ = rx.recv_timeout(Duration::from_millis(200));
        Ok((EARLY_HASH.into(), 0))
    }
    fn status(&self, _: &str) -> Option<ClientStatus> {
        None
    }
    fn files(&self, _: &str) -> Vec<File> {
        Vec::new()
    }
    fn pause(&self, _: &str) -> Result<()> {
        Ok(())
    }
    fn resume(&self, _: &str) -> Result<()> {
        Ok(())
    }
    fn remove(&self, _: &str, _: bool) -> Result<()> {
        Ok(())
    }
    fn set_rates(&self, _: i64, _: i64) {}
    fn checkpoint(&self) {}
    fn port(&self) -> u16 {
        0
    }
    fn close(&self) {}
}

#[test]
fn metadata_before_the_grab_is_stored_still_checks_the_budget() {
    let w = World::new();
    w.limits.lock().unwrap().max_bytes = 1000;
    let m = w.manager(&tmp("acq-r1"), settings(0.0));
    m.set_client(Arc::new(EarlyClient { m: Arc::downgrade(&m) }));
    let req = GrabRequest {
        input: GrabInput {
            magnet: format!("magnet:?xt=urn:btih:{EARLY_HASH}&tr=http://127.0.0.1:1/announce"),
            library_id: w.lib.id.clone(),
            ..Default::default()
        },
        ..Default::default()
    };
    let g = m.grab(req).unwrap();
    let got = wait_failed(&m, &g.id);
    assert_eq!(got.code, CODE_QUOTA, "a 1 GiB magnet must fail the 1000-byte budget: {got:?}");
    m.close();
}

/// Reports completion before metadata (the order Windows runners saw):
/// the grab is sized and budget-checked at completion.
struct LateMetadataClient {
    m: std::sync::Weak<Manager>,
}

impl DownloadClient for LateMetadataClient {
    fn name(&self) -> &str {
        "late"
    }
    fn add(&self, _: AddRequest) -> Result<(String, i64)> {
        let m = self.m.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            if let Some(m) = m.upgrade() {
                m.on_complete_for_tests(EARLY_HASH);
                m.on_metadata(EARLY_HASH, 1 << 30);
            }
        });
        Ok((EARLY_HASH.into(), 0))
    }
    fn status(&self, _: &str) -> Option<ClientStatus> {
        Some(ClientStatus { state: "seeding".into(), size: 1 << 30, ..Default::default() })
    }
    fn files(&self, _: &str) -> Vec<File> {
        Vec::new()
    }
    fn pause(&self, _: &str) -> Result<()> {
        Ok(())
    }
    fn resume(&self, _: &str) -> Result<()> {
        Ok(())
    }
    fn remove(&self, _: &str, _: bool) -> Result<()> {
        Ok(())
    }
    fn set_rates(&self, _: i64, _: i64) {}
    fn checkpoint(&self) {}
    fn port(&self) -> u16 {
        0
    }
    fn close(&self) {}
}

#[test]
fn completion_before_metadata_still_checks_the_budget() {
    let w = World::new();
    w.limits.lock().unwrap().max_bytes = 1000;
    let m = w.manager(&tmp("acq-r2"), settings(0.0));
    m.set_client(Arc::new(LateMetadataClient { m: Arc::downgrade(&m) }));
    let req = GrabRequest {
        input: GrabInput {
            magnet: format!("magnet:?xt=urn:btih:{EARLY_HASH}&tr=http://127.0.0.1:1/announce"),
            library_id: w.lib.id.clone(),
            ..Default::default()
        },
        ..Default::default()
    };
    let g = m.grab(req).unwrap();
    let got = wait_failed(&m, &g.id);
    assert!(got.code == CODE_QUOTA && got.size == 1 << 30, "{got:?}");
    m.close();
}
