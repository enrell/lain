//! `lain-release-model`: the lain-parser model (`filename-parser`'s
//! `packaging/lain-parser`) behind `lain.release.parse@1` (D-225).
//!
//! lain-parser stays its own service: a Python/torch daemon installed
//! and updated from its Hugging Face release channel, under its own
//! systemd user unit, answering `GET /health` and `POST /parse` over a
//! Unix socket (`<lain data dir>/parser.sock` by default) and loopback
//! TCP (`127.0.0.1:9377`). This provider is the Rust port of Go's
//! `release.Model`: it asks the daemon and merges the model's labels
//! with the in-tree tokenizer's technical tags. It runs as a component
//! (`lain component run lain-release-model`) bound before
//! `lain-release-tokenizer`, so the binding falls through to the
//! tokenizer whenever the model declines:
//!
//! - **unhealthy** (daemon stopped, socket missing, model not loaded):
//!   `health` fails with `dependency-unavailable` and the registry skips
//!   the provider;
//! - **abstained** (the model's gated record is null, or the name is
//!   over the encoder window): the answer is the zero `Release`, which
//!   `ordered-many` reads as "not accepted";
//! - **failed at call time**: `dependency-unavailable`, and the next
//!   provider answers.

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};

use lain_api::{MAX_RELEASE_ALTERNATIVES, Release, ReleaseIdentity};
use lain_core::{Error, Provider};
use lain_identify::release::{clean_title, resolution, tokenize};
use lain_identify::{CAP_RELEASE_PARSE, MAX_NAME};

use crate::http1::{self, Endpoint, Request};

/// The provider id (Go's `release.ModelID`).
pub const MODEL_ID: &str = "lain-release-model";

/// The daemon's TCP default when no socket is configured.
pub const DEFAULT_TCP: &str = "http://127.0.0.1:9377";

/// How long one health answer is reused: the registry asks for health
/// before every call, and a parse should not cost two daemon trips.
const HEALTH_TTL: Duration = Duration::from_secs(2);

/// The most episodes kept from one record (Go's cap).
const MAX_EPISODES: usize = 2000;

/// The lain-parser bridge.
pub struct ReleaseModel {
    endpoint: Endpoint,
    timeout: Duration,
    health: Mutex<Option<(Instant, Result<(), Error>)>>,
}

impl ReleaseModel {
    pub fn new(endpoint: Endpoint) -> Self {
        ReleaseModel { endpoint, timeout: Duration::from_secs(3), health: Mutex::new(None) }
    }

