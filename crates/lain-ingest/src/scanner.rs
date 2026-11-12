//! The scan controller behind `POST/GET /api/library/scan`: one scan at
//! a time, run on its own thread through `lain.ingest.scan@1`, then the
//! metadata backfill (`lain.metadata.enrich@1` `backfill`, on unless
//! turned off); and the watch loop that polls `lain.source.watch@1` and
//! rescans dirty libraries. Observers hear every state change (the
//! server turns them into `/api/events`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Duration;

use serde_json::json;

use lain_api::{Library, ScanProgress, ScanStats, ScanStatus};
use lain_core::Registry;

use crate::scan::{CAP_INGEST_SCAN, LibraryDone, ScanInput, Step};
use crate::watch::{CAP_SOURCE_WATCH, PollInput, PollOutput, WatchLibrary};

pub const CAP_METADATA_ENRICH: &str = "lain.metadata.enrich@1";

/// How often the watch loop drains dirty libraries (detection latency;
/// the debounce lives in the watch provider).
pub const WATCH_POLL: Duration = Duration::from_millis(500);

/// What observers hear.
#[derive(Debug, Clone, PartialEq)]
pub enum ScanEvent {
    Started { trigger: String, libraries: i64 },
    Library { done: LibraryDone, libraries: i64, libraries_done: i64, trigger: String },
    Finished(Box<ScanStatus>),
}

pub type Sink = Arc<dyn Fn(ScanEvent) + Send + Sync>;
pub type LibrarySource = Arc<dyn Fn() -> Vec<Library> + Send + Sync>;

/// The running scan's `status.progress` is its live position; a
/// finished scan has none. `queued`: a manual scan was asked for while
/// this one ran (D-286); one follow-up of every library starts when it
/// ends, however many times it was asked for.
#[derive(Default)]
struct Run {
    status: ScanStatus,
    queued: bool,
}

/// What [`Scanner::request_all`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanRequest {
    Started,
    /// A scan was running: one follow-up is queued behind it.
    Queued,
}

/// Phases only move forward: roots walk in parallel, so a late walk
/// must not pull a scan that is already identifying back to `walk`.
fn advance(p: &mut ScanProgress, phase: &str) {
    const ORDER: [&str; 4] = ["walk", "identify", "commit", "enrich"];
    let rank = |s: &str| ORDER.iter().position(|o| *o == s);
    if rank(phase) > rank(&p.phase) {
        p.phase = phase.into();
    }
}

pub struct Scanner {
    registry: Weak<Registry>,
    libraries: LibrarySource,
    run: Mutex<Run>,
    sink: RwLock<Option<Sink>>,
    auto_enrich: AtomicBool,
    stopping: Arc<AtomicBool>,
    watching: AtomicBool,
}

impl Scanner {
    pub fn new(registry: &Arc<Registry>, libraries: LibrarySource, auto_enrich: bool) -> Arc<Scanner> {
        Arc::new(Scanner {
            registry: Arc::downgrade(registry),
            libraries,
            run: Mutex::new(Run {
                status: ScanStatus { state: "idle".into(), ..ScanStatus::default() },
                queued: false,
            }),
            sink: RwLock::new(None),
            auto_enrich: AtomicBool::new(auto_enrich),
            stopping: Arc::new(AtomicBool::new(false)),
            watching: AtomicBool::new(false),
        })
    }

    pub fn set_sink(&self, sink: Sink) {
        *self.sink.write().unwrap_or_else(|e| e.into_inner()) = Some(sink);
    }

    pub fn set_auto_enrich(&self, on: bool) {
        self.auto_enrich.store(on, Ordering::SeqCst);
    }

