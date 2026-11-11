//! The watch module's routes (D-190): playback plans, streams with
//! Range, transcode sessions (progressive and HLS with signed segment
//! URIs), subtitles, thumbnails, local playback and the operator's
//! transcode settings. The Go gateway's handlers, answer for answer;
//! the work goes through the registry like Go's (swaps and module
//! suspend apply), and every item read checks `visible_item` (D-156).
//!
//! Handlers run on the blocking pool (they probe, wait on the cache or
//! read small files); file bodies stream from the async side in chunks
//! read on that pool (`serve.rs`). Media elements cannot send headers,
//! so every media route also takes `?token=`.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::connect_info::ConnectInfo;
use axum::extract::{Path as UrlPath, Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodFilter, MethodRouter, on};
use serde_json::{Value, json};

use lain_api::{
    AdminTranscodeStatus, CatalogItem, MediaInfo, PlaybackOptions, PlaybackPlan, TranscodeSelection, TranscodeSettings,
};
use lain_auth::{PlaybackPolicy, Verified};
use lain_core::Error;
use lain_watch::transcode::{self, Policy, V3Request, V3Status};
use lain_watch::{CAP_MEDIA_PROBE, CAP_PLAYBACK_PLAN, CAP_TRANSCODE_SETTINGS, localplay, settings as st, thumbnail};

use crate::app::App;
use crate::gateway::{Gate, Reply, Req, authorize, parse_query};
use crate::serve;

/// One watch request: the gateway's [`Req`] plus what media routes need.
pub(crate) struct WReq {
    pub req: Req,
    pub headers: HeaderMap,
    pub method: Method,
    pub peer: Option<SocketAddr>,
}

