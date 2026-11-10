//! Extensions, tier 3 of the modding model (DESIGN-v2 §8.3, P6,
//! `docs/MODDING.md`): WebAssembly components with scoped access to the
//! lain API, the user's grants and the workers that run them.
//!
//! - `manifest.rs`: `extension.json`, scopes, install and uninstall.
//! - `runtime.rs`: the wasmtime host (limits, scope gates, workers).
//! - `api.rs`: the server side of the host API on `lain-client`.
//!
//! Grants live in `desktop.json` (`extensions.<id>`): `enabled`, the
//! `granted` scopes and the `reviewed` ones (what the permission prompt
//! has shown). An extension runs only when it is enabled and every scope
//! it asks for has been reviewed; it gets the requested scopes the user
//! granted, nothing else.

pub mod api;
pub mod manifest;
pub mod runtime;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

pub use manifest::{Installed, Manifest, Scope};
pub use runtime::{Cmd, Event, Grants, HostApi, Limits, Out, Panel, Rail};

/// The user's choices for one extension (`desktop.json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtSettings {
    pub enabled: bool,
    pub granted: Vec<String>,
    /// Scopes the permission prompt has shown (granted or not).
    pub reviewed: Vec<String>,
    /// Why it was last stopped (a trap, a load error); cleared on enable.
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Off,
    /// Waits for the permission prompt.
    NeedsReview,
    Starting,
    Running,
    Failed(String),
    /// The manifest does not parse.
    Broken(String),
}

impl Status {
    pub fn label(&self) -> String {
        match self {
            Status::Off => "off".into(),
            Status::NeedsReview => "waiting for your permission".into(),
            Status::Starting => "starting".into(),
            Status::Running => "running".into(),
            Status::Failed(r) => format!("stopped: {r}"),
            Status::Broken(r) => format!("broken: {r}"),
        }
    }
}

/// Something the app should tell the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// Rails or panels changed: redraw.
    Changed,
    /// An extension stopped and was disabled.
    Stopped { name: String, reason: String },
}

struct Running {
    worker: runtime::Worker,
    grants: Grants,
}

pub struct Manager {
    pub root: PathBuf,
    pub installed: Vec<Installed>,
    pub status: HashMap<String, Status>,
    /// Rails per extension, in the order the extension returned them.
    pub rails: BTreeMap<String, Vec<Rail>>,
    /// Panels for one item: (item id, [(extension, panel)]).
    pub panels: (String, Vec<(String, Panel)>),
    pub logs: HashMap<String, VecDeque<String>>,
    pub limits: Limits,
    runtime: Option<runtime::Runtime>,
    pub runtime_error: Option<String>,
    running: HashMap<String, Running>,
    api: Option<Arc<dyn HostApi>>,
    out: tokio::sync::mpsc::UnboundedSender<Out>,
}

impl Manager {
    pub fn new(root: PathBuf) -> (Manager, tokio::sync::mpsc::UnboundedReceiver<Out>) {
        let (out, rx) = tokio::sync::mpsc::unbounded_channel();
        let installed = manifest::list(&root);
        let m = Manager {
            root,
            installed,
            status: HashMap::new(),
            rails: BTreeMap::new(),
            panels: (String::new(), Vec::new()),
            logs: HashMap::new(),
            limits: Limits::default(),
            runtime: None,
            runtime_error: None,
            running: HashMap::new(),
            api: None,
            out,
        };
        (m, rx)
    }

    pub fn rescan(&mut self) {
        self.installed = manifest::list(&self.root);
    }

    pub fn find(&self, id: &str) -> Option<&Installed> {
        self.installed.iter().find(|i| i.id == id)
    }

    pub fn name(&self, id: &str) -> String {
        self.find(id).map(|i| i.name()).unwrap_or_else(|| id.to_string())
    }

    /// A new server session (or none): every running extension restarts
    /// against it.
    pub fn set_api(&mut self, api: Option<Arc<dyn HostApi>>) {
        self.api = api;
        self.stop_all();
    }

