//! The catalog and userstate providers over redb (`lain-catalog-redb`,
//! `lain-userstate-redb`): the Rust ports of the bolt providers, behind
//! the same capabilities and ops. Catalog owns identity; userstate owns
//! progress; neither touches the other's table.

use std::sync::{Arc, RwLock};

use serde::Deserialize;
use serde_json::{Value, json};

use lain_core::{Error, Provider};
use lain_store::{Bucket, Store, StoreError, join_key, key_prefix};

use lain_ingest::{CommitScanInput, CommitScanOutput};

use crate::contracts::{
    CatalogItem, CatalogPage, EXPORT_FORMAT, EXPORT_VERSION, ExportDoc, GROUP_TITLE, PageParams, Progress,
};
use crate::roots::Roots;

pub const CAP_CATALOG_READ: &str = "lain.catalog.read@1";
pub const CAP_CATALOG_WRITE: &str = "lain.catalog.write@1";
pub const CAP_USER_PROGRESS: &str = "lain.userstate.progress@1";
pub const CATALOG_ID: &str = "lain-catalog-redb";
pub const USERSTATE_ID: &str = "lain-userstate-redb";

fn store_err(e: StoreError) -> Error {
    Error::internal(e.to_string())
}

fn input<T: for<'de> Deserialize<'de>>(v: Value, shape: &str) -> Result<T, Error> {
    serde_json::from_value(v).map_err(|e| Error::invalid(format!("{shape} required: {e}")))
}