/// A file to stream.
pub(crate) struct FileOut {
    path: PathBuf,
    content_type: String,
    headers: Vec<(&'static str, String)>,
    /// The 404 text when the file cannot be opened.
    missing: &'static str,
}

pub(crate) enum Out {
    Reply(Reply),
    WithHeaders(Reply, Vec<(&'static str, String)>),
    File(FileOut),
    Text { content_type: &'static str, cache: &'static str, body: String },
}

impl From<Reply> for Out {
    fn from(r: Reply) -> Self {
        Out::Reply(r)
    }
}

type Handler = fn(&App, &WReq, &Verified) -> Out;

fn file(
    path: impl Into<PathBuf>,
    content_type: &str,
    headers: Vec<(&'static str, String)>,
    missing: &'static str,
) -> Out {
    Out::File(FileOut { path: path.into(), content_type: content_type.into(), headers, missing })
}

fn route(filter: MethodFilter, gate: Gate, f: Handler) -> MethodRouter<Arc<App>> {
    on(
        filter,
        move |State(app): State<Arc<App>>, params: Option<UrlPath<BTreeMap<String, String>>>, request: Request| async move {
            let (parts, body) = request.into_parts();
            let body = axum::body::to_bytes(body, 1 << 20).await.unwrap_or_default();
            let peer = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|c| c.0);
            let w = WReq {
                req: Req {
                    query: parse_query(parts.uri.query().unwrap_or("")),
                    authorization: parts
                        .headers
                        .get(header::AUTHORIZATION)
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string),
                    body,
                    params: params.map(|p| p.0).unwrap_or_default(),
                    headers: parts.headers.clone(),
                },
                headers: parts.headers,
                method: parts.method,
                peer,
            };
            let app2 = app.clone();
            let done = tokio::task::spawn_blocking(move || {
                let out = match authorize(&app2, &w.req, gate) {
                    Ok(v) => f(&app2, &w, &v),
                    Err(r) => Out::Reply(r),
                };
                // Opening and stat-ing the file is disk work too.
                let opened = match out {
                    Out::File(fo) => open(fo),
                    other => Ready::Out(other),
                };
                (opened, w.headers, w.method)
            })
            .await;
            match done {
                Ok((Ready::Out(out), _, _)) => respond(out),
                Ok((Ready::File(o), headers, method)) => {
                    serve::content(o.file, o.size, o.modified, &headers, &method, o.headers)
                }
                Err(_) => Reply::err(500, "internal error").into_response(),
            }
        },
    )
}

/// An opened file, ready to stream.
struct Opened {
    file: std::fs::File,
    size: u64,
    modified: Option<std::time::SystemTime>,
    headers: HeaderMap,
}

/// What the blocking side hands back: an answer, or an opened file.
enum Ready {
    Out(Out),
    File(Box<Opened>),
}

/// Opens a file to stream (a JSON answer when it cannot be).
fn open(fo: FileOut) -> Ready {
    let file = match std::fs::File::open(&fo.path) {
        Ok(f) => f,
        Err(_) => return Ready::Out(Reply::err(404, fo.missing).into()),
    };
    let meta = match file.metadata() {
        Ok(m) => m,
        Err(e) => return Ready::Out(Reply::err(500, &e.to_string()).into()),
    };
    let mut h = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(&fo.content_type) {
        h.insert(header::CONTENT_TYPE, v);
    }
    for (k, v) in fo.headers {
        if let Ok(v) = HeaderValue::from_str(&v) {
            h.insert(HeaderName::from_static(k), v);
        }
    }
    Ready::File(Box::new(Opened { file, size: meta.len(), modified: meta.modified().ok(), headers: h }))
}

fn respond(out: Out) -> Response {
    match out {
        Out::Reply(r) => r.into_response(),
        Out::WithHeaders(r, hs) => {
            let mut resp = r.into_response();
            for (k, v) in hs {
                if let Ok(v) = HeaderValue::from_str(&v) {
                    resp.headers_mut().insert(HeaderName::from_static(k), v);
                }
            }
            resp
        }
        Out::Text { content_type, cache, body } => {
            let mut r = Response::new(Body::from(body));
            r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
            r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
            r
        }
        Out::File(_) => Reply::err(500, "internal error").into_response(),
    }
}

const GET: MethodFilter = MethodFilter::GET;
const POST: MethodFilter = MethodFilter::POST;
const PUT: MethodFilter = MethodFilter::PUT;
const DELETE: MethodFilter = MethodFilter::DELETE;

/// The watch routes, merged into the gateway router.
pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/api/items/{id}/playback", route(GET, Gate::User, playback))
        .route("/api/items/{id}/stream", route(GET, Gate::User, stream))
        .route("/api/items/{id}/thumbnail", route(GET, Gate::User, thumbnail_get))
        .route(
            "/api/items/{id}/transcode",
            route(GET, Gate::User, transcode_get).merge(route(POST, Gate::User, transcode_start)).merge(route(
                DELETE,
                Gate::User,
                transcode_cancel,
            )),
        )
        .route("/api/items/{id}/transcode/status", route(GET, Gate::User, transcode_status))
        .route("/api/items/{id}/transcode/hls/{file}", route(GET, Gate::User, transcode_hls))
        .route("/api/items/{id}/subtitles", route(GET, Gate::User, subtitles))
        .route("/api/items/{id}/play-local", route(POST, Gate::User, play_local))
        .route("/api/localplay", route(GET, Gate::User, localplay_get))
        .route("/api/playback/options", route(GET, Gate::User, playback_options))
        .route(
            "/api/admin/settings/transcode",
            route(GET, Gate::Admin, settings_get).merge(route(PUT, Gate::Admin, settings_put)),
        )
        .route("/api/admin/transcodes", route(GET, Gate::Admin, admin_transcodes))
        .route("/api/admin/transcodes/{session}", route(DELETE, Gate::Admin, admin_transcode_cancel))
}

// ---- shared -------------------------------------------------------------

fn item(app: &App, w: &WReq, v: &Verified) -> Result<CatalogItem, Box<Out>> {
    app.visible_item(v, w.req.param("id")).ok_or_else(|| Box::new(Reply::err(404, "unknown item").into()))
}

/// The effective transcode settings: defaults when the provider fails.
fn settings(app: &App) -> TranscodeSettings {
    app.registry
        .call_one(CAP_TRANSCODE_SETTINGS, "get", json!({}))
        .ok()
        .and_then(|(v, _)| serde_json::from_value(v).ok())
        .unwrap_or_else(st::defaults)
}

fn allows(flag: Option<bool>) -> bool {
    flag.unwrap_or(true)
}

fn restricted(p: &PlaybackPolicy) -> bool {
    p.allow_video_transcode.is_some()
        || p.allow_audio_transcode.is_some()
        || p.allow_remux.is_some()
        || p.max_bitrate_kbps > 0
        || p.max_streams > 0
        || !p.subtitle_mode.is_empty()
}

/// The tighter non-zero cap, 0 when neither is set.
fn effective_bitrate(user: i64, server: i64) -> i64 {
    match (user > 0, server > 0) {
        (false, _) => server.max(0),
        (true, false) => user,
        (true, true) => user.min(server),
    }
}

fn source_kbps(info: Option<&MediaInfo>) -> i64 {
    info.map(|i| {
        i.streams.iter().filter(|s| s.kind == "video" || s.kind == "audio").map(|s| s.bit_rate).sum::<i64>() / 1000
    })
    .unwrap_or(0)
}

fn v3(app: &App, req: V3Request) -> Result<V3Status, Error> {
    let action = req.action.clone();
    let (out, _) = app.registry.call_one(transcode::CAP_V3, &action, serde_json::to_value(&req).unwrap_or_default())?;
    serde_json::from_value(out).map_err(|_| Error::internal("bad transcode status"))
}

fn transcode_error(e: &Error) -> Out {
    let (status, msg) = match e.code.as_str() {
        "invalid-message" => (400, e.msg.as_str()),
        "forbidden" => (403, e.msg.as_str()),
        "queue-full" | "too-many-streams" => (429, e.msg.as_str()),
        "not-found" => (404, e.msg.as_str()),
        "unsupported-media" => (422, e.msg.as_str()),
        _ => (503, "transcode unavailable"),
    };
    Reply::json(status, json!({"error": msg, "code": e.code})).into()
}

fn session_required(msg: &str) -> Out {
    Reply::json(400, json!({"error": msg, "code": "invalid-message"})).into()
}

fn public(s: &V3Status) -> Value {
    json!(s.public())
}

fn content_type(path: &str) -> &'static str {
    let ext = Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "mkv" => "video/x-matroska",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "avi" => "video/x-msvideo",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "ogg" | "opus" => "audio/ogg",
        _ => "application/octet-stream",
    }
}

