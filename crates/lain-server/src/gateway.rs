//! The HTTP boundary (axum). Handlers are thin: they authenticate,
//! decode, call auth, the registry or a module crate, and answer the Go
//! server's JSON (`{"error": msg}` on failure, a trailing newline like
//! Go's encoder) or bytes (pages, avatars, backups). Store and Argon2
//! work runs on the blocking pool.
//!
//! Routes a module owns answer `module-disabled` while it is off. What
//! no route takes falls through to the web SPA (`webui.rs`).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Path, Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodFilter, MethodRouter, on};
use serde::Deserialize;
use serde_json::{Value, json};

use lain_admin::{avatars, backup, theme};
use lain_auth::{
    AVATAR_UPLOAD, AuthError, MASCOTS, PlaybackPolicy, PreferencesPatch, ProfilePatch, ROLE_ADMIN, ROLE_USER, Verified,
};
use lain_core::codes;

use crate::app::App;
use crate::catalog::{CAP_CATALOG_READ, CAP_CATALOG_WRITE, CAP_USER_PROGRESS};
use crate::contracts::{CatalogItem, CatalogPage, Library, PageParams, Progress};
use crate::events::{SseBody, SubInfo};
use crate::files::FileBody;
use crate::libraries::LibraryPatch;
use crate::privacy;

/// A JSON answer, or a prepared response for bytes.
pub struct Reply {
    status: u16,
    body: Value,
    headers: Vec<(HeaderName, HeaderValue)>,
    raw: Option<Box<Response>>,
}

impl Reply {
    pub fn json(status: u16, body: Value) -> Reply {
        Reply { status, body, headers: Vec::new(), raw: None }
    }
    /// Bytes with their content type.
    pub fn bytes(status: u16, mime: &str, body: Vec<u8>) -> Reply {
        let mut r = Response::new(Body::from(body));
        *r.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        if let Ok(v) = HeaderValue::from_str(mime) {
            r.headers_mut().insert(header::CONTENT_TYPE, v);
        }
        Reply::response(r)
    }
    pub fn response(r: Response) -> Reply {
        Reply { status: r.status().as_u16(), body: Value::Null, headers: Vec::new(), raw: Some(Box::new(r)) }
    }
    /// Adds a response header (dropped if the value is not valid).
    pub fn header(mut self, name: HeaderName, value: &str) -> Reply {
        if let Ok(v) = HeaderValue::from_str(value) {
            self.headers.push((name, v));
        }
        self
    }
    pub fn ok(body: Value) -> Reply {
        Reply::json(200, body)
    }
    pub fn err(status: u16, msg: &str) -> Reply {
        Reply::json(status, json!({"error": msg}))
    }
}

impl IntoResponse for Reply {
    fn into_response(self) -> Response {
        let mut r = match self.raw {
            Some(r) => *r,
            None => {
                let mut raw = serde_json::to_vec(&self.body).unwrap_or_else(|_| b"{}".to_vec());
                raw.push(b'\n');
                let mut r = Response::new(Body::from(raw));
                *r.status_mut() = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
                r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
                r
            }
        };
        for (k, v) in self.headers {
            r.headers_mut().insert(k, v);
        }
        r
    }
}

/// One request as handlers see it.
pub struct Req {
    pub query: Vec<(String, String)>,
    pub authorization: Option<String>,
    pub body: Bytes,
    pub params: BTreeMap<String, String>,
    pub headers: HeaderMap,
}

impl Req {
    /// A request header as text (`""` when absent or not text).
    pub fn header(&self, name: HeaderName) -> &str {
        self.headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or("")
    }

    pub fn query(&self, key: &str) -> &str {
        self.query.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()).unwrap_or("")
    }

    pub(crate) fn param(&self, key: &str) -> &str {
        self.params.get(key).map(String::as_str).unwrap_or("")
    }

    /// Bearer header first, else `?token=` (the media-element fallback).
    pub(crate) fn token(&self) -> Option<&str> {
        match self.authorization.as_deref().and_then(|h| h.strip_prefix("Bearer ")) {
            Some(t) => Some(t),
            None => Some(self.query("token")),
        }
        .filter(|t| !t.is_empty())
    }

    /// Decodes a JSON body like Go's `json.Decoder`: the first value,
    /// which must be an object (or `null`); `null` fields are no-ops.
    pub(crate) fn decode<T: for<'de> Deserialize<'de>>(&self) -> Result<T, Reply> {
        let invalid = || Reply::err(400, "invalid json");
        let mut it = serde_json::Deserializer::from_slice(&self.body).into_iter::<Value>();
        let v = match it.next() {
            Some(Ok(v)) => v,
            _ => return Err(invalid()),
        };
        let v = match v {
            Value::Null => json!({}),
            Value::Object(_) => strip_nulls(v),
            _ => return Err(invalid()),
        };
        serde_json::from_value(v).map_err(|_| invalid())
    }
}

