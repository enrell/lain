//! The HTTP boundary for the P1a routes (axum). Handlers are thin: they
//! authenticate, decode, call auth or the registry, and answer the Go
//! server's JSON (`{"error": msg}` on failure, a trailing newline like
//! Go's encoder). Store and bcrypt work runs on the blocking pool.
//!
//! Routes a module owns answer `module-disabled` while it is off. Go
//! routes not ported yet answer 501 `not-ported` rather than 404, so a
//! client can tell "not here yet" from "does not exist".

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodFilter, MethodRouter, on};
use serde::Deserialize;
use serde_json::{Value, json};

use lain_auth::{AuthError, MASCOTS, PlaybackPolicy, ProfilePatch, ROLE_ADMIN, ROLE_USER, Verified};
use lain_core::codes;
use lain_core::modules::route_matches;

use crate::app::App;
use crate::catalog::{CAP_CATALOG_READ, CAP_CATALOG_WRITE, CAP_USER_PROGRESS};
use crate::contracts::{CatalogItem, Library, PageParams, Progress};
use crate::events::{SseBody, SubInfo};
use crate::libraries::LibraryPatch;
use crate::privacy;

/// A JSON answer.
pub struct Reply {
    status: u16,
    body: Value,
}

impl Reply {
    pub fn json(status: u16, body: Value) -> Reply {
        Reply { status, body }
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
        let mut raw = serde_json::to_vec(&self.body).unwrap_or_else(|_| b"{}".to_vec());
        raw.push(b'\n');
        let mut r = Response::new(Body::from(raw));
        *r.status_mut() = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
        r
    }
}

/// One request as handlers see it.
pub struct Req {
    pub query: Vec<(String, String)>,
    pub authorization: Option<String>,
    pub body: Bytes,
    pub params: BTreeMap<String, String>,
}

