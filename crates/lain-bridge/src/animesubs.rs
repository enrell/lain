//! `lain-subtitle-animesubs`: AnimeSubs' local server behind
//! `lain.subtitle.translate@1` (D-228). AnimeSubs stays a standalone app
//! with its own users; this bridge finds its server through the
//! discovery file in its home (`$ANIMESUBS_HOME`, else
//! `<config dir>/com.enrell.animesubs`: `server.json` + `server-token`)
//! and drives `/v1/jobs`:
//!
//! | op | AnimeSubs |
//! |---|---|
//! | `start` (`TranslateStartInput`) | `POST /v1/jobs` (`translate_subtitle`) |
//! | `status` (`JobRef`) | `GET /v1/jobs/{id}` |
//! | `cancel` (`JobRef`) | `POST /v1/jobs/{id}/cancel` |
//!
//! It declines (unhealthy, so the binding skips it) while the server is
//! not running; a subtitle format AnimeSubs cannot read is
//! `unsupported-format`, which lets the next translator try.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};

use lain_api::{JobError, JobRef, TranslateJob, TranslateModel, TranslateStartInput};
use lain_core::{Error, Provider};

use crate::discovery::{self, Locator, Server};
use crate::http1::Response;

pub const ID: &str = "lain-subtitle-animesubs";
pub const CAP_SUBTITLE_TRANSLATE: &str = "lain.subtitle.translate@1";

const SERVER_TTL: Duration = Duration::from_secs(2);
const TIMEOUT: Duration = Duration::from_secs(10);

/// The bridge. `default_model` (from the manifest: kind, model and base
/// URL, never a key) is used when a start names no model; the key then
/// comes from AnimeSubs' own environment (`OPENAI_API_KEY`, …).
pub struct AnimeSubs {
    locator: Locator,
    default_model: Option<TranslateModel>,
    server: Mutex<Option<(Instant, Result<Server, Error>)>>,
}

impl AnimeSubs {
    pub fn new(locator: Locator, default_model: Option<TranslateModel>) -> Self {
        AnimeSubs { locator, default_model, server: Mutex::new(None) }
    }

    /// The default locator: `home` (else `$ANIMESUBS_HOME`, else the
    /// config dir), `ANIMESUBS_TOKEN` when set.
    pub fn locator(home: Option<PathBuf>, url: Option<String>) -> Locator {
        let home = home
            .or_else(|| discovery::env("ANIMESUBS_HOME").map(PathBuf::from))
            .or_else(|| discovery::config_dir().map(|d| d.join("com.enrell.animesubs")));
        Locator {
            service: "animesubs",
            health_path: "/v1/health",
            home,
            token_file: "server-token",
            url,
            token: discovery::env("ANIMESUBS_TOKEN"),
        }
    }

