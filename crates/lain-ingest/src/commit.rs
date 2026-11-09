//! What one library's scan changes in the catalog (`commit_scan` of
//! `lain.catalog.write@1`), as a pure plan over the stored items: the
//! catalog provider applies it in one write transaction. A port of the
//! Go catalog's `CommitScan`, `ReconcileMoves` and `MarkMissing`.
//!
//! An item's *location* is `rel_path` inside its root for a rooted
//! library (D-157) and the absolute `file_path` otherwise; ids, the
//! by-path match and the alias history all use it, so a rooted catalog
//! means the same on every machine.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use lain_api::CatalogItem;
use lain_identify::{Candidate, Proposal};

/// Older locations kept per item.
pub const MAX_ALIASES: usize = 8;

/// One enumerated file's outcome. Unidentified entries still mark their
/// file present: an identify failure is never a deletion (D-068).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScanEntry {
    pub candidate: Candidate,
    #[serde(default)]
    pub proposal: Proposal,
    #[serde(default)]
    pub identified: bool,
    /// The file's `/`-separated path inside the library's root, for a
    /// rooted library (Rust-only, D-157).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rel_path: String,
}

/// One library's scan result, committed as one write. `reconcile_missing`
/// asserts the root was walked completely: only then may absent items
/// be marked missing.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CommitScanInput {
    pub library_id: String,
    #[serde(default)]
    pub entries: Vec<ScanEntry>,
    #[serde(default)]
    pub reconcile_missing: bool,
    /// The library's root name, empty for an unrooted library.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub root: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitScanOutput {
    pub migrated: i64,
    pub missing: i64,
    pub restored: i64,
}

/// The writes of one commit.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CommitPlan {
    /// Fresh records, written whole.
    pub upserts: Vec<CatalogItem>,
    /// Stored items whose `missing` flag flips (absent: true, back: false).
    pub flags: Vec<(String, bool)>,
    pub out: CommitScanOutput,
}

/// A stable id from library and location (SHA-256, 16 hex digits).
pub fn item_id(library_id: &str, loc: &str) -> String {
    let mut h = Sha256::new();
    h.update(library_id.as_bytes());
    h.update([0u8]);
    h.update(loc.as_bytes());
    h.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// The grouping key of a title: lower-case, whitespace collapsed.
pub fn title_key(title: &str) -> String {
    lain_identify::text::lower(&lain_identify::text::fields_join(title))
}

/// Where a stored item lives: `rel_path` when rooted, else `file_path`.
pub fn location(it: &CatalogItem) -> &str {
    if it.root.is_empty() { &it.file_path } else { &it.rel_path }
}

fn entry_loc<'a>(root: &str, e: &'a ScanEntry) -> &'a str {
    if root.is_empty() { &e.candidate.path } else { &e.rel_path }
}

fn same_logical(a: &CatalogItem, b: &CatalogItem) -> bool {
    a.library_id == b.library_id
        && a.kind == b.kind
        && title_key(&a.title) == title_key(&b.title)
        && a.season == b.season
        && a.episode == b.episode
        && (a.year == 0 || b.year == 0 || a.year == b.year)
}

/// The canonical item for a proposal; pure apart from `now`.
pub fn new_item(library_id: &str, root: &str, e: &ScanEntry, now: i64) -> CatalogItem {
    let p = &e.proposal;
    let loc = entry_loc(root, e);
    let mut it = CatalogItem {
        id: item_id(library_id, loc),
        library_id: library_id.into(),
        kind: p.kind.clone(),
        title: p.title.clone(),
        season: p.season,
        episode: p.episode,
        year: p.year,
        size: e.candidate.size,
        confidence: p.confidence,
        origin: p.plugin_id.clone(),
        provenance: format!("identify:{}", p.plugin_id),
        updated_at: now,
        ..CatalogItem::default()
    };
    if root.is_empty() {
        it.file_path = e.candidate.path.clone();
    } else {
        it.root = root.into();
        it.rel_path = e.rel_path.clone();
    }
    it
}

fn merge_aliases(dst: &mut Vec<String>, src: &[String]) {
    for a in src {
        if !dst.contains(a) {
            dst.push(a.clone());
        }
    }
}

fn trim_aliases(a: &mut Vec<String>) {
    if a.len() > MAX_ALIASES {
        a.drain(..a.len() - MAX_ALIASES);
    }
}

/// The library's stored items in the Go catalog's `ListByLibrary`
/// order (title, season, episode, year, id), which decides ties.
fn sorted(mut v: Vec<CatalogItem>) -> Vec<CatalogItem> {
    v.sort_by(|a, b| {
        a.title
            .cmp(&b.title)
            .then(a.season.cmp(&b.season))
            .then(a.episode.cmp(&b.episode))
            .then(a.year.cmp(&b.year))
            .then(a.id.cmp(&b.id))
    });
    v
}

