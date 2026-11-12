//! `lain-ingest-default` (`lain.ingest.scan@1`): the scan pipeline. It
//! knows the order but no policy: the source enumerates, identifiers
//! propose (ordered-many, first accepted), the catalog commits one
//! write per library (`commit_scan`, D-075). Roots walk in parallel; a
//! root that could not be walked completely never marks items missing
//! (D-068). A port of `internal/plugins/ingest`.

use std::sync::{Arc, Weak};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use lain_api::{Library, ScanStats, UnreadableRoot};
use lain_core::{Error, Provider, Registry};
use lain_identify::{CAP_MEDIA_IDENTIFY, Proposal};

use crate::commit::{CommitScanInput, CommitScanOutput, ScanEntry};
use crate::source::{CAP_SOURCE_ENUMERATE, EnumerateInput, EnumerateOutput};

pub const INGEST_ID: &str = "lain-ingest-default";
pub const CAP_INGEST_SCAN: &str = "lain.ingest.scan@1";
pub const CAP_CATALOG_WRITE: &str = "lain.catalog.write@1";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScanInput {
    #[serde(default)]
    pub libraries: Vec<Library>,
}

/// One library's committed outcome, reported as it lands.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LibraryDone {
    pub library_id: String,
    pub candidates: i64,
    pub identified: i64,
    pub committed: bool,
    pub commit: CommitScanOutput,
}

/// One step of a running scan, in the order a library goes through
/// them. Roots walk in parallel, so steps of different libraries
/// interleave.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step<'a> {
    /// A root was listed (or could not be): `files` candidates found.
    Walked { files: i64 },
    /// One candidate went through the identifiers.
    Identified,
    /// Every root is read; the commits start.
    Committing,
    /// One library's commit landed (or failed).
    Library(&'a LibraryDone),
}

/// Called at each step (in-process observers only).
pub type Progress = Arc<dyn Fn(Step<'_>) + Send + Sync>;

/// The provider. It reaches the registry through a weak reference: the
/// registry owns it.
pub struct Runner {
    registry: Weak<Registry>,
    progress: Option<Progress>,
}

impl Runner {
    pub fn new(registry: &Arc<Registry>, progress: Option<Progress>) -> Self {
        Runner { registry: Arc::downgrade(registry), progress }
    }

    /// Walks, identifies and commits every library.
    pub fn run(&self, input: &ScanInput) -> Result<ScanStats, Error> {
        let reg = self.registry.upgrade().ok_or_else(|| Error::unavailable("registry closed"))?;
        let mut stats = ScanStats {
            started_at: lain_core::now_unix(),
            libraries: input.libraries.len() as i64,
            ..ScanStats::default()
        };
        let r: &Registry = &reg;
        let step = |s: Step<'_>| {
            if let Some(p) = &self.progress {
                p(s);
            }
        };
        let step = &step;
        let results: Vec<RootResult> = std::thread::scope(|s| {
            let handles: Vec<_> = input.libraries.iter().map(|lib| s.spawn(move || scan_root(r, lib, step))).collect();
            handles.into_iter().map(|h| h.join().unwrap_or_default()).collect()
        });
        step(Step::Committing);
        for (res, lib) in results.iter().zip(&input.libraries) {
            stats.candidates += res.candidates;
            stats.identified += res.identified;
            stats.errors += res.errors;
            stats.walk_errors += res.walk_errors;
            stats.dirs += res.dirs;
            let reason = if !res.accessible {
                Some("the path could not be read".to_string())
            } else if res.walk_errors > 0 {
                Some(format!("{} directories could not be read", res.walk_errors))
            } else {
                None
            };
            if let Some(reason) = reason {
                stats.unreadable.push(UnreadableRoot {
                    library_id: lib.id.clone(),
                    name: lib.name.clone(),
                    path: lib.path.clone(),
                    reason,
                });
            }
        }
        // One commit per root. A failed commit counts as an error and
        // leaves that root's previous catalog intact.
        for (res, lib) in results.into_iter().zip(&input.libraries) {
            let commit = CommitScanInput {
                library_id: lib.id.clone(),
                reconcile_missing: res.accessible && res.walk_errors == 0,
                root: lib.root.clone(),
                entries: res.entries,
            };
            let mut done = LibraryDone {
                library_id: lib.id.clone(),
                candidates: res.candidates,
                identified: res.identified,
                ..LibraryDone::default()
            };
            match reg.call_one(CAP_CATALOG_WRITE, "commit_scan", json!(commit)) {
                Ok((out, _)) => match serde_json::from_value::<CommitScanOutput>(out) {
                    Ok(c) => {
                        stats.migrated += c.migrated;
                        stats.missing += c.missing;
                        stats.restored += c.restored;
                        done.committed = true;
                        done.commit = c;
                    }
                    Err(_) => stats.errors += 1,
                },
                Err(e) => {
                    tracing::warn!(library = %lib.id, err = %e, "scan commit failed");
                    stats.errors += 1;
                }
            }
            tracing::info!(
                library = %lib.id,
                candidates = done.candidates,
                identified = done.identified,
                committed = done.committed,
                "library scanned"
            );
            step(Step::Library(&done));
        }
        stats.unidentified = stats.candidates - stats.identified;
        stats.finished_at = lain_core::now_unix();
        Ok(stats)
    }
}

