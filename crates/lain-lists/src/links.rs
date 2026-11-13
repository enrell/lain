//! The gateway's side of linking (D-078..D-083): the operator's OAuth
//! client, the signed link state, the connector dispatch, imports
//! (on link, on demand and on a ~6 h ticker) and scrobbles. Every call
//! goes through the registry, so swaps and module suspend apply.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use lain_admin::integrations::IntegrationSettings;
use lain_core::{Error, Registry};
use lain_store::{Bucket, Store};

use lain_api::{ListEntryPatch, list_status};

use crate::link::{
    self, AuthorizeInput, AuthorizeOutput, ExchangeInput, FetchInput, FetchOutput, Identity, PushInput, PushOutput,
};
use crate::scrobble::{self, Finished};
use crate::store::op;
use crate::{
    CAP_INTEGRATION_SETTINGS, CAP_LIST_ACCOUNT, CAP_LIST_LINK, CAP_LIST_READ, CAP_LIST_WRITE, LinkPush, LinkedAccount,
    ListEntry, ListSyncStats, anilist, code, encode_query, settings,
};

/// The official Lain application for the auth-pin flow (D-083). Its
/// credentials are extractable by design, like every distributed OAuth
/// client's; what they buy is zero-setup linking, and the registered
/// redirect is AniList's own pin page, so it works for every
/// self-hosted origin.
pub const ANILIST_PIN_CLIENT_ID: &str = "52227";
pub const ANILIST_PIN_CLIENT_SECRET: &str = "bcKlonBogr6E6ovB4r0DNWxFQ7vz3XUeh32S4HEp";
pub const ANILIST_PIN_REDIRECT_URI: &str = "https://anilist.co/api/v2/oauth/pin";
pub const ANILIST_AUTHORIZE_URL: &str = "https://anilist.co/api/v2/oauth/authorize";

/// Link states expire after ten minutes.
pub const STATE_TTL_SECS: i64 = 600;
/// The scheduled import interval (D-081).
pub const SYNC_INTERVAL: Duration = Duration::from_secs(6 * 3600);

/// How long finished items gather before they are written: marking a
/// season watched lands as one write per entry (D-288).
pub const SCROBBLE_DEBOUNCE: Duration = Duration::from_millis(1500);

const STATE_KEY: &[u8] = b"oauth_state_key";

/// Finished items waiting for the scrobble worker, by user.
type Pending = Arc<(Mutex<Vec<(String, Finished)>>, Condvar)>;

/// Called with the user and the `list-changed` event data whenever a
/// link or its entries change.
pub type ChangeHook = Arc<dyn Fn(&str, Value) + Send + Sync>;

pub struct Links {
    registry: Arc<Registry>,
    state_key: Vec<u8>,
    /// One import at a time: a ticker never piles onto a manual sync,
    /// and a scrobble's local update never races a replace.
    sync_lock: Mutex<()>,
    /// One write to the platforms at a time: a scrobble plans against
    /// the copy the previous write left, and an edit never interleaves
    /// with one.
    push_lock: Mutex<()>,
    hook: Mutex<Option<ChangeHook>>,
    started: AtomicBool,
    stop: Arc<AtomicBool>,
    pending: Pending,
    worker: AtomicBool,
    debounce_ms: Arc<AtomicU64>,
}

fn now() -> i64 {
    lain_core::now_unix()
}

/// The wire code of an error: its own, or `sync-failed` for an untyped
/// failure.
pub fn err_code(e: &Error) -> &str {
    &e.code
}

/// Go's `err.Error()`: `code: msg` for a typed error, the bare message
/// for an untyped one.
pub fn err_text(e: &Error) -> String {
    if e.code == code::SYNC_FAILED { e.msg.clone() } else { e.to_string() }
}

