//! The list store (`lain-list-redb`): entries keyed
//! `user\0platform\0remote_id` and linked accounts keyed
//! `user\0platform`, in the lists module's own tables. A sync replaces
//! one platform's set wholesale: the remote is authoritative (D-081).

use std::collections::BTreeSet;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};

use lain_core::{Error, Provider};
use lain_store::{Bucket, Store, StoreError, join_key, key_prefix};

use crate::{CAP_LIST_ACCOUNT, CAP_LIST_READ, CAP_LIST_WRITE, LinkedAccount, ListEntry, ListSyncStats};

pub const PROVIDER_ID: &str = "lain-list-redb";

/// Op names.
pub mod op {
    /// `lain.list.read@1`: `{user_id, type?, status?}` → `[ListEntry]`
    pub const LIST: &str = "list";
    /// `lain.list.write@1`: `{user_id, platform, entries}` → `ListSyncStats`
    pub const PUT_PLATFORM: &str = "put-platform";
    /// `lain.list.write@1`: `{user_id, platform}` → the number removed
    pub const DELETE_PLATFORM: &str = "delete-platform";
    /// `lain.list.account@1`: `{user_id, platform}` → `LinkedAccount`
    pub const GET: &str = "get";
    /// `lain.list.account@1`: `{account}` → `LinkedAccount`
    pub const PUT: &str = "put";
    /// `lain.list.account@1`: `{user_id, platform}` → whether it existed
    pub const DELETE: &str = "delete";
    /// `lain.list.account@1`: `{user_id?}` → `[LinkedAccount]` (all
    /// accounts when `user_id` is empty: the scheduler's view)
    pub const ACCOUNTS: &str = "list";
}

pub struct ListStore {
    store: Arc<Store>,
}

#[derive(Deserialize, Default)]
struct ListIn {
    #[serde(default)]
    user_id: String,
    #[serde(default, rename = "type")]
    media_type: String,
    #[serde(default)]
    status: String,
}

#[derive(Deserialize, Default)]
struct PutPlatformIn {
    #[serde(default)]
    user_id: String,
    #[serde(default)]
    platform: String,
    #[serde(default)]
    entries: Vec<ListEntry>,
}

#[derive(Deserialize, Default)]
struct AccountKey {
    #[serde(default)]
    user_id: String,
    #[serde(default)]
    platform: String,
}

#[derive(Deserialize, Default)]
struct PutAccountIn {
    #[serde(default)]
    account: LinkedAccount,
}

fn decode<T: for<'de> Deserialize<'de> + Default>(v: Value, what: &str) -> Result<T, Error> {
    if v.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value(v).map_err(|_| Error::invalid(format!("{what} required")))
}

fn store_err(e: StoreError) -> Error {
    Error::internal(e.to_string())
}

fn entry_key(user: &str, platform: &str, remote: &str) -> Vec<u8> {
    join_key(&format!("{user}\0{platform}"), remote)
}

fn platform_prefix(user: &str, platform: &str) -> Vec<u8> {
    key_prefix(&format!("{user}\0{platform}"))
}

fn need_pair(k: &AccountKey) -> Result<(), Error> {
    if k.user_id.is_empty() || k.platform.is_empty() {
        return Err(Error::invalid("user_id and platform required"));
    }
    Ok(())
}

impl ListStore {
    pub fn new(store: Arc<Store>) -> ListStore {
        ListStore { store }
    }

    /// The user's entries, filtered, sorted by title then platform.
    pub fn list(&self, user_id: &str, media_type: &str, status: &str) -> Result<Vec<ListEntry>, Error> {
        let rows = self.store.read(|tx| tx.scan_prefix(Bucket::List, &key_prefix(user_id))).map_err(store_err)?;
        let mut out: Vec<ListEntry> = rows
            .into_iter()
            .filter_map(|(_, v)| serde_json::from_slice::<ListEntry>(&v).ok())
            .filter(|e| media_type.is_empty() || e.media_type == media_type)
            .filter(|e| status.is_empty() || e.status == status)
            .collect();
        out.sort_by(|a, b| a.title.cmp(&b.title).then_with(|| a.platform.cmp(&b.platform)));
        Ok(out)
    }

