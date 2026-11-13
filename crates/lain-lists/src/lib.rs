//! The lists module (D-078..D-083): the per-user tracking list, linked
//! external accounts, the platform connectors behind `lain.listlink@1`,
//! scheduled imports and scrobbling.
//!
//! The split is Go's: the list store (`lain-list-redb`) owns entries
//! and accounts; connectors (`lain-listlink-anilist`) own remote calls
//! and never touch the inbound HTTP; [`Links`] is the gateway's logic
//! (OAuth state, imports, scrobbles) and goes through the registry, so
//! swaps and module suspend apply. Tokens are per-user credentials that
//! never leave the server or reach a log line (D-079, D-026).
//!
//! Outbound HTTP is the shared `lain-http` client (D-200); tests point
//! the connector at a loopback fixture server.

pub mod anilist;
pub mod links;
pub mod scrobble;
pub mod settings;
pub mod store;

use serde::{Deserialize, Serialize};

pub use lain_api::{Link, LinkPush, ListEntry, ListSyncStats};

pub use anilist::AniList;
pub use links::Links;
pub use settings::IntegrationsProvider;
pub use store::ListStore;

pub const CAP_LIST_READ: &str = "lain.list.read@1";
pub const CAP_LIST_WRITE: &str = "lain.list.write@1";
pub const CAP_LIST_ACCOUNT: &str = "lain.list.account@1";
/// Platform connectors, ordered-many: a provider that does not serve a
/// platform declines with `unsupported-platform` and the next is tried.
pub const CAP_LIST_LINK: &str = "lain.listlink@1";
/// The operator's external-platform credentials (D-080, D-198).
pub const CAP_INTEGRATION_SETTINGS: &str = "lain.settings.integrations@1";

/// Codes with meaning to the gateway.
pub mod code {
    pub const UNSUPPORTED_PLATFORM: &str = "unsupported-platform";
    pub const TOKEN_INVALID: &str = "token-invalid";
    pub const TOKEN_EXPIRED: &str = "token-expired";
    pub const INVALID_GRANT: &str = "invalid-grant";
    pub const NOT_CONFIGURED: &str = "not-configured";
    /// The platform asked us to slow down (HTTP 429 after the shared
    /// client's own retries).
    pub const RATE_LIMITED: &str = "rate-limited";
    /// A failure with no typed code (network, a remote's odd answer):
    /// Go's untyped error, shown by its message alone.
    pub const SYNC_FAILED: &str = "sync-failed";
}

/// A user's connection to an external list platform (D-079). The token
/// is a credential: it is stored, and [`LinkedAccount::view`] drops it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedAccount {
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub remote_user_id: String,
    #[serde(default)]
    pub remote_username: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub token_expires_at: i64,
    #[serde(default)]
    pub linked_at: i64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub last_sync_at: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_sync_error: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub entry_count: i64,
    /// Pushes finished episodes and chapters back to the platform. A
    /// new link turns it on (D-288: linking is the consent, and a link
    /// that writes nothing gave the owner nothing); stored accounts keep
    /// what they had.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub scrobble: bool,
    /// The last write lain made to the platform, good or failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_push: Option<LinkPush>,
}

fn is_zero(v: &i64) -> bool {
    *v == 0
}

impl LinkedAccount {
    /// Whether the credential is past its remote lifetime; zero means
    /// the platform reported none.
    pub fn token_expired(&self, now: i64) -> bool {
        self.token_expires_at > 0 && now >= self.token_expires_at
    }

    /// The client view: identity and sync state, never the token.
    pub fn view(&self, now: i64) -> Link {
        Link {
            platform: self.platform.clone(),
            remote_user_id: self.remote_user_id.clone(),
            remote_username: self.remote_username.clone(),
            linked_at: self.linked_at,
            last_sync_at: self.last_sync_at,
            last_sync_error: self.last_sync_error.clone(),
            entry_count: self.entry_count,
            token_expires_at: self.token_expires_at,
            token_expired: self.token_expired(now),
            scrobble: self.scrobble,
            last_push: self.last_push.clone(),
        }
    }
}

/// The connector inputs and outputs (`lain.listlink@1`). Secrets travel
/// in the inputs: connectors hold no operator state (D-006, D-080).
pub mod link {
    use serde::{Deserialize, Serialize};

    use crate::ListEntry;

    /// Op names of `lain.listlink@1`.
    pub const AUTHORIZE: &str = "authorize";
    pub const EXCHANGE: &str = "exchange";
    pub const FETCH: &str = "fetch";
    pub const PUSH: &str = "push";

