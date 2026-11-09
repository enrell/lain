//! The acquire module's routes (`/api/acquire/*`) and the download
//! manager's (`/api/downloads*`), with Go's shapes, status codes and
//! error texts. Thin: the work lives in `lain-acquire` and
//! `lain-downloads`; this file wires them to the registry (release
//! parse, `lain.indexer@1`, `lain.media.probe@1`, `lain.subtitle@1`, the
//! metadata episode count), the libraries, the catalog and the scan
//! controller, and turns their changes into `/api/events`. Every route
//! is admin-only (A-9): acquisition makes the server contact remote
//! hosts and write to disk.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use serde_json::{Value, json};

use lain_acquire::error::{AcqError, CODE_INVALID, CODE_NOT_FOUND, CODE_UNSUPPORTED};
use lain_acquire::subtitles::{ProbeInfo, ProbeStream, SubtitleTarget, parse_languages};
use lain_acquire::{Deps, GrabRequest, Manager, SearchQuery};
use lain_api::acquire::*;
use lain_api::downloads::{DownloadAdd, DownloadSettings, DownloadsView};
use lain_api::{CatalogItem, Library, MediaInfo, Release};
use lain_auth::Verified;
use lain_core::Error;
use lain_downloads::AddInput;
use lain_identify::CAP_RELEASE_PARSE;

use crate::app::App;
use crate::catalog::CAP_CATALOG_READ;
use crate::events::Scope;
use crate::gateway::{DELETE, GET, Gate, PATCH, POST, PUT, Reply, Req, route};

/// The lain-parser model's provider id (Go `release.ModelID`).
const PARSER_MODEL_ID: &str = "lain-release-model";

pub fn routes() -> Router<Arc<App>> {
    let a = Gate::Admin;
    Router::new()
        .route("/api/acquire/settings", route(GET, a, settings_get).merge(route(PUT, a, settings_put)))
        .route("/api/acquire/indexers", route(GET, a, indexers).merge(route(POST, a, indexer_create)))
        .route("/api/acquire/indexers/{id}", route(PATCH, a, indexer_patch).merge(route(DELETE, a, indexer_delete)))
        .route("/api/acquire/indexers/{id}/test", route(POST, a, indexer_test))
        .route("/api/acquire/search", route(GET, a, search))
        .route("/api/acquire/parse", route(GET, a, parse))
        .route("/api/acquire/grabs", route(GET, a, grabs).merge(route(POST, a, grab)))
        .route("/api/acquire/grabs/{id}/{action}", route(POST, a, grab_action))
        .route("/api/acquire/grabs/{id}", route(DELETE, a, grab_delete))
        .route("/api/acquire/profiles", route(GET, a, profiles).merge(route(POST, a, profile_create)))
        .route("/api/acquire/profiles/{id}", route(PUT, a, profile_update).merge(route(DELETE, a, profile_delete)))
        .route("/api/acquire/monitored", route(GET, a, monitored).merge(route(POST, a, monitored_create)))
        .route(
            "/api/acquire/monitored/{id}",
            route(GET, a, monitored_get).merge(route(PUT, a, monitored_update)).merge(route(
                DELETE,
                a,
                monitored_delete,
            )),
        )
        .route("/api/acquire/monitored/{id}/wanted", route(GET, a, wanted_one))
        .route("/api/acquire/monitored/{id}/search", route(POST, a, monitored_search))
        .route(
            "/api/acquire/monitored/{id}/subtitles",
            route(GET, a, monitored_subtitles).merge(route(POST, a, monitored_subtitle_pass)),
        )
        .route("/api/acquire/wanted", route(GET, a, wanted))
        .route("/api/acquire/rss", route(POST, a, rss))
        .route("/api/acquire/automation", route(GET, a, automation))
        .route("/api/acquire/blocklist", route(GET, a, blocklist))
        .route("/api/acquire/blocklist/{id}", route(DELETE, a, unblock))
        .route("/api/acquire/replaced", route(GET, a, replaced))
        .route("/api/acquire/replaced/purge", route(POST, a, purge))
        .route(
            "/api/acquire/subtitle-providers",
            route(GET, a, sub_providers).merge(route(POST, a, sub_provider_create)),
        )
        .route(
            "/api/acquire/subtitle-providers/{id}",
            route(PUT, a, sub_provider_update).merge(route(DELETE, a, sub_provider_delete)),
        )
        .route(
            "/api/acquire/items/{id}/subtitles",
            route(GET, a, item_subtitle_search).merge(route(POST, a, item_subtitle_download)),
        )
        .route("/api/acquire/items/{id}/subtitles/status", route(GET, a, item_subtitle_status))
        .route("/api/acquire/items/{id}/sidecars/{name}", route(DELETE, a, sidecar_remove))
        .route("/api/acquire/subtitles", route(GET, a, subtitle_ledger))
        .route(
            "/api/acquire/subtitle-refusals",
            route(GET, a, subtitle_refusals).merge(route(DELETE, a, subtitle_refusals_clear)),
        )
        .route("/api/acquire/subtitle-refusals/{provider}/{file}", route(DELETE, a, subtitle_refusal_clear))
        .route("/api/downloads", route(GET, a, downloads_list).merge(route(POST, a, download_add)))
        .route(
            "/api/downloads/settings",
            route(GET, a, download_settings_get).merge(route(PUT, a, download_settings_put)),
        )
        .route("/api/downloads/cleanup", route(POST, a, download_cleanup))
        .route("/api/downloads/{id}", route(GET, a, download_get).merge(route(DELETE, a, download_remove)))
        .route("/api/downloads/{id}/{action}", route(POST, a, download_action))
}

