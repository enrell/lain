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
    assert_eq!(s.status().progress.unwrap().enrich_total, 0, "a backfill outside the enrich phase is not the scan's");
    s.step(Step::Committing);
    s.step(Step::Walked { files: 0 });
    assert_eq!(s.status().progress.unwrap().phase, "commit", "a late walk does not move the scan back");
    s.step(Step::Library(&LibraryDone { library_id: "lib-a".into(), ..LibraryDone::default() }));
    assert_eq!(s.status().progress.unwrap().libraries_done, 1);
    let v = json!(s.status());
    assert_eq!(v["progress"]["phase"], "commit");
    assert!(v["progress"].get("enrich_total").is_none(), "{v}");
    drop(held);
    assert!(s.wait_idle(Duration::from_secs(5)));
    let st = s.status();
    assert_eq!(st.state, "done");
    assert!(st.progress.is_none(), "a finished scan has no position");
}
