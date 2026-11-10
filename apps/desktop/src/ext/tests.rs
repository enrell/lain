//! Extension host tests: manifests and installs, scope enforcement, the
//! resource limits and crash isolation, against real components.
//!
//! The guests are `crates/lain-ext-api/probe` (a test guest whose config
//! picks a misbehavior) and `examples/extensions/seasonal-stats`, built to
//! `wasm32-wasip2`. They are found in `LAIN_EXT_GUESTS` (a directory),
//! else in the usual target directories. Without them the wasm tests
//! skip, unless `LAIN_REQUIRE_EXT_GUESTS=1` (CI sets it), as the
//! server's ffmpeg tests do with `LAIN_REQUIRE_FFMPEG`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::manifest::{self, Manifest, Scope};
use super::runtime::{self, Cmd, Grants, HostApi, HostError, HttpResponse, Item, Library, Limits, Out, Progress};
use super::{ExtSettings, Manager, Notice, Status};

// ---- fixtures ---------------------------------------------------------------

fn guest(name: &str) -> Option<Vec<u8>> {
    let file = format!("{name}.wasm");
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(d) = std::env::var("LAIN_EXT_GUESTS") {
        dirs.push(PathBuf::from(d));
    }
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if let Ok(t) = std::env::var("CARGO_TARGET_DIR") {
        dirs.push(Path::new(&t).join("wasm32-wasip2/release"));
    }
    dirs.push(repo.join("crates/lain-ext-api/target/wasm32-wasip2/release"));
    dirs.push(repo.join("examples/extensions/seasonal-stats/target/wasm32-wasip2/release"));
    for d in dirs {
        if let Ok(b) = std::fs::read(d.join(&file)) {
            return Some(b);
        }
    }
    if std::env::var("LAIN_REQUIRE_EXT_GUESTS").is_ok_and(|v| v == "1") {
        panic!("{file} not found: build the guests for wasm32-wasip2 (see docs/MODDING.md) or set LAIN_EXT_GUESTS");
    }
    eprintln!("skipping: {file} not built (set LAIN_EXT_GUESTS)");
    None
}

#[derive(Default)]
struct FakeApi {
    calls: Mutex<Vec<String>>,
    now: i64,
}

impl FakeApi {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    fn note(&self, c: impl Into<String>) {
        self.calls.lock().unwrap().push(c.into());
    }
    fn item(&self, n: i64, title: &str) -> Item {
        Item {
            id: format!("item-{n}"),
            library_id: "lib-1".into(),
            kind: "anime".into(),
            title: title.into(),
            season: 1,
            episode: n,
            year: 2026,
            size: 1,
            updated_at: self.now,
        }
    }
}

impl HostApi for FakeApi {
    fn libraries(&self) -> Result<Vec<Library>, HostError> {
        self.note("libraries");
        Ok(vec![Library { id: "lib-1".into(), name: "Anime".into(), kind: "anime".into() }])
    }
    fn catalog(&self, offset: u32, limit: u32) -> Result<Vec<Item>, HostError> {
        self.note(format!("catalog {offset} {limit}"));
        if offset > 0 {
            return Ok(Vec::new());
        }
        Ok(vec![self.item(1, "Frieren"), self.item(2, "Frieren"), self.item(3, "Dandadan")])
    }
    fn item(&self, id: &str) -> Result<Item, HostError> {
        self.note(format!("item {id}"));
        Ok(self.item(1, "Frieren"))
    }
    fn progress(&self, item_id: &str) -> Result<Option<Progress>, HostError> {
        self.note(format!("progress {item_id}"));
        Ok(Some(Progress {
            item_id: item_id.into(),
            position_sec: 600.,
            duration_sec: 1440.,
            completed: item_id == "item-1",
            updated_at: self.now,
        }))
    }
    fn continue_watching(&self) -> Result<Vec<Progress>, HostError> {
        self.note("continue");
        Ok(Vec::new())
    }
    fn http_get(&self, url: &str, _max: usize) -> Result<HttpResponse, HostError> {
        self.note(format!("http {url}"));
        Ok(HttpResponse { status: 200, headers: Vec::new(), body: b"ok".to_vec() })
    }
}

fn scopes(list: &[&str]) -> Grants {
    Grants::new(list.iter().map(|s| Scope::parse(s).unwrap()).collect())
}

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lain-ext-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

struct Run {
    rx: tokio::sync::mpsc::UnboundedReceiver<Out>,
    worker: runtime::Worker,
    _rt: runtime::Runtime,
}