// ---- wiring ---------------------------------------------------------------

fn core_err(e: Error) -> AcqError {
    AcqError { code: e.code, msg: e.msg }
}

impl App {
    /// `lain.release.parse@1`; the tokenizer answers if every bound
    /// provider declines or the capability is withdrawn.
    pub fn parse_release(&self, name: &str, kind: &str) -> Release {
        let accepted = |v: &Value| v["title"].as_str().is_some_and(|t| !t.is_empty());
        match self.registry.call_first(CAP_RELEASE_PARSE, "parse", json!({"name": name, "kind": kind}), accepted) {
            Ok(Some((out, _))) => serde_json::from_value(out).unwrap_or_else(|_| lain_identify::tokenize(name, kind)),
            Ok(None) => lain_identify::tokenize(name, kind),
            Err(e) => {
                tracing::debug!(err = %e, "release parse fell back");
                lain_identify::tokenize(name, kind)
            }
        }
    }

    fn indexer_call(&self, op: &str, input: Value) -> Result<Value, AcqError> {
        match self.registry.call_first(CAP_INDEXER, op, input, |_| true) {
            Ok(Some((out, _))) => Ok(out),
            Ok(None) => Err(AcqError::new(CODE_UNSUPPORTED, "no indexer provider speaks this protocol")),
            Err(e) => Err(core_err(e)),
        }
    }

    fn subtitle_call(&self, op: &str, input: Value) -> Result<Value, AcqError> {
        match self.registry.call_first(CAP_SUBTITLE, op, input, |_| true) {
            Ok(Some((out, _))) => Ok(out),
            Ok(None) => Err(AcqError::new(CODE_UNSUPPORTED, "no subtitle provider speaks this kind")),
            Err(e) => Err(core_err(e)),
        }
    }

    /// Every item of one library (the catalog's `list`).
    pub fn library_items(&self, library_id: &str) -> Vec<CatalogItem> {
        let Ok((out, _)) = self.registry.call_one(CAP_CATALOG_READ, "list", json!({})) else { return Vec::new() };
        let items: Vec<CatalogItem> = serde_json::from_value(out).unwrap_or_default();
        items.into_iter().filter(|it| it.library_id == library_id).collect()
    }

