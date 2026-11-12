//! Configured media roots (`libraries` table). The id is stable over
//! the path so re-creating a root never orphans its catalog entries.

use std::sync::{Arc, RwLock};

use lain_store::{Bucket, Store, StoreError};

use crate::contracts::Library;
use crate::roots::{Roots, valid_rel};

/// The source provider every library is bound to today.
pub const SOURCE_FILESYSTEM: &str = "lain-source-filesystem";

/// A stable short id: the low 32 bits of a 31-multiplier rolling hash
/// over the bytes, as 8 hex digits.
pub fn short_id(s: &str) -> String {
    let h = s.bytes().fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64));
    format!("{:08x}", h as u32)
}

pub struct Libraries {
    store: Arc<Store>,
    roots: Arc<RwLock<Roots>>,
}

/// An admin change to a library (`PATCH /api/libraries/{id}`).
pub use lain_api::LibraryPatch;

impl Libraries {
    pub fn new(store: Arc<Store>, roots: Arc<RwLock<Roots>>) -> Self {
        Libraries { store, roots }
    }

    fn resolve(&self, mut lib: Library) -> Library {
        if !lib.root.is_empty()
            && let Some(p) = self.roots.read().unwrap_or_else(|e| e.into_inner()).resolve(&lib.root, &lib.root_path)
        {
            lib.path = p;
        }
        lib
    }

    /// All libraries in the admin's order, then the never-ordered ones by
    /// name; paths resolved for this machine.
    pub fn list(&self) -> Result<Vec<Library>, StoreError> {
        let rows = self.store.read(|tx| tx.all(Bucket::Libraries))?;
        let mut out: Vec<Library> =
            rows.iter().filter_map(|(_, v)| serde_json::from_slice(v).ok()).map(|l| self.resolve(l)).collect();
        out.sort_by(|a, b| (a.position == 0, a.position, &a.name).cmp(&(b.position == 0, b.position, &b.name)));
        Ok(out)
    }

    /// Orders the libraries: `ids` is every library, first to last.
    /// Answers the list in its new order.
    pub fn reorder(&self, ids: &[String]) -> Result<Vec<Library>, String> {
        let mut seen = std::collections::BTreeSet::new();
        if let Some(dup) = ids.iter().find(|id| !seen.insert(id.as_str())) {
            return Err(format!("{dup} is listed twice"));
        }
        self.store
            .write(|tx| {
                let mut libs: Vec<Library> =
                    tx.all(Bucket::Libraries)?.iter().filter_map(|(_, v)| serde_json::from_slice(v).ok()).collect();
                if let Some(missing) = ids.iter().find(|id| !libs.iter().any(|l| &l.id == *id)) {
                    return Err(StoreError::Abort(format!("unknown library {missing}")));
                }
                if libs.len() != ids.len() {
                    return Err(StoreError::Abort("every library must be listed".into()));
                }
                for lib in libs.iter_mut() {
                    lib.position = ids.iter().position(|id| *id == lib.id).map_or(0, |i| i as i64 + 1);
                    tx.put_json(Bucket::Libraries, lib.id.as_bytes(), lib)?;
                }
                Ok(())
            })
            .map_err(|e| match e {
                StoreError::Abort(m) => m,
                e => e.to_string(),
            })?;
        self.list().map_err(|e| e.to_string())
    }

    /// Where a folder lives: `(absolute path, root, path inside it)`;
    /// the root is empty when no mapped root holds it.
    fn locate(&self, path: &str) -> (String, String, String) {
        let roots = self.roots.read().unwrap_or_else(|e| e.into_inner()).clone();
        match roots.relativize(path) {
            Some((r, rel)) => (path.to_string(), r, rel),
            None => (path.to_string(), String::new(), String::new()),
        }
    }

    pub fn get(&self, id: &str) -> Option<Library> {
        let lib: Option<Library> = self.store.read(|tx| tx.get_json(Bucket::Libraries, id.as_bytes())).ok().flatten();
        lib.map(|l| self.resolve(l))
    }

    /// Validates and stores a library (overwriting one with the same
    /// location). With `root`, `path` is relative to that root; an
    /// absolute `path` inside a mapped root is recorded under it. A
    /// rooted library's id derives from `root:root_path`, so it is the
    /// same on every machine. The message texts are the ones clients read.
    pub fn create(&self, name: &str, kind: &str, path: &str, root: &str) -> Result<Library, String> {
        if name.is_empty() || (path.is_empty() && root.is_empty()) {
            return Err("name and path required".into());
        }
        let roots = self.roots.read().unwrap_or_else(|e| e.into_inner()).clone();
        let (abs, root, root_path) = if root.is_empty() {
            match roots.relativize(path) {
                Some((r, rel)) => (path.to_string(), r, rel),
                None => (path.to_string(), String::new(), String::new()),
            }
        } else {
            let rel = path.trim_matches('/').to_string();
            if !valid_rel(&rel) {
                return Err("path inside a root must be relative, with / separators".into());
            }
            let abs = roots.resolve(root, &rel).ok_or_else(|| format!("unknown root {root}"))?;
            (abs, root.to_string(), rel)
        };
        if !std::fs::metadata(&abs).map(|m| m.is_dir()).unwrap_or(false) {
            return Err("path is not a readable directory".into());
        }
        let id = if root.is_empty() { short_id(&abs) } else { short_id(&format!("{root}:{root_path}")) };
        let lib = Library {
            id: format!("lib-{id}"),
            name: name.into(),
            kind: if kind.is_empty() { "anime".into() } else { kind.into() },
            path: abs,
            source: SOURCE_FILESYSTEM.into(),
            created_at: lain_core::now_unix(),
            root,
            root_path,
            ..Library::default()
        };
        self.store.write(|tx| tx.put_json(Bucket::Libraries, lib.id.as_bytes(), &lib)).map_err(|e| e.to_string())?;
        Ok(lib)
    }

