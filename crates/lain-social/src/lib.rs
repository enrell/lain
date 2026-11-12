//! The social module (docs/slices/social.md): privacy settings and
//! favorites, relationships, activity, notifications, ratings, reviews,
//! comments and collections, in its own store tables beside catalog,
//! userstate and the list. Works are referenced by kind plus normalized
//! title, so every media kind gets the same social surface.
//!
//! The provider trusts the gateway for identity: user ids it receives
//! are authenticated and exist. It owns every privacy rule, so the
//! gateway asks it what a viewer may see instead of deciding itself.
//! Private libraries (D-156) reach it as `hidden_libraries` on every
//! read: a row whose work names an item of one of them is left out
//! before paging, counting and averaging, so nothing about it leaks.
//!
//! One in-process provider, `lain-social-redb`, serves the four Go
//! capabilities with Go's input shapes; the registry needs op names, so
//! each capability's inputs are told apart by op (see [`Social::invoke_op`]).

mod activity;
mod collections;
mod forget;
mod graph;
mod reviews;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use lain_core::{Error, Provider};
use lain_store::{Bucket, ReadTx, Store, StoreError, WriteTx};

pub use lain_api::{
    Activity, Collection, CollectionItem, Comment, Notification, Rating, Relationship, SocialSettings, WorkRef,
};

pub use activity::*;
pub use collections::*;
pub use graph::*;
pub use reviews::*;

pub const PROVIDER_ID: &str = "lain-social-redb";

pub const CAP_SOCIAL_GRAPH: &str = "lain.social.graph@1";
pub const CAP_SOCIAL_ACTIVITY: &str = "lain.social.activity@1";
pub const CAP_SOCIAL_REVIEWS: &str = "lain.social.reviews@1";
pub const CAP_SOCIAL_COLLECTIONS: &str = "lain.social.collections@1";

pub const CAPABILITIES: [&str; 4] = [CAP_SOCIAL_GRAPH, CAP_SOCIAL_ACTIVITY, CAP_SOCIAL_REVIEWS, CAP_SOCIAL_COLLECTIONS];

/// Typed codes the gateway maps onto HTTP.
pub const FORBIDDEN: &str = "forbidden";

// Bounds (S-6..S-10).
pub const MAX_TITLE: usize = 200;
pub const MAX_REVIEW: usize = 2000;
pub const MAX_COMMENT: usize = 1000;
pub const MAX_MESSAGE: usize = 280;
pub const MAX_COLLECTION_NAME: usize = 80;
pub const MAX_COLLECTION_DESC: usize = 500;
pub const MAX_COLLECTION_SIZE: usize = 500;
pub const MAX_FAVORITES: usize = 12;
pub const MAX_ACTIVITY_ROWS: usize = 500;
pub const MAX_NOTIFICATIONS: usize = 200;
pub const MAX_SHARE_TARGETS: usize = 50;
/// One "progress" row per work per window.
pub const PROGRESS_WINDOW_SECS: i64 = 6 * 3600;
pub const DEFAULT_PAGE_LIMIT: usize = 50;
pub const MAX_PAGE_LIMIT: usize = 200;

/// The clock in unix nanoseconds; tests replace it.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// Called with every notification stored, after its transaction
/// commits (the server turns it into an `/api/events` event).
pub type NotifyHook = Arc<dyn Fn(&Notification) + Send + Sync>;

/// The social provider.
pub struct Social {
    store: Arc<Store>,
    clock: Mutex<Clock>,
    seq: AtomicU32,
    hook: Mutex<Option<NotifyHook>>,
}

fn system_nanos() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as i64).unwrap_or(0)
}

impl Social {
    pub fn new(store: Arc<Store>) -> Social {
        Social { store, clock: Mutex::new(Arc::new(system_nanos)), seq: AtomicU32::new(0), hook: Mutex::new(None) }
    }

    /// Replaces the clock (tests cross the dedupe window with it).
    pub fn set_clock(&self, clock: Clock) {
        *self.clock.lock().unwrap_or_else(|e| e.into_inner()) = clock;
    }

    pub fn on_notify(&self, hook: NotifyHook) {
        *self.hook.lock().unwrap_or_else(|e| e.into_inner()) = Some(hook);
    }

    fn nanos(&self) -> i64 {
        (self.clock.lock().unwrap_or_else(|e| e.into_inner()))()
    }

    fn now(&self) -> i64 {
        self.nanos().div_euclid(1_000_000_000)
    }

