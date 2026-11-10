//! The offline snapshot: the last catalog and the art this device saw,
//! per server, so Home and the Library still draw without the server.
//! Plain files under `<dir>/cache/<server hash>/`; a write failure is
//! logged and never fails the call that fetched the data.

use std::path::{Path, PathBuf};

use lain_api::CatalogItem;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct CatalogFile {
    format: String,
    fetched_at_ms: i64,
    items: Vec<CatalogItem>,
}

const FORMAT: &str = "lain.android.catalog-cache@1";

/// FNV-1a: stable across runs and builds (file names only).
pub(crate) fn hash(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

pub(crate) struct Cache {
    dir: PathBuf,
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

impl Cache {
    pub(crate) fn new(root: &Path, server: &str) -> Cache {
        Cache { dir: root.join("cache").join(hash(server)) }
    }

    fn write(&self, rel: &str, bytes: &[u8]) {
        let path = self.dir.join(rel);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = path.with_extension("tmp");
        let res = std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, &path));
        if let Err(e) = res {
            eprintln!("lain-ffi: cache write {}: {e}", path.display());
        }
    }

    pub(crate) fn save_catalog(&self, items: &[CatalogItem], at_ms: i64) {
        let f = CatalogFile { format: FORMAT.into(), fetched_at_ms: at_ms, items: items.to_vec() };
        match serde_json::to_vec(&f) {
            Ok(b) => self.write("catalog.json", &b),
            Err(e) => eprintln!("lain-ffi: catalog cache: {e}"),
        }
    }

    /// The cached catalog and when it was fetched.
    pub(crate) fn catalog(&self) -> Option<(Vec<CatalogItem>, i64)> {
        let raw = std::fs::read(self.dir.join("catalog.json")).ok()?;
        let f: CatalogFile = serde_json::from_slice(&raw).ok()?;
        (f.format == FORMAT).then_some((f.items, f.fetched_at_ms))
    }

    pub(crate) fn save_art(&self, item_id: &str, bytes: &[u8]) {
        self.write(&format!("art/{}", hash(item_id)), bytes);
    }

    pub(crate) fn art(&self, item_id: &str) -> Option<Vec<u8>> {
        std::fs::read(self.dir.join("art").join(hash(item_id))).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_round_trips_and_ignores_garbage() {
        let root = std::env::temp_dir().join(format!("lain-ffi-cache-{}", std::process::id()));
        let c = Cache::new(&root, "http://a:9360");
        assert!(c.catalog().is_none());
        let item = CatalogItem { id: "i1".into(), title: "Frieren".into(), ..Default::default() };
        c.save_catalog(std::slice::from_ref(&item), 42);
        let (items, at) = c.catalog().unwrap();
        assert_eq!((items, at), (vec![item], 42));
        // Another server has its own snapshot.
        assert!(Cache::new(&root, "http://b:9360").catalog().is_none());
        std::fs::write(c.dir.join("catalog.json"), b"{nope").unwrap();
        assert!(c.catalog().is_none());
        c.save_art("i1", b"png");
        assert_eq!(c.art("i1").unwrap(), b"png");
        assert!(c.art("i2").is_none());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn hash_is_stable() {
        assert_eq!(hash(""), "cbf29ce484222325");
        assert_ne!(hash("a"), hash("b"));
    }
}
