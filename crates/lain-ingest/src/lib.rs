//! Library scans (core module): the filesystem source
//! (`lain.source.enumerate@1`), the scan pipeline (`lain.ingest.scan@1`),
//! the catalog commit plan behind `commit_scan` of
//! `lain.catalog.write@1` (identity v2 moves, D-019; missing marking,
//! D-068; `root` + `rel_path` locations, D-157), the library watcher
//! (`lain.source.watch@1`), the scan controller and the folder picker.

pub mod browse;
pub mod commit;
pub mod scan;
pub mod scanner;
pub mod source;
pub mod watch;

pub use commit::{CommitPlan, CommitScanInput, CommitScanOutput, ScanEntry, item_id, location, plan, title_key};
pub use scan::{CAP_INGEST_SCAN, INGEST_ID, LibraryDone, Runner, ScanInput};
pub use scanner::{ScanEvent, Scanner};
pub use source::{CAP_SOURCE_ENUMERATE, Filesystem, SOURCE_ID};
pub use watch::{CAP_SOURCE_WATCH, DEFAULT_DEBOUNCE, SourceWatch, WATCH_ID};
