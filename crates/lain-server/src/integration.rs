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

/// A stored integration. The key itself is never stored.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Integration {
    pub id: String,
    pub name: String,
    /// The libraries the key may list and write to.
    pub libraries: Vec<String>,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    key_hash: String,
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

/// The integration a request's key belongs to.
fn authorized(app: &App, req: &Req) -> Result<Integration, Reply> {
    let key = req.token().filter(|t| t.starts_with(KEY_PREFIX)).ok_or_else(|| Reply::err(401, "unauthorized"))?;
    let want = hash(key);
    all(&app.store).into_iter().find(|i| i.key_hash == want).ok_or_else(|| Reply::err(401, "unauthorized"))
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
    let items: Vec<Value> = all(&app.store)
        .into_iter()
        .map(|i| json!({"id": i.id, "name": i.name, "libraries": i.libraries, "created_at": i.created_at}))
        .collect();
    Reply::ok(json!({"integrations": items}))
}

fn create(app: &App, req: &Req, _: &lain_auth::Verified) -> Reply {
    #[derive(Deserialize)]
    struct In {
        #[serde(default)]
        name: String,
        #[serde(default)]
        libraries: Vec<String>,
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
    if libraries.is_empty() {
        return Reply::err(400, "at least one library required");
    }
    if let Some(unknown) = libraries.iter().find(|l| app.libraries.get(l).is_none()) {
        return Reply::err(400, &format!("unknown library {unknown}"));
    }
    let (id, secret) = match (random_hex(4), random_hex(24)) {
        (Ok(id), Ok(secret)) => (format!("int-{id}"), secret),
        _ => return Reply::err(500, "could not make a key"),
    };
    let key = format!("{KEY_PREFIX}{secret}");
    let rec = Integration { id, name, libraries, created_at: lain_core::now_unix(), key_hash: hash(&key) };
    if let Err(e) = &app.store.write(|tx| tx.put_json(Bucket::Integrations, rec.id.as_bytes(), &rec)) {
        return Reply::err(500, &e.to_string());
    }
    tracing::info!(integration = %rec.id, name = %rec.name, "integration created");
    // The only time the key leaves the server.
    Reply::json(
        201,
        json!({"id": rec.id, "name": rec.name, "libraries": rec.libraries, "created_at": rec.created_at, "key": key}),
    )
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
    fn a_key_is_stored_only_as_a_hash() {
        let h = hash("lain_abc");
        assert_eq!(h.len(), 64);
        assert_ne!(h, hash("lain_abd"));
        let rec = Integration { id: "int-1".into(), key_hash: h.clone(), ..Default::default() };
        let stored = serde_json::to_string(&rec).unwrap();
        assert!(stored.contains(&h) && !stored.contains("lain_abc"));
    }
}