fn strip_nulls(v: Value) -> Value {
    match v {
        Value::Object(o) => {
            Value::Object(o.into_iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k, strip_nulls(v))).collect())
        }
        other => other,
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16) {
                    Ok(x) => {
                        out.push(x);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parses a query string (form encoding).
pub fn parse_query(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect()
}

fn atoi(s: &str) -> i64 {
    s.parse().unwrap_or(0)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Gate {
    Public,
    User,
    Admin,
}

pub(crate) type Handler = fn(&App, &Req, &Verified) -> Reply;

pub(crate) fn authorize(app: &App, req: &Req, gate: Gate) -> Result<Verified, Reply> {
    if gate == Gate::Public {
        return Ok(Verified { user_id: String::new(), role: String::new(), session: String::new(), expires_at: 0 });
    }
    let v = req.token().and_then(|t| app.verify(t)).ok_or_else(|| Reply::err(401, "unauthorized"))?;
    if gate == Gate::Admin && v.role != ROLE_ADMIN {
        return Err(Reply::err(403, "admin required"));
    }
    Ok(v)
}

pub(crate) fn route(filter: MethodFilter, gate: Gate, f: Handler) -> MethodRouter<Arc<App>> {
    on(
        filter,
        move |State(app): State<Arc<App>>,
              params: Option<Path<BTreeMap<String, String>>>,
              uri: Uri,
              headers: HeaderMap,
              body: Bytes| async move {
            let req = Req {
                query: parse_query(uri.query().unwrap_or("")),
                authorization: headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).map(str::to_string),
                body,
                params: params.map(|p| p.0).unwrap_or_default(),
                headers,
            };
            let out = tokio::task::spawn_blocking(move || match authorize(&app, &req, gate) {
                Ok(v) => f(&app, &req, &v),
                Err(r) => r,
            })
            .await;
            out.unwrap_or_else(|_| Reply::err(500, "internal error")).into_response()
        },
    )
}

pub(crate) const GET: MethodFilter = MethodFilter::GET;
pub(crate) const POST: MethodFilter = MethodFilter::POST;
pub(crate) const PUT: MethodFilter = MethodFilter::PUT;
pub(crate) const PATCH: MethodFilter = MethodFilter::PATCH;
pub(crate) const DELETE: MethodFilter = MethodFilter::DELETE;

/// The router over an opened [`App`].
pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/health", route(GET, Gate::Public, health))
        .route("/api/health", route(GET, Gate::Public, health))
        .route("/api/setup/status", route(GET, Gate::Public, setup_status))
        .route("/api/setup", route(POST, Gate::Public, setup))
        .route("/api/auth/login", route(POST, Gate::Public, login))
        .route("/api/me", route(GET, Gate::User, me))
        .route("/api/me/preferences", route(PATCH, Gate::User, me_preferences))
        .route("/api/me/password", route(PATCH, Gate::User, me_password))
        .route("/api/me/profile", route(PATCH, Gate::User, me_profile))
        .route(
            "/api/me/avatar",
            route(PUT, Gate::User, avatar_put)
                .merge(route(DELETE, Gate::User, avatar_delete))
                // Go reads 2 MiB + 1 to answer its own 413.
                .layer(DefaultBodyLimit::max(avatars::MAX_BYTES + 1)),
        )
        .route("/api/users/{id}/avatar", route(GET, Gate::User, avatar_get))
        .route("/api/theme", route(GET, Gate::Public, theme_get))
        .route("/api/admin/backup", route(GET, Gate::Admin, backup_get))
        .route("/api/items/{id}/pages", route(GET, Gate::User, pages_get))
        .route("/api/items/{id}/pages/{n}", route(GET, Gate::User, page_get))
        .route("/api/me/continue", route(GET, Gate::User, me_continue))
        .route("/api/profile/mascots", route(GET, Gate::User, mascots))
        .route("/api/users", route(GET, Gate::Admin, users_list).merge(route(POST, Gate::Admin, users_create)))
        .route("/api/users/{id}", route(PATCH, Gate::Admin, users_patch).merge(route(DELETE, Gate::Admin, users_delete)))
        .route(
            "/api/libraries",
            route(GET, Gate::User, libraries_list).merge(route(POST, Gate::Admin, libraries_create)),
        )
        .route(
            "/api/libraries/{id}",
            route(DELETE, Gate::Admin, libraries_delete).merge(route(PATCH, Gate::Admin, libraries_patch)),
        )
        .route("/api/libraries/{id}/unlock", route(POST, Gate::User, libraries_unlock))
        .route("/api/libraries/{id}/lock", route(POST, Gate::User, libraries_lock))
        .route(
            "/api/me/pin",
            route(GET, Gate::User, pin_get).merge(route(PUT, Gate::User, pin_put)).merge(route(
                DELETE,
                Gate::User,
                pin_delete,
            )),
        )
        .route("/api/roots", route(GET, Gate::Admin, roots_list))
        .route("/api/search", route(GET, Gate::User, search))
        .route("/api/catalog", route(GET, Gate::User, catalog_list))
        .route("/api/catalog/{id}", route(GET, Gate::User, catalog_get))
        .route("/api/catalog/{id}/episodes", route(GET, Gate::User, catalog_episodes))
        .route(
            "/api/items/{id}/progress",
            route(GET, Gate::User, progress_get).merge(route(PUT, Gate::User, progress_put)),
        )
        .route("/api/plugins", route(GET, Gate::Admin, plugins))
        .route("/api/plugins/swap", route(POST, Gate::Admin, plugins_swap))
        .route("/api/plugins/withdraw", route(POST, Gate::Admin, plugins_withdraw))
        .route("/api/modules", route(GET, Gate::User, modules_list))
        .route("/api/events", axum::routing::get(events))
        .merge(crate::population::routes())
        .merge(crate::watch::routes())
        .merge(crate::lists::routes())
        .merge(crate::social::routes())
        .merge(crate::acquire::routes())
        .merge(crate::translate::routes())
        .merge(crate::integration::routes())
        .fallback(fallback)
        .method_not_allowed_fallback(fallback)
        .layer(middleware::from_fn_with_state(app.clone(), privacy_gate))
        .layer(middleware::from_fn_with_state(app.clone(), module_gate))
        .layer(middleware::from_fn(access_log))
        .with_state(app)
}

// ---- liveness and setup ---------------------------------------------------

fn health(app: &App, _: &Req, _: &Verified) -> Reply {
    Reply::ok(json!({"status": "ok", "version": app.version}))
}

fn setup_status(app: &App, _: &Req, _: &Verified) -> Reply {
    Reply::ok(json!({"setup_required": !app.auth().has_users()}))
}

#[derive(Deserialize)]
struct Credentials {
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
}

fn setup(app: &App, req: &Req, _: &Verified) -> Reply {
    let c: Credentials = match req.decode() {
        Ok(c) => c,
        Err(r) => return r,
    };
    match app.auth().setup(&c.username, &c.password) {
        Ok(u) => Reply::json(201, json!(u)),
        Err(e) => Reply::err(400, &e.to_string()),
    }
}

fn login(app: &App, req: &Req, _: &Verified) -> Reply {
    let c: Credentials = match req.decode() {
        Ok(c) => c,
        Err(r) => return r,
    };
    match app.auth().login(&c.username, &c.password) {
        Ok(token) => Reply::ok(json!({"token": token})),
        Err(e) => {
            // Username only: passwords never enter a log line.
            tracing::warn!(username = %c.username, "login failed");
            Reply::err(401, &e.to_string())
        }
    }
}

// ---- the account ----------------------------------------------------------

fn me(app: &App, _: &Req, v: &Verified) -> Reply {
    match app.auth().get_public(&v.user_id) {
        Some(u) => Reply::ok(json!(u)),
        None => Reply::err(404, "unknown user"),
    }
}

/// D-071's one language or D-280's split preferences, any subset; the
/// account as it then is.
fn me_preferences(app: &App, req: &Req, v: &Verified) -> Reply {
    let p: PreferencesPatch = match req.decode() {
        Ok(p) => p,
        Err(r) => return r,
    };
    // An empty patch keeps D-071's answer: older clients read it.
    if p.is_empty() {
        return Reply::err(400, "preferred_language is required");
    }
    match app.auth().update_preferences(&v.user_id, &p) {
        Ok(u) => Reply::ok(json!(u)),
        Err(AuthError::UnknownUser) => Reply::err(404, "unknown user"),
        Err(e) => Reply::err(400, &e.to_string()),
    }
}

fn me_password(app: &App, req: &Req, v: &Verified) -> Reply {
    #[derive(Deserialize)]
    struct In {
        #[serde(default)]
        old: String,
        #[serde(default)]
        new: String,
    }
    let i: In = match req.decode() {
        Ok(i) => i,
        Err(r) => return r,
    };
    match app.auth().change_password(&v.user_id, &i.old, &i.new) {
        Ok(()) => Reply::ok(json!({"status": "ok"})),
        // A stable code, so a client tells "wrong current password" from
        // a rejected new one without reading the message.
        Err(e @ AuthError::WrongPassword) => Reply::json(400, json!({"error": e.to_string(), "code": "wrong-password"})),
        Err(e) => Reply::err(400, &e.to_string()),
    }
}

fn me_profile(app: &App, req: &Req, v: &Verified) -> Reply {
    let p: ProfilePatch = match req.decode() {
        Ok(p) => p,
        Err(r) => return r,
    };
    match app.auth().update_profile(&v.user_id, &p) {
        Ok(u) => {
            // Picking a mascot or clearing the avatar retires an upload.
            if p.mascot.is_some() {
                app.avatars.remove(&v.user_id);
            }
            Reply::ok(json!(u))
        }
        Err(e) => Reply::err(400, &e.to_string()),
    }
}

// ---- avatars (D-086, D-199) -------------------------------------------------

fn avatar_put(app: &App, req: &Req, v: &Verified) -> Reply {
    if let Err(e) = app.avatars.save(&v.user_id, &req.body) {
        return Reply::err(e.status, e.msg);
    }
    match app.auth().set_uploaded_avatar(&v.user_id) {
        Ok(u) => Reply::ok(json!(u)),
        Err(e) => Reply::err(400, &e.to_string()),
    }
}

fn avatar_delete(app: &App, _: &Req, v: &Verified) -> Reply {
    let clear = ProfilePatch { mascot: Some(String::new()), ..ProfilePatch::default() };
    match app.auth().update_profile(&v.user_id, &clear) {
        Ok(u) => {
            app.avatars.remove(&v.user_id);
            Reply::ok(json!(u))
        }
        Err(e) => Reply::err(400, &e.to_string()),
    }
}

/// Any signed-in user may read an uploaded picture; `<img>` sends no
/// headers, so `?token=` works as for streams.
fn avatar_get(app: &App, req: &Req, _: &Verified) -> Reply {
    let id = req.param("id");
    let none = || Reply::err(404, "no avatar");
    match app.auth().get_public(id) {
        Some(u) if u.profile.avatar.kind == AVATAR_UPLOAD => {}
        _ => return none(),
    }
    let Some((body, mime)) = app.avatars.read(id) else { return none() };
    Reply::bytes(200, mime, body)
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        // The URL carries the avatar version: a new picture is a new URL.
        .header(header::CACHE_CONTROL, "private, max-age=31536000, immutable")
}

// ---- theme, integrations, backup (core administration) ---------------------

/// The web UI palette (`lain.ui.theme@1`), relayed as the provider
/// derives it; the SPA keeps its compiled-in defaults on a 503.
fn theme_get(app: &App, _: &Req, _: &Verified) -> Reply {
    let path = app.theme_path.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let out = match app.registry.call_one(theme::CAP_UI_THEME, "derive", json!({"colors_path": path})) {
        Ok((out, _)) if serde_json::from_value::<theme::Palette>(out.clone()).is_ok() => Reply::ok(out),
        Ok(_) => Reply::err(503, "theme provider returned a bad shape"),
        Err(e) => Reply::err(503, &e.to_string()),
    };
    out.header(header::CACHE_CONTROL, "no-store")
}

/// `scheme://host` as the client reached this server (OAuth callbacks).
pub(crate) fn base_url(req: &Req) -> String {
    let scheme = match req.header(HeaderName::from_static("x-forwarded-proto")) {
        p @ ("http" | "https") => p,
        _ => "http",
    };
    format!("{scheme}://{}", req.header(header::HOST))
}

/// A backup artifact: the core snapshots the live store and the data
/// dir's documents into a scratch dir, `lain.backup.create@1` packages
/// them, and the artifact streams from disk; the scratch dir goes with
/// the response.
fn backup_get(app: &App, _: &Req, _: &Verified) -> Reply {
    let tmp = match backup::TempDir::new("backup") {
        Ok(t) => t,
        Err(e) => return Reply::err(500, &e.to_string()),
    };
    let comp = json!(app.registry.composition());
    let input = match backup::prepare(&app.store, &app.data_dir, comp, &app.version, tmp.path()) {
        Ok(i) => i,
        Err(e) => return Reply::err(500, &e),
    };
    let out = match app.registry.call_one(backup::CAP_BACKUP_CREATE, "create", json!(input)) {
        Ok((out, _)) => out,
        Err(e) => return Reply::err(503, &e.to_string()),
    };
    let artifact = match serde_json::from_value::<backup::CreateOutput>(out) {
        Ok(a) if !a.path.is_empty() => a,
        _ => return Reply::err(500, "backup provider returned a bad shape"),
    };
    let body = match FileBody::open(std::path::Path::new(&artifact.path), Some(tmp)) {
        Ok(b) => b,
        Err(e) => return Reply::err(500, &e.to_string()),
    };
    let len = body.len();
    tracing::info!(bytes = len, file = %artifact.filename, "backup served");
    let filename: String = artifact.filename.chars().filter(|c| !matches!(c, '"' | '\\' | '\r' | '\n')).collect();
    let r = Response::new(Body::new(body));
    Reply::response(r)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_DISPOSITION, &format!("attachment; filename=\"{filename}\""))
        .header(header::CONTENT_LENGTH, &len.to_string())
}

