//! The app's own settings (`<client dir>/desktop.json`): which server,
//! which theme, and the client-side playback and offline preferences.
//! Account and server settings live on the server, not here.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// The server this app talks to (empty: ask on first run).
    pub server: String,
    /// The active theme folder id under `themes/`.
    pub theme: String,
    /// Where offline copies go (empty: `<client dir>/offline`).
    pub offline_dir: String,
    /// Offline budget in GiB (D-094 default 10).
    pub offline_max_gib: u64,
    /// Play the next episode when one ends.
    pub autoplay_next: bool,
    /// Server path prefix → local path prefix, for playing files straight
    /// from shared storage when the server cannot stream them yet.
    pub path_map: Vec<(String, String)>,
    /// Per extension: enabled, granted and reviewed scopes (`ext/mod.rs`).
    pub extensions: std::collections::BTreeMap<String, crate::ext::ExtSettings>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            server: String::new(),
            theme: "lain-dark".into(),
            offline_dir: String::new(),
            offline_max_gib: 10,
            autoplay_next: true,
            path_map: Vec::new(),
            extensions: Default::default(),
        }
    }
}

impl Config {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join("desktop.json")
    }

    pub fn load(dir: &Path) -> Config {
        std::fs::read(Self::path(dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) {
        let tmp = dir.join("desktop.json.tmp");
        let ok = std::fs::create_dir_all(dir)
            .and_then(|_| std::fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap_or_default()))
            .and_then(|_| std::fs::rename(&tmp, Self::path(dir)));
        if let Err(e) = ok {
            eprintln!("config: {e}");
        }
    }

    /// The local path of a server file through `path_map`, if it exists.
    pub fn local_file(&self, server_path: &str) -> Option<PathBuf> {
        if server_path.is_empty() {
            return None;
        }
        for (from, to) in &self.path_map {
            if let Some(rest) = server_path.strip_prefix(from.as_str()) {
                let rest = rest.trim_start_matches('/');
                let mut p = PathBuf::from(to);
                for part in rest.split('/').filter(|s| !s.is_empty()) {
                    p.push(part);
                }
                return p.is_file().then_some(p);
            }
        }
        let p = PathBuf::from(server_path);
        p.is_file().then_some(p)
    }
}