    /// The closest-titled metadata candidate's episode total (A-18);
    /// 0 when unknown.
    fn episode_count(&self, title: &str, kind: &str) -> i64 {
        use lain_metadata::{CAP_METADATA_RESOLVE, CAP_METADATA_SEARCH, MetadataCandidate, MetadataRecord};
        let input = json!({"query": title, "kind": kind, "limit": 5});
        let Ok((outs, _)) = self.registry.call_merge_report(CAP_METADATA_SEARCH, "search", input) else { return 0 };
        let lists: Vec<Vec<MetadataCandidate>> =
            outs.into_iter().filter_map(|(_, v)| serde_json::from_value(v).ok()).collect();
        let cands = lain_metadata::merge::merge_candidates(title, &lists, 10);
        let key = crate::catalog::title_key(title);
        let Some(best) = cands.iter().find(|c| crate::catalog::title_key(&c.title) == key).or(cands.first()) else {
            return 0;
        };
        let input = json!({"provider": best.provider, "remote_id": best.remote_id});
        self.registry
            .invoke_provider(CAP_METADATA_RESOLVE, &best.provider, "resolve", input)
            .ok()
            .and_then(|v| serde_json::from_value::<MetadataRecord>(v).ok())
            .map(|r| r.episodes)
            .unwrap_or(0)
    }

    fn probe_info(&self, path: &Path) -> Result<ProbeInfo, AcqError> {
        let (out, _) = self
            .registry
            .call_one(lain_watch::CAP_MEDIA_PROBE, "probe", json!({"file_path": path}))
            .map_err(core_err)?;
        let info: MediaInfo = serde_json::from_value(out).unwrap_or_default();
        Ok(ProbeInfo {
            duration: info.duration,
            streams: info
                .streams
                .into_iter()
                .map(|s| ProbeStream { kind: s.kind, language: s.language, forced: s.forced })
                .collect(),
        })
    }