// ---- read (D-085, D-195) ------------------------------------------------------

/// Resolves an item to its page index, or the reply for why not.
fn pages_for(app: &App, v: &Verified, id: &str) -> Result<(CatalogItem, lain_read::PageIndex), Reply> {
    let it = app.visible_item(v, id).ok_or_else(|| Reply::err(404, "unknown item"))?;
    if it.kind != lain_read::KIND_COMIC && it.kind != lain_read::KIND_MANGA {
        return Err(Reply::err(400, "item is not readable"));
    }
    if it.missing {
        return Err(Reply::err(404, "file unavailable"));
    }
    let out = match app.registry.call_one(lain_read::CAP_COMIC_PAGES, "pages", json!({"path": it.file_path})) {
        Ok((out, _)) => out,
        Err(e) => {
            let status = match e.code.as_str() {
                lain_read::archive::INVALID_ARCHIVE | lain_read::archive::UNSUPPORTED_FORMAT => 422,
                _ => 503,
            };
            tracing::warn!(item = %it.id, err = %e, "comic pages failed");
            return Err(Reply::err(status, &e.to_string()));
        }
    };
    match serde_json::from_value(out) {
        Ok(pages) => Ok((it, pages)),
        Err(_) => Err(Reply::err(500, "bad pages result")),
    }
}

