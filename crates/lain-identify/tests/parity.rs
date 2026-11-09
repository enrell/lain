//! The Go parsers' answers (`conformance/fixtures/parse-parity.json`,
//! written by `internal/plugins/identify/parity_test.go`) must be the
//! Rust parsers' answers, name for name.

use serde_json::{Value, json};

use lain_identify::{Candidate, Proposal, identify_anime, identify_comic, identify_generic, tokenize};

fn fixture() -> Value {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/fixtures/parse-parity.json");
    serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
}

fn accepted(p: Option<Proposal>) -> Value {
    match p {
        Some(p) if p.accepted() => json!(p),
        _ => Value::Null,
    }
}

#[test]
fn identify_matches_go() {
    let doc = fixture();
    let cases = doc["identify"].as_array().unwrap();
    assert!(cases.len() > 500);
    let mut diffs = Vec::new();
    for c in cases {
        let cand = Candidate {
            path: c["path"].as_str().unwrap().into(),
            library_id: "lib-1".into(),
            library_type: c["library_type"].as_str().unwrap_or("").into(),
            ..Candidate::default()
        };
        for (name, got) in [
            ("comic", accepted(identify_comic(&cand))),
            ("anime", accepted(identify_anime(&cand))),
            ("generic", accepted(Some(identify_generic(&cand)))),
        ] {
            if got != c[name] {
                diffs.push(format!(
                    "{name} {:?} [{}]\n  go:   {}\n  rust: {got}",
                    cand.path, cand.library_type, c[name]
                ));
            }
        }
    }
    assert!(diffs.is_empty(), "{} differences:\n{}", diffs.len(), diffs.join("\n"));
}

#[test]
fn release_matches_go() {
    let doc = fixture();
    let mut diffs = Vec::new();
    for c in doc["release"].as_array().unwrap() {
        let (name, kind) = (c["name"].as_str().unwrap(), c["kind"].as_str().unwrap_or(""));
        let got = json!(tokenize(name, kind));
        if got != c["release"] {
            diffs.push(format!("{name:?} [{kind}]\n  go:   {}\n  rust: {got}", c["release"]));
        }
    }
    assert!(diffs.is_empty(), "{} differences:\n{}", diffs.len(), diffs.join("\n"));
}