/// The grouping key of a title: lower-case, whitespace collapsed (the
/// web Library grid's rule). Per-character simple lower-casing, like Go.
pub fn title_key(title: &str) -> String {
    title
        .split_whitespace()
        .map(|w| w.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

fn title_order(a: &CatalogItem, b: &CatalogItem) -> std::cmp::Ordering {
    a.title
        .cmp(&b.title)
        .then(a.season.cmp(&b.season))
        .then(a.episode.cmp(&b.episode))
        .then(a.year.cmp(&b.year))
        .then(a.id.cmp(&b.id))
}

fn watch_order(a: &CatalogItem, b: &CatalogItem) -> std::cmp::Ordering {
    a.season.cmp(&b.season).then(a.episode.cmp(&b.episode)).then(a.year.cmp(&b.year)).then(a.id.cmp(&b.id))
}

/// A page of titles from files already filtered and sorted (D-283): the
/// files sharing a `title_key` are one title, placed where its first file
/// stands in the sort (the newest file for `recent`, the first by title
/// otherwise), with its files in watch order. The total counts titles and
/// a title is never split across pages, so a card counts all its episodes
/// and a search says how many titles match, not how many were loaded.
fn page_titles(files: Vec<CatalogItem>, p: &PageParams) -> (Vec<CatalogItem>, i64) {
    let mut at: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut titles: Vec<Vec<CatalogItem>> = Vec::new();
    for it in files {
        let key = title_key(&it.title);
        match at.get(&key) {
            Some(&i) => titles[i].push(it),
            None => {
                at.insert(key, titles.len());
                titles.push(vec![it]);
            }
        }
    }
    let total = titles.len() as i64;
    let (start, end) = if p.limit >= 0 {
        let start = p.offset.clamp(0, total) as usize;
        (start, (p.offset.max(0) + p.limit).min(total) as usize)
    } else {
        (0, titles.len())
    };
    let mut out = Vec::new();
    for mut title in titles.into_iter().take(end).skip(start) {
        title.sort_by(watch_order);
        out.extend(title);
    }
    (out, total)
}

/// The catalog provider. Items with a root are resolved against this
/// machine's root mapping on every read (D-157).
pub struct Catalog {
    store: Arc<Store>,
    roots: Arc<RwLock<Roots>>,
}

#[derive(Deserialize)]
struct IdInput {
    #[serde(default)]
    id: String,
}

#[derive(Deserialize)]
struct PageInput {
    #[serde(default)]
    page: PageParams,
}

#[derive(Deserialize)]
struct QueryInput {
    #[serde(default)]
    q: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    page: PageParams,
}

#[derive(Deserialize)]
struct SetMissingInput {
    #[serde(default)]
    id: String,
    #[serde(default)]
    missing: bool,
}

#[derive(Deserialize)]
struct DeleteLibraryInput {
    #[serde(default)]
    library_id: String,
}

impl Catalog {
    pub fn new(store: Arc<Store>) -> Self {
        Self::with_roots(store, Arc::new(RwLock::new(Roots::default())))
    }

    pub fn with_roots(store: Arc<Store>, roots: Arc<RwLock<Roots>>) -> Self {
        Catalog { store, roots }
    }

    fn resolve(&self, mut it: CatalogItem) -> CatalogItem {
        if !it.root.is_empty()
            && let Some(p) = self.roots.read().unwrap_or_else(|e| e.into_inner()).resolve(&it.root, &it.rel_path)
        {
            it.file_path = p;
        }
        it
    }

    fn all(&self) -> Result<Vec<CatalogItem>, Error> {
        let rows = self.store.read(|tx| tx.all(Bucket::Items)).map_err(store_err)?;
        // A record that does not decode is skipped, as in Go's walk.
        Ok(rows.iter().filter_map(|(_, v)| serde_json::from_slice(v).ok()).map(|it| self.resolve(it)).collect())
    }

    pub fn get(&self, id: &str) -> Result<Option<CatalogItem>, Error> {
        let it: Option<CatalogItem> =
            self.store.read(|tx| tx.get_json(Bucket::Items, id.as_bytes())).map_err(store_err)?;
        Ok(it.map(|it| self.resolve(it)))
    }

    /// Filters by library, substring and kind, sorts, then pages; the
    /// total counts the filtered set. A negative limit is unbounded.
    /// Grouped by title (D-283), the page and the total count titles.
    pub fn query(&self, q: &str, kind: &str, p: &PageParams) -> Result<(Vec<CatalogItem>, i64), Error> {
        let ql = q.trim().to_lowercase();
        let kl = kind.trim().to_lowercase();
        let ll = p.library_id.trim();
        let mut out: Vec<CatalogItem> = self
            .all()?
            .into_iter()
            .filter(|it| ll.is_empty() || it.library_id == ll)
            .filter(|it| !p.exclude_libraries.contains(&it.library_id))
            .filter(|it| kl.is_empty() || it.kind.to_lowercase() == kl)
            .filter(|it| ql.is_empty() || it.title.to_lowercase().contains(&ql))
            .collect();
        if p.sort == "recent" {
            out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.id.cmp(&b.id)));
        } else {
            out.sort_by(title_order);
        }
        if p.group == GROUP_TITLE {
            return Ok(page_titles(out, p));
        }
        let total = out.len() as i64;
        if p.limit >= 0 {
            if p.offset >= total {
                return Ok((Vec::new(), total));
            }
            let end = (p.offset + p.limit).min(total) as usize;
            out = out.drain(p.offset as usize..end).collect();
        }
        Ok((out, total))
    }

    pub fn page(&self, p: &PageParams) -> Result<CatalogPage, Error> {
        let (items, total) = self.query("", "", p)?;
        Ok(CatalogPage { items, total, limit: p.limit, offset: p.offset, group: p.group.clone() })
    }

    /// Every file of the item's title in watch order, across libraries.
    pub fn episodes(&self, id: &str) -> Result<Option<Vec<CatalogItem>>, Error> {
        let Some(it) = self.get(id)? else { return Ok(None) };
        let key = title_key(&it.title);
        let mut out: Vec<CatalogItem> = self.all()?.into_iter().filter(|c| title_key(&c.title) == key).collect();
        out.sort_by(watch_order);
        Ok(Some(out))
    }

    /// Removes a library's items and their index rows.
    pub fn delete_library(&self, library_id: &str) -> Result<i64, Error> {
        self.store
            .write(|tx| {
                let ids: Vec<Vec<u8>> = tx
                    .scan_prefix(Bucket::ItemsByLibrary, &key_prefix(library_id))?
                    .into_iter()
                    .map(|(k, _)| k[library_id.len() + 1..].to_vec())
                    .collect();
                for id in &ids {
                    tx.delete(Bucket::Items, id)?;
                    tx.delete(Bucket::ItemsByLibrary, &join_key(library_id, &String::from_utf8_lossy(id)))?;
                }
                Ok(ids.len() as i64)
            })
            .map_err(store_err)
    }

    /// Commits one library's scan in one write transaction (D-075): the
    /// plan (`lain_ingest::plan`) reconciles moves (D-019) and marks
    /// missing files (D-068) against the stored rows.
    pub fn commit_scan(&self, input: &CommitScanInput) -> Result<CommitScanOutput, Error> {
        if input.library_id.is_empty() {
            return Err(Error::invalid("library_id required"));
        }
        let lib = input.library_id.as_str();
        self.store
            .write(|tx| {
                let mut stored = Vec::new();
                for (k, _) in tx.scan_prefix(Bucket::ItemsByLibrary, &key_prefix(lib))? {
                    // A record that does not decode is skipped, as in reads.
                    if let Ok(Some(it)) = tx.get_json::<CatalogItem>(Bucket::Items, &k[lib.len() + 1..]) {
                        stored.push(it);
                    }
                }
                let plan = lain_ingest::plan(input, stored, lain_core::now_unix());
                for (id, missing) in &plan.flags {
                    if let Some(mut it) = tx.get_json::<CatalogItem>(Bucket::Items, id.as_bytes())? {
                        it.missing = *missing;
                        tx.put_json(Bucket::Items, id.as_bytes(), &it)?;
                    }
                }
                for it in &plan.upserts {
                    tx.put_json(Bucket::Items, it.id.as_bytes(), it)?;
                    tx.put(Bucket::ItemsByLibrary, &join_key(&it.library_id, &it.id), b"")?;
                }
                Ok(plan.out)
            })
            .map_err(store_err)
    }

    pub fn set_missing(&self, id: &str, missing: bool) -> Result<bool, Error> {
        self.store
            .write(|tx| {
                let Some(mut it) = tx.get_json::<CatalogItem>(Bucket::Items, id.as_bytes())? else { return Ok(false) };
                it.missing = missing;
                tx.put_json(Bucket::Items, id.as_bytes(), &it)?;
                Ok(true)
            })
            .map_err(store_err)
    }

    /// The portable document, `lain.catalog-export@3` (D-175): aliases
    /// and roots; `file_path` is this machine's resolution.
    pub fn export(&self) -> Result<ExportDoc, Error> {
        let mut doc = ExportDoc { format: EXPORT_FORMAT.into(), version: EXPORT_VERSION, ..ExportDoc::default() };
        for it in self.all()? {
            if !it.root.is_empty() && !doc.roots.contains(&it.root) {
                doc.roots.push(it.root.clone());
            }
            doc.items.insert(it.id.clone(), it);
        }
        doc.roots.sort();
        Ok(doc)
    }

    /// Replaces the catalog with a document in one transaction, the
    /// library index rebuilt with it. Only @3 is accepted; a rooted
    /// item keeps `root` + `rel_path` and drops the document's
    /// `file_path` (another machine's resolution).
    pub fn import(&self, doc: &ExportDoc) -> Result<usize, Error> {
        if doc.format != EXPORT_FORMAT || doc.version != EXPORT_VERSION {
            return Err(Error::invalid(format!(
                "unsupported export format/version (want {EXPORT_FORMAT}@{EXPORT_VERSION})"
            )));
        }
        self.store
            .write(|tx| {
                tx.clear(Bucket::ItemsByLibrary)?;
                tx.clear(Bucket::Items)?;
                for (id, it) in &doc.items {
                    let mut it = it.clone();
                    it.id = id.clone();
                    if !it.root.is_empty() {
                        it.file_path.clear();
                    }
                    tx.put_json(Bucket::Items, id.as_bytes(), &it)?;
                    tx.put(Bucket::ItemsByLibrary, &join_key(&it.library_id, id), b"")?;
                }
                Ok(doc.items.len())
            })
            .map_err(store_err)
    }
}

