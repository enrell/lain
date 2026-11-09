//! The embedded database: one redb file (`lain.redb`) with one table
//! per record kind. Values are JSON documents keyed by stable ids;
//! secondary access is through composite keys joined by NUL
//! (`<library_id>\0<item_id>`, `<user_id>\0<item_id>`), so a prefix
//! scan is an index walk. One write transaction per mutation keeps
//! every provider crash-consistent. A store starts empty: nothing is
//! imported from the Go server (D-177).
//!
//! # Schema (`lain.store@1`)
//!
//! | table              | key                      | value                     |
//! |--------------------|--------------------------|---------------------------|
//! | `users`            | user id                  | user JSON (Argon2id hash) |
//! | `users_by_name`    | username                 | user id                   |
//! | `libraries`        | library id               | library JSON              |
//! | `items`            | item id                  | catalog item JSON         |
//! | `items_by_library` | `library\0item`          | empty                     |
//! | `progress`         | `user\0item`             | progress JSON             |
//! | `user_pins`        | user id                  | `{"hash": Argon2id}`      |
//! | `meta`             | `secret`, `schema`       | raw bytes                 |
//!
//! Library JSON carries `private`, `owner`, `members` (D-156), `root`
//! and `root_path` (D-157); a rooted item stores `root` + `rel_path`
//! and no absolute path. Unlocks of private libraries are never stored:
//! they live in memory per session. Tables of modules not ported yet
//! arrive with their module.

use std::path::{Path, PathBuf};

use redb::{Database, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// The schema id stored under `meta/schema`.
pub const SCHEMA: &str = "lain.store@1";

/// The database file name inside the data dir.
pub const FILE_NAME: &str = "lain.redb";

/// One table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Bucket {
    Users,
    UsersByName,
    Libraries,
    Items,
    ItemsByLibrary,
    Progress,
    Meta,
    /// Per-user library PIN hashes (Argon2id), D-156.
    UserPins,
}

impl Bucket {
    pub const ALL: [Bucket; 8] = [
        Bucket::Users,
        Bucket::UsersByName,
        Bucket::Libraries,
        Bucket::Items,
        Bucket::ItemsByLibrary,
        Bucket::Progress,
        Bucket::Meta,
        Bucket::UserPins,
    ];

    /// The table name.
    pub fn name(self) -> &'static str {
        match self {
            Bucket::Users => "users",
            Bucket::UsersByName => "users_by_name",
            Bucket::Libraries => "libraries",
            Bucket::Items => "items",
            Bucket::ItemsByLibrary => "items_by_library",
            Bucket::Progress => "progress",
            Bucket::Meta => "meta",
            Bucket::UserPins => "user_pins",
        }
    }

    fn table(self) -> TableDefinition<'static, &'static [u8], &'static [u8]> {
        TableDefinition::new(self.name())
    }
}

