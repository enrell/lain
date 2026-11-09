//! `lain-conformance`: runs the conformance case suites against a lain
//! server.
//!
//! ```text
//! LAIN_URL=http://127.0.0.1:9360 lain-conformance [--tier v1,v2] \
//!     [--var name=value ...] [--verbose] <suite dir>...
//! ```
//!
//! Exit status 0 when every non-skipped case passed.
//!
//! ```text
//! lain-conformance upstream [--dir <fixtures>] [--port <port>]
//! ```
//!
//! serves the recorded upstream answers (`conformance/fixtures/upstream`)
//! until killed, printing its URL first.

use std::path::PathBuf;

use lain_conformance::cases::{Outcome, Runner};
use lain_conformance::http::Base;
use serde_json::Value;

fn upstream(mut args: impl Iterator<Item = String>) -> ! {
    let mut dir = lain_conformance::upstream::dir();
    let mut port = 0;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--dir" => dir = PathBuf::from(args.next().unwrap_or_default()),
            "--port" => port = args.next().and_then(|p| p.parse().ok()).unwrap_or(0),
            _ => {
                eprintln!("usage: lain-conformance upstream [--dir <fixtures>] [--port <port>]");
                std::process::exit(2);
            }
        }
    }
    match lain_conformance::upstream::Upstream::start(&dir, port) {
        Ok(u) => {
            println!("{}", u.url);
            use std::io::Write;
            let _ = std::io::stdout().flush();
            u.wait()
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1).peekable();
    if args.peek().map(String::as_str) == Some("upstream") {
        args.next();
        upstream(args);
    }
    let mut tiers = vec!["v1".to_string(), "v2".to_string()];
    let mut suites = Vec::new();
    let mut vars = Vec::new();
    let mut verbose = false;
    let mut url = std::env::var("LAIN_URL").ok();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--tier" => tiers = args.next().unwrap_or_default().split(',').map(str::to_string).collect(),
            "--var" => vars.push(args.next().unwrap_or_default()),
            "--url" => url = args.next(),
            "--verbose" | "-v" => verbose = true,
            "-h" | "--help" => {
                eprintln!(
                    "usage: LAIN_URL=http://host:port lain-conformance [--tier v1,v2] [--var k=v] [-v] <suite dir>..."
                );
                return;
            }
            _ => suites.push(PathBuf::from(a)),
        }
    }
    let Some(url) = url else {
        eprintln!("LAIN_URL (or --url) is required");
        std::process::exit(2);
    };
    let base = match Base::parse(&url) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    if suites.is_empty() {
        eprintln!("no suite directories given");
        std::process::exit(2);
    }
    let mut runner = Runner::new(base);
    runner.verbose = verbose;
    for v in vars {
        let (k, val) = v.split_once('=').unwrap_or((v.as_str(), ""));
        runner.vars.insert(k.to_string(), Value::String(val.to_string()));
    }
    let (mut pass, mut fail, mut skip) = (0, 0, 0);
    for s in &suites {
        match runner.run_suite(s, &tiers) {
            Ok(reports) => {
                for r in reports {
                    match r.outcome {
                        Outcome::Pass => pass += 1,
                        Outcome::Fail(_) => fail += 1,
                        Outcome::Skip(_) => skip += 1,
                    }
                }
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(2);
            }
        }
    }
    let total = pass + fail + skip;
    println!(
        "{url}: {pass}/{} passed, {fail} failed, {skip} skipped ({total} cases; tiers {})",
        pass + fail,
        tiers.join(",")
    );
    if fail > 0 {
        std::process::exit(1);
    }
}