fn pages_get(app: &App, req: &Req, v: &Verified) -> Reply {
    match pages_for(app, v, req.param("id")) {
        Ok((it, idx)) => Reply::ok(json!(lain_api::ComicPages {
            direction: lain_read::default_direction(&it.kind, idx.info.as_ref()),
            kind: it.kind,
            format: idx.format,
            pages: idx.pages.iter().map(lain_read::IndexedPage::public).collect(),
            info: idx.info,
        })),
        Err(r) => r,
    }
}

/// One page's bytes, by index (`<img>` sends `?token=`).
fn page_get(app: &App, req: &Req, v: &Verified) -> Reply {
    let n = match req.param("n").parse::<i64>() {
        Ok(n) if n >= 0 => n as usize,
        _ => return Reply::err(400, "bad page"),
    };
    let (it, idx) = match pages_for(app, v, req.param("id")) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let Some(pg) = idx.pages.get(n) else { return Reply::err(404, "no such page") };
    let etag = format!("\"{}-{}-{n}\"", it.size, pg.size);
    if req.header(header::IF_NONE_MATCH) == etag {
        let mut r = Response::new(Body::empty());
        *r.status_mut() = StatusCode::NOT_MODIFIED;
        return Reply::response(r).header(header::ETAG, &etag);
    }
    match lain_read::archive::open_page(std::path::Path::new(&it.file_path), &pg.name) {
        Ok(b) => Reply::bytes(200, &pg.mime, b)
            .header(header::CACHE_CONTROL, "private, max-age=86400")
            .header(header::ETAG, &etag),
        Err(e) => {
            tracing::warn!(item = %it.id, page = n, err = %e, "page unavailable");
            Reply::err(404, "page unavailable")
        }
    }
}

fn me_continue(app: &App, _: &Req, v: &Verified) -> Reply {
    match app.registry.call_one(CAP_USER_PROGRESS, "list", json!({"user_id": v.user_id})) {
        Ok((out, _)) => {
            // Progress on items of a locked private library is left out
            // (D-156); progress on items the catalog does not know stays.
            let hidden = app.hidden_libraries(v);
            if hidden.is_empty() {
                return Reply::ok(out);
            }
            let list: Vec<Value> = out
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|p| match item_library(app, p["item_id"].as_str().unwrap_or("")) {
                    Some(lib) => !hidden.contains(&lib),
                    None => true,
                })
                .collect();
            Reply::ok(json!(list))
        }
        Err(e) => Reply::err(503, &e.to_string()),
    }
}

fn item_library(app: &App, item_id: &str) -> Option<String> {
    let (out, _) = app.registry.call_one(CAP_CATALOG_READ, "get", json!({"id": item_id})).ok()?;
    out["library_id"].as_str().map(str::to_string)
}

fn pin_get(app: &App, _: &Req, v: &Verified) -> Reply {
    Reply::ok(json!({"set": app.auth().has_pin(&v.user_id)}))
}

#[derive(Deserialize)]
struct PinIn {
    #[serde(default)]
    pin: String,
    #[serde(default)]
    password: String,
}

