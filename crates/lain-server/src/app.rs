//! The server's state: store, auth secret, registry with its in-process
//! providers, the module set, the event bus and the component
//! provisioner. Everything a handler needs hangs off [`App`].

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Duration;

use notify::{RecursiveMode, Watcher};
use serde_json::{Value, json};

use lain_auth::{Auth, Verified};
use lain_component::{Change as PluginChange, Provisioner};
use lain_core::modules::{self, ModuleSet};
use lain_core::{Binding, BindingMode, Composition, Registry};
use lain_store::Store;

use crate::catalog::{
    CAP_CATALOG_READ, CAP_CATALOG_WRITE, CAP_USER_PROGRESS, CATALOG_ID, Catalog, USERSTATE_ID, Userstate,
};
use crate::contracts::{CatalogItem, Library};
use crate::events::{EventBus, Scope, SubInfo};
use crate::libraries::Libraries;
use crate::privacy::{self, Unlocks};
use crate::roots::{self, Roots};
use lain_ingest::{ScanEvent, Scanner, SourceWatch};

/// The composition document version.
pub const COMPOSITION_VERSION: u32 = 8;

/// Built-in Go providers that `lain plugin-run` serves as components:
/// a binding may name them before their manifest is installed (they
/// answer `dependency-unavailable` until then). This is the strangler
/// path: Go providers keep working under the Rust core. The network
/// metadata providers stay here until the Rust server speaks HTTPS.
pub const COMPONENT_SERVED: &[&str] = &[
    "lain-release-model",
    "lain-probe-ffprobe",
    "lain-playback-default",
    "lain-thumbnail-ffmpeg",
    "lain-transcode-ffmpeg",
    "lain-metadata-kitsu",
    "lain-metadata-anilist",
    "lain-metadata-jikan",
    "lain-metadata-tvmaze",
];

/// The default composition: everything the Rust server serves
/// in-process (catalog, userstate, catalog population, identification,
/// metadata); the network metadata providers are components.
pub fn default_composition() -> Composition {
    use lain_identify::{ANIME_ID, CAP_MEDIA_IDENTIFY, CAP_RELEASE_PARSE, COMIC_ID, GENERIC_ID, TOKENIZER_ID};
    use lain_metadata::{CAP_METADATA_ENRICH, CAP_METADATA_RESOLVE, CAP_METADATA_SEARCH, DEFAULT_PROVIDERS, ENRICH_ID};
    let mut bindings = BTreeMap::new();
    let one = |id: &str| Binding::new(BindingMode::ExactlyOne, &[id]);
    bindings.insert(CAP_CATALOG_READ.into(), one(CATALOG_ID));
    bindings.insert(CAP_CATALOG_WRITE.into(), one(CATALOG_ID));
    bindings.insert(CAP_USER_PROGRESS.into(), one(USERSTATE_ID));
    bindings.insert(lain_ingest::CAP_SOURCE_ENUMERATE.into(), one(lain_ingest::SOURCE_ID));
    bindings.insert(lain_ingest::CAP_INGEST_SCAN.into(), one(lain_ingest::INGEST_ID));
    bindings.insert(lain_ingest::CAP_SOURCE_WATCH.into(), one(lain_ingest::WATCH_ID));
    bindings
        .insert(CAP_MEDIA_IDENTIFY.into(), Binding::new(BindingMode::OrderedMany, &[COMIC_ID, ANIME_ID, GENERIC_ID]));
    // The lain-parser model (P4) is bound before the tokenizer once it
    // is installed as a component.
    bindings.insert(CAP_RELEASE_PARSE.into(), Binding::new(BindingMode::OrderedMany, &[TOKENIZER_ID]));
    bindings.insert(CAP_METADATA_SEARCH.into(), Binding::new(BindingMode::MergeMany, DEFAULT_PROVIDERS));
    bindings.insert(CAP_METADATA_RESOLVE.into(), Binding::new(BindingMode::MergeMany, DEFAULT_PROVIDERS));
    bindings.insert(CAP_METADATA_ENRICH.into(), one(ENRICH_ID));
    Composition { version: COMPOSITION_VERSION, bindings }
}

