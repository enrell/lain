//! `lain-rs import --from <go data dir>`: a one-shot, read-only copy of
//! the Go server's state into a fresh Rust store (DESIGN-v2 §13 "Data").
//!
//! - **bbolt** (`lain.db`): read with the pure-Rust reader in
//!   `lain_store::bbolt`, under a shared lock so a running Go server is
//!   refused rather than read mid-write. No Go code runs and no Go file
//!   is opened for writing.
//! - **v0.1 JSON documents** (`users.json`, `secret.json`,
//!   `libraries.json`, `catalog.json`, `userstate.json`) when there is
//!   no `lain.db`.
//!
//! What moves: users (bcrypt hashes as stored, so passwords keep
//! working), the token secret (so issued tokens keep verifying),
//! libraries, the catalog as a `lain.catalog-export` document (or a
//! document given with `--catalog`), progress, and every other bucket
//! verbatim (its own table, or `go_raw` for the social and acquire
//! buckets), so nothing is lost before those modules are ported.
//! `composition.json` (Go provider ids), `plugins/` manifests and the
//! file caches are listed in the report and left alone.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};

use lain_auth::User;
use lain_store::bbolt::BoltFile;
use lain_store::{Bucket, Store, join_key};

use crate::catalog::Catalog;
use crate::contracts::{CatalogItem, EXPORT_FORMAT, ExportDoc, Library, Progress};

/// What the import read and wrote.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub source: String,
    pub format: String,
    pub counts: BTreeMap<String, usize>,
    pub skipped: Vec<String>,
    pub notes: Vec<String>,
}

impl Report {
    pub fn to_json(&self) -> Value {
        json!({"source": self.source, "format": self.format, "counts": self.counts, "skipped": self.skipped, "notes": self.notes})
    }
}

/// The Go state, read into memory.
#[derive(Default)]
struct Source {
    users: Vec<(Vec<u8>, Vec<u8>)>,
    secret: Option<Vec<u8>>,
    libraries: Vec<(Vec<u8>, Vec<u8>)>,
    catalog: ExportDoc,
    progress: Vec<(Vec<u8>, Vec<u8>)>,
    raw: BTreeMap<Bucket, Vec<(Vec<u8>, Vec<u8>)>>,
    go_raw_buckets: Vec<String>,
    skipped: Vec<String>,
}

fn read_bolt(path: &Path) -> Result<(Source, String), String> {
    let f = BoltFile::open(path)?;
    let mut src = Source::default();
    for name in f.bucket_names()? {
        let pairs = f.pairs(&name)?;
        match Bucket::from_name(&name) {
            Some(Bucket::Users) => src.users = pairs,
            Some(Bucket::UsersByName) => {} // rebuilt from users
            Some(Bucket::Libraries) => src.libraries = pairs,
            Some(Bucket::Items) => {
                for (k, v) in pairs {
                    let it: CatalogItem = serde_json::from_slice(&v)
                        .map_err(|e| format!("items/{}: {e}", String::from_utf8_lossy(&k)))?;
                    src.catalog.items.insert(String::from_utf8_lossy(&k).into_owned(), it);
                }
            }
            Some(Bucket::ItemsByLibrary) => {} // rebuilt with the catalog
            Some(Bucket::Progress) => src.progress = pairs,
            Some(Bucket::Meta) => {
                for (k, v) in pairs {
                    if k == b"secret" {
                        src.secret = Some(v);
                    } else {
                        src.raw.entry(Bucket::Meta).or_default().push((k, v));
                    }
                }
            }
            Some(Bucket::GoRaw) => return Err("the source has a go_raw bucket; it is not a Go store".into()),
            Some(b) => {
                src.raw.insert(b, pairs);
            }
            None => {
                // Kept verbatim until its module is ported.
                let raw = src.raw.entry(Bucket::GoRaw).or_default();
                for (k, v) in pairs {
                    raw.push((join_key(&name, &String::from_utf8_lossy(&k)), v));
                }
                src.go_raw_buckets.push(name);
            }
        }
    }
    src.catalog.format = EXPORT_FORMAT.into();
    src.catalog.version = 2;
    Ok((src, format!("bbolt (txid {})", f.txid())))
}

