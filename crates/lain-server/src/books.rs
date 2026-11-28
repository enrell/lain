//! The book routes (D-306): the manifest through `lain.book.manifest@1`,
//! and the bytes the gateway serves itself, like comic pages: sanitized
//! chapters, the book's own resources and rendered PDF pages (through
//! `/api/items/{id}/pages`, in `gateway.rs`). Every route checks the
//! item is visible to the session first (D-156).

use std::path::Path;
use std::sync::Arc;

use axum::Router;
use axum::http::header;
use serde_json::json;

use lain_api::CatalogItem;
use lain_auth::Verified;
use lain_read::book::{self, BookError, Books};

use crate::app::App;
use crate::gateway::{GET, Gate, Reply, Req, route};

/// Book bytes are shown inside a sandboxed frame: even a page opened on
/// its own runs no script and fetches nothing outside this server.
const SANDBOX: &str = "sandbox allow-same-origin; default-src 'none'; img-src 'self' data:; media-src 'self'; style-src 'self' 'unsafe-inline'; font-src 'self'";

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/api/items/{id}/book", route(GET, Gate::User, manifest_get))
        .route("/api/items/{id}/book/chapters/{n}", route(GET, Gate::User, chapter_get))
        .route("/api/items/{id}/book/res/{*path}", route(GET, Gate::User, resource_get))
}

/// The process-wide book cache (the thumbnail module shares it).
pub fn books() -> Arc<Books> {
    lain_read::book::shared()
}

/// The item, when it is a book this session may read.
pub(crate) fn book_item(app: &App, v: &Verified, id: &str) -> Result<CatalogItem, Reply> {
    let it = app
        .visible_item(v, id)
        .ok_or_else(|| Reply::json(404, json!({"error": "unknown item", "code": "not-found"})))?;
    if it.kind != book::KIND_BOOK || book::format(Path::new(&it.file_path)).is_empty() {
        return Err(Reply::json(400, json!({"error": "item is not a book", "code": "not-a-book"})));
    }
    if it.missing {
        return Err(Reply::json(404, json!({"error": "file unavailable", "code": "not-found"})));
    }
    Ok(it)
}

pub(crate) fn book_error(it: &CatalogItem, e: BookError) -> Reply {
    let status = match e.code {
        book::INVALID_BOOK | book::DRM_PROTECTED | book::UNSUPPORTED_BOOK => 422,
        "not-found" => 404,
        "dependency-unavailable" => 404,
        _ => 503,
    };
    if status != 404 {
        tracing::warn!(item = %it.id, code = e.code, err = %e.message, "book unavailable");
    }
    Reply::json(status, json!({"error": e.message, "code": e.code}))
}

fn manifest_get(app: &App, req: &Req, v: &Verified) -> Reply {
    let it = match book_item(app, v, req.param("id")) {
        Ok(it) => it,
        Err(r) => return r,
    };
    match app.registry.call_one(book::CAP_BOOK_MANIFEST, "manifest", json!({"path": it.file_path})) {
        Ok((out, _)) => Reply::ok(out),
        Err(e) => {
            let status = match e.code.as_str() {
                book::INVALID_BOOK | book::DRM_PROTECTED | book::UNSUPPORTED_BOOK => 422,
                "not-found" | "dependency-unavailable" => 404,
                "module-disabled" => 404,
                _ => 503,
            };
            if status == 422 {
                tracing::warn!(item = %it.id, code = %e.code, err = %e.msg, "book unreadable");
            }
            Reply::json(status, json!({"error": e.msg, "code": e.code}))
        }
    }
}

fn res_prefix(id: &str) -> String {
    format!("/api/items/{id}/book/res/")
}

fn chapter_get(app: &App, req: &Req, v: &Verified) -> Reply {
    let it = match book_item(app, v, req.param("id")) {
        Ok(it) => it,
        Err(r) => return r,
    };
    let n = match req.param("n").parse::<usize>() {
        Ok(n) => n,
        Err(_) => return Reply::err(400, "bad chapter"),
    };
    let etag = format!("\"{}-{}-{n}\"", it.size, it.updated_at);
    if req.header(header::IF_NONE_MATCH) == etag {
        return Reply::bytes(304, "text/html", Vec::new()).header(header::ETAG, &etag);
    }
    match books().chapter(Path::new(&it.file_path), n, &res_prefix(&it.id)) {
        Ok(html) => Reply::bytes(200, "text/html; charset=utf-8", html.into_bytes())
            .header(header::CONTENT_SECURITY_POLICY, SANDBOX)
            .header(header::CACHE_CONTROL, "private, max-age=3600")
            .header(header::ETAG, &etag),
        Err(e) => book_error(&it, e),
    }
}

fn resource_get(app: &App, req: &Req, v: &Verified) -> Reply {
    let it = match book_item(app, v, req.param("id")) {
        Ok(it) => it,
        Err(r) => return r,
    };
    let member = req.param("path").trim_start_matches('/').to_string();
    if member.is_empty() {
        return Reply::err(404, "no such resource");
    }
    match books().resource(Path::new(&it.file_path), &member, &res_prefix(&it.id)) {
        Ok((data, mime)) => {
            // Documents are never served as pages of this origin.
            let mime = if mime.contains("html") || mime.contains("xml") && !mime.contains("svg") {
                "text/plain; charset=utf-8".to_string()
            } else {
                mime
            };
            Reply::bytes(200, &mime, data)
                .header(header::CONTENT_SECURITY_POLICY, SANDBOX)
                .header(header::CACHE_CONTROL, "private, max-age=86400")
        }
        Err(e) => book_error(&it, e),
    }
}
