//! Installs components from a watched directory (D-077): drop
//! `<id>.json` plus its executable into `<data-dir>/plugins/` and the
//! provisioner spawns, handshakes and registers it; rewrite it to
//! reload; delete it to unregister and kill the process. Sockets live
//! in a hidden `.run` subdirectory so they never match the manifest
//! glob. A bad manifest or an unspawnable binary is a log line, never a
//! failed boot.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

use notify::{RecursiveMode, Watcher};

use lain_core::Registry;

use crate::Change;
use crate::host::{Process, Timeouts};
use crate::manifest::Manifest;

type OnChange = Box<dyn Fn(&Change) + Send + Sync>;

#[derive(Default)]
struct Inner {
    procs: BTreeMap<String, Arc<Process>>,
    /// manifest path -> (provider id, modtime, length) last seen.
    seen: BTreeMap<PathBuf, (String, SystemTime, u64)>,
}

/// The manifest watcher.
pub struct Provisioner {
    dir: PathBuf,
    reg: Arc<Registry>,
    timeouts: Timeouts,
    inner: Arc<Mutex<Inner>>,
    on_change: Arc<Mutex<Option<OnChange>>>,
    stop: Arc<AtomicBool>,
    watcher: Mutex<Option<notify::RecommendedWatcher>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Provisioner {
    pub fn new(dir: &Path, reg: Arc<Registry>) -> Self {
        Self::with_timeouts(dir, reg, Timeouts::default())
    }

    pub fn with_timeouts(dir: &Path, reg: Arc<Registry>, timeouts: Timeouts) -> Self {
        Provisioner {
            dir: dir.to_path_buf(),
            reg,
            timeouts,
            inner: Arc::new(Mutex::new(Inner::default())),
            on_change: Arc::new(Mutex::new(None)),
            stop: Arc::new(AtomicBool::new(false)),
            watcher: Mutex::new(None),
            thread: Mutex::new(None),
        }
    }

    /// Calls `f` after every install or removal.
    pub fn on_change(&self, f: impl Fn(&Change) + Send + Sync + 'static) {
        *self.on_change.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(f));
    }

    /// Provisions every manifest present, then watches. An unwatchable
    /// directory is reported; boot-time discovery has already run.
    pub fn start(&self) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        self.sync();
        let (tx, rx) = mpsc::channel::<()>();
        let mut w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if res.is_ok() {
                let _ = tx.send(());
            }
        })
        .map_err(|e| e.to_string())?;
        w.watch(&self.dir, RecursiveMode::NonRecursive).map_err(|e| e.to_string())?;
        *self.watcher.lock().unwrap_or_else(|e| e.into_inner()) = Some(w);
        let me = Reconciler {
            dir: self.dir.clone(),
            reg: self.reg.clone(),
            timeouts: self.timeouts,
            inner: self.inner.clone(),
            on_change: self.on_change.clone(),
        };
        let stop = self.stop.clone();
        let handle = std::thread::Builder::new()
            .name("plugins-watch".into())
            .spawn(move || {
                // Debounce: manifest writes arrive as bursts, so a pass
                // diffs the directory instead of trusting event names.
                loop {
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    match rx.recv_timeout(Duration::from_millis(200)) {
                        Ok(()) => {
                            while rx.recv_timeout(Duration::from_millis(300)).is_ok() {}
                            if stop.load(Ordering::SeqCst) {
                                return;
                            }
                            me.sync();
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        *self.thread.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
        Ok(())
    }

    /// One reconcile pass (also run by the watcher).
    pub fn sync(&self) {
        self.reconciler().sync();
    }

    fn reconciler(&self) -> Reconciler {
        Reconciler {
            dir: self.dir.clone(),
            reg: self.reg.clone(),
            timeouts: self.timeouts,
            inner: self.inner.clone(),
            on_change: self.on_change.clone(),
        }
    }

    /// Running component ids, sorted.
    pub fn installed(&self) -> Vec<String> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).procs.keys().cloned().collect()
    }

    /// The running process of a component (diagnostics and tests).
    pub fn process(&self, id: &str) -> Option<Arc<Process>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).procs.get(id).cloned()
    }

    /// Stops the watch and every component. Idempotent.
    pub fn close(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.watcher.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(h) = self.thread.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = h.join();
        }
        let procs: Vec<_> =
            std::mem::take(&mut self.inner.lock().unwrap_or_else(|e| e.into_inner()).procs).into_values().collect();
        for p in procs {
            p.close();
        }
    }
}