impl Provider for Catalog {
    fn id(&self) -> &str {
        CATALOG_ID
    }

    fn capabilities(&self) -> Vec<String> {
        vec![CAP_CATALOG_READ.into(), CAP_CATALOG_WRITE.into()]
    }

    fn health(&self) -> Result<(), Error> {
        Ok(())
    }

    fn invoke(&self, cap: &str, op: &str, v: Value) -> Result<Value, Error> {
        let out = match (cap, op) {
            (CAP_CATALOG_READ, "list") => {
                let (items, _) =
                    self.query("", "", &PageParams { limit: -1, sort: "title".into(), ..PageParams::default() })?;
                json!(items)
            }
            (CAP_CATALOG_READ, "get") => {
                let i: IdInput = input(v, "GetInput")?;
                json!(self.get(&i.id)?.ok_or_else(|| Error::not_found(format!("unknown item {}", i.id)))?)
            }
            (CAP_CATALOG_READ, "page") => json!(self.page(&input::<PageInput>(v, "PageInput")?.page)?),
            // Rust-only: substring and kind filter, what lain.search.query@1
            // serves in Go.
            (CAP_CATALOG_READ, "query") => {
                let i: QueryInput = input(v, "QueryInput")?;
                let (items, total) = self.query(&i.q, &i.kind, &i.page)?;
                json!(CatalogPage { items, total, limit: i.page.limit, offset: i.page.offset, group: i.page.group })
            }
            (CAP_CATALOG_READ, "episodes") => {
                let i: IdInput = input(v, "EpisodesInput")?;
                json!(self.episodes(&i.id)?.ok_or_else(|| Error::not_found(format!("unknown item {}", i.id)))?)
            }
            (CAP_CATALOG_WRITE, "commit_scan") => {
                json!(self.commit_scan(&input::<CommitScanInput>(v, "CommitScanInput")?)?)
            }
            (CAP_CATALOG_WRITE, "set_missing") => {
                let i: SetMissingInput = input(v, "SetMissingInput")?;
                json!(self.set_missing(&i.id, i.missing)?)
            }
            (CAP_CATALOG_WRITE, "delete_library") => {
                json!(self.delete_library(&input::<DeleteLibraryInput>(v, "DeleteLibraryInput")?.library_id)?)
            }
            (CAP_CATALOG_READ | CAP_CATALOG_WRITE, op) => {
                return Err(Error::invalid(format!("capability {cap} has no op {op} in lain-rs yet")));
            }
            _ => return Err(Error::invalid(format!("unsupported cap {cap}"))),
        };
        Ok(out)
    }
}