    /// Where the daemon is, in Go's order: `LAIN_PARSER_URL`, then
    /// `LAIN_PARSER_SOCKET`, then `<data-dir>/parser.sock` when it
    /// exists (the installer's default), then loopback TCP.
    pub fn locate(explicit: Option<&str>, data_dir: Option<&Path>) -> Result<Endpoint, String> {
        if let Some(u) = explicit.filter(|u| !u.trim().is_empty()) {
            return Endpoint::parse(u);
        }
        if let Ok(u) = std::env::var("LAIN_PARSER_URL")
            && !u.trim().is_empty()
        {
            return Endpoint::parse(&u);
        }
        if let Ok(s) = std::env::var("LAIN_PARSER_SOCKET")
            && !s.trim().is_empty()
        {
            return Ok(Endpoint::Unix(s.into()));
        }
        if let Some(d) = data_dir {
            let sock = d.join("parser.sock");
            if cfg!(unix) && sock.exists() {
                return Ok(Endpoint::Unix(sock));
            }
        }
        Endpoint::parse(DEFAULT_TCP)
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    fn unavailable(msg: &str) -> Error {
        Error::unavailable(msg)
    }

    fn check(&self) -> Result<(), Error> {
        if let Endpoint::Unix(p) = &self.endpoint
            && !p.exists()
        {
            return Err(Self::unavailable("parser socket not found"));
        }
        let req = Request { timeout: self.timeout, max_body: 64 << 10, ..Request::new("GET", "/health") };
        let res = http1::send(&self.endpoint, &req).map_err(|_| Self::unavailable("parser not answering"))?;
        if res.status != 200 {
            return Err(Self::unavailable("parser unhealthy"));
        }
        match res.json() {
            Some(v) if v["status"] == "ok" => Ok(()),
            _ => Err(Self::unavailable("parser unhealthy")),
        }
    }
}

/// The model's record (`POST /parse` → `record`).
#[derive(Debug, Default, Deserialize)]
struct ModelRecord {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    episode_title: Option<String>,
    #[serde(default)]
    year: Option<i64>,
    #[serde(default)]
    season: Option<i64>,
    #[serde(default)]
    episodes: Option<Vec<i64>>,
    #[serde(default)]
    release_group: Option<String>,
    #[serde(default)]
    resolution: Option<String>,
}

#[derive(Deserialize)]
struct ParseAnswer {
    #[serde(default)]
    record: Option<ModelRecord>,
    /// Other identities the model considered (service 1.3.0 and later).
    #[serde(default)]
    alternatives: Vec<ModelRecord>,
}

#[derive(Deserialize)]
struct ParseInput {
    #[serde(default)]
    name: String,
    #[serde(default)]
    kind: String,
}

/// Go's `merge`: the model is trusted for what it labels (title,
/// numbers, group, resolution) and the tokenizer for the tags the model
/// does not label (source, codec, proper, version…).
fn merge(rec: ModelRecord, tok: Release) -> Release {
    let mut r = tok;
    r.parser = MODEL_ID.into();
    r.title = clean_title(rec.title.as_deref().unwrap_or(""));
    r.episode_title = clean_title(rec.episode_title.as_deref().unwrap_or(""));
    if let Some(y) = rec.year.filter(|y| *y > 0) {
        r.year = y;
    }
    if let Some(s) = rec.season {
        r.season = s;
    }
    if let Some(mut eps) = rec.episodes.filter(|e| !e.is_empty()) {
        eps.truncate(MAX_EPISODES);
        r.episodes = eps;
        r.season_pack = false;
        r.absolute = rec.season.is_none();
    }
    if let Some(g) = rec.release_group.filter(|g| !g.is_empty()) {
        r.group = clean_title(&g);
    }
    if let Some(v) = rec.resolution.and_then(|v| resolution(&v.to_lowercase())) {
        r.resolution = v.into();
    }
    r
}

/// What a model record says the release is, for telling readings apart.
fn identity(rec: &ModelRecord) -> (String, i64, Vec<i64>) {
    let mut episodes = rec.episodes.clone().unwrap_or_default();
    episodes.truncate(MAX_EPISODES);
    (clean_title(rec.title.as_deref().unwrap_or("")).to_lowercase(), rec.season.unwrap_or(0), episodes)
}

/// The model's other identities as `Release.alternatives` (D-275):
/// without `own` (the answer's reading), without repeats, at most
/// `MAX_RELEASE_ALTERNATIVES`. They carry the model's numbers only, never
/// the tokenizer's: a reading without an episode is a reading. A year
/// the model did not read is the tokenizer's, unless the alternative's
/// title holds it.
fn alternatives(others: Vec<ModelRecord>, own: (String, i64, Vec<i64>), tok_year: i64) -> Vec<ReleaseIdentity> {
    let mut seen = vec![own];
    let mut out = Vec::new();
    for rec in others {
        if out.len() == MAX_RELEASE_ALTERNATIVES {
            break;
        }
        let k = identity(&rec);
        if k.0.is_empty() || seen.contains(&k) {
            continue;
        }
        let title = clean_title(rec.title.as_deref().unwrap_or(""));
        let (season, episodes) = (k.1, k.2.clone());
        seen.push(k);
        let year = match rec.year.filter(|y| *y > 0) {
            Some(y) => y,
            None if tok_year > 0 && !title.contains(&tok_year.to_string()) => tok_year,
            None => 0,
        };
        let absolute = rec.season.is_none() && !episodes.is_empty();
        out.push(ReleaseIdentity { title, year, season, episodes, absolute });
    }
    out
}

impl Provider for ReleaseModel {
    fn id(&self) -> &str {
        MODEL_ID
    }

    fn capabilities(&self) -> Vec<String> {
        vec![CAP_RELEASE_PARSE.into()]
    }

