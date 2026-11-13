//! Integrations (D-265..D-267): other programs that feed a library.
//!
//! A downloader or an organizer knows what a file is when it writes it:
//! the series, the episode, where it came from. Instead of lain
//! guessing that again from the name, or reading the tool's own files,
//! the tool tells lain through this API.
//!
//! - An admin creates an **integration** and gets its key once
//!   (`POST /api/integrations`). The key is limited to the libraries
//!   named at creation; it is stored as a keyed hash.
//! - With the key (`Authorization: Bearer lain_…`) a program lists
//!   those libraries with their paths (`GET /api/integration/libraries`),
//!   so it can file a download in a library instead of in a folder it
//!   was told about separately.
//! - It then says what files are (`PUT …/libraries/{id}/files`): for
//!   each path inside the library, the identity (kind, title, season,
//!   episode, year) and, optionally, genres and a synopsis. lain keeps
//!   that in its own store. The next scan identifies those files from
//!   it (`lain-identify-import`, first in the binding order), and the
//!   metadata is the item's overlay at once.
//! - `POST …/libraries/{id}/scan` asks for a scan of that library.
//!
//! An integration key reads and writes nothing else: no catalog, no
//! progress, no users. It works on a private library it was given
//! without unlocking it for anyone.
//!
//! A key has one scope (D-290). `library` is the above. `indexers` is
//! for a coding agent setting up indexers on the owner's behalf: it
//! opens only the `/api/acquire/indexers*` routes (`Gate::Indexers`),
//! takes no libraries, always expires (one hour unless asked, a day at
//! most), and is read from the `Authorization` header only, never from
//! a URL, so it does not end up in logs or shell history as a query.

use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use lain_auth::crypto;
use lain_core::{Error, Provider};
use lain_identify::{CAP_MEDIA_IDENTIFY, Candidate, Proposal};
use lain_metadata::{IMPORT_ID, MetadataRecord};
use lain_store::{Bucket, Store, join_key};

use crate::app::App;
use crate::contracts::Library;
use crate::gateway::{Gate, Reply, Req, route};
use crate::libraries::Libraries;
use crate::roots::{Roots, valid_rel};

pub const IDENTIFY_ID: &str = "lain-identify-import";

/// The most files one request may describe.
const MAX_FILES: usize = 1000;
const MAX_GENRES: usize = 48;
const MAX_TEXT: usize = 8000;
const KEY_PREFIX: &str = "lain_";
const KINDS: &[&str] = &["episode", "video", "movie", "comic", "manga"];

pub const SCOPE_LIBRARY: &str = "library";
pub const SCOPE_INDEXERS: &str = "indexers";
/// The role a request made with an indexers key carries; no gate but
/// `Gate::Indexers` accepts it.
pub const ROLE_INTEGRATION: &str = "integration";
/// An indexers key lives an hour unless asked, and a day at most.
const INDEXERS_TTL: i64 = 3600;
const MIN_TTL: i64 = 60;
const MAX_INDEXERS_TTL: i64 = 86_400;

/// A stored integration. The key itself is never stored.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Integration {
    pub id: String,
    pub name: String,
    /// The libraries the key may list and write to.
    pub libraries: Vec<String>,
    pub created_at: i64,
    /// Empty on keys made before scopes: those are `library` keys.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    /// Unix seconds; 0 never expires.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub expires_at: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    key_hash: String,
}

fn is_zero(v: &i64) -> bool {
    *v == 0
}

impl Integration {
    /// The scope this key has (`library` for keys older than scopes).
    pub fn scope(&self) -> &str {
        self.scopes.first().map(String::as_str).unwrap_or(SCOPE_LIBRARY)
    }

    pub fn expired(&self, now: i64) -> bool {
        self.expires_at != 0 && now >= self.expires_at
    }

    fn view(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "libraries": self.libraries,
            "created_at": self.created_at,
            "scopes": [self.scope()],
            "expires_at": self.expires_at,
        })
    }
}

