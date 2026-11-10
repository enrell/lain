//! The command-line client's state and its account commands: `login`,
//! `logout`, `player` and `language` (D-237).
//!
//! The CLI is a `lain-client` app like the desktop one. It shares that
//! crate's per-user directory ([`lain_client::client_dir`]): sessions in
//! `sessions.json` (one per server), the client database `client.redb`
//! (offline copies and the progress queue) and the copies under
//! `offline/`. Its own choices live in `cli.json` next to them: the
//! current server, the default player and the offline-store limits.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use lain_client::{Client, ClientConfig, ClientDb, FileTokenStore, OfflineConfig, OfflineStore, ProgressSync};

use crate::args::{flag, positional};

pub const DEFAULT_SERVER: &str = "http://127.0.0.1:9360";
pub const FORMAT: &str = "lain.cli@1";
/// D-094's default number of retries of a dropped transfer.
pub const DEFAULT_RETRIES: u32 = 5;
pub const MAX_RETRIES: u32 = 100;

/// The offline store's policy as `cli.json` keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfflineSettings {
    #[serde(default)]
    pub dir: String,
    pub max_bytes: u64,
    pub min_free_bytes: u64,
    pub evict_watched: bool,
    pub retries: u32,
}

impl Default for OfflineSettings {
    fn default() -> Self {
        let d = OfflineConfig::new("");
        OfflineSettings {
            dir: String::new(),
            max_bytes: d.max_bytes,
            min_free_bytes: d.min_free_bytes,
            evict_watched: d.evict_watched,
            retries: DEFAULT_RETRIES,
        }
    }
}

/// `cli.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliConfig {
    pub format: String,
    /// The canonical URL of the server `lain login` last signed in to.
    #[serde(default)]
    pub server: String,
    /// `mpv` or `vlc`; empty means mpv.
    #[serde(default)]
    pub player: String,
    #[serde(default)]
    pub offline: OfflineSettings,
}

impl Default for CliConfig {
    fn default() -> Self {
        CliConfig {
            format: FORMAT.into(),
            server: String::new(),
            player: String::new(),
            offline: OfflineSettings::default(),
        }
    }
}

impl CliConfig {
    pub fn player(&self) -> &str {
        if self.player.is_empty() { "mpv" } else { &self.player }
    }
}

/// Where the client keeps its files.
pub struct Home {
    pub dir: PathBuf,
}

impl Home {
    pub fn open() -> Result<Home, String> {
        let dir = lain_client::client_dir().ok_or("no user data directory (set LAIN_CLIENT_HOME)")?;
        Ok(Home { dir })
    }

    fn config_path(&self) -> PathBuf {
        self.dir.join("cli.json")
    }

    pub fn load(&self) -> Result<CliConfig, String> {
        let p = self.config_path();
        let raw = match std::fs::read(&p) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(CliConfig::default()),
            Err(e) => return Err(format!("{}: {e}", p.display())),
        };
        let c: CliConfig = serde_json::from_slice(&raw).map_err(|e| format!("{}: {e}", p.display()))?;
        if c.format != FORMAT {
            return Err(format!("{}: format {:?} is not {FORMAT}", p.display(), c.format));
        }
        Ok(c)
    }

    pub fn save(&self, c: &CliConfig) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        lain_server::app::save_doc(&self.config_path(), c)
    }

    pub fn tokens(&self) -> Arc<FileTokenStore> {
        Arc::new(FileTokenStore::new(self.dir.join("sessions.json")))
    }

    /// The client database; it is locked while another lain client
    /// (another CLI run, the desktop app) has it open.
    pub fn db(&self) -> Result<Arc<ClientDb>, String> {
        ClientDb::open(&self.dir.join("client.redb"))
            .map(Arc::new)
            .map_err(|e| format!("{e} (is another lain client running?)"))
    }

    pub fn offline_config(&self, c: &CliConfig) -> OfflineConfig {
        let dir = if c.offline.dir.is_empty() {
            self.dir.join("offline").to_string_lossy().into_owned()
        } else {
            c.offline.dir.clone()
        };
        OfflineConfig {
            dir,
            max_bytes: c.offline.max_bytes,
            min_free_bytes: c.offline.min_free_bytes,
            evict_watched: c.offline.evict_watched,
        }
    }

    pub fn offline(&self, c: &CliConfig, db: &Arc<ClientDb>) -> Result<OfflineStore, String> {
        OfflineStore::open(self.offline_config(c), db.clone()).map_err(|e| e.to_string())
    }

    /// A client of `server` with whatever session is saved for it.
    pub fn client(&self, server: &str) -> Result<Client, String> {
        Client::new(ClientConfig::new(server), self.tokens()).map_err(|e| e.to_string())
    }
}

/// The signed-in client: `--server` when given, else the last login.
pub struct Session {
    pub home: Home,
    pub cfg: CliConfig,
    pub client: Client,
}

