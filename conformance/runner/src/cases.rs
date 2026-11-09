//! The case-file interpreter.
//!
//! A suite is a directory of `*.json` files run in name order with one
//! shared variable map. A file is `{"tier", "description", "cases"}`;
//! a case is `{"name", "requires"?, "steps"}`. Steps:
//!
//! - `{"req": {method, path, auth?, body?, raw_body?, raw_body_hex?, headers?},
//!    "expect": {status?, status_not?, body?, body_hex?, headers?}, "save": {var: "/pointer"}}`
//!   `raw_body_hex` sends bytes (images); `body_hex` compares the
//!   response bytes exactly.
//!   `auth` is `"<token var>"` (Bearer), `"query:<token var>"`
//!   (`?token=`) or `"header:<raw Authorization value>"`.
//! - `{"wait_for": <req step>, "timeout_ms"}`: repeats the request until
//!   the expectation holds.
//! - `{"sse_open": {path, auth, as}, "expect": {status?, headers?}}`
//! - `{"sse_expect": {stream, event, data?, within_ms?, strict?}}`: the
//!   next matching event on that stream (others are skipped; with
//!   `strict`, the next event of that name must match).
//! - `{"write_file": {path, json? | text?}}` (parent directories are
//!   created), `{"remove_file": path}`,
//!   `{"sleep_ms": n}`.
//!
//! `{{var}}` in strings is replaced from the variable map; a string
//! that is exactly `{{var}}` takes the variable's JSON value. A case
//! whose `requires` names an unset variable is skipped.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::http::{self, Base, SseStream};
use crate::matcher;

/// Outcome of one case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Pass,
    Fail(String),
    Skip(String),
}

/// One reported case.
#[derive(Debug, Clone)]
pub struct Report {
    pub file: String,
    pub name: String,
    pub outcome: Outcome,
}

/// A suite run's state.
pub struct Runner {
    pub base: Base,
    pub vars: BTreeMap<String, Value>,
    streams: BTreeMap<String, SseStream>,
    pub verbose: bool,
}

/// Replaces `{{var}}` in every string of `v`.
pub fn subst(v: &Value, vars: &BTreeMap<String, Value>) -> Result<Value, String> {
    Ok(match v {
        Value::String(s) => {
            if let Some(name) = s.strip_prefix("{{").and_then(|r| r.strip_suffix("}}"))
                && !name.contains("{{")
            {
                return vars.get(name).cloned().ok_or_else(|| format!("unset variable {name}"));
            }
            Value::String(subst_str(s, vars)?)
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| subst(x, vars)).collect::<Result<_, _>>()?),
        Value::Object(o) => {
            let mut out = serde_json::Map::new();
            for (k, x) in o {
                out.insert(k.clone(), subst(x, vars)?);
            }
            Value::Object(out)
        }
        other => other.clone(),
    })
}

fn subst_str(s: &str, vars: &BTreeMap<String, Value>) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after.find("}}").ok_or_else(|| format!("unclosed {{{{ in {s:?}"))?;
        let name = &after[..end];
        match vars.get(name) {
            Some(Value::String(v)) => out.push_str(v),
            Some(v) => out.push_str(&v.to_string()),
            None => return Err(format!("unset variable {name}")),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn from_hex(s: &str) -> Result<Vec<u8>, String> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if !s.len().is_multiple_of(2) {
        return Err("raw_body_hex has an odd length".into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| format!("raw_body_hex: {e}")))
        .collect()
}

impl Runner {
    pub fn new(base: Base) -> Self {
        Runner { base, vars: BTreeMap::new(), streams: BTreeMap::new(), verbose: false }
    }

    fn auth(&self, auth: Option<&str>, path: &str, headers: &mut Vec<(String, String)>) -> Result<String, String> {
        let Some(auth) = auth else { return Ok(path.to_string()) };
        if let Some(raw) = auth.strip_prefix("header:") {
            headers.push(("Authorization".into(), raw.to_string()));
            return Ok(path.to_string());
        }
        let (query, var) = match auth.strip_prefix("query:") {
            Some(v) => (true, v),
            None => (false, auth),
        };
        let tok =
            self.vars.get(var).and_then(Value::as_str).ok_or_else(|| format!("auth: unset token variable {var}"))?;
        if query {
            let sep = if path.contains('?') { '&' } else { '?' };
            Ok(format!("{path}{sep}token={}", percent_encode(tok)))
        } else {
            headers.push(("Authorization".into(), format!("Bearer {tok}")));
            Ok(path.to_string())
        }
    }

