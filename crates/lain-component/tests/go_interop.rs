//! A Go `lain plugin-run` component under the Rust host (D-171
//! "components first"): handshake, invoke, health, respawn after a
//! kill, and the plugins/ watcher installing, reloading and removing
//! it through the registry.
//!
//! The Go binary is `LAIN_GO_BIN`, or built from this checkout with
//! `go build` when Go is on PATH. Without either the tests skip, unless
//! `LAIN_REQUIRE_GO=1` (CI) makes that a failure.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use lain_component::{Manifest, Process, Provisioner, Timeouts};
use lain_core::{Binding, BindingMode, Composition, Provider, Registry, codes};

const ID: &str = "lain-identify-generic";
const CAP: &str = "lain.media.identify@1";

fn go_bin() -> Option<PathBuf> {
    static BIN: OnceLock<Option<PathBuf>> = OnceLock::new();
    BIN.get_or_init(|| {
        if let Ok(p) = std::env::var("LAIN_GO_BIN") {
            return Some(PathBuf::from(p));
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("lain-go");
        let ok = Command::new("go")
            .args(["build", "-o"])
            .arg(&out)
            .arg("./cmd/lain")
            .current_dir(&root)
            .env("CGO_ENABLED", "0")
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        ok.then_some(out)
    })
    .clone()
}

fn require_go() -> Option<PathBuf> {
    let bin = go_bin();
    if bin.is_none() {
        assert!(std::env::var("LAIN_REQUIRE_GO").is_err(), "LAIN_REQUIRE_GO is set but no Go lain binary is available");
        eprintln!("skipping: no Go toolchain and no LAIN_GO_BIN");
    }
    bin
}

fn manifest(bin: &Path, version: &str) -> Manifest {
    let mut m = Manifest { id: ID.into(), version: version.into(), capabilities: vec![CAP.into()], ..Manifest::default() };
    m.execution.kind = "process".into();
    m.execution.entrypoint = bin.to_string_lossy().into_owned();
    m.execution.args = ["plugin-run", "--id", ID, "--sock", "{sock}"].iter().map(|s| s.to_string()).collect();
    m
}

fn candidate() -> Value {
    json!({"path": "/media/movies/Some Movie (2020) [1080p].mkv", "size": 1024, "mod_time": 0, "library_id": "lib-1", "library_type": "movie"})
}

fn until(timeout: Duration, mut f: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    f()
}

#[test]
fn go_component_serves_under_the_rust_host() {
    let Some(bin) = require_go() else { return };
    let dir = lain_store::test_dir("go-interop");
    let t = Timeouts { backoff_start: Duration::from_millis(100), ..Timeouts::default() };
    let p = Process::spawn_with(manifest(&bin, "1"), &dir, t).unwrap();
    assert!(p.wait_ready(Duration::from_secs(20)), "Go component never handshook: {:?}", p.health());
    p.health().unwrap();
    let out = p.invoke(CAP, "identify", candidate()).unwrap();
    assert_eq!(out["title"], "Some Movie");
    assert_eq!(out["kind"], "video");
    assert_eq!(out["plugin_id"], ID);
    // Op omitted: single-input capability.
    assert_eq!(p.invoke(CAP, "", candidate()).unwrap()["title"], "Some Movie");
    // Typed errors cross the wire with their codes.
    assert_eq!(p.invoke(CAP, "bogus", json!({})).unwrap_err().code, codes::INVALID_MESSAGE);
    assert_eq!(p.invoke("lain.nope@1", "x", json!({})).unwrap_err().code, codes::INVALID_MESSAGE);
    assert_eq!(p.invoke(CAP, "identify", json!({"path": 42})).unwrap_err().code, codes::INVALID_MESSAGE);
    p.health().unwrap();

    // Kill the Go process: health degrades, the supervisor respawns it.
    let pid = p.pid().unwrap();
    assert!(Command::new("kill").arg("-9").arg(pid.to_string()).status().unwrap().success());
    assert!(until(Duration::from_secs(5), || p.health().is_err()), "health stayed ok after kill");
    assert!(until(Duration::from_secs(20), || p.health().is_ok()), "no respawn: {:?}", p.health());
    assert_ne!(p.pid(), Some(pid));
    assert_eq!(p.invoke(CAP, "identify", candidate()).unwrap()["title"], "Some Movie");
    p.close();
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn go_component_through_registry_and_plugins_dir() {
    let Some(bin) = require_go() else { return };
    let dir = lain_store::test_dir("go-provision");
    let plugins = dir.join("plugins");
    std::fs::create_dir_all(&plugins).unwrap();
    let mut bindings = BTreeMap::new();
    bindings.insert(CAP.to_string(), Binding::new(BindingMode::OrderedMany, &[ID]));
    let reg = Arc::new(Registry::new(Composition { version: 1, bindings }));
    let prov = Provisioner::with_timeouts(&plugins, reg.clone(), Timeouts { backoff_start: Duration::from_millis(100), ..Timeouts::default() });
    prov.start().unwrap();
    assert!(reg.call_first(CAP, "identify", candidate(), |_| true).is_err(), "nothing installed yet");

    // Drop a manifest: installed live, no restart.
    let path = plugins.join(format!("{ID}.json"));
    std::fs::write(&path, serde_json::to_vec(&manifest(&bin, "1")).unwrap()).unwrap();
    assert!(until(Duration::from_secs(20), || reg.ordered(CAP).is_ok()), "manifest never installed");
    let accepted = |v: &Value| v["title"].as_str().is_some_and(|t| !t.is_empty());
    let (out, served) = reg.call_first(CAP, "identify", candidate(), accepted).unwrap().unwrap();
    assert_eq!((served.as_str(), out["title"].as_str()), (ID, Some("Some Movie")));
    let info = reg.provider_infos();
    assert!(info.iter().any(|i| i.id == ID && i.healthy));

    // Rewrite: reloaded (a new process).
    let first = prov.process(ID).unwrap().pid();
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(&path, serde_json::to_vec(&manifest(&bin, "2")).unwrap()).unwrap();
    assert!(
        until(Duration::from_secs(20), || prov.process(ID).is_some_and(|p| p.manifest().version == "2" && p.ready())),
        "manifest rewrite never reloaded"
    );
    assert_ne!(prov.process(ID).unwrap().pid(), first);
    assert!(reg.call_first(CAP, "identify", candidate(), accepted).unwrap().is_some());

    // Remove: unregistered and killed.
    std::fs::remove_file(&path).unwrap();
    assert!(until(Duration::from_secs(20), || !reg.has_provider(ID)), "manifest removal never unregistered");
    assert!(prov.installed().is_empty());
    assert_eq!(reg.call_first(CAP, "identify", candidate(), accepted).unwrap_err().code, codes::DEPENDENCY_UNAVAILABLE);
    assert!(reg.events().iter().any(|e| e.kind == "unregister" && e.provider == ID));
    prov.close();
    std::fs::remove_dir_all(dir).ok();
}
