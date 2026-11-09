//! The acquisition manager: settings, indexers, search, grabs on the
//! download client, the monitor loop (live stats, stalls, seeding
//! limits, queue slots), import into libraries and cleanup of torrent
//! copies. Phase 2 automation lives in `automation.rs`, Phase 3
//! subtitles in `subtitles.rs`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use lain_api::acquire::{
    AcquireSettings, AcquireUsage, Candidate, GRAB_DONE, GRAB_DOWNLOADING, GRAB_FAILED, GRAB_IMPORTING, GRAB_METADATA,
    GRAB_PAUSED, GRAB_QUEUED, GRAB_SEEDING, Grab, GrabInput, IMPORT_COPY, IMPORT_HARDLINK, IMPORT_MOVE,
    INDEXER_NEWZNAB, INDEXER_TORZNAB, Indexer, IndexerCaps, IndexerFailure, IndexerInput, IndexerSearchInput,
    PROTOCOL_TORRENT, PROTOCOL_USENET, SEARCH_BOOK, SEARCH_GENERIC, SEARCH_MOVIE, SEARCH_TV, SearchResult,
    SearchResults, Unit,
};
use lain_api::downloads::Limits;
use lain_api::{CatalogItem, Library, Release};
use lain_downloads::http::{Http, Request};
use lain_store::{Bucket, Store};

use crate::client::{AddRequest, ClientEvents, ClientStatus, DownloadClient, Native, NativeClient, NativeConfig};
use crate::errf;
use crate::error::*;
use crate::importer::{Parser, place, plan_import};
use crate::store::AcqStore;
use crate::util::{clean_name, clip, http_url, loose_key, new_id, now, url_host, within_or_equal};

const MAX_TORRENT_FILE: usize = 10 << 20;

pub type SearchFn = dyn Fn(&IndexerSearchInput) -> Result<Vec<SearchResult>> + Send + Sync;
pub type CapsFn = dyn Fn(&Indexer) -> Result<IndexerCaps> + Send + Sync;
pub type BudgetFn = dyn Fn() -> (Limits, i64) + Send + Sync;
pub type LibraryFn = dyn Fn(&str) -> Option<Library> + Send + Sync;
pub type LibrariesFn = dyn Fn() -> Vec<Library> + Send + Sync;
pub type TitlesFn = dyn Fn(&str) -> Vec<String> + Send + Sync;
pub type RescanFn = dyn Fn(&str) + Send + Sync;
pub type ItemsFn = dyn Fn(&str) -> Vec<CatalogItem> + Send + Sync;
pub type EpisodeCountFn = dyn Fn(&str, &str) -> i64 + Send + Sync;
pub type GrabHook = dyn Fn(&Grab) + Send + Sync;

/// What the manager needs from the rest of lain. The server wires these
/// to the registry and the library store; tests pass fakes.
#[derive(Clone)]
pub struct Deps {
    pub store: Arc<Store>,
    pub data_dir: PathBuf,
    pub parse: Arc<Parser>,
    pub search: Arc<SearchFn>,
    pub caps: Arc<CapsFn>,
    /// The shared download limits and the bytes the HTTP download
    /// manager already uses (A-4).
    pub budget: Option<Arc<BudgetFn>>,
    /// Free space of a directory; `None` reads the OS (`lain-downloads`).
    pub free_space: Option<lain_downloads::FreeSpace>,
    pub library: Arc<LibraryFn>,
    /// Every library: cleanup never deletes inside one.
    pub libraries: Arc<LibrariesFn>,
    /// A library's existing titles, for import matching.
    pub titles: Option<Arc<TitlesFn>>,
    pub rescan: Option<Arc<RescanFn>>,
    /// A library's catalog items (what is present).
    pub items: Option<Arc<ItemsFn>>,
    /// The metadata providers' episode total (0 = unknown, A-18).
    pub episode_count: Option<Arc<EpisodeCountFn>>,
    pub http: Arc<dyn Http>,
    /// Engine timings (tests); `listen_host` binds inbound.
    pub engine: Option<lain_torrent::Config>,
    pub listen_host: String,
    /// The monitor period (default 2 s).
    pub tick: Duration,
    /// Replaces the defaults when no settings are saved yet.
    pub initial: Option<AcquireSettings>,
    /// Overrides the stall window (tests).
    pub stall_after: Option<Duration>,
    /// Phase 3: the probe and `lain.subtitle@1`.
    pub subtitles: crate::subtitles::SubtitleDeps,
    /// Called after every stored grab change (the `grab-changed` event).
    pub on_grab: Option<Arc<GrabHook>>,
}

impl Deps {
    /// Minimal deps over a store; everything optional is off.
    pub fn new(store: Arc<Store>, data_dir: &Path, http: Arc<dyn Http>) -> Deps {
        Deps {
            store,
            data_dir: data_dir.to_path_buf(),
            parse: Arc::new(|n: &str, k: &str| lain_identify::tokenize(n, k)),
            search: Arc::new(|_: &IndexerSearchInput| {
                Err(errf!(CODE_UNSUPPORTED, "no indexer provider speaks this protocol"))
            }),
            caps: Arc::new(|_: &Indexer| Err(errf!(CODE_UNSUPPORTED, "no indexer provider speaks this protocol"))),
            budget: None,
            free_space: None,
            library: Arc::new(|_: &str| None),
            libraries: Arc::new(Vec::new),
            titles: None,
            rescan: None,
            items: None,
            episode_count: None,
            http,
            engine: None,
            listen_host: String::new(),
            tick: Duration::from_secs(2),
            initial: None,
            stall_after: None,
            subtitles: Default::default(),
            on_grab: None,
        }
    }
}

/// The defaults fit a small disk: seed to 1.0 or a day.
pub fn default_settings(data_dir: &Path) -> AcquireSettings {
    AcquireSettings {
        dir: data_dir.join("acquire").to_string_lossy().into(),
        listen_port: 51413,
        max_active: 3,
        max_peers: 40,
        seed_ratio: 1.0,
        seed_minutes: 24 * 60,
        remove_after_seeding: true,
        import_mode: IMPORT_HARDLINK.into(),
        rss_minutes: 30,
        search_hours: 12,
        stall_hours: 6,
        subtitle_hours: 24,
        ..AcquireSettings::default()
    }
}

