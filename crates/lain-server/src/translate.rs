//! Subtitle translation (D-229): the subtitles module's routes over
//! `lain.subtitle.translate@1`. Thin: a component (AnimeSubs through
//! `lain-subtitle-animesubs`) does the work and owns the job; lain
//! names a sidecar by index (never by path, like `GET
//! /api/items/{id}/sidecars/{n}`) and relays the job. The translated
//! file lands next to the source sidecar (`<stem>.<lang>.<ext>`), so the
//! sidecar list shows it once the job succeeds.

use std::sync::Arc;

use axum::Router;
use serde::Deserialize;
use serde_json::{Value, json};

use lain_api::{TranslateModel, TranslateStartInput};
use lain_auth::Verified;
use lain_bridge::animesubs::CAP_SUBTITLE_TRANSLATE;
use lain_core::Error;

use crate::app::App;
use crate::gateway::{DELETE, GET, Gate, POST, Reply, Req, route};

pub fn routes() -> Router<Arc<App>> {
    Router::new().route("/api/items/{id}/sidecars/{n}/translate", route(POST, Gate::Admin, start)).route(
        "/api/subtitle-translations/{provider}/{job}",
        route(GET, Gate::Admin, status).merge(route(DELETE, Gate::Admin, cancel)),
    )
}

#[derive(Deserialize)]
struct StartBody {
    #[serde(default)]
    target_language: String,
    #[serde(default)]
    model: Option<TranslateModel>,
    #[serde(default)]
    overwrite: bool,
}

fn component_err(e: &Error) -> Reply {
    let status = match e.code.as_str() {
        "invalid-message" => 400,
        "not-found" => 404,
        "not-cancellable" | "already-exists" => 409,
        "unsupported-format" => 422,
        "module-disabled" => 404,
        "provider-error" => 502,
        _ => 503,
    };
    Reply::json(status, json!({"error": e.msg, "code": e.code}))
}

fn start(app: &App, req: &Req, v: &Verified) -> Reply {
    let Some(it) = app.visible_item(v, req.param("id")) else { return Reply::err(404, "unknown item") };
    let body: StartBody = match req.decode() {
        Ok(b) => b,
        Err(r) => return r,
    };
    if body.target_language.trim().is_empty() {
        return Reply::err(400, "target_language required");
    }
    let list =
        if it.file_path.is_empty() { Vec::new() } else { lain_subtitle::discover(std::path::Path::new(&it.file_path)) };
    let n = req.param("n").parse::<usize>().ok();
    let Some(found) = n.and_then(|n| list.get(n)) else { return Reply::err(404, "no such sidecar") };
    let input = TranslateStartInput {
        input: found.path.to_string_lossy().into_owned(),
        output: String::new(),
        target_language: body.target_language.trim().to_string(),
        model: body.model,
        overwrite: body.overwrite,
    };
    // Ordered-many: the first translator that takes the job serves it;
    // one that declines (`unsupported-format`) lets the next try.
    match app.registry.call_first(CAP_SUBTITLE_TRANSLATE, "start", json!(input), |out: &Value| out["job"].is_string()) {
        Ok(Some((out, _))) => Reply::json(202, out),
        Ok(None) => {
            Reply::json(503, json!({"error": "no translator accepted the job", "code": "dependency-unavailable"}))
        }
        Err(e) => component_err(&e),
    }
}

fn relay(app: &App, req: &Req, op: &str) -> Reply {
    let provider = req.param("provider");
    if !app.registry.has_provider(provider) {
        return Reply::err(404, "unknown translation");
    }
    match app.registry.invoke_provider(CAP_SUBTITLE_TRANSLATE, provider, op, json!({"job": req.param("job")})) {
        Ok(out) => Reply::ok(out),
        Err(e) => component_err(&e),
    }
}

fn status(app: &App, req: &Req, _: &Verified) -> Reply {
    relay(app, req, "status")
}

fn cancel(app: &App, req: &Req, _: &Verified) -> Reply {
    relay(app, req, "cancel")
}