impl Req {
    pub fn query(&self, key: &str) -> &str {
        self.query.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()).unwrap_or("")
    }

    fn param(&self, key: &str) -> &str {
        self.params.get(key).map(String::as_str).unwrap_or("")
    }

    /// Bearer header first, else `?token=` (the media-element fallback).
    fn token(&self) -> Option<&str> {
        match self.authorization.as_deref().and_then(|h| h.strip_prefix("Bearer ")) {
            Some(t) => Some(t),
            None => Some(self.query("token")),
        }
        .filter(|t| !t.is_empty())
    }

    /// Decodes a JSON body like Go's `json.Decoder`: the first value,
    /// which must be an object (or `null`); `null` fields are no-ops.
    fn decode<T: for<'de> Deserialize<'de>>(&self) -> Result<T, Reply> {
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
enum Gate {
    Public,
    User,
    Admin,
}

type Handler = fn(&App, &Req, &Verified) -> Reply;

fn authorize(app: &App, req: &Req, gate: Gate) -> Result<Verified, Reply> {
    if gate == Gate::Public {
        return Ok(Verified { user_id: String::new(), role: String::new(), session: String::new(), expires_at: 0 });
    }
    let v = req.token().and_then(|t| app.verify(t)).ok_or_else(|| Reply::err(401, "unauthorized"))?;
    if gate == Gate::Admin && v.role != ROLE_ADMIN {
        return Err(Reply::err(403, "admin required"));
    }
    Ok(v)
}

fn route(filter: MethodFilter, gate: Gate, f: Handler) -> MethodRouter<Arc<App>> {
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

const GET: MethodFilter = MethodFilter::GET;
const POST: MethodFilter = MethodFilter::POST;
const PUT: MethodFilter = MethodFilter::PUT;
const PATCH: MethodFilter = MethodFilter::PATCH;
const DELETE: MethodFilter = MethodFilter::DELETE;

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
        .route("/api/me/continue", route(GET, Gate::User, me_continue))
        .route("/api/profile/mascots", route(GET, Gate::User, mascots))
        .route("/api/users", route(GET, Gate::Admin, users_list).merge(route(POST, Gate::Admin, users_create)))
        .route("/api/users/{id}", route(PATCH, Gate::Admin, users_patch))
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

fn me_preferences(app: &App, req: &Req, v: &Verified) -> Reply {
    #[derive(Deserialize)]
    struct In {
        preferred_language: Option<String>,
    }
    let i: In = match req.decode() {
        Ok(i) => i,
        Err(r) => return r,
    };
    let Some(lang) = i.preferred_language else { return Reply::err(400, "preferred_language is required") };
    if let Err(e) = app.auth().set_preferred_language(&v.user_id, &lang) {
        return Reply::err(400, &e.to_string());
    }
    me(app, req, v)
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
        Err(e) => Reply::err(400, &e.to_string()),
    }
}

fn me_profile(app: &App, req: &Req, v: &Verified) -> Reply {
    let p: ProfilePatch = match req.decode() {
        Ok(p) => p,
        Err(r) => return r,
    };
    match app.auth().update_profile(&v.user_id, &p) {
        Ok(u) => Reply::ok(json!(u)),
        Err(e) => Reply::err(400, &e.to_string()),
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
        Ok((out, _)) => Reply::ok(out),
        Err(e) => Reply::err(503, &e.to_string()),
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
        Ok((out, _)) => Reply::ok(out),
        Err(e) => Reply::err(500, &e.to_string()),
    }
}

fn catalog_get(app: &App, req: &Req, v: &Verified) -> Reply {
    match app.registry.call_one(CAP_CATALOG_READ, "get", json!({"id": req.param("id")})) {
        Ok((out, _)) if app.visible_item(v, req.param("id")).is_some() => Reply::ok(out),
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
            let items: Vec<Value> = serde_json::from_value::<Vec<CatalogItem>>(out)
                .unwrap_or_default()
                .into_iter()
                .filter(|it| !hidden.contains(&it.library_id))
                .map(|it| json!(it))
                .collect();
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
    match app.registry.call_one(CAP_USER_PROGRESS, "put", json!({"user_id": v.user_id, "progress": p})) {
        Ok((out, _)) => {
            app.publish_progress(&v.user_id, &p.item_id, json!({"item_id": p.item_id, "progress": out}));
            Reply::ok(out)
        }
        Err(e) => Reply::err(503, &e.to_string()),
    }
}

// ---- composition ----------------------------------------------------------

fn plugins(app: &App, _: &Req, _: &Verified) -> Reply {
    let reg = &app.registry;
    Reply::ok(json!({
        "composition": reg.composition().view(),
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
            Reply::ok(json!({"generation": generation, "composition": app.registry.composition().view()}))
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
    Reply::ok(json!({"composition": app.registry.composition().view()}))
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
        ["api", "items", id, "progress"] if !id.is_empty() => None,
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
async fn privacy_gate(State(app): State<Arc<App>>, req: Request, next: Next) -> Response {
    let Some(id) = item_route(req.uri().path()).map(str::to_string) else { return next.run(req).await };
    let r = Req {
        query: parse_query(req.uri().query().unwrap_or("")),
        authorization: req.headers().get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).map(str::to_string),
        body: Bytes::new(),
        params: BTreeMap::new(),
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

/// Every route of the Go gateway, for telling "not ported yet" apart
/// from "no such route".
const GO_ROUTES: &[&str] = &[
    "GET /api/theme",
    "GET /api/browse",
    "POST /api/library/scan",
    "GET /api/library/scan",
    "DELETE /api/items/{id}",
    "GET /api/admin/backup",
    "PUT /api/me/avatar",
    "DELETE /api/me/avatar",
    "GET /api/users/{id}/avatar",
];

fn is_api(path: &str) -> bool {
    path == "/api" || path.starts_with("/api/") || path == "/health" || path.starts_with("/health/")
}

async fn fallback(State(app): State<Arc<App>>, method: Method, uri: Uri) -> Response {
    let path = uri.path();
    let ported_elsewhere = GO_ROUTES.iter().any(|r| {
        let (m, p) = r.split_once(' ').unwrap_or(("", r));
        (m == method.as_str() || (method == Method::HEAD && m == "GET")) && route_matches(p, path)
    });
    if ported_elsewhere || (is_api(path) && app.route_owned(path)) {
        return Reply::json(501, json!({"error": "not served by lain-rs yet", "code": "not-ported"})).into_response();
    }
    if method != Method::GET && method != Method::HEAD {
        let mut r = Reply::err(405, "method not allowed").into_response();
        r.headers_mut().insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
        return r;
    }
    if is_api(path) {
        return Reply::err(404, "not found").into_response();
    }
    let mut r = Response::new(Body::from("lain-rs does not embed the web UI yet; use the Go server for it.\n"));
    *r.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    r
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