    fn emit(&self, ev: ScanEvent) {
        let sink = self.sink.read().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(s) = sink {
            s(ev);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Run> {
        self.run.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn status(&self) -> ScanStatus {
        let r = self.lock();
        ScanStatus { queued: r.queued, ..r.status.clone() }
    }

    pub fn running(&self) -> bool {
        self.lock().status.state == "running"
    }

    /// The ingest runner's hook: every step moves `status.progress`,
    /// a committed library is also an event.
    pub fn step(&self, step: Step<'_>) {
        let done: Option<(&LibraryDone, i64, i64, String)> = {
            let mut r = self.lock();
            let trigger = r.status.trigger.clone();
            let Some(p) = r.status.progress.as_mut() else { return };
            match step {
                Step::Walked { files } => {
                    p.libraries_walked += 1;
                    p.files += files;
                    advance(p, "identify");
                    None
                }
                Step::Identified => {
                    p.files_done += 1;
                    None
                }
                Step::Committing => {
                    advance(p, "commit");
                    None
                }
                Step::Library(done) => {
                    p.libraries_done += 1;
                    Some((done, p.libraries, p.libraries_done, trigger))
                }
            }
        };
        if let Some((done, libraries, libraries_done, trigger)) = done {
            self.emit(ScanEvent::Library { done: done.clone(), libraries, libraries_done, trigger });
        }
    }

    /// The enricher's backfill hook: `done` of `total` items. Heard
    /// only in the scan's own enrich phase, so a backfill someone else
    /// asked for never moves a scan that is still reading files.
    pub fn enrich_progress(&self, done: i64, total: i64) {
        if let Some(p) = self.lock().status.progress.as_mut()
            && p.phase == "enrich"
        {
            p.enrich_done = done;
            p.enrich_total = total;
        }
    }

    /// Starts a scan of `libs` unless one is running (`false` then).
    pub fn start(self: &Arc<Self>, libs: Vec<Library>, trigger: &str) -> bool {
        {
            let mut r = self.lock();
            if r.status.state == "running" {
                return false;
            }
            r.status = ScanStatus {
                state: "running".into(),
                started_at: lain_core::now_unix(),
                trigger: trigger.into(),
                progress: Some(ScanProgress {
                    phase: "walk".into(),
                    libraries: libs.len() as i64,
                    ..ScanProgress::default()
                }),
                ..ScanStatus::default()
            };
        }
        self.emit(ScanEvent::Started { trigger: trigger.into(), libraries: libs.len() as i64 });
        let me = self.clone();
        let trigger = trigger.to_string();
        let spawned = std::thread::Builder::new().name("lain-scan".into()).spawn(move || me.run(libs, &trigger));
        if let Err(e) = spawned {
            self.finish(ScanStatus { state: "error".into(), error: e.to_string(), progress: None, ..self.status() });
        }
        true
    }

    /// A manual scan of every library.
    pub fn start_all(self: &Arc<Self>, trigger: &str) -> bool {
        let libs = (self.libraries)();
        self.start(libs, trigger)
    }

    /// A manual scan of every library, or, while one runs, one follow-up
    /// queued behind it (D-286): the running scan walks only the
    /// libraries it started with, so a library added meanwhile would
    /// otherwise wait for the next scan someone asks for.
    pub fn request_all(self: &Arc<Self>, trigger: &str) -> ScanRequest {
        if self.start_all(trigger) {
            return ScanRequest::Started;
        }
        loop {
            {
                let mut r = self.lock();
                if r.status.state == "running" {
                    r.queued = true;
                    return ScanRequest::Queued;
                }
            }
            // The scan ended between the two looks: start this one now.
            if self.start_all(trigger) {
                return ScanRequest::Started;
            }
        }
    }

    fn finish(self: &Arc<Self>, status: ScanStatus) {
        let again = {
            let mut r = self.lock();
            r.status = ScanStatus { queued: false, ..status.clone() };
            std::mem::take(&mut r.queued)
        };
        self.emit(ScanEvent::Finished(Box::new(status)));
        if again {
            tracing::info!("starting the queued scan");
            self.start_all("manual");
        }
    }

    fn run(self: &Arc<Self>, libs: Vec<Library>, trigger: &str) {
        tracing::info!(libraries = libs.len(), trigger, "scan started");
        let started_at = self.status().started_at;
        let Some(reg) = self.registry.upgrade() else { return };
        let out = reg.call_one(CAP_INGEST_SCAN, "scan", json!(ScanInput { libraries: libs }));
        let mut stats = match out.and_then(|(v, _)| {
            serde_json::from_value::<ScanStats>(v).map_err(|e| lain_core::Error::internal(e.to_string()))
        }) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(err = %e, trigger, "scan failed");
                self.finish(ScanStatus {
                    state: "error".into(),
                    started_at,
                    finished_at: lain_core::now_unix(),
                    error: e.to_string(),
                    trigger: trigger.into(),
                    ..ScanStatus::default()
                });
                return;
            }
        };
        // Enrichment runs with the state still "running" but outside the
        // lock, so status reads never wait behind a long backlog.
        if self.auto_enrich.load(Ordering::SeqCst) {
            if let Some(p) = self.lock().status.progress.as_mut() {
                advance(p, "enrich");
            }
            tracing::info!(trigger, "scan committed; enriching metadata");
            stats.enriched = match reg.call_one(CAP_METADATA_ENRICH, "backfill", json!({})) {
                Ok((v, _)) => v["enriched"].as_i64().unwrap_or(0),
                Err(e) => {
                    tracing::warn!(err = %e, "auto-enrich failed");
                    0
                }
            };
        }
        tracing::info!(
            candidates = stats.candidates,
            identified = stats.identified,
            unidentified = stats.unidentified,
            migrated = stats.migrated,
            missing = stats.missing,
            restored = stats.restored,
            enriched = stats.enriched,
            errors = stats.errors,
            trigger,
            "scan done"
        );
        self.finish(ScanStatus {
            state: "done".into(),
            started_at,
            finished_at: stats.finished_at,
            stats: Some(stats),
            trigger: trigger.into(),
            ..ScanStatus::default()
        });
    }

    /// Rescans one library for the watcher; while a scan runs the
    /// library is re-marked dirty instead of racing it.
    pub fn scan_library(self: &Arc<Self>, id: &str) {
        let Some(lib) = (self.libraries)().into_iter().find(|l| l.id == id) else { return };
        if !self.start(vec![lib], "watch")
            && let Some(reg) = self.registry.upgrade()
        {
            let _ = reg.call_one(CAP_SOURCE_WATCH, "dirty", json!({"library_id": id}));
        }
    }

    /// One watch pass: hand the provider the library set, rescan what
    /// comes back dirty.
    pub fn poll_watch(self: &Arc<Self>) {
        let Some(reg) = self.registry.upgrade() else { return };
        let libs: Vec<WatchLibrary> =
            (self.libraries)().into_iter().map(|l| WatchLibrary { id: l.id, path: l.path }).collect();
        match reg.call_one(CAP_SOURCE_WATCH, "poll", json!(PollInput { libraries: libs })) {
            Ok((v, _)) => {
                for id in serde_json::from_value::<PollOutput>(v).map(|o| o.dirty).unwrap_or_default() {
                    self.scan_library(&id);
                }
            }
            Err(e) => tracing::debug!(err = %e, "watch poll failed"),
        }
    }

    /// Starts the watch loop (once). The first poll runs before this
    /// returns, so the roots are watched from then on.
    pub fn start_watching(self: &Arc<Self>) {
        if self.watching.swap(true, Ordering::SeqCst) {
            return;
        }
        self.poll_watch();
        let weak = Arc::downgrade(self);
        let stopping = self.stopping.clone();
        let _ = std::thread::Builder::new().name("lain-watch".into()).spawn(move || {
            loop {
                std::thread::sleep(WATCH_POLL);
                if stopping.load(Ordering::SeqCst) {
                    return;
                }
                match weak.upgrade() {
                    Some(s) => s.poll_watch(),
                    None => return,
                }
            }
        });
    }

    /// Ends the watch loop.
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
    }

    /// Waits until no scan runs (tests, shutdown); `false` on timeout.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        while self.running() {
            if std::time::Instant::now() > deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        true
    }
}
