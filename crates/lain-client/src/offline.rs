//! Offline copies: library items fetched onto this device so they play
//! or read without the server. It mirrors D-094's CLI store
//! (`internal/offline`): the same limits (a byte budget, default
//! 10 GiB, and a free-space floor, default 5 GiB), resumable transfers
//! through the Range-capable `/api/items/{id}/stream`, and the same
//! eviction: when a new copy needs room, copies that are watched and
//! have no unsynced progress are deleted, least recently used first,
//! without asking (on by default; `evict_watched: false` turns it off).
//! Only local copies are ever deleted, never server files.
//!
//! The index lives in the client database (`offline` table) instead of
//! `index.json`; unsynced positions live in the progress queue
//! ([`crate::ProgressSync`]), which eviction consults. Finished files
//! sit in the store directory, partial ones in `.parts/`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use lain_api::CatalogItem;

use crate::api::{Client, stream_path};
use crate::db::{self, ClientDb, OFFLINE, QUEUE};
use crate::error::{ClientError, store_err};
use crate::http::next_chunk;

/// D-094's defaults; every one is a setting.
pub const DEFAULT_MAX_BYTES: u64 = 10 << 30;
pub const DEFAULT_MIN_FREE_BYTES: u64 = 5 << 30;

pub const CODE_QUOTA: &str = "quota-exceeded";
pub const CODE_DISK_FULL: &str = "disk-full";

/// The store's policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfflineConfig {
    /// The directory for the copies (absolute).
    pub dir: String,
    /// Caps finished files plus partials; 0 means no cap.
    pub max_bytes: u64,
    /// Free space the filesystem must keep after a write; 0 disables it.
    pub min_free_bytes: u64,
    /// Lets the store delete watched, synced copies (LRU) for room.
    pub evict_watched: bool,
}

impl OfflineConfig {
    pub fn new(dir: &str) -> Self {
        OfflineConfig {
            dir: dir.to_string(),
            max_bytes: DEFAULT_MAX_BYTES,
            min_free_bytes: DEFAULT_MIN_FREE_BYTES,
            evict_watched: true,
        }
    }
}

/// Free space of the filesystem holding a path, `None` when unknown
/// (the floor is then skipped and only the budget applies, as in Go).
pub trait DiskSpace: Send + Sync {
    fn free_bytes(&self, dir: &Path) -> Option<u64>;
}

/// The OS probe: `statvfs` on Unix (Android included); unknown elsewhere
/// for now (Windows needs `GetDiskFreeSpaceExW`).
pub struct SystemDiskSpace;

impl DiskSpace for SystemDiskSpace {
    #[cfg(unix)]
    fn free_bytes(&self, dir: &Path) -> Option<u64> {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: `c` is a valid NUL-terminated path and `st` a properly
        // sized out-parameter that statvfs fully writes on success.
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            return None;
        }
        #[allow(clippy::unnecessary_cast)]
        Some(st.f_bavail as u64 * st.f_frsize as u64)
    }

    #[cfg(not(unix))]
    fn free_bytes(&self, _dir: &Path) -> Option<u64> {
        None
    }
}

/// An entry's place in the offline queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OfflineState {
    Queued,
    Paused,
    Done,
    Failed,
}

/// One offline copy, or a copy in progress.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OfflineEntry {
    pub server: String,
    pub item_id: String,
    pub title: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub season: i64,
    #[serde(default)]
    pub episode: i64,
    /// The file's base name inside the store.
    pub file: String,
    /// Expected bytes (catalog or the server's answer), 0 when unknown.
    pub size: u64,
    /// Bytes on disk so far.
    pub bytes: u64,
    /// ETag or Last-Modified of the bytes held, for `If-Range`.
    #[serde(default)]
    pub validator: String,
    pub state: OfflineState,
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub error: String,
    pub seq: u64,
    /// Client clock, milliseconds (LRU needs finer than seconds).
    pub added_at_ms: i64,
    #[serde(default)]
    pub done_at_ms: i64,
    /// Last local play or read, for LRU eviction.
    #[serde(default)]
    pub used_at_ms: i64,
    /// The server says it is completed (or it was completed offline).
    #[serde(default)]
    pub watched: bool,
}

