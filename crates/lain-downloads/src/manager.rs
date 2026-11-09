//! The server download queue (Go `manager.go`). Jobs and settings
//! persist in the store's `downloads` table (`settings`, `job/<id>`); a
//! restart resumes what was running. Until a job is done its bytes
//! accumulate in a hidden sibling part, so a library scan never sees
//! half a file.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, RwLock, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lain_store::{Bucket, Store};

use crate::fetch::{FetchRequest, fetch};
use crate::http::{Cancel, Http, parse_url};
use crate::limits::{FreeSpace, check_with, disk_free, full_with};
use crate::names::{free_path, name_from_url, safe_name};
use crate::{
    CODE_INVALID, CODE_IO, CODE_NOT_FOUND, CODE_STATE, CleanupReport, DownloadSettings, DownloadUsage, Error,
    JOB_CANCELED, JOB_DONE, JOB_FAILED, JOB_PAUSED, JOB_QUEUED, JOB_RUNNING, Job, Limits, unix,
};

/// Defaults are deliberately small for a disk-constrained host; the
/// operator raises them from the settings.
pub const DEFAULT_MAX_BYTES: i64 = 20 << 30;
pub const DEFAULT_MIN_FREE_BYTES: i64 = 5 << 30;
pub const DEFAULT_CONCURRENCY: i64 = 2;
pub const DEFAULT_KEEP_DAYS: i64 = 30;
pub const DEFAULT_MAX_RETRIES: i64 = 5;
const MAX_CONCURRENCY: i64 = 8;
const MAX_RETRIES: i64 = 20;

/// How long pause and cancel wait for a transfer to unwind, so their
/// answer is the settled state rather than "still running".
const STOP_WAIT: Duration = Duration::from_secs(5);

/// The least time between two progress reports of one job.
const PROGRESS_EVERY: Duration = Duration::from_secs(1);

const KEY_SETTINGS: &[u8] = b"settings";
const JOB_PREFIX: &str = "job/";

/// The first-boot policy for a data dir.
pub fn default_settings(data_dir: &Path) -> DownloadSettings {
    DownloadSettings {
        dir: data_dir.join("downloads").to_string_lossy().into_owned(),
        limits: Limits { max_bytes: DEFAULT_MAX_BYTES, min_free_bytes: DEFAULT_MIN_FREE_BYTES },
        concurrency: DEFAULT_CONCURRENCY,
        keep_finished_days: DEFAULT_KEEP_DAYS,
        max_retries: DEFAULT_MAX_RETRIES,
    }
}

/// Lexical clean of an absolute path (Go's `filepath.Clean`).
fn clean(p: &str) -> String {
    let pb: PathBuf = Path::new(p).components().collect();
    pb.to_string_lossy().into_owned()
}

/// Normalizes settings or explains why they cannot be saved.
pub fn validate_settings(mut s: DownloadSettings) -> Result<DownloadSettings, Error> {
    s.dir = s.dir.trim().to_string();
    if s.dir.is_empty() || !Path::new(&s.dir).is_absolute() {
        return Err(Error::new(CODE_INVALID, "dir must be an absolute path"));
    }
    s.dir = clean(&s.dir);
    if s.max_retries < 0 || s.max_retries > MAX_RETRIES {
        return Err(Error::new(CODE_INVALID, "max_retries must be 0-20"));
    }
    if s.limits.max_bytes < 0 || s.limits.min_free_bytes < 0 || s.keep_finished_days < 0 {
        return Err(Error::new(CODE_INVALID, "limits cannot be negative"));
    }
    if s.concurrency < 1 || s.concurrency > MAX_CONCURRENCY {
        return Err(Error::new(CODE_INVALID, "concurrency must be 1-8"));
    }
    Ok(s)
}

