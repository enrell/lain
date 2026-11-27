//! The scan controller over a stub pipeline: one scan at a time, the
//! watcher re-marks a library dirty instead of racing a running scan,
//! failures end in `error`, observers hear every transition.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use lain_api::Library;
use lain_core::{Binding, BindingMode, Composition, Error, Provider, Registry};
use lain_ingest::{CAP_INGEST_SCAN, CAP_SOURCE_WATCH, LibraryDone, ScanEvent, Scanner, SourceWatch, Step};

struct SlowScan {
    gate: Arc<Mutex<()>>,
    fail: bool,
}

impl Provider for SlowScan {
    fn id(&self) -> &str {
        "stub-scan"
    }
    fn capabilities(&self) -> Vec<String> {
        vec![CAP_INGEST_SCAN.into()]
    }
    fn health(&self) -> Result<(), Error> {
        Ok(())
    }
    fn invoke(&self, _: &str, _: &str, input: Value) -> Result<Value, Error> {
        let _held = self.gate.lock().unwrap();
        if self.fail {
            return Err(Error::internal("walk exploded"));
        }
        let n = input["libraries"].as_array().map(|a| a.len()).unwrap_or(0);
        Ok(json!({"libraries": n, "candidates": 0, "identified": 0, "unidentified": 0, "errors": 0, "pruned": 0,
                  "missing": 0, "restored": 0, "migrated": 0, "enriched": 0, "walk_errors": 0, "finished_at": 7}))
    }
}

fn setup(fail: bool) -> (Arc<Registry>, Arc<Mutex<()>>, Arc<SourceWatch>) {
    let mut bindings = std::collections::BTreeMap::new();
    bindings.insert(CAP_INGEST_SCAN.to_string(), Binding::new(BindingMode::ExactlyOne, &["stub-scan"]));
    bindings.insert(CAP_SOURCE_WATCH.to_string(), Binding::new(BindingMode::ExactlyOne, &[lain_ingest::WATCH_ID]));
    bindings.insert(
        lain_ingest::scanner::CAP_METADATA_ENRICH.to_string(),
        Binding::new(BindingMode::ExactlyOne, &["stub-enrich"]),
    );
    let reg = Arc::new(Registry::new(Composition { version: 8, bindings }));
    let gate = Arc::new(Mutex::new(()));
    reg.register(Arc::new(SlowScan { gate: gate.clone(), fail }));
    let watch = Arc::new(SourceWatch::new(Duration::from_secs(60), false));
    reg.register(watch.clone());
    (reg, gate, watch)
}

fn libs() -> lain_ingest::scanner::LibrarySource {
    Arc::new(|| vec![Library { id: "lib-a".into(), path: "/m/a".into(), ..Library::default() }])
}

#[test]
fn one_scan_at_a_time_and_watch_renudges() {
    let (reg, gate, watch) = setup(false);
    let s = Scanner::new(&reg, libs(), false);
    let events = Arc::new(Mutex::new(Vec::new()));
    let ev = events.clone();
    s.set_sink(Arc::new(move |e| ev.lock().unwrap().push(e)));
    assert_eq!(s.status().state, "idle");
    let held = gate.lock().unwrap();
    assert!(s.start_all("manual"));
    assert!(s.running());
    assert!(!s.start_all("manual"), "a second scan is refused while one runs");
    s.poll_watch();
    assert!(watch.watching("lib-a"));
    s.scan_library("lib-a");
    drop(held);
    assert!(s.wait_idle(Duration::from_secs(5)));
    let st = s.status();
    assert_eq!((st.state.as_str(), st.trigger.as_str(), st.finished_at), ("done", "manual", 7));
    assert_eq!(st.stats.unwrap().libraries, 1);
    let got = events.lock().unwrap().clone();
    assert!(matches!(&got[0], ScanEvent::Started { trigger, libraries: 1 } if trigger == "manual"));
    assert!(matches!(got.last(), Some(ScanEvent::Finished(st)) if st.state == "done"));
    assert_eq!(got.len(), 2, "the watch nudge did not start a scan: {got:?}");
    // The nudge re-marked the library dirty in the provider instead.
    let out = reg.call_one(CAP_SOURCE_WATCH, "poll", json!({"libraries": [{"id": "lib-a", "path": "/m/a"}]})).unwrap();
    assert_eq!(out.0["dirty"], json!([]), "inside the 60 s window");
    s.scan_library("lib-gone");
    assert_eq!(s.status().trigger, "manual", "an unknown library starts nothing");
}