    pub fn stop_all(&mut self) {
        self.running.clear();
        self.rails.clear();
        self.panels.1.clear();
        for s in self.status.values_mut() {
            if matches!(s, Status::Running | Status::Starting) {
                *s = Status::Off;
            }
        }
    }

    /// Stops one extension (a reinstall restarts it on the next sync).
    pub fn stop_one(&mut self, id: &str) {
        self.running.remove(id);
        self.rails.remove(id);
    }

    /// The scopes an extension asks for that the prompt has not shown yet.
    pub fn unreviewed(&self, id: &str, cfg: &BTreeMap<String, ExtSettings>) -> Vec<Scope> {
        let Some(Ok(m)) = self.find(id).map(|i| &i.manifest) else { return Vec::new() };
        let reviewed = cfg.get(id).map(|s| s.reviewed.clone()).unwrap_or_default();
        m.requested().unwrap_or_default().into_iter().filter(|s| !reviewed.contains(&s.id())).collect()
    }

    /// The first installed extension waiting for the permission prompt.
    pub fn pending_review(&self, cfg: &BTreeMap<String, ExtSettings>) -> Option<String> {
        self.installed
            .iter()
            .filter(|i| i.manifest.is_ok())
            .find(|i| !self.unreviewed(&i.id, cfg).is_empty() && cfg.get(&i.id).is_none_or(|s| s.error.is_empty()))
            .map(|i| i.id.clone())
    }

    /// What an extension would run with: requested ∩ granted.
    pub fn grants(&self, id: &str, cfg: &BTreeMap<String, ExtSettings>) -> Grants {
        let Some(Ok(m)) = self.find(id).map(|i| &i.manifest) else { return Grants::default() };
        let granted = cfg.get(id).map(|s| s.granted.clone()).unwrap_or_default();
        Grants::new(m.requested().unwrap_or_default().into_iter().filter(|s| granted.contains(&s.id())).collect())
    }

    /// Starts, stops and restarts workers to match the settings.
    pub fn sync(&mut self, cfg: &BTreeMap<String, ExtSettings>) -> Vec<Notice> {
        let mut notes = Vec::new();
        let ids: Vec<String> = self.installed.iter().map(|i| i.id.clone()).collect();
        self.running.retain(|id, _| ids.contains(id));
        self.rails.retain(|id, _| ids.contains(id));
        for inst in self.installed.clone() {
            let id = inst.id.clone();
            let m = match &inst.manifest {
                Ok(m) => m.clone(),
                Err(e) => {
                    self.running.remove(&id);
                    self.status.insert(id, Status::Broken(e.clone()));
                    continue;
                }
            };
            let s = cfg.get(&id).cloned().unwrap_or_default();
            let want = s.enabled && self.unreviewed(&id, cfg).is_empty();
            let grants = self.grants(&id, cfg);
            if !want || self.api.is_none() {
                if self.running.remove(&id).is_some() {
                    self.rails.remove(&id);
                    notes.push(Notice::Changed);
                }
                let st = if !s.error.is_empty() {
                    Status::Failed(s.error.clone())
                } else if !self.unreviewed(&id, cfg).is_empty() {
                    Status::NeedsReview
                } else {
                    Status::Off
                };
                self.status.insert(id, st);
                continue;
            }
            if self.running.get(&id).is_some_and(|r| r.grants == grants) {
                continue;
            }
            // New, or its grants changed: (re)start it.
            self.running.remove(&id);
            self.rails.remove(&id);
            match self.start(&inst, &m, grants.clone()) {
                Ok(worker) => {
                    self.running.insert(id.clone(), Running { worker, grants });
                    self.status.insert(id, Status::Starting);
                }
                Err(e) => {
                    self.status.insert(id.clone(), Status::Failed(e.clone()));
                    notes.push(Notice::Stopped { name: m.name.clone(), reason: e });
                }
            }
            notes.push(Notice::Changed);
        }
        notes
    }