/// Doubles from 5 s up to 10 minutes, with ±20 % jitter so jobs failing
/// together do not retry together.
pub fn default_backoff(attempt: u32) -> Duration {
    let shift = attempt.saturating_sub(1).min(7);
    let d = (Duration::from_secs(5) * (1 << shift)).min(Duration::from_secs(600));
    let span = d.as_nanos() as u64 / 5 * 2 + 1;
    let mut b = [0u8; 8];
    let r = if getrandom::fill(&mut b).is_ok() { u64::from_le_bytes(b) % span } else { span / 2 };
    let jitter = r as i128 - (d.as_nanos() as i128 / 5);
    Duration::from_nanos((d.as_nanos() as i128 + jitter).max(0) as u64)
}

/// The part file of a job: a hidden sibling of its destination.
pub fn part_path(j: &Job) -> PathBuf {
    Path::new(&j.dir).join(format!(".lain-{}.part", j.id))
}

fn terminal(state: &str) -> bool {
    matches!(state, JOB_DONE | JOB_FAILED | JOB_CANCELED)
}

/// A download request. `dir` is resolved by the caller (a library
/// directory, or empty for the settings' dir); `library_id` comes back
/// in the done hook so the caller can rescan.
#[derive(Debug, Clone, Default)]
pub struct AddInput {
    pub url: String,
    pub name: String,
    pub dir: String,
    pub library_id: String,
    pub created_by: String,
}

/// A job callback.
pub type Hook = Arc<dyn Fn(&Job) + Send + Sync>;
type Backoff = Arc<dyn Fn(u32) -> Duration + Send + Sync>;
type Clock = Arc<dyn Fn() -> SystemTime + Send + Sync>;

/// The cancel handle of an active transfer; `reason` is the state the
/// job lands in when the worker unwinds.
struct Run {
    cancel: Cancel,
    reason: &'static str,
    done: Arc<(Mutex<bool>, Condvar)>,
}

struct Inner {
    settings: DownloadSettings,
    jobs: HashMap<String, Job>,
    seq: u64,
    running: HashMap<String, Run>,
    started: bool,
    closed: bool,
    last_report: HashMap<String, Instant>,
    backoff: Backoff,
    clock: Clock,
}

/// The server download queue.
pub struct Manager {
    store: Arc<Store>,
    http: Arc<dyn Http>,
    free: RwLock<FreeSpace>,
    inner: Mutex<Inner>,
    kick: SyncSender<()>,
    kick_rx: Mutex<Option<mpsc::Receiver<()>>>,
    threads: Mutex<Vec<JoinHandle<()>>>,
    on_done: RwLock<Option<Hook>>,
    on_change: RwLock<Option<Hook>>,
}

fn store_err(e: impl std::fmt::Display) -> Error {
    Error::new(CODE_IO, e.to_string())
}