fn read_json_docs(dir: &Path) -> Result<Source, String> {
    let load = |name: &str| -> Result<Option<Value>, String> {
        match std::fs::read(dir.join(name)) {
            Ok(raw) => serde_json::from_slice(&raw).map(Some).map_err(|e| format!("corrupt document {name}: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{name}: {e}")),
        }
    };
    let mut src = Source::default();
    // v0.1 users had no roles: user-admin is the admin (kv.ImportLegacy).
    for u in load("users.json")?.and_then(|v| v.as_array().cloned()).unwrap_or_default() {
        let id = u["id"].as_str().unwrap_or_default().to_string();
        let rec = json!({
            "id": id, "username": u["username"], "pass_hash": u["pass_hash"],
            "role": if id == "user-admin" { "admin" } else { "user" },
            "disabled": false, "pwd_ver": 1, "created_at": u["created_at"],
        });
        src.users.push((id.into_bytes(), serde_json::to_vec(&rec).map_err(|e| e.to_string())?));
    }
    if let Some(Value::String(s)) = load("secret.json")? {
        src.secret = Some(s.into_bytes());
    }
    for l in load("libraries.json")?.and_then(|v| v.as_array().cloned()).unwrap_or_default() {
        let id = l["id"].as_str().unwrap_or_default().to_string();
        src.libraries.push((id.into_bytes(), serde_json::to_vec(&l).map_err(|e| e.to_string())?));
    }
    if let Some(Value::Object(items)) = load("catalog.json")? {
        for (id, raw) in items {
            let it: CatalogItem = serde_json::from_value(raw).map_err(|e| format!("catalog.json/{id}: {e}"))?;
            src.catalog.items.insert(id, it);
        }
    }
    if let Some(Value::Object(prog)) = load("userstate.json")? {
        for (k, raw) in prog {
            src.progress.push((k.into_bytes(), serde_json::to_vec(&raw).map_err(|e| e.to_string())?));
        }
    }
    src.catalog.format = EXPORT_FORMAT.into();
    src.catalog.version = 1;
    Ok(src)
}

/// Size and modification time of every file under `dir`, to prove the
/// source was not touched.
pub fn fingerprint(dir: &Path) -> BTreeMap<PathBuf, (u64, std::time::SystemTime)> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            match e.metadata() {
                Ok(m) if m.is_dir() => stack.push(p),
                Ok(m) => {
                    out.insert(p, (m.len(), m.modified().unwrap_or(std::time::UNIX_EPOCH)));
                }
                Err(_) => {}
            }
        }
    }
    out
}

