//! `lain-source-watch` (`lain.source.watch@1`, D-068/D-076): watches
//! library roots and reports the libraries that changed, after a quiet
//! window. The server polls it and rescans what comes back dirty, so
//! deletions, additions and moves ride the one scan path. Network
//! mounts may report nothing; those keep reconciling through manual
//! scans. A port of `internal/plugins/sourcewatch` over `notify`'s
//! recursive watches.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use lain_core::{Error, Provider};

pub const WATCH_ID: &str = "lain-source-watch";
pub const CAP_SOURCE_WATCH: &str = "lain.source.watch@1";

/// Event bursts (a deleted season, a copied episode) collapse into one
/// rescan after this much quiet.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchLibrary {
    pub id: String,
    pub path: String,
}

/// The current library set: poll is the sync point, so adds and removes
/// need no other signal.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollInput {
    #[serde(default)]
    pub libraries: Vec<WatchLibrary>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollOutput {
    #[serde(default)]
    pub dirty: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirtyInput {
    #[serde(default)]
    pub library_id: String,
}

#[derive(Default)]
struct State {
    /// library id -> root
    known: BTreeMap<String, PathBuf>,
    /// library id -> the root's canonical spelling, when it differs
    /// (macOS reports `/private/var/...` for `/var/...`).
    canon: BTreeMap<String, PathBuf>,
    /// library id -> last event
    dirty: BTreeMap<String, Instant>,
    closed: bool,
}

impl State {
    /// The library whose root holds `path` (the deepest one).
    fn owner(&self, path: &Path) -> Option<String> {
        self.known
            .iter()
            .chain(self.canon.iter())
            .filter(|(_, root)| path.starts_with(root))
            .max_by_key(|(_, root)| root.components().count())
            .map(|(id, _)| id.clone())
    }
}

/// The provider. `fs: false` keeps only the bookkeeping (tests drive
/// it with `dirty`).
pub struct SourceWatch {
    debounce: Duration,
    fs: bool,
    state: Arc<Mutex<State>>,
    watcher: Mutex<Option<notify::RecommendedWatcher>>,
}

impl SourceWatch {
    pub fn new(debounce: Duration, fs: bool) -> Self {
        SourceWatch { debounce, fs, state: Arc::default(), watcher: Mutex::new(None) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn watcher(&self) -> Result<(), Error> {
        let mut w = self.watcher.lock().unwrap_or_else(|e| e.into_inner());
        if w.is_some() || !self.fs {
            return Ok(());
        }
        let state = self.state.clone();
        let made = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(ev) = res else { return };
            if ev.kind.is_access() {
                return;
            }
            let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
            if st.closed {
                return;
            }
            let owners: BTreeSet<String> = ev.paths.iter().filter_map(|p| st.owner(p)).collect();
            for id in owners {
                st.dirty.insert(id, Instant::now());
            }
        })
        .map_err(|e| Error::unavailable(format!("watcher unavailable: {e}")))?;
        *w = Some(made);
        Ok(())
    }

    /// Syncs the watched set to the live libraries and drains the dirty
    /// ones that stayed quiet for the debounce window.
    pub fn poll(&self, input: &PollInput) -> Result<PollOutput, Error> {
        if self.lock().closed {
            return Err(Error::unavailable("watcher closed"));
        }
        self.watcher()?;
        let live: BTreeMap<&str, PathBuf> =
            input.libraries.iter().map(|l| (l.id.as_str(), PathBuf::from(&l.path))).collect();
        let (fresh, gone) = {
            let st = self.lock();
            let fresh: Vec<(String, PathBuf)> = live
                .iter()
                .filter(|(id, p)| st.known.get(**id) != Some(p))
                .map(|(id, p)| (id.to_string(), p.clone()))
                .collect();
            let gone: Vec<(String, PathBuf)> = st
                .known
                .iter()
                .filter(|(id, _)| !live.contains_key(id.as_str()))
                .map(|(id, p)| (id.clone(), p.clone()))
                .collect();
            (fresh, gone)
        };
        // The state lock is never held across watch calls: notify's
        // event thread takes it in the callback, and a watch call waits
        // on that thread.
        let mut replaced = Vec::new();
        {
            let mut st = self.lock();
            for (id, _) in &gone {
                st.known.remove(id);
                st.canon.remove(id);
                st.dirty.remove(id);
            }
            for (id, root) in &fresh {
                if let Some(old) = st.known.insert(id.clone(), root.clone()) {
                    replaced.push(old);
                }
                match std::fs::canonicalize(root) {
                    Ok(c) if c != *root => {
                        st.canon.insert(id.clone(), c);
                    }
                    _ => {
                        st.canon.remove(id);
                    }
                }
            }
        }
        if let Some(w) = self.watcher.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            for root in gone.iter().map(|(_, r)| r).chain(&replaced) {
                let _ = w.unwatch(root);
            }
            for (id, root) in &fresh {
                if let Err(e) = w.watch(root, RecursiveMode::Recursive) {
                    // Unwatchable roots keep reconciling through scans;
                    // the scan's unreadable report names them.
                    tracing::warn!(library = %id, root = %root.display(), err = %e, "library root not watchable");
                }
            }
        }
        let mut st = self.lock();
        let now = Instant::now();
        let mut out = PollOutput::default();
        let ids: Vec<String> = st.dirty.keys().cloned().collect();
        for id in ids {
            if !live.contains_key(id.as_str()) {
                st.dirty.remove(&id);
                continue;
            }
            if st.dirty.get(&id).is_some_and(|at| now.duration_since(*at) >= self.debounce) {
                st.dirty.remove(&id);
                out.dirty.push(id);
            }
        }
        Ok(out)
    }

    /// Marks a library dirty now (the re-nudge while a scan runs).
    pub fn mark(&self, library_id: &str) {
        let mut st = self.lock();
        if !st.closed {
            st.dirty.insert(library_id.to_string(), Instant::now());
        }
    }

    /// Whether a library has an active watch.
    pub fn watching(&self, library_id: &str) -> bool {
        self.lock().known.contains_key(library_id)
    }

    pub fn close(&self) {
        self.lock().closed = true;
        self.watcher.lock().unwrap_or_else(|e| e.into_inner()).take();
    }
}

