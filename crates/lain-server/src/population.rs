//! The catalog-population routes (scan, folder picker, item delete,
//! metadata overlays, sidecar subtitles). Thin: the work lives in
//! `lain-ingest`, `lain-metadata` and `lain-subtitle`; these decode,
//! check visibility (D-156) and answer the Go server's JSON.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use serde_json::{Value, json};

use lain_api::ItemRemoval;
use lain_auth::Verified;
use lain_core::{Error, codes};
use lain_ingest::scanner::ScanRequest;
use lain_metadata::CAP_METADATA_ENRICH;

use crate::app::App;
use crate::catalog::CAP_CATALOG_WRITE;
use crate::gateway::{DELETE, GET, Gate, POST, Reply, Req, route};

/// The most ids one `GET /api/enrichments` reads (a grid page).
pub const MAX_ENRICH_BATCH: usize = 200;

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/api/library/scan", route(POST, Gate::Manager, scan_start).merge(route(GET, Gate::User, scan_status)))
        .route("/api/library/enrich", route(POST, Gate::Manager, enrich_start))
        .route("/api/browse", route(GET, Gate::Admin, browse))
        .route("/api/items/{id}", route(DELETE, Gate::Admin, item_delete))
        .route(
            "/api/catalog/{id}/enrich",
            route(POST, Gate::Admin, enrich).merge(route(GET, Gate::User, enrich_get)).merge(route(
                DELETE,
                Gate::Admin,
                enrich_delete,
            )),
        )
        .route("/api/enrichments", route(GET, Gate::User, enrichments))
        .route("/api/items/{id}/sidecars", route(GET, Gate::User, sidecars))
        .route("/api/items/{id}/sidecars/{n}", route(GET, Gate::User, sidecar))
}

// ---- scans ----------------------------------------------------------------

/// Starts a scan of every library; while one runs, queues one follow-up
/// behind it (D-286) and says so. Asked again while queued, it stays
/// one follow-up.
fn scan_start(app: &App, _: &Req, _: &Verified) -> Reply {
    match app.scanner.request_all("manual") {
        ScanRequest::Started => Reply::json(202, json!({"state": "running"})),
        ScanRequest::Queued => Reply::json(202, json!({"state": "running", "queued": true})),
    }
}

/// Starts the metadata pass (D-302), or one more behind the running
/// one. `retry` asks again about items no provider knew last time.
fn enrich_start(app: &App, req: &Req, _: &Verified) -> Reply {
    let r: lain_api::EnrichRequest = if req.body.trim_ascii().is_empty() {
        lain_api::EnrichRequest::default()
    } else {
        match req.decode() {
            Ok(r) => r,
            Err(e) => return e,
        }
    };
    let started = app.scanner.request_enrich(r.retry);
    let mut body = json!({"state": "running"});
    if !started {
        body["queued"] = json!(true);
    }
    Reply::json(202, body)
}

fn scan_status(app: &App, _: &Req, v: &Verified) -> Reply {
    let mut st = app.scanner.status();
    // Unreadable roots name libraries and folders: scrubbed for admins too (D-293).
    st.stats = st.stats.map(|s| app.visible_stats(v, s));
    Reply::ok(json!(st))
}

/// The folder picker lists what is on disk, so a library whose items this
/// session may not see (a private one that is locked here) is left out of
/// it: its folder is not listed, and neither it nor anything inside it can
/// be opened, with the answer of a folder that is not there (D-293).
fn browse(app: &App, req: &Req, v: &Verified) -> Reply {
    use lain_ingest::browse::{browse as list, clean};
    let access = app.access(v);
    let closed: Vec<std::path::PathBuf> = app
        .libraries
        .list()
        .unwrap_or_default()
        .iter()
        .filter(|l| !l.path.is_empty() && !crate::privacy::items_visible(l, &app.unlocks, &access, &v.session))
        .map(|l| clean(std::path::Path::new(&l.path)))
        .collect();
    let inside = |p: &std::path::Path| closed.iter().any(|c| p.starts_with(c));
    let asked = req.query("path").trim();
    if !asked.is_empty() && inside(&clean(std::path::Path::new(asked))) {
        return Reply::err(400, "path is not a readable directory");
    }
    match list(asked) {
        Ok(mut out) => {
            if inside(std::path::Path::new(&out.path)) {
                return Reply::err(400, "path is not a readable directory");
            }
            out.dirs.retain(|d| !inside(std::path::Path::new(&d.path)));
            Reply::ok(json!(out))
        }
        Err(e) => Reply::err(400, e),
    }
}

// ---- items ----------------------------------------------------------------