fn pin_put(app: &App, req: &Req, v: &Verified) -> Reply {
    let i: PinIn = match req.decode() {
        Ok(i) => i,
        Err(r) => return r,
    };
    match app.auth().set_pin(&v.user_id, &i.pin, &i.password) {
        Ok(()) => Reply::ok(json!({"set": true})),
        Err(e) => Reply::err(400, &e.to_string()),
    }
}

fn pin_delete(app: &App, req: &Req, v: &Verified) -> Reply {
    let i: PinIn = match req.decode() {
        Ok(i) => i,
        Err(r) => return r,
    };
    match app.auth().clear_pin(&v.user_id, &i.password) {
        Ok(()) => Reply::ok(json!({"set": false})),
        Err(e) => Reply::err(400, &e.to_string()),
    }
}

fn mascots(_: &App, _: &Req, _: &Verified) -> Reply {
    Reply::ok(json!({"mascots": MASCOTS}))
}

// ---- users (admin) --------------------------------------------------------

fn users_list(app: &App, _: &Req, _: &Verified) -> Reply {
    match app.auth().list() {
        Ok(list) => Reply::ok(json!(list)),
        Err(e) => Reply::err(500, &e.to_string()),
    }
}

fn users_create(app: &App, req: &Req, _: &Verified) -> Reply {
    #[derive(Deserialize)]
    struct In {
        #[serde(default)]
        username: String,
        #[serde(default)]
        password: String,
        #[serde(default)]
        role: String,
    }
    let i: In = match req.decode() {
        Ok(i) => i,
        Err(r) => return r,
    };
    let role = if i.role.is_empty() { ROLE_USER } else { i.role.as_str() };
    match app.auth().create(&i.username, &i.password, role) {
        Ok(u) => Reply::json(201, json!(u)),
        Err(e) => Reply::err(400, &e.to_string()),
    }
}

/// Admin user management: any subset of disable, role, password reset
/// and playback limits, applied in that fixed order.
fn users_patch(app: &App, req: &Req, v: &Verified) -> Reply {
    #[derive(Deserialize)]
    struct In {
        disabled: Option<bool>,
        role: Option<String>,
        password: Option<String>,
        playback: Option<PlaybackPolicy>,
    }
    let id = req.param("id");
    let i: In = match req.decode() {
        Ok(i) => i,
        Err(r) => return r,
    };
    let auth = app.auth();
    if id == v.user_id && i.disabled == Some(true) {
        return Reply::err(400, "cannot disable yourself");
    }
    if let Some(d) = i.disabled
        && let Err(e) = auth.set_disabled(id, d)
    {
        return Reply::err(404, &e.to_string());
    }
    if let Some(role) = &i.role {
        if id == v.user_id && role != ROLE_ADMIN {
            return Reply::err(400, "cannot demote yourself");
        }
        if let Err(e) = auth.set_role(id, role) {
            return Reply::err(400, &e.to_string());
        }
    }
    if let Some(p) = &i.password
        && let Err(e) = auth.admin_reset(id, p)
    {
        return Reply::err(400, &e.to_string());
    }
    if let Some(p) = i.playback
        && let Err(e) = auth.set_playback(id, p)
    {
        return Reply::err(400, &e.to_string());
    }
    match auth.get_public(id) {
        Some(u) => Reply::ok(json!(u)),
        None => Reply::err(404, "unknown user"),
    }
}

/// Deletes an account and what it owned: its progress, tracking lists
/// and linked accounts, social rows (ratings, comments, collections,
/// friends, notifications) and avatar. A private library it owned
/// passes to the admin deleting it, so nothing is left without an
/// owner; its membership elsewhere ends. The account's id derives from
/// its name, so a row left behind would belong to the next account of
/// that name: a cleanup that fails is an error, not a warning.
fn users_delete(app: &App, req: &Req, v: &Verified) -> Reply {
    let id = req.param("id");
    if id == v.user_id {
        return Reply::err(400, "cannot delete yourself");
    }
    let gone = match app.auth().delete(id) {
        Ok(u) => u,
        Err(AuthError::UnknownUser) => return Reply::err(404, "unknown user"),
        Err(e) => return Reply::err(400, &e.to_string()),
    };
    let mut failed = Vec::new();
    let mut removed = 0i64;
    let forget = [
        (CAP_USER_PROGRESS, "forget-user"),
        (lain_lists::CAP_LIST_ACCOUNT, "forget-user"),
        (lain_social::CAP_SOCIAL_GRAPH, "forget-user"),
    ];
    for (cap, op) in forget {
        match app.registry.call_one(cap, op, json!({"user_id": id})) {
            Ok((out, _)) => removed += out.as_i64().or_else(|| out["removed"].as_i64()).unwrap_or(0),
            Err(e) => {
                tracing::error!(user = %id, capability = cap, err = %e, "deleted account's data not removed");
                failed.push(cap);
            }
        }
    }
    let auth = app.auth();
    let mut reassigned = Vec::new();
    for lib in app.libraries.list().unwrap_or_default() {
        if !lib.is_member(id) {
            continue;
        }
        let members: Vec<String> = lib.members.iter().filter(|m| *m != id).cloned().collect();
        let owner = if lib.owner == id { v.user_id.clone() } else { lib.owner.clone() };
        let patch = LibraryPatch { owner: Some(owner), members: Some(members), ..LibraryPatch::default() };
        match app.libraries.update(&lib.id, &patch, &v.user_id, &|u| auth.get_public(u).is_some()) {
            Ok(_) if lib.owner == id => reassigned.push(lib.id.clone()),
            Ok(_) => {}
            Err(e) => {
                tracing::error!(user = %id, library = %lib.id, err = %e, "deleted account's library membership not removed");
                failed.push("libraries");
            }
        }
    }
    app.avatars.remove(id);
    tracing::info!(user = %id, username = %gone.username, rows = removed, "account deleted");
    if !failed.is_empty() {
        return Reply::json(
            500,
            json!({"error": format!("account deleted, but some of its data was not removed: {}", failed.join(", "))}),
        );
    }
    Reply::ok(json!({"status": "ok", "rows_removed": removed, "libraries_reassigned": reassigned}))
}

