//! [`LainCore`]: the one object the app holds.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use lain_api::{CatalogItem, ProgressUpdate};
use lain_client::{
    CatalogQuery, Client, ClientConfig, ClientDb, ClientError, EventStream, FetchObserver, FileTokenStore,
    OfflineConfig, OfflineEntry, OfflineState, OfflineStore, ProgressSync, RecordOutcome, Resolution, SearchQuery,
    ServerEvent, StreamConfig, StreamItem,
};

use crate::cache::{Cache, now_ms};
use crate::error::{LainError, io};
use crate::task::{Task, spawn};
use crate::types::*;

/// The current server: its client, sync engine and offline store.
pub(crate) struct Backend {
    pub(crate) client: Client,
    pub(crate) sync: ProgressSync,
    pub(crate) offline: Arc<OfflineStore>,
    pub(crate) cache: Arc<Cache>,
}

impl Backend {
    pub(crate) fn server(&self) -> String {
        self.client.server()
    }
}

struct Events {
    server: String,
    stream: Arc<EventStream>,
    /// The task waiting in `next`: it holds the stream's lock, so
    /// closing aborts it first.
    waiting: Option<tokio::task::AbortHandle>,
}

/// The app's handle on lain: sessions, the current server, offline
/// copies and progress sync. Safe to share across threads.
#[derive(uniffi::Object)]
pub struct LainCore {
    rt: Option<tokio::runtime::Runtime>,
    dir: PathBuf,
    offline_dir: PathBuf,
    db: Arc<ClientDb>,
    tokens: Arc<FileTokenStore>,
    timeout_ms: u64,
    policy: RwLock<OfflinePolicy>,
    backend: RwLock<Option<Arc<Backend>>>,
    events: Mutex<Option<Events>>,
    pub(crate) books: Arc<crate::read::Books>,
}

impl Drop for LainCore {
    fn drop(&mut self) {
        // The last reference may go away on a runtime thread, where a
        // blocking shutdown would panic.
        if let Some(rt) = self.rt.take() {
            rt.shutdown_background();
        }
    }
}

fn default_policy() -> OfflinePolicy {
    let d = OfflineConfig::new("");
    OfflinePolicy { max_bytes: d.max_bytes as i64, min_free_bytes: d.min_free_bytes as i64, evict_watched: true }
}

fn open_store(dir: &Path, policy: &OfflinePolicy, db: Arc<ClientDb>) -> Result<Arc<OfflineStore>, LainError> {
    std::fs::create_dir_all(dir).map_err(io)?;
    let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let mut cfg = OfflineConfig::new(&dir.to_string_lossy());
    cfg.max_bytes = policy.max_bytes.max(0) as u64;
    cfg.min_free_bytes = policy.min_free_bytes.max(0) as u64;
    cfg.evict_watched = policy.evict_watched;
    Ok(Arc::new(OfflineStore::open(cfg, db)?))
}

/// Forwards transfer progress to the app, at most every 250 ms.
struct Observer {
    item_id: String,
    inner: Arc<dyn DownloadObserver>,
    last: Mutex<Option<Instant>>,
}

impl FetchObserver for Observer {
    fn progress(&self, done: u64, total: u64) {
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        let due = last.is_none_or(|t| t.elapsed() >= Duration::from_millis(250)) || (total > 0 && done >= total);
        if due {
            *last = Some(Instant::now());
            drop(last);
            self.inner.progress(self.item_id.clone(), done as i64, total as i64);
        }
    }

    fn evicted(&self, entry: &OfflineEntry) {
        self.inner.evicted(entry.item_id.clone(), entry.title.clone());
    }
}

impl LainCore {
    pub(crate) fn handle(&self) -> tokio::runtime::Handle {
        self.rt.as_ref().expect("runtime lives until drop").handle().clone()
    }

    pub(crate) fn backend(&self) -> Result<Arc<Backend>, LainError> {
        self.backend.read().unwrap_or_else(|e| e.into_inner()).clone().ok_or(LainError::NoServer)
    }

    pub(crate) fn run<T, F>(&self, f: F) -> Task<T>
    where
        T: Send + 'static,
        F: std::future::Future<Output = Result<T, LainError>> + Send + 'static,
    {
        spawn(&self.handle(), f)
    }

