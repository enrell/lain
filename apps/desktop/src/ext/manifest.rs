//! `extension.json` (`lain.extension@1`) and the scopes an extension can
//! ask for. Parsing never panics: every problem is a message the
//! Extensions settings show.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const FORMAT: &str = "lain.extension@1";
/// The `lain:extension` world version this host implements (WIT package).
pub const API_VERSION: (u32, u32) = (0, 1);
/// The largest component the host loads.
pub const MAX_WASM: u64 = 32 << 20;

/// One capability. The host checks it on every call; nothing is implied.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    CatalogRead,
    ProgressRead,
    EventsSubscribe,
    UiRail,
    UiPanel,
    /// A private folder, `/data` inside the extension.
    FsData,
    /// HTTP(S) GETs to this host (lower case, optional `:port`).
    Net(String),
}

impl Scope {
    pub fn parse(s: &str) -> Result<Scope, String> {
        Ok(match s {
            "catalog:read" => Scope::CatalogRead,
            "progress:read" => Scope::ProgressRead,
            "events:subscribe" => Scope::EventsSubscribe,
            "ui:rail" => Scope::UiRail,
            "ui:panel" => Scope::UiPanel,
            "fs:data" => Scope::FsData,
            _ => match s.strip_prefix("net:") {
                Some(h) if valid_host(h) => Scope::Net(h.to_ascii_lowercase()),
                Some(h) => return Err(format!("net scope {h:?}: name one host (example.org or example.org:8080)")),
                None => return Err(format!("unknown scope {s:?} (this app may be older than the extension)")),
            },
        })
    }

    pub fn id(&self) -> String {
        match self {
            Scope::CatalogRead => "catalog:read".into(),
            Scope::ProgressRead => "progress:read".into(),
            Scope::EventsSubscribe => "events:subscribe".into(),
            Scope::UiRail => "ui:rail".into(),
            Scope::UiPanel => "ui:panel".into(),
            Scope::FsData => "fs:data".into(),
            Scope::Net(h) => format!("net:{h}"),
        }
    }

    /// What granting it allows, for the permission prompt.
    pub fn describe(&self) -> String {
        match self {
            Scope::CatalogRead => "Read your libraries and every title and file your account can see".into(),
            Scope::ProgressRead => "Read your watch and reading progress".into(),
            Scope::EventsSubscribe => {
                "Be told when your progress or the library changes, and which titles you open and play".into()
            }
            Scope::UiRail => "Add rails to Home".into(),
            Scope::UiPanel => "Add panels to title pages".into(),
            Scope::FsData => "Keep files in its own private folder on this device".into(),
            Scope::Net(h) => format!("Connect to {h} over the internet (GET requests, without your lain token)"),
        }
    }

    /// Scopes that let data leave the device or persist on it.
    pub fn sensitive(&self) -> bool {
        matches!(self, Scope::Net(_) | Scope::FsData)
    }
}

/// `example.org`, `sub.example.org`, `localhost:8080`: letters, digits,
/// `-` and `.`, an optional port; no wildcards, paths or userinfo.
fn valid_host(h: &str) -> bool {
    let (name, port) = match h.rsplit_once(':') {
        Some((n, p)) => (n, Some(p)),
        None => (h, None),
    };
    let name_ok = !name.is_empty()
        && name.len() <= 253
        && !name.starts_with(['.', '-'])
        && !name.ends_with(['.', '-'])
        && !name.contains("..")
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
    let port_ok = port.is_none_or(|p| p.parse::<u16>().is_ok_and(|p| p > 0));
    name_ok && port_ok
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Manifest {
    pub format: String,
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    /// The world version it was built against, `major.minor[.patch]`.
    pub api: String,
    /// The component file, relative to the folder.
    pub wasm: String,
    pub scopes: Vec<String>,
    /// Handed to `activate` as JSON.
    pub config: serde_json::Value,
    /// Unknown keys (forward compatible).
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Manifest {
    pub fn parse(raw: &[u8]) -> Result<Manifest, String> {
        let m: Manifest = serde_json::from_slice(raw).map_err(|e| format!("extension.json: {e}"))?;
        m.validate()?;
        Ok(m)
    }

    fn validate(&self) -> Result<(), String> {
        if self.format != FORMAT {
            return Err(format!("format {:?}, expected {FORMAT}", self.format));
        }
        if !valid_id(&self.id) {
            return Err(format!("id {:?}: use lowercase letters, digits, - and _ (at most 64)", self.id));
        }
        if self.name.trim().is_empty() {
            return Err("name is empty".into());
        }
        let (major, minor) = parse_api(&self.api)?;
        if major != API_VERSION.0 || minor > API_VERSION.1 {
            return Err(format!(
                "built for API {}, this app implements {}.{}",
                self.api, API_VERSION.0, API_VERSION.1
            ));
        }
        safe_rel(&self.wasm).ok_or_else(|| format!("wasm {:?}: a file name inside the folder", self.wasm))?;
        if !self.wasm.ends_with(".wasm") {
            return Err(format!("wasm {:?}: expected a .wasm file", self.wasm));
        }
        self.requested()?;
        Ok(())
    }

    /// The requested scopes, parsed and deduplicated.
    pub fn requested(&self) -> Result<Vec<Scope>, String> {
        let mut out = Vec::new();
        for s in &self.scopes {
            let sc = Scope::parse(s)?;
            if !out.contains(&sc) {
                out.push(sc);
            }
        }
        Ok(out)
    }

    pub fn config_json(&self) -> String {
        if self.config.is_null() { "{}".into() } else { self.config.to_string() }
    }
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && !id.starts_with(['-', '_'])
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

fn parse_api(v: &str) -> Result<(u32, u32), String> {
    let mut it = v.split('.');
    let mut num = || it.next().and_then(|p| p.parse::<u32>().ok());
    match (num(), num()) {
        (Some(a), Some(b)) => Ok((a, b)),
        _ => Err(format!("api {v:?}: expected major.minor, e.g. \"0.1.0\"")),
    }
}

/// A plain relative path (no `..`, no root, no drive), as a PathBuf.
pub fn safe_rel(rel: &str) -> Option<PathBuf> {
    let p = Path::new(rel);
    if rel.is_empty() || rel.contains('\\') || p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(p.to_path_buf())
}

/// An installed extension: its folder and manifest (or why it is broken).
#[derive(Debug, Clone)]
pub struct Installed {
    pub id: String,
    pub dir: PathBuf,
    pub manifest: Result<Manifest, String>,
}

impl Installed {
    pub fn name(&self) -> String {
        self.manifest.as_ref().map(|m| m.name.clone()).unwrap_or_else(|_| self.id.clone())
    }
}

pub fn extensions_dir(client_dir: &Path) -> PathBuf {
    client_dir.join("extensions")
}

/// Every folder under `extensions/` with an `extension.json`.
pub fn list(root: &Path) -> Vec<Installed> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(root) else { return out };
    for e in rd.flatten() {
        let dir = e.path();
        let file = dir.join("extension.json");
        if !file.is_file() {
            continue;
        }
        let id = e.file_name().to_string_lossy().into_owned();
        let manifest = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display())).and_then(|raw| {
            let m = Manifest::parse(&raw)?;
            if m.id != id {
                return Err(format!("the folder is {id:?} but the manifest says {:?}", m.id));
            }
            Ok(m)
        });
        out.push(Installed { id, dir, manifest });
    }
    out.sort_by_key(|i| i.name().to_lowercase());
    out
}