// ---- plans and streams --------------------------------------------------

fn playback(app: &App, w: &WReq, v: &Verified) -> Out {
    let it = match item(app, w, v) {
        Ok(it) => it,
        Err(o) => return *o,
    };
    // A row can outlive its file between reconciliations (D-068).
    if std::fs::metadata(&it.file_path).is_err() {
        return Reply::ok(json!(PlaybackPlan {
            mode: "unavailable".into(),
            available: false,
            reason: "the file is no longer on disk".into(),
            ..PlaybackPlan::default()
        }))
        .into();
    }
    let client = w.req.query("client").to_string();
    let caps_raw: Vec<String> = w.req.query.iter().filter(|(k, _)| k == "caps").map(|(_, v)| v.clone()).collect();
    let caps = lain_watch::plan::parse_capabilities(&caps_raw);
    let settings = settings(app);
    let lower = client.to_ascii_lowercase();
    let mut info: Option<MediaInfo> = None;
    if !lower.contains("mpv") && !lower.contains("desktop") {
        match app.registry.call_one(
            CAP_MEDIA_PROBE,
            "probe",
            json!({"file_path": it.file_path, "ffprobe_path": settings.ffprobe_path}),
        ) {
            Ok((out, _)) => info = serde_json::from_value(out).ok(),
            Err(e) => tracing::debug!(item = %it.id, err = %e, "media probe failed"),
        }
    }
    // Tone mapping is promised only when enabled and probed (D-031); the
    // probe runs only when there is HDR to map.
    let hdr = info.as_ref().is_some_and(|i| {
        i.streams.iter().any(|s| {
            s.kind == "video" && matches!(s.color_transfer.to_ascii_lowercase().as_str(), "smpte2084" | "arib-std-b67")
        })
    });
    let tone_map = hdr
        && settings.tone_mapping
        && settings.tone_mapping_mode != st::TONEMAP_MODE_NEVER
        && app.media.transcoder.probe(&settings).tone_mapping;
    let input = lain_watch::plan::PlanInput {
        item_id: it.id.clone(),
        client,
        network: w.req.query("network").to_string(),
        capabilities: caps,
        file_path: it.file_path.clone(),
        media_info: info.clone(),
        tone_map,
    };
    let mut plan: PlaybackPlan =
        match app.registry.call_one(CAP_PLAYBACK_PLAN, "plan", serde_json::to_value(&input).unwrap_or_default()) {
            Ok((out, _)) => match serde_json::from_value(out) {
                Ok(p) => p,
                Err(_) => return Reply::err(500, "bad playback plan").into(),
            },
            Err(e) => return Reply::err(503, &e.to_string()).into(),
        };
    let policy = app.auth().playback_policy(&v.user_id);
    let max = effective_bitrate(policy.max_bitrate_kbps, settings.remote_bitrate_limit_kbps);
    // A capped account must not stream above its limit.
    if max > 0 && plan.mode == "direct" && source_kbps(info.as_ref()) > max {
        plan.mode = "transcode".into();
    }
    if plan.mode == "transcode" {
        if !allows(policy.allow_video_transcode) && !allows(policy.allow_remux) {
            plan.available = false;
            plan.mode = "transcode-required".into();
            plan.reason = "transcoding is disabled for this user".into();
        } else if app.registry.ordered(transcode::CAP_V1).is_err() {
            plan.available = false;
            plan.mode = "transcode-required".into();
            plan.reason = "this container needs transcode for browser clients (no transcode provider installed)".into();
        } else if let Ok(s) = v3(
            app,
            V3Request {
                action: transcode::ACTION_INSPECT.into(),
                file_path: it.file_path.clone(),
                delivery: settings.default_delivery.clone(),
                max_bitrate_kbps: max,
                settings: settings.clone(),
                ..V3Request::default()
            },
        ) {
            plan.profile = s.profile;
            plan.session = s.session;
            plan.state = s.state;
            plan.reasons = s.reasons;
        }
    }
    // The probed length lets an HLS player span the real timeline.
    plan.duration_sec = info.map(|i| i.duration).filter(|d| *d > 0.0);
    Reply::ok(json!(plan)).into()
}