    /// A time-ordered unique id: 16 hex digits of unix nanos, then a
    /// process sequence and random bits, so ids minted in the same
    /// nanosecond still sort in creation order and never collide.
    fn new_id(&self) -> String {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst).wrapping_add(1);
        let mut r = [0u8; 4];
        let _ = getrandom::fill(&mut r);
        format!("{:016x}{:08x}{:08x}", self.nanos() as u64, seq, u32::from_be_bytes(r))
    }

    /// A write transaction whose notifications are announced only once
    /// it commits.
    fn update<T>(&self, f: impl FnOnce(&mut Tx<'_>) -> Result<T, Error>) -> Result<T, Error> {
        let mut pending = Vec::new();
        let mut failure = None;
        let out = self.store.write(|w| {
            let mut tx = Tx { w, notes: &mut pending };
            match f(&mut tx) {
                Ok(v) => Ok(Some(v)),
                Err(e) => {
                    failure = Some(e);
                    Err(StoreError::Abort("rolled back".into()))
                }
            }
        });
        if let Some(e) = failure {
            return Err(e);
        }
        let out = out.map_err(store_err)?.ok_or_else(|| Error::internal("empty transaction"))?;
        let hook = self.hook.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(h) = hook {
            for n in &pending {
                h(n);
            }
        }
        Ok(out)
    }

    fn view<T>(&self, f: impl FnOnce(&ReadTx) -> Result<T, Error>) -> Result<T, Error> {
        let mut failure = None;
        let out = self.store.read(|r| match f(r) {
            Ok(v) => Ok(Some(v)),
            Err(e) => {
                failure = Some(e);
                Ok(None)
            }
        });
        if let Some(e) = failure {
            return Err(e);
        }
        out.map_err(store_err)?.ok_or_else(|| Error::internal("empty read"))
    }

    /// Dispatches one capability call by op; inputs are Go's JSON shapes.
    pub fn invoke_op(&self, cap: &str, op: &str, input: Value) -> Result<Value, Error> {
        match cap {
            CAP_SOCIAL_GRAPH => self.invoke_graph(op, input),
            CAP_SOCIAL_ACTIVITY => self.invoke_activity(op, input),
            CAP_SOCIAL_REVIEWS => self.invoke_reviews(op, input),
            CAP_SOCIAL_COLLECTIONS => self.invoke_collections(op, input),
            _ => Err(Error::invalid(format!("unsupported cap {cap}"))),
        }
    }
}

impl Provider for Social {
    fn id(&self) -> &str {
        PROVIDER_ID
    }
    fn capabilities(&self) -> Vec<String> {
        CAPABILITIES.iter().map(|c| c.to_string()).collect()
    }
    fn health(&self) -> Result<(), Error> {
        Ok(())
    }
    fn invoke(&self, cap: &str, op: &str, input: Value) -> Result<Value, Error> {
        self.invoke_op(cap, op, input)
    }
}

/// A write transaction plus the notifications it stored.
pub(crate) struct Tx<'a> {
    w: &'a mut WriteTx,
    notes: &'a mut Vec<Notification>,
}

pub(crate) type Pairs = Vec<(Vec<u8>, Vec<u8>)>;

/// Reads shared by read and write transactions.
pub(crate) trait Rx {
    fn raw(&self, b: Bucket, k: &[u8]) -> Result<Option<Vec<u8>>, StoreError>;
    fn pairs(&self, b: Bucket, prefix: &[u8]) -> Result<Pairs, StoreError>;
}

impl Rx for ReadTx {
    fn raw(&self, b: Bucket, k: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        self.get(b, k)
    }
    fn pairs(&self, b: Bucket, prefix: &[u8]) -> Result<Pairs, StoreError> {
        self.scan_prefix(b, prefix)
    }
}

impl Rx for Tx<'_> {
    fn raw(&self, b: Bucket, k: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        self.w.get(b, k)
    }
    fn pairs(&self, b: Bucket, prefix: &[u8]) -> Result<Pairs, StoreError> {
        self.w.scan_prefix(b, prefix)
    }
}

impl Tx<'_> {
    fn put<T: Serialize>(&mut self, b: Bucket, k: &[u8], v: &T) -> Result<(), Error> {
        self.w.put_json(b, k, v).map_err(store_err)
    }
    fn put_raw(&mut self, b: Bucket, k: &[u8], v: &[u8]) -> Result<(), Error> {
        self.w.put(b, k, v).map_err(store_err)
    }
    fn delete(&mut self, b: Bucket, k: &[u8]) -> Result<bool, Error> {
        self.w.delete(b, k).map_err(store_err)
    }
}

/// A JSON record; a missing or undecodable one reads as `None` (Go's
/// `GetJSON` error path).
pub(crate) fn get_json<T: DeserializeOwned>(rx: &impl Rx, b: Bucket, k: &[u8]) -> Result<Option<T>, Error> {
    Ok(rx.raw(b, k).map_err(store_err)?.and_then(|raw| serde_json::from_slice(&raw).ok()))
}