/// Deletes the item's file (D-073, admin) and what only existed for it
/// (D-301): its subtitle sidecars and stills, the folders the deletion
/// left empty, and the torrent copy that still holds its bytes. A
/// monitored title stops wanting it. The row is never deleted: it reads
/// `missing` (D-068), so progress and overlays stay attached and a file
/// back at the same place restores it. The answer is the item with
/// `removal`, what this call did. Deleting a missing item answers 200
/// too: the end state is the same.
fn item_delete(app: &App, req: &Req, v: &Verified) -> Reply {
    let id = req.param("id");
    let Some(mut it) = app.visible_item(v, id) else { return Reply::err(404, "unknown item") };
    if it.file_path.is_empty() {
        // A rooted item whose root this machine does not map: there is
        // no file to remove here.
        return Reply::err(409, "the item's root is not mapped on this machine");
    }
    let path = Path::new(&it.file_path);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let lib = app.libraries.get(&it.library_id);
    let mut removal = ItemRemoval::default();
    let (mut linked, mut file_bytes) = (false, 0);
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            if let Err(e) = std::fs::remove_file(path) {
                tracing::error!(item = %id, path = %it.file_path, err = %e, "item file delete failed");
                return Reply::err(500, &format!("could not delete {name}: {e}"));
            }
            removal.file = true;
            file_bytes = meta.len() as i64;
            removal.freed_bytes = file_bytes;
            linked = hard_links(&meta) > 1;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Not there. With the library's folder away (an unmounted
            // disk) that says nothing about the file: it would be back,
            // undeleted, with the disk (D-287).
            let reachable = lib.as_ref().is_some_and(|l| {
                !l.path.is_empty() && std::fs::read_dir(&l.path).is_ok_and(|mut entries| entries.next().is_some())
            });
            if !it.missing && !reachable {
                return Reply::err(409, "the library's folder cannot be reached, so nothing was deleted");
            }
        }
        Err(e) => {
            tracing::error!(item = %id, path = %it.file_path, err = %e, "item file delete failed");
            return Reply::err(500, &format!("could not delete {name}: {e}"));
        }
    }
    for side in companions(path) {
        let size = std::fs::symlink_metadata(&side).map(|m| m.len() as i64).unwrap_or(0);
        match std::fs::remove_file(&side) {
            Ok(()) => {
                removal.sidecars += 1;
                removal.freed_bytes += size;
            }
            Err(e) => tracing::warn!(item = %id, path = %side.display(), err = %e, "sidecar delete failed"),
        }
    }
    if let Some(lib) = &lib {
        removal.folders = prune_empty_dirs(path, Path::new(&lib.path));
    }
    if let Err(e) = app.registry.call_one(CAP_CATALOG_WRITE, "set_missing", json!({"id": id, "missing": true})) {
        tracing::error!(item = %id, err = %e, "item missing-flag failed after file delete");
        return Reply::err(500, "file removed, but the catalog entry could not be marked missing");
    }
    it.missing = true;
    if let Some(m) = app.acquire.get() {
        let forgotten = m.library_file_deleted(&it);
        removal.downloads = forgotten.downloads;
        removal.not_wanted = forgotten.skipped;
    }
    if linked && removal.downloads == 0 {
        // Another hard link still holds the bytes: nothing was freed.
        removal.still_linked = true;
        removal.freed_bytes -= file_bytes;
    }
    tracing::info!(
        item = %id, file = removal.file, freed = removal.freed_bytes, sidecars = removal.sidecars,
        folders = removal.folders, downloads = removal.downloads, not_wanted = removal.not_wanted,
        still_linked = removal.still_linked, "item deleted"
    );
    it.extra.insert("removal".into(), json!(removal));
    if let Some(lib) = lib {
        app.publish_library(&lib, json!({"library_id": lib.id, "action": "item-deleted", "item_id": it.id}));
    }
    Reply::ok(json!(it))
}

#[cfg(unix)]
fn hard_links(meta: &std::fs::Metadata) -> u64 {
    std::os::unix::fs::MetadataExt::nlink(meta)
}

#[cfg(not(unix))]
fn hard_links(_: &std::fs::Metadata) -> u64 {
    1
}

/// The files that only exist for one media file: its subtitle sidecars
/// (`<name>.<lang>.srt`, D-202) and its own stills (`<name>-poster.*`,
/// `<name>-thumb.*`). By name, in its folder, never a symlink's target.
fn companions(media: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = lain_subtitle::discover(media).into_iter().map(|f| f.path).collect();
    if let (Some(dir), Some(stem)) = (media.parent(), media.file_stem()) {
        let stem = stem.to_string_lossy();
        for suffix in ["-poster", "-thumb"] {
            for ext in ["jpg", "jpeg", "png", "webp"] {
                let p = dir.join(format!("{stem}{suffix}.{ext}"));
                if std::fs::symlink_metadata(&p).is_ok_and(|m| m.is_file()) {
                    out.push(p);
                }
            }
        }
    }
    out
}

