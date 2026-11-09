//! `lain-metadata-enrich` (`lain.metadata.enrich@1`, D-076): the
//! overlay pipeline end to end. Ops: `enrich` (one item, optionally
//! one named provider), `get`, `batch`, `delete`, `backfill` (every
//! catalog item without an overlay, one at a time so provider APIs see
//! a trickle, never a burst).

use std::sync::{Arc, Weak};

use serde::Deserialize;
use serde_json::{Value, json};

use lain_api::{CatalogItem, Enrichment};
use lain_core::{Error, Provider, Registry};
use lain_store::Store;

use crate::merge::merge_candidates;
use crate::nfo::{CAP_METADATA_RESOLVE, CAP_METADATA_SEARCH};
use crate::store;
use crate::types::{MetadataCandidate, MetadataRecord, MetadataResolveInput, MetadataSearchInput};

pub const ENRICH_ID: &str = "lain-metadata-enrich";
pub const CAP_METADATA_ENRICH: &str = "lain.metadata.enrich@1";
const CAP_CATALOG_READ: &str = "lain.catalog.read@1";

pub struct Enricher {
    registry: Weak<Registry>,
    store: Arc<Store>,
}

#[derive(Deserialize)]
struct EnrichInput {
    #[serde(default)]
    item: CatalogItem,
    #[serde(default)]
    only: String,
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
        Enricher { registry: Arc::downgrade(registry), store }
    }

    fn reg(&self) -> Result<Arc<Registry>, Error> {
        self.registry.upgrade().ok_or_else(|| Error::unavailable("enricher missing registry or store"))
    }

    /// Searches, picks, resolves and saves one overlay.
    pub fn enrich(&self, item: &CatalogItem, only: &str) -> Result<Enrichment, Error> {
        let kind = if item.kind.is_empty() || item.kind == "episode" || item.kind == "video" {
            "anime"
        } else {
            item.kind.as_str()
        };
        let merged = self.search(&item.title, kind, &dir_of(&item.file_path), only).inspect_err(|e| {
            tracing::warn!(item = %item.id, title = %item.title, err = %e, "enrich search failed");
        })?;
        let Some(best) = merged.first() else {
            tracing::warn!(item = %item.id, title = %item.title, kind, "enrich no match");
            return Err(Error::not_found("no metadata found"));
        };
        let rec = self.resolve(&best.provider, &best.remote_id).inspect_err(|e| {
            tracing::warn!(item = %item.id, provider = %best.provider, remote_id = %best.remote_id, err = %e, "enrich resolve failed");
        })?;
        let saved = store::save(&self.store, &item.id, &rec, lain_core::now_unix()).map_err(store_err)?;
        tracing::debug!(item = %item.id, title = %saved.title, provider = %saved.provider, "enrich ok");
        Ok(saved)
    }

    /// Cache, else the merge-many fan-out (cached when not empty and not
    /// restricted to one provider). A failing provider degrades the
    /// merge; only total failure is an error.
    fn search(&self, query: &str, kind: &str, dir: &str, only: &str) -> Result<Vec<MetadataCandidate>, Error> {
        let now = lain_core::now_unix();
        if only.is_empty()
            && let Some(hit) = store::cached_search(&self.store, kind, query, now)
        {
            return Ok(hit);
        }
        let input = MetadataSearchInput { query: query.into(), kind: kind.into(), limit: 5, dir: dir.into() };
        let (outputs, skipped) = self.reg()?.call_merge_report(CAP_METADATA_SEARCH, "search", json!(input))?;
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
        if only.is_empty() && !merged.is_empty() {
            let _ = store::put_search(&self.store, kind, query, &merged, now);
        }
        Ok(merged)
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
    /// are skipped, never failing the scan that just succeeded.
    pub fn backfill(&self) -> Result<i64, Error> {
        let (out, _) = self.reg()?.call_one(CAP_CATALOG_READ, "list", Value::Null)?;
        let items: Vec<CatalogItem> =
            serde_json::from_value(out).map_err(|_| Error::internal("catalog read returned a bad shape"))?;
        let mut done = 0;
        for it in items {
            if store::get(&self.store, &it.id).is_some() {
                continue;
            }
            if self.enrich(&it, "").is_ok() {
                done += 1;
            }
        }
        Ok(done)
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
            "backfill" => Ok(json!({"enriched": self.backfill()?})),
            _ => Err(Error::invalid(format!("capability {cap} has no op {op}"))),
        }
    }
}