    fn server(&self) -> Result<Server, Error> {
        let mut g = self.server.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, s)) = g.as_ref()
            && at.elapsed() < SERVER_TTL
        {
            return s.clone();
        }
        let s = self.locator.locate(Duration::from_secs(3));
        *g = Some((Instant::now(), s.clone()));
        s
    }

    fn forget(&self) {
        *self.server.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    fn call(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, Error> {
        let s = self.server()?;
        let res = s.call(method, path, body, TIMEOUT).inspect_err(|_| self.forget())?;
        if (200..300).contains(&res.status) {
            return res.json().ok_or_else(|| Error::unavailable("animesubs sent bad json"));
        }
        if res.status == 401 {
            self.forget();
        }
        Err(app_error(&res))
    }
}

/// AnimeSubs' `{"error": {code, message}}` as a lain error.
fn app_error(res: &Response) -> Error {
    let v = res.json().unwrap_or(Value::Null);
    let code = v["error"]["code"].as_str().unwrap_or("");
    let msg = v["error"]["message"].as_str().unwrap_or("animesubs error").to_string();
    let lain = match (res.status, code) {
        (401, _) => "auth-failed",
        (404, _) => "not-found",
        (409, "not_cancellable") => "not-cancellable",
        (409, _) => "already-exists",
        (_, "unsupported") => "unsupported-format",
        (400 | 422, _) => "invalid-message",
        (_, "provider_error") => "provider-error",
        (_, "missing_tool") => "dependency-unavailable",
        _ => "dependency-unavailable",
    };
    Error::new(lain, format!("animesubs: {msg}"))
}

#[derive(Deserialize, Default)]
struct AppProgress {
    #[serde(default)]
    done: u64,
    #[serde(default)]
    total: u64,
}

#[derive(Deserialize, Default)]
struct AppError {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

#[derive(Deserialize, Default)]
struct AppJob {
    id: String,
    state: String,
    #[serde(default)]
    phase: String,
    #[serde(default)]
    progress: Option<AppProgress>,
    #[serde(default)]
    input: Option<String>,
    #[serde(default)]
    output: Option<String>,
    #[serde(default)]
    target_language: Option<String>,
    #[serde(default)]
    created_at_ms: Option<i64>,
    #[serde(default)]
    finished_at_ms: Option<i64>,
    #[serde(default)]
    error: Option<AppError>,
}

fn job(v: Value) -> Result<TranslateJob, Error> {
    let a: AppJob = serde_json::from_value(v).map_err(|e| Error::unavailable(format!("animesubs job: {e}")))?;
    let p = a.progress.unwrap_or_default();
    Ok(TranslateJob {
        job: a.id,
        provider: ID.into(),
        state: a.state,
        phase: a.phase,
        done: p.done,
        total: p.total,
        input: a.input.unwrap_or_default(),
        output: a.output.unwrap_or_default(),
        target_language: a.target_language.unwrap_or_default(),
        created_at_ms: a.created_at_ms.unwrap_or(0),
        finished_at_ms: a.finished_at_ms.unwrap_or(0),
        error: a.error.map(|e| JobError { code: e.code, msg: e.message }),
    })
}

fn job_id(input: Value) -> Result<String, Error> {
    let r: JobRef = serde_json::from_value(input).map_err(|e| Error::invalid(format!("JobRef required: {e}")))?;
    // Ids go into a path: the app's ids are `job-<hex>`.
    if r.job.is_empty()
        || r.job.len() > 128
        || !r.job.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(Error::invalid("job must be an AnimeSubs job id"));
    }
    Ok(r.job)
}

fn absolute(p: &str) -> bool {
    std::path::Path::new(p).is_absolute()
}

impl Provider for AnimeSubs {
    fn id(&self) -> &str {
        ID
    }

    fn capabilities(&self) -> Vec<String> {
        vec![CAP_SUBTITLE_TRANSLATE.into()]
    }

    fn health(&self) -> Result<(), Error> {
        self.server().map(|_| ())
    }

    fn invoke(&self, cap: &str, op: &str, input: Value) -> Result<Value, Error> {
        if cap != CAP_SUBTITLE_TRANSLATE {
            return Err(Error::invalid(format!("unsupported cap {cap}")));
        }
        let out = match op {
            "start" => {
                let i: TranslateStartInput = serde_json::from_value(input)
                    .map_err(|e| Error::invalid(format!("TranslateStartInput required: {e}")))?;
                if !absolute(&i.input) || (!i.output.is_empty() && !absolute(&i.output)) {
                    return Err(Error::invalid("input and output must be absolute paths"));
                }
                let lang = i.target_language.trim();
                if lang.is_empty() || lang.len() > 35 {
                    return Err(Error::invalid("target_language must be a BCP 47 tag"));
                }
                let model = i
                    .model
                    .or_else(|| self.default_model.clone())
                    .ok_or_else(|| Error::invalid("no translation model given and none configured (--model-kind)"))?;
                let mut provider = json!({"kind": model.kind});
                for (k, v) in [("model", &model.model), ("base_url", &model.base_url), ("api_key", &model.api_key)] {
                    if !v.is_empty() {
                        provider[k] = json!(v);
                    }
                }
                let mut spec = json!({
                    "kind": "translate_subtitle",
                    "input": i.input,
                    "target_language": lang,
                    "provider": provider,
                    "overwrite": i.overwrite,
                });
                if !i.output.is_empty() {
                    spec["output"] = json!(i.output);
                }
                job(self.call("POST", "/v1/jobs", Some(&spec))?)?
            }
            "status" => job(self.call("GET", &format!("/v1/jobs/{}", job_id(input)?), None)?)?,
            "cancel" => job(self.call("POST", &format!("/v1/jobs/{}/cancel", job_id(input)?), None)?)?,
            other => return Err(Error::invalid(format!("capability {cap} has no op {other:?}"))),
        };
        Ok(json!(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_map_to_lain_codes() {
        let r = |status, code: &str| Response {
            status,
            headers: vec![],
            body: serde_json::to_vec(&json!({"error": {"code": code, "message": "m"}})).unwrap(),
        };
        assert_eq!(app_error(&r(401, "unauthorized")).code, "auth-failed");
        assert_eq!(app_error(&r(404, "not_found")).code, "not-found");
        assert_eq!(app_error(&r(409, "not_cancellable")).code, "not-cancellable");
        assert_eq!(app_error(&r(422, "unsupported")).code, "unsupported-format");
        assert_eq!(app_error(&r(400, "bad_request")).code, "invalid-message");
        assert_eq!(app_error(&r(502, "provider_error")).code, "provider-error");
        assert_eq!(app_error(&r(500, "internal")).msg, "animesubs: m");
    }

    #[test]
    fn job_ids_are_checked() {
        assert_eq!(job_id(json!({"job": "job-3f9c"})).unwrap(), "job-3f9c");
        assert!(job_id(json!({"job": "../x"})).is_err());
        assert!(job_id(json!({"job": ""})).is_err());
        assert!(job_id(json!({})).is_err());
    }
}
