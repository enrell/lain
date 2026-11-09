//! `lain-acquire-anything`: the Anything daemon behind
//! `lain.acquire.extract@1` (D-230). Anything stays a standalone app
//! (its own queue, browser runtime and library layout); this bridge
//! finds the daemon through `~/.config/Anything/server.json` and
//! `server-token` (`ANYTHING_CONFIG_DIR`, `ANYTHING_TOKEN`) and maps:
//!
//! | op | Anything |
//! |---|---|
//! | `extract` (`ExtractInput`) | `POST /extract` |
//! | `enqueue` (`ExtractEnqueueInput`) | `POST /queue` (`options.format`, `options.output_dir`) |
//! | `status` (`JobRef`) | `GET /queue/{id}` |
//! | `cancel` (`JobRef`) | `DELETE /queue/{id}` |
//!
//! Stream URLs never leave the component: candidates carry ids and
//! qualities, and Anything downloads. A daemon that is not running (or
//! an older one without token auth, which has no `/health`) is
//! unhealthy, so the binding skips it.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use lain_api::{ExtractCandidate, ExtractEnqueueInput, ExtractInput, ExtractJob, ExtractOutput, JobError, JobRef};
use lain_core::{Error, Provider};

use crate::discovery::{self, Locator, Server};
use crate::http1::Response;

pub const ID: &str = "lain-acquire-anything";
pub const CAP_ACQUIRE_EXTRACT: &str = "lain.acquire.extract@1";

const SERVER_TTL: Duration = Duration::from_secs(2);
/// Anything gives one extraction 60 s (a browser fallback can be slow).
const EXTRACT_TIMEOUT: Duration = Duration::from_secs(75);
const TIMEOUT: Duration = Duration::from_secs(10);

pub struct Anything {
    locator: Locator,
    server: Mutex<Option<(Instant, Result<Server, Error>)>>,
}

impl Anything {
    pub fn new(locator: Locator) -> Self {
        Anything { locator, server: Mutex::new(None) }
    }

    pub fn locator(home: Option<PathBuf>, url: Option<String>) -> Locator {
        let home = home
            .or_else(|| discovery::env("ANYTHING_CONFIG_DIR").map(PathBuf::from))
            .or_else(|| discovery::home_dir().map(|h| h.join(".config").join("Anything")));
        Locator {
            service: "anything",
            health_path: "/health",
            home,
            token_file: "server-token",
            url,
            token: discovery::env("ANYTHING_TOKEN"),
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

    fn call(&self, method: &str, path: &str, body: Option<&Value>, timeout: Duration) -> Result<Value, Error> {
        let s = self.server()?;
        let res = s.call(method, path, body, timeout).inspect_err(|_| {
            *self.server.lock().unwrap_or_else(|e| e.into_inner()) = None;
        })?;
        if (200..300).contains(&res.status) {
            return res.json().ok_or_else(|| Error::unavailable("anything sent bad json"));
        }
        Err(app_error(&res))
    }
}

/// Anything's `{"error": "..."}` as a lain error.
fn app_error(res: &Response) -> Error {
    let msg = res.json().and_then(|v| v["error"].as_str().map(String::from)).unwrap_or_else(|| "anything error".into());
    let code = match res.status {
        401 | 403 => "auth-failed",
        404 => "not-found",
        400 => "invalid-message",
        503 => "dependency-unavailable",
        _ if msg.to_lowercase().contains("unsupported") || msg.contains("No extractor") => "unsupported-url",
        _ => "extract-failed",
    };
    Error::new(code, format!("anything: {msg}"))
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

fn i(v: &Value) -> i64 {
    v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)).unwrap_or(0)
}

fn extracted(v: &Value) -> ExtractOutput {
    let candidates = v["streams"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|st| ExtractCandidate {
                    id: s(&st["id"]),
                    kind: if st["is_audio_only"] == true {
                        "audio".into()
                    } else if st["is_video_only"] == true {
                        "video-only".into()
                    } else {
                        "video".into()
                    },
                    quality: s(&st["quality"]),
                    width: i(&st["width"]),
                    height: i(&st["height"]),
                    codec: s(&st["codec"]),
                    mime_type: s(&st["mime_type"]),
                    filesize: i(&st["filesize"]),
                })
                .collect()
        })
        .unwrap_or_default();
    let subtitles = v["subtitles"]
        .as_array()
        .map(|a| a.iter().map(|x| s(&x["language"])).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default();
    ExtractOutput {
        url: s(&v["webpage_url"]),
        title: s(&v["title"]),
        site: s(&v["site"]),
        duration_s: v["duration"].as_f64().unwrap_or(0.0),
        thumbnail: s(&v["thumbnail"]),
        content_class: s(&v["content_class"]),
        candidates,
        subtitles,
        provider: ID.into(),
    }
}

/// Anything's queue status as the contract's state.
fn state(status: &str) -> &'static str {
    match status {
        "pending" => "queued",
        "running" => "running",
        "paused" => "paused",
        "completed" => "succeeded",
        "failed" => "failed",
        "cancelled" => "cancelled",
        _ => "running",
    }
}