impl Run {
    /// The next output that is not a log line.
    fn next(&mut self, secs: u64) -> Option<Out> {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            match self.rx.try_recv() {
                Ok(Out::Log { .. }) => {}
                Ok(o) => return Some(o),
                Err(_) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        None
    }
    fn ready(&mut self) {
        match self.next(30) {
            Some(Out::Ready { .. }) => {}
            other => panic!("expected Ready, got {other:?}"),
        }
    }
    fn rails(&mut self) -> Vec<runtime::Rail> {
        match self.next(30) {
            Some(Out::Rails { rails, .. }) => rails,
            other => panic!("expected Rails, got {other:?}"),
        }
    }
    fn failed(&mut self) -> String {
        loop {
            match self.next(60) {
                Some(Out::Failed { reason, .. }) => return reason,
                Some(Out::Ready { .. } | Out::Rails { .. } | Out::Panels { .. }) => continue,
                other => panic!("expected Failed, got {other:?}"),
            }
        }
    }
}

fn run(wasm: Vec<u8>, mode: &str, grants: Grants, limits: Limits, api: Arc<FakeApi>, data: &Path) -> Run {
    let rt = runtime::Runtime::new().unwrap();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let worker = rt.start(
        runtime::Spec {
            id: "probe".into(),
            wasm,
            config: format!(r#"{{"mode":"{mode}"}}"#),
            grants,
            limits,
            data_dir: data.to_path_buf(),
            api,
        },
        tx,
    );
    Run { rx, worker, _rt: rt }
}

fn card<'a>(rails: &'a [runtime::Rail], title: &str) -> &'a str {
    rails[0].cards.iter().find(|c| c.title == title).map(|c| c.subtitle.as_str()).unwrap_or("<missing>")
}

// ---- manifests and installs ---------------------------------------------------------

const MANIFEST: &str = r#"{"format":"lain.extension@1","id":"probe","name":"Probe","version":"0.1.0",
  "api":"0.1.0","wasm":"probe.wasm","scopes":["catalog:read","net:example.org","ui:rail"]}"#;

#[test]
fn scopes_parse_strictly() {
    assert_eq!(Scope::parse("catalog:read"), Ok(Scope::CatalogRead));
    assert_eq!(Scope::parse("net:Example.org"), Ok(Scope::Net("example.org".into())));
    assert_eq!(Scope::parse("net:localhost:8080").map(|s| s.id()), Ok("net:localhost:8080".into()));
    for bad in ["net:*", "net:*.example.org", "net:", "net:a/b", "net:user@host", "net:h:0", "fs:all", "admin"] {
        assert!(Scope::parse(bad).is_err(), "{bad} should not parse");
    }
}

#[test]
fn manifests_are_validated() {
    let m = Manifest::parse(MANIFEST.as_bytes()).unwrap();
    assert_eq!(m.requested().unwrap().len(), 3);
    assert_eq!(m.config_json(), "{}");
    let with = |k: &str, v: &str| MANIFEST.replace(&format!("\"{k}\":\"{}\"", field(k)), &format!("\"{k}\":{v}"));
    fn field(k: &str) -> &'static str {
        match k {
            "id" => "probe",
            "api" => "0.1.0",
            "wasm" => "probe.wasm",
            _ => "lain.extension@1",
        }
    }
    for (k, v) in [
        ("id", "\"Bad Id\""),
        ("api", "\"1.0.0\""),
        ("api", "\"0.9\""),
        ("wasm", "\"../x.wasm\""),
        ("wasm", "\"/abs.wasm\""),
        ("wasm", "\"x.exe\""),
        ("format", "\"lain.extension@2\""),
    ] {
        assert!(Manifest::parse(with(k, v).as_bytes()).is_err(), "{k}={v} should be refused");
    }
    let unknown = MANIFEST.replace("ui:rail", "ui:everything");
    assert!(Manifest::parse(unknown.as_bytes()).unwrap_err().contains("unknown scope"));
}

