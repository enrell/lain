//! The catalog-population routes (scan, folder picker, item delete,
//! metadata overlays, sidecar subtitles). Thin: the work lives in
//! `lain-ingest`, `lain-metadata` and `lain-subtitle`; these decode,
//! check visibility (D-156) and answer the Go server's JSON.

use std::sync::Arc;

use axum::Router;
use serde_json::{Value, json};

use lain_auth::Verified;
use lain_core::{Error, codes};
use lain_metadata::CAP_METADATA_ENRICH;

use crate::app::App;
use crate::catalog::CAP_CATALOG_WRITE;
use crate::gateway::{DELETE, GET, Gate, POST, Reply, Req, route};

/// The most ids one `GET /api/enrichments` reads (a grid page).
pub const MAX_ENRICH_BATCH: usize = 200;

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/api/library/scan", route(POST, Gate::Admin, scan_start).merge(route(GET, Gate::User, scan_status)))
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

fn scan_start(app: &App, _: &Req, _: &Verified) -> Reply {
    if !app.scanner.start_all("manual") {
        return Reply::err(409, "scan already running");
    }
    Reply::json(202, json!({"state": "running"}))
}

fn scan_status(app: &App, _: &Req, v: &Verified) -> Reply {
    let mut st = app.scanner.status();
    if !v.is_admin() {
        st.stats = st.stats.map(|s| app.public_stats(s));
    }
    Reply::ok(json!(st))
}

fn browse(_: &App, req: &Req, _: &Verified) -> Reply {
    match lain_ingest::browse::browse(req.query("path")) {
        Ok(out) => Reply::ok(json!(out)),
        Err(e) => Reply::err(400, e),
    }
}

// ---- items ----------------------------------------------------------------

/// Removes the item's file (D-073, admin). The row is never deleted: it
/// reads `missing` (D-068), so progress and overlays stay attached and
/// a file back at the same place restores it. Deleting a missing item
/// answers 200 too: the end state is the same.
fn item_delete(app: &App, req: &Req, v: &Verified) -> Reply {
    let id = req.param("id");
    let Some(mut it) = app.visible_item(v, id) else { return Reply::err(404, "unknown item") };
    if it.file_path.is_empty() {
        // A rooted item whose root this machine does not map: there is
        // no file to remove here.
        return Reply::err(409, "the item's root is not mapped on this machine");
    }
    if let Err(e) = std::fs::remove_file(&it.file_path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::error!(item = %id, path = %it.file_path, err = %e, "item file delete failed");
        return Reply::err(500, &format!("remove {}: {e}", it.file_path));
    }
    if let Err(e) = app.registry.call_one(CAP_CATALOG_WRITE, "set_missing", json!({"id": id, "missing": true})) {
        tracing::error!(item = %id, err = %e, "item missing-flag failed after file delete");
        return Reply::err(500, "file removed, but the catalog entry could not be marked missing");
    }
    it.missing = true;
    if let Some(lib) = app.libraries.get(&it.library_id) {
        app.publish_library(&lib, json!({"library_id": lib.id, "action": "item-deleted", "item_id": it.id}));
    }
    Reply::ok(json!(it))
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
        Ok(cues) => Reply::bytes(
            200,
            "text/vtt; charset=utf-8",
            lain_subtitle::webvtt(&cues),
            vec![("cache-control", "private, no-cache")],
        ),
        Err(_) => Reply::json(422, json!({"error": "this subtitle file cannot be read", "code": "unsupported-media"})),
    }
}