/// Accepts the raw code, or the whole pin URL or fragment the user
/// copied: `code=` or `access_token=` anywhere in it wins.
pub fn code_from_paste(raw: &str) -> String {
    let raw = raw.trim();
    for key in ["code=", "access_token="] {
        let Some(i) = raw.find(key) else { continue };
        let name = &key[..key.len() - 1];
        let rest = raw[i..].trim_start_matches('#');
        for (k, v) in parse_query(rest) {
            if k == name && !v.is_empty() {
                return v;
            }
        }
    }
    raw.to_string()
}

fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex =
            b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
        match (b[i], hex) {
            (b'+', _) => out.push(b' '),
            (b'%', Some(x)) => {
                out.push(x);
                i += 2;
            }
            (c, _) => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Form-decoded `k=v` pairs of a query string.
pub fn parse_query(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (unescape(k), unescape(v))
        })
        .collect()
}

impl Links {
    /// Loads (or creates) the state signing key from the `meta` table.
    pub fn open(store: &Store, registry: Arc<Registry>) -> Result<Links, String> {
        let key = store
            .write(|tx| {
                if let Some(k) = tx.get(Bucket::Meta, STATE_KEY)? {
                    return Ok(k);
                }
                let mut k = vec![0u8; 32];
                lain_auth::crypto::random_bytes(&mut k).map_err(|e| lain_store::StoreError::Abort(e.to_string()))?;
                tx.put(Bucket::Meta, STATE_KEY, &k)?;
                Ok(k)
            })
            .map_err(|e| e.to_string())?;
        if key.len() < 32 {
            return Err("corrupt oauth state key".into());
        }
        Ok(Links {
            registry,
            state_key: key,
            sync_lock: Mutex::new(()),
            push_lock: Mutex::new(()),
            hook: Mutex::new(None),
            started: AtomicBool::new(false),
            stop: Arc::new(AtomicBool::new(false)),
            pending: Arc::new((Mutex::new(Vec::new()), Condvar::new())),
            worker: AtomicBool::new(false),
            debounce_ms: Arc::new(AtomicU64::new(SCROBBLE_DEBOUNCE.as_millis() as u64)),
        })
    }

    pub fn on_change(&self, hook: ChangeHook) {
        *self.hook.lock().unwrap_or_else(|e| e.into_inner()) = Some(hook);
    }

    fn changed(&self, user: &str, data: Value) {
        let hook = self.hook.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(h) = hook {
            h(user, data);
        }
    }

    // ---- operator settings ------------------------------------------------

    /// The operator's integrations document through its capability;
    /// empty when it cannot be read ("not configured").
    pub fn integrations(&self) -> IntegrationSettings {
        self.registry
            .call_one(CAP_INTEGRATION_SETTINGS, settings::op::GET, Value::Null)
            .ok()
            .and_then(|(v, _)| serde_json::from_value(v).ok())
            .unwrap_or_default()
    }

    pub fn save_integrations(&self, s: &IntegrationSettings) -> Result<(), Error> {
        self.registry.call_one(CAP_INTEGRATION_SETTINGS, settings::op::PUT, json!({"settings": s})).map(|_| ())
    }

    /// The operator-configured OAuth client of a platform; `None`
    /// without one (the connector may be bound, but there is no flow to
    /// start, D-080).
    pub fn client(&self, platform: &str) -> Option<(String, String)> {
        let s = self.integrations();
        match platform {
            anilist::PLATFORM if !s.anilist_client_id.is_empty() && !s.anilist_client_secret.is_empty() => {
                Some((s.anilist_client_id, s.anilist_client_secret))
            }
            _ => None,
        }
    }

    // ---- connectors -------------------------------------------------------

    /// Tries the bound connectors in order; one that declines the
    /// platform is skipped, and the last decline answers when nobody
    /// serves it.
    pub fn connector(&self, op: &str, input: Value) -> Result<Value, Error> {
        let (provs, _) = self.registry.ordered(CAP_LIST_LINK)?;
        let mut last = Error::new(code::UNSUPPORTED_PLATFORM, "no list connector installed");
        for p in provs {
            match p.invoke(CAP_LIST_LINK, op, input.clone()) {
                Err(e) if e.code == code::UNSUPPORTED_PLATFORM => last = e,
                other => return other,
            }
        }
        Err(last)
    }