    /// Builds and starts the acquisition manager. A failure never blocks
    /// boot: the routes answer 503 instead.
    pub(crate) fn start_acquire(self: &Arc<Self>, listen_port: Option<u16>, tick: Duration) {
        let w = Arc::downgrade(self);
        let mut d = Deps::new(self.store.clone(), &self.data_dir, self.acquire_http.clone());
        d.engine = Some(lain_torrent::Config {
            http: Some(Arc::new(lain_acquire::net::TrackerHttp(self.http.clone()))),
            ..lain_torrent::Config::default()
        });
        let w1 = w.clone();
        d.parse = Arc::new(move |name: &str, kind: &str| match w1.upgrade() {
            Some(a) => a.parse_release(name, kind),
            None => lain_identify::tokenize(name, kind),
        });
        let w1 = w.clone();
        d.search = Arc::new(move |i: &IndexerSearchInput| {
            let a = w1.upgrade().ok_or_else(|| AcqError::new("unavailable", "server stopping"))?;
            let out: IndexerSearchOutput =
                serde_json::from_value(a.indexer_call("search", json!(i))?).unwrap_or_default();
            Ok(out.results)
        });
        let w1 = w.clone();
        d.caps = Arc::new(move |ix: &Indexer| {
            let a = w1.upgrade().ok_or_else(|| AcqError::new("unavailable", "server stopping"))?;
            let v = a.indexer_call("caps", json!({"indexer": ix}))?;
            serde_json::from_value(v).map_err(|e| AcqError::new("indexer-error", e.to_string()))
        });
        let dl = self.downloads.clone();
        d.budget = Some(Arc::new(move || (dl.settings().limits, dl.usage().used_bytes)));
        let w1 = w.clone();
        d.library = Arc::new(move |id: &str| w1.upgrade().and_then(|a| a.libraries.get(id)));
        let w1 = w.clone();
        d.libraries =
            Arc::new(move || w1.upgrade().map(|a| a.libraries.list().unwrap_or_default()).unwrap_or_default());
        let w1 = w.clone();
        d.items = Some(Arc::new(move |lib: &str| w1.upgrade().map(|a| a.library_items(lib)).unwrap_or_default()));
        let w1 = w.clone();
        d.titles = Some(Arc::new(move |lib: &str| {
            let Some(a) = w1.upgrade() else { return Vec::new() };
            let mut t: Vec<String> = a.library_items(lib).into_iter().map(|it| it.title).collect();
            t.sort();
            t.dedup();
            t
        }));
        let w1 = w.clone();
        d.rescan = Some(Arc::new(move |lib: &str| {
            if let Some(a) = w1.upgrade() {
                a.scanner.scan_library(lib);
            }
        }));
        let w1 = w.clone();
        d.episode_count =
            Some(Arc::new(move |t: &str, k: &str| w1.upgrade().map(|a| a.episode_count(t, k)).unwrap_or(0)));
        let w1 = w.clone();
        d.subtitles.probe = Some(Arc::new(move |p: &Path| {
            let a = w1.upgrade().ok_or_else(|| AcqError::new("unavailable", "server stopping"))?;
            a.probe_info(p)
        }));
        let w1 = w.clone();
        d.subtitles.search = Some(Arc::new(move |i: &SubtitleSearchInput| {
            let a = w1.upgrade().ok_or_else(|| AcqError::new("unavailable", "server stopping"))?;
            let out: SubtitleSearchOutput =
                serde_json::from_value(a.subtitle_call("search", json!(i))?).unwrap_or_default();
            Ok(out.candidates)
        }));
        let w1 = w.clone();
        d.subtitles.download = Some(Arc::new(move |i: &SubtitleDownloadInput| {
            let a = w1.upgrade().ok_or_else(|| AcqError::new("unavailable", "server stopping"))?;
            serde_json::from_value(a.subtitle_call("download", json!(i))?)
                .map_err(|e| AcqError::new("fetch-failed", e.to_string()))
        }));
        let w1 = w.clone();
        d.on_grab = Some(Arc::new(move |g: &Grab| {
            if let Some(a) = w1.upgrade() {
                a.events.publish(Scope::Admins, "grab-changed", json!(g));
            }
        }));
        d.tick = tick;
        let mut initial = lain_acquire::default_settings(&self.data_dir);
        if let Some(p) = listen_port {
            initial.listen_port = p as i64;
            d.initial = Some(initial.clone());
        }
        let m = match Manager::open(d.clone()) {
            Ok(m) => m,
            Err(e) => {
                // Most likely the listen port is taken: a free port keeps
                // acquisition working, and the log says so.
                tracing::warn!(err = %e, "acquisition engine failed to start; retrying on a free port");
                initial.listen_port = 0;
                d.initial = Some(initial);
                match Manager::open(d) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::error!(err = %e, "acquisition disabled");
                        return;
                    }
                }
            }
        };
        let _ = self.acquire.set(m);
    }

    /// Download events and the rescan after a finished download.
    pub(crate) fn hook_downloads(self: &Arc<Self>) {
        let w = Arc::downgrade(self);
        self.downloads.on_change(Arc::new(move |j| {
            if let Some(a) = w.upgrade() {
                a.events.publish(Scope::Admins, "download-changed", json!(j));
            }
        }));
        let w = Arc::downgrade(self);
        self.downloads.on_done(Arc::new(move |j| {
            tracing::info!(download = %j.id, library = %j.library_id, bytes = j.bytes, "download finished");
            if !j.library_id.is_empty()
                && let Some(a) = w.upgrade()
            {
                a.scanner.scan_library(&j.library_id);
            }
        }));
    }

    fn parser_healthy(&self) -> bool {
        self.registry
            .ordered(CAP_RELEASE_PARSE)
            .map(|(provs, _)| provs.iter().any(|p| p.id() == PARSER_MODEL_ID && p.health().is_ok()))
            .unwrap_or(false)
    }
}

// ---- helpers --------------------------------------------------------------

fn acq_err(e: AcqError) -> Reply {
    let code = if e.code.is_empty() || e.code == "store-error" { "error".to_string() } else { e.code.clone() };
    Reply::json(e.status(), json!({"error": e.msg, "code": code}))
}

fn with_acq(app: &App, f: impl FnOnce(&Manager) -> Reply) -> Reply {
    match app.acquire.get() {
        Some(m) => f(m),
        None => {
            Reply::json(503, json!({"error": "acquisition is unavailable; see the server log", "code": "unavailable"}))
        }
    }
}

fn ok_or(r: Result<Value, AcqError>) -> Reply {
    match r {
        Ok(v) => Reply::ok(v),
        Err(e) => acq_err(e),
    }
}