// ---- libraries ------------------------------------------------------------

/// One library as this viewer sees it: private ones carry `locked`.
fn library_view(app: &App, lib: &Library, v: &Verified) -> Value {
    let mut out = json!(lib);
    if lib.private {
        out["locked"] = json!(!privacy::items_visible(lib, &app.unlocks, &v.user_id, &v.session));
    }
    out
}

fn libraries_list(app: &App, _: &Req, v: &Verified) -> Reply {
    match app.libraries.list() {
        Ok(l) => Reply::ok(json!(
            l.iter()
                .filter(|lib| privacy::record_visible(lib, &v.user_id, v.is_admin()))
                .map(|lib| library_view(app, lib, v))
                .collect::<Vec<_>>()
        )),
        Err(e) => Reply::err(500, &e.to_string()),
    }
}

fn libraries_patch(app: &App, req: &Req, v: &Verified) -> Reply {
    let p: LibraryPatch = match req.decode() {
        Ok(p) => p,
        Err(r) => return r,
    };
    let auth = app.auth();
    let before = app.libraries.get(req.param("id"));
    match app.libraries.update(req.param("id"), &p, &v.user_id, &|u| auth.get_public(u).is_some()) {
        Ok(lib) => {
            let data = json!({"library_id": lib.id, "action": "updated"});
            // Who could see it before (made private) and after (made public).
            if let Some(b) = before.filter(|b| b.private && !lib.private) {
                app.publish_library(&b, data.clone());
                let public = Library { private: false, ..lib.clone() };
                app.events.publish_where("library-changed", data, &|s: &SubInfo| {
                    !privacy::record_visible(&b, &s.user_id, s.admin)
                        && privacy::record_visible(&public, &s.user_id, s.admin)
                });
            } else {
                app.publish_library(&lib, data);
            }
            Reply::ok(library_view(app, &lib, v))
        }
        Err(e) if e == "unknown library" => Reply::err(404, &e),
        Err(e) => Reply::err(400, &e),
    }
}

fn libraries_unlock(app: &App, req: &Req, v: &Verified) -> Reply {
    #[derive(Deserialize)]
    struct In {
        pin: Option<String>,
        password: Option<String>,
    }
    let i: In = match req.decode() {
        Ok(i) => i,
        Err(r) => return r,
    };
    let Some(lib) = app.libraries.get(req.param("id")).filter(|l| privacy::record_visible(l, &v.user_id, v.is_admin()))
    else {
        return Reply::err(404, "unknown library");
    };
    if !lib.private {
        return Reply::err(400, "library is not private");
    }
    if !lib.is_member(&v.user_id) {
        return Reply::err(403, "not a member of this library");
    }
    if !app.unlocks.may_try(&v.user_id) {
        return Reply::err(429, "too many attempts; try again in a few minutes");
    }
    let auth = app.auth();
    let ok = match (&i.pin, &i.password) {
        (Some(pin), _) => auth.check_pin(&v.user_id, pin),
        (None, Some(pw)) => auth.check_password(&v.user_id, pw),
        (None, None) => return Reply::err(400, "pin or password required"),
    };
    if !ok {
        app.unlocks.failed(&v.user_id);
        return Reply::err(403, "wrong pin or password");
    }
    app.unlocks.succeeded(&v.user_id);
    app.unlocks.unlock(v, &lib.id);
    Reply::ok(library_view(app, &lib, v))
}

fn libraries_lock(app: &App, req: &Req, v: &Verified) -> Reply {
    let Some(lib) = app.libraries.get(req.param("id")).filter(|l| privacy::record_visible(l, &v.user_id, v.is_admin()))
    else {
        return Reply::err(404, "unknown library");
    };
    app.unlocks.lock(v, &lib.id);
    Reply::ok(library_view(app, &lib, v))
}

fn roots_list(app: &App, _: &Req, _: &Verified) -> Reply {
    Reply::ok(app.roots.read().unwrap_or_else(|e| e.into_inner()).view())
}

fn search(app: &App, req: &Req, v: &Verified) -> Reply {
    let mut p = PageParams::normalize(atoi(req.query("limit")), atoi(req.query("offset")), req.query("sort"));
    p.exclude_libraries = app.hidden_libraries(v);
    let input = json!({"q": req.query("q"), "kind": req.query("kind"), "page": p});
    match app.registry.call_one(CAP_CATALOG_READ, "query", input) {
        Ok((out, _)) => Reply::ok(page_with_thumbnails(out)),
        Err(e) => Reply::err(503, &e.to_string()),
    }
}

/// A catalog page with what each item's thumbnail is (D-282); a page that
/// does not decode (a swapped provider's) goes out as it came.
fn page_with_thumbnails(out: Value) -> Value {
    match serde_json::from_value::<CatalogPage>(out.clone()) {
        Ok(mut page) => {
            crate::watch::tell_thumbnails(&mut page.items);
            json!(page)
        }
        Err(_) => out,
    }
}

fn libraries_create(app: &App, req: &Req, _: &Verified) -> Reply {
    #[derive(Deserialize)]
    struct In {
        #[serde(default)]
        name: String,
        #[serde(default, rename = "type")]
        kind: String,
        #[serde(default)]
        path: String,
        #[serde(default)]
        root: String,
    }
    let i: In = match req.decode() {
        Ok(i) => i,
        Err(r) => return r,
    };
    match app.libraries.create(&i.name, &i.kind, &i.path, &i.root) {
        Ok(lib) => {
            app.publish_library(&lib, json!({"library_id": lib.id, "action": "created"}));
            Reply::json(201, json!(lib))
        }
        Err(e) => Reply::err(400, &e),
    }
}