fn stream(app: &App, w: &WReq, v: &Verified) -> Out {
    let it = match item(app, w, v) {
        Ok(it) => it,
        Err(o) => return *o,
    };
    let mut headers = Vec::new();
    // ?download=1 saves the original (an offline copy).
    if w.req.query("download") == "1" {
        let name = Path::new(&it.file_path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        headers.push(("content-disposition", serve::attachment(&name)));
    }
    file(&it.file_path, content_type(&it.file_path), headers, "file unavailable")
}

fn thumbnail_get(app: &App, w: &WReq, v: &Verified) -> Out {
    let it = match item(app, w, v) {
        Ok(it) => it,
        Err(o) => return *o,
    };
    // A file that came with its own still shows that one; a frame is
    // cut only when a time is asked for.
    if w.req.query("t").trim().is_empty()
        && let Some(art) = thumbnail::local_cover(Path::new(&it.file_path))
    {
        let headers = vec![("cache-control", "private, max-age=86400".into())];
        let kind = thumbnail::image_type(&art);
        return file(art, kind, headers, "thumbnail unavailable");
    }
    let at = w.req.query("t").trim().parse::<f64>().ok().filter(|t| *t >= 0.0 && t.is_finite()).unwrap_or(10.0);
    let width = w.req.query("w").trim().parse::<i64>().ok().filter(|n| *n > 0).unwrap_or(480).clamp(32, 1280);
    let req = thumbnail::Request { file_path: it.file_path.clone(), time_sec: at, width };
    let thumb: thumbnail::Thumbnail =
        match app.registry.call_one(thumbnail::CAP, "thumbnail", serde_json::to_value(&req).unwrap_or_default()) {
            Ok((out, _)) => match serde_json::from_value::<thumbnail::Thumbnail>(out) {
                Ok(t) if !t.path.is_empty() => t,
                _ => return Reply::err(500, "bad thumbnail result").into(),
            },
            Err(e) => {
                tracing::warn!(item = %it.id, err = %e, "thumbnail failed");
                return Reply::err(503, &e.to_string()).into();
            }
        };
    file(&thumb.path, "image/jpeg", vec![("cache-control", "private, max-age=86400".into())], "thumbnail unavailable")
}

// ---- transcode ----------------------------------------------------------

/// The plugin request from operator settings, the account's limits and
/// the client's selection.
fn transcode_request(app: &App, v: &Verified, path: &str, sel: &TranscodeSelection, item_id: &str) -> V3Request {
    let settings = settings(app);
    let policy = app.auth().playback_policy(&v.user_id);
    let limits = restricted(&policy).then(|| Policy {
        allow_video_transcode: allows(policy.allow_video_transcode),
        allow_audio_transcode: allows(policy.allow_audio_transcode),
        allow_remux: allows(policy.allow_remux),
        max_streams: policy.max_streams,
    });
    V3Request {
        action: transcode::ACTION_START.into(),
        file_path: path.into(),
        delivery: sel.delivery.clone(),
        quality: sel.quality.clone(),
        max_bitrate_kbps: effective_bitrate(policy.max_bitrate_kbps, settings.remote_bitrate_limit_kbps),
        video_codec: sel.video_codec.clone(),
        audio_codec: sel.audio_codec.clone(),
        audio_stream: sel.audio_stream,
        subtitle_stream: sel.subtitle_stream,
        subtitle_mode: if sel.subtitle_mode.is_empty() {
            policy.subtitle_mode.clone()
        } else {
            sel.subtitle_mode.clone()
        },
        start_sec: sel.start_sec,
        allow_video_stream_copy: sel.allow_video_stream_copy,
        allow_audio_stream_copy: sel.allow_audio_stream_copy,
        video_copy_codecs: sel.video_copy_codecs.clone(),
        video_effect: sel.video_effect.clone(),
        policy: limits,
        user_id: v.user_id.clone(),
        item_id: item_id.into(),
        settings,
        ..V3Request::default()
    }
}

fn transcode_start(app: &App, w: &WReq, v: &Verified) -> Out {
    let it = match item(app, w, v) {
        Ok(it) => it,
        Err(o) => return *o,
    };
    let sel: TranscodeSelection = if w.req.body.is_empty() {
        TranscodeSelection::default()
    } else {
        match w.req.decode() {
            Ok(s) => s,
            Err(r) => return r.into(),
        }
    };
    let req = transcode_request(app, v, &it.file_path, &sel, &it.id);
    let s = match v3(app, req) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(item = %it.id, err = %e, "transcode start failed");
            return transcode_error(&e);
        }
    };
    tracing::info!(item = %it.id, state = %s.state, session = %s.session, delivery = %s.delivery, method = %s.method, "transcode start");
    if s.state == transcode::READY {
        return Reply::ok(public(&s)).into();
    }
    Out::WithHeaders(Reply::json(202, public(&s)), vec![("retry-after", "1".into())])
}