fn created_or(r: Result<Value, AcqError>) -> Reply {
    match r {
        Ok(v) => Reply::json(201, v),
        Err(e) => acq_err(e),
    }
}

fn removed(r: Result<(), AcqError>) -> Reply {
    ok_or(r.map(|_| json!({"removed": true})))
}

fn atoi(s: &str) -> i64 {
    s.parse().unwrap_or(0)
}

fn decode<T: for<'de> serde::Deserialize<'de>>(req: &Req) -> Result<T, Reply> {
    req.decode()
}

fn indexer_view(m: &Manager, ix: &Indexer) -> IndexerView {
    IndexerView { indexer: ix.public(), has_api_key: m.has_api_key(&ix.id) }
}

fn acq_view(app: &App, m: &Manager, s: AcquireSettings) -> Value {
    json!(AcquireView { settings: s, usage: m.usage(), port: m.port(), parser: app.parser_healthy() })
}

// ---- settings and indexers --------------------------------------------------

fn settings_get(app: &App, _: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| Reply::ok(acq_view(app, m, m.settings())))
}

fn settings_put(app: &App, req: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| {
        // Go decodes over the current settings: absent fields stay.
        let patch: Value = match decode(req) {
            Ok(v) => v,
            Err(r) => return r,
        };
        let mut cur = json!(m.settings());
        if let (Some(c), Value::Object(p)) = (cur.as_object_mut(), patch) {
            c.extend(p);
        }
        let Ok(s) = serde_json::from_value::<AcquireSettings>(cur) else { return Reply::err(400, "invalid json") };
        match m.set_settings(s) {
            Ok(s) => Reply::ok(acq_view(app, m, s)),
            Err(e) => acq_err(e),
        }
    })
}

fn indexers(app: &App, _: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| {
        let list: Vec<IndexerView> = m.indexers().iter().map(|ix| indexer_view(m, ix)).collect();
        Reply::ok(json!({"indexers": list}))
    })
}

fn indexer_create(app: &App, req: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| {
        let input: IndexerInput = match decode(req) {
            Ok(v) => v,
            Err(r) => return r,
        };
        created_or(m.create_indexer(&input).map(|ix| json!(indexer_view(m, &ix))))
    })
}

fn indexer_patch(app: &App, req: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| {
        let input: IndexerInput = match decode(req) {
            Ok(v) => v,
            Err(r) => return r,
        };
        ok_or(m.update_indexer(req.param("id"), &input).map(|ix| json!(indexer_view(m, &ix))))
    })
}

fn indexer_delete(app: &App, req: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| removed(m.delete_indexer(req.param("id"))))
}

/// 200 with the recorded health either way: a failing indexer is a
/// result of the test, not a failed request.
fn indexer_test(app: &App, req: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| {
        let (ix, err) = m.test_indexer(req.param("id"));
        let ix = match ix {
            Ok(ix) => ix,
            Err(e) => return acq_err(e),
        };
        let mut out = json!({"indexer": indexer_view(m, &ix), "ok": err.is_none()});
        if err.is_some() {
            out["error"] = json!(ix.last_error);
        }
        Reply::ok(out)
    })
}

fn search(app: &App, req: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| {
        let ids = req.query("indexers");
        let q = SearchQuery {
            query: req.query("q").into(),
            kind: req.query("kind").into(),
            season: atoi(req.query("season")),
            episode: atoi(req.query("episode")),
            year: atoi(req.query("year")),
            indexer_ids: if ids.is_empty() { Vec::new() } else { ids.split(',').map(str::to_string).collect() },
        };
        ok_or(m.search(&q).map(|r| json!(r)))
    })
}

fn parse(app: &App, req: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| {
        let name = req.query("name");
        if name.trim().is_empty() || name.len() > 1024 {
            return acq_err(AcqError::new(CODE_INVALID, "name required (up to 1024 bytes)"));
        }
        Reply::ok(json!(m.parse(name, req.query("kind"))))
    })
}

// ---- grabs ----------------------------------------------------------------

fn grabs(app: &App, _: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| Reply::ok(json!(GrabList { grabs: m.grabs(), usage: m.usage() })))
}

