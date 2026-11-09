//! The client's local database: one redb file (`client.redb`) shared
//! by the offline store and the progress queue, so eviction can see
//! unsynced progress in the same transaction.
//!
//! # Schema (`lain.client-store@1`)
//!
//! | table            | key                     | value                          |
//! |------------------|-------------------------|--------------------------------|
//! | `offline`        | `server\0item`          | [`crate::OfflineEntry`] JSON   |
//! | `progress_queue` | `server\0user\0item`    | queued offline progress JSON   |
//! | `progress_seen`  | `server\0user\0item`    | last server progress seen JSON |
//! | `meta`           | `schema`, `seq`         | raw bytes                      |

use std::path::{Path, PathBuf};

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{ClientError, store_err};

pub(crate) const OFFLINE: TableDefinition<&str, &[u8]> = TableDefinition::new("offline");
pub(crate) const QUEUE: TableDefinition<&str, &[u8]> = TableDefinition::new("progress_queue");
pub(crate) const SEEN: TableDefinition<&str, &[u8]> = TableDefinition::new("progress_seen");
const META: TableDefinition<&str, &[u8]> = TableDefinition::new("meta");

const SCHEMA: &str = "lain.client-store@1";

/// The opened database. Opening takes redb's file lock, so two
/// processes cannot mutate one client store at once (D-094's `.lock`).
pub struct ClientDb {
    db: Database,
    path: PathBuf,
}

/// Composite keys joined by NUL, so `a` never prefixes `ab`.
pub(crate) fn key(parts: &[&str]) -> String {
    parts.join("\0")
}

pub(crate) fn prefix(parts: &[&str]) -> String {
    let mut k = parts.join("\0");
    k.push('\0');
    k
}

impl ClientDb {
    /// Opens (creating if needed) the database at `path`, in an
    /// owner-only directory.
    pub fn open(path: &Path) -> Result<ClientDb, ClientError> {
        if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            crate::token::private_dir(dir).map_err(|e| store_err(format!("{}: {e}", dir.display())))?;
        }
        let db = Database::create(path).map_err(|e| store_err(format!("{}: {e}", path.display())))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        let me = ClientDb { db, path: path.to_path_buf() };
        me.write(|tx| {
            for t in [OFFLINE, QUEUE, SEEN, META] {
                tx.open_table(t).map_err(store_err)?;
            }
            let mut meta = tx.open_table(META).map_err(store_err)?;
            let found = meta.get("schema").map_err(store_err)?.map(|v| v.value().to_vec());
            match found {
                Some(v) if v != SCHEMA.as_bytes() => {
                    return Err(store_err(format!(
                        "client store schema {} is not {SCHEMA}",
                        String::from_utf8_lossy(&v)
                    )));
                }
                Some(_) => {}
                None => {
                    meta.insert("schema", SCHEMA.as_bytes()).map_err(store_err)?;
                }
            }
            Ok(())
        })?;
        Ok(me)
    }

    /// `<client dir>/client.redb` (see [`crate::client_dir`]).
    pub fn open_default() -> Result<ClientDb, ClientError> {
        let dir = crate::client_dir().ok_or_else(|| store_err("no user data directory"))?;
        Self::open(&dir.join("client.redb"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn read<T>(
        &self,
        f: impl FnOnce(&redb::ReadTransaction) -> Result<T, ClientError>,
    ) -> Result<T, ClientError> {
        let tx = self.db.begin_read().map_err(store_err)?;
        f(&tx)
    }

    /// Commits on `Ok`, rolls back on `Err`.
    pub(crate) fn write<T>(
        &self,
        f: impl FnOnce(&redb::WriteTransaction) -> Result<T, ClientError>,
    ) -> Result<T, ClientError> {
        let tx = self.db.begin_write().map_err(store_err)?;
        match f(&tx) {
            Ok(v) => {
                tx.commit().map_err(store_err)?;
                Ok(v)
            }
            Err(e) => {
                let _ = tx.abort();
                Err(e)
            }
        }
    }
}

fn decode<T: DeserializeOwned>(k: &str, raw: &[u8]) -> Result<T, ClientError> {
    serde_json::from_slice(raw).map_err(|e| store_err(format!("corrupt record {:?}: {e}", k.replace('\0', "/"))))
}

pub(crate) fn get<T: DeserializeOwned>(
    t: &impl ReadableTable<&'static str, &'static [u8]>,
    k: &str,
) -> Result<Option<T>, ClientError> {
    match t.get(k).map_err(store_err)? {
        Some(v) => decode(k, v.value()).map(Some),
        None => Ok(None),
    }
}

pub(crate) fn put<T: Serialize>(
    t: &mut redb::Table<&'static str, &'static [u8]>,
    k: &str,
    v: &T,
) -> Result<(), ClientError> {
    let raw = serde_json::to_vec(v).map_err(store_err)?;
    t.insert(k, raw.as_slice()).map_err(store_err)?;
    Ok(())
}

/// Every record whose key starts with `prefix`, in key order.
pub(crate) fn scan<T: DeserializeOwned>(
    t: &impl ReadableTable<&'static str, &'static [u8]>,
    prefix: &str,
) -> Result<Vec<(String, T)>, ClientError> {
    let mut out = Vec::new();
    for row in t.range::<&str>(prefix..).map_err(store_err)? {
        let (k, v) = row.map_err(store_err)?;
        if !k.value().starts_with(prefix) {
            break;
        }
        out.push((k.value().to_string(), decode(k.value(), v.value())?));
    }
    Ok(out)
}

/// The next value of the store-wide sequence (queue and offline order).
pub(crate) fn next_seq(tx: &redb::WriteTransaction) -> Result<u64, ClientError> {
    let mut meta = tx.open_table(META).map_err(store_err)?;
    let cur = meta
        .get("seq")
        .map_err(store_err)?
        .and_then(|v| v.value().try_into().ok().map(u64::from_be_bytes))
        .unwrap_or(0);
    let next = cur + 1;
    meta.insert("seq", next.to_be_bytes().as_slice()).map_err(store_err)?;
    Ok(next)
}