impl Drop for Provisioner {
    fn drop(&mut self) {
        self.close();
    }
}

struct Reconciler {
    dir: PathBuf,
    reg: Arc<Registry>,
    timeouts: Timeouts,
    inner: Arc<Mutex<Inner>>,
    on_change: Arc<Mutex<Option<OnChange>>>,
}

impl Reconciler {
    fn notify(&self, c: Change) {
        if let Some(f) = self.on_change.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            f(&c);
        }
    }

    fn sync(&self) {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(rd) => rd,
            Err(e) => {
                tracing::warn!(dir = %self.dir.display(), err = %e, "plugin dir unreadable");
                return;
            }
        };
        let mut present: BTreeMap<PathBuf, (String, SystemTime, u64)> = BTreeMap::new();
        let mut manifests = Vec::new();
        for e in entries.flatten() {
            let path = e.path();
            if !path.extension().is_some_and(|x| x == "json") || !path.is_file() {
                continue;
            }
            let Ok(meta) = e.metadata() else { continue };
            let stamp = (meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), meta.len());
            let parsed = std::fs::read(&path).ok().and_then(|raw| serde_json::from_slice::<Manifest>(&raw).ok());
            let Some(m) = parsed.filter(|m| !m.id.is_empty()) else {
                tracing::warn!(file = %path.display(), "invalid plugin manifest");
                continue;
            };
            present.insert(path.clone(), (m.id.clone(), stamp.0, stamp.1));
            manifests.push((path, m, stamp));
        }
        let mut to_install = Vec::new();
        let mut to_remove = Vec::new();
        {
            let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            for (path, m, stamp) in &manifests {
                let prev = inner.seen.get(path);
                let running = inner.procs.contains_key(&m.id);
                let unchanged = prev.is_some_and(|(id, t, l)| *id == m.id && *t == stamp.0 && *l == stamp.1);
                if running && unchanged {
                    continue;
                }
                if let Some((old_id, _, _)) = prev {
                    if *old_id != m.id {
                        to_remove.push(old_id.clone());
                    }
                }
                to_install.push(m.clone());
            }
            for (path, (id, _, _)) in &inner.seen {
                if !present.contains_key(path) && !present.values().any(|(pid, _, _)| pid == id) {
                    to_remove.push(id.clone());
                }
            }
        }
        for id in to_remove {
            self.remove(&id);
        }
        for m in to_install {
            self.reinstall(m);
        }
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).seen = present;
    }

    fn reinstall(&self, m: Manifest) {
        let id = m.id.clone();
        // The old process goes first: both would bind the same socket.
        let old = self.inner.lock().unwrap_or_else(|e| e.into_inner()).procs.remove(&id);
        if let Some(old) = old {
            old.close();
        }
        let proc = match Process::spawn_with(m.clone(), &self.dir.join(".run"), self.timeouts) {
            Ok(p) => Arc::new(p),
            Err(e) => {
                tracing::warn!(id = %id, err = %e, "plugin spawn failed");
                return;
            }
        };
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).procs.insert(id.clone(), proc.clone());
        // Registered under the manifest id: a component naming an
        // existing provider id takes over its bindings (D-074).
        self.reg.register(proc);
        tracing::info!(id = %id, capabilities = %m.capabilities.join(","), "plugin installed");
        self.notify(Change::Installed(id));
    }

    fn remove(&self, id: &str) {
        let proc = self.inner.lock().unwrap_or_else(|e| e.into_inner()).procs.remove(id);
        let Some(proc) = proc else { return };
        proc.close();
        if let Err(e) = self.reg.unregister(id) {
            tracing::warn!(id = %id, err = %e, "plugin unregister failed");
        }
        tracing::info!(id = %id, "plugin removed");
        self.notify(Change::Removed(id.to_string()));
    }
}