/// Server construction options.
#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub version: String,
    /// SSE keep-alive and token re-check interval.
    pub keep_alive: Duration,
    /// Debounce for the `modules.json` watch.
    pub reload_debounce: Duration,
    /// Metadata backfill after every scan (`LAIN_AUTO_ENRICH=0` turns it
    /// off).
    pub auto_enrich: bool,
    /// Library file watching (`--watch=0` / `LAIN_WATCH=0` turns it off).
    pub watch: bool,
    /// Quiet window before a changed library is rescanned.
    pub watch_debounce: Duration,
}

impl Config {
    pub fn new(data_dir: &Path, version: &str) -> Self {
        Config {
            data_dir: data_dir.to_path_buf(),
            version: version.into(),
            keep_alive: Duration::from_secs(15),
            reload_debounce: Duration::from_millis(200),
            auto_enrich: true,
            watch: true,
            watch_debounce: lain_ingest::DEFAULT_DEBOUNCE,
        }
    }
}

/// Writes a JSON document atomically (temp file + rename, 0600), like
/// Go's `store.Dir.Save`.
pub fn save_doc(path: &Path, v: &impl serde::Serialize) -> Result<(), String> {
    let mut raw = serde_json::to_vec_pretty(v).map_err(|e| e.to_string())?;
    raw.push(b'\n');
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&tmp, &raw).map_err(|e| format!("{}: {e}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

pub struct App {
    pub data_dir: PathBuf,
    pub version: String,
    pub keep_alive: Duration,
    reload_debounce: Duration,
    pub store: Arc<Store>,
    secret: Vec<u8>,
    pub registry: Arc<Registry>,
    pub libraries: Libraries,
    pub roots: Arc<RwLock<Roots>>,
    pub unlocks: Unlocks,
    pub modules: RwLock<ModuleSet>,
    pub events: EventBus,
    /// Library scans (`/api/library/scan`) and the watch loop.
    pub scanner: Arc<Scanner>,
    watch: bool,
    watch_provider: Arc<SourceWatch>,
    provisioner: Mutex<Option<Provisioner>>,
    watcher: Mutex<Option<notify::RecommendedWatcher>>,
    stopping: Arc<AtomicBool>,
}

impl App {
    /// Opens the data dir and builds the server state.
    pub fn open(cfg: Config) -> Result<Arc<App>, String> {
        let dir = &cfg.data_dir;
        let store = Arc::new(Store::open(dir).map_err(|e| e.to_string())?);
        let secret = lain_auth::load_or_create_secret(&store).map_err(|e| e.to_string())?;

        let comp_path = dir.join("composition.json");
        let mut comp = default_composition();
        if comp_path.exists() {
            let raw = std::fs::read(&comp_path).map_err(|e| format!("{}: {e}", comp_path.display()))?;
            let mut saved: Composition =
                serde_json::from_slice(&raw).map_err(|e| format!("corrupt document composition.json: {e}"))?;
            if !saved.bindings.is_empty() {
                let added = saved.upgrade(&default_composition());
                if !added.is_empty() {
                    tracing::info!(added = %added.join(","), "new capabilities from defaults");
                }
                comp = saved;
            }
        }
        let roots = Arc::new(RwLock::new(Roots::load(dir)?));
        let registry = Arc::new(Registry::new(comp.clone()));
        registry.register(Arc::new(Catalog::with_roots(store.clone(), roots.clone())));
        registry.register(Arc::new(Userstate::new(store.clone())));
        for id in lain_identify::Identifier::ALL {
            registry.register(Arc::new(id));
        }
        registry.register(Arc::new(lain_identify::Tokenizer));
        registry.register(Arc::new(lain_ingest::Filesystem));
        let watch_provider = Arc::new(SourceWatch::new(cfg.watch_debounce, true));
        registry.register(watch_provider.clone());
        let libs_store = (store.clone(), roots.clone());
        let library_source: lain_ingest::scanner::LibrarySource =
            Arc::new(move || Libraries::new(libs_store.0.clone(), libs_store.1.clone()).list().unwrap_or_default());
        let scanner = Scanner::new(&registry, library_source, cfg.auto_enrich);
        let weak_scanner = Arc::downgrade(&scanner);
        let progress: lain_ingest::scan::Progress = Arc::new(move |done: &lain_ingest::LibraryDone| {
            if let Some(s) = weak_scanner.upgrade() {
                s.library_done(done);
            }
        });
        registry.register(Arc::new(lain_ingest::Runner::new(&registry, Some(progress))));
        registry.register(Arc::new(lain_metadata::Nfo));
        registry.register(Arc::new(lain_metadata::Enricher::new(&registry, store.clone())));

        let mut known: BTreeSet<String> = registry.providers().into_iter().collect();
        known.extend(COMPONENT_SERVED.iter().map(|s| s.to_string()));
        known.extend(manifest_ids(&dir.join("plugins")));
        comp.validate(&known).map_err(|e| format!("composition: {e}"))?;
        save_doc(&comp_path, &registry.composition())?;

        let app = Arc::new(App {
            data_dir: dir.clone(),
            version: cfg.version,
            keep_alive: cfg.keep_alive,
            reload_debounce: cfg.reload_debounce,
            libraries: Libraries::new(store.clone(), roots.clone()),
            roots,
            unlocks: Unlocks::default(),
            store,
            secret,
            registry,
            modules: RwLock::new(ModuleSet::new(modules::builtin())),
            events: EventBus::new(),
            scanner,
            watch: cfg.watch,
            watch_provider,
            provisioner: Mutex::new(None),
            watcher: Mutex::new(None),
            stopping: Arc::new(AtomicBool::new(false)),
        });
        // A corrupt modules.json fails the boot (never silently
        // ignored); later edits that do not parse keep the last state.
        let doc = app.read_modules_doc()?;
        app.apply_modules(doc.as_ref());
        let weak = Arc::downgrade(&app);
        app.scanner.set_sink(Arc::new(move |ev| {
            if let Some(app) = weak.upgrade() {
                app.publish_scan(ev);
            }
        }));
        Ok(app)
    }

    pub fn auth(&self) -> Auth<'_> {
        Auth::new(&self.store, self.secret.clone())
    }

    /// Verifies a token against live state.
    pub fn verify(&self, token: &str) -> Option<Verified> {
        self.auth().verify(token).ok()
    }

    // ---- private libraries (D-156) ----------------------------------

    /// Libraries whose items this session may not see.
    pub fn hidden_libraries(&self, v: &Verified) -> Vec<String> {
        self.libraries
            .list()
            .unwrap_or_default()
            .into_iter()
            .filter(|l| !privacy::items_visible(l, &self.unlocks, &v.user_id, &v.session))
            .map(|l| l.id)
            .collect()
    }

    /// A catalog item, if it exists and this session may see it.
    pub fn visible_item(&self, v: &Verified, item_id: &str) -> Option<CatalogItem> {
        let (out, _) = self.registry.call_one(CAP_CATALOG_READ, "get", json!({"id": item_id})).ok()?;
        let it: CatalogItem = serde_json::from_value(out).ok()?;
        match self.libraries.get(&it.library_id) {
            Some(lib) if !privacy::items_visible(&lib, &self.unlocks, &v.user_id, &v.session) => None,
            _ => Some(it),
        }
    }

    /// Whether the catalog knows an item (any library).
    pub fn item_exists(&self, item_id: &str) -> bool {
        self.registry.call_one(CAP_CATALOG_READ, "get", json!({"id": item_id})).is_ok()
    }

    /// The private library an item belongs to, if any (event filters).
    fn private_library_of(&self, item_id: &str) -> Option<Library> {
        let (out, _) = self.registry.call_one(CAP_CATALOG_READ, "get", json!({"id": item_id})).ok()?;
        let lib_id = out["library_id"].as_str()?.to_string();
        self.libraries.get(&lib_id).filter(|l| l.private)
    }

    /// `library-changed`, delivered only to who may see the record.
    pub fn publish_library(&self, lib: &Library, data: Value) {
        let lib = lib.clone();
        self.events
            .publish_where("library-changed", data, &|s: &SubInfo| privacy::record_visible(&lib, &s.user_id, s.admin));
    }

    /// `progress-changed` for the user's own streams; for an item of a
    /// private library only the sessions that unlocked it.
    pub fn publish_progress(&self, user_id: &str, item_id: &str, data: Value) {
        let lib = self.private_library_of(item_id);
        self.events.publish_where("progress-changed", data, &|s: &SubInfo| {
            s.user_id == user_id
                && lib.as_ref().is_none_or(|l| privacy::items_visible(l, &self.unlocks, &s.user_id, &s.session))
        });
    }

    /// Scan progress as `/api/events`: `scan-state` for everyone (the
    /// unreadable roots of private libraries only to admins), and
    /// `library-changed` `scanned` to who may see the library.
    fn publish_scan(&self, ev: ScanEvent) {
        let (state, trigger, libraries, libraries_done, status) = match ev {
            ScanEvent::Started { trigger, libraries } => ("running".to_string(), trigger, libraries, 0, None),
            ScanEvent::Library { done, libraries, libraries_done, trigger } => {
                if let Some(lib) = self.libraries.get(&done.library_id) {
                    let c = &done.commit;
                    self.publish_library(
                        &lib,
                        json!({"library_id": lib.id, "action": "scanned", "items": done.identified,
                               "missing": c.missing, "restored": c.restored, "migrated": c.migrated}),
                    );
                }
                ("running".to_string(), trigger, libraries, libraries_done, None)
            }
            ScanEvent::Finished(st) => (st.state.clone(), st.trigger.clone(), 0, 0, Some(st)),
        };
        let mut data =
            json!({"state": state, "trigger": trigger, "libraries": libraries, "libraries_done": libraries_done});
        if let Some(st) = &status {
            let n = st.stats.as_ref().map(|s| s.libraries).unwrap_or(0);
            data["libraries"] = json!(n);
            data["libraries_done"] = json!(n);
            if !st.error.is_empty() {
                data["error"] = json!(st.error);
            }
        }
        let full = status.as_ref().and_then(|s| s.stats.clone());
        let mut admin = data.clone();
        if let Some(stats) = &full {
            admin["stats"] = json!(stats);
            data["stats"] = json!(self.public_stats(stats.clone()));
        }
        self.events.publish(Scope::Admins, "scan-state", admin);
        self.events.publish_where("scan-state", data, &|s: &SubInfo| !s.admin);
    }

    /// Scan stats as a non-admin sees them: unreadable roots of private
    /// libraries are left out (their names and paths are not theirs).
    pub fn public_stats(&self, mut stats: lain_api::ScanStats) -> lain_api::ScanStats {
        stats.unreadable.retain(|u| self.libraries.get(&u.library_id).is_none_or(|l| !l.private));
        stats
    }

    /// Re-reads `roots.json`; a bad edit is logged and ignored.
    pub fn reload_roots(&self) {
        match Roots::load(&self.data_dir) {
            Ok(r) => {
                *self.roots.write().unwrap_or_else(|e| e.into_inner()) = r;
                tracing::info!("roots.json applied");
            }
            Err(e) => tracing::error!(err = %e, "roots.json not applied; keeping the current roots"),
        }
    }

    pub fn modules_path(&self) -> PathBuf {
        self.data_dir.join("modules.json")
    }

    fn read_modules_doc(&self) -> Result<Option<modules::ModulesDoc>, String> {
        match std::fs::read(self.modules_path()) {
            Ok(raw) => modules::parse_doc(&raw).map(Some),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("modules.json: {e}")),
        }
    }

    /// Applies a modules document: unbinds disabled modules'
    /// capabilities, rebinds enabled ones and announces each change.
    pub fn apply_modules(&self, doc: Option<&modules::ModulesDoc>) {
        let mut set = self.modules.write().unwrap_or_else(|e| e.into_inner());
        let caps_before: BTreeMap<String, Vec<String>> =
            set.defs().iter().map(|d| (d.id.clone(), d.capabilities.clone())).collect();
        let (changes, warnings) = set.apply(doc);
        for w in warnings {
            tracing::warn!(warning = %w, "modules.json");
        }
        for c in &changes {
            match (c.enabled, set.get(&c.id).is_some()) {
                (true, _) => {
                    let caps = set.get(&c.id).map(|d| d.capabilities.clone()).unwrap_or_default();
                    self.registry.resume(&c.id, &caps);
                }
                (false, true) => {
                    let caps = set.exclusive_capabilities(&c.id);
                    self.registry.suspend(&c.id, &caps);
                }
                (false, false) => {
                    // A declared module vanished: release what it held.
                    let caps = caps_before.get(&c.id).cloned().unwrap_or_default();
                    self.registry.resume(&c.id, &caps);
                }
            }
            tracing::info!(module = %c.id, enabled = c.enabled, "module state");
        }
        drop(set);
        for c in changes {
            let state = self.module_state(&c.id);
            self.events.publish(
                Scope::All,
                "module-state",
                json!({"module": c.id, "enabled": c.enabled, "state": state}),
            );
        }
    }

    /// Re-reads `modules.json`; a bad edit is logged and ignored.
    pub fn reload_modules(&self) {
        match self.read_modules_doc() {
            Ok(doc) => self.apply_modules(doc.as_ref()),
            Err(e) => tracing::error!(err = %e, "modules.json not applied; keeping the current module state"),
        }
    }

    fn capability_view(&self, comp: &Composition, cap: &str, probe: bool) -> (Value, bool, bool) {
        let bound = comp.bindings.get(cap);
        let healthy = probe && bound.is_some() && self.registry.capability_healthy(cap);
        let providers = bound.map(|b| b.providers.clone()).unwrap_or_default();
        (
            json!({"capability": cap, "bound": bound.is_some(), "healthy": healthy, "providers": providers}),
            bound.is_some(),
            healthy,
        )
    }

    /// `disabled`, `unavailable` (nothing of it is served by this
    /// server), `degraded` (a served capability has no healthy
    /// provider) or `active`.
    pub fn module_state(&self, id: &str) -> String {
        self.module_view(id).and_then(|v| v["state"].as_str().map(str::to_string)).unwrap_or_else(|| "disabled".into())
    }

    pub fn module_view(&self, id: &str) -> Option<Value> {
        let set = self.modules.read().unwrap_or_else(|e| e.into_inner());
        let d = set.get(id)?.clone();
        let enabled = set.is_enabled(id);
        drop(set);
        let comp = self.registry.composition();
        let mut caps = Vec::new();
        let (mut any_bound, mut all_healthy) = (false, true);
        for c in &d.capabilities {
            let (v, bound, healthy) = self.capability_view(&comp, c, enabled);
            any_bound |= bound;
            if bound && !healthy {
                all_healthy = false;
            }
            caps.push(v);
        }
        let state = if !enabled {
            "disabled"
        } else if !any_bound && !d.required {
            "unavailable"
        } else if !all_healthy {
            "degraded"
        } else {
            "active"
        };
        Some(json!({
            "id": d.id,
            "name": d.name,
            "required": d.required,
            "enabled": enabled,
            "state": state,
            "capabilities": caps,
            "routes": d.routes,
            "library_kinds": d.library_kinds,
            "ui": d.ui,
            "components": d.components,
        }))
    }

    /// The `GET /api/modules` document.
    pub fn modules_view(&self) -> Value {
        let ids: Vec<String> =
            self.modules.read().unwrap_or_else(|e| e.into_inner()).defs().iter().map(|d| d.id.clone()).collect();
        let list: Vec<Value> = ids.iter().filter_map(|id| self.module_view(id)).collect();
        json!({"format": modules::FORMAT, "modules": list})
    }

    /// The disabled module owning a route, if any.
    pub fn disabled_owner(&self, path: &str) -> Option<String> {
        let set = self.modules.read().unwrap_or_else(|e| e.into_inner());
        let owner = set.route_owner(path)?;
        (!set.is_enabled(&owner.id)).then(|| owner.id.clone())
    }

    /// Whether some module owns a route (enabled or not).
    pub fn route_owned(&self, path: &str) -> bool {
        self.modules.read().unwrap_or_else(|e| e.into_inner()).route_owner(path).is_some()
    }

    pub fn save_composition(&self) -> Result<(), String> {
        save_doc(&self.data_dir.join("composition.json"), &self.registry.composition())
    }

    /// Starts the component provisioner and the `modules.json` watch.
    pub fn start_background(self: &Arc<Self>) {
        let prov = Provisioner::new(&self.data_dir.join("plugins"), self.registry.clone());
        let weak: Weak<App> = Arc::downgrade(self);
        prov.on_change(move |c| {
            if let Some(app) = weak.upgrade() {
                let (provider, action) = match c {
                    PluginChange::Installed(id) => (id.clone(), "installed"),
                    PluginChange::Removed(id) => (id.clone(), "removed"),
                };
                app.events.publish(Scope::Admins, "plugin-changed", json!({"provider": provider, "action": action}));
            }
        });
        if let Err(e) = prov.start() {
            // Never a failed boot: the operator learns why live installs
            // do not land.
            tracing::warn!(err = %e, "component provisioning disabled");
        }
        *self.provisioner.lock().unwrap_or_else(|e| e.into_inner()) = Some(prov);
        self.watch_modules();
        if self.watch {
            self.scanner.start_watching();
        }
    }

    fn watch_modules(self: &Arc<Self>) {
        // One watch on the data dir for both documents; a pass reloads
        // whichever changed.
        let (tx, rx) = std::sync::mpsc::channel::<&'static str>();
        let w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                for p in &ev.paths {
                    match p.file_name().and_then(|n| n.to_str()) {
                        Some("modules.json") => {
                            let _ = tx.send("modules");
                        }
                        Some(roots::FILE_NAME) => {
                            let _ = tx.send("roots");
                        }
                        _ => {}
                    }
                }
            }
        });
        let mut w = match w {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(err = %e, "modules.json watch unavailable");
                return;
            }
        };
        if let Err(e) = w.watch(&self.data_dir, RecursiveMode::NonRecursive) {
            tracing::warn!(err = %e, "modules.json watch unavailable");
            return;
        }
        *self.watcher.lock().unwrap_or_else(|e| e.into_inner()) = Some(w);
        let weak = Arc::downgrade(self);
        let stopping = self.stopping.clone();
        let debounce = self.reload_debounce;
        std::thread::Builder::new()
            .name("modules-watch".into())
            .spawn(move || {
                loop {
                    match rx.recv_timeout(Duration::from_millis(250)) {
                        Ok(first) => {
                            let mut which = BTreeSet::from([first]);
                            while let Ok(more) = rx.recv_timeout(debounce) {
                                which.insert(more);
                            }
                            match weak.upgrade() {
                                Some(app) if !stopping.load(Ordering::SeqCst) => {
                                    if which.contains("roots") {
                                        app.reload_roots();
                                    }
                                    if which.contains("modules") {
                                        app.reload_modules();
                                    }
                                }
                                _ => return,
                            }
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                            if stopping.load(Ordering::SeqCst) || weak.strong_count() == 0 {
                                return;
                            }
                        }
                        Err(_) => return,
                    }
                }
            })
            .ok();
    }

    /// Ends streams, stops watches and components.
    pub fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.scanner.stop();
        self.watch_provider.close();
        self.events.close_all();
        self.watcher.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(p) = self.provisioner.lock().unwrap_or_else(|e| e.into_inner()).take() {
            p.close();
        }
    }
}

/// Provider ids declared by manifests in a plugins dir.
fn manifest_ids(dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    rd.flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read(e.path()).ok())
        .filter_map(|raw| serde_json::from_slice::<lain_component::Manifest>(&raw).ok())
        .map(|m| m.id)
        .filter(|id| !id.is_empty())
        .collect()
}
