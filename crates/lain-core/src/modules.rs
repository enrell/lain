//! The module layer (D-154, D-155): a named, user-facing bundle of
//! capabilities, routes, library kinds, UI surfaces and component
//! manifests over the composition. `modules.json` (`lain.modules@1`)
//! enables and disables modules and may declare extra component-only
//! modules. Disabling a module unbinds its capabilities
//! ([`crate::Registry::suspend`]) and unmounts its routes, which then
//! answer `module-disabled`. Data a module wrote is kept. The core
//! module cannot be disabled.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The document format this build reads and writes.
pub const FORMAT: &str = "lain.modules@1";

/// One top-level client destination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Destination {
    pub id: String,
    pub label: String,
    /// The web client's route for it (other clients map the id).
    pub path: String,
}

/// What a module shows in each client.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiSurfaces {
    #[serde(default)]
    pub destinations: Vec<Destination>,
    /// Settings section ids.
    #[serde(default)]
    pub settings: Vec<String>,
    /// Ctrl+K command ids.
    #[serde(default)]
    pub commands: Vec<String>,
}

/// A module definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModuleDef {
    pub id: String,
    pub name: String,
    /// Always on (core).
    pub required: bool,
    /// Enabled when `modules.json` does not mention it.
    #[serde(skip)]
    pub default_enabled: bool,
    /// Declared in `modules.json` rather than built in.
    #[serde(skip)]
    pub declared: bool,
    pub capabilities: Vec<String>,
    /// Gateway route patterns it owns (`{x}` is one path segment, a
    /// trailing `*` any rest). Core routes are never gated.
    pub routes: Vec<String>,
    pub library_kinds: Vec<String>,
    pub ui: UiSurfaces,
    /// Component manifest ids it installs.
    pub components: Vec<String>,
}

/// One entry of `modules.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleEntry {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Declaring fields: only for modules that are not built in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub library_kinds: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<UiSurfaces>,
}

/// `modules.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModulesDoc {
    pub format: String,
    #[serde(default)]
    pub modules: Vec<ModuleEntry>,
}