    fn start(&mut self, inst: &Installed, m: &Manifest, grants: Grants) -> Result<runtime::Worker, String> {
        if self.runtime.is_none() {
            match runtime::Runtime::new() {
                Ok(r) => self.runtime = Some(r),
                Err(e) => {
                    self.runtime_error = Some(e.clone());
                    return Err(e);
                }
            }
        }
        let api = self.api.clone().ok_or("not connected to a server")?;
        let wasm_path = inst.dir.join(manifest::safe_rel(&m.wasm).ok_or("bad wasm path")?);
        let size = std::fs::metadata(&wasm_path).map_err(|e| format!("{}: {e}", m.wasm))?.len();
        if size > manifest::MAX_WASM {
            return Err(format!("{} is larger than {} MiB", m.wasm, manifest::MAX_WASM >> 20));
        }
        let wasm = std::fs::read(&wasm_path).map_err(|e| format!("{}: {e}", m.wasm))?;
        let spec = runtime::Spec {
            id: inst.id.clone(),
            wasm,
            config: m.config_json(),
            grants,
            limits: self.limits.clone(),
            data_dir: inst.dir.join("data"),
            api,
        };
        let rt = self.runtime.as_ref().expect("runtime made above");
        Ok(rt.start(spec, self.out.clone()))
    }

    /// Sends to every running extension that holds `scope`.
    pub fn broadcast(&self, scope: &Scope, c: Cmd) {
        for r in self.running.values().filter(|r| r.grants.has(scope)) {
            r.worker.send(c.clone());
        }
    }

    /// Asks every panel extension for an item's panels.
    pub fn request_panels(&mut self, item: &str) {
        if self.panels.0 != item {
            self.panels = (item.to_string(), Vec::new());
        }
        self.broadcast(&Scope::UiPanel, Cmd::Panels(item.to_string()));
    }

    pub fn event(&self, e: Event) {
        self.broadcast(&Scope::EventsSubscribe, Cmd::Event(e));
    }

    #[cfg(test)]
    pub fn is_running(&self, id: &str) -> bool {
        self.running.contains_key(id)
    }

    /// Applies a worker's output. A failure disables the extension in
    /// `cfg` (the caller saves it) and drops its rails.
    pub fn on_out(&mut self, o: Out, cfg: &mut BTreeMap<String, ExtSettings>) -> Option<Notice> {
        match o {
            Out::Ready { ext } => {
                if self.running.contains_key(&ext) {
                    self.status.insert(ext, Status::Running);
                }
                Some(Notice::Changed)
            }
            Out::Rails { ext, rails } => {
                if !self.running.contains_key(&ext) {
                    return None;
                }
                self.rails.insert(ext, rails);
                Some(Notice::Changed)
            }
            Out::Panels { ext, item, panels } => {
                if item != self.panels.0 || !self.running.contains_key(&ext) {
                    return None;
                }
                self.panels.1.retain(|(e, _)| *e != ext);
                self.panels.1.extend(panels.into_iter().map(|p| (ext.clone(), p)));
                self.panels.1.sort_by(|a, b| a.0.cmp(&b.0));
                Some(Notice::Changed)
            }
            Out::Log { ext, level, line } => {
                let log = self.logs.entry(ext).or_default();
                log.push_back(format!("{}: {line}", runtime::level_name(level)));
                while log.len() > 20 {
                    log.pop_front();
                }
                None
            }
            Out::Failed { ext, reason } => {
                self.running.remove(&ext);
                self.rails.remove(&ext);
                self.panels.1.retain(|(e, _)| *e != ext);
                self.status.insert(ext.clone(), Status::Failed(reason.clone()));
                let s = cfg.entry(ext.clone()).or_default();
                s.enabled = false;
                s.error = reason.clone();
                Some(Notice::Stopped { name: self.name(&ext), reason })
            }
        }
    }
}