    fn health(&self) -> Result<(), Error> {
        let mut g = self.health.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, r)) = g.as_ref()
            && at.elapsed() < HEALTH_TTL
        {
            return r.clone();
        }
        let r = self.check();
        *g = Some((Instant::now(), r.clone()));
        r
    }

    fn invoke(&self, cap: &str, op: &str, input: Value) -> Result<Value, Error> {
        if cap != CAP_RELEASE_PARSE {
            return Err(Error::invalid(format!("unsupported cap {cap}")));
        }
        if !op.is_empty() && op != "parse" {
            return Err(Error::invalid(format!("capability {cap} has no op {op}")));
        }
        let i: ParseInput =
            serde_json::from_value(input).map_err(|e| Error::invalid(format!("ReleaseParseInput required: {e}")))?;
        if i.name.trim().is_empty() || i.name.len() > MAX_NAME {
            return Err(Error::invalid("name must be non-empty text up to 1024 bytes"));
        }
        if let Endpoint::Unix(p) = &self.endpoint
            && !p.exists()
        {
            return Err(Self::unavailable("parser socket not found"));
        }
        let body = json!({"filename": i.name, "alternatives": MAX_RELEASE_ALTERNATIVES});
        let req =
            Request { json: Some(&body), timeout: self.timeout, max_body: 256 << 10, ..Request::new("POST", "/parse") };
        let res = http1::send(&self.endpoint, &req).map_err(|_| Self::unavailable("parser not answering"))?;
        if res.status != 200 {
            return Err(Self::unavailable("parser error"));
        }
        let ans: ParseAnswer =
            serde_json::from_slice(&res.body).map_err(|_| Self::unavailable("parser sent bad json"))?;
        match ans.record {
            Some(rec) if rec.title.as_deref().is_some_and(|t| !t.trim().is_empty()) => {
                let tok = tokenize(&i.name, &i.kind);
                let (own, tok_year) = (identity(&rec), tok.year);
                let mut r = merge(rec, tok);
                r.alternatives = alternatives(ans.alternatives, own, tok_year);
                Ok(json!(r))
            }
            // Abstained: the zero Release is a decline.
            _ => Ok(json!(Release::default())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_trusts_the_model_for_its_fields() {
        let tok = tokenize("[Fansub-A] Show - 05 [1080p][HEVC].mkv", "anime");
        let rec = ModelRecord {
            title: Some("  Show  Name ".into()),
            episodes: Some(vec![5]),
            release_group: Some("Fansub-A".into()),
            resolution: Some("1080i".into()),
            ..ModelRecord::default()
        };
        let r = merge(rec, tok.clone());
        assert_eq!(r.parser, MODEL_ID);
        assert_eq!(r.title, "Show Name");
        assert_eq!(r.episodes, vec![5]);
        assert!(r.absolute, "no season stated: absolute numbering");
        assert_eq!(r.resolution, "1080p");
        assert_eq!(r.codec, tok.codec, "tags come from the tokenizer");
        let r = merge(
            ModelRecord { title: Some("X".into()), season: Some(2), episodes: Some(vec![1, 2]), ..Default::default() },
            Release { season_pack: true, year: 2001, ..Release::default() },
        );
        assert_eq!((r.season, r.absolute, r.season_pack, r.year), (2, false, false, 2001));
        let many = ModelRecord { title: Some("X".into()), episodes: Some((0..5000).collect()), ..Default::default() };
        assert_eq!(merge(many, Release::default()).episodes.len(), MAX_EPISODES);
    }

    #[test]
    fn alternatives_follow_the_merge_rules() {
        let own = || ("blade runner".to_string(), 0, Vec::new());
        let rec = |title: &str, year: Option<i64>, season: Option<i64>, eps: Option<Vec<i64>>| ModelRecord {
            title: Some(title.into()),
            year,
            season,
            episodes: eps,
            ..ModelRecord::default()
        };
        let out = alternatives(
            vec![
                rec("blade  runner", Some(2049), None, None), // the answer again
                rec("Blade Runner 2049", None, None, None),   // the year read as title
                rec("Blade", None, None, Some((0..5000).collect())),
                rec("Blade Runner", Some(1982), Some(0), Some(vec![1])),
            ],
            own(),
            2049,
        );
        assert_eq!(out.len(), 3);
        assert_eq!((out[0].title.as_str(), out[0].year), ("Blade Runner 2049", 0));
        assert_eq!((out[1].year, out[1].episodes.len(), out[1].absolute), (2049, MAX_EPISODES, true));
        assert_eq!((out[2].year, out[2].season, out[2].absolute), (1982, 0, false), "a stated season 0 is seasonal");
        let many = (0..9).map(|i| rec(&format!("Show {i}"), None, None, None)).collect();
        assert_eq!(alternatives(many, own(), 0).len(), MAX_RELEASE_ALTERNATIVES);
    }

    #[test]
    fn a_missing_socket_is_unavailable_without_a_dial() {
        let m = ReleaseModel::new(Endpoint::Unix("/nonexistent/lain-parser.sock".into()));
        assert_eq!(m.health().unwrap_err().code, "dependency-unavailable");
        let e = m.invoke(CAP_RELEASE_PARSE, "parse", json!({"name": "Show - 01"})).unwrap_err();
        assert_eq!((e.code.as_str(), e.msg.as_str()), ("dependency-unavailable", "parser socket not found"));
        assert_eq!(m.invoke(CAP_RELEASE_PARSE, "", json!({"name": ""})).unwrap_err().code, "invalid-message");
        assert_eq!(m.invoke("lain.x@1", "", json!({})).unwrap_err().code, "invalid-message");
        assert_eq!(m.invoke(CAP_RELEASE_PARSE, "nope", json!({})).unwrap_err().code, "invalid-message");
    }

    #[test]
    fn locate_prefers_explicit_then_env_then_the_data_dir_socket() {
        assert_eq!(ReleaseModel::locate(Some("unix:///x.sock"), None).unwrap(), Endpoint::Unix("/x.sock".into()));
        // Without LAIN_PARSER_* in the environment of the test runner.
        if std::env::var_os("LAIN_PARSER_URL").is_none() && std::env::var_os("LAIN_PARSER_SOCKET").is_none() {
            assert_eq!(ReleaseModel::locate(None, None).unwrap(), Endpoint::Tcp("127.0.0.1:9377".into()));
        }
    }
}