/// Parses and checks a `modules.json` document.
pub fn parse_doc(raw: &[u8]) -> Result<ModulesDoc, String> {
    let doc: ModulesDoc = serde_json::from_slice(raw).map_err(|e| format!("modules.json: {e}"))?;
    if doc.format != FORMAT {
        return Err(format!("modules.json: format must be {FORMAT}, got {:?}", doc.format));
    }
    let mut seen = std::collections::BTreeSet::new();
    for m in &doc.modules {
        if m.id.trim().is_empty() {
            return Err("modules.json: module id required".into());
        }
        if !seen.insert(m.id.as_str()) {
            return Err(format!("modules.json: duplicate module {}", m.id));
        }
    }
    Ok(doc)
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

fn dest(id: &str, label: &str, path: &str) -> Destination {
    Destination { id: id.into(), label: label.into(), path: path.into() }
}

#[allow(clippy::too_many_arguments)]
fn def(
    id: &str,
    name: &str,
    default_enabled: bool,
    caps: &[&str],
    routes: &[&str],
    kinds: &[&str],
    destinations: Vec<Destination>,
    settings: &[&str],
    commands: &[&str],
) -> ModuleDef {
    ModuleDef {
        id: id.into(),
        name: name.into(),
        required: id == "core",
        default_enabled,
        declared: false,
        capabilities: s(caps),
        routes: s(routes),
        library_kinds: s(kinds),
        ui: UiSurfaces { destinations, settings: s(settings), commands: s(commands) },
        components: Vec::new(),
    }
}

/// The built-in module set (DESIGN-v2 §5.2). Route patterns are the Go
/// gateway's routes grouped by owner, so a disabled module unmounts the
/// same surface on either server.
pub fn builtin() -> Vec<ModuleDef> {
    vec![
        def(
            "core",
            "Core",
            true,
            &[
                "lain.catalog.read@1",
                "lain.catalog.write@1",
                "lain.userstate.progress@1",
                "lain.search.query@1",
                "lain.ingest.scan@1",
                "lain.source.enumerate@1",
                "lain.source.watch@1",
                "lain.settings.transcode@1",
                "lain.ui.theme@1",
                "lain.backup.create@1",
            ],
            &[],
            &[],
            vec![dest("home", "Home", "/"), dest("search", "Search", "/search"), dest("settings", "Settings", "/settings")],
            &["account", "users", "libraries", "plugins", "modules", "backup"],
            &["search.open", "library.scan", "settings.open"],
        ),
        def(
            "watch",
            "Watch",
            true,
            &[
                "lain.playback.plan@1",
                "lain.playback.transcode@1",
                "lain.playback.transcode@2",
                "lain.playback.transcode@3",
                "lain.media.probe@1",
                "lain.transform.thumbnail@1",
                "lain.playback.local@1",
            ],
            &[
                "/api/items/{id}/playback",
                "/api/items/{id}/stream",
                "/api/items/{id}/play-local",
                "/api/items/{id}/thumbnail",
                "/api/items/{id}/transcode",
                "/api/items/{id}/transcode/*",
                "/api/localplay",
                "/api/playback/options",
                "/api/admin/transcodes",
                "/api/admin/transcodes/*",
                "/api/admin/settings/transcode",
            ],
            &["anime", "movie", "video"],
            vec![dest("library", "Library", "/library"), dest("player", "Player", "/player")],
            &["playback", "transcode"],
            &["watch.resume", "watch.next"],
        ),
        def(
            "read",
            "Read",
            true,
            &["lain.comic.pages@1"],
            &["/api/items/{id}/pages", "/api/items/{id}/pages/*"],
            &["manga", "comic"],
            vec![dest("read", "Read", "/read")],
            &["reader"],
            &["read.resume"],
        ),
        def(
            "acquire",
            "Acquire",
            true,
            &["lain.indexer@1"],
            &["/api/acquire/*", "/api/downloads", "/api/downloads/*"],
            &[],
            vec![dest("acquire", "Acquire", "/acquire")],
            &["indexers", "profiles", "downloads"],
            &["acquire.search", "acquire.wanted"],
        ),
        def(
            "subtitles",
            "Subtitles",
            true,
            &["lain.subtitle@1", "lain.subtitle.translate@1"],
            &[
                "/api/acquire/subtitles",
                "/api/acquire/subtitle-providers",
                "/api/acquire/subtitle-providers/*",
                "/api/acquire/subtitle-refusals",
                "/api/acquire/subtitle-refusals/*",
                "/api/acquire/items/{id}/subtitles",
                "/api/acquire/items/{id}/subtitles/*",
                "/api/acquire/items/{id}/sidecars/*",
                "/api/acquire/monitored/{id}/subtitles",
                "/api/items/{id}/subtitles",
                "/api/items/{id}/sidecars",
                "/api/items/{id}/sidecars/*",
            ],
            &[],
            vec![],
            &["subtitles"],
            &["subtitles.panel"],
        ),
        def(
            "identify",
            "Identify",
            true,
            &["lain.release.parse@1", "lain.media.identify@1"],
            &["/api/acquire/parse"],
            &[],
            vec![],
            &[],
            &[],
        ),
        def(
            "metadata",
            "Metadata",
            true,
            &["lain.metadata.search@1", "lain.metadata.resolve@1", "lain.metadata.enrich@1"],
            &["/api/catalog/{id}/enrich", "/api/enrichments"],
            &[],
            vec![],
            &["metadata"],
            &["metadata.enrich"],
        ),
        def(
            "lists",
            "Lists",
            true,
            &["lain.list.read@1", "lain.list.write@1", "lain.list.account@1", "lain.listlink@1", "lain.settings.integrations@1"],
            &["/api/list", "/api/list/*", "/api/me/links", "/api/me/links/*", "/api/auth/{platform}/callback", "/api/admin/settings/integrations"],
            &[],
            vec![dest("list", "Lists", "/list")],
            &["integrations", "links"],
            &["list.open"],
        ),
        def(
            "social",
            "Social",
            true,
            &["lain.social.graph@1", "lain.social.activity@1", "lain.social.reviews@1", "lain.social.collections@1"],
            &["/api/social/*"],
            &[],
            vec![dest("social", "Social", "/social")],
            &["social"],
            &["social.feed"],
        ),
        def("lab", "Lab", false, &[], &[], &[], vec![], &["lab"], &[]),
    ]
}

/// Whether `pattern` matches `path` (no query string).
pub fn route_matches(pattern: &str, path: &str) -> bool {
    let mut pat = pattern.trim_matches('/').split('/').peekable();
    let mut segs = path.trim_matches('/').split('/');
    while let Some(p) = pat.next() {
        if p == "*" && pat.peek().is_none() {
            return true;
        }
        let Some(seg) = segs.next() else { return false };
        if p.starts_with('{') && p.ends_with('}') {
            if seg.is_empty() {
                return false;
            }
            continue;
        }
        if p != seg {
            return false;
        }
    }
    segs.next().is_none()
}

fn literal_segments(pattern: &str) -> usize {
    pattern.split('/').filter(|p| !p.is_empty() && *p != "*" && !p.starts_with('{')).count()
}

/// One enable/disable transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub id: String,
    pub enabled: bool,
}

/// The live module set: definitions plus their enabled state.
#[derive(Debug, Clone)]
pub struct ModuleSet {
    defs: Vec<ModuleDef>,
    enabled: BTreeMap<String, bool>,
}