/// Removes the folders a deleted file left empty, from its own up to,
/// never including, the library's. `remove_dir` only removes an empty
/// folder, so anything else in one keeps it. The library's folder is
/// never left empty: an empty one reads as an unmounted disk (D-287).
fn prune_empty_dirs(file: &Path, root: &Path) -> i64 {
    let root = lain_ingest::browse::clean(root);
    if root.as_os_str().is_empty() {
        return 0;
    }
    let mut removed = 0;
    let mut dir = file.parent().map(lain_ingest::browse::clean);
    while let Some(d) = dir {
        if d == root || !d.starts_with(&root) {
            break;
        }
        let last_of_root = d.parent() == Some(root.as_path())
            && std::fs::read_dir(&root).map(|entries| entries.take(2).count() < 2).unwrap_or(true);
        if last_of_root || std::fs::remove_dir(&d).is_err() {
            break;
        }
        removed += 1;
        dir = d.parent().map(Path::to_path_buf);
    }
    removed
}

// ---- metadata overlays ----------------------------------------------------

fn enrich_err(e: &Error) -> Reply {
    let status = match e.code.as_str() {
        codes::NOT_FOUND => 404,
        codes::DEPENDENCY_UNAVAILABLE => 503,
        _ => 500,
    };
    Reply::err(status, &e.to_string())
}

fn enrich(app: &App, req: &Req, v: &Verified) -> Reply {
    let Some(item) = app.visible_item(v, req.param("id")) else { return Reply::err(404, "unknown item") };
    let input = json!({"item": item, "only": req.query("provider")});
    match app.registry.call_one(CAP_METADATA_ENRICH, "enrich", input) {
        Ok((out, _)) => Reply::ok(out),
        Err(e) => enrich_err(&e),
    }
}

fn enrich_get(app: &App, req: &Req, _: &Verified) -> Reply {
    match app.registry.call_one(CAP_METADATA_ENRICH, "get", json!({"item_id": req.param("id")})) {
        Ok((out, _)) => Reply::ok(out),
        Err(e) => enrich_err(&e),
    }
}

fn enrich_delete(app: &App, req: &Req, _: &Verified) -> Reply {
    match app.registry.call_one(CAP_METADATA_ENRICH, "delete", json!({"item_id": req.param("id")})) {
        Ok(_) => Reply::ok(json!({"status": "ok"})),
        Err(e) => enrich_err(&e),
    }
}

/// The overlays that exist for a page of item ids, in one read. Items
/// of libraries this session may not see are left out (D-156).
fn enrichments(app: &App, req: &Req, v: &Verified) -> Reply {
    let raw = req.query("ids").trim();
    if raw.is_empty() {
        return Reply::err(400, "ids required");
    }
    let parts: Vec<&str> = raw.split(',').collect();
    if parts.len() > MAX_ENRICH_BATCH {
        return Reply::err(400, &format!("too many ids (max {MAX_ENRICH_BATCH})"));
    }
    let hidden = app.hidden_libraries(v);
    let ids: Vec<String> = parts
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .filter(|id| hidden.is_empty() || app.visible_item(v, id).is_some() || !app.item_exists(id))
        .map(str::to_string)
        .collect();
    match app.registry.call_one(CAP_METADATA_ENRICH, "batch", json!({"ids": ids})) {
        Ok((out, _)) => Reply::ok(json!({"items": out.get("items").cloned().unwrap_or(Value::Array(Vec::new()))})),
        Err(e) => enrich_err(&e),
    }
}

// ---- sidecar subtitles (A-28/A-29) ----------------------------------------

fn sidecars(app: &App, req: &Req, v: &Verified) -> Reply {
    let Some(it) = app.visible_item(v, req.param("id")) else { return Reply::err(404, "unknown item") };
    let list: Vec<_> = if it.file_path.is_empty() {
        Vec::new()
    } else {
        lain_subtitle::discover(std::path::Path::new(&it.file_path)).into_iter().map(|f| f.view).collect()
    };
    Reply::ok(json!({"sidecars": list}))
}

/// Go's `strconv.Atoi` on an index (a sign is allowed).
fn atoi(s: &str) -> Option<i64> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// One sidecar, as WebVTT, by index (never by path).
fn sidecar(app: &App, req: &Req, v: &Verified) -> Reply {
    let Some(it) = app.visible_item(v, req.param("id")) else { return Reply::err(404, "unknown item") };
    let list =
        if it.file_path.is_empty() { Vec::new() } else { lain_subtitle::discover(std::path::Path::new(&it.file_path)) };
    let Some(found) = atoi(req.param("n")).filter(|n| *n >= 0).and_then(|n| list.get(n as usize)) else {
        return Reply::err(404, "no such sidecar");
    };
    let file = match std::fs::File::open(&found.path) {
        Ok(f) => f,
        Err(_) => return Reply::err(404, "no such sidecar"),
    };
    let mut raw = Vec::new();
    use std::io::Read;
    if file.take(lain_subtitle::MAX_BYTES as u64 + 1).read_to_end(&mut raw).is_err() {
        return Reply::err(500, "could not read the sidecar");
    }
    match lain_subtitle::parse(&raw) {
        Ok(cues) => Reply::bytes(200, "text/vtt; charset=utf-8", lain_subtitle::webvtt(&cues))
            .header(axum::http::header::CACHE_CONTROL, "private, no-cache"),
        Err(_) => Reply::json(422, json!({"error": "this subtitle file cannot be read", "code": "unsupported-media"})),
    }
}