/// The userstate provider.
pub struct Userstate {
    store: Arc<Store>,
}

#[derive(Deserialize)]
struct PutInput {
    #[serde(default)]
    user_id: String,
    #[serde(default)]
    progress: Progress,
}

#[derive(Deserialize)]
struct GetInput {
    #[serde(default)]
    user_id: String,
    #[serde(default)]
    item_id: String,
}

#[derive(Deserialize)]
struct ListInput {
    #[serde(default)]
    user_id: String,
}

impl Userstate {
    pub fn new(store: Arc<Store>) -> Self {
        Userstate { store }
    }

    /// Stores progress; the time is the server's.
    pub fn put(&self, user_id: &str, mut p: Progress) -> Result<Progress, Error> {
        p.user_id = user_id.to_string();
        p.updated_at = lain_core::now_unix();
        self.store
            .write(|tx| tx.put_json(Bucket::Progress, &join_key(&p.user_id, &p.item_id), &p))
            .map_err(store_err)?;
        Ok(p)
    }

    /// The stored progress, or the zero value.
    pub fn get(&self, user_id: &str, item_id: &str) -> Progress {
        self.store
            .read(|tx| tx.get_json::<Progress>(Bucket::Progress, &join_key(user_id, item_id)))
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    /// All of one user's progress, in key order.
    pub fn list(&self, user_id: &str) -> Vec<Progress> {
        self.store
            .read(|tx| tx.scan_prefix(Bucket::Progress, &key_prefix(user_id)))
            .map(|rows| rows.iter().filter_map(|(_, v)| serde_json::from_slice(v).ok()).collect())
            .unwrap_or_default()
    }
}

impl Provider for Userstate {
    fn id(&self) -> &str {
        USERSTATE_ID
    }

    fn capabilities(&self) -> Vec<String> {
        vec![CAP_USER_PROGRESS.into()]
    }

    fn health(&self) -> Result<(), Error> {
        Ok(())
    }