    #[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct AuthorizeInput {
        #[serde(default)]
        pub platform: String,
        #[serde(default)]
        pub client_id: String,
        #[serde(default)]
        pub redirect_uri: String,
        #[serde(default)]
        pub state: String,
    }

    #[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct AuthorizeOutput {
        #[serde(default)]
        pub url: String,
    }

    #[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ExchangeInput {
        #[serde(default)]
        pub platform: String,
        #[serde(default)]
        pub client_id: String,
        #[serde(default)]
        pub client_secret: String,
        #[serde(default)]
        pub code: String,
        #[serde(default)]
        pub redirect_uri: String,
    }

    impl std::fmt::Debug for ExchangeInput {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("ExchangeInput")
                .field("platform", &self.platform)
                .field("client_id", &self.client_id)
                .field("redirect_uri", &self.redirect_uri)
                .finish_non_exhaustive()
        }
    }

    /// The remote account an exchange produced.
    #[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Identity {
        #[serde(default)]
        pub token: String,
        #[serde(default, skip_serializing_if = "crate::is_zero")]
        pub token_expires_at: i64,
        #[serde(default)]
        pub remote_user_id: String,
        #[serde(default)]
        pub remote_username: String,
    }

    impl std::fmt::Debug for Identity {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Identity")
                .field("remote_user_id", &self.remote_user_id)
                .field("remote_username", &self.remote_username)
                .finish_non_exhaustive()
        }
    }

    #[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct FetchInput {
        #[serde(default)]
        pub platform: String,
        #[serde(default)]
        pub token: String,
        #[serde(default)]
        pub remote_user_id: String,
    }

    #[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
    pub struct FetchOutput {
        #[serde(default)]
        pub entries: Vec<ListEntry>,
    }

    /// One entry written back (a scrobble or an edit). Every field but
    /// the identity is optional: an absent one (or an empty status)
    /// leaves the remote value alone. `progress` was required before
    /// D-289; old callers always send it, so they read as before.
    #[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
    pub struct PushInput {
        #[serde(default)]
        pub platform: String,
        #[serde(default)]
        pub token: String,
        #[serde(default)]
        pub remote_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub progress: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub progress_volumes: Option<i64>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        pub status: String,
        /// 0..10; 0 clears the score.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub score: Option<f64>,
    }

    impl std::fmt::Debug for PushInput {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("PushInput")
                .field("platform", &self.platform)
                .field("remote_id", &self.remote_id)
                .field("progress", &self.progress)
                .field("progress_volumes", &self.progress_volumes)
                .field("status", &self.status)
                .field("score", &self.score)
                .finish_non_exhaustive()
        }
    }

    #[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
    pub struct PushOutput {
        #[serde(default)]
        pub progress: i64,
        #[serde(default)]
        pub status: String,
        /// The whole entry as the platform saved it, when the connector
        /// reads it back (title, cover and totals for an entry the list
        /// did not hold yet).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub entry: Option<ListEntry>,
    }
}

/// The title page's grouping key (D-056).
pub fn title_key(title: &str) -> String {
    title.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// The key a list entry and a library title are matched by (D-288):
/// letters and digits in lower case, every run of anything else one
/// space, so "Frieren: Beyond Journey's End" and "frieren beyond
/// journey s end" agree while different words never do.
pub fn match_key(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut gap = false;
    for c in title.chars() {
        if c.is_alphanumeric() {
            if gap && !out.is_empty() {
                out.push(' ');
            }
            gap = false;
            out.extend(c.to_lowercase());
        } else {
            gap = true;
        }
    }
    out
}

/// Go's `url.QueryEscape`: letters, digits and `-_.~` stay, a space is
/// `+`, every other byte is `%XX`.
pub fn query_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Go's `url.Values.Encode`: pairs sorted by key, escaped.
pub fn encode_query(pairs: &[(&str, &str)]) -> String {
    let mut p = pairs.to_vec();
    p.sort_by(|a, b| a.0.cmp(b.0));
    p.iter().map(|(k, v)| format!("{}={}", query_escape(k), query_escape(v))).collect::<Vec<_>>().join("&")
}

/// The default bindings: the list store for the three list
/// capabilities, the integrations settings, and the AniList connector
/// as the first (only) `lain.listlink@1` provider.
pub fn bindings() -> Vec<(&'static str, lain_core::Binding)> {
    use lain_core::{Binding, BindingMode};
    let one = |id: &str| Binding::new(BindingMode::ExactlyOne, &[id]);
    vec![
        (CAP_LIST_READ, one(store::PROVIDER_ID)),
        (CAP_LIST_WRITE, one(store::PROVIDER_ID)),
        (CAP_LIST_ACCOUNT, one(store::PROVIDER_ID)),
        (CAP_INTEGRATION_SETTINGS, one(settings::PROVIDER_ID)),
        (CAP_LIST_LINK, Binding::new(BindingMode::OrderedMany, &[anilist::PROVIDER_ID])),
    ]
}

/// Registers the module's in-process providers.
pub fn register(
    registry: &lain_core::Registry,
    store: std::sync::Arc<lain_store::Store>,
    endpoints: anilist::Endpoints,
    http: lain_http::Client,
) {
    use std::sync::Arc;
    registry.register(Arc::new(ListStore::new(store.clone())));
    registry.register(Arc::new(IntegrationsProvider::new(store)));
    registry.register(Arc::new(AniList::new(endpoints, http)));
}

#[cfg(test)]
mod tests {
    #[test]
    fn match_keys() {
        assert_eq!(super::match_key("  Frieren: Beyond Journey's End "), "frieren beyond journey s end");
        assert_eq!(super::match_key("Re:ZERO"), "re zero");
        assert_eq!(super::match_key("葬送のフリーレン"), "葬送のフリーレン");
        assert_eq!(super::match_key("!!"), "");
    }
}
