//! Reads `{"id", "name"}` JSON lines on stdin and writes one line per
//! name with every parser's answer in one schema (title, episode_title,
//! year, season, episodes, release_group, resolution), plus latency.
//!
//! Parsers: lain's in-tree tokenizer, zantetsu's heuristic parser, the
//! lain-parser model's raw record (`--parser unix:///…` or `http://…`),
//! and `binding`: what lain's `lain.release.parse@1` answers with the
//! model component bound first (the model merged with the tokenizer's
//! tags, or the tokenizer when the model abstains or fails).
//!
//! Output goes to stdout and must stay on the machine: it holds the
//! names. Only `score.py`'s aggregates are committed.

use std::io::{BufRead, Write};
use std::time::Instant;

use serde_json::{Value, json};

use lain_bridge::http1::{self, Endpoint, Request};
use lain_bridge::parser::ReleaseModel;
use lain_core::Provider;
use lain_identify::{CAP_RELEASE_PARSE, tokenize};

fn opt_i(v: i64) -> Value {
    if v > 0 { json!(v) } else { Value::Null }
}

fn opt_s(v: &str) -> Value {
    if v.is_empty() { Value::Null } else { json!(v) }
}

fn from_release(r: &Value) -> Value {
    if r["title"].as_str().unwrap_or("").is_empty() {
        return Value::Null;
    }
    let season = r["season"].as_i64().unwrap_or(0);
    json!({
        "title": r["title"],
        "episode_title": opt_s(r["episode_title"].as_str().unwrap_or("")),
        "year": opt_i(r["year"].as_i64().unwrap_or(0)),
        // The Release contract cannot tell "season 0" from "no season".
        "season": opt_i(season),
        "episodes": r.get("episodes").cloned().filter(|e| e.as_array().is_some_and(|a| !a.is_empty())).unwrap_or(Value::Null),
        "release_group": opt_s(r["group"].as_str().unwrap_or("")),
        "resolution": opt_s(r["resolution"].as_str().unwrap_or("")),
    })
}

fn from_zantetsu(r: &zantetsu_core::ParseResult) -> Value {
    use zantetsu_core::EpisodeSpec;
    let Some(title) = r.title.as_ref().filter(|t| !t.is_empty()) else { return Value::Null };
    let episodes: Option<Vec<u32>> = r.episode.as_ref().map(|e| match e {
        EpisodeSpec::Single(n) => vec![*n],
        EpisodeSpec::Range(a, b) if b >= a && b - a < 2000 => (*a..=*b).collect(),
        EpisodeSpec::Range(a, _) => vec![*a],
        EpisodeSpec::Multi(v) => v.clone(),
        EpisodeSpec::Version { episode, .. } => vec![*episode],
    });
    json!({
        "title": title,
        "episode_title": null,
        "year": r.year,
        "season": r.season,
        "episodes": episodes,
        "release_group": r.group,
        "resolution": r.resolution.map(|x| x.to_string()),
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let parser_url = args.iter().position(|a| a == "--parser").and_then(|i| args.get(i + 1)).cloned();
    let endpoint = parser_url.as_deref().map(|u| Endpoint::parse(u).expect("--parser url"));
    let model = endpoint.clone().map(ReleaseModel::new);
    if let Some(m) = &model {
        m.health().expect("the lain-parser daemon must be healthy");
    }
    let zan = zantetsu_core::HeuristicParser::new().expect("zantetsu");
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        if line.trim().is_empty() {
            continue;
        }
        let row: Value = serde_json::from_str(&line).unwrap();
        let name = row["name"].as_str().unwrap().to_string();
        let mut rec = json!({"id": row["id"]});
        let mut lat = json!({});

        let t = Instant::now();
        let tok = serde_json::to_value(tokenize(&name, "")).unwrap();
        lat["tokenizer"] = json!(t.elapsed().as_secs_f64() * 1e3);
        rec["tokenizer"] = from_release(&tok);

        let t = Instant::now();
        let z = zan.parse(&name).ok();
        lat["zantetsu"] = json!(t.elapsed().as_secs_f64() * 1e3);
        rec["zantetsu"] = z.as_ref().map(from_zantetsu).unwrap_or(Value::Null);

        if let (Some(ep), Some(m)) = (&endpoint, &model) {
            let body = json!({"filename": name});
            let req = Request { json: Some(&body), max_body: 1 << 20, ..Request::new("POST", "/parse") };
            let t = Instant::now();
            let raw = http1::send(ep, &req).ok().and_then(|r| r.json());
            lat["model"] = json!(t.elapsed().as_secs_f64() * 1e3);
            rec["model"] = match raw.as_ref().map(|v| &v["record"]) {
                Some(Value::Object(o)) => {
                    let mut v = Value::Object(o.clone());
                    for k in ["title", "episode_title", "year", "season", "episodes", "release_group", "resolution"] {
                        if v.get(k).is_none() {
                            v[k] = Value::Null;
                        }
                    }
                    v
                }
                _ => Value::Null,
            };
            rec["model_status"] = json!(match &raw {
                None => "error",
                Some(v) if v["record"].is_object() => "ok",
                Some(_) => "abstain",
            });
            // The binding: the model provider, else the tokenizer.
            let bound = m.invoke(CAP_RELEASE_PARSE, "parse", json!({"name": name, "kind": ""})).ok();
            rec["binding"] = match bound.as_ref().map(from_release) {
                Some(v) if !v.is_null() => v,
                _ => rec["tokenizer"].clone(),
            };
        }
        rec["latency_ms"] = lat;
        writeln!(out, "{rec}").unwrap();
    }
}