fn libraries_delete(app: &App, req: &Req, _: &Verified) -> Reply {
    let id = req.param("id");
    let record = app.libraries.get(id);
    if let Err(e) = app.libraries.delete(id) {
        return Reply::err(404, &e);
    }
    // A library record without its items is a leak: nothing would ever
    // collect them, so the catalog removes them now.
    let removed = match app.registry.call_one(CAP_CATALOG_WRITE, "delete_library", json!({"library_id": id})) {
        Ok((n, _)) => n.as_i64().unwrap_or(0),
        Err(e) => {
            tracing::error!(library = %id, err = %e, "library catalog cleanup failed");
            return Reply::err(500, "library removed, but its catalog entries could not be deleted");
        }
    };
    if let Some(lib) = record {
        app.publish_library(&lib, json!({"library_id": id, "action": "deleted", "items_removed": removed}));
    }
    Reply::ok(json!({"status": "ok", "items_removed": removed}))
}

// ---- catalog and progress -------------------------------------------------

fn catalog_list(app: &App, req: &Req, v: &Verified) -> Reply {
    let mut p = PageParams::normalize(atoi(req.query("limit")), atoi(req.query("offset")), req.query("sort"));
    p.library_id = req.query("library_id").to_string();
    p.exclude_libraries = app.hidden_libraries(v);
    match app.registry.call_one(CAP_CATALOG_READ, "page", json!({"page": p})) {
        Ok((out, _)) => Reply::ok(page_with_thumbnails(out)),
        Err(e) => Reply::err(500, &e.to_string()),
    }
}

fn catalog_get(app: &App, req: &Req, v: &Verified) -> Reply {
    match app.registry.call_one(CAP_CATALOG_READ, "get", json!({"id": req.param("id")})) {
        Ok((out, _)) if app.visible_item(v, req.param("id")).is_some() => {
            match serde_json::from_value::<CatalogItem>(out.clone()) {
                Ok(mut it) => {
                    crate::watch::tell_thumbnails(std::slice::from_mut(&mut it));
                    Reply::ok(json!(it))
                }
                Err(_) => Reply::ok(out),
            }
        }
        Ok(_) => Reply::err(404, "unknown item"),
        Err(e) => {
            if !e.is(codes::NOT_FOUND) {
                tracing::warn!(item = %req.param("id"), err = %e, "catalog get failed");
            }
            Reply::err(404, "unknown item")
        }
    }
}

fn catalog_episodes(app: &App, req: &Req, v: &Verified) -> Reply {
    if app.visible_item(v, req.param("id")).is_none() {
        return Reply::err(404, "unknown item");
    }
    match app.registry.call_one(CAP_CATALOG_READ, "episodes", json!({"id": req.param("id")})) {
        Ok((out, _)) => {
            // A title split across a public and a locked library shows
            // only the public part.
            let hidden = app.hidden_libraries(v);
            let mut items: Vec<CatalogItem> = serde_json::from_value::<Vec<CatalogItem>>(out)
                .unwrap_or_default()
                .into_iter()
                .filter(|it| !hidden.contains(&it.library_id))
                .collect();
            crate::watch::tell_thumbnails(&mut items);
            Reply::ok(json!({"items": items}))
        }
        Err(e) => {
            if !e.is(codes::NOT_FOUND) {
                tracing::warn!(item = %req.param("id"), err = %e, "catalog episodes failed");
            }
            Reply::err(404, "unknown item")
        }
    }
}

fn progress_get(app: &App, req: &Req, v: &Verified) -> Reply {
    // An item of a locked private library reads like one never played.
    if let Some(lib) = item_library(app, req.param("id"))
        && app.hidden_libraries(v).contains(&lib)
    {
        return Reply::ok(json!(Progress::default()));
    }
    match app.registry.call_one(CAP_USER_PROGRESS, "get", json!({"user_id": v.user_id, "item_id": req.param("id")})) {
        Ok((out, _)) => Reply::ok(out),
        Err(e) => Reply::err(503, &e.to_string()),
    }
}

fn progress_put(app: &App, req: &Req, v: &Verified) -> Reply {
    let mut p: Progress = match req.decode() {
        Ok(p) => p,
        Err(r) => return r,
    };
    p.item_id = req.param("id").to_string();
    // Scrobbles and social activity look at the transition: the
    // reporter keeps re-sending the final position.
    let prev: Progress = app
        .registry
        .call_one(CAP_USER_PROGRESS, "get", json!({"user_id": v.user_id, "item_id": p.item_id}))
        .ok()
        .and_then(|(v, _)| serde_json::from_value(v).ok())
        .unwrap_or_default();
    match app.registry.call_one(CAP_USER_PROGRESS, "put", json!({"user_id": v.user_id, "progress": p})) {
        Ok((out, _)) => {
            app.publish_progress(&v.user_id, &p.item_id, json!({"item_id": p.item_id, "progress": out}));
            if let Some(it) = crate::social::catalog_item(app, &p.item_id) {
                if p.completed && !prev.completed {
                    crate::lists::scrobble_after(app, &v.user_id, &it);
                }
                crate::social::record_progress(app, &v.user_id, &it, p.completed, prev.completed);
            }
            Reply::ok(out)
        }
        Err(e) => Reply::err(503, &e.to_string()),
    }
}

// ---- composition ----------------------------------------------------------

fn plugins(app: &App, _: &Req, _: &Verified) -> Reply {
    let reg = &app.registry;
    Reply::ok(json!({
        "composition": reg.composition_view(),
        "providers": reg.providers(),
        "provider_info": reg.provider_infos(),
        "events": reg.events(),
    }))
}

fn plugins_swap(app: &App, req: &Req, _: &Verified) -> Reply {
    #[derive(Deserialize)]
    struct In {
        #[serde(default)]
        capability: String,
        #[serde(default)]
        providers: Vec<String>,
        #[serde(default)]
        generation: u64,
    }
    let i: In = match req.decode() {
        Ok(i) => i,
        Err(r) => return r,
    };
    match app.registry.swap(&i.capability, &i.providers, i.generation) {
        Ok(generation) => {
            if let Err(e) = app.save_composition() {
                return Reply::err(500, &e);
            }
            Reply::ok(json!({"generation": generation, "composition": app.registry.composition_view()}))
        }
        Err((e, generation)) => {
            let status = match e.code.as_str() {
                codes::STALE_GENERATION => 409,
                codes::DEPENDENCY_UNAVAILABLE => 422,
                _ => 400,
            };
            Reply::json(status, json!({"error": e.msg, "code": e.code, "generation": generation}))
        }
    }
}