impl ModuleSet {
    pub fn new(defs: Vec<ModuleDef>) -> Self {
        let enabled = defs.iter().map(|d| (d.id.clone(), d.default_enabled || d.required)).collect();
        ModuleSet { defs, enabled }
    }

    pub fn defs(&self) -> &[ModuleDef] {
        &self.defs
    }

    pub fn get(&self, id: &str) -> Option<&ModuleDef> {
        self.defs.iter().find(|d| d.id == id)
    }

    pub fn is_enabled(&self, id: &str) -> bool {
        self.enabled.get(id).copied().unwrap_or(false)
    }

    /// Applies a document (`None` = no file: defaults). Returns the
    /// transitions to carry out on the registry and warnings to log.
    /// A declared module that disappears from the document is removed
    /// and reported as disabled.
    pub fn apply(&mut self, doc: Option<&ModulesDoc>) -> (Vec<Change>, Vec<String>) {
        let before = self.enabled.clone();
        let mut warnings = Vec::new();
        let empty = Vec::new();
        let entries = doc.map(|d| &d.modules).unwrap_or(&empty);

        // Declared modules: keep only those still in the document.
        let declared_now: Vec<&ModuleEntry> = entries
            .iter()
            .filter(|e| !self.defs.iter().any(|d| d.id == e.id && !d.declared))
            .collect();
        self.defs.retain(|d| !d.declared || declared_now.iter().any(|e| e.id == d.id));
        for e in &declared_now {
            if e.capabilities.is_empty() && e.components.is_empty() {
                warnings.push(format!("module {} is not built in and declares no capabilities or components; ignored", e.id));
                self.defs.retain(|d| d.id != e.id);
                continue;
            }
            let def = ModuleDef {
                id: e.id.clone(),
                name: e.name.clone().unwrap_or_else(|| e.id.clone()),
                required: false,
                default_enabled: true,
                declared: true,
                capabilities: e.capabilities.clone(),
                routes: Vec::new(),
                library_kinds: e.library_kinds.clone(),
                ui: e.ui.clone().unwrap_or_default(),
                components: e.components.clone(),
            };
            match self.defs.iter_mut().find(|d| d.id == e.id) {
                Some(d) => *d = def,
                None => self.defs.push(def),
            }
        }

        let mut next = BTreeMap::new();
        for d in &self.defs {
            let entry = entries.iter().find(|e| e.id == d.id);
            let mut on = entry.and_then(|e| e.enabled).unwrap_or(d.default_enabled);
            if d.required && !on {
                warnings.push(format!("module {} cannot be disabled; keeping it on", d.id));
                on = true;
            }
            if !d.declared && entry.is_some_and(|e| e.name.is_some() || !e.capabilities.is_empty()) {
                warnings.push(format!("module {} is built in; its declaration fields are ignored", d.id));
            }
            next.insert(d.id.clone(), on);
        }
        self.enabled = next;

        let mut changes = Vec::new();
        for (id, on) in &self.enabled {
            if before.get(id).copied().unwrap_or(false) != *on {
                changes.push(Change { id: id.clone(), enabled: *on });
            }
        }
        for (id, was) in &before {
            if *was && !self.enabled.contains_key(id) {
                changes.push(Change { id: id.clone(), enabled: false });
            }
        }
        (changes, warnings)
    }

    /// The enabled-or-not module that owns a route, most specific
    /// pattern first. Core routes are never owned.
    pub fn route_owner(&self, path: &str) -> Option<&ModuleDef> {
        let mut best: Option<(&ModuleDef, usize)> = None;
        for d in &self.defs {
            for p in &d.routes {
                if route_matches(p, path) {
                    let score = literal_segments(p) * 2 + usize::from(!p.ends_with('*'));
                    if best.is_none_or(|(_, b)| score > b) {
                        best = Some((d, score));
                    }
                }
            }
        }
        best.map(|(d, _)| d)
    }

    /// Capabilities a disabled module unbinds. A capability shared
    /// with another enabled module stays bound.
    pub fn exclusive_capabilities(&self, id: &str) -> Vec<String> {
        let Some(d) = self.get(id) else { return Vec::new() };
        d.capabilities
            .iter()
            .filter(|c| !self.defs.iter().any(|o| o.id != id && self.is_enabled(&o.id) && o.capabilities.contains(c)))
            .cloned()
            .collect()
    }