    fn close_events(&self) {
        if let Some(ev) = self.events.lock().unwrap_or_else(|e| e.into_inner()).take() {
            if let Some(w) = ev.waiting {
                w.abort();
            }
            self.handle().spawn(async move { ev.stream.close().await });
        }
    }

    /// The catalog item, from the server or, offline, the snapshot.
    pub(crate) async fn catalog_item(b: &Backend, item_id: &str) -> Result<CatalogItem, LainError> {
        match b.client.catalog_item(item_id).await {
            Ok(i) => Ok(i),
            Err(e) if e.is_offline() => b
                .cache
                .catalog()
                .and_then(|(items, _)| items.into_iter().find(|i| i.id == item_id))
                .ok_or_else(|| e.into()),
            Err(e) => Err(e.into()),
        }
    }
}

async fn all_items(client: &Client) -> Result<Vec<CatalogItem>, ClientError> {
    let mut out: Vec<CatalogItem> = Vec::new();
    loop {
        let page = client.catalog(CatalogQuery { limit: 500, offset: out.len() as i64, ..Default::default() }).await?;
        let n = page.items.len();
        out.extend(page.items);
        if n == 0 || out.len() as i64 >= page.total {
            return Ok(out);
        }
    }
}

fn summary(r: lain_client::FlushReport) -> FlushSummary {
    let count = |want: Resolution| r.items.iter().filter(|i| i.resolution == want).count() as i64;
    FlushSummary {
        pushed: count(Resolution::Pushed),
        kept_server: count(Resolution::KeptServer),
        dropped: r.items.len() as i64 - count(Resolution::Pushed) - count(Resolution::KeptServer),
        remaining: r.remaining as i64,
        stopped: r.stopped.map(|e| e.to_string()),
    }
}