fn plugins_withdraw(app: &App, req: &Req, _: &Verified) -> Reply {
    #[derive(Deserialize)]
    struct In {
        #[serde(default)]
        provider: String,
    }
    let i: In = match req.decode() {
        Ok(i) => i,
        Err(r) => return r,
    };
    if i.provider.is_empty() {
        return Reply::err(400, "provider required");
    }
    if let Err(e) = app.registry.withdraw(&i.provider) {
        return Reply::json(400, json!({"error": e.msg, "code": e.code}));
    }
    if let Err(e) = app.save_composition() {
        return Reply::err(500, &e);
    }
    Reply::ok(json!({"composition": app.registry.composition_view()}))
}

fn modules_list(app: &App, _: &Req, _: &Verified) -> Reply {
    Reply::ok(app.modules_view())
}

// ---- events (SSE) ---------------------------------------------------------

async fn events(State(app): State<Arc<App>>, uri: Uri, headers: HeaderMap) -> Response {
    let req = Req {
        query: parse_query(uri.query().unwrap_or("")),
        authorization: headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).map(str::to_string),
        body: Bytes::new(),
        params: BTreeMap::new(),
        headers: HeaderMap::new(),
    };
    let token = req.token().unwrap_or("").to_string();
    let app2 = app.clone();
    let verified = tokio::task::spawn_blocking(move || authorize(&app2, &req, Gate::User)).await;
    let v = match verified {
        Ok(Ok(v)) => v,
        Ok(Err(r)) => return r.into_response(),
        Err(_) => return Reply::err(500, "internal error").into_response(),
    };
    let info = SubInfo { user_id: v.user_id.clone(), admin: v.is_admin(), session: v.session.clone() };
    let rx = app.events.subscribe(info, &json!({"server": "lain-rs", "version": app.version}));
    let check_app = app.clone();
    let check = Box::new(move || check_app.verify(&token).is_some());
    let mut r = Response::new(Body::new(SseBody::new(rx, app.keep_alive, check)));
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    h.insert("x-accel-buffering", HeaderValue::from_static("no"));
    r
}

// ---- modules, fallbacks and logging --------------------------------------

async fn module_gate(State(app): State<Arc<App>>, req: Request, next: Next) -> Response {
    if let Some(module) = app.disabled_owner(req.uri().path()) {
        return Reply::json(
            404,
            json!({"error": format!("module {module} is disabled"), "code": codes::MODULE_DISABLED, "module": module}),
        )
        .into_response();
    }
    next.run(req).await
}

/// The item a route addresses, for the privacy gate. Progress is not
/// gated here: it answers its own zero value (see `progress_get`).
fn item_route(path: &str) -> Option<&str> {
    let segs: Vec<&str> = path.trim_matches('/').split('/').collect();
    match segs.as_slice() {
        ["api", "items", id, "progress"] | ["api", "items", id] if !id.is_empty() => None,
        ["api", "items", id, ..] | ["api", "catalog", id, ..] | ["api", "acquire", "items", id, ..]
            if !id.is_empty() =>
        {
            Some(id)
        }
        _ => None,
    }
}

/// D-156 at the boundary: every item route (detail, episodes,
/// thumbnails, sidecars, stream, pages, playback, ...) answers 404
/// `unknown item` for an item this session may not see, exactly as for
/// one that does not exist, whether or not the route is ported yet.
/// `DELETE /api/items/{id}` checks visibility itself, after its admin
/// gate, so a user deleting any item gets Go's 403.
async fn privacy_gate(State(app): State<Arc<App>>, req: Request, next: Next) -> Response {
    let Some(id) = item_route(req.uri().path()).map(str::to_string) else { return next.run(req).await };
    let r = Req {
        query: parse_query(req.uri().query().unwrap_or("")),
        authorization: req.headers().get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).map(str::to_string),
        body: Bytes::new(),
        params: BTreeMap::new(),
        headers: HeaderMap::new(),
    };
    let app2 = app.clone();
    let blocked = tokio::task::spawn_blocking(move || match r.token().and_then(|t| app2.verify(t)) {
        // Anonymous calls fall through to the route's own 401.
        None => false,
        Some(v) => app2.visible_item(&v, &id).is_none(),
    })
    .await
    .unwrap_or(true);
    if blocked {
        return Reply::err(404, "unknown item").into_response();
    }
    next.run(req).await
}

/// Everything the API router did not take: the web SPA, which also
/// answers unknown `/api` and `/health` paths with a JSON 404 and other
/// methods with a 405 (D-235).
async fn fallback(State(app): State<Arc<App>>, method: Method, uri: Uri, headers: HeaderMap) -> Response {
    app.web.respond(&method, uri.path(), &headers)
}

async fn access_log(req: Request, next: Next) -> Response {
    let start = Instant::now();
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let resp = next.run(req).await;
    tracing::info!(method = %method, path = %path, status = resp.status().as_u16(), ms = start.elapsed().as_millis() as u64, "request");
    resp
}

impl From<AuthError> for Reply {
    fn from(e: AuthError) -> Reply {
        Reply::err(400, &e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_decoding() {
        assert_eq!(
            parse_query("a=1&b=x%20y&c=a+b&token=t.k.n&d"),
            vec![
                ("a".into(), "1".into()),
                ("b".into(), "x y".into()),
                ("c".into(), "a b".into()),
                ("token".into(), "t.k.n".into()),
                ("d".into(), "".into())
            ]
        );
        assert_eq!(percent_decode("%zz%4"), "%zz%4");
    }
}