impl Manager {
    /// Loads persisted jobs and settings. Jobs that were running when the
    /// process stopped come back queued; their parts resume.
    pub fn open(store: Arc<Store>, defaults: DownloadSettings, http: Arc<dyn Http>) -> Result<Arc<Manager>, Error> {
        let (settings_raw, rows) = store
            .read(|tx| {
                Ok((
                    tx.get(Bucket::Downloads, KEY_SETTINGS)?,
                    tx.scan_prefix(Bucket::Downloads, JOB_PREFIX.as_bytes())?,
                ))
            })
            .map_err(store_err)?;
        let mut settings = defaults.clone();
        if let Some(raw) = settings_raw
            && let Ok(serde_json::Value::Object(saved)) = serde_json::from_slice::<serde_json::Value>(&raw)
            && let Ok(serde_json::Value::Object(mut base)) = serde_json::to_value(&defaults)
        {
            // Fields a newer version added keep their defaults.
            base.extend(saved);
            if let Ok(s) = serde_json::from_value::<DownloadSettings>(serde_json::Value::Object(base))
                && let Ok(v) = validate_settings(s)
            {
                settings = v;
            }
        }
        let mut jobs = HashMap::new();
        let mut seq = 0;
        for (_, v) in rows {
            // A corrupt record is skipped, not fatal.
            let Ok(mut j) = serde_json::from_slice::<Job>(&v) else { continue };
            if j.state == JOB_RUNNING {
                j.state = JOB_QUEUED.into();
            }
            if !terminal(&j.state) {
                j.bytes = std::fs::metadata(part_path(&j)).map(|m| m.len() as i64).unwrap_or(0);
            }
            seq = seq.max(j.seq);
            jobs.insert(j.id.clone(), j);
        }
        let (kick, rx) = mpsc::sync_channel(1);
        Ok(Arc::new(Manager {
            store,
            http,
            free: RwLock::new(disk_free),
            inner: Mutex::new(Inner {
                settings,
                jobs,
                seq,
                running: HashMap::new(),
                started: false,
                closed: false,
                last_report: HashMap::new(),
                backoff: Arc::new(default_backoff),
                clock: Arc::new(SystemTime::now),
            }),
            kick,
            kick_rx: Mutex::new(Some(rx)),
            threads: Mutex::new(Vec::new()),
            on_done: RwLock::new(None),
            on_change: RwLock::new(None),
        }))
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Called (outside the lock) after a job's file is in place, e.g. to
    /// rescan its library.
    pub fn on_done(&self, f: Hook) {
        *self.on_done.write().unwrap_or_else(|e| e.into_inner()) = Some(f);
    }

    /// Called (outside the lock) on every state change and at most once
    /// a second per job while bytes move. Removed records are not
    /// reported.
    pub fn on_change(&self, f: Hook) {
        *self.on_change.write().unwrap_or_else(|e| e.into_inner()) = Some(f);
    }

    /// The wait before retry number `attempt` (1-based); tests shorten it.
    pub fn set_backoff(&self, f: impl Fn(u32) -> Duration + Send + Sync + 'static) {
        self.lock().backoff = Arc::new(f);
    }

    pub fn set_clock(&self, f: impl Fn() -> SystemTime + Send + Sync + 'static) {
        self.lock().clock = Arc::new(f);
    }

    /// Replaces the free-space probe (tests).
    pub fn set_free_space(&self, f: FreeSpace) {
        *self.free.write().unwrap_or_else(|e| e.into_inner()) = f;
    }

    fn free(&self) -> FreeSpace {
        *self.free.read().unwrap_or_else(|e| e.into_inner())
    }

    fn emit(&self, jobs: Vec<Job>) {
        if jobs.is_empty() {
            return;
        }
        let hook = self.on_change.read().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(h) = hook {
            for j in &jobs {
                h(j);
            }
        }
    }

    fn poke(&self) {
        let _ = self.kick.try_send(());
    }

    /// Launches the scheduler. Safe to call once; `close` stops it.
    pub fn start(self: &Arc<Self>) {
        {
            let mut g = self.lock();
            if g.started || g.closed {
                return;
            }
            g.started = true;
        }
        let Some(rx) = self.kick_rx.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
        let weak: Weak<Manager> = Arc::downgrade(self);
        let h = std::thread::Builder::new()
            .name("downloads".into())
            .spawn(move || {
                let mut wake: Option<SystemTime> = None;
                loop {
                    let wait = wake
                        .map(|t| t.duration_since(SystemTime::now()).unwrap_or(Duration::ZERO))
                        .unwrap_or(Duration::from_secs(3600));
                    match rx.recv_timeout(wait) {
                        Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                    let Some(m) = weak.upgrade() else { return };
                    if m.lock().closed {
                        return;
                    }
                    wake = m.schedule();
                }
            })
            .ok();
        if let Some(h) = h {
            self.threads.lock().unwrap_or_else(|e| e.into_inner()).push(h);
        }
        self.poke();
    }

    /// Stops every transfer. Running jobs persist as queued so the next
    /// start resumes them from their parts.
    pub fn close(&self) {
        {
            let mut g = self.lock();
            if g.closed {
                return;
            }
            g.closed = true;
            for r in g.running.values_mut() {
                r.reason = JOB_QUEUED;
                r.cancel.cancel();
            }
        }
        self.poke();
        let hs: Vec<JoinHandle<()>> = std::mem::take(&mut *self.threads.lock().unwrap_or_else(|e| e.into_inner()));
        for h in hs {
            let _ = h.join();
        }
    }

    /// Starts queued jobs, oldest first, up to the concurrency; returns
    /// when the earliest waiting retry is due.
    fn schedule(self: &Arc<Self>) -> Option<SystemTime> {
        let mut changed = Vec::new();
        let mut g = self.lock();
        if g.closed {
            return None;
        }
        let now = unix((g.clock)());
        let mut next = 0i64;
        let mut queued: Vec<(u64, String)> = Vec::new();
        for j in g.jobs.values() {
            if j.state != JOB_QUEUED {
                continue;
            }
            if j.retry_at > now {
                if next == 0 || j.retry_at < next {
                    next = j.retry_at;
                }
                continue;
            }
            queued.push((j.seq, j.id.clone()));
        }
        queued.sort();
        for (_, id) in queued {
            if g.running.len() as i64 >= g.settings.concurrency {
                break;
            }
            let run =
                Run { cancel: Cancel::new(), reason: JOB_FAILED, done: Arc::new((Mutex::new(false), Condvar::new())) };
            let (cancel, done) = (run.cancel.clone(), run.done.clone());
            g.running.insert(id.clone(), run);
            if let Some(j) = self.set_state(&mut g, &id, JOB_RUNNING, "", "") {
                changed.push(j);
            }
            let m = self.clone();
            let h = std::thread::Builder::new().name("download".into()).spawn(move || {
                m.work(&id, &cancel);
                let (lock, cv) = &*done;
                *lock.lock().unwrap_or_else(|e| e.into_inner()) = true;
                cv.notify_all();
            });
            if let Ok(h) = h {
                let mut ts = self.threads.lock().unwrap_or_else(|e| e.into_inner());
                ts.retain(|t| !t.is_finished());
                ts.push(h);
            }
        }
        drop(g);
        self.emit(changed);
        (next > 0).then(|| UNIX_EPOCH + Duration::from_secs(next as u64))
    }

    /// Transfers one job and settles its state.
    fn work(self: &Arc<Self>, id: &str, cancel: &Cancel) {
        let Some(j) = self.lock().jobs.get(id).cloned() else { return };
        let part = part_path(&j);
        let (res, err) = match std::fs::create_dir_all(&j.dir) {
            Ok(()) => {
                let reserve = |n: i64| self.reserve(id, n);
                let progress = |done: i64, total: i64| self.progress(id, done, total);
                let mut req = FetchRequest::new(&j.url, &part);
                req.validator = j.validator.clone();
                req.total = j.total.max(0);
                req.ranges = j.ranges;
                req.reserve = Some(&reserve);
                req.progress = Some(&progress);
                fetch(&*self.http, &req, cancel)
            }
            Err(e) => (Default::default(), Some(Error::new(CODE_IO, e.to_string()))),
        };

        let mut changed = Vec::new();
        let mut done_job = None;
        let mut g = self.lock();
        let run = g.running.remove(id);
        let reason = run.as_ref().map(|r| r.reason).unwrap_or(JOB_FAILED);
        g.last_report.remove(id);
        let now = (g.clock)();
        let max_retries = g.settings.max_retries;
        let backoff = g.backoff.clone();
        if !g.jobs.contains_key(id) {
            // Removed while running.
            drop(g);
            let _ = std::fs::remove_file(&part);
            self.poke();
            return;
        }
        let reserved = reserved_paths(&g.jobs, id);
        let jp = g.jobs.get_mut(id).expect("checked above");
        jp.validator = res.validator.clone();
        jp.ranges = res.ranges;
        if res.total > 0 {
            jp.total = res.total;
        }
        if res.restarted {
            jp.total = res.total.max(-1);
        }
        match std::fs::metadata(&part) {
            Ok(m) => jp.bytes = m.len() as i64,
            Err(_) if err.is_some() => jp.bytes = 0,
            Err(_) => {}
        }
        // Progress is progress: a transfer that moved bytes before
        // failing starts its retry budget over.
        if res.bytes > j.bytes {
            jp.attempts = 0;
        }
        let (state, code, msg): (&str, String, String) = if cancel.is_canceled() {
            // Stopped from outside: pause, cancel or shutdown decide.
            match reason {
                JOB_CANCELED => {
                    let _ = std::fs::remove_file(&part);
                    jp.bytes = 0;
                    (JOB_CANCELED, String::new(), String::new())
                }
                JOB_PAUSED => (JOB_PAUSED, String::new(), String::new()),
                _ => (JOB_QUEUED, String::new(), String::new()),
            }
        } else if let Some(e) = err {
            let code = if e.code.is_empty() { CODE_IO.to_string() } else { e.code.clone() };
            if e.retry && (jp.attempts as i64) < max_retries {
                jp.attempts += 1;
                let wait = backoff(jp.attempts).max(e.retry_after);
                // Second granularity: a sub-second wait runs on the next pass.
                jp.retry_at = unix(now + wait);
                (JOB_QUEUED, code, e.to_string())
            } else {
                (JOB_FAILED, code, e.to_string())
            }
        } else {
            if jp.name_auto && !res.filename.is_empty() {
                jp.name = safe_name(&res.filename, &jp.name);
            }
            let fin = free_path(Path::new(&jp.dir), &jp.name, &reserved);
            match std::fs::rename(&part, &fin) {
                Err(e) => (JOB_FAILED, CODE_IO.to_string(), e.to_string()),
                Ok(()) => {
                    jp.path = fin.to_string_lossy().into_owned();
                    jp.bytes = res.bytes;
                    jp.attempts = 0;
                    jp.retry_at = 0;
                    jp.finished_at = unix(now);
                    (JOB_DONE, String::new(), String::new())
                }
            }
        };
        if let Some(j) = self.set_state(&mut g, id, state, &code, &msg) {
            if state == JOB_DONE {
                done_job = Some(j.clone());
            }
            changed.push(j);
        }
        drop(g);
        self.emit(changed);
        if let Some(j) = done_job {
            let hook = self.on_done.read().unwrap_or_else(|e| e.into_inner()).clone();
            if let Some(h) = hook {
                h(&j);
            }
        }
        self.poke();
    }

    /// The budget check a transfer runs before it grows by `n`.
    fn reserve(&self, id: &str, n: i64) -> Result<(), Error> {
        let free = self.free();
        let g = self.lock();
        let Some(j) = g.jobs.get(id) else { return Err(Error::new(CODE_NOT_FOUND, "job removed")) };
        let used = used(&g.jobs, id) + j.bytes;
        check_with(&g.settings.limits, Path::new(&j.dir), used, n, free)
    }

    fn progress(&self, id: &str, done: i64, total: i64) {
        let mut g = self.lock();
        let Some(j) = g.jobs.get_mut(id) else { return };
        j.bytes = done;
        if total > 0 {
            j.total = total;
        }
        let snap = j.clone();
        let due = g.last_report.get(id).is_none_or(|t| t.elapsed() >= PROGRESS_EVERY);
        if due {
            g.last_report.insert(id.to_string(), Instant::now());
        }
        drop(g);
        if due {
            self.emit(vec![snap]);
        }
    }

    /// Records a transition and persists it; returns the new record.
    fn set_state(&self, g: &mut Inner, id: &str, state: &str, code: &str, msg: &str) -> Option<Job> {
        let now = unix((g.clock)());
        let j = g.jobs.get_mut(id)?;
        j.state = state.into();
        j.code = code.into();
        j.error = msg.into();
        j.updated_at = now;
        let j = j.clone();
        self.persist(&j);
        Some(j)
    }

    fn persist(&self, j: &Job) {
        let key = format!("{JOB_PREFIX}{}", j.id);
        if let Err(e) = self.store.write(|tx| tx.put_json(Bucket::Downloads, key.as_bytes(), j)) {
            tracing::warn!(download = %j.id, err = %e, "download record not saved");
        }
    }

    /// Validates and queues a download.
    pub fn add(&self, input: AddInput) -> Result<Job, Error> {
        let url = input.url.trim();
        if !parse_url(url).is_some_and(|u| u.scheme == "http" || u.scheme == "https") {
            return Err(Error::new(CODE_INVALID, "url must be an absolute http or https URL"));
        }
        let id = new_id()?;
        let free = self.free();
        let mut g = self.lock();
        let dir = if input.dir.is_empty() { g.settings.dir.clone() } else { input.dir.clone() };
        let name = input.name.trim();
        let auto = name.is_empty();
        let name = if auto { name_from_url(url) } else { name.to_string() };
        let now = unix((g.clock)());
        let j = Job {
            seq: g.seq + 1,
            id: id.clone(),
            url: url.to_string(),
            library_id: input.library_id,
            dir: clean(&dir),
            name: safe_name(&name, &format!("download-{id}")),
            name_auto: auto,
            state: JOB_QUEUED.into(),
            created_by: input.created_by,
            created_at: now,
            updated_at: now,
            total: -1,
            ..Default::default()
        };
        // A known-full store refuses at the door instead of queueing a job
        // that can only fail.
        full_with(&g.settings.limits, &existing_parent(Path::new(&j.dir)), used(&g.jobs, ""), free)?;
        g.seq = j.seq;
        g.jobs.insert(id, j.clone());
        self.persist(&j);
        drop(g);
        self.emit(vec![j.clone()]);
        self.poke();
        Ok(j)
    }

    /// Every job, newest first.
    pub fn list(&self) -> Vec<Job> {
        let mut out: Vec<Job> = self.lock().jobs.values().cloned().collect();
        out.sort_by_key(|j| std::cmp::Reverse(j.seq));
        out
    }

    pub fn get(&self, id: &str) -> Result<Job, Error> {
        self.lock().jobs.get(id).cloned().ok_or_else(unknown)
    }

    /// Stops a queued or running job, keeping its partial bytes.
    pub fn pause(&self, id: &str) -> Result<Job, Error> {
        self.stop(id, JOB_PAUSED, |m, g, j| match j.state.as_str() {
            JOB_QUEUED => {
                let id = j.id.clone();
                Ok(m.set_state(g, &id, JOB_PAUSED, "", ""))
            }
            JOB_PAUSED => Ok(None),
            s => Err(Error::new(CODE_STATE, format!("cannot pause a {s} download"))),
        })
    }

    /// Interrupts a running job, landing it in `reason`, and waits for the
    /// worker to settle; jobs that are not running go through `idle`.
    fn stop(
        &self,
        id: &str,
        reason: &'static str,
        idle: impl FnOnce(&Manager, &mut Inner, &Job) -> Result<Option<Job>, Error>,
    ) -> Result<Job, Error> {
        let mut g = self.lock();
        let Some(j) = g.jobs.get(id).cloned() else { return Err(unknown()) };
        if j.state == JOB_RUNNING
            && let Some(r) = g.running.get_mut(id)
        {
            r.reason = reason;
            r.cancel.cancel();
            let done = r.done.clone();
            drop(g);
            let (lock, cv) = &*done;
            let guard = lock.lock().unwrap_or_else(|e| e.into_inner());
            let _ = cv.wait_timeout_while(guard, STOP_WAIT, |d| !*d);
            return self.get(id);
        }
        let changed = idle(self, &mut g, &j)?;
        let out = g.jobs.get(id).cloned().unwrap_or(j);
        drop(g);
        self.emit(changed.into_iter().collect());
        Ok(out)
    }

    /// Requeues a paused or failed job; it continues from its part.
    pub fn resume(&self, id: &str) -> Result<Job, Error> {
        let mut g = self.lock();
        let Some(state) = g.jobs.get(id).map(|j| j.state.clone()) else { return Err(unknown()) };
        let changed = match state.as_str() {
            JOB_PAUSED | JOB_FAILED => {
                if let Some(j) = g.jobs.get_mut(id) {
                    j.attempts = 0;
                    j.retry_at = 0;
                }
                self.set_state(&mut g, id, JOB_QUEUED, "", "")
            }
            JOB_QUEUED | JOB_RUNNING => None,
            s => return Err(Error::new(CODE_STATE, format!("cannot resume a {s} download"))),
        };
        let out = g.jobs.get(id).cloned().ok_or_else(unknown)?;
        drop(g);
        self.emit(changed.into_iter().collect());
        self.poke();
        Ok(out)
    }

    /// Stops a job for good and deletes its partial bytes. A finished
    /// download cannot be canceled: its file is library media now.
    pub fn cancel(&self, id: &str) -> Result<Job, Error> {
        self.stop(id, JOB_CANCELED, |m, g, j| match j.state.as_str() {
            JOB_QUEUED | JOB_PAUSED | JOB_FAILED => {
                let _ = std::fs::remove_file(part_path(j));
                if let Some(x) = g.jobs.get_mut(&j.id) {
                    x.bytes = 0;
                }
                Ok(m.set_state(g, &j.id, JOB_CANCELED, "", ""))
            }
            JOB_CANCELED => Ok(None),
            s => Err(Error::new(CODE_STATE, format!("cannot cancel a {s} download"))),
        })
    }

    /// Forgets a finished, failed or canceled job (its partial bytes go
    /// with it; a finished file stays where it is).
    pub fn remove(&self, id: &str) -> Result<(), Error> {
        let mut g = self.lock();
        let Some(j) = g.jobs.get(id) else { return Err(unknown()) };
        if !terminal(&j.state) {
            return Err(Error::new(CODE_STATE, "cancel or let the download finish first"));
        }
        if j.state != JOB_DONE {
            let _ = std::fs::remove_file(part_path(j));
        }
        g.jobs.remove(id);
        let key = format!("{JOB_PREFIX}{id}");
        self.store.write(|tx| tx.delete(Bucket::Downloads, key.as_bytes()).map(|_| ())).map_err(store_err)
    }

    pub fn settings(&self) -> DownloadSettings {
        self.lock().settings.clone()
    }

    /// Validates and saves a new policy. Lowering concurrency lets running
    /// jobs finish; raising it starts queued ones.
    pub fn set_settings(&self, s: DownloadSettings) -> Result<DownloadSettings, Error> {
        let s = validate_settings(s)?;
        let mut g = self.lock();
        self.store.write(|tx| tx.put_json(Bucket::Downloads, KEY_SETTINGS, &s)).map_err(store_err)?;
        g.settings = s.clone();
        drop(g);
        self.poke();
        Ok(s)
    }

    /// The footprint against the limits.
    pub fn usage(&self) -> DownloadUsage {
        let free = self.free();
        let g = self.lock();
        let used = used(&g.jobs, "");
        let s = g.settings.clone();
        drop(g);
        DownloadUsage {
            used_bytes: used,
            max_bytes: s.limits.max_bytes,
            free_bytes: free(&existing_parent(Path::new(&s.dir))).unwrap_or(-1),
            min_free_bytes: s.limits.min_free_bytes,
        }
    }

    /// Removes partial bytes of failed and canceled jobs and, past the
    /// retention window, the records of terminal jobs. It never deletes a
    /// finished file: those are library media. Failures are logged; the
    /// report says what was freed either way.
    pub fn cleanup(&self) -> CleanupReport {
        let mut rep = CleanupReport::default();
        let mut g = self.lock();
        let now = (g.clock)();
        let cutoff = if g.settings.keep_finished_days > 0 {
            unix(now - Duration::from_secs(g.settings.keep_finished_days as u64 * 86400))
        } else {
            0
        };
        let mut errs: Vec<String> = Vec::new();
        let ids: Vec<String> = g.jobs.keys().cloned().collect();
        for id in ids {
            let j = g.jobs[&id].clone();
            if j.state == JOB_FAILED || j.state == JOB_CANCELED {
                let part = part_path(&j);
                if let Ok(m) = std::fs::metadata(&part) {
                    match std::fs::remove_file(&part) {
                        Ok(()) => {
                            rep.parts += 1;
                            rep.freed_bytes += m.len() as i64;
                        }
                        Err(e) => errs.push(e.to_string()),
                    }
                }
                if j.state == JOB_FAILED && j.bytes > 0 {
                    let x = g.jobs.get_mut(&id).expect("listed");
                    x.bytes = 0;
                    x.validator.clear();
                    let x = x.clone();
                    self.persist(&x);
                }
            }
            if terminal(&j.state) && cutoff > 0 && j.updated_at < cutoff {
                let key = format!("{JOB_PREFIX}{id}");
                if let Err(e) = self.store.write(|tx| tx.delete(Bucket::Downloads, key.as_bytes()).map(|_| ())) {
                    errs.push(e.to_string());
                    continue;
                }
                g.jobs.remove(&id);
                rep.records += 1;
            }
        }
        // Orphaned parts: a .lain-*.part in the download dir no job owns
        // (a record removed while the process was down).
        let owned: HashSet<PathBuf> = g.jobs.values().map(part_path).collect();
        let dir = PathBuf::from(&g.settings.dir);
        drop(g);
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                let p = dir.join(&n);
                if e.file_type().is_ok_and(|t| t.is_file())
                    && n.starts_with(".lain-")
                    && n.ends_with(".part")
                    && !owned.contains(&p)
                    && let Ok(m) = e.metadata()
                    && std::fs::remove_file(&p).is_ok()
                {
                    rep.parts += 1;
                    rep.freed_bytes += m.len() as i64;
                }
            }
        }
        if !errs.is_empty() {
            tracing::warn!(err = %errs.join("; "), "download cleanup incomplete");
        }
        rep
    }
}