    /// The authorize URL for the operator's client, carrying a signed
    /// state bound to the user and platform.
    pub fn authorize(
        &self,
        user_id: &str,
        platform: &str,
        client_id: &str,
        redirect_uri: &str,
    ) -> Result<String, Error> {
        let input = AuthorizeInput {
            platform: platform.into(),
            client_id: client_id.into(),
            redirect_uri: redirect_uri.into(),
            state: self.sign_state(user_id, platform),
        };
        let out = self.connector(link::AUTHORIZE, json!(input))?;
        let url = serde_json::from_value::<AuthorizeOutput>(out).map(|o| o.url).unwrap_or_default();
        if url.is_empty() {
            return Err(Error::internal("connector returned no url"));
        }
        Ok(url)
    }

    /// The pin flow's authorize URL (official app, AniList's pin page).
    pub fn pin_url(&self, platform: &str) -> Option<String> {
        (platform == anilist::PLATFORM).then(|| {
            let q = encode_query(&[
                ("client_id", ANILIST_PIN_CLIENT_ID),
                ("response_type", "code"),
                ("redirect_uri", ANILIST_PIN_REDIRECT_URI),
            ]);
            format!("{ANILIST_AUTHORIZE_URL}?{q}")
        })
    }

    /// Trades a code for the remote identity.
    pub fn exchange(&self, i: ExchangeInput) -> Result<Identity, Error> {
        let out = self.connector(link::EXCHANGE, json!(i))?;
        let id: Identity = serde_json::from_value(out).unwrap_or_default();
        if id.token.is_empty() {
            return Err(Error::internal("connector returned no identity"));
        }
        Ok(id)
    }

    /// The pin flow's exchange under the official app.
    pub fn exchange_pin(&self, platform: &str, pasted: &str) -> Result<Identity, Error> {
        self.exchange(ExchangeInput {
            platform: platform.into(),
            client_id: ANILIST_PIN_CLIENT_ID.into(),
            client_secret: ANILIST_PIN_CLIENT_SECRET.into(),
            code: pasted.into(),
            redirect_uri: ANILIST_PIN_REDIRECT_URI.into(),
        })
    }

    /// Stores a fresh link and runs the first import (remote is
    /// authoritative, D-081). A failed import still leaves a valid link:
    /// the error lands on the account and the next sync retries.
    pub fn link(&self, user_id: &str, platform: &str, id: Identity, via: &str) -> LinkedAccount {
        // A relink keeps the owner's choice; a first link updates the
        // platform as lain is used (D-288).
        let scrobble = self.account(user_id, platform).is_none_or(|old| old.scrobble);
        let a = LinkedAccount {
            scrobble,
            user_id: user_id.into(),
            platform: platform.into(),
            remote_user_id: id.remote_user_id,
            remote_username: id.remote_username,
            token: id.token,
            token_expires_at: id.token_expires_at,
            linked_at: now(),
            ..LinkedAccount::default()
        };
        self.put_account(&a);
        self.changed(user_id, json!({"platform": platform, "action": "linked"}));
        if let Err(e) = self.sync(a.clone()) {
            tracing::warn!(platform, user = user_id, code = %e.code, "first list import failed");
        }
        tracing::info!(platform, user = user_id, remote_user = %a.remote_username, via, "list platform linked");
        a
    }

    // ---- the store --------------------------------------------------------

    pub fn accounts(&self, user_id: &str) -> Result<Vec<LinkedAccount>, Error> {
        let (v, _) = self.registry.call_one(CAP_LIST_ACCOUNT, op::ACCOUNTS, json!({"user_id": user_id}))?;
        Ok(serde_json::from_value(v).unwrap_or_default())
    }