/// Re-homes stable ids onto moved or renamed files (D-019): a scanned
/// item whose location-derived id is unknown adopts the id of an
/// existing item of the same logical identity whose location vanished,
/// recording the old location in `aliases`. A known location keeps its
/// row first, duplicates (both present) and cross-library titles never
/// merge, and each row is claimed once per pass.
fn reconcile_moves(
    library_id: &str,
    items: &mut [CatalogItem],
    present: &mut HashSet<String>,
    existing: &[CatalogItem],
) -> i64 {
    if items.is_empty() {
        return 0;
    }
    let by_id: HashMap<&str, &CatalogItem> = existing.iter().map(|it| (it.id.as_str(), it)).collect();
    let mut by_loc: HashMap<&str, &CatalogItem> = HashMap::new();
    for it in existing {
        by_loc.insert(location(it), it);
    }
    let mut claimed: HashSet<String> = HashSet::new();
    let mut settled = vec![false; items.len()];

    // Pass 1: a known location keeps its row.
    for (i, it) in items.iter_mut().enumerate() {
        let Some(row) = by_loc.get(location(it)).copied() else { continue };
        if claimed.contains(&row.id) {
            continue;
        }
        if row.id != it.id && !same_logical(row, it) {
            continue;
        }
        if row.id != it.id {
            present.remove(&it.id);
            it.id = row.id.clone();
        }
        merge_aliases(&mut it.aliases, &row.aliases);
        claimed.insert(row.id.clone());
        settled[i] = true;
    }

    let mut migrated = 0;
    for (i, it) in items.iter_mut().enumerate() {
        if settled[i] {
            continue;
        }
        if let Some(old) = by_id.get(it.id.as_str()).copied() {
            if claimed.contains(&it.id) {
                // The row this location derives to belongs to a file that
                // is still here: the newcomer gets its own identity.
                present.remove(&it.id);
                it.id = item_id(library_id, &format!("{}\0dup", location(it)));
            } else if same_logical(old, it) {
                // Move-back (A->B->A): adopt the history, log the vacated
                // location.
                merge_aliases(&mut it.aliases, &old.aliases);
                let old_loc = location(old).to_string();
                if !it.aliases.contains(&old_loc) {
                    it.aliases.push(old_loc);
                }
                trim_aliases(&mut it.aliases);
                claimed.insert(it.id.clone());
                migrated += 1;
                continue;
            }
            // Otherwise a different item takes over this id.
        }
        let mut found: Option<&CatalogItem> = None;
        for cand in existing {
            if cand.id == it.id || present.contains(&cand.id) || claimed.contains(&cand.id) || !same_logical(cand, it) {
                continue;
            }
            if found.is_none_or(|m| cand.updated_at < m.updated_at) {
                found = Some(cand);
            }
        }
        let Some(m) = found else { continue };
        let m_loc = location(m).to_string();
        if m_loc != location(it) && !it.aliases.contains(&m_loc) {
            it.aliases.push(m_loc);
        }
        merge_aliases(&mut it.aliases, &m.aliases);
        trim_aliases(&mut it.aliases);
        present.remove(&it.id);
        it.id = m.id.clone();
        claimed.insert(it.id.clone());
        migrated += 1;
    }
    for it in items.iter() {
        present.insert(it.id.clone());
    }
    migrated
}