/// Installs from a folder holding `extension.json`, or from a zip of one
/// (the folder itself or its contents at the root). Only the manifest,
/// the component and an optional `README.md` are copied; a previous
/// install of the same id is replaced, keeping its `data/` folder.
pub fn install(src: &Path, root: &Path) -> Result<Manifest, String> {
    let files: Vec<(String, Vec<u8>)> = if src.is_dir() {
        let raw = std::fs::read(src.join("extension.json")).map_err(|e| format!("extension.json: {e}"))?;
        let m = Manifest::parse(&raw)?;
        let wasm_path = src.join(safe_rel(&m.wasm).unwrap_or_default());
        let size = std::fs::metadata(&wasm_path).map_err(|e| format!("{}: {e}", m.wasm))?.len();
        if size > MAX_WASM {
            return Err(format!("{} is {size} bytes; the limit is {MAX_WASM}", m.wasm));
        }
        let wasm = std::fs::read(&wasm_path).map_err(|e| format!("{}: {e}", m.wasm))?;
        let mut v = vec![("extension.json".to_string(), raw), (m.wasm.clone(), wasm)];
        if let Ok(readme) = std::fs::read(src.join("README.md")) {
            v.push(("README.md".into(), readme));
        }
        v
    } else {
        let raw = std::fs::read(src).map_err(|e| format!("{}: {e}", src.display()))?;
        let all = crate::zip::read_all(&raw)?;
        let manifest_name = all
            .iter()
            .map(|(n, _)| n.as_str())
            .filter(|n| (*n == "extension.json" || n.ends_with("/extension.json")) && n.matches('/').count() <= 1)
            .min_by_key(|n| n.len())
            .ok_or("the archive has no extension.json")?
            .to_string();
        let prefix = manifest_name.trim_end_matches("extension.json").to_string();
        if !prefix.is_empty() && safe_rel(prefix.trim_end_matches('/')).is_none() {
            return Err(format!("unsafe path in archive: {manifest_name}"));
        }
        let get = |rel: &str| all.iter().find(|(n, _)| *n == format!("{prefix}{rel}")).map(|(_, b)| b.clone());
        let raw = get("extension.json").unwrap_or_default();
        let m = Manifest::parse(&raw)?;
        let wasm = get(&m.wasm).ok_or_else(|| format!("the archive has no {}", m.wasm))?;
        if wasm.len() as u64 > MAX_WASM {
            return Err(format!("{} is {} bytes; the limit is {MAX_WASM}", m.wasm, wasm.len()));
        }
        let mut v = vec![("extension.json".to_string(), raw), (m.wasm.clone(), wasm)];
        if let Some(readme) = get("README.md") {
            v.push(("README.md".into(), readme));
        }
        v
    };
    let m = Manifest::parse(&files[0].1)?;
    let target = root.join(&m.id);
    // Replace the code, keep the extension's data.
    if let Ok(rd) = std::fs::read_dir(&target) {
        for e in rd.flatten() {
            if e.file_name() != "data" && e.path().is_file() {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    for (rel, bytes) in &files {
        let out = target.join(safe_rel(rel).ok_or_else(|| format!("unsafe path {rel:?}"))?);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&out, bytes).map_err(|e| format!("{}: {e}", out.display()))?;
    }
    Ok(m)
}

/// Removes an installed extension, its data included.
pub fn uninstall(root: &Path, id: &str) -> Result<(), String> {
    if !valid_id(id) {
        return Err(format!("bad extension id {id:?}"));
    }
    let dir = root.join(id);
    std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))
}