    /// Replaces the user's platform set with the remote answer: upserts
    /// incoming entries and removes stored ones the remote dropped.
    pub fn put_platform(&self, user_id: &str, platform: &str, entries: Vec<ListEntry>) -> Result<ListSyncStats, Error> {
        if user_id.is_empty() || platform.is_empty() {
            return Err(Error::invalid("user_id and platform required"));
        }
        if entries.iter().any(|e| e.remote_id.trim().is_empty() || e.title.trim().is_empty()) {
            return Err(Error::invalid("entries need remote_id and title"));
        }
        let now = lain_core::now_unix();
        self.store
            .write(|tx| {
                let mut stats = ListSyncStats::default();
                let mut seen = BTreeSet::new();
                for mut e in entries {
                    let rid = e.remote_id.trim().to_string();
                    if !seen.insert(rid.clone()) {
                        continue;
                    }
                    e.user_id = user_id.into();
                    e.platform = platform.into();
                    e.id = format!("{platform}:{rid}");
                    e.remote_id = rid.clone();
                    e.updated_at = now;
                    tx.put_json(Bucket::List, &entry_key(user_id, platform, &rid), &e)?;
                    stats.upserted += 1;
                }
                let prefix = platform_prefix(user_id, platform);
                for (k, _) in tx.scan_prefix(Bucket::List, &prefix)? {
                    let rid = String::from_utf8_lossy(&k[prefix.len()..]).into_owned();
                    if !seen.contains(&rid) {
                        tx.delete(Bucket::List, &k)?;
                        stats.removed += 1;
                    }
                }
                Ok(stats)
            })
            .map_err(store_err)
    }

    /// Removes every entry one platform owns for a user (unlink, D-079).
    pub fn delete_platform(&self, user_id: &str, platform: &str) -> Result<i64, Error> {
        self.store
            .write(|tx| {
                let keys: Vec<Vec<u8>> = tx
                    .scan_prefix(Bucket::List, &platform_prefix(user_id, platform))?
                    .into_iter()
                    .map(|(k, _)| k)
                    .collect();
                for k in &keys {
                    tx.delete(Bucket::List, k)?;
                }
                Ok(keys.len() as i64)
            })
            .map_err(store_err)
    }

    /// One linked account; missing is a typed not-found.
    pub fn account(&self, user_id: &str, platform: &str) -> Result<LinkedAccount, Error> {
        let got = self
            .store
            .read(|tx| tx.get(Bucket::ListAccounts, &join_key(user_id, platform)))
            .map_err(store_err)?
            .and_then(|raw| serde_json::from_slice(&raw).ok());
        got.ok_or_else(|| Error::not_found(format!("no {platform} account linked")))
    }

    pub fn put_account(&self, a: &LinkedAccount) -> Result<(), Error> {
        self.store
            .write(|tx| tx.put_json(Bucket::ListAccounts, &join_key(&a.user_id, &a.platform), a))
            .map_err(store_err)
    }

    /// Removes a linked account; reports whether it existed.
    pub fn delete_account(&self, user_id: &str, platform: &str) -> Result<bool, Error> {
        self.store.write(|tx| tx.delete(Bucket::ListAccounts, &join_key(user_id, platform))).map_err(store_err)
    }

    /// One user's accounts, or every account when `user_id` is empty.
    pub fn accounts(&self, user_id: &str) -> Result<Vec<LinkedAccount>, Error> {
        let prefix = if user_id.is_empty() { Vec::new() } else { key_prefix(user_id) };
        let rows = self.store.read(|tx| tx.scan_prefix(Bucket::ListAccounts, &prefix)).map_err(store_err)?;
        Ok(rows.into_iter().filter_map(|(_, v)| serde_json::from_slice(&v).ok()).collect())
    }
}