#[test]
fn a_failed_pipeline_ends_in_error() {
    let (reg, _, _) = setup(true);
    let s = Scanner::new(&reg, libs(), true);
    assert!(s.start_all("manual"));
    assert!(s.wait_idle(Duration::from_secs(5)));
    let st = s.status();
    assert_eq!(st.state, "error");
    assert!(st.error.contains("walk exploded"), "{}", st.error);
    assert!(st.stats.is_none() && st.finished_at > 0);
}

/// A running scan says where it is; phases only move forward and the
/// position ends with the scan.
#[test]
fn a_running_scan_reports_its_position() {
    let (reg, gate, _) = setup(false);
    let s = Scanner::new(&reg, libs(), false);
    assert!(s.status().progress.is_none(), "an idle scanner has no position");
    s.step(Step::Identified);
    assert!(s.status().progress.is_none(), "a step outside a scan is ignored");
    let held = gate.lock().unwrap();
    assert!(s.start_all("manual"));
    let p = s.status().progress.expect("a running scan has a position");
    assert_eq!((p.phase.as_str(), p.libraries, p.files), ("walk", 1, 0));
    s.step(Step::Walked { files: 3 });
    s.step(Step::Identified);
    s.step(Step::Identified);
    let p = s.status().progress.unwrap();
    assert_eq!((p.phase.as_str(), p.libraries_walked, p.files, p.files_done), ("identify", 1, 3, 2));
    s.enrich_progress(1, 9);
    assert!(s.status().enrich.is_none(), "a backfill this controller did not start is not its own");
    s.step(Step::Committing);
    s.step(Step::Walked { files: 0 });
    assert_eq!(s.status().progress.unwrap().phase, "commit", "a late walk does not move the scan back");
    s.step(Step::Library(&LibraryDone { library_id: "lib-a".into(), ..LibraryDone::default() }));
    assert_eq!(s.status().progress.unwrap().libraries_done, 1);
    let v = json!(s.status());
    assert_eq!(v["progress"]["phase"], "commit");
    drop(held);
    assert!(s.wait_idle(Duration::from_secs(5)));
    let st = s.status();
    assert_eq!(st.state, "done");
    assert!(st.progress.is_none(), "a finished scan has no position");
}

/// D-286: a scan asked for while one runs is queued, once, and starts
/// by itself when the running one ends, over the libraries there are
/// then (one added meanwhile included).
#[test]
fn a_scan_asked_for_while_one_runs_follows_it_once() {
    use lain_ingest::scanner::ScanRequest;
    let (reg, gate, _) = setup(false);
    let added = Arc::new(Mutex::new(false));
    let seen = added.clone();
    let source: lain_ingest::scanner::LibrarySource = Arc::new(move || {
        let mut l = vec![Library { id: "lib-a".into(), path: "/m/a".into(), ..Library::default() }];
        if *seen.lock().unwrap() {
            l.push(Library { id: "lib-b".into(), path: "/m/b".into(), ..Library::default() });
        }
        l
    });
    let s = Scanner::new(&reg, source, false);
    let events = Arc::new(Mutex::new(Vec::new()));
    let ev = events.clone();
    s.set_sink(Arc::new(move |e| ev.lock().unwrap().push(e)));
    let held = gate.lock().unwrap();
    assert_eq!(s.request_all("manual"), ScanRequest::Started);
    assert!(!s.status().queued);
    *added.lock().unwrap() = true;
    assert_eq!(s.request_all("manual"), ScanRequest::Queued);
    assert_eq!(s.request_all("manual"), ScanRequest::Queued, "asked twice, still one follow-up");
    assert!(s.status().queued);
    assert_eq!(json!(s.status())["queued"], true);
    drop(held);
    let finished = || events.lock().unwrap().iter().filter(|e| matches!(e, ScanEvent::Finished(_))).count();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while finished() < 2 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(s.wait_idle(Duration::from_secs(5)));
    let got = events.lock().unwrap().clone();
    let started: Vec<i64> = got
        .iter()
        .filter_map(|e| if let ScanEvent::Started { libraries, .. } = e { Some(*libraries) } else { None })
        .collect();
    assert_eq!(started, vec![1, 2], "the follow-up walked the library added meanwhile: {got:?}");
    let st = s.status();
    assert_eq!((st.state.as_str(), st.queued), ("done", false));
    assert!(json!(st).get("queued").is_none(), "false is left out of the answer");
}

