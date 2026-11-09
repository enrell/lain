//! Operator credentials for external platforms (D-080, D-198): the
//! AniList OAuth client the lists module will use to link accounts.
//! They live in the store's `meta` table under `integrations`; the
//! secret is never returned to a client, only whether one is stored.
//! The lists module serves this document as
//! `lain.settings.integrations@1` (`lain-settings-integrations`, D-215).

use serde::{Deserialize, Serialize};

use lain_store::{Bucket, Store, StoreError};

pub use lain_api::{IntegrationPlatform, IntegrationsUpdate, IntegrationsView};

const KEY: &[u8] = b"integrations";

/// The stored document (Go's `contracts.IntegrationSettings`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrationSettings {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub anilist_client_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub anilist_client_secret: String,
}

/// The stored settings; absent or unreadable reads as empty ("not
/// configured"), logged when unreadable.
pub fn get(store: &Store) -> IntegrationSettings {
    match store.read(|tx| tx.get_json::<IntegrationSettings>(Bucket::Meta, KEY)) {
        Ok(v) => v.unwrap_or_default(),
        Err(e) => {
            tracing::warn!(err = %e, "integrations settings unreadable; serving none");
            IntegrationSettings::default()
        }
    }
}

pub fn put(store: &Store, s: &IntegrationSettings) -> Result<(), StoreError> {
    store.write(|tx| tx.put_json(Bucket::Meta, KEY, s))
}

/// Seeds the settings only when no document exists (environment
/// provisioning on first boot); an operator's save, even an empty one,
/// is never overwritten. Returns whether it wrote.
pub fn ensure(store: &Store, base: &IntegrationSettings) -> Result<bool, StoreError> {
    store.write(|tx| {
        if tx.get(Bucket::Meta, KEY)?.is_some() {
            return Ok(false);
        }
        tx.put_json(Bucket::Meta, KEY, base)?;
        Ok(true)
    })
}

/// The first-boot seed from `LAIN_ANILIST_CLIENT_ID` and
/// `LAIN_ANILIST_CLIENT_SECRET`, if either is set.
pub fn from_env() -> Option<IntegrationSettings> {
    let var = |k: &str| std::env::var(k).map(|v| v.trim().to_string()).unwrap_or_default();
    let s = IntegrationSettings {
        anilist_client_id: var("LAIN_ANILIST_CLIENT_ID"),
        anilist_client_secret: var("LAIN_ANILIST_CLIENT_SECRET"),
    };
    (s != IntegrationSettings::default()).then_some(s)
}

/// Applies an update: absent fields keep their value, values are
/// trimmed, and an empty secret keeps the stored one (the admin UI
/// sends it only when it changes).
pub fn apply(mut cur: IntegrationSettings, u: &IntegrationsUpdate) -> IntegrationSettings {
    if let Some(id) = &u.anilist_client_id {
        cur.anilist_client_id = id.trim().to_string();
    }
    if let Some(secret) = u.anilist_client_secret.as_deref().filter(|s| !s.is_empty()) {
        cur.anilist_client_secret = secret.trim().to_string();
    }
    cur
}

/// The client view: per platform the client id, whether a secret is
/// stored and the OAuth callback under `base_url` (`scheme://host`).
pub fn view(s: &IntegrationSettings, base_url: &str) -> IntegrationsView {
    let mut v = IntegrationsView::default();
    v.platforms.insert(
        "anilist".into(),
        IntegrationPlatform {
            client_id: s.anilist_client_id.clone(),
            secret_set: !s.anilist_client_secret.is_empty(),
            callback_url: format!("{base_url}/api/auth/anilist/callback"),
        },
    );
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_merge_and_view() {
        let dir = lain_store::test_dir("integrations");
        let store = Store::open(&dir).unwrap();
        assert_eq!(get(&store), IntegrationSettings::default());
        let seed = IntegrationSettings { anilist_client_id: "env-id".into(), anilist_client_secret: "env-s".into() };
        assert!(ensure(&store, &seed).unwrap());
        assert!(!ensure(&store, &IntegrationSettings::default()).unwrap(), "a saved document is never reseeded");
        let cur = apply(
            get(&store),
            &IntegrationsUpdate {
                anilist_client_id: Some(" id-1 ".into()),
                anilist_client_secret: Some(String::new()),
            },
        );
        assert_eq!((cur.anilist_client_id.as_str(), cur.anilist_client_secret.as_str()), ("id-1", "env-s"));
        put(&store, &cur).unwrap();
        let v = view(&get(&store), "http://h:1");
        let a = &v.platforms["anilist"];
        assert_eq!((a.client_id.as_str(), a.secret_set), ("id-1", true));
        assert_eq!(a.callback_url, "http://h:1/api/auth/anilist/callback");
        assert!(!serde_json::to_string(&v).unwrap().contains("env-s"), "the secret never leaves");
        // An explicit empty save stays empty across a reseed.
        put(&store, &IntegrationSettings::default()).unwrap();
        assert!(!ensure(&store, &seed).unwrap());
        assert_eq!(get(&store), IntegrationSettings::default());
        drop(store);
        std::fs::remove_dir_all(dir).ok();
    }
}