fn transcode_status(app: &App, w: &WReq, v: &Verified) -> Out {
    let it = match item(app, w, v) {
        Ok(it) => it,
        Err(o) => return *o,
    };
    let session = w.req.query("session");
    if session.is_empty() {
        return session_required("session required");
    }
    let req = V3Request {
        action: transcode::ACTION_STATUS.into(),
        file_path: it.file_path,
        session: session.into(),
        ..V3Request::default()
    };
    match v3(app, req) {
        Ok(s) => Reply::ok(public(&s)).into(),
        Err(e) => transcode_error(&e),
    }
}

/// Stops a session the caller owns, for the item it names.
fn transcode_cancel(app: &App, w: &WReq, v: &Verified) -> Out {
    let session = w.req.query("session");
    if session.is_empty() {
        return session_required("session required");
    }
    let file_path = app.visible_item(v, w.req.param("id")).map(|i| i.file_path).unwrap_or_default();
    let req = V3Request {
        action: transcode::ACTION_CANCEL.into(),
        session: session.into(),
        file_path,
        user_id: v.user_id.clone(),
        ..V3Request::default()
    };
    match v3(app, req) {
        Ok(s) => {
            tracing::info!(session = %session, "transcode cancelled");
            Reply::ok(public(&s)).into()
        }
        Err(e) => transcode_error(&e),
    }
}

fn resolve(app: &App, path: &str, session: &str) -> Result<V3Status, Error> {
    v3(
        app,
        V3Request {
            action: transcode::ACTION_RESOLVE.into(),
            file_path: path.into(),
            session: session.into(),
            ..V3Request::default()
        },
    )
}

/// Progressive output; without a session the synchronous v1 path.
fn transcode_get(app: &App, w: &WReq, v: &Verified) -> Out {
    let it = match item(app, w, v) {
        Ok(it) => it,
        Err(o) => return *o,
    };
    let mp4 = |p: &str| file(p, "video/mp4", vec![("accept-ranges", "bytes".into())], "file unavailable");
    let session = w.req.query("session");
    if !session.is_empty() {
        let s = match resolve(app, &it.file_path, session) {
            Ok(s) => s,
            Err(e) => return transcode_error(&e),
        };
        if s.delivery == st::DELIVERY_HLS {
            return Reply::json(409, json!({"error": "session uses HLS delivery", "code": "invalid-message"})).into();
        }
        if s.state != transcode::READY || s.path.is_empty() {
            return Out::WithHeaders(Reply::json(202, public(&s)), vec![("retry-after", "1".into())]);
        }
        return mp4(&s.path);
    }
    // The v1 path has no per-session options: a restricted account must
    // not get a full-rate transcode that ignores its limits.
    let policy = app.auth().playback_policy(&v.user_id);
    if !allows(policy.allow_video_transcode) && !allows(policy.allow_remux) {
        return Reply::err(403, "transcoding is disabled for this user").into();
    }
    if policy.max_bitrate_kbps > 0 {
        return Reply::err(403, "this account must use the session API for transcoding").into();
    }
    match app.registry.call_one(transcode::CAP_V1, "prepare", json!({"file_path": it.file_path})) {
        Ok((out, _)) => match serde_json::from_value::<transcode::LegacyResult>(out) {
            Ok(r) if !r.path.is_empty() => mp4(&r.path),
            _ => Reply::err(500, "bad transcode result").into(),
        },
        Err(e) => {
            tracing::warn!(item = %it.id, err = %e, "transcode failed");
            transcode_error(&e)
        }
    }
}