fn grab(app: &App, req: &Req, v: &Verified) -> Reply {
    with_acq(app, |m| {
        let input: GrabInput = match decode(req) {
            Ok(v) => v,
            Err(r) => return r,
        };
        created_or(m.grab(GrabRequest { input, created_by: v.user_id.clone(), ..Default::default() }).map(|g| json!(g)))
    })
}

fn grab_action(app: &App, req: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| {
        let id = req.param("id");
        let r = match req.param("action") {
            "pause" => m.pause(id),
            "resume" => m.resume(id),
            "import" => m.retry_import(id),
            _ => return acq_err(AcqError::new(CODE_NOT_FOUND, "unknown action")),
        };
        ok_or(r.map(|g| json!(g)))
    })
}

fn grab_delete(app: &App, req: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| {
        let id = req.param("id");
        let r = m.remove(id, req.query("data") == "1");
        if r.is_ok() {
            app.events.publish(Scope::Admins, "grab-changed", json!({"id": id, "removed": true}));
        }
        removed(r)
    })
}

// ---- profiles, monitored titles, automation ---------------------------------

fn profiles(app: &App, _: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| Reply::ok(json!({"profiles": m.profiles()})))
}

fn profile_create(app: &App, req: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| match decode::<Profile>(req) {
        Ok(p) => created_or(m.create_profile(p).map(|p| json!(p))),
        Err(r) => r,
    })
}

fn profile_update(app: &App, req: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| match decode::<Profile>(req) {
        Ok(p) => ok_or(m.update_profile(req.param("id"), p).map(|p| json!(p))),
        Err(r) => r,
    })
}

fn profile_delete(app: &App, req: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| removed(m.delete_profile(req.param("id"))))
}

fn monitored(app: &App, _: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| Reply::ok(json!({"monitored": m.monitored_list()})))
}

fn monitored_create(app: &App, req: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| match decode::<Monitored>(req) {
        Ok(x) => created_or(m.create_monitored(x).map(|x| json!(x))),
        Err(r) => r,
    })
}

fn monitored_get(app: &App, req: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| ok_or(m.monitored(req.param("id")).map(|x| json!(x))))
}

fn monitored_update(app: &App, req: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| match decode::<Monitored>(req) {
        Ok(x) => ok_or(m.update_monitored(req.param("id"), x).map(|x| json!(x))),
        Err(r) => r,
    })
}

fn monitored_delete(app: &App, req: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| removed(m.delete_monitored(req.param("id"))))
}

fn wanted_one(app: &App, req: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| ok_or(m.wanted_for(req.param("id")).map(|w| json!(w))))
}

fn wanted(app: &App, _: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| {
        let titles: Vec<WantedRow> = m
            .monitored_list()
            .into_iter()
            .filter_map(|mon| {
                let w = m.wanted_for(&mon.id).ok()?;
                Some(WantedRow {
                    missing_count: w.missing.len() as i64,
                    upgrade_count: w.upgrades.len() as i64,
                    monitored: mon,
                    wanted: w,
                })
            })
            .collect();
        Reply::ok(json!(WantedOverview { titles, automation: m.automation_status() }))
    })
}

fn monitored_search(app: &App, req: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| ok_or(m.search_monitored(req.param("id")).map(|r| json!(r))))
}

fn rss(app: &App, _: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| ok_or(m.rss_sync().map(|r| json!(r))))
}

fn automation(app: &App, _: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| Reply::ok(json!(m.automation_status())))
}

fn blocklist(app: &App, _: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| Reply::ok(json!({"blocklist": m.blocklist()})))
}

fn unblock(app: &App, req: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| removed(m.unblock(req.param("id"))))
}

fn replaced(app: &App, _: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| Reply::ok(json!({"files": m.replaced()})))
}

fn purge(app: &App, _: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| ok_or(m.purge_replaced().map(|n| json!({"deleted": n}))))
}

// ---- subtitles ------------------------------------------------------------

fn sub_view(m: &Manager, p: &SubtitleProvider) -> SubtitleProviderView {
    let (k, pw) = m.subtitle_provider_secrets(&p.id);
    SubtitleProviderView { provider: p.public(), has_api_key: k, has_password: pw }
}