    pub fn account(&self, user_id: &str, platform: &str) -> Option<LinkedAccount> {
        let (v, _) = self
            .registry
            .call_one(CAP_LIST_ACCOUNT, op::GET, json!({"user_id": user_id, "platform": platform}))
            .ok()?;
        serde_json::from_value(v).ok()
    }

    pub fn put_account(&self, a: &LinkedAccount) {
        if let Err(e) = self.registry.call_one(CAP_LIST_ACCOUNT, op::PUT, json!({"account": a})) {
            tracing::warn!(platform = %a.platform, user = %a.user_id, err = %e, "list account write failed");
        }
    }

    /// Changes the scrobble preference; `None` when nothing is linked.
    pub fn set_scrobble(&self, user_id: &str, platform: &str, on: bool) -> Option<LinkedAccount> {
        let mut a = self.account(user_id, platform)?;
        a.scrobble = on;
        self.put_account(&a);
        self.changed(user_id, json!({"platform": platform, "action": "updated"}));
        Some(a)
    }

    /// Unlinks: the token and the platform's entries go (D-079).
    /// `Ok(None)` when nothing was linked; else the entries removed.
    pub fn unlink(&self, user_id: &str, platform: &str) -> Result<Option<i64>, Error> {
        let key = json!({"user_id": user_id, "platform": platform});
        let (v, _) = self.registry.call_one(CAP_LIST_ACCOUNT, op::DELETE, key.clone())?;
        if !v.as_bool().unwrap_or(false) {
            return Ok(None);
        }
        let removed = match self.registry.call_one(CAP_LIST_WRITE, op::DELETE_PLATFORM, key) {
            Ok((v, _)) => v.as_i64().unwrap_or(0),
            Err(e) => {
                tracing::warn!(platform, user = user_id, err = %e, "platform entries delete failed");
                0
            }
        };
        tracing::info!(platform, user = user_id, "list platform unlinked");
        self.changed(user_id, json!({"platform": platform, "action": "unlinked", "removed": removed}));
        Ok(Some(removed))
    }

    pub fn list(&self, user_id: &str, media_type: &str, status: &str) -> Result<Vec<ListEntry>, Error> {
        let (v, _) = self.registry.call_one(
            CAP_LIST_READ,
            op::LIST,
            json!({"user_id": user_id, "type": media_type, "status": status}),
        )?;
        Ok(serde_json::from_value(v).unwrap_or_default())
    }

    // ---- imports ----------------------------------------------------------

    /// Imports the remote list into the user's entries (a
    /// remote-authoritative replace) and records the sync on the
    /// account. Failures record a stable code, never a remote body.
    pub fn sync(&self, mut a: LinkedAccount) -> Result<ListSyncStats, Error> {
        let _one = self.sync_lock.lock().unwrap_or_else(|e| e.into_inner());
        if a.token_expired(now()) {
            self.mark_error(a.clone(), code::TOKEN_EXPIRED);
            return Err(Error::new(code::TOKEN_EXPIRED, format!("the {} token expired; link again", a.platform)));
        }
        let input = FetchInput {
            platform: a.platform.clone(),
            token: a.token.clone(),
            remote_user_id: a.remote_user_id.clone(),
        };
        let fetched: FetchOutput = match self.connector(link::FETCH, serde_json::to_value(&input).unwrap_or_default()) {
            Ok(v) => serde_json::from_value(v).map_err(|_| Error::internal("connector returned a bad list shape"))?,
            Err(e) => {
                self.mark_error(a, &e.code);
                return Err(e);
            }
        };
        let n = fetched.entries.len() as i64;
        let (v, _) = self.registry.call_one(
            CAP_LIST_WRITE,
            op::PUT_PLATFORM,
            json!({"user_id": a.user_id, "platform": a.platform, "entries": fetched.entries}),
        )?;
        let stats: ListSyncStats = serde_json::from_value(v).unwrap_or_default();
        // The stored account, not the one this sync started from: a
        // write recorded meanwhile (its last push) must not be lost.
        if let Some(cur) = self.account(&a.user_id, &a.platform) {
            a = cur;
        }
        a.last_sync_at = now();
        a.last_sync_error.clear();
        a.entry_count = n;
        self.put_account(&a);
        tracing::info!(platform = %a.platform, user = %a.user_id, upserted = stats.upserted, removed = stats.removed, "list synced");
        self.changed(
            &a.user_id,
            json!({"platform": a.platform, "action": "synced", "upserted": stats.upserted, "removed": stats.removed}),
        );
        Ok(stats)
    }