/// Imports `from` (a Go data dir) into the Rust store at `to`.
/// `catalog` overrides the catalog with an export document.
pub fn run(from: &Path, to: &Path, catalog: Option<&Path>) -> Result<Report, String> {
    let from_abs = from.canonicalize().map_err(|e| format!("{}: {e}", from.display()))?;
    if to.exists() && to.canonicalize().ok().as_deref() == Some(from_abs.as_path()) {
        return Err("--from and --data-dir are the same directory; the Go data dir is never written".into());
    }
    if to.canonicalize().ok().is_some_and(|t| t.starts_with(&from_abs)) {
        return Err("the target data dir must not live inside the Go data dir".into());
    }
    let before = fingerprint(&from_abs);
    let bolt = from_abs.join("lain.db");
    let (mut src, format) = if bolt.exists() {
        read_bolt(&bolt)?
    } else if from_abs.join("users.json").exists() {
        (read_json_docs(&from_abs)?, "v0.1 JSON documents".to_string())
    } else {
        return Err(format!("{} has neither lain.db nor users.json; is it a lain data dir?", from_abs.display()));
    };
    if let Some(path) = catalog {
        let raw = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        src.catalog = serde_json::from_slice(&raw).map_err(|e| format!("{}: {e}", path.display()))?;
    }

    let store = Arc::new(Store::open(to).map_err(|e| e.to_string())?);
    let existing = store.read(|tx| tx.count(Bucket::Users)).map_err(|e| e.to_string())?;
    if existing > 0 {
        return Err(format!("{} already has {existing} users; import only into a fresh data dir", to.display()));
    }
    let mut report = Report {
        source: from_abs.display().to_string(),
        format,
        skipped: std::mem::take(&mut src.skipped),
        ..Report::default()
    };

    // Validate the shapes this server reads before writing anything.
    let mut users = Vec::new();
    for (k, v) in &src.users {
        let u: User = serde_json::from_slice(v).map_err(|e| format!("users/{}: {e}", String::from_utf8_lossy(k)))?;
        users.push((k.clone(), v.clone(), u.username));
    }
    for (k, v) in &src.libraries {
        serde_json::from_slice::<Library>(v).map_err(|e| format!("libraries/{}: {e}", String::from_utf8_lossy(k)))?;
    }
    for (k, v) in &src.progress {
        serde_json::from_slice::<Progress>(v).map_err(|e| format!("progress/{}: {e}", String::from_utf8_lossy(k)))?;
    }
    store
        .write(|tx| {
            for (k, v, name) in &users {
                tx.put(Bucket::Users, k, v)?;
                tx.put(Bucket::UsersByName, name.as_bytes(), k)?;
            }
            if let Some(s) = &src.secret {
                tx.put(Bucket::Meta, b"secret", s)?;
            }
            for (k, v) in &src.libraries {
                tx.put(Bucket::Libraries, k, v)?;
            }
            for (k, v) in &src.progress {
                // v0.1 keys were "user\x00item" already; keep them.
                let key = if k.contains(&0) {
                    k.clone()
                } else {
                    let p: Progress = serde_json::from_slice(v).unwrap_or_default();
                    join_key(&p.user_id, &p.item_id)
                };
                tx.put(Bucket::Progress, &key, v)?;
            }
            for (b, pairs) in &src.raw {
                for (k, v) in pairs {
                    tx.put(*b, k, v)?;
                }
            }
            Ok(())
        })
        .map_err(|e| e.to_string())?;
    let items = Catalog::new(store.clone()).import(&src.catalog).map_err(|e| e.to_string())?;

    report.counts.insert("users".into(), users.len());
    report.counts.insert("libraries".into(), src.libraries.len());
    report.counts.insert("items".into(), items);
    report.counts.insert("progress".into(), src.progress.len());
    for (b, pairs) in &src.raw {
        report.counts.insert(b.name().into(), pairs.len());
    }
    if !src.go_raw_buckets.is_empty() {
        report.notes.push(format!("kept verbatim in go_raw (no module ported yet): {}", src.go_raw_buckets.join(", ")));
    }
    report.notes.push(if src.secret.is_some() {
        "token secret imported: tokens the Go server issued keep verifying".into()
    } else {
        "no token secret in the source; a new one was generated".into()
    });
    if from_abs.join("composition.json").exists() {
        report.skipped.push("composition.json: names Go providers; lain-rs keeps its own composition".into());
    }
    let manifests = std::fs::read_dir(from_abs.join("plugins"))
        .map(|rd| rd.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "json")).count())
        .unwrap_or(0);
    if manifests > 0 {
        report
            .skipped
            .push(format!("plugins/: {manifests} component manifests; copy the ones you want into the new plugins/"));
    }
    for d in ["avatars", "thumbnails", "transcodes"] {
        if from_abs.join(d).is_dir() {
            report.skipped.push(format!("{d}/: files are not copied in P1a"));
        }
    }
    if fingerprint(&from_abs) != before {
        return Err(
            "the source data dir changed during the import (is a Go server running?); the import is not trustworthy"
                .into(),
        );
    }
    report.notes.push("source verified unchanged (every file's size and mtime)".into());
    drop(store);
    let _ = crate::app::save_doc(&to.join("import-report.json"), &report.to_json());
    Ok(report)
}