fn unknown() -> Error {
    Error::new(CODE_NOT_FOUND, "unknown download")
}

/// Final paths other jobs already own.
fn reserved_paths(jobs: &HashMap<String, Job>, except: &str) -> HashSet<PathBuf> {
    jobs.iter().filter(|(id, j)| *id != except && !j.path.is_empty()).map(|(_, j)| PathBuf::from(&j.path)).collect()
}

/// What tracked jobs hold on disk or have committed to: a finished file
/// that still exists, a partial's bytes, and the full size of a running
/// transfer whose total is known (so two parallel jobs cannot both claim
/// the last free gigabyte).
fn used(jobs: &HashMap<String, Job>, except: &str) -> i64 {
    let mut used = 0i64;
    for (id, j) in jobs {
        if id == except {
            continue;
        }
        match j.state.as_str() {
            JOB_DONE => {
                if let Ok(m) = std::fs::metadata(&j.path) {
                    used += m.len() as i64;
                }
            }
            JOB_RUNNING => used += j.bytes.max(j.total),
            JOB_QUEUED | JOB_PAUSED | JOB_FAILED => used += j.bytes,
            _ => {}
        }
    }
    used
}

/// Walks up to a directory that exists, so free space can be reported
/// before the download dir is created.
fn existing_parent(p: &Path) -> PathBuf {
    let mut p = p.to_path_buf();
    loop {
        if p.exists() {
            return p;
        }
        match p.parent() {
            Some(parent) if parent != p => p = parent.to_path_buf(),
            _ => return p,
        }
    }
}

fn new_id() -> Result<String, Error> {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).map_err(|e| Error::new(CODE_IO, e.to_string()))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

#[cfg(test)]
mod tests;
