//! Finding a local app server the way its own clients do: a discovery
//! file (`server.json`: `{service, version, api, url, pid,
//! started_at_ms}`) and a token file in the app's home, both written by
//! the app. The file is re-read on every check, so a restarted app (on
//! a new port, or with a rotated token) is picked up without restarting
//! the component.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use lain_core::{Error, codes};

use crate::http1::{self, Endpoint, Request, Response};

/// An app server: where its files are, or a fixed URL and token.
#[derive(Debug, Clone)]
pub struct Locator {
    /// `service` the health answer and the discovery file must name.
    pub service: &'static str,
    pub health_path: &'static str,
    /// The app's home (holds `server.json` and the token file).
    pub home: Option<PathBuf>,
    pub token_file: &'static str,
    /// A fixed base URL instead of the discovery file.
    pub url: Option<String>,
    /// A fixed token instead of the token file (from the app's own
    /// environment variable, never from a manifest).
    pub token: Option<String>,
}

#[derive(Deserialize)]
struct DiscoveryFile {
    #[serde(default)]
    service: String,
    #[serde(default)]
    url: String,
}

/// A located, live server.
#[derive(Debug, Clone)]
pub struct Server {
    pub endpoint: Endpoint,
    pub token: String,
}

fn unavailable(msg: impl Into<String>) -> Error {
    Error::unavailable(msg)
}

impl Locator {
    fn read_url(&self) -> Result<String, Error> {
        if let Some(u) = &self.url {
            return Ok(u.clone());
        }
        let home = self.home.as_ref().ok_or_else(|| unavailable(format!("{}: no home directory", self.service)))?;
        let raw = std::fs::read(home.join("server.json"))
            .map_err(|_| unavailable(format!("{} is not running (no server.json)", self.service)))?;
        let d: DiscoveryFile = serde_json::from_slice(&raw)
            .map_err(|_| unavailable(format!("{}: unreadable server.json", self.service)))?;
        if d.service != self.service || d.url.is_empty() {
            return Err(unavailable(format!("{}: server.json names another service", self.service)));
        }
        Ok(d.url)
    }

    fn read_token(&self) -> Result<String, Error> {
        if let Some(t) = &self.token {
            return Ok(t.clone());
        }
        let home = self.home.as_ref().ok_or_else(|| unavailable(format!("{}: no home directory", self.service)))?;
        let t = std::fs::read_to_string(home.join(self.token_file))
            .map_err(|_| unavailable(format!("{}: token file unreadable", self.service)))?;
        let t = t.trim().to_string();
        if t.is_empty() {
            return Err(unavailable(format!("{}: empty token file", self.service)));
        }
        Ok(t)
    }

    /// Reads the files and checks the health answer names the service.
    pub fn locate(&self, timeout: Duration) -> Result<Server, Error> {
        let url = self.read_url()?;
        let endpoint = Endpoint::parse(&url).map_err(|e| unavailable(format!("{}: {e}", self.service)))?;
        let token = self.read_token()?;
        let req = Request { timeout, max_body: 64 << 10, ..Request::new("GET", self.health_path) };
        let res = http1::send(&endpoint, &req)
            .map_err(|_| unavailable(format!("{} not answering at {}", self.service, endpoint.describe())))?;
        let ok = res.status == 200 && res.json().is_some_and(|v| v["service"] == self.service);
        if !ok {
            return Err(unavailable(format!("{} unhealthy at {}", self.service, endpoint.describe())));
        }
        Ok(Server { endpoint, token })
    }
}

impl Server {
    /// One authenticated JSON call.
    pub fn call(&self, method: &str, path: &str, body: Option<&Value>, timeout: Duration) -> Result<Response, Error> {
        let req =
            Request { bearer: Some(&self.token), json: body, timeout, max_body: 4 << 20, ..Request::new(method, path) };
        http1::send(&self.endpoint, &req).map_err(|e| {
            if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut {
                Error::new(
                    codes::OUTCOME_UNKNOWN,
                    format!("{} timed out; it may or may not have run", self.endpoint.describe()),
                )
            } else {
                unavailable(format!("{} not answering", self.endpoint.describe()))
            }
        })
    }
}

/// The per-user config directory (`$XDG_CONFIG_HOME` or `~/.config`,
/// `~/Library/Application Support`, `%APPDATA%`), as the `dirs` crate
/// resolves it for the apps.
pub fn config_dir() -> Option<PathBuf> {
    let home = || std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        return std::env::var_os("APPDATA").map(PathBuf::from);
    }
    if cfg!(target_os = "macos") {
        return home().map(|h| h.join("Library/Application Support"));
    }
    match std::env::var_os("XDG_CONFIG_HOME").filter(|v| Path::new(v).is_absolute()) {
        Some(x) => Some(PathBuf::from(x)),
        None => home().map(|h| h.join(".config")),
    }
}

/// `$HOME` (`%USERPROFILE%` on Windows).
pub fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var).filter(|h| !h.is_empty()).map(PathBuf::from)
}

/// An environment variable, when set and not blank.
pub fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}