fn sub_providers(app: &App, _: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| {
        let list: Vec<SubtitleProviderView> = m.subtitle_providers().iter().map(|p| sub_view(m, p)).collect();
        Reply::ok(json!({"providers": list}))
    })
}

fn sub_provider_create(app: &App, req: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| match decode::<SubtitleProviderInput>(req) {
        Ok(i) => created_or(m.create_subtitle_provider(&i).map(|p| json!(sub_view(m, &p)))),
        Err(r) => r,
    })
}

fn sub_provider_update(app: &App, req: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| match decode::<SubtitleProviderInput>(req) {
        Ok(i) => ok_or(m.update_subtitle_provider(req.param("id"), &i).map(|p| json!(sub_view(m, &p)))),
        Err(r) => r,
    })
}

fn sub_provider_delete(app: &App, req: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| removed(m.delete_subtitle_provider(req.param("id"))))
}

/// The item, if this session may see it (D-156), else Go's typed 404.
fn item(app: &App, v: &Verified, id: &str) -> Result<CatalogItem, Reply> {
    app.visible_item(v, id).ok_or_else(|| Reply::json(404, json!({"error": "unknown item", "code": CODE_NOT_FOUND})))
}

fn item_subtitle_search(app: &App, req: &Req, v: &Verified) -> Reply {
    with_acq(app, |m| {
        let it = match item(app, v, req.param("id")) {
            Ok(it) => it,
            Err(r) => return r,
        };
        let p = match m.profile_for_item(&it) {
            Ok(p) => p,
            Err(e) => return acq_err(e),
        };
        let raw = req.query("languages").trim();
        let langs = if raw.is_empty() {
            p.subtitle_languages.clone()
        } else {
            match parse_languages(raw) {
                Ok(l) => l,
                Err(e) => return acq_err(e),
            }
        };
        if langs.is_empty() {
            return acq_err(AcqError::new(
                CODE_INVALID,
                "name the languages to search, or set them in the quality profile",
            ));
        }
        let t = SubtitleTarget {
            path: it.file_path.clone().into(),
            kind: it.kind.clone(),
            title: it.title.clone(),
            season: it.season,
            episode: it.episode,
            year: it.year,
        };
        ok_or(m.search_subtitles(&t, &p, &langs).map(|choices| json!({"choices": choices, "languages": langs})))
    })
}

fn item_subtitle_download(app: &App, req: &Req, v: &Verified) -> Reply {
    with_acq(app, |m| {
        let it = match item(app, v, req.param("id")) {
            Ok(it) => it,
            Err(r) => return r,
        };
        let take: SubtitleTake = match decode(req) {
            Ok(t) => t,
            Err(r) => return r,
        };
        ok_or(m.download_subtitle(Path::new(&it.file_path), &take.candidate, take.replace).map(|r| json!(r)))
    })
}

fn monitored_subtitles(app: &App, req: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| ok_or(m.subtitle_status(req.param("id")).map(|f| json!({"files": f}))))
}

fn monitored_subtitle_pass(app: &App, req: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| ok_or(m.subtitles_for_monitored(req.param("id")).map(|n| json!({"written": n}))))
}

fn subtitle_ledger(app: &App, _: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| Reply::ok(json!({"subtitles": m.subtitle_ledger()})))
}

fn item_subtitle_status(app: &App, req: &Req, v: &Verified) -> Reply {
    with_acq(app, |m| {
        let it = match item(app, v, req.param("id")) {
            Ok(it) => it,
            Err(r) => return r,
        };
        ok_or(m.item_subtitle_status(&it).map(|(f, langs)| json!({"file": f, "languages": langs})))
    })
}

/// Moves a sidecar into the holding folder; nothing is deleted.
fn sidecar_remove(app: &App, req: &Req, v: &Verified) -> Reply {
    with_acq(app, |m| {
        let it = match item(app, v, req.param("id")) {
            Ok(it) => it,
            Err(r) => return r,
        };
        ok_or(m.remove_sidecar(Path::new(&it.file_path), req.param("name")).map(|h| json!(h)))
    })
}

