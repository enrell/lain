//! The lists routes (D-078..D-083): `GET /api/list`, the linked
//! accounts under `/api/me/links`, the OAuth landing
//! `GET /api/auth/{platform}/callback`, and the operator's integration
//! settings. Thin: the work lives in `lain_lists::Links`, through the
//! registry. Go: `internal/gateway/list.go` and `scrobble.go`.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use serde::Deserialize;
use serde_json::json;

use lain_admin::integrations;
use lain_auth::Verified;
use lain_core::Error;
use lain_lists::code;
use lain_lists::links::{code_from_paste, err_text};
use lain_lists::scrobble::Finished;

use crate::app::App;
use crate::contracts::CatalogItem;
use crate::gateway::{DELETE, GET, Gate, PATCH, POST, PUT, Reply, Req, base_url, route};

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/api/list", route(GET, Gate::User, list))
        .route("/api/me/links", route(GET, Gate::User, links))
        .route(
            "/api/me/links/{platform}",
            route(PATCH, Gate::User, link_patch).merge(route(DELETE, Gate::User, unlink)),
        )
        .route("/api/me/links/{platform}/authorize", route(GET, Gate::User, authorize))
        .route("/api/me/links/{platform}/pin", route(GET, Gate::User, pin))
        .route("/api/me/links/{platform}/code", route(POST, Gate::User, pasted_code))
        .route("/api/me/links/{platform}/sync", route(POST, Gate::User, sync))
        .route("/api/auth/{platform}/callback", route(GET, Gate::Public, callback))
        .route(
            "/api/admin/settings/integrations",
            route(GET, Gate::Admin, integrations_get).merge(route(PUT, Gate::Admin, integrations_put)),
        )
}

fn now() -> i64 {
    lain_core::now_unix()
}

/// Go's `writeLinkErr`: the code decides the status, and stays in the
/// body.
fn link_err(e: &Error) -> Reply {
    let status = match e.code.as_str() {
        code::UNSUPPORTED_PLATFORM => 404,
        code::TOKEN_INVALID | code::TOKEN_EXPIRED => 401,
        "invalid-message" | code::INVALID_GRANT => 400,
        _ => 503,
    };
    Reply::json(status, json!({"error": err_text(e), "code": e.code}))
}

fn not_configured(what: &str, platform: &str) -> Reply {
    Reply::json(503, json!({"error": format!("no {what} configured for {platform}"), "code": code::NOT_CONFIGURED}))
}

fn not_linked(platform: &str) -> Reply {
    Reply::err(404, &format!("no {platform} account linked"))
}

/// The redirect URI the platform sends the user back to, derived from
/// the request the client used (the admin page prints the same value).
fn callback_url(req: &Req, platform: &str) -> String {
    format!("{}/api/auth/{platform}/callback", base_url(req))
}

fn list(app: &App, req: &Req, v: &Verified) -> Reply {
    match app.links.list(&v.user_id, req.query("type"), req.query("status")) {
        Ok(entries) => Reply::ok(json!({"entries": entries})),
        Err(e) => Reply::err(503, &e.to_string()),
    }
}

fn links(app: &App, _: &Req, v: &Verified) -> Reply {
    match app.links.accounts(&v.user_id) {
        Ok(accounts) => {
            let views: Vec<_> = accounts.iter().map(|a| a.view(now())).collect();
            Reply::ok(json!({"links": views}))
        }
        Err(e) => Reply::err(503, &e.to_string()),
    }
}

/// Starts an OAuth link: a signed, short-lived, user-bound state goes
/// into the platform's authorize URL (D-080).
fn authorize(app: &App, req: &Req, v: &Verified) -> Reply {
    let platform = req.param("platform");
    let Some((client_id, _)) = app.links.client(platform) else { return not_configured("OAuth client", platform) };
    match app.links.authorize(&v.user_id, platform, &client_id, &callback_url(req, platform)) {
        Ok(url) => Reply::ok(json!({"url": url})),
        Err(e) if e.code == "internal" => Reply::err(500, &e.msg),
        Err(e) => link_err(&e),
    }
}

/// The zero-setup path (D-083): the official app's authorize URL; the
/// user pastes back the code AniList's pin page shows.
fn pin(app: &App, req: &Req, _: &Verified) -> Reply {
    let platform = req.param("platform");
    match app.links.pin_url(platform) {
        Some(url) => Reply::ok(json!({"url": url})),
        None => not_configured("pin client", platform),
    }
}

/// Exchanges the pasted pin code; a paste that fails never links.
fn pasted_code(app: &App, req: &Req, v: &Verified) -> Reply {
    let platform = req.param("platform");
    if app.links.pin_url(platform).is_none() {
        return not_configured("pin client", platform);
    }
    #[derive(Deserialize)]
    struct In {
        #[serde(default)]
        code: String,
    }
    let i: In = match req.decode() {
        Ok(i) => i,
        Err(r) => return r,
    };
    let pasted = code_from_paste(&i.code);
    if pasted.is_empty() {
        return Reply::err(400, "code is required");
    }
    match app.links.exchange_pin(platform, &pasted) {
        Ok(id) => {
            let a = app.links.link(&v.user_id, platform, id, "pin");
            Reply::ok(json!({"link": a.view(now())}))
        }
        Err(e) if e.code == "internal" => Reply::err(500, &e.msg),
        Err(e) => link_err(&e),
    }
}