#[test]
fn installs_from_a_folder_and_a_zip_and_refuses_escapes() {
    let d = temp("install");
    let root = d.join("extensions");
    let src = d.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("extension.json"), MANIFEST).unwrap();
    std::fs::write(src.join("probe.wasm"), b"\0asm").unwrap();
    std::fs::write(src.join("secret.txt"), b"not copied").unwrap();
    let m = manifest::install(&src, &root).unwrap();
    assert_eq!(m.id, "probe");
    assert!(root.join("probe/probe.wasm").is_file());
    assert!(!root.join("probe/secret.txt").exists());
    // Reinstalling keeps the extension's data.
    std::fs::create_dir_all(root.join("probe/data")).unwrap();
    std::fs::write(root.join("probe/data/state"), b"1").unwrap();
    let zip = d.join("probe.zip");
    std::fs::write(
        &zip,
        crate::zip::tests::stored_many(&[
            ("probe/extension.json", MANIFEST.as_bytes()),
            ("probe/probe.wasm", b"\0asm"),
        ]),
    )
    .unwrap();
    manifest::install(&zip, &root).unwrap();
    assert!(root.join("probe/data/state").is_file());
    let list = manifest::list(&root);
    assert_eq!(list.len(), 1);
    assert!(list[0].manifest.is_ok());
    // A manifest that points outside its folder never installs.
    let evil = MANIFEST.replace("probe.wasm", "../../evil.wasm");
    std::fs::write(&zip, crate::zip::tests::stored_many(&[("extension.json", evil.as_bytes())])).unwrap();
    assert!(manifest::install(&zip, &root).is_err());
    manifest::uninstall(&root, "probe").unwrap();
    assert!(manifest::list(&root).is_empty());
    assert!(manifest::uninstall(&root, "../x").is_err());
    let _ = std::fs::remove_dir_all(&d);
}

// ---- scopes --------------------------------------------------------------------------