fn item(v: &Value) -> ExtractJob {
    let st = state(v["status"].as_str().unwrap_or("pending"));
    let err = s(&v["error"]);
    ExtractJob {
        job: match &v["id"] {
            Value::Number(n) => n.to_string(),
            other => s(other),
        },
        provider: ID.into(),
        state: st.into(),
        url: s(&v["url"]),
        title: s(&v["title"]),
        output_path: s(&v["output_path"]),
        downloaded_bytes: i(&v["downloaded_bytes"]),
        total_bytes: i(&v["total_bytes"]),
        error: (st == "failed" && !err.is_empty()).then(|| JobError { code: "extract-failed".into(), msg: err }),
    }
}

fn job_id(input: Value) -> Result<u64, Error> {
    let r: JobRef = serde_json::from_value(input).map_err(|e| Error::invalid(format!("JobRef required: {e}")))?;
    r.job.parse().map_err(|_| Error::invalid("job must be an Anything queue id"))
}

fn web_url(u: &str) -> bool {
    let u = u.trim();
    (u.starts_with("https://") || u.starts_with("http://")) && u.len() <= 4096 && !u.contains(char::is_whitespace)
}

impl Provider for Anything {
    fn id(&self) -> &str {
        ID
    }

    fn capabilities(&self) -> Vec<String> {
        vec![CAP_ACQUIRE_EXTRACT.into()]
    }

    fn health(&self) -> Result<(), Error> {
        self.server().map(|_| ())
    }

    fn invoke(&self, cap: &str, op: &str, input: Value) -> Result<Value, Error> {
        if cap != CAP_ACQUIRE_EXTRACT {
            return Err(Error::invalid(format!("unsupported cap {cap}")));
        }
        match op {
            "extract" => {
                let x: ExtractInput =
                    serde_json::from_value(input).map_err(|e| Error::invalid(format!("ExtractInput required: {e}")))?;
                if !web_url(&x.url) {
                    return Err(Error::new("unsupported-url", "url must be http(s)"));
                }
                let v = self.call("POST", "/extract", Some(&json!({"url": x.url.trim()})), EXTRACT_TIMEOUT)?;
                Ok(json!(extracted(&v)))
            }
            "enqueue" => {
                let x: ExtractEnqueueInput = serde_json::from_value(input)
                    .map_err(|e| Error::invalid(format!("ExtractEnqueueInput required: {e}")))?;
                if !web_url(&x.url) {
                    return Err(Error::new("unsupported-url", "url must be http(s)"));
                }
                if !x.output_dir.is_empty() && !std::path::Path::new(&x.output_dir).is_absolute() {
                    return Err(Error::invalid("output_dir must be absolute"));
                }
                let mut options = json!({});
                if !x.format.is_empty() {
                    options["format"] = json!(x.format);
                }
                if !x.output_dir.is_empty() {
                    options["output_dir"] = json!(x.output_dir);
                }
                let v =
                    self.call("POST", "/queue", Some(&json!({"url": x.url.trim(), "options": options})), TIMEOUT)?;
                Ok(json!(item(&v)))
            }
            "status" => {
                let id = job_id(input)?;
                Ok(json!(item(&self.call("GET", &format!("/queue/{id}"), None, TIMEOUT)?)))
            }
            "cancel" => {
                let id = job_id(input)?;
                let before = item(&self.call("GET", &format!("/queue/{id}"), None, TIMEOUT)?);
                if matches!(before.state.as_str(), "succeeded" | "failed" | "cancelled") {
                    return Err(Error::new("not-cancellable", format!("job {id} already {}", before.state)));
                }
                self.call("DELETE", &format!("/queue/{id}"), None, TIMEOUT)?;
                Ok(json!(ExtractJob { state: "cancelled".into(), ..before }))
            }
            other => Err(Error::invalid(format!("capability {cap} has no op {other:?}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping() {
        assert_eq!(state("completed"), "succeeded");
        assert_eq!(state("pending"), "queued");
        let j = item(&json!({"id": 7, "status": "failed", "error": "boom", "url": "https://example.com/v"}));
        assert_eq!((j.job.as_str(), j.state.as_str()), ("7", "failed"));
        assert_eq!(j.error.unwrap().msg, "boom");
        let o = extracted(&json!({"title": "T", "streams": [
            {"id": "a", "url": "https://cdn.example/signed", "is_audio_only": true},
            {"id": "v", "height": 1080, "quality": "1080p"}]}));
        assert_eq!(o.candidates[0].kind, "audio");
        assert_eq!(o.candidates[1].height, 1080);
        assert!(!serde_json::to_string(&o).unwrap().contains("signed"), "stream urls stay inside");
        assert!(!web_url("file:///etc/passwd"));
        assert!(web_url("https://example.com/watch?v=1"));
        assert!(job_id(json!({"job": "x"})).is_err());
    }
}