#[uniffi::export]
impl LainCore {
    /// Opens the client state in `dir` (the app's files dir:
    /// `sessions.json`, `client.redb`, `cache/`) with offline copies in
    /// `offline_dir`. The last server used is not chosen here; call
    /// [`LainCore::set_server`].
    #[uniffi::constructor]
    pub fn new(dir: String, offline_dir: String) -> Result<Arc<LainCore>, LainError> {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).map_err(io)?;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("lain-core")
            .enable_all()
            .build()
            .map_err(|e| LainError::Internal { detail: e.to_string() })?;
        let db = Arc::new(ClientDb::open(&dir.join("client.redb"))?);
        let tokens = Arc::new(FileTokenStore::new(dir.join("sessions.json")));
        Ok(Arc::new(LainCore {
            rt: Some(rt),
            offline_dir: PathBuf::from(offline_dir),
            dir,
            db,
            tokens,
            timeout_ms: 20_000,
            policy: RwLock::new(default_policy()),
            backend: RwLock::new(None),
            events: Mutex::new(None),
            books: Arc::new(crate::read::Books::default()),
        }))
    }

    /// Chooses the server (keeping a session saved for it) and returns
    /// its canonical URL. The client speaks plain HTTP only for now.
    pub fn set_server(&self, url: String) -> Result<String, LainError> {
        let client = Client::new(ClientConfig { server: url, timeout_ms: self.timeout_ms }, self.tokens.clone())?;
        let server = client.server();
        let policy = self.policy.read().unwrap_or_else(|e| e.into_inner()).clone();
        let offline = open_store(&self.offline_dir, &policy, self.db.clone())?;
        let b = Backend {
            sync: ProgressSync::new(client.clone(), self.db.clone()),
            cache: Arc::new(Cache::new(&self.dir, &server)),
            client,
            offline,
        };
        self.close_events();
        self.books.clear();
        *self.backend.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(b));
        Ok(server)
    }

    /// The current server's canonical URL.
    pub fn server(&self) -> Option<String> {
        self.backend().ok().map(|b| b.server())
    }

    /// The saved login on the current server, if any.
    pub fn session(&self) -> Option<SessionInfo> {
        self.backend().ok().and_then(|b| b.client.session()).map(Into::into)
    }

    /// `GET /health`: the server's version.
    pub async fn health(&self) -> Result<String, LainError> {
        let b = self.backend()?;
        self.run(async move { Ok(b.client.health().await?.version) }).await
    }

    pub async fn setup_required(&self) -> Result<bool, LainError> {
        let b = self.backend()?;
        self.run(async move { Ok(b.client.setup_status().await?.setup_required) }).await
    }

    /// Logs in; the session is saved for the next start.
    pub async fn login(&self, username: String, password: String) -> Result<UserInfo, LainError> {
        let b = self.backend()?;
        self.close_events();
        self.run(async move { Ok(b.client.login(&username, &password).await?.into()) }).await
    }

    /// Forgets the session (queued progress stays for the next login).
    pub fn logout(&self) -> Result<(), LainError> {
        let b = self.backend()?;
        self.close_events();
        Ok(b.client.logout()?)
    }

    pub async fn me(&self) -> Result<UserInfo, LainError> {
        let b = self.backend()?;
        self.run(async move { Ok(b.client.me().await?.into()) }).await
    }

    /// Sets the account's language for audio and subtitles (ISO 639-2/T,
    /// empty for the file's own choice), the one the web client's Settings
    /// edits too; answers the account as it then is.
    pub async fn set_preferred_language(&self, language: String) -> Result<UserInfo, LainError> {
        let b = self.backend()?;
        self.run(async move { Ok(b.client.set_preferred_language(language.trim()).await?.into()) }).await
    }

    /// `GET /api/modules`: what the navigation shows.
    pub async fn modules(&self) -> Result<Vec<ModuleInfo>, LainError> {
        let b = self.backend()?;
        self.run(async move {
            let v = b.client.modules().await?;
            Ok(v.modules
                .into_iter()
                .map(|m| ModuleInfo {
                    destinations: m
                        .ui
                        .destinations
                        .into_iter()
                        .map(|d| DestinationInfo { id: d.id, label: d.label })
                        .collect(),
                    id: m.id,
                    name: m.name,
                    enabled: m.enabled,
                    state: m.state,
                })
                .collect())
        })
        .await
    }

    /// Every item this session may see; offline, the last snapshot.
    pub async fn catalog(&self) -> Result<CatalogSnapshot, LainError> {
        let b = self.backend()?;
        self.run(async move {
            match all_items(&b.client).await {
                Ok(items) => {
                    let at = now_ms();
                    b.cache.save_catalog(&items, at);
                    Ok(CatalogSnapshot {
                        items: items.iter().map(Item::from).collect(),
                        from_cache: false,
                        fetched_at_ms: at,
                    })
                }
                Err(e) if e.is_offline() => match b.cache.catalog() {
                    Some((items, at)) => Ok(CatalogSnapshot {
                        items: items.iter().map(Item::from).collect(),
                        from_cache: true,
                        fetched_at_ms: at,
                    }),
                    None => Err(e.into()),
                },
                Err(e) => Err(e.into()),
            }
        })
        .await
    }

    pub async fn libraries(&self) -> Result<Vec<LibraryInfo>, LainError> {
        let b = self.backend()?;
        self.run(async move {
            Ok(b.client
                .libraries()
                .await?
                .into_iter()
                .map(|l| LibraryInfo {
                    locked: l.locked.unwrap_or(false),
                    id: l.id,
                    name: l.name,
                    kind: l.kind,
                    private: l.private,
                })
                .collect())
        })
        .await
    }

    /// One title's files (`/api/catalog/{id}/episodes`).
    pub async fn episodes(&self, item_id: String) -> Result<Vec<Item>, LainError> {
        let b = self.backend()?;
        self.run(async move { Ok(b.client.episodes(&item_id).await?.iter().map(Item::from).collect()) }).await
    }

    /// The Continue rail: unfinished records, newest first.
    pub async fn continue_watching(&self) -> Result<Vec<ProgressInfo>, LainError> {
        let b = self.backend()?;
        self.run(async move {
            let mut v: Vec<ProgressInfo> =
                b.client.continue_watching().await?.into_iter().filter(|p| !p.completed).map(Into::into).collect();
            v.sort_by_key(|p| std::cmp::Reverse(p.updated_at));
            Ok(v)
        })
        .await
    }

    /// Every position this account has on the server, finished ones
    /// included: what a library's posters and filters are marked from.
    /// Newest first.
    pub async fn all_progress(&self) -> Result<Vec<ProgressInfo>, LainError> {
        let b = self.backend()?;
        self.run(async move {
            let mut v: Vec<ProgressInfo> = b.client.continue_watching().await?.into_iter().map(Into::into).collect();
            v.sort_by_key(|p| std::cmp::Reverse(p.updated_at));
            Ok(v)
        })
        .await
    }

    /// `GET /api/search`; offline, a title match over the snapshot.
    pub async fn search(&self, query: String, limit: i64) -> Result<Vec<Item>, LainError> {
        let b = self.backend()?;
        self.run(async move {
            let q = SearchQuery { q: query.clone(), limit, ..Default::default() };
            match b.client.search(q).await {
                Ok(page) => Ok(page.items.iter().map(Item::from).collect()),
                Err(e) if e.is_offline() => {
                    let (items, _) = b.cache.catalog().ok_or(LainError::from(e))?;
                    let needle = query.to_lowercase();
                    Ok(items
                        .iter()
                        .filter(|i| !i.missing && i.title.to_lowercase().contains(&needle))
                        .take(limit.max(1) as usize)
                        .map(Item::from)
                        .collect())
                }
                Err(e) => Err(e.into()),
            }
        })
        .await
    }

    /// The item's position: the server's, or offline the queued one.
    pub async fn progress(&self, item_id: String) -> Result<ProgressInfo, LainError> {
        let b = self.backend()?;
        self.run(async move { Ok(b.sync.fetch(&item_id).await?.into()) }).await
    }

    /// Saves a position through the sync engine: queued when offline.
    pub async fn save_progress(
        &self,
        item_id: String,
        position_sec: f64,
        duration_sec: f64,
        completed: bool,
    ) -> Result<SaveOutcome, LainError> {
        let b = self.backend()?;
        self.run(async move {
            let update =
                ProgressUpdate { position_sec: position_sec.max(0.), duration_sec: duration_sec.max(0.), completed };
            Ok(match b.sync.record(&item_id, update).await? {
                RecordOutcome::Synced { progress } => SaveOutcome::Synced { progress: progress.into() },
                RecordOutcome::Queued { reason } => SaveOutcome::Queued { reason },
                RecordOutcome::KeptServer { progress } => SaveOutcome::KeptServer { progress: progress.into() },
            })
        })
        .await
    }

    /// Pushes queued progress (the event stream does it on connect).
    pub async fn flush_progress(&self) -> Result<FlushSummary, LainError> {
        let b = self.backend()?;
        self.run(async move { Ok(summary(b.sync.flush().await?)) }).await
    }

    /// How many positions wait for the server.
    pub fn queued_progress(&self) -> Result<i64, LainError> {
        match self.backend()?.sync.queued() {
            Ok(q) => Ok(q.len() as i64),
            // Logged out: nothing is queued for this session.
            Err(ClientError::Unauthorized) => Ok(0),
            Err(e) => Err(e.into()),
        }
    }

    /// Unlocks a private library for this session with the account's PIN
    /// (`pin` true) or its password (D-156). The catalog then holds its
    /// items: fetch it again.
    pub async fn unlock_library(
        &self,
        library_id: String,
        secret: String,
        pin: bool,
    ) -> Result<LibraryInfo, LainError> {
        let b = self.backend()?;
        self.run(async move {
            let how = if pin { lain_client::Unlock::Pin(secret) } else { lain_client::Unlock::Password(secret) };
            let l = b.client.unlock_library(&library_id, how).await?;
            Ok(LibraryInfo {
                locked: l.locked.unwrap_or(false),
                id: l.id,
                name: l.name,
                kind: l.kind,
                private: l.private,
            })
        })
        .await
    }

    /// Locks a private library again for this session.
    pub async fn lock_library(&self, library_id: String) -> Result<LibraryInfo, LainError> {
        let b = self.backend()?;
        self.run(async move {
            let l = b.client.lock_library(&library_id).await?;
            Ok(LibraryInfo {
                locked: l.locked.unwrap_or(true),
                id: l.id,
                name: l.name,
                kind: l.kind,
                private: l.private,
            })
        })
        .await
    }

    /// A frame of the item at `at_sec`, for the seek bar's preview.
    pub async fn frame(&self, item_id: String, at_sec: f64, width: u32) -> Result<Vec<u8>, LainError> {
        let b = self.backend()?;
        self.run(async move { Ok(b.client.frame(&item_id, at_sec, width).await?) }).await
    }

    /// The works this account keeps as favourites (the social module's
    /// list, the same one the web client shows).
    pub async fn favorites(&self) -> Result<Vec<FavoriteInfo>, LainError> {
        let b = self.backend()?;
        self.run(
            async move { Ok(b.client.social_settings().await?.favorites.into_iter().map(favorite_info).collect()) },
        )
        .await
    }

    /// Adds the work an item belongs to to the favourites, or takes it out;
    /// answers the list as it then stands. A work is its kind and title, so
    /// any episode of a series names the series.
    pub async fn set_favorite(&self, item_id: String, on: bool) -> Result<Vec<FavoriteInfo>, LainError> {
        let b = self.backend()?;
        self.run(async move {
            let target = lain_api::WorkRef { item_id, ..Default::default() };
            let work = b.client.work(&target).await?.work;
            let same = |w: &lain_api::WorkRef| work_key(w) == work_key(&work);
            let mut next: Vec<lain_api::WorkRef> =
                b.client.social_settings().await?.favorites.into_iter().filter(|w| !same(w)).collect();
            if on {
                next.push(target);
            }
            let patch = lain_api::SocialSettingsPatch { favorites: Some(next), ..Default::default() };
            Ok(b.client.update_social_settings(patch).await?.favorites.into_iter().map(favorite_info).collect())
        })
        .await
    }

    /// How the work an item belongs to is rated (the social module's
    /// ratings, the ones the web client shows).
    pub async fn rating(&self, item_id: String) -> Result<RatingInfo, LainError> {
        let b = self.backend()?;
        self.run(async move {
            let target = lain_api::WorkRef { item_id, ..Default::default() };
            Ok(rating_info(&b.client.work(&target).await?))
        })
        .await
    }

    /// Scores the work an item belongs to, 1 to 10, or takes the score
    /// back with 0; answers how the work is then rated. A review written
    /// on the web is kept as it is.
    pub async fn rate(&self, item_id: String, score: u32) -> Result<RatingInfo, LainError> {
        let b = self.backend()?;
        self.run(async move {
            let target = lain_api::WorkRef { item_id, ..Default::default() };
            let mine = b.client.work(&target).await?.mine;
            let (review, spoiler) = mine.map(|m| (m.review, m.spoiler)).unwrap_or_default();
            if score == 0 && review.is_empty() {
                b.client.delete_rating(&target).await?;
            } else {
                let input =
                    lain_api::RatingInput { work: target.clone(), score: i64::from(score.min(10)), review, spoiler };
                b.client.rate(input).await?;
            }
            Ok(rating_info(&b.client.work(&target).await?))
        })
        .await
    }

    /// How long these videos are (the ones the server can tell).
    pub async fn durations(&self, item_ids: Vec<String>) -> Result<Vec<DurationInfo>, LainError> {
        let b = self.backend()?;
        self.run(async move {
            let mut out = Vec::new();
            for chunk in item_ids.chunks(100) {
                for d in b.client.durations(chunk).await? {
                    out.push(DurationInfo { item_id: d.item_id, duration_sec: d.duration_sec });
                }
            }
            Ok(out)
        })
        .await
    }

    /// The metadata overlays that exist for these items (genres, synopsis).
    pub async fn overlays(&self, item_ids: Vec<String>) -> Result<Vec<Overlay>, LainError> {
        let b = self.backend()?;
        self.run(async move {
            let mut out = Vec::new();
            for chunk in item_ids.chunks(150) {
                for e in b.client.enrichments(chunk).await? {
                    out.push(Overlay {
                        item_id: e.item_id,
                        title: e.title,
                        year: e.year,
                        genres: e.genres,
                        synopsis: e.synopsis,
                    });
                }
            }
            Ok(out)
        })
        .await
    }

    /// The item's cover or frame; offline, the last one seen.
    pub async fn thumbnail(&self, item_id: String) -> Result<Vec<u8>, LainError> {
        let b = self.backend()?;
        self.run(async move {
            match b.client.thumbnail(&item_id).await {
                Ok(bytes) => {
                    b.cache.save_art(&item_id, &bytes);
                    Ok(bytes)
                }
                Err(e) if e.is_offline() => b.cache.art(&item_id).ok_or_else(|| e.into()),
                Err(e) => Err(e.into()),
            }
        })
        .await
    }

    // ---- offline copies (D-094, D-248) ---------------------------------

    pub fn offline_policy(&self) -> OfflinePolicy {
        self.policy.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Changes the device budget; applies to the current server now.
    pub fn set_offline_policy(&self, policy: OfflinePolicy) -> Result<(), LainError> {
        if policy.max_bytes < 0 || policy.min_free_bytes < 0 {
            return Err(LainError::Invalid { detail: "sizes must be >= 0".into() });
        }
        *self.policy.write().unwrap_or_else(|e| e.into_inner()) = policy.clone();
        let mut slot = self.backend.write().unwrap_or_else(|e| e.into_inner());
        if let Some(b) = slot.as_ref() {
            let offline = open_store(&self.offline_dir, &policy, self.db.clone())?;
            *slot = Some(Arc::new(Backend {
                client: b.client.clone(),
                sync: b.sync.clone(),
                cache: b.cache.clone(),
                offline,
            }));
        }
        Ok(())
    }

    /// This server's copies, in queue order.
    pub fn offline_copies(&self) -> Result<Vec<OfflineCopy>, LainError> {
        let b = self.backend()?;
        let server = b.server();
        Ok(b.offline.list()?.into_iter().filter(|e| e.server == server).map(Into::into).collect())
    }

    pub fn offline_copy(&self, item_id: String) -> Result<Option<OfflineCopy>, LainError> {
        let b = self.backend()?;
        Ok(b.offline.get(&b.server(), &item_id)?.map(Into::into))
    }

    /// Queues an item for an offline copy (or requeues a paused or
    /// failed one).
    pub async fn offline_add(&self, item_id: String) -> Result<OfflineCopy, LainError> {
        let b = self.backend()?;
        self.run(async move {
            let item = LainCore::catalog_item(&b, &item_id).await?;
            Ok(b.offline.add(&b.server(), &item)?.into())
        })
        .await
    }

    /// Transfers one queued copy, resuming its partial bytes.
    pub async fn offline_fetch(
        &self,
        item_id: String,
        observer: Option<Arc<dyn DownloadObserver>>,
    ) -> Result<OfflineCopy, LainError> {
        let b = self.backend()?;
        self.run(async move {
            let obs = observer.map(|inner| {
                Arc::new(Observer { item_id: item_id.clone(), inner, last: Mutex::new(None) }) as Arc<dyn FetchObserver>
            });
            Ok(b.offline.fetch(&b.client, &item_id, obs).await?.into())
        })
        .await
    }

    /// Transfers every queued copy in order. It stops when the server
    /// is unreachable or the store is full; a failed copy is counted and
    /// the run goes on.
    pub async fn offline_run(&self, observer: Option<Arc<dyn DownloadObserver>>) -> Result<DownloadRun, LainError> {
        let b = self.backend()?;
        self.run(async move {
            let server = b.server();
            let mut run = DownloadRun { done: 0, failed: 0, stopped: None };
            loop {
                let next =
                    b.offline.list()?.into_iter().find(|e| e.server == server && e.state == OfflineState::Queued);
                let Some(e) = next else { return Ok(run) };
                let obs = observer.clone().map(|inner| {
                    Arc::new(Observer { item_id: e.item_id.clone(), inner, last: Mutex::new(None) })
                        as Arc<dyn FetchObserver>
                });
                match b.offline.fetch(&b.client, &e.item_id, obs).await {
                    Ok(done) if done.state == OfflineState::Done => run.done += 1,
                    Ok(other) if other.state == OfflineState::Paused => {
                        run.stopped = Some("the server is unreachable".into());
                        return Ok(run);
                    }
                    Ok(_) => run.failed += 1,
                    Err(err @ (ClientError::NoRoom { .. } | ClientError::Unauthorized)) => {
                        run.stopped = Some(err.to_string());
                        return Ok(run);
                    }
                    Err(err) if err.is_offline() => {
                        run.stopped = Some(err.to_string());
                        return Ok(run);
                    }
                    Err(_) => run.failed += 1,
                }
            }
        })
        .await
    }

    pub fn offline_pause(&self, item_id: String) -> Result<OfflineCopy, LainError> {
        let b = self.backend()?;
        Ok(b.offline.pause(&b.server(), &item_id)?.into())
    }

    pub fn offline_resume(&self, item_id: String) -> Result<OfflineCopy, LainError> {
        let b = self.backend()?;
        Ok(b.offline.resume(&b.server(), &item_id)?.into())
    }

    /// Deletes a copy (and its part file); returns the bytes freed.
    pub fn offline_remove(&self, item_id: String) -> Result<i64, LainError> {
        let b = self.backend()?;
        self.books.forget(&item_id);
        Ok(b.offline.remove(&b.server(), &item_id, true)? as i64)
    }

    pub fn offline_usage(&self) -> Result<OfflineUsage, LainError> {
        let b = self.backend()?;
        let cfg = b.offline.config();
        let free = {
            use lain_client::DiskSpace;
            lain_client::offline::SystemDiskSpace.free_bytes(Path::new(&cfg.dir)).map(|v| v as i64)
        };
        Ok(OfflineUsage {
            used_bytes: b.offline.used()? as i64,
            max_bytes: cfg.max_bytes as i64,
            min_free_bytes: cfg.min_free_bytes as i64,
            free_bytes: free,
        })
    }

    /// Marks copies the server says are watched (eviction candidates).
    pub async fn refresh_watched(&self) -> Result<i64, LainError> {
        let b = self.backend()?;
        self.run(async move { Ok(b.offline.refresh_watched(&b.client).await? as i64) }).await
    }

    // ---- events ------------------------------------------------------

    /// The next server event. Waits; reconnects by itself. On connect it
    /// pushes queued progress first. `None` once the stream is closed
    /// (logout, another server).
    pub async fn next_event(&self) -> Option<CoreEvent> {
        let b = self.backend().ok()?;
        let stream = {
            let mut g = self.events.lock().unwrap_or_else(|e| e.into_inner());
            let server = b.server();
            if g.as_ref().is_none_or(|e| e.server != server) {
                let stream = Arc::new(EventStream::new(b.client.clone(), StreamConfig::default()));
                *g = Some(Events { server, stream, waiting: None });
            }
            g.as_ref().map(|e| e.stream.clone())?
        };
        let task = self.run(async move {
            loop {
                let Some(item) = stream.next().await else { return Ok(None) };
                let ev = match item {
                    StreamItem::Connected(_) => {
                        let pushed = b.sync.flush().await.map(|r| summary(r).pushed).unwrap_or(0);
                        CoreEvent::Connected { pushed }
                    }
                    StreamItem::Reconnecting { attempt, delay_ms, .. } => {
                        CoreEvent::Reconnecting { attempt: i64::from(attempt), delay_ms: delay_ms as i64 }
                    }
                    StreamItem::AuthRequired => CoreEvent::AuthRequired,
                    StreamItem::Event(ev) => {
                        if let Err(e) = b.sync.observe(&ev) {
                            eprintln!("lain-ffi: progress base: {e}");
                        }
                        match ev {
                            ServerEvent::ProgressChanged(p) => CoreEvent::ProgressChanged { item_id: p.item_id },
                            ServerEvent::LibraryChanged(_) => CoreEvent::LibraryChanged,
                            ServerEvent::ModuleState(_) => CoreEvent::ModulesChanged,
                            ServerEvent::Other { event, .. } if event == "item-deleted" => CoreEvent::LibraryChanged,
                            _ => continue,
                        }
                    }
                };
                return Ok(Some(ev));
            }
        });
        if let Some(e) = self.events.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            e.waiting = Some(task.abort_handle());
        }
        task.await.ok().flatten()
    }

    /// Ends the event stream (the app went to the background).
    pub fn stop_events(&self) {
        self.close_events();
    }
}

fn rating_info(view: &lain_api::WorkView) -> RatingInfo {
    RatingInfo {
        mine: view.mine.as_ref().map_or(0, |m| m.score.clamp(0, 10) as u32),
        average: view.average,
        scored: view.scored.max(0) as u32,
    }
}

fn favorite_info(w: lain_api::WorkRef) -> FavoriteInfo {
    FavoriteInfo { item_id: w.item_id, kind: w.kind, title: w.title }
}

/// A work's identity as the server compares it: kind and title, case and
/// spacing aside.
fn work_key(w: &lain_api::WorkRef) -> (String, String) {
    (w.kind.to_lowercase(), w.title.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" "))
}