    fn invoke(&self, cap: &str, op: &str, v: Value) -> Result<Value, Error> {
        if cap != CAP_USER_PROGRESS {
            return Err(Error::invalid(format!("unsupported cap {cap}")));
        }
        match op {
            "put" => {
                let i: PutInput = input(v, "PutInput")?;
                Ok(json!(self.put(&i.user_id, i.progress)?))
            }
            "get" => {
                let i: GetInput = input(v, "GetInput")?;
                Ok(json!(self.get(&i.user_id, &i.item_id)))
            }
            "list" => {
                let i: ListInput = input(v, "ListInput")?;
                Ok(json!(self.list(&i.user_id)))
            }
            _ => Err(Error::invalid("PutInput, GetInput or ListInput required")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, lib: &str, title: &str, ep: i64, updated: i64) -> CatalogItem {
        CatalogItem {
            id: id.into(),
            library_id: lib.into(),
            kind: "episode".into(),
            title: title.into(),
            episode: ep,
            updated_at: updated,
            ..CatalogItem::default()
        }
    }

    #[test]
    fn query_sorts_pages_and_groups() {
        let dir = lain_store::test_dir("catalog");
        let store = Arc::new(Store::open(&dir).unwrap());
        let cat = Catalog::new(store.clone());
        let mut doc = ExportDoc { format: EXPORT_FORMAT.into(), version: EXPORT_VERSION, ..ExportDoc::default() };
        for it in [
            item("c", "l1", "Frieren", 2, 5),
            item("a", "l1", "Frieren", 1, 9),
            item("b", "l2", "frieren ", 3, 1),
            item("d", "l2", "Akira", 0, 7),
        ] {
            doc.items.insert(it.id.clone(), it);
        }
        assert_eq!(cat.import(&doc).unwrap(), 4);
        let ids = |v: Vec<CatalogItem>| v.into_iter().map(|i| i.id).collect::<Vec<_>>();
        let p = PageParams::normalize(0, 0, "");
        assert_eq!(ids(cat.page(&p).unwrap().items), vec!["d", "a", "c", "b"]);
        assert_eq!(ids(cat.page(&PageParams::normalize(0, 0, "recent")).unwrap().items), vec!["a", "d", "c", "b"]);
        let mut by_lib = PageParams::normalize(1, 1, "");
        by_lib.library_id = "l1".into();
        let page = cat.page(&by_lib).unwrap();
        assert_eq!((ids(page.items), page.total), (vec!["c".to_string()], 2));
        assert_eq!(ids(cat.episodes("c").unwrap().unwrap()), vec!["a", "c", "b"]);
        assert!(cat.episodes("zz").unwrap().is_none());
        assert_eq!(cat.invoke(CAP_CATALOG_READ, "get", json!({"id": "zz"})).unwrap_err().code, "not-found");
        assert_eq!(cat.delete_library("l2").unwrap(), 2);
        assert_eq!(cat.page(&p).unwrap().total, 2);
        assert!(cat.set_missing("a", true).unwrap());
        assert!(cat.get("a").unwrap().unwrap().missing);
        assert!(!cat.set_missing("zz", true).unwrap());
        assert!(cat.import(&ExportDoc { format: "x".into(), version: 3, ..ExportDoc::default() }).is_err());
        for v in [1, 2, 4] {
            assert!(
                cat.import(&ExportDoc { format: EXPORT_FORMAT.into(), version: v, ..ExportDoc::default() }).is_err()
            );
        }
        let mut hidden = PageParams::normalize(0, 0, "");
        hidden.exclude_libraries = vec!["l1".into()];
        assert_eq!(cat.page(&hidden).unwrap().total, 0, "excluded before counting");
        drop(cat);
        drop(store);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn titles_page_whole_titles() {
        let dir = lain_store::test_dir("catalog-titles");
        let store = Arc::new(Store::open(&dir).unwrap());
        let cat = Catalog::new(store.clone());
        let mut doc = ExportDoc { format: EXPORT_FORMAT.into(), version: EXPORT_VERSION, ..ExportDoc::default() };
        for it in [
            item("f2", "l1", "Frieren", 2, 5),
            item("f1", "l1", "Frieren", 1, 3),
            item("f3", "l2", "frieren ", 3, 1),
            item("ak", "l2", "Akira", 0, 4),
            item("m1", "l1", "Monster", 1, 2),
            item("m2", "l1", "Monster", 2, 9),
        ] {
            doc.items.insert(it.id.clone(), it);
        }
        cat.import(&doc).unwrap();
        let ids = |v: &[CatalogItem]| v.iter().map(|i| i.id.as_str()).collect::<Vec<_>>().join(" ");
        let titled = |limit: i64, offset: i64, sort: &str| {
            let mut p = PageParams::normalize(limit, offset, sort);
            p.group = GROUP_TITLE.into();
            p
        };
        // By title: each title where its first file stands, its files in watch order.
        let page = cat.page(&titled(0, 0, "")).unwrap();
        assert_eq!((ids(&page.items).as_str(), page.total, page.group.as_str()), ("ak f1 f2 f3 m1 m2", 3, "title"));
        // A page of one title holds all of it; the total still counts titles.
        let page = cat.page(&titled(1, 1, "")).unwrap();
        assert_eq!((ids(&page.items).as_str(), page.total), ("f1 f2 f3", 3));
        assert!(cat.page(&titled(5, 3, "")).unwrap().items.is_empty());
        // Newest first: a title by its newest file.
        assert_eq!(ids(&cat.page(&titled(0, 0, "recent")).unwrap().items), "m1 m2 f1 f2 f3 ak");
        // A library's part of a title is that library's title.
        let mut lib = titled(0, 0, "");
        lib.library_id = "l2".into();
        assert_eq!(ids(&cat.page(&lib).unwrap().items), "ak f3");
        // Search: the matching titles, whole, and how many there are.
        let (items, total) = cat.query("fri", "", &titled(1, 0, "")).unwrap();
        assert_eq!((ids(&items).as_str(), total), ("f1 f2 f3", 1));
        let out = cat.invoke(CAP_CATALOG_READ, "query", json!({"q": "o", "page": titled(1, 0, "")})).unwrap();
        assert_eq!((out["total"].as_i64(), out["group"].as_str()), (Some(1), Some("title")));
        // Without the group, files as before and no group in the envelope.
        let out = cat.invoke(CAP_CATALOG_READ, "page", json!({"page": PageParams::normalize(0, 0, "")})).unwrap();
        assert_eq!((out["total"].as_i64(), out.get("group")), (Some(6), None));
        drop(cat);
        drop(store);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn rooted_items_resolve_per_machine_and_export_v3() {
        let dir = lain_store::test_dir("catalog-roots");
        let store = Arc::new(Store::open(&dir).unwrap());
        let roots = Arc::new(RwLock::new(
            Roots::parse(br#"{"format":"lain.roots@1","roots":{"media":"/mnt/v/media"}}"#).unwrap(),
        ));
        let cat = Catalog::with_roots(store.clone(), roots.clone());
        let mut it = item("a", "l1", "Akira", 0, 1);
        it.file_path = r"V:\media\movies\Akira.mkv".into();
        it.root = "media".into();
        it.rel_path = "movies/Akira.mkv".into();
        let mut doc = ExportDoc { format: EXPORT_FORMAT.into(), version: 3, ..ExportDoc::default() };
        doc.items.insert("a".into(), it);
        cat.import(&doc).unwrap();
        let stored: CatalogItem = store.read(|tx| tx.get_json(Bucket::Items, b"a")).unwrap().unwrap();
        assert_eq!((stored.file_path.as_str(), stored.rel_path.as_str()), ("", "movies/Akira.mkv"));
        assert_eq!(cat.get("a").unwrap().unwrap().file_path, "/mnt/v/media/movies/Akira.mkv");
        // Another machine: only the mapping changes.
        *roots.write().unwrap() = Roots::parse(br#"{"format":"lain.roots@1","roots":{"media":"D:\\Media"}}"#).unwrap();
        assert_eq!(cat.get("a").unwrap().unwrap().file_path, r"D:\Media\movies\Akira.mkv");
        let out = cat.export().unwrap();
        assert_eq!((out.version, out.roots.clone()), (3, vec!["media".to_string()]));
        assert_eq!(out.items["a"].rel_path, "movies/Akira.mkv");
        drop(cat);
        drop(store);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn userstate_is_per_user() {
        let dir = lain_store::test_dir("userstate");
        let store = Arc::new(Store::open(&dir).unwrap());
        let us = Userstate::new(store.clone());
        let p = us
            .put(
                "u1",
                Progress { item_id: "i1".into(), position_sec: 3.5, user_id: "spoof".into(), ..Progress::default() },
            )
            .unwrap();
        assert_eq!((p.user_id.as_str(), p.position_sec), ("u1", 3.5));
        us.put("u10", Progress { item_id: "i1".into(), ..Progress::default() }).unwrap();
        assert_eq!(us.get("u2", "i1"), Progress::default());
        assert_eq!(us.list("u1").len(), 1);
        drop(us);
        drop(store);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn title_keys() {
        assert_eq!(title_key("  Frieren:   Beyond "), "frieren: beyond");
        assert_eq!(title_key("İstanbul"), "istanbul");
    }
}
