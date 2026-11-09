//! The gateway's side of linking (D-078..D-083): the operator's OAuth
//! client, the signed link state, the connector dispatch, imports
//! (on link, on demand and on a ~6 h ticker) and scrobbles. Every call
//! goes through the registry, so swaps and module suspend apply.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use lain_admin::integrations::IntegrationSettings;
use lain_core::{Error, Registry};
use lain_store::{Bucket, Store};

use crate::link::{self, AuthorizeInput, AuthorizeOutput, ExchangeInput, FetchInput, FetchOutput, Identity, PushInput};
use crate::scrobble::{self, Finished};
use crate::store::op;
use crate::{
    CAP_INTEGRATION_SETTINGS, CAP_LIST_ACCOUNT, CAP_LIST_LINK, CAP_LIST_READ, CAP_LIST_WRITE, LinkedAccount, ListEntry,
    ListSyncStats, anilist, code, encode_query, settings,
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

const STATE_KEY: &[u8] = b"oauth_state_key";

/// Called with the user and the `list-changed` event data whenever a
/// link or its entries change.
pub type ChangeHook = Arc<dyn Fn(&str, Value) + Send + Sync>;

pub struct Links {
    registry: Arc<Registry>,
    state_key: Vec<u8>,
    /// One import at a time: a ticker never piles onto a manual sync,
    /// and a scrobble's local update never races a replace.
    sync_lock: Mutex<()>,
    hook: Mutex<Option<ChangeHook>>,
    started: AtomicBool,
    stop: Arc<AtomicBool>,
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
            hook: Mutex::new(None),
            started: AtomicBool::new(false),
            stop: Arc::new(AtomicBool::new(false)),
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
        let a = LinkedAccount {
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

    /// Pushes a finished episode to every account that opted in. Runs
    /// synchronously; the caller runs it off the request path so a slow
    /// platform never delays playback reporting.
    pub fn scrobble(&self, user_id: &str, item: &Finished) {
        let accounts = match self.accounts(user_id) {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!(user = user_id, err = %e, "scrobble accounts failed");
                return;
            }
        };
        for a in accounts.into_iter().filter(|a| a.scrobble && !a.token.is_empty()) {
            self.scrobble_to(a, item);
        }
    }

    fn scrobble_to(&self, a: LinkedAccount, item: &Finished) {
        let entries = match self.list(&a.user_id, "anime", "") {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(platform = %a.platform, err = %e, "scrobble list read failed");
                return;
            }
        };
        let Some(d) = scrobble::plan(item, &a.platform, &entries) else { return };
        if a.token_expired(now()) {
            tracing::warn!(platform = %a.platform, user = %a.user_id, "scrobble skipped: token expired");
            return;
        }
        let push = PushInput {
            platform: a.platform.clone(),
            token: a.token.clone(),
            remote_id: d.remote_id.clone(),
            progress: d.progress,
            status: d.status.clone(),
        };
        if let Err(e) = self.connector(link::PUSH, serde_json::to_value(&push).unwrap_or_default()) {
            tracing::warn!(platform = %a.platform, user = %a.user_id, code = %e.code, "scrobble failed");
            return;
        }
        tracing::info!(platform = %a.platform, user = %a.user_id, progress = d.progress, status = %d.status, "scrobbled");
        self.update_local(&a, &d);
    }

    /// Keeps the stored copy in step with the platform, so the next
    /// finished episode compares against it without waiting for a sync.
    fn update_local(&self, a: &LinkedAccount, d: &scrobble::Decision) {
        let _one = self.sync_lock.lock().unwrap_or_else(|e| e.into_inner());
        let all = match self.list(&a.user_id, "", "") {
            Ok(all) => all,
            Err(e) => {
                tracing::warn!(err = %e, "scrobble local update failed");
                return;
            }
        };
        let mine: Vec<ListEntry> = all
            .into_iter()
            .filter(|e| e.platform == a.platform)
            .map(|mut e| {
                if e.remote_id == d.remote_id {
                    e.progress = d.progress;
                    e.status = d.status.clone();
                }
                e
            })
            .collect();
        let input = json!({"user_id": a.user_id, "platform": a.platform, "entries": mine});
        match self.registry.call_one(CAP_LIST_WRITE, op::PUT_PLATFORM, input) {
            Ok(_) => self.changed(
                &a.user_id,
                json!({"platform": a.platform, "action": "scrobbled", "remote_id": d.remote_id, "progress": d.progress, "status": d.status}),
            ),
            Err(e) => tracing::warn!(err = %e, "scrobble local update failed"),
        }
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