fn subtitle_refusals(app: &App, _: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| Reply::ok(json!({"refusals": m.subtitle_refusals()})))
}

fn subtitle_refusal_clear(app: &App, req: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| removed(m.clear_subtitle_refusal(req.param("provider"), req.param("file"))))
}

fn subtitle_refusals_clear(app: &App, _: &Req, _: &Verified) -> Reply {
    with_acq(app, |m| Reply::ok(json!({"cleared": m.clear_subtitle_refusals()})))
}

// ---- downloads ------------------------------------------------------------

/// Typed manager errors to statuses, the stable code next to the message.
fn dl_err(e: lain_downloads::Error) -> Reply {
    use lain_downloads::{CODE_DISK_FULL, CODE_INVALID, CODE_NOT_FOUND, CODE_QUOTA, CODE_STATE};
    let status = match e.code.as_str() {
        CODE_INVALID => 400,
        CODE_NOT_FOUND => 404,
        CODE_STATE => 409,
        CODE_QUOTA | CODE_DISK_FULL => 507,
        _ => 500,
    };
    Reply::json(status, json!({"error": e.msg, "code": e.code}))
}

fn downloads_list(app: &App, _: &Req, _: &Verified) -> Reply {
    let d = &app.downloads;
    Reply::ok(json!(DownloadsView { jobs: d.list(), usage: d.usage(), settings: d.settings() }))
}

fn download_add(app: &App, req: &Req, v: &Verified) -> Reply {
    let input: DownloadAdd = match decode(req) {
        Ok(i) => i,
        Err(r) => return r,
    };
    let mut add = AddInput { url: input.url, name: input.name, created_by: v.user_id.clone(), ..Default::default() };
    if !input.library_id.is_empty() {
        let Some(lib) = app.libraries.get(&input.library_id) else {
            return dl_err(lain_downloads::Error::new(lain_downloads::CODE_NOT_FOUND, "unknown library"));
        };
        let lib: Library = lib;
        add.dir = lib.path;
        add.library_id = lib.id;
    }
    match app.downloads.add(add) {
        Ok(j) => {
            tracing::info!(download = %j.id, library = %j.library_id, "download queued");
            Reply::json(201, json!(j))
        }
        Err(e) => dl_err(e),
    }
}

fn download_get(app: &App, req: &Req, _: &Verified) -> Reply {
    match app.downloads.get(req.param("id")) {
        Ok(j) => Reply::ok(json!(j)),
        Err(e) => dl_err(e),
    }
}

fn download_action(app: &App, req: &Req, _: &Verified) -> Reply {
    let id = req.param("id");
    let r = match req.param("action") {
        "pause" => app.downloads.pause(id),
        "resume" => app.downloads.resume(id),
        "cancel" => app.downloads.cancel(id),
        _ => return Reply::err(404, "unknown action"),
    };
    match r {
        Ok(j) => Reply::ok(json!(j)),
        Err(e) => dl_err(e),
    }
}

fn download_remove(app: &App, req: &Req, _: &Verified) -> Reply {
    let id = req.param("id");
    match app.downloads.remove(id) {
        Ok(()) => {
            app.events.publish(Scope::Admins, "download-changed", json!({"id": id, "removed": true}));
            Reply::ok(json!({"status": "ok"}))
        }
        Err(e) => dl_err(e),
    }
}

fn download_settings_get(app: &App, _: &Req, _: &Verified) -> Reply {
    Reply::ok(json!(app.downloads.settings()))
}

fn download_settings_put(app: &App, req: &Req, _: &Verified) -> Reply {
    let s: DownloadSettings = match decode(req) {
        Ok(s) => s,
        Err(r) => return r,
    };
    match app.downloads.set_settings(s) {
        Ok(out) => {
            tracing::info!(
                max_bytes = out.limits.max_bytes,
                min_free_bytes = out.limits.min_free_bytes,
                concurrency = out.concurrency,
                "download settings saved"
            );
            Reply::ok(json!(out))
        }
        Err(e) => dl_err(e),
    }
}

fn download_cleanup(app: &App, _: &Req, _: &Verified) -> Reply {
    Reply::ok(json!(app.downloads.cleanup()))
}