/// Signs one media URI with the session and token. Only same-directory
/// relative names are signed: an absolute or protocol-relative URI
/// would leak the token to a third party.
pub fn sign_uri(uri: &str, session: &str, token: &str) -> String {
    if uri.is_empty() || uri.starts_with("//") || uri.contains("://") || uri.starts_with('/') {
        return uri.to_string();
    }
    if let Some(i) = uri.find(':')
        && !uri[..i].contains(['/', '?', '#'])
    {
        return uri.to_string(); // a scheme (`data:`, `mailto:`)
    }
    let (path, query) = uri.split_once('?').unwrap_or((uri, ""));
    let mut pairs: Vec<(String, String)> =
        parse_query(query).into_iter().filter(|(k, _)| k != "session" && (token.is_empty() || k != "token")).collect();
    pairs.push(("session".into(), session.into()));
    if !token.is_empty() {
        pairs.push(("token".into(), token.into()));
    }
    // Go's url.Values.Encode: keys sorted, values in order.
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    let enc =
        pairs.iter().map(|(k, v)| format!("{}={}", query_escape(k), query_escape(v))).collect::<Vec<_>>().join("&");
    format!("{path}?{enc}")
}

/// Go's `url.QueryEscape`.
fn query_escape(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Signs the `EXT-X-MAP` URI and every segment line; tags and blank
/// lines stay byte-identical. A relative URI in an m3u8 drops the
/// playlist's query, so without this hls.js and Safari would fetch
/// media without credentials.
pub fn rewrite_playlist(raw: &str, session: &str, token: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 64);
    for (i, line) in raw.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let t = line.trim();
        if t.is_empty() {
            out.push_str(line);
        } else if t.starts_with("#EXT-X-MAP:") {
            out.push_str(&sign_map(line, session, token));
        } else if t.starts_with('#') {
            out.push_str(line);
        } else {
            out.push_str(&sign_uri(t, session, token));
        }
    }
    out
}

fn sign_map(line: &str, session: &str, token: &str) -> String {
    const ATTR: &str = "URI=\"";
    let Some(start) = line.find(ATTR) else { return line.to_string() };
    let begin = start + ATTR.len();
    let Some(end) = line[begin..].find('"') else { return line.to_string() };
    format!("{}{}{}", &line[..begin], sign_uri(&line[begin..begin + end], session, token), &line[begin + end..])
}

fn transcode_hls(app: &App, w: &WReq, v: &Verified) -> Out {
    let it = match item(app, w, v) {
        Ok(it) => it,
        Err(o) => return *o,
    };
    let session = w.req.query("session");
    if session.is_empty() {
        return session_required("session required");
    }
    let name = w.req.param("file");
    if !transcode::hls::valid_file(name) {
        return Reply::err(404, "unknown hls file").into();
    }
    let s = match resolve(app, &it.file_path, session) {
        Ok(s) => s,
        Err(e) => return transcode_error(&e),
    };
    if s.delivery != st::DELIVERY_HLS || s.playlist_path.is_empty() {
        return Reply::err(404, "session has no HLS output").into();
    }
    let path = Path::new(&s.playlist_path).parent().unwrap_or(Path::new(".")).join(name);
    if name == transcode::hls::INDEX_PLAYLIST {
        let Ok(raw) = std::fs::read(&path) else { return Reply::err(404, "unknown hls file").into() };
        let body = rewrite_playlist(&String::from_utf8_lossy(&raw), session, w.req.token().unwrap_or(""));
        return Out::Text { content_type: "application/vnd.apple.mpegurl", cache: "no-store", body };
    }
    if !path.exists() {
        return Reply::err(404, "unknown hls file").into();
    }
    // Fetching a segment that exists is the client's progress signal.
    if let Some(index) = transcode::hls::segment_index(name) {
        let req = V3Request {
            action: transcode::ACTION_POSITION.into(),
            file_path: it.file_path.clone(),
            session: session.into(),
            segment_index: index,
            ..V3Request::default()
        };
        if let Err(e) = v3(app, req) {
            tracing::debug!(err = %e, "transcode position update failed");
        }
    }
    let ct = if name == "init.mp4" { "video/mp4" } else { "video/iso.segment" };
    file(path, ct, vec![("cache-control", "private, max-age=3600".into())], "unknown hls file")
}