#[test]
fn host_calls_answer_only_granted_scopes() {
    let Some(wasm) = guest("lain_ext_probe") else { return };
    let d = temp("scopes");
    let api = Arc::new(FakeApi::default());
    let mut r = run(
        wasm,
        "calls",
        scopes(&["catalog:read", "ui:rail", "net:allowed.example"]),
        Limits::default(),
        api.clone(),
        &d,
    );
    r.ready();
    let rails = r.rails();
    assert_eq!(card(&rails, "granted"), "catalog:read,ui:rail,net:allowed.example");
    assert_eq!(card(&rails, "list-libraries"), "ok 1");
    assert_eq!(card(&rails, "list-catalog"), "ok 3");
    assert_eq!(card(&rails, "get-item"), "ok Frieren");
    assert_eq!(card(&rails, "get-progress"), "denied progress:read");
    assert_eq!(card(&rails, "continue-watching"), "denied progress:read");
    assert_eq!(card(&rails, "http-allowed"), "ok 200");
    assert_eq!(card(&rails, "http-other"), "denied net:other.example");
    // No fs:data: no preopened folder, so no filesystem at all.
    assert!(card(&rails, "fs").starts_with("error"), "{}", card(&rails, "fs"));
    // No environment leaks in.
    assert_eq!(card(&rails, "env"), "ok 0");
    // Denied calls never reached the API.
    let calls = api.calls();
    assert!(calls.iter().all(|c| !c.starts_with("progress") && c != "continue"), "{calls:?}");
    assert!(calls.contains(&"http https://allowed.example/x".to_string()));
    assert!(!calls.iter().any(|c| c.contains("other.example")));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn fs_data_is_a_private_folder() {
    let Some(wasm) = guest("lain_ext_probe") else { return };
    let d = temp("fsdata");
    let data = d.join("data");
    let mut r =
        run(wasm, "calls", scopes(&["ui:rail", "fs:data"]), Limits::default(), Arc::new(FakeApi::default()), &data);
    r.ready();
    let rails = r.rails();
    assert_eq!(card(&rails, "fs"), "ok 2");
    assert_eq!(std::fs::read(data.join("probe.txt")).unwrap(), b"hi");
    assert_eq!(card(&rails, "list-catalog"), "denied catalog:read");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn ui_and_event_exports_need_their_scopes() {
    let Some(wasm) = guest("lain_ext_probe") else { return };
    let d = temp("ui");
    // Without ui:rail and events:subscribe the host never calls them.
    let mut r = run(wasm.clone(), "ok", scopes(&["ui:panel"]), Limits::default(), Arc::new(FakeApi::default()), &d);
    r.ready();
    r.worker.send(Cmd::Rails);
    r.worker.send(Cmd::Event(runtime::Event::LibraryChanged));
    r.worker.send(Cmd::Panels("item-9".into()));
    match r.next(30) {
        Some(Out::Panels { item, panels, .. }) => {
            assert_eq!(item, "item-9");
            assert_eq!(panels[0].rows[0].value, "item-9");
        }
        other => panic!("expected only Panels, got {other:?}"),
    }
    assert!(r.next(1).is_none());
    // With them: events arrive, and invalidate brings fresh rails.
    let mut r =
        run(wasm, "ok", scopes(&["ui:rail", "events:subscribe"]), Limits::default(), Arc::new(FakeApi::default()), &d);
    r.ready();
    assert!(r.rails()[1].cards.is_empty());
    r.worker.send(Cmd::Event(runtime::Event::ItemOpened("item-1".into())));
    let rails = r.rails();
    assert_eq!(rails[1].cards[0].title, "item-opened item-1");
    let _ = std::fs::remove_dir_all(&d);
}

// ---- limits and crash isolation ----------------------------------------------------

#[test]
fn fuel_stops_a_busy_loop() {
    let Some(wasm) = guest("lain_ext_probe") else { return };
    let d = temp("fuel");
    let limits = Limits { fuel: 20_000_000, deadline: Duration::from_secs(120), ..Limits::default() };
    let mut r = run(wasm, "loop", scopes(&["ui:rail"]), limits, Arc::new(FakeApi::default()), &d);
    let reason = r.failed();
    assert!(reason.contains("CPU budget"), "{reason}");
}

#[test]
fn the_deadline_stops_a_busy_loop() {
    let Some(wasm) = guest("lain_ext_probe") else { return };
    let d = temp("epoch");
    let limits = Limits { fuel: u64::MAX / 4, deadline: Duration::from_millis(300), ..Limits::default() };
    let start = Instant::now();
    let mut r = run(wasm, "loop", scopes(&["ui:rail"]), limits, Arc::new(FakeApi::default()), &d);
    let reason = r.failed();
    assert!(reason.contains("longer than"), "{reason}");
    assert!(start.elapsed() < Duration::from_secs(20));
}

#[test]
fn the_memory_cap_stops_a_hoarder() {
    let Some(wasm) = guest("lain_ext_probe") else { return };
    let d = temp("oom");
    let limits = Limits { memory: 16 << 20, ..Limits::default() };
    let mut r = run(wasm, "oom", scopes(&["ui:rail"]), limits, Arc::new(FakeApi::default()), &d);
    let reason = r.failed();
    assert!(reason.contains("memory") || reason.contains("crashed"), "{reason}");
}

#[test]
fn a_panic_is_a_trap_that_names_the_panic() {
    let Some(wasm) = guest("lain_ext_probe") else { return };
    let d = temp("panic");
    let mut r = run(wasm, "panic", scopes(&["ui:rail"]), Limits::default(), Arc::new(FakeApi::default()), &d);
    let reason = r.failed();
    assert!(reason.contains("crashed"), "{reason}");
    assert!(reason.contains("panics on purpose"), "{reason}");
    // The worker is gone: commands go nowhere and nothing else arrives.
    r.worker.send(Cmd::Rails);
    assert!(r.next(1).is_none());
}

#[test]
fn bad_components_and_refusals_fail_cleanly() {
    let d = temp("bad");
    let mut r =
        run(b"\0asm not really".to_vec(), "ok", scopes(&[]), Limits::default(), Arc::new(FakeApi::default()), &d);
    assert!(r.failed().contains("not a valid component"));
    let Some(wasm) = guest("lain_ext_probe") else { return };
    let mut r = run(wasm, "refuse", scopes(&["ui:rail"]), Limits::default(), Arc::new(FakeApi::default()), &d);
    let reason = r.failed();
    assert!(reason.contains("activate refused") && reason.contains("refuses this config"), "{reason}");
}

#[test]
fn outputs_are_clipped_to_the_host_caps() {
    let Some(wasm) = guest("lain_ext_probe") else { return };
    let d = temp("big");
    let mut r = run(wasm, "big", scopes(&["ui:rail"]), Limits::default(), Arc::new(FakeApi::default()), &d);
    r.ready();
    let rails = r.rails();
    assert_eq!(rails.len(), runtime::caps::RAILS);
    assert!(rails.iter().all(|r| r.cards.len() == runtime::caps::CARDS));
    assert!(rails.iter().all(|r| r.title.chars().count() == runtime::caps::TITLE));
    assert!(rails[0].cards[0].subtitle.chars().count() <= runtime::caps::SUBTITLE);
}

// ---- the manager: grants, review and disable on failure ------------------------------------------

#[test]
fn the_manager_waits_for_review_and_disables_a_crashing_extension() {
    let Some(wasm) = guest("lain_ext_probe") else { return };
    let d = temp("manager");
    let root = d.join("extensions");
    std::fs::create_dir_all(root.join("probe")).unwrap();
    let m = MANIFEST
        .replace("net:example.org", "events:subscribe")
        .replace("\"ui:rail\"]", "\"ui:rail\"],\"config\":{\"mode\":\"panic\"}");
    std::fs::write(root.join("probe/extension.json"), m).unwrap();
    std::fs::write(root.join("probe/probe.wasm"), wasm).unwrap();
    let (mut mgr, mut rx) = Manager::new(root);
    mgr.set_api(Some(Arc::new(FakeApi::default())));
    let mut cfg: BTreeMap<String, ExtSettings> = BTreeMap::new();
    // Installed, never reviewed: it waits for the prompt.
    assert_eq!(mgr.pending_review(&cfg), Some("probe".into()));
    mgr.sync(&cfg);
    assert_eq!(mgr.status["probe"], Status::NeedsReview);
    assert!(!mgr.is_running("probe"));
    // Reviewed with ui:rail only: it runs with exactly that.
    cfg.insert(
        "probe".into(),
        ExtSettings {
            enabled: true,
            granted: vec!["ui:rail".into(), "net:other.example".into()],
            reviewed: vec!["catalog:read".into(), "events:subscribe".into(), "ui:rail".into()],
            error: String::new(),
        },
    );
    assert_eq!(mgr.pending_review(&cfg), None);
    assert_eq!(mgr.grants("probe", &cfg), scopes(&["ui:rail"]));
    mgr.sync(&cfg);
    assert!(mgr.is_running("probe"));
    // It panics in `rails`: the manager stops and disables it.
    let end = Instant::now() + Duration::from_secs(30);
    let mut stopped = None;
    while Instant::now() < end && stopped.is_none() {
        match rx.try_recv() {
            Ok(o) => {
                if let Some(Notice::Stopped { reason, .. }) = mgr.on_out(o, &mut cfg) {
                    stopped = Some(reason);
                }
            }
            Err(_) => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    let reason = stopped.expect("the manager saw the failure");
    assert!(reason.contains("crashed"), "{reason}");
    assert!(!cfg["probe"].enabled);
    assert_eq!(cfg["probe"].error, reason);
    assert!(!mgr.is_running("probe"));
    assert!(matches!(mgr.status["probe"], Status::Failed(_)));
    mgr.sync(&cfg);
    assert!(!mgr.is_running("probe"), "a disabled extension stays stopped");
    let _ = std::fs::remove_dir_all(&d);
}

// ---- the sample extension ------------------------------------------------------------------------------------

#[test]
fn seasonal_stats_builds_its_rail_and_panel() {
    let Some(wasm) = guest("seasonal_stats") else { return };
    let d = temp("seasonal");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let api = Arc::new(FakeApi { now, ..Default::default() });
    let rt = runtime::Runtime::new().unwrap();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let worker = rt.start(
        runtime::Spec {
            id: "seasonal-stats".into(),
            wasm,
            config: r#"{"days":0}"#.into(),
            grants: scopes(&["catalog:read", "progress:read", "events:subscribe", "ui:rail", "ui:panel"]),
            limits: Limits::default(),
            data_dir: d.clone(),
            api: api.clone(),
        },
        tx,
    );
    let mut r = Run { rx, worker, _rt: rt };
    r.ready();
    let rails = r.rails();
    assert_eq!(rails.len(), 1);
    assert!(rails[0].title.ends_with("· new this season"), "{}", rails[0].title);
    let titles: Vec<&str> = rails[0].cards.iter().map(|c| c.title.as_str()).collect();
    assert_eq!(titles.len(), 2, "{titles:?}");
    let frieren = rails[0].cards.iter().find(|c| c.title == "Frieren").unwrap();
    assert_eq!(frieren.subtitle, "2 new · 0 finished");
    assert!(frieren.item_id.is_some());
    r.worker.send(Cmd::Panels("item-1".into()));
    match r.next(30) {
        Some(Out::Panels { panels, .. }) => {
            let rows: Vec<(String, String)> =
                panels[0].rows.iter().map(|r| (r.label.clone(), r.value.clone())).collect();
            assert!(rows.contains(&("new files".into(), "2 of 2".into())), "{rows:?}");
            assert!(rows.contains(&("finished".into(), "1".into())), "{rows:?}");
            assert!(rows.contains(&("time spent".into(), "34 min".into())), "{rows:?}");
        }
        other => panic!("expected Panels, got {other:?}"),
    }
    // A library change drops its catalog cache and refreshes the rail.
    let before = api.calls().iter().filter(|c| c.starts_with("catalog")).count();
    r.worker.send(Cmd::Event(runtime::Event::LibraryChanged));
    r.rails();
    assert!(api.calls().iter().filter(|c| c.starts_with("catalog")).count() > before);
    let _ = std::fs::remove_dir_all(&d);
}