impl Provider for ListStore {
    fn id(&self) -> &str {
        PROVIDER_ID
    }
    fn capabilities(&self) -> Vec<String> {
        vec![CAP_LIST_READ.into(), CAP_LIST_WRITE.into(), CAP_LIST_ACCOUNT.into()]
    }
    fn health(&self) -> Result<(), Error> {
        Ok(())
    }
    fn invoke(&self, cap: &str, op: &str, input: Value) -> Result<Value, Error> {
        match (cap, op) {
            (CAP_LIST_READ, op::LIST) => {
                let i: ListIn = decode(input, "ListInput")?;
                if i.user_id.is_empty() {
                    return Err(Error::invalid("user_id required"));
                }
                Ok(json!(self.list(&i.user_id, &i.media_type, &i.status)?))
            }
            (CAP_LIST_WRITE, op::PUT_PLATFORM) => {
                let i: PutPlatformIn = decode(input, "PutPlatformInput")?;
                Ok(json!(self.put_platform(&i.user_id, &i.platform, i.entries)?))
            }
            (CAP_LIST_WRITE, op::DELETE_PLATFORM) => {
                let k: AccountKey = decode(input, "DeletePlatformInput")?;
                need_pair(&k)?;
                Ok(json!(self.delete_platform(&k.user_id, &k.platform)?))
            }
            (CAP_LIST_ACCOUNT, op::GET) => {
                let k: AccountKey = decode(input, "GetAccountInput")?;
                need_pair(&k)?;
                Ok(json!(self.account(&k.user_id, &k.platform)?))
            }
            (CAP_LIST_ACCOUNT, op::PUT) => {
                let i: PutAccountIn = decode(input, "PutAccountInput")?;
                if i.account.user_id.is_empty() || i.account.platform.is_empty() {
                    return Err(Error::invalid("account needs user_id and platform"));
                }
                self.put_account(&i.account)?;
                Ok(json!(i.account))
            }
            (CAP_LIST_ACCOUNT, op::DELETE) => {
                let k: AccountKey = decode(input, "DeleteAccountInput")?;
                need_pair(&k)?;
                Ok(json!(self.delete_account(&k.user_id, &k.platform)?))
            }
            (CAP_LIST_ACCOUNT, op::ACCOUNTS) => {
                let k: AccountKey = decode(input, "ListAccountsInput")?;
                Ok(json!(self.accounts(&k.user_id)?))
            }
            (CAP_LIST_READ | CAP_LIST_WRITE | CAP_LIST_ACCOUNT, _) => {
                Err(Error::invalid(format!("unknown op {op} of {cap}")))
            }
            _ => Err(Error::invalid(format!("unsupported cap {cap}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(rid: &str, title: &str) -> ListEntry {
        ListEntry { remote_id: rid.into(), title: title.into(), media_type: "anime".into(), ..ListEntry::default() }
    }

    fn open() -> (ListStore, std::path::PathBuf) {
        let dir = lain_store::test_dir("list");
        (ListStore::new(Arc::new(Store::open(&dir).unwrap())), dir)
    }

    #[test]
    fn put_platform_upserts_removes_and_scopes() {
        let (s, dir) = open();
        let st = s
            .put_platform("u1", "anilist", vec![entry("1", "Frieren"), entry("2", "Bocchi"), entry("1", "dup")])
            .unwrap();
        assert_eq!((st.upserted, st.removed), (2, 0));
        let st = s.put_platform("u1", "anilist", vec![entry(" 2 ", "Bocchi"), entry("3", "Apothecary")]).unwrap();
        assert_eq!((st.upserted, st.removed), (2, 1));
        s.put_platform("u10", "anilist", vec![entry("9", "Other")]).unwrap();
        s.put_platform("u1", "mal", vec![entry("1", "Bocchi")]).unwrap();
        let all = s.list("u1", "", "").unwrap();
        let ids: Vec<&str> = all.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["anilist:3", "anilist:2", "mal:1"], "sorted by title then platform, scoped to u1");
        assert_eq!((all[0].user_id.as_str(), all[0].platform.as_str()), ("u1", "anilist"));
        assert_eq!(s.delete_platform("u1", "anilist").unwrap(), 2);
        assert_eq!(s.list("u1", "", "").unwrap().len(), 1);
        assert_eq!(s.list("u10", "", "").unwrap().len(), 1);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn filters_and_bad_input() {
        let (s, dir) = open();
        let mut m = entry("2", "Solo");
        m.media_type = "manga".into();
        m.status = "planning".into();
        let mut a = entry("1", "Frieren");
        a.status = "current".into();
        s.put_platform("u", "anilist", vec![a, m]).unwrap();
        assert_eq!(s.list("u", "manga", "").unwrap().len(), 1);
        assert_eq!(s.list("u", "", "current").unwrap()[0].title, "Frieren");
        assert!(s.list("u", "anime", "planning").unwrap().is_empty());
        assert_eq!(s.put_platform("u", "", vec![]).unwrap_err().code, "invalid-message");
        assert_eq!(s.put_platform("u", "anilist", vec![entry(" ", "x")]).unwrap_err().code, "invalid-message");
        assert_eq!(s.invoke(CAP_LIST_READ, op::LIST, json!({})).unwrap_err().code, "invalid-message");
        assert_eq!(s.invoke(CAP_LIST_READ, "nope", json!({"user_id": "u"})).unwrap_err().code, "invalid-message");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn account_crud_and_token_never_in_view() {
        let (s, dir) = open();
        assert_eq!(s.account("u", "anilist").unwrap_err().msg, "no anilist account linked");
        let a = LinkedAccount {
            user_id: "u".into(),
            platform: "anilist".into(),
            remote_user_id: "42".into(),
            token: "sekret".into(),
            token_expires_at: 100,
            ..LinkedAccount::default()
        };
        s.invoke(CAP_LIST_ACCOUNT, op::PUT, json!({"account": a})).unwrap();
        assert_eq!(s.account("u", "anilist").unwrap().token, "sekret");
        let other = LinkedAccount { user_id: "v".into(), ..a.clone() };
        s.put_account(&other).unwrap();
        assert_eq!(s.accounts("u").unwrap().len(), 1);
        assert_eq!(s.accounts("").unwrap().len(), 2);
        let view = serde_json::to_string(&a.view(200)).unwrap();
        assert!(!view.contains("sekret") && view.contains("\"token_expired\":true"), "{view}");
        assert!(s.delete_account("u", "anilist").unwrap());
        assert!(!s.delete_account("u", "anilist").unwrap());
        assert_eq!(s.invoke(CAP_LIST_ACCOUNT, op::PUT, json!({"account": {}})).unwrap_err().code, "invalid-message");
        std::fs::remove_dir_all(dir).ok();
    }
}
