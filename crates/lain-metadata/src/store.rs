//! The overlay table (one `Enrichment` per item id; identity stays in
//! the catalog) and the TTL cache in front of providers (repeat
//! enrichments and rescans do not touch the network).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use lain_api::Enrichment;
use lain_store::{Bucket, Store, StoreError};

use crate::types::{MetadataCandidate, MetadataRecord};

/// Search answers go stale fast (new seasons); records are stable.
pub const SEARCH_TTL_SECS: i64 = 7 * 24 * 3600;
pub const RECORD_TTL_SECS: i64 = 30 * 24 * 3600;

pub fn save(store: &Store, item_id: &str, rec: &MetadataRecord, now: i64) -> Result<Enrichment, StoreError> {
    let e = Enrichment {
        item_id: item_id.into(),
        provider: rec.provider.clone(),
        remote_id: rec.remote_id.clone(),
        title: rec.title.clone(),
        year: rec.year,
        genres: rec.genres.clone(),
        synopsis: rec.synopsis.clone(),
        poster: rec.poster.clone(),
        cover: rec.cover.clone(),
        fetched_at: now,
    };
    store.write(|tx| tx.put_json(Bucket::Enrichments, item_id.as_bytes(), &e))?;
    Ok(e)
}

/// The overlay, if any (a corrupt record reads as absent).
pub fn get(store: &Store, item_id: &str) -> Option<Enrichment> {
    store
        .read(|tx| tx.get(Bucket::Enrichments, item_id.as_bytes()))
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_slice(&raw).ok())
}

/// The overlays that exist for `ids`, in `ids` order, in one read.
pub fn get_many(store: &Store, ids: &[String]) -> Vec<Enrichment> {
    store
        .read(|tx| {
            let mut out = Vec::new();
            for id in ids {
                if let Some(raw) = tx.get(Bucket::Enrichments, id.as_bytes())?
                    && let Ok(e) = serde_json::from_slice(&raw)
                {
                    out.push(e);
                }
            }
            Ok(out)
        })
        .unwrap_or_default()
}

pub fn delete(store: &Store, item_id: &str) -> Result<(), StoreError> {
    store.write(|tx| tx.delete(Bucket::Enrichments, item_id.as_bytes())).map(|_| ())
}

/// Removes every overlay of one provider (uninstall leaves nothing).
pub fn delete_provider(store: &Store, provider: &str) -> Result<i64, StoreError> {
    store.write(|tx| {
        let ids: Vec<Vec<u8>> = tx
            .all(Bucket::Enrichments)?
            .into_iter()
            .filter(|(_, v)| serde_json::from_slice::<Enrichment>(v).is_ok_and(|e| e.provider == provider))
            .map(|(k, _)| k)
            .collect();
        for k in &ids {
            tx.delete(Bucket::Enrichments, k)?;
        }
        Ok(ids.len() as i64)
    })
}

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    at: i64,
    data: Value,
}

fn search_key(kind: &str, query: &str) -> Vec<u8> {
    format!("q\0{}\0{}", kind.to_lowercase(), query.trim().to_lowercase()).into_bytes()
}

fn record_key(provider: &str, id: &str) -> Vec<u8> {
    format!("r\0{provider}\0{id}").into_bytes()
}

fn cached(store: &Store, key: &[u8], ttl: i64, now: i64) -> Option<Value> {
    let raw = store.read(|tx| tx.get(Bucket::MetadataCache, key)).ok().flatten()?;
    let e: CacheEntry = serde_json::from_slice(&raw).ok()?;
    (now - e.at <= ttl).then_some(e.data)
}

fn put(store: &Store, key: &[u8], data: Value, now: i64) -> Result<(), StoreError> {
    store.write(|tx| tx.put_json(Bucket::MetadataCache, key, &CacheEntry { at: now, data }))
}

/// Fresh cached candidates; an empty answer is a miss, so an outage
/// never poisons later enrichments.
pub fn cached_search(store: &Store, kind: &str, query: &str, now: i64) -> Option<Vec<MetadataCandidate>> {
    let v = cached(store, &search_key(kind, query), SEARCH_TTL_SECS, now)?;
    serde_json::from_value::<Vec<MetadataCandidate>>(v).ok().filter(|l| !l.is_empty())
}

pub fn put_search(
    store: &Store,
    kind: &str,
    query: &str,
    out: &[MetadataCandidate],
    now: i64,
) -> Result<(), StoreError> {
    put(store, &search_key(kind, query), serde_json::to_value(out).unwrap_or(Value::Null), now)
}

pub fn cached_record(store: &Store, provider: &str, id: &str, now: i64) -> Option<MetadataRecord> {
    serde_json::from_value(cached(store, &record_key(provider, id), RECORD_TTL_SECS, now)?).ok()
}

pub fn put_record(store: &Store, rec: &MetadataRecord, now: i64) -> Result<(), StoreError> {
    put(store, &record_key(&rec.provider, &rec.remote_id), serde_json::to_value(rec).unwrap_or(Value::Null), now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlays_and_cache() {
        let dir = lain_store::test_dir("metadata-store");
        let store = Store::open(&dir).unwrap();
        let rec =
            MetadataRecord { provider: "p".into(), remote_id: "1".into(), title: "T".into(), ..Default::default() };
        assert_eq!(save(&store, "i1", &rec, 5).unwrap().fetched_at, 5);
        save(&store, "i2", &MetadataRecord { provider: "q".into(), ..rec.clone() }, 5).unwrap();
        store.write(|tx| tx.put(Bucket::Enrichments, b"bad", b"{")).unwrap();
        let ids = vec!["i2".to_string(), "nope".into(), "bad".into(), "i1".into()];
        let got: Vec<_> = get_many(&store, &ids).into_iter().map(|e| e.item_id).collect();
        assert_eq!(got, vec!["i2", "i1"]);
        assert_eq!(delete_provider(&store, "p").unwrap(), 1);
        assert!(get(&store, "i1").is_none());
        delete(&store, "i2").unwrap();
        assert!(get(&store, "i2").is_none());

        let c = vec![MetadataCandidate { title: "T".into(), ..Default::default() }];
        put_search(&store, "Anime", " Frieren ", &c, 100).unwrap();
        assert_eq!(cached_search(&store, "anime", "frieren", 100 + SEARCH_TTL_SECS).unwrap(), c);
        assert!(cached_search(&store, "anime", "frieren", 101 + SEARCH_TTL_SECS).is_none());
        put_search(&store, "anime", "empty", &[], 100).unwrap();
        assert!(cached_search(&store, "anime", "empty", 100).is_none());
        put_record(&store, &rec, 100).unwrap();
        assert_eq!(cached_record(&store, "p", "1", 100 + RECORD_TTL_SECS).unwrap(), rec);
        assert!(cached_record(&store, "p", "1", 101 + RECORD_TTL_SECS).is_none());
        drop(store);
        std::fs::remove_dir_all(dir).ok();
    }
}
