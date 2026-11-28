//! `lain-metadata-enrich` (`lain.metadata.enrich@1`, D-076): the
//! overlay pipeline end to end. Ops: `enrich` (one item, optionally
//! one named provider), `get`, `batch`, `delete`, `backfill` (every
//! catalog item without an overlay, one at a time so provider APIs see
//! a trickle, never a burst; an item no network provider knew is left
//! out until a backfill asks with `retry`, D-302).

use std::sync::{Arc, Weak};

use serde::Deserialize;
use serde_json::{Value, json};

use lain_api::{CatalogItem, Enrichment, Library};
use lain_core::{Error, Provider, Registry};
use lain_store::{Bucket, Store};

use crate::LOCAL_PROVIDERS;
use crate::merge::merge_candidates;
use crate::nfo::{CAP_METADATA_RESOLVE, CAP_METADATA_SEARCH};
use crate::store;
use crate::types::{MetadataCandidate, MetadataRecord, MetadataResolveInput, MetadataSearchInput};

pub const ENRICH_ID: &str = "lain-metadata-enrich";
pub const CAP_METADATA_ENRICH: &str = "lain.metadata.enrich@1";
const CAP_CATALOG_READ: &str = "lain.catalog.read@1";

/// Called by `backfill` with (`done`, `total`) before the first item
/// and after each one (in-process observers only).
pub type BackfillProgress = Arc<dyn Fn(i64, i64) + Send + Sync>;

pub struct Enricher {
    registry: Weak<Registry>,
    store: Arc<Store>,
    progress: Option<BackfillProgress>,
}

#[derive(Deserialize)]
struct EnrichInput {
    #[serde(default)]
    item: CatalogItem,
    #[serde(default)]
    only: String,
}

#[derive(Deserialize, Default)]
struct BackfillInput {
    #[serde(default)]
    retry: bool,
}

/// What one backfill did: `checked` items without an overlay were
/// asked about, `enriched` got one, `skipped` were left out as known
/// misses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct Backfill {
    pub checked: i64,
    pub enriched: i64,
    pub skipped: i64,
}

/// Why one item got no overlay.
enum Failed {
    /// Every provider asked answered and none knew the title.
    Unknown(Error),
    /// Anything else: a provider was down, a store failed.
    Other(Error),
}

impl Failed {
    fn into_error(self) -> Error {
        match self {
            Failed::Unknown(e) | Failed::Other(e) => e,
        }
    }
}

#[derive(Deserialize)]
struct ItemInput {
    #[serde(default)]
    item_id: String,
}

#[derive(Deserialize)]
struct BatchInput {
    #[serde(default)]
    ids: Vec<String>,
}

fn store_err(e: lain_store::StoreError) -> Error {
    Error::internal(e.to_string())
}

fn dir_of(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    std::path::Path::new(path).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| ".".into())
}

impl Enricher {
    pub fn new(registry: &Arc<Registry>, store: Arc<Store>) -> Self {
        Enricher { registry: Arc::downgrade(registry), store, progress: None }
    }

    pub fn with_progress(mut self, progress: BackfillProgress) -> Self {
        self.progress = Some(progress);
        self
    }

    fn reg(&self) -> Result<Arc<Registry>, Error> {
        self.registry.upgrade().ok_or_else(|| Error::unavailable("enricher missing registry or store"))
    }

    /// Searches, picks, resolves and saves one overlay.
    pub fn enrich(&self, item: &CatalogItem, only: &str) -> Result<Enrichment, Error> {
        self.enrich_item(item, only, self.local_for(item)).map_err(Failed::into_error)
    }

    fn enrich_item(&self, item: &CatalogItem, only: &str, local: bool) -> Result<Enrichment, Failed> {
        let kind = if item.kind.is_empty() || item.kind == "episode" || item.kind == "video" {
            "anime"
        } else {
            item.kind.as_str()
        };
        let (merged, complete) = self
            .search(&item.title, kind, &dir_of(&item.file_path), &item.file_path, only, local)
            .inspect_err(|e| {
                tracing::warn!(item = %item.id, title = %item.title, err = %e, "enrich search failed");
            })
            .map_err(Failed::Other)?;
        let Some(best) = merged.first() else {
            // Debug: a library of home videos says this once per file.
            tracing::debug!(item = %item.id, title = %item.title, kind, local, "enrich no match");
            let e = Error::not_found("no metadata found");
            return Err(if complete { Failed::Unknown(e) } else { Failed::Other(e) });
        };
        let rec = self
            .resolve(&best.provider, &best.remote_id)
            .inspect_err(|e| {
                tracing::warn!(item = %item.id, provider = %best.provider, remote_id = %best.remote_id, err = %e, "enrich resolve failed");
            })
            .map_err(Failed::Other)?;
        let saved =
            store::save(&self.store, &item.id, &rec, lain_core::now_unix()).map_err(|e| Failed::Other(store_err(e)))?;
        let _ = store::clear_miss(&self.store, &item.id);
        tracing::debug!(item = %item.id, title = %saved.title, provider = %saved.provider, "enrich ok");
        Ok(saved)
    }