    /// Applies an admin patch. Making a library private makes `by` its
    /// owner unless one is named; making it public clears owner and
    /// members. `known_user` checks member ids.
    pub fn update(
        &self,
        id: &str,
        p: &LibraryPatch,
        by: &str,
        known_user: &dyn Fn(&str) -> bool,
    ) -> Result<Library, String> {
        let mut lib: Library = self
            .store
            .read(|tx| tx.get_json(Bucket::Libraries, id.as_bytes()))
            .map_err(|e| e.to_string())?
            .ok_or("unknown library")?;
        if let Some(n) = &p.name {
            if n.trim().is_empty() {
                return Err("name required".into());
            }
            lib.name = n.trim().to_string();
        }
        if let Some(k) = &p.kind {
            if k.trim().is_empty() {
                return Err("type required".into());
            }
            lib.kind = k.trim().to_lowercase();
        }
        if let Some(path) = &p.path {
            let path = path.trim();
            if path.is_empty() {
                return Err("path required".into());
            }
            let (abs, root, root_path) = self.locate(path);
            if !std::fs::metadata(&abs).map(|m| m.is_dir()).unwrap_or(false) {
                return Err("path is not a readable directory".into());
            }
            let others = self.list().map_err(|e| e.to_string())?;
            if others.iter().any(|o| o.id != lib.id && o.path == abs) {
                return Err("another library already reads that folder".into());
            }
            (lib.path, lib.root, lib.root_path) = (abs, root, root_path);
        }
        for u in p.owner.iter().chain(p.members.iter().flatten()) {
            if !known_user(u) {
                return Err(format!("unknown user {u}"));
            }
        }
        if let Some(priv_) = p.private {
            lib.private = priv_;
        }
        if lib.private {
            if let Some(o) = &p.owner {
                lib.owner = o.clone();
            }
            if lib.owner.is_empty() {
                lib.owner = by.to_string();
            }
            if let Some(m) = &p.members {
                let mut m: Vec<String> = m.iter().filter(|u| **u != lib.owner).cloned().collect();
                m.sort();
                m.dedup();
                lib.members = m;
            }
        } else {
            lib.owner.clear();
            lib.members.clear();
        }
        self.store.write(|tx| tx.put_json(Bucket::Libraries, lib.id.as_bytes(), &lib)).map_err(|e| e.to_string())?;
        Ok(self.resolve(lib))
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
    fn short_ids_are_stable_and_fixed_width() {
        assert_eq!(short_id(""), "00000000");
        assert_eq!(short_id("media:Frieren"), short_id("media:Frieren"));
        assert_ne!(short_id("/srv/media"), short_id("/srv/media/Show"));
        let long = short_id(&"x".repeat(10_000));
        assert!(long.len() == 8 && long.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn rooted_libraries_and_privacy_patches() {
        let dir = lain_store::test_dir("libs");
        let media = dir.join("media");
        std::fs::create_dir_all(media.join("anime")).unwrap();
        let store = Arc::new(Store::open(&dir.join("data")).unwrap());
        let doc = format!(
            r#"{{"format":"lain.roots@1","roots":{{"media":{{"path":{:?},"aliases":["V:\\media"]}}}}}}"#,
            media.to_string_lossy()
        );
        let roots = Arc::new(RwLock::new(Roots::parse(doc.as_bytes()).unwrap()));
        let libs = Libraries::new(store.clone(), roots);
        let by_root = libs.create("Anime", "", "anime", "media").unwrap();
        assert_eq!((by_root.root.as_str(), by_root.root_path.as_str()), ("media", "anime"));
        let by_path = libs.create("Anime", "", &media.join("anime").to_string_lossy(), "").unwrap();
        assert_eq!(by_path.id, by_root.id, "same location, same id");
        assert!(libs.create("x", "", "../etc", "media").is_err());
        assert!(libs.create("x", "", "anime", "nope").unwrap_err().contains("unknown root"));
        let known = |u: &str| u == "user-admin" || u == "user-bob";
        let p = libs
            .update(&by_root.id, &LibraryPatch { private: Some(true), ..Default::default() }, "user-admin", &known)
            .unwrap();
        assert!(p.private && p.owner == "user-admin" && p.is_member("user-admin") && !p.is_member("user-bob"));
        let p = libs
            .update(
                &by_root.id,
                &LibraryPatch { members: Some(vec!["user-bob".into(), "user-admin".into()]), ..Default::default() },
                "user-admin",
                &known,
            )
            .unwrap();
        assert_eq!(p.members, vec!["user-bob".to_string()]);
        assert!(
            libs.update(
                &by_root.id,
                &LibraryPatch { members: Some(vec!["user-eve".into()]), ..Default::default() },
                "user-admin",
                &known
            )
            .is_err()
        );
        let p = libs
            .update(&by_root.id, &LibraryPatch { private: Some(false), ..Default::default() }, "user-admin", &known)
            .unwrap();
        assert!(!p.private && p.owner.is_empty() && p.members.is_empty());
        drop(libs);
        drop(store);
        std::fs::remove_dir_all(dir).ok();
    }
}
