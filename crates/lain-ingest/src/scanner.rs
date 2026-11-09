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

use lain_api::{Library, ScanStats, ScanStatus};
use lain_core::Registry;

use crate::scan::{CAP_INGEST_SCAN, LibraryDone, ScanInput};
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
    Finished(ScanStatus),
}

pub type Sink = Arc<dyn Fn(ScanEvent) + Send + Sync>;
pub type LibrarySource = Arc<dyn Fn() -> Vec<Library> + Send + Sync>;

#[derive(Default)]
struct Run {
    status: ScanStatus,
    libraries: i64,
    libraries_done: i64,
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
                ..Run::default()
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
        self.lock().status.clone()
    }

    pub fn running(&self) -> bool {
        self.lock().status.state == "running"
    }

    /// The ingest runner's per-library hook.
    pub fn library_done(&self, done: &LibraryDone) {
        let (libraries, libraries_done, trigger) = {
            let mut r = self.lock();
            r.libraries_done += 1;
            (r.libraries, r.libraries_done, r.status.trigger.clone())
        };
        self.emit(ScanEvent::Library { done: done.clone(), libraries, libraries_done, trigger });
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
                ..ScanStatus::default()
            };
            r.libraries = libs.len() as i64;
            r.libraries_done = 0;
        }
        self.emit(ScanEvent::Started { trigger: trigger.into(), libraries: libs.len() as i64 });
        let me = self.clone();
        let trigger = trigger.to_string();
        let spawned = std::thread::Builder::new().name("lain-scan".into()).spawn(move || me.run(libs, &trigger));
        if let Err(e) = spawned {
            self.finish(ScanStatus { state: "error".into(), error: e.to_string(), ..self.status() });
        }
        true
    }

    /// A manual scan of every library.
    pub fn start_all(self: &Arc<Self>, trigger: &str) -> bool {
        let libs = (self.libraries)();
        self.start(libs, trigger)
    }

    fn finish(&self, status: ScanStatus) {
        self.lock().status = status.clone();
        self.emit(ScanEvent::Finished(status));
    }

    fn run(&self, libs: Vec<Library>, trigger: &str) {
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
