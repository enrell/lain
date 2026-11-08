//! Configured media roots (`libraries` table). The id is stable over
//! the path so re-creating a root never orphans its catalog entries.

use std::sync::Arc;

use lain_store::{Bucket, Store, StoreError};

use crate::contracts::Library;

/// The source provider every library is bound to today.
pub const SOURCE_FILESYSTEM: &str = "lain-source-filesystem";

/// Go's `shortID`: a 31-multiplier rolling hash over the bytes (int64,
/// wrapping), made non-negative, reduced mod 0xffffffff, printed `%08x`.
pub fn short_id(s: &str) -> String {
    let mut h: i64 = 0;
    for b in s.bytes() {
        h = h.wrapping_mul(31).wrapping_add(b as i64);
    }
    if h < 0 {
        h = h.wrapping_neg();
    }
    let r = h % 0xffff_ffff;
    if r < 0 {
        // Only for i64::MIN, whose negation stays negative: Go pads the
        // sign into the width.
        format!("-{:07x}", r.unsigned_abs())
    } else {
        format!("{r:08x}")
    }
}

pub struct Libraries {
    store: Arc<Store>,
}

impl Libraries {
    pub fn new(store: Arc<Store>) -> Self {
        Libraries { store }
    }

    /// All libraries by name.
    pub fn list(&self) -> Result<Vec<Library>, StoreError> {
        let rows = self.store.read(|tx| tx.all(Bucket::Libraries))?;
        let mut out: Vec<Library> = rows.iter().filter_map(|(_, v)| serde_json::from_slice(v).ok()).collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Validates and stores a library (overwriting one with the same
    /// path). The message texts are Go's.
    pub fn create(&self, name: &str, kind: &str, path: &str) -> Result<Library, String> {
        if name.is_empty() || path.is_empty() {
            return Err("name and path required".into());
        }
        if !std::fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false) {
            return Err("path is not a readable directory".into());
        }
        let lib = Library {
            id: format!("lib-{}", short_id(path)),
            name: name.into(),
            kind: if kind.is_empty() { "anime".into() } else { kind.into() },
            path: path.into(),
            source: SOURCE_FILESYSTEM.into(),
            created_at: lain_core::now_unix(),
            ..Library::default()
        };
        self.store.write(|tx| tx.put_json(Bucket::Libraries, lib.id.as_bytes(), &lib)).map_err(|e| e.to_string())?;
        Ok(lib)
    }

    /// Removes a library record; the caller removes its catalog items.
    pub fn delete(&self, id: &str) -> Result<(), String> {
        let existed = self.store.write(|tx| tx.delete(Bucket::Libraries, id.as_bytes())).map_err(|e| e.to_string())?;
        if existed { Ok(()) } else { Err("unknown library".into()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_id_matches_go() {
        // Recorded from the Go server (lain-go, 2026-11-08).
        assert_eq!(
            short_id("/tmp/claude-1000/-home-lain/226a7066-d814-4509-84ee-c770bd5c8927/scratchpad/media"),
            "8bcbce82"
        );
        assert_eq!(
            short_id("/tmp/claude-1000/-home-lain/226a7066-d814-4509-84ee-c770bd5c8927/scratchpad/media/Show"),
            "83593479"
        );
        assert_eq!(short_id(""), "00000000");
    }
}