    /// The document that reproduces the current state.
    pub fn to_doc(&self) -> ModulesDoc {
        ModulesDoc {
            format: FORMAT.into(),
            modules: self
                .defs
                .iter()
                .map(|d| ModuleEntry {
                    id: d.id.clone(),
                    enabled: Some(self.is_enabled(&d.id)),
                    name: d.declared.then(|| d.name.clone()),
                    capabilities: if d.declared { d.capabilities.clone() } else { Vec::new() },
                    components: if d.declared { d.components.clone() } else { Vec::new() },
                    library_kinds: if d.declared { d.library_kinds.clone() } else { Vec::new() },
                    ui: if d.declared { Some(d.ui.clone()) } else { None },
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_patterns() {
        assert!(route_matches("/api/items/{id}/playback", "/api/items/abc/playback"));
        assert!(!route_matches("/api/items/{id}/playback", "/api/items//playback"));
        assert!(!route_matches("/api/items/{id}/playback", "/api/items/abc/progress"));
        assert!(route_matches("/api/social/*", "/api/social/feed"));
        assert!(route_matches("/api/social/*", "/api/social/users/u/activity"));
        assert!(!route_matches("/api/social/*", "/api/socialx"));
        assert!(route_matches("/api/downloads", "/api/downloads"));
        assert!(!route_matches("/api/downloads", "/api/downloads/x"));
    }

    #[test]
    fn route_owner_prefers_the_specific_module() {
        let set = ModuleSet::new(builtin());
        assert_eq!(set.route_owner("/api/acquire/grabs").unwrap().id, "acquire");
        assert_eq!(set.route_owner("/api/acquire/subtitles").unwrap().id, "subtitles");
        assert_eq!(set.route_owner("/api/acquire/items/x/subtitles/status").unwrap().id, "subtitles");
        assert_eq!(set.route_owner("/api/acquire/parse").unwrap().id, "identify");
        assert_eq!(set.route_owner("/api/items/x/playback").unwrap().id, "watch");
        assert_eq!(set.route_owner("/api/items/x/pages/3").unwrap().id, "read");
        assert!(set.route_owner("/api/items/x/progress").is_none());
        assert!(set.route_owner("/api/catalog").is_none());
    }

    #[test]
    fn defaults_and_core_cannot_be_disabled() {
        let mut set = ModuleSet::new(builtin());
        assert!(set.is_enabled("watch"));
        assert!(!set.is_enabled("lab"));
        let doc = parse_doc(br#"{"format":"lain.modules@1","modules":[{"id":"core","enabled":false},{"id":"watch","enabled":false},{"id":"lab","enabled":true}]}"#).unwrap();
        let (changes, warnings) = set.apply(Some(&doc));
        assert!(set.is_enabled("core"));
        assert!(!set.is_enabled("watch"));
        assert!(set.is_enabled("lab"));
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            changes,
            vec![Change { id: "lab".into(), enabled: true }, Change { id: "watch".into(), enabled: false }]
        );
        // Removing the file restores defaults.
        let (changes, _) = set.apply(None);
        assert_eq!(changes.len(), 2);
        assert!(set.is_enabled("watch"));
    }

    #[test]
    fn declared_modules_come_and_go() {
        let mut set = ModuleSet::new(builtin());
        let doc = parse_doc(br#"{"format":"lain.modules@1","modules":[{"id":"community.extras","name":"Extras","capabilities":["lain.x@1"],"components":["community.x"]}]}"#).unwrap();
        let (changes, _) = set.apply(Some(&doc));
        assert_eq!(changes, vec![Change { id: "community.extras".into(), enabled: true }]);
        assert_eq!(set.get("community.extras").unwrap().name, "Extras");
        let back = set.to_doc();
        assert_eq!(back.modules.iter().find(|m| m.id == "community.extras").unwrap().capabilities, vec!["lain.x@1".to_string()]);
        let (changes, _) = set.apply(Some(&ModulesDoc { format: FORMAT.into(), modules: vec![] }));
        assert_eq!(changes, vec![Change { id: "community.extras".into(), enabled: false }]);
        assert!(set.get("community.extras").is_none());
    }

    #[test]
    fn bad_documents_are_rejected() {
        assert!(parse_doc(br#"{"format":"lain.modules@2","modules":[]}"#).is_err());
        assert!(parse_doc(br#"{"format":"lain.modules@1","modules":[{"id":"a"},{"id":"a"}]}"#).is_err());
        assert!(parse_doc(b"not json").is_err());
    }

    #[test]
    fn shared_capabilities_stay_bound() {
        let mut defs = builtin();
        defs[2].capabilities.push("lain.playback.plan@1".into());
        let mut set = ModuleSet::new(defs);
        let doc = parse_doc(br#"{"format":"lain.modules@1","modules":[{"id":"watch","enabled":false}]}"#).unwrap();
        set.apply(Some(&doc));
        let caps = set.exclusive_capabilities("watch");
        assert!(!caps.contains(&"lain.playback.plan@1".to_string()));
        assert!(caps.contains(&"lain.media.probe@1".to_string()));
    }
}
