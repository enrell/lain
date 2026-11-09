//! Component protocol fixtures (`conformance/component/*.ndjson`).

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Who sent a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum From {
    Host,
    Component,
}

/// One recorded frame.
#[derive(Debug, Clone)]
pub struct Line {
    pub from: From,
    pub note: String,
    pub frame: Value,
}

/// The fixture directory of this checkout.
pub fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("component")
}

/// Every fixture file, sorted.
pub fn files() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir())
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "ndjson"))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// Parses one fixture file.
pub fn load(path: &Path) -> Result<Vec<Line>, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = Vec::new();
    for (i, l) in raw.lines().enumerate() {
        if l.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(l).map_err(|e| format!("{}:{}: {e}", path.display(), i + 1))?;
        let from = match v["from"].as_str() {
            Some("host") => From::Host,
            Some("component") => From::Component,
            other => return Err(format!("{}:{}: bad from {other:?}", path.display(), i + 1)),
        };
        out.push(Line { from, note: v["note"].as_str().unwrap_or("").to_string(), frame: v["frame"].clone() });
    }
    if out.first().is_none_or(|l| l.from != From::Component || l.frame["op"] != "hello") {
        return Err(format!("{}: the first frame must be the component's hello", path.display()));
    }
    Ok(out)
}