fn redirect(location: &str) -> Reply {
    let mut r = Response::new(Body::from(format!("<a href=\"{location}\">Found</a>.\n\n")));
    *r.status_mut() = StatusCode::FOUND;
    if let Ok(v) = HeaderValue::from_str(location) {
        r.headers_mut().insert(header::LOCATION, v);
    }
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    Reply::response(r)
}

/// The OAuth landing: public (no session header survives the
/// cross-site redirect), identity proven by the signed state. On
/// success the account is stored, the first import runs, and the
/// browser lands back on the account settings page.
fn callback(app: &App, req: &Req, _: &Verified) -> Reply {
    let platform = req.param("platform");
    let fail = |code: &str| redirect(&format!("/settings/connections?link_error={code}"));
    if !req.query("error").is_empty() {
        return fail("denied");
    }
    let code = req.query("code");
    let uid = match app.links.verify_state(req.query("state"), platform) {
        Some(uid) if !code.is_empty() => uid,
        _ => return Reply::err(400, "bad link state or code"),
    };
    // The state proves who asked; the account must still exist and be
    // enabled to complete a link.
    if !app.auth().get_public(&uid).is_some_and(|u| !u.disabled) {
        return Reply::err(400, "unknown or disabled user");
    }
    let Some((client_id, client_secret)) = app.links.client(platform) else { return fail(code::NOT_CONFIGURED) };
    let input = lain_lists::link::ExchangeInput {
        platform: platform.into(),
        client_id,
        client_secret,
        code: code.into(),
        redirect_uri: callback_url(req, platform),
    };
    match app.links.exchange(input) {
        Ok(id) => {
            app.links.link(&uid, platform, id, "oauth");
            redirect(&format!("/settings/connections?linked={platform}"))
        }
        Err(e) if e.code == "internal" => fail("exchange-failed"),
        Err(e) => fail(&e.code),
    }
}

fn link_patch(app: &App, req: &Req, v: &Verified) -> Reply {
    let bad = || Reply::err(400, "scrobble (boolean) required");
    let Ok(p) = req.decode::<lain_api::LinkPatch>() else { return bad() };
    let Some(on) = p.scrobble else { return bad() };
    match app.links.set_scrobble(&v.user_id, req.param("platform"), on) {
        Some(a) => Reply::ok(json!(a.view(now()))),
        None => not_linked(req.param("platform")),
    }
}

/// Unlinks: the token and the platform's imported entries (D-079).
fn unlink(app: &App, req: &Req, v: &Verified) -> Reply {
    let platform = req.param("platform");
    match app.links.unlink(&v.user_id, platform) {
        Ok(Some(removed)) => Reply::ok(json!({"status": "ok", "removed": removed})),
        Ok(None) => not_linked(platform),
        Err(e) => Reply::err(503, &e.to_string()),
    }
}

fn sync(app: &App, req: &Req, v: &Verified) -> Reply {
    let platform = req.param("platform");
    let Some(a) = app.links.account(&v.user_id, platform) else { return not_linked(platform) };
    match app.links.sync(a) {
        Ok(stats) => Reply::ok(json!({"status": "ok", "stats": stats})),
        Err(e) => link_err(&e),
    }
}

// ---- operator integrations (D-080, D-198) ----------------------------------

fn integrations_get(app: &App, req: &Req, _: &Verified) -> Reply {
    Reply::ok(json!(integrations::view(&app.links.integrations(), &base_url(req))))
}

fn integrations_put(app: &App, req: &Req, _: &Verified) -> Reply {
    let u: integrations::IntegrationsUpdate = match req.decode() {
        Ok(u) => u,
        Err(r) => return r,
    };
    let cur = integrations::apply(app.links.integrations(), &u);
    if let Err(e) = app.links.save_integrations(&cur) {
        return Reply::err(500, &e.to_string());
    }
    tracing::info!("integrations settings updated");
    Reply::ok(json!(integrations::view(&cur, &base_url(req))))
}

// ---- scrobble ---------------------------------------------------------------

/// Pushes a finished item to the platforms that opted in, off the
/// request path. Items of a private library never leave the server
/// (D-156): scrobbling is an outward action about what the user watched.
pub(crate) fn scrobble_after(app: &App, user_id: &str, it: &CatalogItem) {
    if app.libraries.get(&it.library_id).is_some_and(|l| l.private) {
        return;
    }
    let item = Finished { kind: it.kind.clone(), title: it.title.clone(), season: it.season, episode: it.episode };
    let (links, user) = (app.links.clone(), user_id.to_string());
    std::thread::Builder::new().name("scrobble".into()).spawn(move || links.scrobble(&user, &item)).ok();
}