/// Plans one library's commit over its stored items.
pub fn plan(input: &CommitScanInput, stored: Vec<CatalogItem>, now: i64) -> CommitPlan {
    let lib = input.library_id.as_str();
    let existing = sorted(stored.into_iter().filter(|it| it.library_id == lib).collect());
    let mut present: HashSet<String> = input.entries.iter().map(|e| item_id(lib, entry_loc(&input.root, e))).collect();
    let mut items: Vec<CatalogItem> =
        input.entries.iter().filter(|e| e.identified).map(|e| new_item(lib, &input.root, e, now)).collect();
    let migrated = reconcile_moves(lib, &mut items, &mut present, &existing);
    let mut plan = CommitPlan { out: CommitScanOutput { migrated, ..Default::default() }, ..Default::default() };
    if input.reconcile_missing {
        for it in &existing {
            match (present.contains(&it.id), it.missing) {
                (true, true) => {
                    plan.flags.push((it.id.clone(), false));
                    plan.out.restored += 1;
                }
                (false, false) => {
                    plan.flags.push((it.id.clone(), true));
                    plan.out.missing += 1;
                }
                _ => {}
            }
        }
    }
    plan.upserts = items;
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, title: &str, ep: i64) -> ScanEntry {
        ScanEntry {
            candidate: Candidate { path: path.into(), library_id: "lib".into(), ..Candidate::default() },
            proposal: Proposal {
                kind: "episode".into(),
                title: title.into(),
                episode: ep,
                confidence: 0.8,
                plugin_id: "p".into(),
                ..Proposal::default()
            },
            identified: true,
            rel_path: String::new(),
        }
    }

    fn input(entries: Vec<ScanEntry>) -> CommitScanInput {
        CommitScanInput { library_id: "lib".into(), entries, reconcile_missing: true, root: String::new() }
    }

    /// Applies a plan to a stored list, like the catalog does.
    fn apply(stored: &mut Vec<CatalogItem>, p: &CommitPlan) {
        for (id, m) in &p.flags {
            if let Some(it) = stored.iter_mut().find(|it| &it.id == id) {
                it.missing = *m;
            }
        }
        for u in &p.upserts {
            stored.retain(|it| it.id != u.id);
            stored.push(u.clone());
        }
    }

    #[test]
    fn ids_are_stable_and_location_derived() {
        assert_eq!(item_id("lib", "/a.mkv"), item_id("lib", "/a.mkv"));
        assert_ne!(item_id("lib", "/a.mkv"), item_id("lib2", "/a.mkv"));
        assert_eq!(item_id("lib", "/a.mkv").len(), 16);
    }

    #[test]
    fn rename_keeps_the_id_and_records_an_alias() {
        let mut db = Vec::new();
        let p = plan(&input(vec![entry("/m/Show - 01.mkv", "Show", 1)]), db.clone(), 10);
        apply(&mut db, &p);
        let id = db[0].id.clone();
        let p = plan(&input(vec![entry("/m/Show 01 renamed.mkv", "Show", 1)]), db.clone(), 20);
        assert_eq!((p.out.migrated, p.out.missing), (1, 0));
        apply(&mut db, &p);
        assert_eq!(db.len(), 1);
        assert_eq!((db[0].id.as_str(), db[0].aliases.clone()), (id.as_str(), vec!["/m/Show - 01.mkv".to_string()]));
        assert_eq!(db[0].file_path, "/m/Show 01 renamed.mkv");
        // Move back: same id, both locations in the history.
        let p = plan(&input(vec![entry("/m/Show - 01.mkv", "Show", 1)]), db.clone(), 30);
        apply(&mut db, &p);
        assert_eq!(db.len(), 1);
        assert_eq!(db[0].id, id);
        assert!(db[0].aliases.contains(&"/m/Show 01 renamed.mkv".to_string()));
    }

    #[test]
    fn duplicates_and_deletions() {
        let mut db = Vec::new();
        let p = plan(&input(vec![entry("/m/a.mkv", "Show", 1), entry("/m/b.mkv", "Show", 1)]), db.clone(), 1);
        apply(&mut db, &p);
        assert_eq!(db.len(), 2, "both copies present: no merge");
        let p = plan(&input(vec![entry("/m/a.mkv", "Show", 1)]), db.clone(), 2);
        assert_eq!((p.out.missing, p.out.migrated), (1, 0));
        apply(&mut db, &p);
        assert_eq!(db.iter().filter(|i| i.missing).count(), 1);
        let p = plan(&input(vec![entry("/m/a.mkv", "Show", 1), entry("/m/b.mkv", "Show", 1)]), db.clone(), 3);
        assert_eq!(p.out.restored, 1);
        // An unclean walk never marks anything missing.
        let mut partial = input(vec![]);
        partial.reconcile_missing = false;
        assert!(plan(&partial, db.clone(), 4).flags.is_empty());
        // An unidentified file keeps its row present.
        let mut unid = entry("/m/a.mkv", "", 0);
        unid.identified = false;
        let p = plan(&input(vec![unid, entry("/m/b.mkv", "Show", 1)]), db.clone(), 5);
        assert_eq!(p.out.missing, 0);
    }

    #[test]
    fn rooted_items_use_the_relative_location() {
        let mut e = entry("/mnt/v/media/anime/Show - 01.mkv", "Show", 1);
        e.rel_path = "anime/Show - 01.mkv".into();
        let mut i = input(vec![e]);
        i.root = "media".into();
        let p = plan(&i, Vec::new(), 1);
        let it = &p.upserts[0];
        assert_eq!(
            (it.root.as_str(), it.rel_path.as_str(), it.file_path.as_str()),
            ("media", "anime/Show - 01.mkv", "")
        );
        assert_eq!(it.id, item_id("lib", "anime/Show - 01.mkv"));
    }

    #[test]
    fn year_matching_and_oldest_wins() {
        let mut db = Vec::new();
        let mut a = entry("/m/old1.mkv", "Movie", 0);
        a.proposal.year = 2001;
        let p = plan(&input(vec![a]), db.clone(), 5);
        apply(&mut db, &p);
        // A move that loses the year still matches (unknown year).
        let b = entry("/m/new.mkv", "Movie", 0);
        let p = plan(&input(vec![b]), db.clone(), 9);
        assert_eq!(p.out.migrated, 1);
        // A different known year does not.
        let mut c = entry("/m/new2.mkv", "Movie", 0);
        c.proposal.year = 1999;
        assert_eq!(plan(&input(vec![c]), db.clone(), 9).out.migrated, 0);
    }
}