    /// Cache, else the merge-many fan-out (cached when not restricted to
    /// one provider; an empty answer only when no provider failed). A
    /// failing provider degrades the merge; only total failure is an
    /// error.
    /// A private library (D-156) says nothing about its titles to the
    /// outside, and a library with "Fetch metadata online" off (D-302)
    /// holds titles no provider knows: their items are enriched from the
    /// files next to them (`.nfo`) and never by a network provider.
    fn local_for(&self, item: &CatalogItem) -> bool {
        crate::local_kind(&item.kind) || self.local_only(&item.library_id)
    }

    fn local_only(&self, library_id: &str) -> bool {
        if library_id.is_empty() {
            return false;
        }
        let lib: Option<Library> =
            self.store.read(|tx| tx.get_json(Bucket::Libraries, library_id.as_bytes())).ok().flatten();
        lib.is_some_and(|l| l.private || l.local_metadata)
    }

    /// The local providers only, each asked on its own: the merge-many
    /// call would reach the network ones too, and the answer is about
    /// one folder, so it is not cached under the title either.
    fn search_local(&self, input: &MetadataSearchInput, only: &str) -> Result<(Vec<MetadataCandidate>, bool), Error> {
        let reg = self.reg()?;
        let mut complete = true;
        let lists: Vec<Vec<MetadataCandidate>> = LOCAL_PROVIDERS
            .iter()
            .filter(|id| only.is_empty() || **id == only)
            .filter_map(|id| {
                reg.invoke_provider(CAP_METADATA_SEARCH, id, "search", json!(input))
                    .inspect_err(|_| complete = false)
                    .ok()
            })
            .map(|v| serde_json::from_value::<Vec<MetadataCandidate>>(v).unwrap_or_default())
            .collect();
        Ok((merge_candidates(&input.query, &lists, 10), complete))
    }

    /// The candidates, and whether every provider asked answered (an
    /// empty answer is then a real "nobody knows it").
    fn search(
        &self,
        query: &str,
        kind: &str,
        dir: &str,
        path: &str,
        only: &str,
        local: bool,
    ) -> Result<(Vec<MetadataCandidate>, bool), Error> {
        if local {
            let input = MetadataSearchInput {
                query: query.into(),
                kind: kind.into(),
                limit: 5,
                dir: dir.into(),
                path: path.into(),
            };
            return self.search_local(&input, only);
        }
        let now = lain_core::now_unix();
        if only.is_empty()
            && let Some(hit) = store::cached_search(&self.store, kind, query, now)
        {
            return Ok((hit, true));
        }
        let input = MetadataSearchInput {
            query: query.into(),
            kind: kind.into(),
            limit: 5,
            dir: dir.into(),
            path: String::new(),
        };
        let (outputs, skipped) = self.reg()?.call_merge_report(CAP_METADATA_SEARCH, "search", json!(input))?;
        let all_answered = skipped.is_empty();
        if !skipped.is_empty() {
            let causes: Vec<String> = skipped.iter().map(|f| format!("{}: {}", f.provider, f.error)).collect();
            tracing::debug!(query, kind, causes = %causes.join("; "), "metadata providers skipped");
        }
        let lists: Vec<(String, Vec<MetadataCandidate>)> = outputs
            .into_iter()
            .map(|(id, v)| (id, serde_json::from_value::<Vec<MetadataCandidate>>(v).unwrap_or_default()))
            .collect();
        let merged = if only.is_empty() {
            merge_candidates(query, &lists.into_iter().map(|(_, l)| l).collect::<Vec<_>>(), 10)
        } else {
            match lists.into_iter().find(|(id, _)| id == only) {
                Some((_, l)) => merge_candidates(query, &[l], 10),
                None => Vec::new(),
            }
        };
        // "Nothing found" is cached only when every asked provider
        // answered: a skipped one may know the title.
        if only.is_empty() && (!merged.is_empty() || all_answered) {
            let _ = store::put_search(&self.store, kind, query, &merged, now);
        }
        Ok((merged, all_answered))
    }