/// WebVTT for one item: `?stream=N` extracts an embedded track on
/// demand (direct play), `?session=` serves a session's sidecar.
fn subtitles(app: &App, w: &WReq, v: &Verified) -> Out {
    let it = match item(app, w, v) {
        Ok(it) => it,
        Err(o) => return *o,
    };
    let vtt = |p: &str| file(p, "text/vtt; charset=utf-8", vec![("accept-ranges", "bytes".into())], "file unavailable");
    let raw = w.req.query("stream");
    if !raw.is_empty() {
        let Some(index) = raw.parse::<i64>().ok().filter(|n| *n >= 0) else {
            return session_required("stream must be a non-negative index");
        };
        let req = V3Request {
            action: transcode::ACTION_SUBTITLES.into(),
            file_path: it.file_path.clone(),
            subtitle_stream: Some(index),
            settings: settings(app),
            ..V3Request::default()
        };
        return match v3(app, req) {
            Ok(s) if !s.subtitle_path.is_empty() => vtt(&s.subtitle_path),
            Ok(_) => Reply::err(500, "bad subtitle result").into(),
            Err(e) => {
                tracing::warn!(item = %it.id, stream = index, err = %e, "subtitle extraction failed");
                transcode_error(&e)
            }
        };
    }
    let session = w.req.query("session");
    if session.is_empty() {
        return session_required("session or stream required");
    }
    match resolve(app, &it.file_path, session) {
        Ok(s) if s.subtitle_path.is_empty() => Reply::err(404, "subtitles unavailable").into(),
        Ok(s) => vtt(&s.subtitle_path),
        Err(e) => transcode_error(&e),
    }
}

fn playback_options(app: &App, _: &WReq, _: &Verified) -> Out {
    let s = settings(app);
    let effects = if app.media.transcoder.probe(&s).video_effects {
        transcode::effects::PRESETS.iter().map(|p| p.to_string()).collect()
    } else {
        Vec::new()
    };
    Reply::ok(json!(PlaybackOptions {
        default_delivery: s.default_delivery,
        deliveries: vec![st::DELIVERY_HLS.into(), st::DELIVERY_PROGRESSIVE.into()],
        effects,
        qualities: s.qualities,
    }))
    .into()
}

fn settings_view(app: &App) -> Value {
    let s = settings(app);
    let caps = app.media.transcoder.probe(&s);
    json!({"settings": s, "capabilities": caps})
}

fn settings_get(app: &App, _: &WReq, _: &Verified) -> Out {
    Reply::ok(settings_view(app)).into()
}

/// Decodes over the current settings, so a partial document keeps every
/// omitted field (a missing bool must not read as `false`).
fn settings_put(app: &App, w: &WReq, _: &Verified) -> Out {
    let invalid = || Out::from(Reply::err(400, "invalid json"));
    let mut it = serde_json::Deserializer::from_slice(&w.req.body).into_iter::<Value>();
    let patch = match it.next() {
        Some(Ok(Value::Object(o))) => o,
        Some(Ok(Value::Null)) => serde_json::Map::new(),
        _ => return invalid(),
    };
    let mut cur = json!(settings(app));
    for (k, v) in patch {
        if v.is_null() {
            // Only the optional switches take a null (back to default).
            if k == "allow_subtitle_extraction" || k == "fallback_font_enabled" {
                cur.as_object_mut().map(|o| o.remove(&k));
            }
            continue;
        }
        cur[k] = v;
    }
    let s: TranscodeSettings = match serde_json::from_value(cur) {
        Ok(s) => s,
        Err(_) => return invalid(),
    };
    if let Err(e) = app.registry.call_one(CAP_TRANSCODE_SETTINGS, "put", json!({"settings": s})) {
        return Reply::json(400, json!({"error": e.to_string(), "code": "invalid-message"})).into();
    }
    tracing::info!("transcode settings updated");
    Reply::ok(settings_view(app)).into()
}

fn admin_transcodes(app: &App, _: &WReq, _: &Verified) -> Out {
    let list = match app.registry.call_one(transcode::CAP_V3, transcode::ACTION_LIST, json!({"action": "list"})) {
        Ok((out, _)) => serde_json::from_value::<Vec<V3Status>>(out).unwrap_or_default(),
        Err(e) => return transcode_error(&e),
    };
    let sessions: Vec<AdminTranscodeStatus> =
        list.iter().map(|s| AdminTranscodeStatus { status: s.public(), user_id: s.user_id.clone() }).collect();
    Reply::ok(json!({"sessions": sessions})).into()
}

fn admin_transcode_cancel(app: &App, w: &WReq, _: &Verified) -> Out {
    let session = w.req.param("session");
    if session.is_empty() {
        return Reply::err(400, "session required").into();
    }
    let req = V3Request { action: transcode::ACTION_CANCEL.into(), session: session.into(), ..V3Request::default() };
    match v3(app, req) {
        Ok(s) => {
            tracing::info!(session = %session, "transcode cancelled by admin");
            Reply::ok(public(&s)).into()
        }
        Err(e) => transcode_error(&e),
    }
}

// ---- local playback -----------------------------------------------------

/// The request came from this machine. Forwarded headers mean a proxy
/// hop: the loopback peer is the proxy, not proof the viewer is local.
fn local_request(w: &WReq) -> bool {
    if ["forwarded", "x-forwarded-for", "x-real-ip"].iter().any(|h| w.headers.contains_key(*h)) {
        return false;
    }
    w.peer.is_some_and(|p| p.ip().is_loopback())
}