#[derive(Default)]
struct RootResult {
    entries: Vec<ScanEntry>,
    accessible: bool,
    candidates: i64,
    identified: i64,
    errors: i64,
    walk_errors: i64,
    dirs: i64,
}

/// `rel` of an absolute path below the library directory, joined onto
/// the library's path inside its root (D-157).
fn rel_path(lib: &Library, path: &str) -> String {
    let below = std::path::Path::new(path)
        .strip_prefix(&lib.path)
        .map(|p| p.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/"))
        .unwrap_or_default();
    match (lib.root_path.trim_matches('/'), below.as_str()) {
        ("", b) => b.to_string(),
        (r, "") => r.to_string(),
        (r, b) => format!("{r}/{b}"),
    }
}

fn scan_root(reg: &Registry, lib: &Library, step: &(dyn Fn(Step<'_>) + Sync)) -> RootResult {
    let mut res = RootResult::default();
    // A root that was not walked still reports, so the walked count
    // always reaches the library count.
    let out = enumerate_root(reg, lib, &mut res);
    step(Step::Walked { files: out.as_ref().map_or(0, |o| o.candidates.len() as i64) });
    let Some(out) = out else { return res };
    tracing::info!(library = %lib.id, files = out.candidates.len(), dirs = out.stats.dirs, "library walked");
    res.dirs = out.stats.dirs;
    res.walk_errors = out.stats.walk_errors;
    res.accessible = out.stats.accessible;
    for c in out.candidates {
        res.candidates += 1;
        let accepted = |v: &Value| serde_json::from_value::<Proposal>(v.clone()).is_ok_and(|p| p.accepted());
        // A file no identifier accepts still exists: an identify failure
        // is never a deletion (D-011/D-068).
        let mut entry = ScanEntry {
            rel_path: if lib.root.is_empty() { String::new() } else { rel_path(lib, &c.path) },
            ..ScanEntry::default()
        };
        match reg.call_first(CAP_MEDIA_IDENTIFY, "identify", json!(c), accepted) {
            Ok(Some((out, _))) => match serde_json::from_value::<Proposal>(out) {
                Ok(p) => {
                    entry.proposal = p;
                    entry.identified = true;
                    res.identified += 1;
                }
                Err(_) => res.errors += 1,
            },
            _ => res.errors += 1,
        }
        entry.candidate = c;
        res.entries.push(entry);
        step(Step::Identified);
    }
    crate::siblings::number_siblings(&mut res.entries);
    res
}

/// The root's listing; `None` (counted in `res.errors`) when it could
/// not be walked: nothing may be marked missing then.
fn enumerate_root(reg: &Registry, lib: &Library, res: &mut RootResult) -> Option<EnumerateOutput> {
    let input = EnumerateInput { root: lib.path.clone(), library_id: lib.id.clone(), kind: lib.kind.clone() };
    let out = match reg.call_one(CAP_SOURCE_ENUMERATE, "enumerate", json!(input)) {
        Ok((out, _)) => out,
        Err(e) => {
            tracing::warn!(library = %lib.id, err = %e, "library root not walked");
            res.errors += 1;
            return None;
        }
    };
    let out = serde_json::from_value::<EnumerateOutput>(out).ok();
    if out.is_none() {
        res.errors += 1;
    }
    out
}

impl Provider for Runner {
    fn id(&self) -> &str {
        INGEST_ID
    }

    fn capabilities(&self) -> Vec<String> {
        vec![CAP_INGEST_SCAN.into()]
    }

    fn health(&self) -> Result<(), Error> {
        if self.registry.strong_count() == 0 {
            return Err(Error::unavailable("registry closed"));
        }
        Ok(())
    }

    fn invoke(&self, cap: &str, op: &str, input: Value) -> Result<Value, Error> {
        if cap != CAP_INGEST_SCAN {
            return Err(Error::invalid(format!("unsupported cap {cap}")));
        }
        if !op.is_empty() && op != "scan" {
            return Err(Error::invalid(format!("capability {cap} has no op {op}")));
        }
        let i: ScanInput =
            serde_json::from_value(input).map_err(|e| Error::invalid(format!("ScanInput required: {e}")))?;
        Ok(json!(self.run(&i)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rel_paths_join_the_library_path_in_its_root() {
        let lib = Library {
            path: "/mnt/v/media/anime".into(),
            root: "media".into(),
            root_path: "anime".into(),
            ..Library::default()
        };
        assert_eq!(rel_path(&lib, "/mnt/v/media/anime/Show/01.mkv"), "anime/Show/01.mkv");
        let top = Library { path: "/mnt/v/media".into(), root: "media".into(), ..Library::default() };
        assert_eq!(rel_path(&top, "/mnt/v/media/Show/01.mkv"), "Show/01.mkv");
    }
}