    fn mark_error(&self, mut a: LinkedAccount, code: &str) {
        if let Some(cur) = self.account(&a.user_id, &a.platform) {
            a = cur;
        }
        a.last_sync_error = code.into();
        self.put_account(&a);
        self.changed(&a.user_id, json!({"platform": a.platform, "action": "sync-failed", "error": code}));
    }

    /// Re-imports every linked account; one dead token never stalls the
    /// others.
    pub fn sync_all(&self) {
        let accounts = match self.accounts("") {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!(err = %e, "list accounts read failed");
                return;
            }
        };
        for a in accounts {
            let (platform, user) = (a.platform.clone(), a.user_id.clone());
            if let Err(e) = self.sync(a) {
                tracing::warn!(platform = %platform, user = %user, code = %e.code, "scheduled list sync failed");
            }
        }
    }

    /// The periodic import: a catch-up pass at once, then one per
    /// `interval` until [`Links::stop`]. A second call is a no-op.
    pub fn start(self: &Arc<Self>, interval: Duration) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let weak = Arc::downgrade(self);
        let stop = self.stop.clone();
        std::thread::Builder::new()
            .name("list-sync".into())
            .spawn(move || {
                let mut next = std::time::Instant::now();
                loop {
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    if std::time::Instant::now() >= next {
                        match weak.upgrade() {
                            Some(l) => l.sync_all(),
                            None => return,
                        }
                        next = std::time::Instant::now() + interval;
                    }
                    std::thread::sleep(Duration::from_millis(250).min(interval));
                }
            })
            .ok();
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    // ---- scrobble ---------------------------------------------------------

    /// Sets how long finished items gather before they are written.
    pub fn set_debounce(&self, d: Duration) {
        self.debounce_ms.store(d.as_millis() as u64, Ordering::SeqCst);
    }

    /// Queues a finished item for the scrobble worker, which writes
    /// after [`SCROBBLE_DEBOUNCE`] of quiet, off the request path, so a
    /// slow platform never delays playback reporting and a run of
    /// finishes is one write per entry.
    pub fn enqueue(self: &Arc<Self>, user_id: &str, item: Finished) {
        {
            let (q, cv) = &*self.pending;
            q.lock().unwrap_or_else(|e| e.into_inner()).push((user_id.to_string(), item));
            cv.notify_one();
        }
        if self.worker.swap(true, Ordering::SeqCst) {
            return;
        }
        let (weak, pending, stop, debounce) =
            (Arc::downgrade(self), self.pending.clone(), self.stop.clone(), self.debounce_ms.clone());
        let spawned = std::thread::Builder::new().name("scrobble".into()).spawn(move || {
            loop {
                let (q, cv) = &*pending;
                let mut guard = q.lock().unwrap_or_else(|e| e.into_inner());
                while guard.is_empty() {
                    if stop.load(Ordering::SeqCst) || weak.strong_count() == 0 {
                        return;
                    }
                    guard = cv.wait_timeout(guard, Duration::from_millis(250)).unwrap_or_else(|e| e.into_inner()).0;
                }
                // Quiet for the debounce: every finish that keeps coming
                // in (a season marked at once) joins this batch.
                let mut seen = guard.len();
                drop(guard);
                loop {
                    std::thread::sleep(Duration::from_millis(debounce.load(Ordering::SeqCst)));
                    let n = q.lock().unwrap_or_else(|e| e.into_inner()).len();
                    if n == seen || stop.load(Ordering::SeqCst) {
                        break;
                    }
                    seen = n;
                }
                let batch = std::mem::take(&mut *q.lock().unwrap_or_else(|e| e.into_inner()));
                let Some(links) = weak.upgrade() else { return };
                let mut users: Vec<String> = Vec::new();
                for (u, _) in &batch {
                    if !users.contains(u) {
                        users.push(u.clone());
                    }
                }
                for u in users {
                    let items: Vec<Finished> = batch.iter().filter(|(x, _)| *x == u).map(|(_, i)| i.clone()).collect();
                    links.scrobble_many(&u, &items);
                }
            }
        });
        if spawned.is_err() {
            self.worker.store(false, Ordering::SeqCst);
            tracing::warn!("scrobble worker did not start");
        }
    }

    /// Pushes a finished item to every account that opted in, now.
    pub fn scrobble(&self, user_id: &str, item: &Finished) {
        self.scrobble_many(user_id, std::slice::from_ref(item));
    }

    /// Pushes a run of finished items to every account that opted in:
    /// one write per entry they advance.
    pub fn scrobble_many(&self, user_id: &str, items: &[Finished]) {
        let accounts = match self.accounts(user_id) {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!(user = user_id, err = %e, "scrobble accounts failed");
                return;
            }
        };
        let _one = self.push_lock.lock().unwrap_or_else(|e| e.into_inner());
        for a in accounts.into_iter().filter(|a| a.scrobble && !a.token.is_empty()) {
            self.scrobble_to(&a, items);
        }
    }

    fn scrobble_to(&self, a: &LinkedAccount, items: &[Finished]) {
        let entries = match self.list(&a.user_id, "", "") {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(platform = %a.platform, err = %e, "scrobble list read failed");
                return;
            }
        };
        let decisions = scrobble::plan_many(items, &a.platform, &entries);
        if decisions.is_empty() {
            return;
        }
        if a.token_expired(now()) {
            tracing::warn!(platform = %a.platform, user = %a.user_id, "scrobble skipped: token expired");
            return;
        }
        for d in decisions {
            let push = PushInput {
                platform: a.platform.clone(),
                token: a.token.clone(),
                remote_id: d.remote_id.clone(),
                progress: d.progress,
                progress_volumes: d.progress_volumes,
                status: d.status.clone(),
                score: None,
            };
            let local = entries.iter().find(|e| e.platform == a.platform && e.remote_id == d.remote_id).cloned();
            let mut record = LinkPush {
                at: now(),
                remote_id: d.remote_id.clone(),
                title: d.title.clone(),
                progress: d.progress.or(d.progress_volumes).unwrap_or(0),
                status: d.status.clone(),
                via: "scrobble".into(),
                error: String::new(),
            };
            match self.push(&push) {
                Ok(out) => {
                    tracing::info!(platform = %a.platform, user = %a.user_id, progress = ?d.progress, status = %d.status, "scrobbled");
                    let mut e = local.unwrap_or_default();
                    scrobble::apply(&mut e, &d);
                    let saved = self.store_written(a, e, out);
                    self.record_push(a, record);
                    self.changed(
                        &a.user_id,
                        json!({"platform": a.platform, "action": "scrobbled", "remote_id": d.remote_id,
                               "progress": saved.as_ref().map_or(0, |e| e.progress), "status": d.status}),
                    );
                }
                Err(e) => {
                    tracing::warn!(platform = %a.platform, user = %a.user_id, code = %e.code, "scrobble failed");
                    record.error = e.code.clone();
                    if let Some(mut l) = local {
                        l.push_error = e.code.clone();
                        self.put_entry(a, l);
                    }
                    self.record_push(a, record);
                    self.changed(
                        &a.user_id,
                        json!({"platform": a.platform, "action": "push-failed", "remote_id": d.remote_id, "error": e.code}),
                    );
                }
            }
        }
    }

    fn push(&self, push: &PushInput) -> Result<PushOutput, Error> {
        let out = self.connector(link::PUSH, serde_json::to_value(push).unwrap_or_default())?;
        Ok(serde_json::from_value(out).unwrap_or_default())
    }

    /// Stores what a good write left: the platform's copy when the
    /// connector read one back, else the local copy with the change.
    fn store_written(&self, a: &LinkedAccount, local: ListEntry, out: PushOutput) -> Option<ListEntry> {
        let mut e = match out.entry {
            Some(mut remote) => {
                // The platform's answer is the truth; what it does not
                // carry (the write time) comes from the local copy.
                remote.pushed_at = local.pushed_at;
                remote
            }
            // A connector that answers only the two counts it wrote:
            // those are still the platform's word.
            None => {
                let mut l = local;
                if out.progress > 0 {
                    l.progress = out.progress;
                }
                if !out.status.is_empty() {
                    l.status = out.status;
                }
                l
            }
        };
        e.pushed_at = now();
        e.push_error.clear();
        self.put_entry(a, e)
    }

    fn put_entry(&self, a: &LinkedAccount, e: ListEntry) -> Option<ListEntry> {
        let input = json!({"user_id": a.user_id, "platform": a.platform, "entry": e});
        match self.registry.call_one(CAP_LIST_WRITE, op::PUT_ENTRY, input) {
            Ok((v, _)) => serde_json::from_value(v).ok(),
            Err(e) => {
                tracing::warn!(platform = %a.platform, err = %e, "list entry write failed");
                None
            }
        }
    }

    /// The account's "last write" line: what went out and whether it
    /// took (D-288).
    fn record_push(&self, a: &LinkedAccount, p: LinkPush) {
        let mut cur = self.account(&a.user_id, &a.platform).unwrap_or_else(|| a.clone());
        cur.last_push = Some(p);
        self.put_account(&cur);
    }

    // ---- edits from a title page (D-289) ---------------------------------

    /// Writes an edit (status, score, progress) to the platform, then
    /// to the local copy. `fallback` describes the work when the list
    /// does not hold it yet (a title added from its page) and the
    /// connector does not read the entry back.
    pub fn save_entry(
        &self,
        user_id: &str,
        platform: &str,
        remote_id: &str,
        patch: &ListEntryPatch,
        fallback: Option<ListEntry>,
    ) -> Result<ListEntry, Error> {
        let statuses = [
            list_status::CURRENT,
            list_status::PLANNING,
            list_status::COMPLETED,
            list_status::PAUSED,
            list_status::DROPPED,
            list_status::REPEATING,
        ];
        if let Some(s) = patch.status.as_deref()
            && !statuses.contains(&s)
        {
            return Err(Error::invalid(format!("status must be one of {}", statuses.join(", "))));
        }
        if patch.score.is_some_and(|s| !(0.0..=10.0).contains(&s) || s.is_nan()) {
            return Err(Error::invalid("score must be between 0 and 10"));
        }
        if patch.progress.is_some_and(|p| p < 0) || patch.progress_volumes.is_some_and(|p| p < 0) {
            return Err(Error::invalid("progress must not be negative"));
        }
        if patch.status.is_none()
            && patch.score.is_none()
            && patch.progress.is_none()
            && patch.progress_volumes.is_none()
        {
            return Err(Error::invalid("status, score, progress or progress_volumes required"));
        }
        let a =
            self.account(user_id, platform).ok_or_else(|| Error::not_found(format!("no {platform} account linked")))?;
        if a.token_expired(now()) {
            return Err(Error::new(code::TOKEN_EXPIRED, format!("the {platform} token expired; link again")));
        }
        let _one = self.push_lock.lock().unwrap_or_else(|e| e.into_inner());
        let local = self
            .list(user_id, "", "")?
            .into_iter()
            .find(|e| e.platform == platform && e.remote_id == remote_id)
            .or(fallback.map(|f| ListEntry { remote_id: remote_id.into(), ..f }));
        let push = PushInput {
            platform: platform.into(),
            token: a.token.clone(),
            remote_id: remote_id.into(),
            progress: patch.progress,
            progress_volumes: patch.progress_volumes,
            status: patch.status.clone().unwrap_or_default(),
            score: patch.score,
        };
        let title = local.as_ref().map(|e| e.title.clone()).unwrap_or_default();
        let mut record = LinkPush {
            at: now(),
            remote_id: remote_id.into(),
            title,
            progress: patch.progress.unwrap_or(0),
            status: patch.status.clone().unwrap_or_default(),
            via: "edit".into(),
            error: String::new(),
        };
        let out = match self.push(&push) {
            Ok(out) => out,
            Err(e) => {
                record.error = e.code.clone();
                self.record_push(&a, record);
                return Err(e);
            }
        };
        if out.entry.is_none() && local.as_ref().is_none_or(|l| l.title.trim().is_empty()) {
            self.record_push(&a, record);
            return Err(Error::internal("the platform saved the entry but lain cannot describe it; sync to see it"));
        }
        let mut e = local.unwrap_or_default();
        if let Some(s) = &patch.status {
            e.status = s.clone();
            // A completed entry is complete on the platform too.
            if s == list_status::COMPLETED && patch.progress.is_none() && e.progress_total > 0 {
                e.progress = e.progress_total;
            }
        }
        if let Some(p) = patch.progress {
            e.progress = p;
        }
        if let Some(v) = patch.progress_volumes {
            e.progress_volumes = v;
        }
        if let Some(sc) = patch.score {
            e.score = sc;
        }
        if e.media_type.is_empty() {
            e.media_type = lain_api::list_media::ANIME.into();
        }
        let saved =
            self.store_written(&a, e, out).ok_or_else(|| Error::internal("the list entry could not be stored"))?;
        if record.title.is_empty() {
            record.title = saved.title.clone();
        }
        record.progress = saved.progress;
        record.status = saved.status.clone();
        self.record_push(&a, record);
        self.changed(
            user_id,
            json!({"platform": platform, "action": "saved", "remote_id": remote_id, "progress": saved.progress, "status": saved.status}),
        );
        Ok(saved)
    }

    // ---- the OAuth state (HMAC-signed, expiring, bound to user and platform)

    /// `base64url(json).base64url(hmac)`, valid for ten minutes.
    pub fn sign_state(&self, user_id: &str, platform: &str) -> String {
        self.sign_state_at(user_id, platform, now() + STATE_TTL_SECS)
    }

    pub fn sign_state_at(&self, user_id: &str, platform: &str, exp: i64) -> String {
        let mut nonce = [0u8; 12];
        let _ = lain_auth::crypto::random_bytes(&mut nonce);
        let raw = json!({"uid": user_id, "p": platform, "exp": exp, "n": lain_auth::crypto::b64_url(&nonce)});
        let body = lain_auth::crypto::b64_url(raw.to_string().as_bytes());
        let mac = lain_auth::crypto::hmac_sha256(&self.state_key, body.as_bytes());
        format!("{body}.{}", lain_auth::crypto::b64_url(&mac))
    }

    /// The user a state was signed for, if it is intact, unexpired and
    /// for this platform.
    pub fn verify_state(&self, state: &str, platform: &str) -> Option<String> {
        let (body, sig) = state.split_once('.')?;
        let tag = lain_auth::crypto::b64_url_decode(sig)?;
        if !lain_auth::crypto::hmac_sha256_verify(&self.state_key, body.as_bytes(), &tag) {
            return None;
        }
        let raw = lain_auth::crypto::b64_url_decode(body)?;
        let st: Value = serde_json::from_slice(&raw).ok()?;
        let uid = st["uid"].as_str().unwrap_or("");
        if st["p"].as_str() != Some(platform) || uid.is_empty() || now() > st["exp"].as_i64().unwrap_or(0) {
            return None;
        }
        Some(uid.to_string())
    }
}