impl Provider for SourceWatch {
    fn id(&self) -> &str {
        WATCH_ID
    }

    fn capabilities(&self) -> Vec<String> {
        vec![CAP_SOURCE_WATCH.into()]
    }

    fn health(&self) -> Result<(), Error> {
        if self.lock().closed { Err(Error::unavailable("watcher closed")) } else { Ok(()) }
    }

    fn invoke(&self, cap: &str, op: &str, input: Value) -> Result<Value, Error> {
        if cap != CAP_SOURCE_WATCH {
            return Err(Error::invalid(format!("unsupported cap {cap}")));
        }
        match op {
            "poll" => {
                let i: PollInput =
                    serde_json::from_value(input).map_err(|e| Error::invalid(format!("PollInput required: {e}")))?;
                Ok(json!(self.poll(&i)?))
            }
            "dirty" => {
                let i: DirtyInput =
                    serde_json::from_value(input).map_err(|e| Error::invalid(format!("DirtyInput required: {e}")))?;
                self.mark(&i.library_id);
                Ok(json!(true))
            }
            "close" => {
                self.close();
                Ok(json!(true))
            }
            _ => Err(Error::invalid(format!("capability {cap} has no op {op}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn libs(ids: &[(&str, &str)]) -> PollInput {
        PollInput {
            libraries: ids.iter().map(|(i, p)| WatchLibrary { id: i.to_string(), path: p.to_string() }).collect(),
        }
    }

    #[test]
    fn debounce_and_library_diff() {
        let w = SourceWatch::new(Duration::from_millis(50), false);
        let set = libs(&[("a", "/m/a"), ("b", "/m/b")]);
        assert!(w.poll(&set).unwrap().dirty.is_empty());
        assert!(w.watching("a"));
        w.mark("a");
        w.mark("a");
        assert!(w.poll(&set).unwrap().dirty.is_empty(), "still inside the window");
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(w.poll(&set).unwrap().dirty, vec!["a".to_string()]);
        assert!(w.poll(&set).unwrap().dirty.is_empty(), "drained once");
        // A removed library forgets its pending state.
        w.mark("b");
        let only_a = libs(&[("a", "/m/a")]);
        std::thread::sleep(Duration::from_millis(80));
        assert!(w.poll(&only_a).unwrap().dirty.is_empty());
        assert!(!w.watching("b"));
        w.close();
        w.mark("a");
        assert_eq!(w.poll(&only_a).unwrap_err().code, "dependency-unavailable");
        assert!(w.health().is_err());
    }

    #[test]
    fn deepest_root_owns_a_path() {
        let mut st = State::default();
        st.known.insert("m".into(), PathBuf::from("/m"));
        st.known.insert("a".into(), PathBuf::from("/m/anime"));
        assert_eq!(st.owner(Path::new("/m/anime/x.mkv")).as_deref(), Some("a"));
        assert_eq!(st.owner(Path::new("/m/movies/x.mkv")).as_deref(), Some("m"));
        assert_eq!(st.owner(Path::new("/other/x.mkv")), None);
    }

    #[test]
    fn file_events_mark_the_library() {
        let dir = std::env::temp_dir().join(format!("lain-watch-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let w = SourceWatch::new(Duration::from_millis(100), true);
        let set = libs(&[("lib", &dir.to_string_lossy())]);
        w.poll(&set).unwrap();
        std::fs::write(dir.join("sub/new.mkv"), b"x").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut got = Vec::new();
        while got.is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
            got = w.poll(&set).unwrap().dirty;
        }
        assert_eq!(got, vec!["lib".to_string()]);
        w.close();
        std::fs::remove_dir_all(dir).ok();
    }
}