/// Normalizes settings or explains why they cannot be saved.
pub fn validate_settings(mut s: AcquireSettings) -> Result<AcquireSettings> {
    s.dir = s.dir.trim().to_string();
    if s.dir.is_empty() || !Path::new(&s.dir).is_absolute() {
        return Err(errf!(CODE_INVALID, "dir must be an absolute path"));
    }
    s.dir = crate::util::clean_path(Path::new(&s.dir)).to_string_lossy().into();
    if !(0..=65535).contains(&s.listen_port) {
        return Err(errf!(CODE_INVALID, "listen_port must be 0-65535"));
    }
    if !(1..=20).contains(&s.max_active) {
        return Err(errf!(CODE_INVALID, "max_active must be 1-20"));
    }
    if !(1..=200).contains(&s.max_peers) {
        return Err(errf!(CODE_INVALID, "max_peers must be 1-200"));
    }
    if s.upload_kbps < 0 || s.download_kbps < 0 {
        return Err(errf!(CODE_INVALID, "rates cannot be negative"));
    }
    if !(0.0..=100.0).contains(&s.seed_ratio) || !(0..=525600).contains(&s.seed_minutes) {
        return Err(errf!(CODE_INVALID, "seed limits out of range"));
    }
    if s.rss_minutes == 0 {
        s.rss_minutes = 30;
    }
    if !(10..=1440).contains(&s.rss_minutes) {
        return Err(errf!(CODE_INVALID, "rss_minutes must be 10-1440"));
    }
    if !(0..=168).contains(&s.search_hours) {
        return Err(errf!(CODE_INVALID, "search_hours must be 0-168"));
    }
    if !(0..=168).contains(&s.stall_hours) {
        return Err(errf!(CODE_INVALID, "stall_hours must be 0-168"));
    }
    if !(0..=720).contains(&s.subtitle_hours) {
        return Err(errf!(CODE_INVALID, "subtitle_hours must be 0-720"));
    }
    match s.import_mode.as_str() {
        IMPORT_HARDLINK | IMPORT_COPY | IMPORT_MOVE => {}
        "" => s.import_mode = IMPORT_HARDLINK.into(),
        _ => return Err(errf!(CODE_INVALID, "import_mode must be hardlink, copy or move")),
    }
    Ok(s)
}

/// Whether the policy seeds at all.
pub fn seeds(s: &AcquireSettings) -> bool {
    s.seed_ratio > 0.0 || s.seed_minutes > 0
}

/// A manual search (`GET /api/acquire/search`).
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    pub query: String,
    /// The library type: anime, series, movie, manga, comic.
    pub kind: String,
    pub season: i64,
    pub episode: i64,
    pub year: i64,
    pub indexer_ids: Vec<String>,
}

pub(crate) fn search_kind(kind: &str) -> &'static str {
    match kind {
        "anime" | "series" => SEARCH_TV,
        "movie" => SEARCH_MOVIE,
        "manga" | "comic" => SEARCH_BOOK,
        _ => SEARCH_GENERIC,
    }
}

/// A grab request from automation (the fields clients never send).
#[derive(Debug, Clone, Default)]
pub struct GrabRequest {
    pub input: GrabInput,
    pub created_by: String,
    pub monitored_id: String,
    pub units: Vec<Unit>,
    pub replace_from: Vec<String>,
}

pub(crate) struct Mark {
    pub bytes: i64,
    pub at: Instant,
}

pub(crate) struct Inner {
    pub settings: AcquireSettings,
    pub client: Option<Arc<dyn DownloadClient>>,
    /// By grab id.
    pub live: HashMap<String, ClientStatus>,
    /// Persisted downloaded/uploaded at session start.
    pub base: HashMap<String, [i64; 2]>,
    /// Indexer id → last request.
    pub last_call: HashMap<String, Instant>,
    pub importing: HashSet<String>,
    pub marks: HashMap<String, Mark>,
    pub last_rss: Option<i64>,
    pub auto_busy: bool,
    pub auto_paused: String,
}