    fn send(&self, req: &Value) -> Result<http::Response, String> {
        let req = subst(req, &self.vars)?;
        let method = req["method"].as_str().unwrap_or("GET").to_string();
        let path = req["path"].as_str().ok_or("req.path required")?;
        let mut headers = Vec::new();
        if let Some(h) = req["headers"].as_object() {
            for (k, v) in h {
                headers.push((k.clone(), v.as_str().unwrap_or_default().to_string()));
            }
        }
        let path = self.auth(req["auth"].as_str(), path, &mut headers)?;
        let body: Option<Vec<u8>> = if let Some(hex) = req["raw_body_hex"].as_str() {
            Some(from_hex(hex)?)
        } else if let Some(raw) = req["raw_body"].as_str() {
            Some(raw.as_bytes().to_vec())
        } else if !req["body"].is_null() || req.get("body").is_some() {
            headers.push(("Content-Type".into(), "application/json".into()));
            Some(serde_json::to_vec(&req["body"]).map_err(|e| e.to_string())?)
        } else {
            None
        };
        http::request(&self.base, &method, &path, &headers, body.as_deref())
    }

    fn check(&self, resp: &http::Response, expect: &Value) -> Result<Value, String> {
        let expect = subst(expect, &self.vars)?;
        let body: Value = if resp.body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&resp.body)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&resp.body).into_owned()))
        };
        if let Some(st) = expect["status"].as_u64()
            && resp.status as u64 != st
        {
            return Err(format!(
                "status: want {st}, got {} (body {})",
                resp.status,
                String::from_utf8_lossy(&resp.body)
            ));
        }
        if let Some(st) = expect["status_not"].as_u64()
            && resp.status as u64 == st
        {
            return Err(format!("status: want anything but {st} (body {})", String::from_utf8_lossy(&resp.body)));
        }
        if let Some(h) = expect["headers"].as_object() {
            for (k, want) in h {
                let got = resp.header(k).map(|s| Value::String(s.to_string())).unwrap_or(Value::Null);
                if want.as_str() == Some("$absent") {
                    if !got.is_null() {
                        return Err(format!("header {k}: want absent, got {got}"));
                    }
                    continue;
                }
                matcher::matches(want, &got).map_err(|e| format!("header {k}: {e}"))?;
            }
        }
        if let Some(want) = expect["body_hex"].as_str() {
            let got: String = resp.body.iter().map(|b| format!("{b:02x}")).collect();
            if got != want.to_ascii_lowercase() {
                return Err(format!(
                    "body bytes: want {} bytes, got {} ({}...)",
                    want.len() / 2,
                    resp.body.len(),
                    &got[..got.len().min(32)]
                ));
            }
        }
        if let Some(want) = expect.get("body") {
            matcher::matches(want, &body).map_err(|e| format!("body {e}"))?;
        }
        Ok(body)
    }

    fn save(&mut self, body: &Value, save: &Value) -> Result<(), String> {
        if let Some(m) = save.as_object() {
            for (var, ptr) in m {
                let ptr = ptr.as_str().ok_or("save pointer must be a string")?;
                let v = body.pointer(ptr).ok_or_else(|| format!("save {var}: {ptr} not in response"))?;
                self.vars.insert(var.clone(), v.clone());
            }
        }
        Ok(())
    }

    fn step(&mut self, step: &Value) -> Result<(), String> {
        if let Some(req) = step.get("req") {
            let resp = self.send(req)?;
            let body = self.check(&resp, step.get("expect").unwrap_or(&Value::Null))?;
            return self.save(&body, step.get("save").unwrap_or(&Value::Null));
        }
        if let Some(inner) = step.get("wait_for") {
            let timeout = Duration::from_millis(step["timeout_ms"].as_u64().unwrap_or(10_000));
            let deadline = Instant::now() + timeout;
            loop {
                let resp = self.send(&inner["req"])?;
                match self.check(&resp, inner.get("expect").unwrap_or(&Value::Null)) {
                    Ok(body) => return self.save(&body, inner.get("save").unwrap_or(&Value::Null)),
                    Err(e) if Instant::now() >= deadline => return Err(format!("wait_for timed out: {e}")),
                    Err(_) => std::thread::sleep(Duration::from_millis(100)),
                }
            }
        }
        if let Some(open) = step.get("sse_open") {
            let open = subst(open, &self.vars)?;
            let name = open["as"].as_str().ok_or("sse_open.as required")?.to_string();
            let mut headers = Vec::new();
            let path =
                self.auth(open["auth"].as_str(), open["path"].as_str().ok_or("sse_open.path required")?, &mut headers)?;
            let stream = http::open_sse(&self.base, &path, &headers)?;
            if let Some(expect) = step.get("expect") {
                let fake = http::Response { status: stream.status, headers: stream.headers.clone(), body: Vec::new() };
                let mut e = expect.clone();
                if let Some(o) = e.as_object_mut() {
                    o.remove("body");
                }
                self.check(&fake, &e)?;
            }
            self.streams.insert(name, stream);
            return Ok(());
        }
        if let Some(exp) = step.get("sse_expect") {
            let exp = subst(exp, &self.vars)?;
            let name = exp["stream"].as_str().ok_or("sse_expect.stream required")?;
            let stream = self.streams.get(name).ok_or_else(|| format!("no open stream {name}"))?;
            let event = exp["event"].as_str().unwrap_or("message");
            let strict = exp["strict"].as_bool().unwrap_or(false);
            let deadline = Instant::now() + Duration::from_millis(exp["within_ms"].as_u64().unwrap_or(5_000));
            let mut seen = Vec::new();
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                match stream.events.recv_timeout(left) {
                    Ok(ev) => {
                        if ev.event != event {
                            seen.push(ev.event);
                            continue;
                        }
                        let data: Value = serde_json::from_str(&ev.data).unwrap_or(Value::String(ev.data.clone()));
                        match exp.get("data").map(|w| matcher::matches(w, &data)) {
                            None | Some(Ok(())) => {
                                let save = step.get("save").cloned().unwrap_or(Value::Null);
                                return self.save(&data, &save);
                            }
                            Some(Err(e)) if strict => {
                                return Err(format!("first {event} event does not match: {e} (data {})", ev.data));
                            }
                            Some(Err(_)) => seen.push(format!("{} {}", ev.event, ev.data)),
                        }
                    }
                    Err(_) => return Err(format!("no {event} event within the deadline (saw {seen:?})")),
                }
            }
        }
        if let Some(w) = step.get("write_file") {
            let w = subst(w, &self.vars)?;
            let path = w["path"].as_str().ok_or("write_file.path required")?;
            let content = match (w.get("json"), w["text"].as_str()) {
                (Some(j), _) => serde_json::to_vec_pretty(j).map_err(|e| e.to_string())?,
                (None, Some(t)) => t.as_bytes().to_vec(),
                _ => return Err("write_file needs json or text".into()),
            };
            if let Some(parent) = std::path::Path::new(path).parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
            }
            let tmp = format!("{path}.tmp-conformance");
            std::fs::write(&tmp, content).map_err(|e| format!("write {path}: {e}"))?;
            return std::fs::rename(&tmp, path).map_err(|e| format!("rename {path}: {e}"));
        }
        if let Some(p) = step.get("remove_file") {
            let p = subst(p, &self.vars)?;
            let p = p.as_str().ok_or("remove_file must be a path")?;
            return match std::fs::remove_file(p) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(format!("remove {p}: {e}")),
            };
        }
        if let Some(ms) = step.get("sleep_ms").and_then(Value::as_u64) {
            std::thread::sleep(Duration::from_millis(ms));
            return Ok(());
        }
        Err(format!("unknown step {step}"))
    }

    fn run_case(&mut self, case: &Value) -> Outcome {
        if let Some(req) = case["requires"].as_array() {
            for r in req {
                let r = r.as_str().unwrap_or_default();
                if !self.vars.contains_key(r) {
                    return Outcome::Skip(format!("needs {r}"));
                }
            }
        }
        let Some(steps) = case["steps"].as_array() else { return Outcome::Fail("case has no steps".into()) };
        for (i, step) in steps.iter().enumerate() {
            if let Err(e) = self.step(step) {
                return Outcome::Fail(format!("step {}: {e}", i + 1));
            }
        }
        Outcome::Pass
    }

    /// Runs one suite directory. Files whose `tier` is not in `tiers`
    /// are skipped entirely (not counted).
    pub fn run_suite(&mut self, dir: &Path, tiers: &[String]) -> Result<Vec<Report>, String> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        files.sort();
        let mut out = Vec::new();
        for f in files {
            let raw = std::fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))?;
            let doc: Value = serde_json::from_slice(&raw).map_err(|e| format!("{}: {e}", f.display()))?;
            let tier = doc["tier"].as_str().unwrap_or("v1");
            if !tiers.iter().any(|t| t == tier) {
                continue;
            }
            let file = format!(
                "{}/{}",
                dir.file_name().unwrap_or_default().to_string_lossy(),
                f.file_name().unwrap_or_default().to_string_lossy()
            );
            for case in doc["cases"].as_array().cloned().unwrap_or_default() {
                let name = case["name"].as_str().unwrap_or("?").to_string();
                let outcome = self.run_case(&case);
                if self.verbose || !matches!(outcome, Outcome::Pass) {
                    match &outcome {
                        Outcome::Pass => println!("PASS {file} :: {name}"),
                        Outcome::Fail(e) => println!("FAIL {file} :: {name}\n     {e}"),
                        Outcome::Skip(e) => println!("SKIP {file} :: {name} ({e})"),
                    }
                }
                out.push(Report { file: file.clone(), name, outcome });
            }
        }
        self.streams.clear();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn substitution() {
        let mut vars = BTreeMap::new();
        vars.insert("id".to_string(), json!("lib-1"));
        vars.insert("n".to_string(), json!(3));
        assert_eq!(
            subst(&json!({"p": "/api/{{id}}/x", "n": "{{n}}", "s": "n={{n}}"}), &vars).unwrap(),
            json!({"p": "/api/lib-1/x", "n": 3, "s": "n=3"})
        );
        assert!(subst(&json!("{{missing}}"), &vars).is_err());
        assert_eq!(percent_encode("a.b-c_d~"), "a.b-c_d~");
        assert_eq!(percent_encode("a b/"), "a%20b%2F");
        assert_eq!(from_hex("00ff 10").unwrap(), vec![0, 255, 16]);
        assert!(from_hex("abc").is_err() && from_hex("zz").is_err());
    }
}