    /// Cache, else the winning provider itself (through authority).
    fn resolve(&self, provider: &str, remote_id: &str) -> Result<MetadataRecord, Error> {
        let now = lain_core::now_unix();
        if let Some(rec) = store::cached_record(&self.store, provider, remote_id, now) {
            return Ok(rec);
        }
        let input = MetadataResolveInput { provider: provider.into(), remote_id: remote_id.into() };
        let out = self.reg()?.invoke_provider(CAP_METADATA_RESOLVE, provider, "resolve", json!(input))?;
        let rec: MetadataRecord =
            serde_json::from_value(out).map_err(|_| Error::internal(format!("bad record shape from {provider}")))?;
        let _ = store::put_record(&self.store, &rec, now);
        Ok(rec)
    }

    /// Enriches every catalog item without an overlay; per-item failures
    /// are skipped, never failing the pass. An item every network
    /// provider answered "unknown" for is remembered and left out of
    /// later passes (D-302) until one asks with `retry`; items of a
    /// local-only library are always asked again (it costs a file read).
    pub fn backfill(&self, retry: bool) -> Result<Backfill, Error> {
        let (out, _) = self.reg()?.call_one(CAP_CATALOG_READ, "list", Value::Null)?;
        let items: Vec<CatalogItem> =
            serde_json::from_value(out).map_err(|_| Error::internal("catalog read returned a bad shape"))?;
        if retry {
            let _ = store::clear_misses(&self.store);
        }
        let mut local_of = std::collections::HashMap::<String, bool>::new();
        let mut result = Backfill::default();
        let mut pending: Vec<(CatalogItem, bool)> = Vec::new();
        for it in items {
            if store::get(&self.store, &it.id).is_some() {
                continue;
            }
            let local = crate::local_kind(&it.kind)
                || *local_of.entry(it.library_id.clone()).or_insert_with(|| self.local_only(&it.library_id));
            if !local && store::is_miss(&self.store, &it.id) {
                result.skipped += 1;
                continue;
            }
            pending.push((it, local));
        }
        let total = pending.len() as i64;
        let report = |n: i64| {
            if let Some(p) = &self.progress {
                p(n, total);
            }
        };
        report(0);
        let now = lain_core::now_unix();
        for (n, (it, local)) in pending.iter().enumerate() {
            match self.enrich_item(it, "", *local) {
                Ok(_) => result.enriched += 1,
                Err(Failed::Unknown(_)) if !*local => {
                    let _ = store::put_miss(&self.store, &it.id, now);
                }
                Err(_) => {}
            }
            result.checked += 1;
            report(n as i64 + 1);
        }
        tracing::info!(
            checked = result.checked,
            enriched = result.enriched,
            skipped = result.skipped,
            retry,
            "metadata backfill done"
        );
        Ok(result)
    }
}

impl Provider for Enricher {
    fn id(&self) -> &str {
        ENRICH_ID
    }

    fn capabilities(&self) -> Vec<String> {
        vec![CAP_METADATA_ENRICH.into()]
    }

    fn health(&self) -> Result<(), Error> {
        self.reg().map(|_| ())
    }

    fn invoke(&self, cap: &str, op: &str, input: Value) -> Result<Value, Error> {
        if cap != CAP_METADATA_ENRICH {
            return Err(Error::invalid(format!("unsupported cap {cap}")));
        }
        let bad = |e: serde_json::Error| Error::invalid(format!("enrich input required: {e}"));
        match op {
            "enrich" => {
                let i: EnrichInput = serde_json::from_value(input).map_err(bad)?;
                Ok(json!(self.enrich(&i.item, &i.only)?))
            }
            "get" => {
                let i: ItemInput = serde_json::from_value(input).map_err(bad)?;
                store::get(&self.store, &i.item_id).map(|e| json!(e)).ok_or_else(|| Error::not_found("not enriched"))
            }
            "batch" => {
                let i: BatchInput = serde_json::from_value(input).map_err(bad)?;
                Ok(json!({"items": store::get_many(&self.store, &i.ids)}))
            }
            "delete" => {
                let i: ItemInput = serde_json::from_value(input).map_err(bad)?;
                store::delete(&self.store, &i.item_id).map_err(store_err)?;
                Ok(json!(true))
            }
            "backfill" => {
                let i: BackfillInput = if input.is_null() {
                    BackfillInput::default()
                } else {
                    serde_json::from_value(input).map_err(bad)?
                };
                Ok(json!(self.backfill(i.retry)?))
            }
            _ => Err(Error::invalid(format!("capability {cap} has no op {op}"))),
        }
    }
}