/// A store failure. Missing keys are not errors (`Ok(None)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// The database or a transaction failed.
    Db(String),
    /// A present record does not decode: reported, never ignored.
    Corrupt { bucket: &'static str, key: String, err: String },
    /// A caller-level failure raised inside a transaction (rolled back).
    Abort(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Db(e) => write!(f, "store: {e}"),
            StoreError::Corrupt { bucket, key, err } => write!(f, "corrupt record {bucket}/{key}: {err}"),
            StoreError::Abort(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for StoreError {}

fn db_err(e: impl std::fmt::Display) -> StoreError {
    StoreError::Db(e.to_string())
}

/// The database. Opening it takes redb's file lock, so two servers on
/// one data dir fail fast instead of corrupting each other.
pub struct Store {
    db: Database,
    path: PathBuf,
}

/// Owner-only data dir (0700 on Unix).
pub fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(dir)?.permissions();
        if perm.mode() & 0o077 != 0 {
            perm.set_mode(0o700);
            let _ = std::fs::set_permissions(dir, perm);
        }
    }
    Ok(())
}

impl Store {
    /// Opens (creating if needed) `<data_dir>/lain.redb` with every
    /// table present and the schema recorded.
    pub fn open(data_dir: &Path) -> Result<Store, StoreError> {
        ensure_private_dir(data_dir).map_err(db_err)?;
        let path = data_dir.join(FILE_NAME);
        let db = Database::create(&path).map_err(db_err)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        let store = Store { db, path };
        store.write(|tx| {
            for b in Bucket::ALL {
                tx.ensure(b)?;
            }
            match tx.get(Bucket::Meta, b"schema")? {
                Some(v) if v != SCHEMA.as_bytes() => {
                    return Err(StoreError::Abort(format!(
                        "store schema {} is not {SCHEMA}; this build cannot open it",
                        String::from_utf8_lossy(&v)
                    )));
                }
                Some(_) => {}
                None => tx.put(Bucket::Meta, b"schema", SCHEMA.as_bytes())?,
            }
            Ok(())
        })?;
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Runs `f` in a read transaction (a consistent snapshot).
    pub fn read<T>(&self, f: impl FnOnce(&ReadTx) -> Result<T, StoreError>) -> Result<T, StoreError> {
        let tx = self.db.begin_read().map_err(db_err)?;
        f(&ReadTx { tx })
    }

    /// Runs `f` in a write transaction; it commits on `Ok` and rolls
    /// back on `Err`.
    pub fn write<T>(&self, f: impl FnOnce(&mut WriteTx) -> Result<T, StoreError>) -> Result<T, StoreError> {
        let tx = self.db.begin_write().map_err(db_err)?;
        let mut w = WriteTx { tx };
        match f(&mut w) {
            Ok(v) => {
                w.tx.commit().map_err(db_err)?;
                Ok(v)
            }
            Err(e) => {
                let _ = w.tx.abort();
                Err(e)
            }
        }
    }
}

fn decode<T: DeserializeOwned>(bucket: Bucket, key: &[u8], raw: &[u8]) -> Result<T, StoreError> {
    serde_json::from_slice(raw).map_err(|e| StoreError::Corrupt {
        bucket: bucket.name(),
        key: String::from_utf8_lossy(key).into_owned(),
        err: e.to_string(),
    })
}

/// The successor of every key that starts with `prefix`, for a range
/// end (`None` when the prefix is all 0xff).
fn prefix_end(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut end = prefix.to_vec();
    while let Some(last) = end.pop() {
        if last < 0xff {
            end.push(last + 1);
            return Some(end);
        }
    }
    None
}

type Pairs = Vec<(Vec<u8>, Vec<u8>)>;

fn scan<T: ReadableTable<&'static [u8], &'static [u8]>>(t: &T, prefix: &[u8]) -> Result<Pairs, StoreError> {
    let mut out = Vec::new();
    let end = prefix_end(prefix);
    let iter = match &end {
        Some(end) => t.range::<&[u8]>(prefix..end.as_slice()).map_err(db_err)?,
        None => t.range::<&[u8]>(prefix..).map_err(db_err)?,
    };
    for row in iter {
        let (k, v) = row.map_err(db_err)?;
        out.push((k.value().to_vec(), v.value().to_vec()));
    }
    Ok(out)
}

macro_rules! reads {
    () => {
        /// The raw value under `key`.
        pub fn get(&self, bucket: Bucket, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
            let t = self.table(bucket)?;
            let v = t.get(key).map_err(db_err)?;
            Ok(v.map(|g| g.value().to_vec()))
        }

        /// The JSON document under `key`; a bad document is `Corrupt`.
        pub fn get_json<T: DeserializeOwned>(&self, bucket: Bucket, key: &[u8]) -> Result<Option<T>, StoreError> {
            match self.get(bucket, key)? {
                Some(raw) => decode(bucket, key, &raw).map(Some),
                None => Ok(None),
            }
        }

        /// Every pair whose key starts with `prefix`, in key order.
        pub fn scan_prefix(&self, bucket: Bucket, prefix: &[u8]) -> Result<Pairs, StoreError> {
            scan(&self.table(bucket)?, prefix)
        }

        /// Every pair in key order.
        pub fn all(&self, bucket: Bucket) -> Result<Pairs, StoreError> {
            self.scan_prefix(bucket, b"")
        }

        /// The number of keys.
        pub fn count(&self, bucket: Bucket) -> Result<u64, StoreError> {
            self.table(bucket)?.len().map_err(db_err)
        }
    };
}

/// A read snapshot.
pub struct ReadTx {
    tx: redb::ReadTransaction,
}

impl ReadTx {
    fn table(&self, bucket: Bucket) -> Result<redb::ReadOnlyTable<&'static [u8], &'static [u8]>, StoreError> {
        self.tx.open_table(bucket.table()).map_err(db_err)
    }
    reads!();
}

/// A write transaction.
pub struct WriteTx {
    tx: redb::WriteTransaction,
}

impl WriteTx {
    fn table(&self, bucket: Bucket) -> Result<redb::Table<'_, &'static [u8], &'static [u8]>, StoreError> {
        self.tx.open_table(bucket.table()).map_err(db_err)
    }

    fn ensure(&self, bucket: Bucket) -> Result<(), StoreError> {
        self.table(bucket).map(|_| ())
    }

    reads!();

    pub fn put(&mut self, bucket: Bucket, key: &[u8], value: &[u8]) -> Result<(), StoreError> {
        self.table(bucket)?.insert(key, value).map_err(db_err)?;
        Ok(())
    }

    pub fn put_json<T: Serialize>(&mut self, bucket: Bucket, key: &[u8], value: &T) -> Result<(), StoreError> {
        let raw = serde_json::to_vec(value).map_err(db_err)?;
        self.put(bucket, key, &raw)
    }

    /// Removes `key`; returns whether it existed.
    pub fn delete(&mut self, bucket: Bucket, key: &[u8]) -> Result<bool, StoreError> {
        let existed = self.table(bucket)?.remove(key).map_err(db_err)?.is_some();
        Ok(existed)
    }

    /// Removes every key of a bucket.
    pub fn clear(&mut self, bucket: Bucket) -> Result<(), StoreError> {
        let keys: Vec<Vec<u8>> = self.all(bucket)?.into_iter().map(|(k, _)| k).collect();
        let mut t = self.table(bucket)?;
        for k in keys {
            t.remove(k.as_slice()).map_err(db_err)?;
        }
        Ok(())
    }
}

/// A composite key: `a\0b`.
pub fn join_key(a: &str, b: &str) -> Vec<u8> {
    let mut k = Vec::with_capacity(a.len() + b.len() + 1);
    k.extend_from_slice(a.as_bytes());
    k.push(0);
    k.extend_from_slice(b.as_bytes());
    k
}

/// The scan prefix of composite keys under `a`: `a\0`.
pub fn key_prefix(a: &str) -> Vec<u8> {
    let mut k = a.as_bytes().to_vec();
    k.push(0);
    k
}

/// A unique scratch directory for tests (removed by the caller).
pub fn test_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "lain-test-{tag}-{}-{nanos}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).expect("test dir");
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_get_scan_delete() {
        let dir = test_dir("store");
        let store = Store::open(&dir).unwrap();
        store
            .write(|tx| {
                tx.put_json(Bucket::Users, b"user-a", &serde_json::json!({"id": "user-a"}))?;
                tx.put(Bucket::Progress, &join_key("u1", "i1"), b"{}")?;
                tx.put(Bucket::Progress, &join_key("u1", "i2"), b"{}")?;
                tx.put(Bucket::Progress, &join_key("u10", "i1"), b"{}")?;
                tx.put(Bucket::Progress, &join_key("u2", "i1"), b"{}")?;
                Ok(())
            })
            .unwrap();
        store
            .read(|tx| {
                let v: serde_json::Value = tx.get_json(Bucket::Users, b"user-a")?.unwrap();
                assert_eq!(v["id"], "user-a");
                assert!(tx.get(Bucket::Users, b"nope")?.is_none());
                // The NUL separator keeps u1 from matching u10.
                assert_eq!(tx.scan_prefix(Bucket::Progress, &key_prefix("u1"))?.len(), 2);
                assert_eq!(tx.count(Bucket::Progress)?, 4);
                Ok(())
            })
            .unwrap();
        let err = store
            .write(|tx| {
                tx.delete(Bucket::Users, b"user-a")?;
                Err::<(), _>(StoreError::Abort("rollback".into()))
            })
            .unwrap_err();
        assert_eq!(err, StoreError::Abort("rollback".into()));
        assert!(store.read(|tx| tx.get(Bucket::Users, b"user-a")).unwrap().is_some());
        store.write(|tx| tx.clear(Bucket::Progress)).unwrap();
        assert_eq!(store.read(|tx| tx.count(Bucket::Progress)).unwrap(), 0);
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_records_are_reported() {
        let dir = test_dir("corrupt");
        let store = Store::open(&dir).unwrap();
        store.write(|tx| tx.put(Bucket::Items, b"x", b"{not json")).unwrap();
        let err = store.read(|tx| tx.get_json::<serde_json::Value>(Bucket::Items, b"x")).unwrap_err();
        assert!(matches!(err, StoreError::Corrupt { bucket: "items", .. }));
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reopen_keeps_data_and_schema() {
        let dir = test_dir("reopen");
        {
            let store = Store::open(&dir).unwrap();
            store.write(|tx| tx.put(Bucket::Meta, b"secret", b"s")).unwrap();
        }
        let store = Store::open(&dir).unwrap();
        assert_eq!(store.read(|tx| tx.get(Bucket::Meta, b"secret")).unwrap().unwrap(), b"s");
        assert_eq!(store.read(|tx| tx.get(Bucket::Meta, b"schema")).unwrap().unwrap(), SCHEMA.as_bytes());
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn prefix_end_handles_ff() {
        assert_eq!(prefix_end(b"a\0"), Some(b"a\x01".to_vec()));
        assert_eq!(prefix_end(&[0x61, 0xff]), Some(vec![0x62]));
        assert_eq!(prefix_end(&[0xff, 0xff]), None);
    }
}