/// A backfill that waits on a gate, as a slow provider would.
struct SlowEnrich {
    gate: Arc<Mutex<()>>,
    calls: Arc<Mutex<Vec<Value>>>,
}

impl Provider for SlowEnrich {
    fn id(&self) -> &str {
        "stub-enrich"
    }
    fn capabilities(&self) -> Vec<String> {
        vec![lain_ingest::scanner::CAP_METADATA_ENRICH.into()]
    }
    fn health(&self) -> Result<(), Error> {
        Ok(())
    }
    fn invoke(&self, _: &str, _: &str, input: Value) -> Result<Value, Error> {
        self.calls.lock().unwrap().push(input);
        let _held = self.gate.lock().unwrap();
        Ok(json!({"checked": 4, "enriched": 3, "skipped": 2}))
    }
}

/// D-302: a scan ends when the catalog is committed; the metadata pass
/// runs on its own, a scan can start while it runs, and a pass asked
/// for meanwhile follows it once, with `retry` carried over.
#[test]
fn the_metadata_pass_runs_apart_from_the_scan() {
    let (reg, _gate, _) = setup(false);
    let egate = Arc::new(Mutex::new(()));
    let calls = Arc::new(Mutex::new(Vec::new()));
    reg.register(Arc::new(SlowEnrich { gate: egate.clone(), calls: calls.clone() }));
    let s = Scanner::new(&reg, libs(), true);
    let events = Arc::new(Mutex::new(Vec::new()));
    let ev = events.clone();
    s.set_sink(Arc::new(move |e| ev.lock().unwrap().push(e)));

    let held = egate.lock().unwrap();
    assert!(s.start_all("manual"));
    assert!(s.wait_idle(Duration::from_secs(5)), "the scan does not wait for metadata");
    assert_eq!(s.status().state, "done");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while calls.lock().unwrap().is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(s.enriching());
    assert_eq!(s.status().enrich.unwrap().state, "running");
    s.enrich_progress(1, 4);
    assert_eq!(s.status().enrich.map(|e| (e.done, e.total)), Some((1, 4)));
    assert!(s.start_all("manual"), "a scan starts while metadata is fetched");
    assert!(s.wait_idle(Duration::from_secs(5)));
    assert!(!s.request_enrich(true), "asked while running: one more pass follows");
    drop(held);
    assert!(s.wait_enriched(Duration::from_secs(5)));
    let e = s.status().enrich.unwrap();
    assert_eq!((e.state.as_str(), e.enriched, e.skipped), ("done", 3, 2));
    assert!(e.finished_at >= e.started_at && e.started_at > 0);
    let got = calls.lock().unwrap().clone();
    assert_eq!(got, vec![json!({"retry": false}), json!({"retry": true})], "two passes, not three");
    let enrich_events = events.lock().unwrap().iter().filter(|e| matches!(e, ScanEvent::Enrich(_))).count();
    assert_eq!(enrich_events, 2, "started and finished");
}