impl Session {
    pub fn open(args: &[String]) -> Result<Session, String> {
        let home = Home::open()?;
        let cfg = home.load()?;
        let server = flag(args, "server").unwrap_or_else(|| cfg.server.clone());
        if server.is_empty() {
            return Err("not logged in (run `lain login`)".into());
        }
        let client = home.client(&server)?;
        if client.session().is_none() {
            return Err(format!("not logged in to {} (run `lain login --server {server}`)", client.server()));
        }
        Ok(Session { home, cfg, client })
    }

    pub fn sync(&self, db: &Arc<ClientDb>) -> ProgressSync {
        ProgressSync::new(self.client.clone(), db.clone())
    }
}

pub fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().map_err(|e| e.to_string())
}

fn read_line(prompt: &str) -> Result<String, String> {
    use std::io::Write;
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).map_err(|e| e.to_string())?;
    Ok(line.trim().to_string())
}

pub fn login(args: &[String]) -> Result<(), String> {
    let home = Home::open()?;
    let mut cfg = home.load()?;
    let server = flag(args, "server").unwrap_or_else(|| DEFAULT_SERVER.into());
    let username = match flag(args, "username") {
        Some(u) => u,
        None => read_line("username: ")?,
    };
    // The password never goes in argv: every local user can read it
    // through /proc.
    let password = match std::env::var("LAIN_PASSWORD") {
        Ok(p) if !p.is_empty() => p,
        _ => crate::prompt::password("password: ")?,
    };
    let client = home.client(&server)?;
    let me = runtime()?.block_on(client.login(&username, &password)).map_err(|e| e.to_string())?;
    cfg.server = client.server();
    home.save(&cfg)?;
    println!("logged in as {} on {}", me.username, client.server());
    Ok(())
}

pub fn logout(args: &[String]) -> Result<(), String> {
    let home = Home::open()?;
    let mut cfg = home.load()?;
    let server = flag(args, "server").unwrap_or_else(|| cfg.server.clone());
    if !server.is_empty() {
        let client = home.client(&server)?;
        client.logout().map_err(|e| e.to_string())?;
        if client.server() == cfg.server {
            cfg.server.clear();
            home.save(&cfg)?;
        }
    }
    println!("logged out");
    Ok(())
}

pub fn player(args: &[String]) -> Result<(), String> {
    let home = Home::open()?;
    let mut cfg = home.load()?;
    let pos = positional(args);
    match pos.as_slice() {
        [] => {
            println!("{}", cfg.player());
            Ok(())
        }
        [p] if p == "mpv" || p == "vlc" => {
            cfg.player = p.clone();
            home.save(&cfg)?;
            println!("default player: {p}");
            Ok(())
        }
        _ => Err("usage: lain player [mpv|vlc]".into()),
    }
}

/// A three-letter ISO 639-2 code, lowercased; `--clear` is `""`.
pub fn parse_language(raw: &str) -> Result<String, String> {
    let l = raw.trim().to_ascii_lowercase();
    if l == "--clear" {
        return Ok(String::new());
    }
    if l.len() != 3 || !l.bytes().all(|b| b.is_ascii_lowercase()) {
        return Err("language must be a three-letter ISO 639-2 code".into());
    }
    Ok(l)
}

pub fn language(args: &[String]) -> Result<(), String> {
    let set: Vec<&String> = args.iter().filter(|a| *a == "--clear" || !a.starts_with("--")).collect();
    if set.len() > 1 {
        return Err("usage: lain language [three-letter-code|--clear]".into());
    }
    let s = Session::open(&[])?;
    let rt = runtime()?;
    let user = match set.first() {
        None => {
            let me = rt.block_on(s.client.me()).map_err(|e| e.to_string())?;
            println!("{}", if me.preferred_language.is_empty() { "player default" } else { &me.preferred_language });
            return Ok(());
        }
        Some(raw) => {
            let l = parse_language(raw)?;
            rt.block_on(s.client.set_preferred_language(&l)).map_err(|e| e.to_string())?
        }
    };
    if user.preferred_language.is_empty() {
        println!("preferred language cleared");
    } else {
        println!("preferred language: {}", user.preferred_language);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages() {
        assert_eq!(parse_language("JPN").unwrap(), "jpn");
        assert_eq!(parse_language("--clear").unwrap(), "");
        assert!(parse_language("ja").is_err());
        assert!(parse_language("j1n").is_err());
    }

    #[test]
    fn config_round_trips_with_defaults() {
        let c: CliConfig = serde_json::from_str(r#"{"format":"lain.cli@1","server":"http://h:9360"}"#).unwrap();
        assert_eq!(c.player(), "mpv");
        assert_eq!(c.offline, OfflineSettings::default());
        assert_eq!(c.offline.max_bytes, 10 << 30);
        assert_eq!(c.offline.retries, DEFAULT_RETRIES);
    }
}