/// Runs acquisition.
pub struct Manager {
    pub(crate) d: Deps,
    pub(crate) st: AcqStore,
    pub(crate) inner: Mutex<Inner>,
    /// Serializes grabs so two cannot both pass the budget check for the
    /// same free bytes; client events take it too, so an event racing
    /// `add` waits until the grab is stored.
    pub(crate) grab_mu: Mutex<()>,
    me: Weak<Manager>,
    stop: AtomicBool,
    wake: (Mutex<bool>, Condvar),
    pub(crate) threads: Mutex<Vec<JoinHandle<()>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Manager {
    /// Opens the tables and starts the download client. Grabs that were
    /// importing when the process stopped import again.
    pub fn open(d: Deps) -> Result<Arc<Manager>> {
        let st = AcqStore { db: d.store.clone() };
        let def = match &d.initial {
            Some(i) => validate_settings(i.clone())?,
            None => validate_settings(default_settings(&d.data_dir))?,
        };
        let settings = st.settings(def);
        let m = Arc::new_cyclic(|me| Manager {
            d,
            st,
            inner: Mutex::new(Inner {
                settings,
                client: None,
                live: HashMap::new(),
                base: HashMap::new(),
                last_call: HashMap::new(),
                importing: HashSet::new(),
                marks: HashMap::new(),
                last_rss: None,
                auto_busy: false,
                auto_paused: String::new(),
            }),
            grab_mu: Mutex::new(()),
            me: me.clone(),
            stop: AtomicBool::new(false),
            wake: (Mutex::new(false), Condvar::new()),
            threads: Mutex::new(Vec::new()),
        });
        m.start_client()?;
        for g in m.st.grabs() {
            lock(&m.inner).base.insert(g.id.clone(), [g.downloaded, g.uploaded]);
            if g.state == GRAB_IMPORTING {
                // Interrupted mid-import: placement is idempotent.
                m.queue_import(&g.id);
            }
        }
        Ok(m)
    }

    fn start_client(&self) -> Result<()> {
        let s = self.settings();
        let (w1, w2) = (self.me.clone(), self.me.clone());
        let events = ClientEvents {
            on_metadata: Some(Arc::new(move |ih: &str, size: i64| {
                if let Some(m) = w1.upgrade() {
                    m.on_metadata(ih, size);
                }
            })),
            on_complete: Some(Arc::new(move |ih: &str| {
                if let Some(m) = w2.upgrade() {
                    m.on_complete(ih);
                }
            })),
        };
        let cfg = NativeConfig {
            listen_port: s.listen_port as u16,
            listen_host: self.d.listen_host.clone(),
            max_peers: s.max_peers as usize,
            upload_bps: s.upload_kbps * 1024,
            download_bps: s.download_kbps * 1024,
            engine: self.d.engine.clone(),
        };
        let n = Native::new(self.st.clone(), cfg, events).map_err(|e| errf!(&e.code, "torrent engine: {}", e.msg))?;
        lock(&self.inner).client = Some(Arc::new(NativeClient(n)));
        Ok(())
    }

    pub(crate) fn client(&self) -> Arc<dyn DownloadClient> {
        lock(&self.inner).client.clone().expect("the client starts in open")
    }

    /// The engine's inbound port.
    pub fn port(&self) -> u16 {
        lock(&self.inner).client.as_ref().map(|c| c.port()).unwrap_or(0)
    }

    /// Starts the monitor loop.
    pub fn start(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let tick = self.d.tick;
        let h = std::thread::Builder::new()
            .name("acquire-monitor".into())
            .spawn(move || {
                let mut n = 0u64;
                loop {
                    let Some(m) = weak.upgrade() else { return };
                    if !m.sleep(tick) {
                        return;
                    }
                    n += 1;
                    m.monitor();
                    m.schedule(now());
                    if n.is_multiple_of(15) {
                        m.client().checkpoint();
                        m.persist_live();
                    }
                }
            })
            .ok();
        if let Some(h) = h {
            lock(&self.threads).push(h);
        }
    }

    /// Waits one tick; false once closing.
    fn sleep(&self, d: Duration) -> bool {
        let (mu, cv) = &self.wake;
        let g = lock(mu);
        let (g, _) = cv.wait_timeout_while(g, d, |stop| !*stop).unwrap_or_else(|e| e.into_inner());
        !*g
    }

    pub(crate) fn stopping(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// Runs `f` on a tracked thread (imports, subtitle passes).
    pub(crate) fn spawn(&self, name: &str, f: impl FnOnce(Arc<Manager>) + Send + 'static) {
        if self.stopping() {
            return;
        }
        let Some(me) = self.me.upgrade() else { return };
        if let Ok(h) = std::thread::Builder::new().name(name.into()).spawn(move || f(me)) {
            let mut ts = lock(&self.threads);
            ts.retain(|t| !t.is_finished());
            ts.push(h);
        }
    }

    /// Stops the loop and the client.
    pub fn close(&self) {
        if self.stop.swap(true, Ordering::SeqCst) {
            return;
        }
        {
            let (mu, cv) = &self.wake;
            *lock(mu) = true;
            cv.notify_all();
        }
        let threads: Vec<JoinHandle<()>> = std::mem::take(&mut *lock(&self.threads));
        let me = std::thread::current().id();
        for t in threads {
            if t.thread().id() != me {
                let _ = t.join();
            }
        }
        self.persist_live();
        if let Some(c) = lock(&self.inner).client.take() {
            c.close();
        }
    }

    pub(crate) fn save(&self, g: &mut Grab) {
        if let Err(e) = self.st.put_grab(g) {
            tracing::warn!(grab = %g.id, err = %e, "grab write failed");
        }
        self.emit(g);
    }

    fn emit(&self, g: &Grab) {
        if let Some(f) = &self.d.on_grab {
            f(&self.with_live(g.clone()));
        }
    }

    /// Replaces the download client (tests drive the manager through a
    /// fake one).
    #[cfg(test)]
    pub(crate) fn set_client(&self, c: Arc<dyn DownloadClient>) {
        lock(&self.inner).client = Some(c);
    }

    // ---- settings -------------------------------------------------------

    pub fn settings(&self) -> AcquireSettings {
        lock(&self.inner).settings.clone()
    }

    /// Validates and applies a policy. Rates apply live; a new port or
    /// peer limit restarts the engine (torrents resume from disk).
    pub fn set_settings(&self, s: AcquireSettings) -> Result<AcquireSettings> {
        let s = validate_settings(s)?;
        if let Some(name) = overlaps_library(Path::new(&s.dir), &(self.d.libraries)()) {
            return Err(errf!(
                CODE_INVALID,
                "the download folder must not be, contain or lie inside a library ({name})"
            ));
        }
        self.st.save_settings(&s)?;
        let (old, c) = {
            let mut i = lock(&self.inner);
            let old = std::mem::replace(&mut i.settings, s.clone());
            (old, i.client.clone())
        };
        if let Some(c) = &c {
            c.set_rates(s.upload_kbps * 1024, s.download_kbps * 1024);
        }
        if old.listen_port != s.listen_port || old.max_peers != s.max_peers {
            self.persist_live();
            if let Some(c) = lock(&self.inner).client.take() {
                c.close();
            }
            self.start_client()?;
        }
        Ok(s)
    }

    fn used_bytes(&self, except: &str) -> i64 {
        self.st.grabs().iter().filter(|g| g.id != except && g.on_disk()).map(|g| g.size).sum()
    }

    fn budget(&self) -> (Limits, i64) {
        self.d.budget.as_ref().map(|f| f()).unwrap_or_default()
    }

    pub fn usage(&self) -> AcquireUsage {
        let (lim, other) = self.budget();
        AcquireUsage {
            used_bytes: self.used_bytes(""),
            downloads_bytes: other,
            max_bytes: lim.max_bytes,
            min_free_bytes: lim.min_free_bytes,
        }
    }

    fn free_space(&self) -> lain_downloads::FreeSpace {
        self.d.free_space.unwrap_or(lain_downloads::disk_free)
    }

    /// Whether `size` more bytes fit the shared budget (A-4).
    fn reserve(&self, except: &str, size: i64) -> Result<()> {
        let (lim, other) = self.budget();
        let dir = PathBuf::from(self.settings().dir);
        std::fs::create_dir_all(&dir).map_err(|e| errf!(CODE_DISK_FULL, "cannot create {}: {e}", dir.display()))?;
        let free = self.free_space();
        lain_downloads::check_with(&lim, &dir, other + self.used_bytes(except), size, free).map_err(|e| {
            let code = if e.code == lain_downloads::CODE_DISK_FULL { CODE_DISK_FULL } else { CODE_QUOTA };
            AcqError::new(code, e.msg)
        })
    }

    // ---- indexers -------------------------------------------------------

    /// Indexers without credentials.
    pub fn indexers(&self) -> Vec<Indexer> {
        self.st.indexers().iter().map(Indexer::public).collect()
    }

    pub fn has_api_key(&self, id: &str) -> bool {
        self.st.get::<Indexer>(Bucket::AcqIndexers, id, "indexer").is_ok_and(|ix| !ix.api_key.is_empty())
    }

    /// Adds an indexer (enabled, 2 s minimum interval).
    pub fn create_indexer(&self, input: &IndexerInput) -> Result<Indexer> {
        let mut ix =
            Indexer { id: new_id(), enabled: true, min_interval_ms: 2000, created_at: now(), ..Default::default() };
        apply_indexer(input, &mut ix)?;
        self.st.put(Bucket::AcqIndexers, &ix.id, &ix)?;
        Ok(ix.public())
    }

    /// Patches an indexer; an empty `api_key` keeps the old one.
    pub fn update_indexer(&self, id: &str, input: &IndexerInput) -> Result<Indexer> {
        let mut ix: Indexer = self.st.get(Bucket::AcqIndexers, id, "indexer")?;
        apply_indexer(input, &mut ix)?;
        self.st.put(Bucket::AcqIndexers, id, &ix)?;
        Ok(ix.public())
    }

    pub fn delete_indexer(&self, id: &str) -> Result<()> {
        self.st.delete(Bucket::AcqIndexers, id, "indexer")
    }

    /// Enforces the indexer's minimum interval (A-8) until `deadline`.
    pub(crate) fn wait(&self, ix: &Indexer, deadline: Instant) -> Result<()> {
        let gap = Duration::from_millis(ix.min_interval_ms.max(0) as u64);
        loop {
            let pause = {
                let mut i = lock(&self.inner);
                let now = Instant::now();
                match i.last_call.get(&ix.id).map(|t| *t + gap) {
                    Some(next) if now < next => next - now,
                    _ => {
                        i.last_call.insert(ix.id.clone(), now);
                        return Ok(());
                    }
                }
            };
            if Instant::now() + pause > deadline {
                return Err(errf!(CODE_STATE, "rate limit wait exceeded the request time"));
            }
            std::thread::sleep(pause);
        }
    }

    /// Probes caps and records health. The error is the probe's own.
    pub fn test_indexer(&self, id: &str) -> (Result<Indexer>, Option<AcqError>) {
        let mut ix: Indexer = match self.st.get(Bucket::AcqIndexers, id, "indexer") {
            Ok(ix) => ix,
            Err(e) => return (Err(e), None),
        };
        if let Err(e) = self.wait(&ix, Instant::now() + Duration::from_secs(30)) {
            return (Ok(ix.public()), Some(e));
        }
        let res = (self.d.caps)(&ix);
        ix.last_check_at = now();
        let err = match res {
            Ok(caps) => {
                ix.last_error.clear();
                ix.caps = Some(caps);
                None
            }
            Err(e) => {
                ix.last_error = e.to_string();
                Some(e)
            }
        };
        if let Err(e) = self.st.put(Bucket::AcqIndexers, id, &ix) {
            return (Ok(ix.public()), Some(e));
        }
        (Ok(ix.public()), err)
    }

    /// Queries indexers concurrently, parses and ranks the results.
    pub fn search(&self, q: &SearchQuery) -> Result<SearchResults> {
        let text = q.query.trim().to_string();
        if text.is_empty() || text.len() > 200 {
            return Err(errf!(CODE_INVALID, "a query of 1-200 characters is required"));
        }
        let targets: Vec<Indexer> = self
            .st
            .indexers()
            .into_iter()
            .filter(|ix| ix.enabled && (q.indexer_ids.is_empty() || q.indexer_ids.contains(&ix.id)))
            .collect();
        if targets.is_empty() {
            return Err(errf!(CODE_NOT_FOUND, "no enabled indexer; add one under Settings › Indexers"));
        }
        let deadline = Instant::now() + Duration::from_secs(45);
        let q = SearchQuery { query: text, ..q.clone() };
        let answers: Vec<(Indexer, Result<Vec<SearchResult>>)> = std::thread::scope(|scope| {
            let handles: Vec<_> = targets
                .into_iter()
                .map(|ix| {
                    let q = &q;
                    scope.spawn(move || {
                        let res = self.wait(&ix, deadline).and_then(|_| {
                            (self.d.search)(&IndexerSearchInput {
                                indexer: ix.clone(),
                                query: q.query.clone(),
                                kind: search_kind(&q.kind).into(),
                                season: q.season,
                                episode: q.episode,
                                year: q.year,
                                limit: 100,
                                ..Default::default()
                            })
                        });
                        (ix, res)
                    })
                })
                .collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        let mut out = SearchResults::default();
        for (ix, res) in answers {
            match res {
                Ok(rs) => out.candidates.extend(rs.into_iter().map(|r| self.rank(&q, r))),
                Err(e) => out.failures.push(IndexerFailure { indexer_id: ix.id, name: ix.name, error: e.to_string() }),
            }
        }
        out.candidates.sort_by(|a, b| {
            b.rejections
                .is_empty()
                .cmp(&a.rejections.is_empty())
                .then(b.score.cmp(&a.score))
                .then(b.result.seeders.cmp(&a.result.seeders))
        });
        Ok(out)
    }

    /// The Phase 1 score: fit to the query first, then swarm health and
    /// quality.
    fn rank(&self, q: &SearchQuery, r: SearchResult) -> Candidate {
        let rel = (self.d.parse)(&r.title, &q.kind);
        let mut rejections = Vec::new();
        if loose_key(&rel.title) != loose_key(&q.query) {
            rejections.push("title does not match".to_string());
        }
        if q.season > 0 && rel.season > 0 && rel.season != q.season {
            rejections.push(format!("season {}, wanted {}", rel.season, q.season));
        }
        if q.episode > 0 && !rel.episodes.is_empty() && !rel.episodes.contains(&q.episode) {
            rejections.push(format!("episode not in release, wanted {}", q.episode));
        }
        if r.protocol == PROTOCOL_USENET {
            rejections.push("usenet: no usenet client yet".into());
        }
        if r.protocol == PROTOCOL_TORRENT && r.seeders == 0 {
            rejections.push("no seeders".into());
        }
        let score = crate::decide::search_score(&rel, &r);
        Candidate { result: r, release: rel, score, rejections }
    }

    // ---- grabs ----------------------------------------------------------

    /// Downloads a `.torrent`; indexers (and Jackett/Prowlarr) may answer
    /// with a redirect to a magnet instead. Answers the body or a magnet.
    fn fetch_torrent(&self, link: &str) -> Result<(Vec<u8>, String)> {
        if !http_url(link) {
            return Err(errf!(CODE_INVALID, "the link must be http(s)"));
        }
        let host = url_host(link);
        let req = Request {
            method: "GET".into(),
            url: link.into(),
            headers: vec![("User-Agent".into(), "Lain".into())],
            timeout: Duration::from_secs(30),
            follow_redirects: true,
            cancel: None,
        };
        // The URL may carry an API key: report the host only.
        let res = self.d.http.send(&req).map_err(|_| errf!(CODE_FETCH, "{host} did not answer"))?;
        if (300..400).contains(&res.status) {
            if let Some(loc) = res.header("location")
                && loc.trim().starts_with("magnet:")
            {
                return Ok((Vec::new(), loc.trim().to_string()));
            }
            return Err(errf!(CODE_FETCH, "{host} did not answer"));
        }
        if res.status != 200 {
            return Err(errf!(CODE_FETCH, "{host} answered {}", res.status));
        }
        let body = res
            .read_all(MAX_TORRENT_FILE)
            .map_err(|_| errf!(CODE_FETCH, "the .torrent from {host} is unreadable or too large"))?;
        let text = String::from_utf8_lossy(&body);
        if text.trim().starts_with("magnet:") {
            return Ok((Vec::new(), text.trim().to_string()));
        }
        Ok((body, String::new()))
    }

    /// Starts acquiring a release into a library.
    pub fn grab(&self, req: GrabRequest) -> Result<Grab> {
        let input = req.input;
        let lib = (self.d.library)(&input.library_id)
            .ok_or_else(|| errf!(CODE_NO_LIBRARY, "choose the library to import into"))?;
        let mut g = Grab {
            id: new_id(),
            library_id: lib.id.clone(),
            kind: lib.kind.clone(),
            created_by: req.created_by,
            created_at: now(),
            title: input.title.trim().to_string(),
            monitored_id: req.monitored_id,
            units: req.units,
            upgrade: !req.replace_from.is_empty(),
            replace_from: req.replace_from,
            ..Default::default()
        };
        let (mut link, mut magnet) = (input.url.trim().to_string(), input.magnet.trim().to_string());
        if let Some(r) = &input.result {
            if r.protocol == PROTOCOL_USENET {
                return Err(errf!(
                    CODE_UNSUPPORTED,
                    "usenet releases need a usenet client, which Lain does not have yet"
                ));
            }
            g.indexer_id = r.indexer_id.clone();
            g.title = r.title.clone();
            link = r.link.clone();
            magnet = r.magnet.clone();
        }
        let mut body = Vec::new();
        if !link.is_empty() && magnet.is_empty() {
            (body, magnet) = self.fetch_torrent(&link)?;
        }
        if body.is_empty() && magnet.is_empty() {
            return Err(errf!(CODE_INVALID, "a search result, a .torrent URL or a magnet is required"));
        }
        if !body.is_empty() {
            let mi = lain_torrent::parse_metainfo(&body).map_err(|_| errf!(CODE_FETCH, "not a valid .torrent"))?;
            g.source = "torrent-url".into();
            g.size = mi.info.length as i64;
            if g.title.is_empty() {
                g.title = mi.info.name.clone();
            }
            self.reserve("", g.size)?;
        } else {
            let mg = lain_torrent::parse_magnet(&magnet).map_err(|_| errf!(CODE_INVALID, "bad magnet link"))?;
            g.source = "magnet".into();
            g.magnet = magnet.clone();
            if g.title.is_empty() {
                g.title = mg.name.clone();
            }
            // Size unknown until metadata: refuse only a store already full.
            let (lim, other) = self.budget();
            let free = self.free_space();
            lain_downloads::full_with(&lim, Path::new(&self.settings().dir), other + self.used_bytes(""), free)
                .map_err(|e| AcqError::new(CODE_QUOTA, e.to_string()))?;
        }
        let _g = lock(&self.grab_mu);
        if !body.is_empty() {
            // Again under the lock: another grab may have taken the room.
            self.reserve("", g.size)?;
        }
        if g.title.is_empty() {
            g.title = "Untitled".into();
        }
        g.title = clip(&g.title, 300);
        g.release = (self.d.parse)(&g.title, &lib.kind);
        let (paused, dir, c) = {
            let i = lock(&self.inner);
            let active = self.active();
            (active >= i.settings.max_active, Path::new(&i.settings.dir).join(&g.id), i.client.clone())
        };
        let c = c.ok_or_else(|| errf!(CODE_STATE, "the torrent engine is not running"))?;
        let (ih, size) =
            c.add(AddRequest { torrent: body.clone(), magnet: magnet.clone(), dir: dir.clone(), paused })?;
        g.info_hash = ih;
        g.dir = dir.to_string_lossy().into();
        if size > 0 {
            g.size = size;
        }
        g.state = if paused {
            GRAB_QUEUED
        } else if body.is_empty() {
            GRAB_METADATA
        } else {
            GRAB_DOWNLOADING
        }
        .into();
        if let Err(e) = self.st.put_grab(&mut g) {
            self.discard(&g);
            return Err(e);
        }
        self.emit(&g);
        tracing::info!(grab = %g.id, library = %g.library_id, state = %g.state, bytes = g.size, "grab added");
        Ok(g)
    }

    fn active(&self) -> i64 {
        self.st.grabs().iter().filter(|g| g.state == GRAB_DOWNLOADING || g.state == GRAB_METADATA).count() as i64
    }

    fn by_info_hash(&self, ih: &str) -> Option<Grab> {
        self.st.grabs().into_iter().find(|g| g.info_hash == ih && !g.data_removed)
    }

    /// Runs on the engine's threads, maybe while `grab` is still inside
    /// `add`: the grab lock makes it wait for the stored record.
    pub(crate) fn on_metadata(&self, ih: &str, size: i64) {
        let _l = lock(&self.grab_mu);
        let Some(mut g) = self.by_info_hash(ih) else { return };
        // The engine may report completion first; that path sized and
        // checked the grab already.
        if !matches!(g.state.as_str(), GRAB_METADATA | GRAB_DOWNLOADING | GRAB_QUEUED | GRAB_PAUSED) {
            return;
        }
        g.size = size;
        if let Err(e) = self.reserve(&g.id, size) {
            self.fail(g, e);
            return;
        }
        if g.state == GRAB_METADATA {
            g.state = GRAB_DOWNLOADING.into();
        }
        self.save(&mut g);
    }

    fn on_complete(&self, ih: &str) {
        let _l = lock(&self.grab_mu);
        let Some(mut g) = self.by_info_hash(ih) else { return };
        if g.state == GRAB_SEEDING || g.state == GRAB_DONE || g.state == GRAB_FAILED {
            return;
        }
        // A magnet whose completion arrives before its metadata event:
        // size it and check the budget here.
        // The client is cloned out first: `reserve` takes the same lock.
        let client = lock(&self.inner).client.clone();
        if g.size == 0
            && let Some(size) = client.and_then(|c| c.status(ih)).map(|s| s.size)
            && size > 0
        {
            g.size = size;
            if let Err(e) = self.reserve(&g.id, size) {
                self.fail(g, e);
                return;
            }
        }
        g.state = GRAB_IMPORTING.into();
        g.finished_at = now();
        self.save(&mut g);
        self.queue_import(&g.id);
    }

    /// Marks a grab failed and frees its disk share: a failed download
    /// is removed with its data (it never reached the library).
    pub(crate) fn fail(&self, mut g: Grab, e: AcqError) {
        g.state = GRAB_FAILED.into();
        g.code = if e.code.is_empty() { CODE_IMPORT.into() } else { e.code.clone() };
        g.error = e.msg.clone();
        if g.finished_at == 0 && !g.info_hash.is_empty() {
            g.data_removed = self.discard(&g);
        }
        // A release that failed is not grabbed again by automation (A-22).
        self.block(&g, &g.error.clone());
        self.save(&mut g);
        tracing::warn!(grab = %g.id, code = %g.code, err = %g.error, "grab failed");
    }

    pub(crate) fn queue_import(&self, id: &str) {
        {
            let mut i = lock(&self.inner);
            if !i.importing.insert(id.to_string()) {
                return;
            }
        }
        let id = id.to_string();
        self.spawn("acquire-import", move |m| {
            m.run_import(&id);
            lock(&m.inner).importing.remove(&id);
            // A retry asked while this run was finishing found it busy.
            if m.st.grab(&id).is_ok_and(|g| g.state == GRAB_IMPORTING) {
                m.queue_import(&id);
            }
        });
    }

    /// Places the files, rescans, then applies the seeding policy.
    fn run_import(&self, id: &str) {
        let Ok(mut g) = self.st.grab(id) else { return };
        if g.state != GRAB_IMPORTING {
            return;
        }
        let Some(lib) = (self.d.library)(&g.library_id) else {
            self.import_failed(g, errf!(CODE_NO_LIBRARY, "the target library no longer exists"));
            return;
        };
        let (c, s) = {
            let i = lock(&self.inner);
            (i.client.clone(), i.settings.clone())
        };
        let Some(c) = c else { return };
        let files = c.files(&g.info_hash);
        // An upgrade first moves the files it replaces into the holding
        // folder, so new names never collide and a failed import can
        // put them back.
        let held = match self.hold_replaced(&g, &lib) {
            Ok(h) => h,
            Err(e) => {
                self.import_failed(g, e);
                return;
            }
        };
        let titles = self.d.titles.as_ref().map(|f| f(&lib.id)).unwrap_or_default();
        let plan = match plan_import(&lib, &files, &g.release, &*self.d.parse, &titles) {
            Ok(p) => p,
            Err(e) => {
                self.restore_held(&held);
                self.import_failed(g, e);
                return;
            }
        };
        let mut mode = s.import_mode.as_str();
        if mode == IMPORT_MOVE && seeds(&s) {
            mode = IMPORT_COPY; // seeding needs the torrent's own copy
        }
        g.imported.clear();
        for it in &plan {
            if it.exists {
                g.imported.push(it.dst.to_string_lossy().into());
                continue;
            }
            match place(&it.src, &it.dst, mode) {
                Ok(used) => {
                    if used == IMPORT_COPY && mode == IMPORT_HARDLINK {
                        tracing::info!(grab = %g.id, dst = %it.dst.display(), "import copied instead of linking");
                    }
                    g.imported.push(it.dst.to_string_lossy().into());
                }
                Err(e) => {
                    // Placed files stay; held ones return where their
                    // path is still free.
                    self.restore_held(&held);
                    self.import_failed(g, e);
                    return;
                }
            }
        }
        // The quality ledger (path → resolution) outlives the grab:
        // lain's names carry no quality (D-118).
        for it in &plan {
            let q = if it.release.resolution.is_empty() { &g.release.resolution } else { &it.release.resolution };
            if !q.is_empty() {
                let _ = self.st.put(Bucket::AcqQuality, &it.dst.to_string_lossy(), q);
            }
        }
        g.code.clear();
        g.error.clear();
        g.replaced.extend(held.iter().map(|h| h.to.to_string_lossy().into_owned()));
        if let Some(f) = &self.d.rescan {
            f(&lib.id);
        }
        if seeds(&s) && mode != IMPORT_MOVE {
            g.state = GRAB_SEEDING.into();
            g.seeding_at = now();
        } else {
            g.state = GRAB_DONE.into();
            g.data_removed = self.discard(&g);
        }
        self.save(&mut g);
        tracing::info!(grab = %g.id, files = g.imported.len(), state = %g.state, "grab imported");
        // Subtitles for a monitored title follow its import (A-36).
        if !g.monitored_id.is_empty() && self.d.subtitles.search.is_some() {
            self.spawn("acquire-subtitles", move |m| m.subtitles_after_import(&g));
        }
    }

    /// Keeps the data so the import can be retried by hand.
    fn import_failed(&self, mut g: Grab, e: AcqError) {
        g.state = GRAB_FAILED.into();
        g.code = e.code.clone();
        g.error = e.msg.clone();
        // Automation must not grab it again; a manual retry is not
        // affected by the blocklist.
        self.block(&g, &g.error.clone());
        self.save(&mut g);
        tracing::warn!(grab = %g.id, err = %g.error, "import failed");
    }

    /// Refreshes live stats, enforces seeding limits and fills slots.
    pub fn monitor(&self) {
        let (c, s) = {
            let i = lock(&self.inner);
            (i.client.clone(), i.settings.clone())
        };
        let Some(c) = c else { return };
        let mut active = 0;
        let mut queued = Vec::new();
        let now_s = now();
        for g in self.st.grabs() {
            if g.info_hash.is_empty() || g.data_removed {
                continue;
            }
            let st = c.status(&g.info_hash);
            if let Some(st) = &st {
                lock(&self.inner).live.insert(g.id.clone(), st.clone());
            }
            match g.state.as_str() {
                GRAB_DOWNLOADING | GRAB_METADATA => {
                    active += 1;
                    let Some(st) = st else { continue };
                    if st.state == "error" {
                        self.fail(g, AcqError::new(CODE_IMPORT, st.error.clone()));
                        continue;
                    }
                    if self.stalled(&g, &st, &s) {
                        let w = self.stall_window(&s);
                        self.fail(g, errf!(CODE_STALLED, "no progress for {}", go_duration(w)));
                        continue;
                    }
                    if st.state == "seeding" && g.finished_at == 0 {
                        self.on_complete(&g.info_hash); // completed while the event was missed
                    }
                }
                GRAB_QUEUED => queued.push(g),
                GRAB_SEEDING => {
                    let up = self.total(&g, st.as_ref())[1];
                    let ratio_done = s.seed_ratio > 0.0 && g.size > 0 && up as f64 / g.size as f64 >= s.seed_ratio;
                    let time_done = s.seed_minutes > 0 && now_s - g.seeding_at >= s.seed_minutes * 60;
                    if ratio_done || time_done || !seeds(&s) {
                        self.stop_seeding(g, &s);
                    }
                }
                _ => {}
            }
        }
        queued.sort_by_key(|g| g.created_at);
        for mut g in queued {
            if active >= s.max_active {
                break;
            }
            if c.resume(&g.info_hash).is_ok() {
                g.state = if g.source == "magnet" && g.size == 0 { GRAB_METADATA } else { GRAB_DOWNLOADING }.into();
                self.save(&mut g);
                active += 1;
            }
        }
    }

    fn stop_seeding(&self, mut g: Grab, s: &AcquireSettings) {
        let t = self.total(&g, None);
        g.downloaded = t[0];
        g.uploaded = t[1];
        if s.remove_after_seeding {
            g.data_removed = self.discard(&g);
        } else if let Err(e) = self.client().remove(&g.info_hash, false)
            && e.code != CODE_NOT_FOUND
        {
            tracing::warn!(grab = %g.id, err = %e, "could not stop seeding");
        }
        g.state = GRAB_DONE.into();
        self.save(&mut g);
        tracing::info!(grab = %g.id, uploaded = g.uploaded, size = g.size, "seeding stopped");
    }

    pub(crate) fn stall_window(&self, s: &AcquireSettings) -> Duration {
        self.d.stall_after.unwrap_or(Duration::from_secs(s.stall_hours.max(0) as u64 * 3600))
    }

    /// A download whose completed bytes have not moved for the stall
    /// window (A-22). The window starts when lain first sees the grab,
    /// so a restart never fails a download on the spot.
    fn stalled(&self, g: &Grab, st: &ClientStatus, s: &AcquireSettings) -> bool {
        let window = self.stall_window(s);
        if window.is_zero() {
            return false;
        }
        let mut i = lock(&self.inner);
        match i.marks.get(&g.id) {
            Some(mk) if mk.bytes == st.completed => mk.at.elapsed() >= window,
            _ => {
                i.marks.insert(g.id.clone(), Mark { bytes: st.completed, at: Instant::now() });
                false
            }
        }
    }

    /// Cumulative downloaded/uploaded: the persisted base plus this
    /// engine session's counters.
    fn total(&self, g: &Grab, st: Option<&ClientStatus>) -> [i64; 2] {
        let i = lock(&self.inner);
        let st = st.cloned().or_else(|| i.live.get(&g.id).cloned());
        let b = i.base.get(&g.id).copied().unwrap_or_default();
        match st {
            Some(st) => [b[0] + st.downloaded, b[1] + st.uploaded],
            None => [b[0].max(g.downloaded), b[1].max(g.uploaded)],
        }
    }

    /// Writes live counters into the grab records.
    fn persist_live(&self) {
        for mut g in self.st.grabs() {
            let st = lock(&self.inner).live.get(&g.id).cloned();
            let Some(st) = st else { continue };
            if g.data_removed {
                continue;
            }
            let t = self.total(&g, Some(&st));
            if t[0] != g.downloaded || t[1] != g.uploaded {
                g.downloaded = t[0];
                g.uploaded = t[1];
                let _ = self.st.put_grab(&mut g);
            }
        }
    }

    /// Grabs with live stats, newest first.
    pub fn grabs(&self) -> Vec<Grab> {
        self.st.grabs().into_iter().map(|g| self.with_live(g)).collect()
    }

    pub(crate) fn with_live(&self, mut g: Grab) -> Grab {
        let st = lock(&self.inner).live.get(&g.id).cloned();
        if let Some(st) = st
            && !g.data_removed
        {
            let t = self.total(&g, Some(&st));
            g.downloaded = t[0];
            g.uploaded = t[1];
            g.completed = st.completed;
            g.peers = st.peers;
            g.down_rate = st.down_rate;
            g.up_rate = st.up_rate;
            if st.size > 0 {
                g.size = st.size;
            }
        }
        if g.state == GRAB_DONE || g.state == GRAB_SEEDING {
            g.completed = g.size;
        }
        g
    }

    pub fn get(&self, id: &str) -> Result<Grab> {
        Ok(self.with_live(self.st.grab(id)?))
    }

    /// Stops a downloading or seeding grab.
    pub fn pause(&self, id: &str) -> Result<Grab> {
        let mut g = self.st.grab(id)?;
        match g.state.as_str() {
            GRAB_DOWNLOADING | GRAB_METADATA | GRAB_QUEUED | GRAB_SEEDING => {}
            _ => return Err(errf!(CODE_STATE, "a {} grab cannot be paused", g.state)),
        }
        self.client().pause(&g.info_hash)?;
        g.state = GRAB_PAUSED.into();
        self.save(&mut g);
        Ok(g)
    }

    /// Restarts a paused grab (queued when no slot is free).
    pub fn resume(&self, id: &str) -> Result<Grab> {
        let mut g = self.st.grab(id)?;
        if g.state != GRAB_PAUSED {
            return Err(errf!(CODE_STATE, "only a paused grab can be resumed"));
        }
        if g.seeding_at > 0 {
            self.client().resume(&g.info_hash)?;
            g.state = GRAB_SEEDING.into();
        } else {
            g.state = GRAB_QUEUED.into(); // the monitor starts it when a slot is free
        }
        self.save(&mut g);
        Ok(g)
    }

    /// Runs the import again after a failure.
    pub fn retry_import(&self, id: &str) -> Result<Grab> {
        let mut g = self.st.grab(id)?;
        if g.state != GRAB_FAILED || g.finished_at == 0 || g.data_removed {
            return Err(errf!(CODE_STATE, "only a finished download whose import failed can be retried"));
        }
        g.state = GRAB_IMPORTING.into();
        g.code.clear();
        g.error.clear();
        self.save(&mut g);
        self.queue_import(&g.id);
        Ok(g)
    }

    /// Forgets a grab; `delete_data` also deletes the torrent's copy
    /// (never the files imported into the library).
    pub fn remove(&self, id: &str, delete_data: bool) -> Result<()> {
        let g = self.st.grab(id)?;
        if g.state == GRAB_IMPORTING {
            return Err(errf!(CODE_STATE, "wait for the import to finish"));
        }
        if delete_data {
            self.discard(&g);
        } else if !g.info_hash.is_empty()
            && !g.data_removed
            && let Err(e) = self.client().remove(&g.info_hash, false)
            && e.code != CODE_NOT_FOUND
        {
            return Err(e);
        }
        {
            let mut i = lock(&self.inner);
            i.live.remove(id);
            i.base.remove(id);
        }
        self.st.remove(Bucket::AcqGrabs, id);
        Ok(())
    }

    /// The release parser (diagnostics).
    pub fn parse(&self, name: &str, kind: &str) -> Release {
        (self.d.parse)(name, kind)
    }

    // ---- cleanup (D-112) ------------------------------------------------
    //
    // Deleting a torrent copy never removes or unlinks a file in a
    // library, whether hardlinked, moved in place, or inside a library
    // root because of an unusual folder layout. Every deletion of
    // torrent data goes through `discard`.

    pub(crate) fn library_roots(&self) -> Vec<PathBuf> {
        (self.d.libraries)().into_iter().filter(|l| !l.path.is_empty()).map(|l| PathBuf::from(l.path)).collect()
    }

    fn protected(&self, p: &Path, g: &Grab, roots: &[PathBuf]) -> bool {
        roots.iter().any(|r| within_or_equal(r, p))
            || g.imported.iter().any(|i| crate::util::clean_path(Path::new(i)) == crate::util::clean_path(p))
    }

    fn grab_dir(&self, g: &Grab) -> PathBuf {
        if g.dir.is_empty() { Path::new(&self.settings().dir).join(&g.id) } else { PathBuf::from(&g.dir) }
    }

    /// Drops the torrent from the client and deletes its copy file by
    /// file, sparing protected paths. Answers whether nothing was left.
    /// Symlinks are removed as links, never followed.
    pub(crate) fn discard(&self, g: &Grab) -> bool {
        if !g.info_hash.is_empty()
            && let Some(c) = lock(&self.inner).client.clone()
            && let Err(e) = c.remove(&g.info_hash, false)
            && e.code != CODE_NOT_FOUND
        {
            tracing::warn!(grab = %g.id, err = %e, "could not drop torrent");
        }
        let dir = self.grab_dir(g);
        let roots = self.library_roots();
        if self.protected(&dir, g, &roots) {
            tracing::warn!(grab = %g.id, dir = %dir.display(), "torrent copy lies inside a library; kept");
            return false;
        }
        let mut clean = true;
        let mut dirs = Vec::new();
        let mut stack = vec![dir.clone()];
        while let Some(d) = stack.pop() {
            let meta = match std::fs::symlink_metadata(&d) {
                Ok(m) => m,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => {
                    clean = false;
                    continue;
                }
            };
            if meta.is_dir() {
                dirs.push(d.clone());
                match std::fs::read_dir(&d) {
                    Ok(rd) => stack.extend(rd.flatten().map(|e| e.path())),
                    Err(_) => clean = false,
                }
                continue;
            }
            if self.protected(&d, g, &roots) {
                clean = false;
                tracing::warn!(grab = %g.id, path = %d.display(), "kept a protected file during cleanup");
                continue;
            }
            if let Err(e) = std::fs::remove_file(&d)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                clean = false;
            }
        }
        // Deepest folders first; a non-empty one simply stays.
        dirs.sort_by_key(|d| std::cmp::Reverse(d.as_os_str().len()));
        for d in dirs {
            let _ = std::fs::remove_dir(d);
        }
        if std::fs::symlink_metadata(&dir).is_ok() {
            clean = false;
        }
        clean
    }
}

/// Go's `time.Duration.String` for whole hours, minutes and seconds
/// ("6h0m0s", "1m30s", "500ms").
pub fn go_duration(d: Duration) -> String {
    let ms = d.as_millis();
    if ms < 1000 {
        return format!("{ms}ms");
    }
    let s = d.as_secs();
    let frac = d.subsec_millis();
    let secs = if frac == 0 { format!("{}s", s % 60) } else { format!("{}.{:03}s", s % 60, frac).replace("0s", "s") };
    match (s / 3600, (s % 3600) / 60) {
        (0, 0) => secs,
        (0, m) => format!("{m}m{secs}"),
        (h, m) => format!("{h}h{m}m{secs}"),
    }
}

/// A download folder that is, contains or lies inside a library root:
/// the torrent copy and the library must never share a tree.
pub fn overlaps_library(dir: &Path, libs: &[Library]) -> Option<String> {
    libs.iter()
        .filter(|l| !l.path.is_empty())
        .find(|l| within_or_equal(Path::new(&l.path), dir) || within_or_equal(dir, Path::new(&l.path)))
        .map(|l| l.name.clone())
}

fn apply_indexer(input: &IndexerInput, ix: &mut Indexer) -> Result<()> {
    if let Some(n) = &input.name {
        ix.name = clean_name(n, 60)?;
    }
    if let Some(p) = &input.protocol {
        ix.protocol = p.clone();
    }
    if let Some(u) = &input.url {
        ix.url = u.trim().to_string();
    }
    if let Some(k) = &input.api_key {
        // An empty key in a patch keeps the stored one; "-" clears it.
        match k.trim() {
            "" => {}
            "-" => ix.api_key.clear(),
            k if k.len() > 512 => return Err(errf!(CODE_INVALID, "api key too long")),
            k => ix.api_key = k.to_string(),
        }
    }
    if let Some(c) = &input.categories {
        if c.len() > 50 {
            return Err(errf!(CODE_INVALID, "at most 50 categories"));
        }
        ix.categories = c.clone();
    }
    if let Some(e) = input.enabled {
        ix.enabled = e;
    }
    if let Some(p) = input.priority {
        ix.priority = p;
    }
    if let Some(m) = input.min_interval_ms {
        ix.min_interval_ms = m;
    }
    if ix.name.is_empty() {
        return Err(errf!(CODE_INVALID, "name required"));
    }
    if ix.protocol != INDEXER_TORZNAB && ix.protocol != INDEXER_NEWZNAB {
        return Err(errf!(CODE_INVALID, "protocol must be torznab or newznab"));
    }
    if !http_url(&ix.url) {
        return Err(errf!(CODE_INVALID, "url must be http(s) without credentials (use the API key field)"));
    }
    if !(0..=600_000).contains(&ix.min_interval_ms) {
        return Err(errf!(CODE_INVALID, "min_interval_ms must be 0-600000"));
    }
    Ok(())
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn settings_validate() {
        let data = std::env::temp_dir().join("data");
        validate_settings(default_settings(&data)).unwrap();
        let bads: Vec<fn(&mut AcquireSettings)> = vec![
            |s| s.dir = "relative".into(),
            |s| s.listen_port = 70000,
            |s| s.max_active = 0,
            |s| s.seed_ratio = -1.0,
            |s| s.import_mode = "symlink".into(),
            |s| s.upload_kbps = -5,
            |s| s.rss_minutes = 5,
            |s| s.subtitle_hours = 721,
        ];
        for bad in bads {
            let mut s = default_settings(&data);
            bad(&mut s);
            assert_eq!(validate_settings(s.clone()).unwrap_err().code, CODE_INVALID, "accepted {s:?}");
        }
        let mut s = default_settings(&data);
        s.rss_minutes = 0;
        s.import_mode = String::new();
        let s = validate_settings(s).unwrap();
        assert_eq!((s.rss_minutes, s.import_mode.as_str()), (30, IMPORT_HARDLINK));
    }

    #[test]
    fn indexer_input_validation() {
        let base = || IndexerInput {
            name: Some("tracker-exemplo".into()),
            protocol: Some("torznab".into()),
            url: Some("http://127.0.0.1:9117/api".into()),
            ..Default::default()
        };
        let mut ix = Indexer::default();
        apply_indexer(&base(), &mut ix).unwrap();
        assert_eq!(ix.categories, Vec::<i64>::new());
        let bads: Vec<fn(&mut IndexerInput)> = vec![
            |i| i.name = Some(String::new()),
            |i| i.protocol = Some("rss".into()),
            |i| i.url = Some("ftp://x".into()),
            |i| i.url = Some("http://user:pw@host/".into()),
            |i| i.min_interval_ms = Some(-1),
            |i| i.categories = Some(vec![1; 51]),
            |i| i.api_key = Some("k".repeat(513)),
        ];
        for bad in bads {
            let mut input = base();
            bad(&mut input);
            assert_eq!(apply_indexer(&input, &mut Indexer::default()).unwrap_err().code, CODE_INVALID);
        }
        // Keys: empty keeps, "-" clears.
        let mut ix = Indexer::default();
        apply_indexer(&IndexerInput { api_key: Some("k1".into()), ..base() }, &mut ix).unwrap();
        apply_indexer(&IndexerInput { api_key: Some(" ".into()), ..base() }, &mut ix).unwrap();
        assert_eq!(ix.api_key, "k1");
        apply_indexer(&IndexerInput { api_key: Some("-".into()), ..base() }, &mut ix).unwrap();
        assert_eq!(ix.api_key, "");
    }

    #[test]
    fn durations_read_like_go() {
        assert_eq!(go_duration(Duration::from_secs(6 * 3600)), "6h0m0s");
        assert_eq!(go_duration(Duration::from_secs(90)), "1m30s");
        assert_eq!(go_duration(Duration::from_millis(500)), "500ms");
        assert_eq!(go_duration(Duration::from_secs(2)), "2s");
    }
}