impl OfflineEntry {
    fn last_use(&self) -> i64 {
        self.used_at_ms.max(self.done_at_ms)
    }
}

/// Told about a transfer as it runs.
pub trait FetchObserver: Send + Sync {
    fn progress(&self, done: u64, total: u64);
    /// A copy removed to make room.
    fn evicted(&self, entry: &OfflineEntry);
}

/// What a cleanup pass did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GcReport {
    pub orphan_parts: u64,
    pub vanished: u64,
    pub evicted: Vec<OfflineEntry>,
    pub freed_bytes: u64,
}

/// The opened store.
pub struct OfflineStore {
    db: Arc<ClientDb>,
    cfg: OfflineConfig,
    dir: PathBuf,
    disk: Arc<dyn DiskSpace>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn io_err(e: impl std::fmt::Display) -> ClientError {
    ClientError::Store { message: e.to_string() }
}

/// A file name safe on every OS, like Go's `downloads.SafeName`.
fn safe_name(s: &str, fallback: &str) -> String {
    let base = s.replace('\\', "/");
    let base = base.rsplit('/').next().unwrap_or("");
    let mut out: String = base
        .chars()
        .filter(|c| !c.is_control() && *c != '\u{fffd}')
        .map(|c| if matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') { '_' } else { c })
        .collect();
    out = out.trim().trim_start_matches('.').trim().to_string();
    while out.len() > 200 {
        out.pop();
    }
    if out.is_empty() { fallback.to_string() } else { out }
}

/// Records local use of an offline copy from a progress write (inside
/// the progress transaction).
pub(crate) fn note_use(
    tx: &redb::WriteTransaction,
    server: &str,
    item_id: &str,
    completed: bool,
) -> Result<(), ClientError> {
    let mut t = tx.open_table(OFFLINE).map_err(store_err)?;
    let k = db::key(&[server, item_id]);
    if let Some(mut e) = db::get::<OfflineEntry>(&t, &k)? {
        e.used_at_ms = now_ms();
        e.watched |= completed;
        db::put(&mut t, &k, &e)?;
    }
    Ok(())
}

/// Marks a copy watched after the server confirmed completion.
pub(crate) fn note_watched(tx: &redb::WriteTransaction, server: &str, item_id: &str) -> Result<(), ClientError> {
    let mut t = tx.open_table(OFFLINE).map_err(store_err)?;
    let k = db::key(&[server, item_id]);
    if let Some(mut e) = db::get::<OfflineEntry>(&t, &k)?
        && !e.watched
    {
        e.watched = true;
        db::put(&mut t, &k, &e)?;
    }
    Ok(())
}

/// Whether any user has unsynced progress on the item.
fn has_pending(tx: &redb::ReadTransaction, server: &str, item_id: &str) -> Result<bool, ClientError> {
    let q = tx.open_table(QUEUE).map_err(store_err)?;
    let rows: Vec<(String, serde_json::Value)> = db::scan(&q, &db::prefix(&[server]))?;
    Ok(rows.iter().any(|(k, _)| k.rsplit('\0').next() == Some(item_id)))
}

impl OfflineStore {
    /// Opens the store at `cfg.dir` over the client database.
    pub fn open(cfg: OfflineConfig, db: Arc<ClientDb>) -> Result<OfflineStore, ClientError> {
        Self::with_disk(cfg, db, Arc::new(SystemDiskSpace))
    }

    /// [`OfflineStore::open`] with another free-space probe.
    pub fn with_disk(
        cfg: OfflineConfig,
        db: Arc<ClientDb>,
        disk: Arc<dyn DiskSpace>,
    ) -> Result<OfflineStore, ClientError> {
        let dir = PathBuf::from(&cfg.dir);
        if !dir.is_absolute() {
            return Err(ClientError::Invalid { message: "offline dir must be an absolute path".into() });
        }
        std::fs::create_dir_all(dir.join(".parts")).map_err(|e| io_err(format!("{}: {e}", dir.display())))?;
        Ok(OfflineStore { db, cfg, dir, disk })
    }

    pub fn config(&self) -> OfflineConfig {
        self.cfg.clone()
    }

    fn all(&self) -> Result<Vec<OfflineEntry>, ClientError> {
        let mut v: Vec<OfflineEntry> = self
            .db
            .read(|tx| db::scan(&tx.open_table(OFFLINE).map_err(store_err)?, ""))?
            .into_iter()
            .map(|(_, e)| e)
            .collect();
        v.sort_by_key(|e: &OfflineEntry| e.seq);
        Ok(v)
    }

    /// Every entry, in queue order.
    pub fn list(&self) -> Result<Vec<OfflineEntry>, ClientError> {
        self.all()
    }

    pub fn get(&self, server: &str, item_id: &str) -> Result<Option<OfflineEntry>, ClientError> {
        self.db.read(|tx| db::get(&tx.open_table(OFFLINE).map_err(store_err)?, &db::key(&[server, item_id])))
    }

    fn save(&self, e: &OfflineEntry) -> Result<(), ClientError> {
        self.db
            .write(|tx| db::put(&mut tx.open_table(OFFLINE).map_err(store_err)?, &db::key(&[&e.server, &e.item_id]), e))
    }

    fn file_path(&self, e: &OfflineEntry) -> PathBuf {
        self.dir.join(&e.file)
    }

    fn part_path(&self, e: &OfflineEntry) -> PathBuf {
        self.dir
            .join(".parts")
            .join(format!("{}.part", safe_name(&format!("{}-{}", short_hash(&e.server), e.item_id), "item")))
    }

    /// The finished copy's path, `None` when there is none on disk.
    pub fn local_path(&self, server: &str, item_id: &str) -> Result<Option<String>, ClientError> {
        Ok(self
            .get(server, item_id)?
            .filter(|e| e.state == OfflineState::Done)
            .map(|e| self.file_path(&e))
            .filter(|p| p.is_file())
            .map(|p| p.to_string_lossy().into_owned()))
    }

    /// Queues an item of `server`. Adding one already in the store
    /// requeues it when paused or failed and otherwise returns it.
    pub fn add(&self, server: &str, item: &CatalogItem) -> Result<OfflineEntry, ClientError> {
        if item.id.is_empty() {
            return Err(ClientError::Invalid { message: "item id required".into() });
        }
        if let Some(mut cur) = self.get(server, &item.id)? {
            if matches!(cur.state, OfflineState::Paused | OfflineState::Failed) {
                (cur.state, cur.code, cur.error) = (OfflineState::Queued, String::new(), String::new());
                self.save(&cur)?;
            }
            return Ok(cur);
        }
        let source = if item.rel_path.is_empty() { &item.file_path } else { &item.rel_path };
        let name = safe_name(source, &safe_name(&item.title, &format!("item-{}", item.id)));
        let taken: Vec<String> = self.all()?.into_iter().map(|e| e.file).collect();
        let file = free_name(&self.dir, &name, &taken);
        self.db.write(|tx| {
            let seq = db::next_seq(tx)?;
            let e = OfflineEntry {
                server: server.to_string(),
                item_id: item.id.clone(),
                title: item.title.clone(),
                kind: item.kind.clone(),
                season: item.season,
                episode: item.episode,
                file,
                size: item.size.max(0) as u64,
                bytes: 0,
                validator: String::new(),
                state: OfflineState::Queued,
                code: String::new(),
                error: String::new(),
                seq,
                added_at_ms: now_ms(),
                done_at_ms: 0,
                used_at_ms: 0,
                watched: false,
            };
            db::put(&mut tx.open_table(OFFLINE).map_err(store_err)?, &db::key(&[server, &item.id]), &e)?;
            Ok(e)
        })
    }

    fn must(&self, server: &str, item_id: &str) -> Result<OfflineEntry, ClientError> {
        self.get(server, item_id)?.ok_or_else(|| ClientError::NotFound { message: "not in the offline store".into() })
    }

    /// Marks a queued entry paused: fetches skip it until resumed.
    pub fn pause(&self, server: &str, item_id: &str) -> Result<OfflineEntry, ClientError> {
        let mut e = self.must(server, item_id)?;
        if e.state == OfflineState::Done {
            return Err(ClientError::Invalid { message: "already downloaded".into() });
        }
        e.state = OfflineState::Paused;
        self.save(&e)?;
        Ok(e)
    }

    /// Requeues a paused or failed entry.
    pub fn resume(&self, server: &str, item_id: &str) -> Result<OfflineEntry, ClientError> {
        let mut e = self.must(server, item_id)?;
        if matches!(e.state, OfflineState::Paused | OfflineState::Failed) {
            (e.state, e.code, e.error) = (OfflineState::Queued, String::new(), String::new());
            self.save(&e)?;
        }
        Ok(e)
    }

    /// Deletes an entry and every byte it holds. Unsynced progress is
    /// refused unless `force`, so a position is never silently lost.
    pub fn remove(&self, server: &str, item_id: &str, force: bool) -> Result<u64, ClientError> {
        let e = self.must(server, item_id)?;
        if !force && self.db.read(|tx| has_pending(tx, server, item_id))? {
            return Err(ClientError::Invalid {
                message: "offline progress not synced yet (flush first, or force)".into(),
            });
        }
        let mut freed = 0;
        for p in [self.file_path(&e), self.part_path(&e)] {
            if let Ok(m) = std::fs::metadata(&p) {
                std::fs::remove_file(&p).map_err(|err| io_err(format!("{}: {err}", p.display())))?;
                freed += m.len();
            }
        }
        self.db.write(|tx| {
            tx.open_table(OFFLINE)
                .map_err(store_err)?
                .remove(db::key(&[server, item_id]).as_str())
                .map_err(store_err)?;
            Ok(())
        })?;
        Ok(freed)
    }

    /// What the store holds on disk: finished files and partials.
    pub fn used(&self) -> Result<u64, ClientError> {
        self.used_except("", "")
    }

    fn used_except(&self, server: &str, item_id: &str) -> Result<u64, ClientError> {
        Ok(self
            .all()?
            .iter()
            .filter(|e| !(e.server == server && e.item_id == item_id))
            .map(|e| match e.state {
                OfflineState::Done => std::fs::metadata(self.file_path(e)).map(|m| m.len()).unwrap_or(0),
                _ => e.bytes,
            })
            .sum())
    }

    /// Records local playback of an entry, for LRU eviction.
    pub fn touch(&self, server: &str, item_id: &str) -> Result<(), ClientError> {
        let mut e = self.must(server, item_id)?;
        e.used_at_ms = now_ms();
        self.save(&e)
    }

    /// Records the server's completed flag.
    pub fn set_watched(&self, server: &str, item_id: &str, watched: bool) -> Result<(), ClientError> {
        let mut e = self.must(server, item_id)?;
        e.watched = watched;
        self.save(&e)
    }

    /// D-094's check: may the store grow by `grow` bytes when it already
    /// accounts for `used`?
    fn check(&self, used: u64, grow: u64) -> Result<(), ClientError> {
        if grow == 0 {
            return Ok(());
        }
        let max = self.cfg.max_bytes;
        if max > 0 && used + grow > max {
            return Err(ClientError::NoRoom {
                code: CODE_QUOTA.into(),
                message: format!(
                    "needs {}, {} of {} budget left",
                    human(grow),
                    human(max.saturating_sub(used)),
                    human(max)
                ),
            });
        }
        if self.cfg.min_free_bytes > 0
            && let Some(free) = self.disk.free_bytes(&self.dir)
            && free.saturating_sub(grow) < self.cfg.min_free_bytes
        {
            return Err(ClientError::NoRoom {
                code: CODE_DISK_FULL.into(),
                message: format!(
                    "needs {}, {} free and {} must stay free",
                    human(grow),
                    human(free),
                    human(self.cfg.min_free_bytes)
                ),
            });
        }
        Ok(())
    }

    /// The least recently used watched copy with no unsynced progress.
    fn eviction_candidate(&self, server: &str, except: &str) -> Result<Option<OfflineEntry>, ClientError> {
        let mut best: Option<OfflineEntry> = None;
        for e in self.all()? {
            if (e.server == server && e.item_id == except) || e.state != OfflineState::Done || !e.watched {
                continue;
            }
            if self.db.read(|tx| has_pending(tx, &e.server, &e.item_id))? {
                continue;
            }
            if best.as_ref().is_none_or(|b| e.last_use() < b.last_use()) {
                best = Some(e);
            }
        }
        Ok(best)
    }

    /// Checks that `item_id` may grow by `n` bytes beyond `have`,
    /// evicting when the policy allows.
    fn make_room(&self, server: &str, item_id: &str, have: u64, n: u64) -> Result<Vec<OfflineEntry>, ClientError> {
        let mut evicted = Vec::new();
        loop {
            let err = match self.check(self.used_except(server, item_id)? + have, n) {
                Ok(()) => return Ok(evicted),
                Err(e) => e,
            };
            if !self.cfg.evict_watched {
                return Err(err);
            }
            let Some(v) = self.eviction_candidate(server, item_id)? else { return Err(err) };
            if self.remove(&v.server, &v.item_id, false).is_err() {
                return Err(err);
            }
            evicted.push(v);
        }
    }

    /// Transfers one entry of `client`'s server, resuming from its
    /// partial bytes when the server names the same representation
    /// (`If-Range`). On success the file moves into place and the entry
    /// is done; when offline it is paused with its part kept; any other
    /// failure marks it failed with a stable code. A route the server
    /// does not serve yet fails with [`ClientError::NotPorted`].
    pub async fn fetch(
        &self,
        client: &Client,
        item_id: &str,
        observer: Option<Arc<dyn FetchObserver>>,
    ) -> Result<OfflineEntry, ClientError> {
        let server = client.server();
        let mut e = self.must(&server, item_id)?;
        if e.state == OfflineState::Done {
            return Ok(e);
        }
        let part = self.part_path(&e);
        e.bytes = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        let res = self.transfer(client, &mut e, &part, observer.as_deref()).await;
        match &res {
            Ok(()) => match std::fs::rename(&part, self.file_path(&e)) {
                Ok(()) => {
                    (e.state, e.code, e.error) = (OfflineState::Done, String::new(), String::new());
                    e.done_at_ms = now_ms();
                }
                Err(err) => {
                    (e.state, e.code, e.error) = (OfflineState::Failed, "io-error".into(), err.to_string());
                }
            },
            Err(err) if err.is_offline() => e.state = OfflineState::Paused,
            Err(err) => {
                e.state = OfflineState::Failed;
                e.code = match err {
                    ClientError::NoRoom { code, .. } => code.clone(),
                    ClientError::NotPorted { .. } => lain_api::CODE_NOT_PORTED.into(),
                    ClientError::NotFound { .. } => "not-found".into(),
                    _ => "http-error".into(),
                };
                e.error = err.to_string();
            }
        }
        self.save(&e)?;
        let failed = e.state == OfflineState::Failed && e.code == "io-error";
        match res {
            Ok(()) if failed => Err(io_err(e.error.clone())),
            Ok(()) => Ok(e),
            Err(err) => Err(err),
        }
    }

    async fn transfer(
        &self,
        client: &Client,
        e: &mut OfflineEntry,
        part: &Path,
        observer: Option<&dyn FetchObserver>,
    ) -> Result<(), ClientError> {
        let mut headers = Vec::new();
        let have = e.bytes;
        if have > 0 && !e.validator.is_empty() {
            headers.push(("range", format!("bytes={have}-")));
            headers.push(("if-range", e.validator.clone()));
        }
        let path = stream_path(&e.item_id);
        let mut s = tokio::time::timeout(client.timeout(), client.open(&path, &headers))
            .await
            .map_err(|_| ClientError::Timeout)??;
        let header = |name: &str| s.headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let (append, total) = match s.status {
            206 => {
                let (start, total) = parse_content_range(&header("content-range")).unwrap_or((u64::MAX, 0));
                if start != have {
                    let _ = std::fs::remove_file(part);
                    e.bytes = 0;
                    return Err(ClientError::Offline {
                        message: "server answered a different range; restarting".into(),
                    });
                }
                (true, total)
            }
            200 => (false, header("content-length").parse().unwrap_or(0)),
            416 => {
                if header("content-range").rsplit('/').next().and_then(|t| t.trim().parse::<u64>().ok()) == Some(have) {
                    return Ok(());
                }
                let _ = std::fs::remove_file(part);
                e.bytes = 0;
                return Err(ClientError::Offline { message: "server refused the resume range; restarting".into() });
            }
            status => {
                let body = http_body_util::BodyExt::collect(s.body).await.map(|b| b.to_bytes()).unwrap_or_default();
                return Err(ClientError::from_response(status, &format!("GET {}", stream_path(&e.item_id)), &body));
            }
        };
        let etag = header("etag");
        e.validator = if !etag.is_empty() && !etag.starts_with("W/") { etag } else { header("last-modified") };
        let mut done = if append { have } else { 0 };
        if total > 0 {
            e.size = total;
        }
        // Keep the validator now: a transfer dropped mid-way (the CLI's
        // Ctrl+C) then resumes from its part file instead of restarting.
        self.save(e)?;
        let evict = |v: Vec<OfflineEntry>| {
            for x in &v {
                if let Some(o) = observer {
                    o.evicted(x);
                }
            }
        };
        if total > 0 {
            // Refuse up front when the whole remainder cannot fit.
            evict(self.make_room(&e.server, &e.item_id, done, total.saturating_sub(done))?);
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(part)
            .map_err(|err| io_err(format!("{}: {err}", part.display())))?;
        if let Some(o) = observer {
            o.progress(done, total);
        }
        loop {
            let chunk = match tokio::time::timeout(client.timeout(), next_chunk(&mut s.body)).await {
                Err(_) => Err(ClientError::Timeout),
                Ok(r) => r,
            };
            let chunk = match chunk {
                Ok(Some(c)) => c,
                Ok(None) => break,
                Err(err) => {
                    e.bytes = done;
                    return Err(err);
                }
            };
            if total == 0 {
                evict(self.make_room(&e.server, &e.item_id, done, chunk.len() as u64).inspect_err(|_| e.bytes = done)?);
            }
            f.write_all(&chunk).map_err(|err| {
                e.bytes = done;
                io_err(err)
            })?;
            done += chunk.len() as u64;
            e.bytes = done;
            if let Some(o) = observer {
                o.progress(done, total);
            }
        }
        f.sync_all().map_err(io_err)?;
        if total > 0 && done != total {
            return Err(ClientError::Offline { message: format!("short body: {done} of {total} bytes") });
        }
        e.size = done;
        Ok(())
    }

    /// D-094's `sync` second half: refreshes the watched flag of every
    /// finished copy of `client`'s server from the server, so eviction
    /// knows what is safe. Copies with unsynced progress are skipped.
    pub async fn refresh_watched(&self, client: &Client) -> Result<u64, ClientError> {
        let server = client.server();
        let mut n = 0;
        for e in self.all()? {
            if e.server != server || e.state != OfflineState::Done {
                continue;
            }
            if self.db.read(|tx| has_pending(tx, &e.server, &e.item_id))? {
                continue;
            }
            let p = client.progress(&e.item_id).await?;
            if p.completed != e.watched {
                self.set_watched(&server, &e.item_id, p.completed)?;
            }
            n += 1;
        }
        Ok(n)
    }

    /// Removes partial files no entry owns, forgets finished entries
    /// whose file was deleted by hand, and with `watched` removes every
    /// watched copy without unsynced progress.
    pub fn gc(&self, watched: bool) -> Result<GcReport, ClientError> {
        let mut rep = GcReport::default();
        let entries = self.all()?;
        let owned: Vec<PathBuf> = entries.iter().map(|e| self.part_path(e)).collect();
        if let Ok(rd) = std::fs::read_dir(self.dir.join(".parts")) {
            for d in rd.flatten() {
                let p = d.path();
                if owned.contains(&p) || !d.file_type().is_ok_and(|t| t.is_file()) {
                    continue;
                }
                let len = d.metadata().map(|m| m.len()).unwrap_or(0);
                if std::fs::remove_file(&p).is_ok() {
                    rep.orphan_parts += 1;
                    rep.freed_bytes += len;
                }
            }
        }
        for e in entries {
            if e.state != OfflineState::Done {
                continue;
            }
            let pending = self.db.read(|tx| has_pending(tx, &e.server, &e.item_id))?;
            if !self.file_path(&e).exists() && !pending {
                self.db.write(|tx| {
                    tx.open_table(OFFLINE)
                        .map_err(store_err)?
                        .remove(db::key(&[&e.server, &e.item_id]).as_str())
                        .map_err(store_err)?;
                    Ok(())
                })?;
                rep.vanished += 1;
                continue;
            }
            if watched && e.watched && !pending {
                rep.freed_bytes += self.remove(&e.server, &e.item_id, false)?;
                rep.evicted.push(e);
            }
        }
        Ok(rep)
    }
}

/// `bytes start-end/total` → (start, total).
fn parse_content_range(v: &str) -> Option<(u64, u64)> {
    let r = v.strip_prefix("bytes ")?;
    let (range, total) = r.split_once('/')?;
    let (start, _) = range.split_once('-')?;
    Some((start.trim().parse().ok()?, total.trim().parse().ok()?))
}

/// A name not used by another entry nor present on disk.
fn free_name(dir: &Path, name: &str, taken: &[String]) -> String {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let mut cand = name.to_string();
    let mut n = 2;
    while taken.contains(&cand) || dir.join(&cand).exists() {
        cand = format!("{stem} ({n}){ext}");
        n += 1;
    }
    cand
}

/// A short stable tag of a server URL, so part names of two servers'
/// items never collide.
fn short_hash(s: &str) -> String {
    // FNV-1a: stable across builds and platforms, unlike `DefaultHasher`.
    let mut h: u32 = 0x811c9dc5;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    format!("{h:08x}")
}

fn human(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 { format!("{n} B") } else { format!("{v:.1} {}", UNITS[u]) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_safe() {
        assert_eq!(
            safe_name("Frieren/[Fansub-A] Frieren - 01 [1080p].mkv", "x"),
            "[Fansub-A] Frieren - 01 [1080p].mkv"
        );
        assert_eq!(safe_name("C:\\m\\a:b?.mkv", "x"), "a_b_.mkv");
        assert_eq!(safe_name("..", "fallback"), "fallback");
        assert_eq!(parse_content_range("bytes 10-19/20"), Some((10, 20)));
        assert_eq!(parse_content_range("bytes */20"), None);
        assert_eq!(human(10 << 30), "10.0 GiB");
    }
}