/// Every decodable record under `prefix`, keys with the prefix cut, in
/// key order.
pub(crate) fn scan<T: DeserializeOwned>(rx: &impl Rx, b: Bucket, prefix: &[u8]) -> Result<Vec<(String, T)>, Error> {
    Ok(rx
        .pairs(b, prefix)
        .map_err(store_err)?
        .into_iter()
        .filter_map(|(k, v)| {
            let rest = String::from_utf8_lossy(&k[prefix.len()..]).into_owned();
            serde_json::from_slice(&v).ok().map(|t| (rest, t))
        })
        .collect())
}

pub(crate) fn store_err(e: StoreError) -> Error {
    Error::internal(e.to_string())
}

pub(crate) fn invalid(msg: impl Into<String>) -> Error {
    Error::invalid(msg)
}

pub(crate) fn not_found(msg: impl Into<String>) -> Error {
    Error::not_found(msg)
}

pub(crate) fn forbidden(msg: impl Into<String>) -> Error {
    Error::new(FORBIDDEN, msg)
}

pub(crate) fn pair(a: &str, b: &str) -> Vec<u8> {
    lain_store::join_key(a, b)
}

pub(crate) fn prefix(a: &str) -> Vec<u8> {
    lain_store::key_prefix(a)
}

/// Decodes an op's input; a shape that does not decode is
/// `invalid-message`.
pub(crate) fn input<T: DeserializeOwned>(v: Value, what: &str) -> Result<T, Error> {
    let v = if v.is_null() { Value::Object(Default::default()) } else { v };
    serde_json::from_value(v).map_err(|_| invalid(format!("{what} required")))
}

pub(crate) fn out<T: Serialize>(v: T) -> Result<Value, Error> {
    serde_json::to_value(v).map_err(|e| Error::internal(e.to_string()))
}

pub(crate) fn need_user(ids: &[&str]) -> Result<(), Error> {
    if ids.iter().any(|id| id.is_empty()) {
        return Err(invalid("user ids required"));
    }
    Ok(())
}

/// The libraries a read must leave out (D-156).
pub(crate) type Hidden = BTreeSet<String>;

/// Whether a work names an item of a hidden library.
pub(crate) fn hidden_work(hidden: &Hidden, w: &WorkRef) -> bool {
    !w.library_id.is_empty() && hidden.contains(&w.library_id)
}

/// The profile text rules (D-086): trimmed, no control characters (a
/// newline only in multi-line text), single-line text has its
/// whitespace collapsed, and the length in characters is bounded.
pub fn clean_text(s: &str, max: usize, field: &str, multiline: bool) -> Result<String, Error> {
    let s = s.trim();
    for c in s.chars() {
        if c == '\n' && multiline {
            continue;
        }
        if c.is_control() {
            return Err(invalid(format!("{field} must not contain control characters")));
        }
    }
    let s = if multiline { s.to_string() } else { s.split_whitespace().collect::<Vec<_>>().join(" ") };
    if s.chars().count() > max {
        return Err(invalid(format!("{field} is too long")));
    }
    Ok(s)
}

fn valid_kind(k: &str) -> bool {
    !k.is_empty() && k.len() <= 24 && k.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Validates a work reference and cleans its title. The item id is kept
/// only as a link hint and must look like a catalog id; the library id
/// travels with it (the server sets both from the catalog).
pub fn normalize_work(mut w: WorkRef) -> Result<WorkRef, Error> {
    w.kind = w.kind.trim().to_lowercase();
    if !valid_kind(&w.kind) {
        return Err(invalid("work kind must be a lowercase token"));
    }
    let t = clean_text(&w.title, MAX_TITLE, "work title", false)?;
    if t.is_empty() {
        return Err(invalid("work title required"));
    }
    w.title = t;
    if w.item_id.len() > 64 || w.item_id.contains(['\0', '/', '\\']) {
        w.item_id.clear();
    }
    if w.item_id.is_empty() || w.library_id.len() > 64 || w.library_id.contains(['\0', '/', '\\']) {
        w.library_id.clear();
    }
    Ok(w)
}

/// The title page's grouping key (D-056): lower case, whitespace
/// collapsed.
pub fn title_key(title: &str) -> String {
    title.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// A work's identity: kind plus the title key, so every episode,
/// chapter or volume of one title shares ratings and comments.
pub fn work_key(w: &WorkRef) -> String {
    format!("{}\0{}", w.kind, title_key(&w.title))
}

pub(crate) fn clamp_limit(n: i64) -> usize {
    if n <= 0 { DEFAULT_PAGE_LIMIT } else { (n as usize).min(MAX_PAGE_LIMIT) }
}

pub(crate) fn valid_visibility(v: &str) -> bool {
    matches!(v, "public" | "friends" | "private")
}

#[cfg(test)]
mod tests;

/// The default bindings: the one provider for all four capabilities.
pub fn bindings() -> Vec<(&'static str, lain_core::Binding)> {
    CAPABILITIES
        .iter()
        .map(|c| (*c, lain_core::Binding::new(lain_core::BindingMode::ExactlyOne, &[PROVIDER_ID])))
        .collect()
}