/// What an integration said one file is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Import {
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub season: i64,
    #[serde(default)]
    pub episode: i64,
    #[serde(default)]
    pub year: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub synopsis: String,
    /// The integration that wrote it.
    #[serde(default)]
    pub by: String,
    #[serde(default)]
    pub updated_at: i64,
}

fn hash(key: &str) -> String {
    crypto::hmac_sha256(b"lain.integration.key", key.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn random_hex(n: usize) -> Result<String, String> {
    let mut buf = vec![0u8; n];
    crypto::random_bytes(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

fn all(store: &Store) -> Vec<Integration> {
    let rows = store.read(|tx| tx.all(Bucket::Integrations)).unwrap_or_default();
    let mut out: Vec<Integration> = rows.iter().filter_map(|(_, v)| serde_json::from_slice(v).ok()).collect();
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    out
}

/// The integration `key` belongs to, when it has `scope` and has not
/// expired. An expired key is removed on the way.
fn lookup(app: &App, key: &str, scope: &str) -> Result<Integration, Reply> {
    let want = hash(key);
    let who =
        all(&app.store).into_iter().find(|i| i.key_hash == want).ok_or_else(|| Reply::err(401, "unauthorized"))?;
    if who.expired(lain_core::now_unix()) {
        let _ = app.store.write(|tx| tx.delete(Bucket::Integrations, who.id.as_bytes()).map(|_| ()));
        tracing::info!(integration = %who.id, "expired integration removed");
        return Err(Reply::err(401, "key expired"));
    }
    if who.scope() != scope {
        // Answered like a wrong key: a key never learns what else exists.
        return Err(Reply::err(401, "unauthorized"));
    }
    Ok(who)
}

/// The library integration a request's key belongs to. These routes
/// predate D-290 and still take `?token=`.
fn authorized(app: &App, req: &Req) -> Result<Integration, Reply> {
    let key = req.token().filter(|t| t.starts_with(KEY_PREFIX)).ok_or_else(|| Reply::err(401, "unauthorized"))?;
    lookup(app, key, SCOPE_LIBRARY)
}

/// For `Gate::Indexers`: `None` when the request carries no integration
/// key (the session check decides), else the indexers integration or
/// the refusal. The key is read from the `Authorization` header only.
pub(crate) fn indexers_caller(app: &App, req: &Req) -> Option<Result<Integration, Reply>> {
    if req.query("token").starts_with(KEY_PREFIX) {
        return Some(Err(Reply::err(401, "an integration key goes in the Authorization header, never in the URL")));
    }
    let key = req.authorization.as_deref().and_then(|h| h.strip_prefix("Bearer ")).map(str::trim)?;
    key.starts_with(KEY_PREFIX).then(|| lookup(app, key, SCOPE_INDEXERS))
}

/// Drops expired keys (they no longer open anything either way).
fn purge_expired(store: &Store) {
    let now = lain_core::now_unix();
    let gone: Vec<Integration> = all(store).into_iter().filter(|i| i.expired(now)).collect();
    if gone.is_empty() {
        return;
    }
    let _ = store.write(|tx| {
        for i in &gone {
            tx.delete(Bucket::Integrations, i.id.as_bytes())?;
        }
        Ok(())
    });
}

/// The library `id`, when this integration was given it. One it was
/// not given answers like one that does not exist.
fn library(app: &App, who: &Integration, id: &str) -> Result<Library, Reply> {
    who.libraries
        .iter()
        .find(|l| l.as_str() == id)
        .and_then(|l| app.libraries.get(l))
        .ok_or_else(|| Reply::err(404, "unknown library"))
}

/// A file's path inside its library, `/`-separated, from the absolute
/// path a scan sees.
fn relative(lib: &Library, path: &str) -> Option<String> {
    let below = std::path::Path::new(path).strip_prefix(&lib.path).ok()?;
    let rel = below.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
    (!rel.is_empty()).then_some(rel)
}

/// The catalog id a scan gives the file (`lain_ingest::commit`): by
/// absolute path, or by root-relative path for a library in a root.
fn item_id(lib: &Library, rel: &str) -> String {
    let loc = if lib.root.is_empty() {
        format!("{}/{rel}", lib.path.trim_end_matches('/'))
    } else {
        match lib.root_path.trim_matches('/') {
            "" => rel.to_string(),
            r => format!("{r}/{rel}"),
        }
    };
    lain_ingest::commit::item_id(&lib.id, &loc)
}

fn clip(s: &str, max: usize) -> String {
    let s = s.trim();
    let cut = (0..=max.min(s.len())).rev().find(|i| s.is_char_boundary(*i)).unwrap_or(0);
    s[..cut].to_string()
}

// ---- admin: the integrations themselves ----------------------------------------

fn list(app: &App, _: &Req, _: &lain_auth::Verified) -> Reply {
    purge_expired(&app.store);
    let items: Vec<Value> = all(&app.store).iter().map(Integration::view).collect();
    Reply::ok(json!({"integrations": items}))
}

/// The scope, libraries and expiry a create asks for, checked.
fn shape(scopes: &[String], libraries: Vec<String>, ttl: i64, now: i64) -> Result<(Vec<String>, i64), String> {
    let scope = match scopes {
        [] => SCOPE_LIBRARY,
        [s] if s == SCOPE_LIBRARY || s == SCOPE_INDEXERS => s.as_str(),
        _ => return Err(format!("scopes: one of {SCOPE_LIBRARY} or {SCOPE_INDEXERS}")),
    };
    if scope == SCOPE_INDEXERS {
        if !libraries.is_empty() {
            return Err("an indexers key takes no libraries".into());
        }
        let ttl = if ttl == 0 { INDEXERS_TTL } else { ttl };
        if !(MIN_TTL..=MAX_INDEXERS_TTL).contains(&ttl) {
            return Err(format!("ttl_seconds: {MIN_TTL} to {MAX_INDEXERS_TTL} for an indexers key"));
        }
        return Ok((vec![SCOPE_INDEXERS.into()], now + ttl));
    }
    if libraries.is_empty() {
        return Err("at least one library required".into());
    }
    if ttl != 0 && ttl < MIN_TTL {
        return Err(format!("ttl_seconds: 0 (never) or at least {MIN_TTL}"));
    }
    Ok((Vec::new(), if ttl == 0 { 0 } else { now + ttl }))
}

fn create(app: &App, req: &Req, _: &lain_auth::Verified) -> Reply {
    #[derive(Deserialize)]
    struct In {
        #[serde(default)]
        name: String,
        #[serde(default)]
        libraries: Vec<String>,
        #[serde(default)]
        scopes: Vec<String>,
        #[serde(default)]
        ttl_seconds: i64,
    }
    let i: In = match req.decode() {
        Ok(i) => i,
        Err(r) => return r,
    };
    let name = clip(&i.name, 80);
    if name.is_empty() {
        return Reply::err(400, "name required");
    }
    let mut libraries = i.libraries;
    libraries.sort();
    libraries.dedup();
    let now = lain_core::now_unix();
    let (scopes, expires_at) = match shape(&i.scopes, libraries.clone(), i.ttl_seconds, now) {
        Ok(s) => s,
        Err(e) => return Reply::err(400, &e),
    };
    purge_expired(&app.store);
    if let Some(unknown) = libraries.iter().find(|l| app.libraries.get(l).is_none()) {
        return Reply::err(400, &format!("unknown library {unknown}"));
    }
    let (id, secret) = match (random_hex(4), random_hex(24)) {
        (Ok(id), Ok(secret)) => (format!("int-{id}"), secret),
        _ => return Reply::err(500, "could not make a key"),
    };
    let key = format!("{KEY_PREFIX}{secret}");
    let rec = Integration { id, name, libraries, created_at: now, scopes, expires_at, key_hash: hash(&key) };
    if let Err(e) = &app.store.write(|tx| tx.put_json(Bucket::Integrations, rec.id.as_bytes(), &rec)) {
        return Reply::err(500, &e.to_string());
    }
    // Never the key: only its id, name and scope.
    tracing::info!(integration = %rec.id, name = %rec.name, scope = rec.scope(), "integration created");
    // The only time the key leaves the server.
    let mut out = rec.view();
    out["key"] = json!(key);
    Reply::json(201, out)
}

fn delete(app: &App, req: &Req, _: &lain_auth::Verified) -> Reply {
    let id = req.param("id").to_string();
    match &app.store.write(|tx| tx.delete(Bucket::Integrations, id.as_bytes())) {
        Ok(true) => {
            tracing::info!(integration = %id, "integration removed");
            Reply::ok(json!({"status": "ok"}))
        }
        Ok(false) => Reply::err(404, "unknown integration"),
        Err(e) => Reply::err(500, &e.to_string()),
    }
}

// ---- with a key ------------------------------------------------------------------

fn libraries(app: &App, req: &Req, _: &lain_auth::Verified) -> Reply {
    let who = match authorized(app, req) {
        Ok(w) => w,
        Err(r) => return r,
    };
    let libs: Vec<Value> = who
        .libraries
        .iter()
        .filter_map(|id| app.libraries.get(id))
        .map(|l| json!({"id": l.id, "name": l.name, "type": l.kind, "path": l.path}))
        .collect();
    Reply::ok(json!({"integration": {"id": who.id, "name": who.name}, "libraries": libs}))
}

#[derive(Deserialize)]
struct FileIn {
    #[serde(default)]
    path: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    season: i64,
    #[serde(default)]
    episode: i64,
    #[serde(default)]
    year: i64,
    #[serde(default)]
    genres: Vec<String>,
    #[serde(default)]
    synopsis: String,
}

/// Checks one entry: the path stays inside the library, the kind is
/// one lain has, and the numbers are plausible.
fn import_of(f: FileIn, by: &str, now: i64) -> Result<(String, Import), String> {
    let rel = f.path.trim().trim_start_matches("./").to_string();
    if rel.is_empty() || !valid_rel(&rel) || rel.len() > 1024 {
        return Err(format!("path must be relative to the library, with / separators: {:?}", clip(&f.path, 80)));
    }
    let title = clip(&f.title, 300);
    if title.is_empty() {
        return Err(format!("{rel}: title required"));
    }
    let kind = if f.kind.is_empty() { "video".to_string() } else { f.kind.to_lowercase() };
    if !KINDS.contains(&kind.as_str()) {
        return Err(format!("{rel}: unknown kind {kind}"));
    }
    if !(0..=9999).contains(&f.season)
        || !(0..=99999).contains(&f.episode)
        || !(f.year == 0 || (1800..=2200).contains(&f.year))
    {
        return Err(format!("{rel}: season, episode or year out of range"));
    }
    let mut genres: Vec<String> = Vec::new();
    for g in f.genres {
        let g = clip(&g, 64);
        if !g.is_empty() && !genres.iter().any(|x| x.eq_ignore_ascii_case(&g)) {
            genres.push(g);
        }
        if genres.len() == MAX_GENRES {
            break;
        }
    }
    let rec = Import {
        kind,
        title,
        season: f.season,
        episode: f.episode,
        year: f.year,
        genres,
        synopsis: clip(&f.synopsis, MAX_TEXT),
        by: by.to_string(),
        updated_at: now,
    };
    Ok((rel, rec))
}

fn files_put(app: &App, req: &Req, _: &lain_auth::Verified) -> Reply {
    #[derive(Deserialize)]
    struct In {
        #[serde(default)]
        files: Vec<FileIn>,
    }
    let who = match authorized(app, req) {
        Ok(w) => w,
        Err(r) => return r,
    };
    let lib = match library(app, &who, req.param("id")) {
        Ok(l) => l,
        Err(r) => return r,
    };
    let i: In = match req.decode() {
        Ok(i) => i,
        Err(r) => return r,
    };
    if i.files.is_empty() || i.files.len() > MAX_FILES {
        return Reply::err(400, &format!("files: 1 to {MAX_FILES} entries"));
    }
    let now = lain_core::now_unix();
    let mut recs = Vec::with_capacity(i.files.len());
    for f in i.files {
        match import_of(f, &who.id, now) {
            Ok(r) => recs.push(r),
            // All or nothing: a bad entry is the caller's bug to see.
            Err(e) => return Reply::err(400, &e),
        }
    }
    let written = &app.store.write(|tx| {
        for (rel, rec) in &recs {
            tx.put_json(Bucket::Imports, &join_key(&lib.id, rel), rec)?;
        }
        Ok(())
    });
    if let Err(e) = written {
        return Reply::err(500, &e.to_string());
    }
    // The overlay is keyed by the id the scan will give the file, so it
    // is there whether the scan has seen the file yet or not.
    let mut described = 0;
    for (rel, rec) in &recs {
        if rec.genres.is_empty() && rec.synopsis.is_empty() {
            continue;
        }
        let overlay = MetadataRecord {
            provider: IMPORT_ID.into(),
            remote_id: rel.clone(),
            title: rec.title.clone(),
            year: rec.year,
            genres: rec.genres.clone(),
            synopsis: rec.synopsis.clone(),
            ..MetadataRecord::default()
        };
        if lain_metadata::store::save(&app.store, &item_id(&lib, rel), &overlay, now).is_ok() {
            described += 1;
        }
    }
    tracing::info!(integration = %who.id, library = %lib.id, files = recs.len(), "files imported");
    Reply::ok(json!({"stored": recs.len(), "described": described}))
}

fn scan(app: &App, req: &Req, _: &lain_auth::Verified) -> Reply {
    let who = match authorized(app, req) {
        Ok(w) => w,
        Err(r) => return r,
    };
    let lib = match library(app, &who, req.param("id")) {
        Ok(l) => l,
        Err(r) => return r,
    };
    if !app.scanner.start(vec![lib], "integration") {
        return Reply::err(409, "scan already running");
    }
    Reply::json(202, json!({"state": "running"}))
}

pub fn routes() -> axum::Router<Arc<App>> {
    use axum::routing::MethodFilter;
    // The key routes are public to the session gate: `authorized` checks
    // the integration key instead.
    axum::Router::new()
        .route(
            "/api/integrations",
            route(MethodFilter::GET, Gate::Admin, list).merge(route(MethodFilter::POST, Gate::Admin, create)),
        )
        .route("/api/integrations/{id}", route(MethodFilter::DELETE, Gate::Admin, delete))
        .route("/api/integration/libraries", route(MethodFilter::GET, Gate::Public, libraries))
        .route("/api/integration/libraries/{id}/files", route(MethodFilter::PUT, Gate::Public, files_put))
        .route("/api/integration/libraries/{id}/scan", route(MethodFilter::POST, Gate::Public, scan))
        .route("/api/agent/indexers.md", route(MethodFilter::GET, Gate::Public, agent_guide))
}

/// The guide a coding agent reads before it touches indexers (D-291):
/// the rules and the exact calls. Public: it holds no secret, and the
/// agent may fetch it before it is handed the key.
pub const AGENT_GUIDE: &str = include_str!("agent-indexers.md");

fn agent_guide(_: &App, _: &Req, _: &lain_auth::Verified) -> Reply {
    Reply::bytes(200, "text/markdown; charset=utf-8", AGENT_GUIDE.as_bytes().to_vec())
        .header(axum::http::header::CACHE_CONTROL, "no-cache")
}

// ---- identification --------------------------------------------------------------

/// `lain-identify-import` (`lain.media.identify@1`), first in the
/// binding order: a file an integration described is what it said. It
/// declines every other file, so the name-based identifiers decide.
pub struct ImportIdentifier {
    store: Arc<Store>,
    libraries: Libraries,
}

impl ImportIdentifier {
    pub fn new(store: Arc<Store>, roots: Arc<RwLock<Roots>>) -> Self {
        ImportIdentifier { libraries: Libraries::new(store.clone(), roots), store }
    }

    pub fn identify(&self, c: &Candidate) -> Proposal {
        let Some(lib) = self.libraries.get(&c.library_id) else { return Proposal::default() };
        let Some(rel) = relative(&lib, &c.path) else { return Proposal::default() };
        let rec: Option<Import> =
            self.store.read(|tx| tx.get_json(Bucket::Imports, &join_key(&lib.id, &rel))).ok().flatten();
        let Some(rec) = rec.filter(|r| !r.title.is_empty()) else { return Proposal::default() };
        Proposal {
            kind: rec.kind,
            title: rec.title,
            season: rec.season,
            episode: rec.episode,
            year: rec.year,
            confidence: 0.98,
            evidence: vec!["import".into()],
            plugin_id: IDENTIFY_ID.into(),
        }
    }
}

impl Provider for ImportIdentifier {
    fn id(&self) -> &str {
        IDENTIFY_ID
    }

    fn capabilities(&self) -> Vec<String> {
        vec![CAP_MEDIA_IDENTIFY.into()]
    }

    fn health(&self) -> Result<(), Error> {
        Ok(())
    }

    fn invoke(&self, cap: &str, op: &str, input: Value) -> Result<Value, Error> {
        if cap != CAP_MEDIA_IDENTIFY {
            return Err(Error::invalid(format!("unsupported cap {cap}")));
        }
        if !op.is_empty() && op != "identify" {
            return Err(Error::invalid(format!("capability {cap} has no op {op}")));
        }
        let c: Candidate =
            serde_json::from_value(input).map_err(|e| Error::invalid(format!("Candidate required: {e}")))?;
        Ok(json!(self.identify(&c)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, title: &str) -> FileIn {
        FileIn {
            path: path.into(),
            kind: String::new(),
            title: title.into(),
            season: 0,
            episode: 0,
            year: 0,
            genres: Vec::new(),
            synopsis: String::new(),
        }
    }

    #[test]
    fn an_entry_stays_inside_its_library() {
        let (rel, rec) = import_of(file("./Show/Show - 01.mkv", " Show "), "int-1", 7).unwrap();
        assert_eq!((rel.as_str(), rec.kind.as_str(), rec.title.as_str()), ("Show/Show - 01.mkv", "video", "Show"));
        assert_eq!((rec.by.as_str(), rec.updated_at), ("int-1", 7));
        for bad in ["/etc/passwd", "../other/x.mkv", "a/../../x.mkv", "a\\b.mkv", ""] {
            assert!(import_of(file(bad, "t"), "i", 0).is_err(), "{bad}");
        }
        assert!(import_of(file("a.mkv", "  "), "i", 0).is_err());
        let mut odd = file("a.mkv", "t");
        odd.kind = "podcast".into();
        assert!(import_of(odd, "i", 0).is_err());
        let mut far = file("a.mkv", "t");
        far.year = 99;
        assert!(import_of(far, "i", 0).is_err());
    }

    #[test]
    fn genres_are_bounded_and_deduplicated() {
        let mut f = file("a.mkv", "t");
        f.genres = vec!["Drama".into(), "drama".into(), " ".into(), "x".repeat(200)];
        f.synopsis = "é".repeat(MAX_TEXT);
        let (_, rec) = import_of(f, "i", 0).unwrap();
        assert_eq!(rec.genres.len(), 2);
        assert_eq!(rec.genres[1].len(), 64);
        assert!(rec.synopsis.len() <= MAX_TEXT && rec.synopsis.is_char_boundary(rec.synopsis.len()));
    }

    #[test]
    fn ids_match_what_a_scan_derives() {
        let plain = Library { id: "lib-1".into(), path: "/media/anime/".into(), ..Default::default() };
        assert_eq!(item_id(&plain, "Show/01.mkv"), lain_ingest::commit::item_id("lib-1", "/media/anime/Show/01.mkv"));
        let rooted = Library {
            id: "lib-2".into(),
            path: "/mnt/v/media/anime".into(),
            root: "media".into(),
            root_path: "anime".into(),
            ..Default::default()
        };
        assert_eq!(item_id(&rooted, "Show/01.mkv"), lain_ingest::commit::item_id("lib-2", "anime/Show/01.mkv"));
        assert_eq!(relative(&plain, "/media/anime/Show/01.mkv").as_deref(), Some("Show/01.mkv"));
        assert_eq!(relative(&plain, "/elsewhere/01.mkv"), None);
    }

    #[test]
    fn an_indexers_key_takes_no_libraries_and_always_expires() {
        let libs = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let ix = vec![SCOPE_INDEXERS.to_string()];
        assert_eq!(shape(&ix, vec![], 0, 1000), Ok((ix.clone(), 1000 + INDEXERS_TTL)));
        assert_eq!(shape(&ix, vec![], 900, 1000), Ok((ix.clone(), 1900)));
        assert!(shape(&ix, libs(&["lib-1"]), 0, 0).is_err());
        for ttl in [-1, 59, MAX_INDEXERS_TTL + 1] {
            assert!(shape(&ix, vec![], ttl, 0).is_err(), "{ttl}");
        }
        // Library keys keep their old shape: libraries, no expiry.
        assert_eq!(shape(&[], libs(&["lib-1"]), 0, 1000), Ok((vec![], 0)));
        assert_eq!(shape(&[SCOPE_LIBRARY.into()], libs(&["lib-1"]), 120, 1000), Ok((vec![], 1120)));
        assert!(shape(&[], vec![], 0, 0).is_err());
        assert!(shape(&["admin".into()], vec![], 0, 0).is_err());
        assert!(shape(&[SCOPE_LIBRARY.into(), SCOPE_INDEXERS.into()], libs(&["lib-1"]), 0, 0).is_err());
    }

    #[test]
    fn scope_and_expiry_of_a_stored_key() {
        let old: Integration =
            serde_json::from_str(r#"{"id":"int-1","name":"n","libraries":["l"],"created_at":1}"#).unwrap();
        assert_eq!(old.scope(), SCOPE_LIBRARY, "keys from before scopes are library keys");
        assert!(!old.expired(i64::MAX));
        let ix = Integration { scopes: vec![SCOPE_INDEXERS.into()], expires_at: 100, ..Default::default() };
        assert_eq!(ix.scope(), SCOPE_INDEXERS);
        assert!(!ix.expired(99) && ix.expired(100));
        assert_eq!(ix.view()["scopes"], json!(["indexers"]));
    }

    /// The indexers gate opens only the indexer routes: no route file
    /// names it in a `route(…)`, and the routes of acquire.rs that take
    /// its alias are exactly the `/api/acquire/indexers*` ones.
    #[test]
    fn only_indexer_routes_take_an_indexers_key() {
        for (name, src) in [
            ("gateway.rs", include_str!("gateway.rs")),
            ("acquire.rs", include_str!("acquire.rs")),
            ("population.rs", include_str!("population.rs")),
            ("watch.rs", include_str!("watch.rs")),
            ("lists.rs", include_str!("lists.rs")),
            ("social.rs", include_str!("social.rs")),
            ("translate.rs", include_str!("translate.rs")),
        ] {
            assert!(
                !src.lines().any(|l| l.contains("route(") && l.contains("Gate::Indexers")),
                "{name} gives a route the indexers gate directly"
            );
        }
        assert!(
            !include_str!("integration.rs")
                .lines()
                .any(|l| l.contains("route(") && l.contains(concat!("Gate::", "Indexers,")))
        );
        let acquire = include_str!("acquire.rs");
        assert_eq!(acquire.lines().filter(|l| l.trim() == "let ix = Gate::Indexers;").count(), 1);
        let mut gated = 0;
        for line in acquire.lines().filter(|l| l.contains(".route(\"")) {
            let path = line.split('"').nth(1).unwrap_or("");
            if line.contains(", ix,") {
                gated += 1;
                assert!(path.starts_with("/api/acquire/indexers"), "{path} takes an indexers key");
            } else {
                assert!(!path.starts_with("/api/acquire/indexers"), "{path} should take an indexers key");
            }
        }
        assert_eq!(gated, 6);
    }

    #[test]
    fn a_key_is_stored_only_as_a_hash() {
        let h = hash("lain_abc");
        assert_eq!(h.len(), 64);
        assert_ne!(h, hash("lain_abd"));
        let rec = Integration { id: "int-1".into(), key_hash: h.clone(), ..Default::default() };
        let stored = serde_json::to_string(&rec).unwrap();
        assert!(stored.contains(&h) && !stored.contains("lain_abc"));
    }
}