fn localplay_get(app: &App, w: &WReq, _: &Verified) -> Out {
    let players: Option<Vec<String>> = if local_request(w) {
        app.registry
            .call_one(localplay::CAP, "players", json!({}))
            .ok()
            .and_then(|(v, _)| serde_json::from_value::<lain_api::LocalPlayers>(v).ok())
            .and_then(|p| p.players)
            .filter(|p| !p.is_empty())
    } else {
        None
    };
    Reply::ok(json!(lain_api::LocalPlayers { players })).into()
}

fn play_local(app: &App, w: &WReq, v: &Verified) -> Out {
    if !local_request(w) {
        return Reply::err(403, "local playback requires the browser and server on the same machine").into();
    }
    let input: lain_api::PlayLocalRequest = if w.req.body.is_empty() {
        Default::default()
    } else {
        match w.req.decode() {
            Ok(i) => i,
            Err(r) => return r.into(),
        }
    };
    let it = match item(app, w, v) {
        Ok(it) => it,
        Err(o) => return *o,
    };
    let language = app.auth().get_public(&v.user_id).map(|u| u.preferred_language).unwrap_or_default();
    let play = localplay::PlayInput {
        player: input.player,
        user_id: v.user_id.clone(),
        language,
        item: it,
        hidden_libraries: app.hidden_libraries(v),
    };
    match app.registry.call_one(localplay::CAP, "play", serde_json::to_value(&play).unwrap_or_default()) {
        Ok((out, _)) => Reply::ok(out).into(),
        Err(e) => {
            let status = match e.code.as_str() {
                "invalid-message" => 400,
                "busy" | "not-found" => 409,
                "dependency-unavailable" => 503,
                _ => 500,
            };
            Reply::err(status, &e.to_string()).into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playlist_rewrite_is_byte_exact() {
        let input = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:6\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:6.000,\nseg00000.m4s\n#EXTINF:6.000,\nseg00001.m4s\n#EXT-X-ENDLIST\n";
        let want = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:6\n#EXT-X-MAP:URI=\"init.mp4?session=sess-1&token=tok-1\"\n#EXTINF:6.000,\nseg00000.m4s?session=sess-1&token=tok-1\n#EXTINF:6.000,\nseg00001.m4s?session=sess-1&token=tok-1\n#EXT-X-ENDLIST\n";
        assert_eq!(rewrite_playlist(input, "sess-1", "tok-1"), want);
    }

    #[test]
    fn map_lines_without_a_uri_stay() {
        for line in ["#EXT-X-MAP:BYTERANGE=\"1024@0\"", "#EXT-X-MAP:URI=\"init.mp4"] {
            let out = rewrite_playlist(&format!("#EXTM3U\n{line}\nseg00000.m4s\n"), "sess-2", "tok-2");
            assert!(out.contains(&format!("{line}\n")), "{out}");
            assert!(out.contains("seg00000.m4s?session=sess-2&token=tok-2"), "{out}");
        }
    }

    #[test]
    fn empty_token_and_existing_query() {
        let out = rewrite_playlist("#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\nseg00000.m4s\n", "sess-3", "");
        assert!(out.contains("URI=\"init.mp4?session=sess-3\"") && !out.contains("token="), "{out}");
        assert_eq!(
            sign_uri("seg00000.m4s?v=2&name=a%20b", "sess-4", "tok-4"),
            "seg00000.m4s?name=a+b&session=sess-4&token=tok-4&v=2"
        );
    }

    #[test]
    fn foreign_uris_are_never_signed() {
        for u in ["https://cdn.example/seg.m4s", "//cdn.example/seg.m4s", "/abs/seg.m4s", "data:text/plain,x"] {
            assert_eq!(sign_uri(u, "s", "t"), u);
        }
        assert_eq!(sign_uri("seg.m4s?token=old", "s", "t"), "seg.m4s?session=s&token=t");
    }

    #[test]
    fn bitrate_caps() {
        assert_eq!(effective_bitrate(0, 0), 0);
        assert_eq!(effective_bitrate(0, 800), 800);
        assert_eq!(effective_bitrate(500, 0), 500);
        assert_eq!(effective_bitrate(500, 800), 500);
        assert_eq!(effective_bitrate(900, 800), 800);
    }

    #[test]
    fn content_types() {
        assert_eq!(content_type("/m/a.MKV"), "video/x-matroska");
        assert_eq!(content_type("/m/a.opus"), "audio/ogg");
        assert_eq!(content_type("/m/a"), "application/octet-stream");
    }
}
