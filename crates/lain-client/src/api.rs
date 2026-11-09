//! The typed API client. One method per route a client uses: the
//! conformance cases (`conformance/cases/`) plus the desktop parity
//! routes (DESIGN-v2 §8.4). Answers decode into the `lain-api` types
//! the server itself serializes (D-179).

use std::sync::{Arc, RwLock};
use std::time::Duration;

use hyper::Method;
use hyper::body::Bytes;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;

use lain_api::{
    ActivityPage, Changed, CollectionItemInput, CollectionPatch, CollectionReply, CollectionShareInput, Collections,
    Comment, CommentInput, Friends, Link, LinkPatch, LinkReply, LinkSyncReply, LinkUrl, Links, ListEntries, ListEntry,
    ListSyncStats, MarkRead, NewCollection, Notifications, Rating, RatingInput, Ratings, RelateReply, Removed,
    ShareInput, ShareReply, SocialProfile, SocialSettings, SocialSettingsPatch, UnlinkReply, UserSearch,
    UserWithRelation, WorkRef, WorkView,
};
use lain_api::{
    AdminTranscodes, BrowseResult, CatalogItem, CatalogPage, ComicPages, Enrichment, Enrichments, Episodes, Health,
    IntegrationsUpdate, IntegrationsView, Library, LibraryDeleted, LibraryPatch, LocalPlayers, LoginReply, Mascots,
    ModulesView, NewLibrary, Palette, PinState, PlayLocalReply, PlayLocalRequest, PlaybackOptions, PlaybackPlan,
    PluginsView, ProfilePatch, Progress, ProgressUpdate, RootsView, ScanStatus, SetupStatus, Sidecar, Sidecars,
    StatusReply, SwapReply, SwapRequest, TranscodeSelection, TranscodeSettings, TranscodeSettingsView, TranscodeStatus,
    User, UserPatch, WithdrawReply,
};

use crate::error::ClientError;
use crate::http::{self, ServerUrl, Streaming, encode};
use crate::token::{Session, TokenStore};

/// How a client talks to one server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConfig {
    /// The server URL (see [`ServerUrl::parse`]).
    pub server: String,
    /// The limit for one whole request (connect, send, read).
    pub timeout_ms: u64,
}

impl ClientConfig {
    pub fn new(server: &str) -> Self {
        ClientConfig { server: server.to_string(), timeout_ms: 30_000 }
    }
}

/// Paging of `GET /api/catalog`. Zero values are left to the server
/// (limit 50, cap 500; sort `title`, or `recent`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CatalogQuery {
    pub limit: i64,
    pub offset: i64,
    pub sort: String,
    pub library_id: String,
}

/// `GET /api/search`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchQuery {
    pub q: String,
    pub kind: String,
    pub limit: i64,
    pub offset: i64,
    pub sort: String,
}

/// How to unlock a private library for this session (D-156).
#[derive(Clone, PartialEq, Eq)]
pub enum Unlock {
    Pin(String),
    Password(String),
}

impl std::fmt::Debug for Unlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Unlock::Pin(_) => "Unlock::Pin(<redacted>)",
            Unlock::Password(_) => "Unlock::Password(<redacted>)",
        })
    }
}

struct Inner {
    url: ServerUrl,
    server: String,
    tokens: Arc<dyn TokenStore>,
    session: RwLock<Option<Session>>,
    timeout: Duration,
}

/// A client of one server. Cheap to clone; clones share the session.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

pub(crate) enum Body {
    None,
    Json(Vec<u8>),
    Raw(Vec<u8>, String),
}

pub(crate) fn query(pairs: &[(&str, String)]) -> String {
    let parts: Vec<String> =
        pairs.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| format!("{k}={}", encode(v))).collect();
    if parts.is_empty() { String::new() } else { format!("?{}", parts.join("&")) }
}

/// A number for [`query`]: zero means "the server's default".
fn num(n: i64) -> String {
    if n == 0 { String::new() } else { n.to_string() }
}

fn decode<T: DeserializeOwned>(raw: &[u8]) -> Result<T, ClientError> {
    serde_json::from_slice(raw).map_err(|e| ClientError::Decode { message: e.to_string() })
}

pub(crate) fn json_body(v: &impl Serialize) -> Body {
    Body::Json(serde_json::to_vec(v).unwrap_or_else(|_| b"{}".to_vec()))
}

impl Client {
    /// A client of `config.server`, with the session `tokens` holds for
    /// it, if any.
    pub fn new(config: ClientConfig, tokens: Arc<dyn TokenStore>) -> Result<Client, ClientError> {
        let url = ServerUrl::parse(&config.server)?;
        let server = url.canonical();
        let session = tokens.load(&server)?;
        Ok(Client {
            inner: Arc::new(Inner {
                url,
                server,
                tokens,
                session: RwLock::new(session),
                timeout: Duration::from_millis(config.timeout_ms.max(1)),
            }),
        })
    }

    /// The canonical server URL; it keys every local record.
    pub fn server(&self) -> String {
        self.inner.server.clone()
    }

    /// The current login, if any.
    pub fn session(&self) -> Option<Session> {
        self.inner.session.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn token(&self) -> Option<String> {
        self.inner.session.read().unwrap_or_else(|e| e.into_inner()).as_ref().map(|s| s.token.clone())
    }

    /// Re-reads the session from the token store (another part of the
    /// app logged in again). Returns whether the token changed.
    pub fn reload_session(&self) -> Result<bool, ClientError> {
        let fresh = self.inner.tokens.load(&self.inner.server)?;
        let mut cur = self.inner.session.write().unwrap_or_else(|e| e.into_inner());
        let changed = cur.as_ref().map(|s| &s.token) != fresh.as_ref().map(|s| &s.token);
        *cur = fresh;
        Ok(changed)
    }

    pub(crate) fn timeout(&self) -> Duration {
        self.inner.timeout
    }

    fn auth_headers(&self, auth: bool) -> Result<Vec<(&'static str, String)>, ClientError> {
        if !auth {
            return Ok(Vec::new());
        }
        let t = self.token().ok_or(ClientError::Unauthorized)?;
        Ok(vec![("authorization", format!("Bearer {t}"))])
    }

    pub(crate) async fn raw(&self, method: Method, path: &str, body: Body, auth: bool) -> Result<Bytes, ClientError> {
        let mut headers = self.auth_headers(auth)?;
        headers.push(("accept", "application/json".into()));
        let (payload, ctype);
        let body = match body {
            Body::None => None,
            Body::Json(b) => {
                payload = Bytes::from(b);
                Some((payload, "application/json"))
            }
            Body::Raw(b, t) => {
                payload = Bytes::from(b);
                ctype = t;
                Some((payload, ctype.as_str()))
            }
        };
        let route = format!("{method} {}", path.split('?').next().unwrap_or(path));
        let r = http::send(&self.inner.url, method, path, &headers, body, self.inner.timeout).await?;
        if !(200..300).contains(&r.status) {
            return Err(ClientError::from_response(r.status, &route, &r.body));
        }
        Ok(r.body)
    }

    pub(crate) async fn call<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Body,
    ) -> Result<T, ClientError> {
        decode(&self.raw(method, path, body, true).await?)
    }

    /// Opens an authenticated request whose body the caller streams.
    pub(crate) async fn open(&self, path: &str, extra: &[(&'static str, String)]) -> Result<Streaming, ClientError> {
        let mut headers = self.auth_headers(true)?;
        headers.extend(extra.iter().cloned());
        http::open(&self.inner.url, Method::GET, path, &headers, None).await
    }

    // ---- liveness, setup and login ------------------------------------

    /// `GET /health` (public): probes a discovered server.
    pub async fn health(&self) -> Result<Health, ClientError> {
        decode(&self.raw(Method::GET, "/health", Body::None, false).await?)
    }

    pub async fn setup_status(&self) -> Result<SetupStatus, ClientError> {
        decode(&self.raw(Method::GET, "/api/setup/status", Body::None, false).await?)
    }

    /// Creates the first admin on a fresh server. It does not log in.
    pub async fn setup(&self, username: &str, password: &str) -> Result<User, ClientError> {
        let body = json_body(&json!({"username": username, "password": password}));
        decode(&self.raw(Method::POST, "/api/setup", body, false).await?)
    }

    /// Logs in and keeps the session in the token store. Returns the
    /// account.
    pub async fn login(&self, username: &str, password: &str) -> Result<User, ClientError> {
        let payload = serde_json::to_vec(&json!({"username": username, "password": password})).unwrap_or_default();
        // Not through `raw`: a 401 here is a refused login with a reason,
        // not a lost session.
        let r = http::send(
            &self.inner.url,
            Method::POST,
            "/api/auth/login",
            &[],
            Some((Bytes::from(payload), "application/json")),
            self.inner.timeout,
        )
        .await?;
        if r.status == 401 {
            let e: lain_api::ApiError = serde_json::from_slice(&r.body).unwrap_or_default();
            return Err(ClientError::LoginFailed { message: e.error });
        }
        if r.status != 200 {
            return Err(ClientError::from_response(r.status, "POST /api/auth/login", &r.body));
        }
        let reply: LoginReply = decode(&r.body)?;
        let headers = [("authorization", format!("Bearer {}", reply.token))];
        let r = http::send(&self.inner.url, Method::GET, "/api/me", &headers, None, self.inner.timeout).await?;
        if r.status != 200 {
            return Err(ClientError::from_response(r.status, "GET /api/me", &r.body));
        }
        let me: User = decode(&r.body)?;
        let session = Session {
            server: self.inner.server.clone(),
            token: reply.token,
            user_id: me.id.clone(),
            username: me.username.clone(),
        };
        self.inner.tokens.save(&session)?;
        *self.inner.session.write().unwrap_or_else(|e| e.into_inner()) = Some(session);
        Ok(me)
    }

    /// Forgets the session here. Tokens are stateless on the server, so
    /// nothing is sent; the token expires on its own (24 h), and a
    /// password change ends every session at once.
    pub fn logout(&self) -> Result<(), ClientError> {
        self.inner.tokens.clear(&self.inner.server)?;
        *self.inner.session.write().unwrap_or_else(|e| e.into_inner()) = None;
        Ok(())
    }

    // ---- the account --------------------------------------------------

    pub async fn me(&self) -> Result<User, ClientError> {
        self.call(Method::GET, "/api/me", Body::None).await
    }

    /// A three-letter ISO 639-2 code, or `""` to clear it.
    pub async fn set_preferred_language(&self, language: &str) -> Result<User, ClientError> {
        self.call(Method::PATCH, "/api/me/preferences", json_body(&json!({"preferred_language": language}))).await
    }

    /// Changes the password; every session of the account, this one
    /// included, stops verifying, so log in again afterwards.
    pub async fn change_password(&self, old: &str, new: &str) -> Result<(), ClientError> {
        let _: StatusReply =
            self.call(Method::PATCH, "/api/me/password", json_body(&json!({"old": old, "new": new}))).await?;
        Ok(())
    }

    pub async fn update_profile(&self, patch: ProfilePatch) -> Result<User, ClientError> {
        self.call(Method::PATCH, "/api/me/profile", json_body(&patch)).await
    }

    /// The Continue rail: this user's progress records.
    pub async fn continue_watching(&self) -> Result<Vec<Progress>, ClientError> {
        self.call(Method::GET, "/api/me/continue", Body::None).await
    }

    pub async fn mascots(&self) -> Result<Vec<String>, ClientError> {
        let m: Mascots = self.call(Method::GET, "/api/profile/mascots", Body::None).await?;
        Ok(m.mascots)
    }

    /// Whether this account has a library PIN.
    pub async fn pin_status(&self) -> Result<bool, ClientError> {
        let p: PinState = self.call(Method::GET, "/api/me/pin", Body::None).await?;
        Ok(p.set)
    }

    pub async fn set_pin(&self, pin: &str, password: &str) -> Result<(), ClientError> {
        let _: PinState =
            self.call(Method::PUT, "/api/me/pin", json_body(&json!({"pin": pin, "password": password}))).await?;
        Ok(())
    }

    pub async fn clear_pin(&self, password: &str) -> Result<(), ClientError> {
        let _: PinState = self.call(Method::DELETE, "/api/me/pin", json_body(&json!({"password": password}))).await?;
        Ok(())
    }

    // ---- users (admin) ------------------------------------------------

    pub async fn users(&self) -> Result<Vec<User>, ClientError> {
        self.call(Method::GET, "/api/users", Body::None).await
    }

    /// `role` is `user` or `admin`; empty means `user`.
    pub async fn create_user(&self, username: &str, password: &str, role: &str) -> Result<User, ClientError> {
        let body = json_body(&json!({"username": username, "password": password, "role": role}));
        self.call(Method::POST, "/api/users", body).await
    }

    pub async fn update_user(&self, id: &str, patch: UserPatch) -> Result<User, ClientError> {
        self.call(Method::PATCH, &format!("/api/users/{}", encode(id)), json_body(&patch)).await
    }

    // ---- libraries ----------------------------------------------------

    /// The libraries this user may see; a private one carries `locked`.
    pub async fn libraries(&self) -> Result<Vec<Library>, ClientError> {
        self.call(Method::GET, "/api/libraries", Body::None).await
    }

    pub async fn create_library(&self, library: NewLibrary) -> Result<Library, ClientError> {
        self.call(Method::POST, "/api/libraries", json_body(&library)).await
    }

    pub async fn update_library(&self, id: &str, patch: LibraryPatch) -> Result<Library, ClientError> {
        self.call(Method::PATCH, &format!("/api/libraries/{}", encode(id)), json_body(&patch)).await
    }

    /// Removes the library and its catalog entries (never files).
    pub async fn delete_library(&self, id: &str) -> Result<LibraryDeleted, ClientError> {
        self.call(Method::DELETE, &format!("/api/libraries/{}", encode(id)), Body::None).await
    }

    /// Unlocks a private library for this session (D-156). Repeated
    /// failures answer [`ClientError::RateLimited`].
    pub async fn unlock_library(&self, id: &str, how: Unlock) -> Result<Library, ClientError> {
        let body = match how {
            Unlock::Pin(p) => json!({"pin": p}),
            Unlock::Password(p) => json!({"password": p}),
        };
        self.call(Method::POST, &format!("/api/libraries/{}/unlock", encode(id)), json_body(&body)).await
    }

    pub async fn lock_library(&self, id: &str) -> Result<Library, ClientError> {
        self.call(Method::POST, &format!("/api/libraries/{}/lock", encode(id)), Body::None).await
    }

    /// The named roots this server maps (admin, D-157).
    pub async fn roots(&self) -> Result<RootsView, ClientError> {
        self.call(Method::GET, "/api/roots", Body::None).await
    }

    // ---- catalog and progress -----------------------------------------

    pub async fn catalog(&self, q: CatalogQuery) -> Result<CatalogPage, ClientError> {
        let path = format!(
            "/api/catalog{}",
            query(&[
                ("limit", num(q.limit)),
                ("offset", num(q.offset)),
                ("sort", q.sort),
                ("library_id", q.library_id)
            ])
        );
        self.call(Method::GET, &path, Body::None).await
    }

    pub async fn catalog_item(&self, id: &str) -> Result<CatalogItem, ClientError> {
        self.call(Method::GET, &format!("/api/catalog/{}", encode(id)), Body::None).await
    }

    /// Every file of the item's title, in episode order.
    pub async fn episodes(&self, id: &str) -> Result<Vec<CatalogItem>, ClientError> {
        let e: Episodes = self.call(Method::GET, &format!("/api/catalog/{}/episodes", encode(id)), Body::None).await?;
        Ok(e.items)
    }

    pub async fn search(&self, q: SearchQuery) -> Result<CatalogPage, ClientError> {
        let path = format!(
            "/api/search{}",
            query(&[
                ("q", q.q),
                ("kind", q.kind),
                ("limit", num(q.limit)),
                ("offset", num(q.offset)),
                ("sort", q.sort)
            ])
        );
        self.call(Method::GET, &path, Body::None).await
    }

    /// This user's progress on an item; the zero record when none.
    pub async fn progress(&self, item_id: &str) -> Result<Progress, ClientError> {
        self.call(Method::GET, &format!("/api/items/{}/progress", encode(item_id)), Body::None).await
    }

    /// Writes progress directly (last writer wins on the server). Use
    /// [`crate::ProgressSync::record`] to get the offline queue.
    pub async fn put_progress(&self, item_id: &str, update: ProgressUpdate) -> Result<Progress, ClientError> {
        self.call(Method::PUT, &format!("/api/items/{}/progress", encode(item_id)), json_body(&update)).await
    }

    // ---- composition and modules --------------------------------------

    pub async fn modules(&self) -> Result<ModulesView, ClientError> {
        self.call(Method::GET, "/api/modules", Body::None).await
    }

    pub async fn plugins(&self) -> Result<PluginsView, ClientError> {
        self.call(Method::GET, "/api/plugins", Body::None).await
    }

    /// Rebinds a capability; a stale `generation` answers 409
    /// `stale-generation`.
    pub async fn swap_providers(&self, swap: SwapRequest) -> Result<SwapReply, ClientError> {
        self.call(Method::POST, "/api/plugins/swap", json_body(&swap)).await
    }

    pub async fn withdraw_provider(&self, provider: &str) -> Result<WithdrawReply, ClientError> {
        self.call(Method::POST, "/api/plugins/withdraw", json_body(&json!({"provider": provider}))).await
    }

    // ---- catalog population -----------------------------------------

    /// Starts a scan of every library (admin); answers when it starts.
    /// A scan already running is `Api { status: 409, .. }`.
    pub async fn start_scan(&self) -> Result<(), ClientError> {
        self.raw(Method::POST, "/api/library/scan", Body::None, true).await.map(|_| ())
    }

    /// The last or running scan.
    pub async fn scan_status(&self) -> Result<ScanStatus, ClientError> {
        self.call(Method::GET, "/api/library/scan", Body::None).await
    }

    /// Server directories for the library folder picker (admin); an
    /// empty `path` auto-detects the media root.
    pub async fn browse(&self, path: &str) -> Result<BrowseResult, ClientError> {
        self.call(Method::GET, &format!("/api/browse{}", query(&[("path", path.to_string())])), Body::None).await
    }

    /// Deletes an item's file (admin); the item stays, `missing`.
    pub async fn delete_item(&self, item_id: &str) -> Result<CatalogItem, ClientError> {
        self.call(Method::DELETE, &format!("/api/items/{}", encode(item_id)), Body::None).await
    }

    /// Fetches an item's metadata overlay (admin); `provider` restricts
    /// the search to one provider, empty means all bound ones.
    pub async fn enrich(&self, item_id: &str, provider: &str) -> Result<Enrichment, ClientError> {
        let path = format!("/api/catalog/{}/enrich{}", encode(item_id), query(&[("provider", provider.to_string())]));
        self.call(Method::POST, &path, Body::None).await
    }

    /// An item's overlay; one that does not exist is `NotFound`.
    pub async fn enrichment(&self, item_id: &str) -> Result<Enrichment, ClientError> {
        self.call(Method::GET, &format!("/api/catalog/{}/enrich", encode(item_id)), Body::None).await
    }

    /// Removes an item's overlay (admin).
    pub async fn delete_enrichment(&self, item_id: &str) -> Result<(), ClientError> {
        self.call::<StatusReply>(Method::DELETE, &format!("/api/catalog/{}/enrich", encode(item_id)), Body::None)
            .await
            .map(|_| ())
    }

    /// The overlays that exist for up to 200 items, in one request.
    pub async fn enrichments(&self, item_ids: &[String]) -> Result<Vec<Enrichment>, ClientError> {
        let path = format!("/api/enrichments{}", query(&[("ids", item_ids.join(","))]));
        Ok(self.call::<Enrichments>(Method::GET, &path, Body::None).await?.items)
    }

    /// The subtitle files next to an item's media file.
    pub async fn sidecars(&self, item_id: &str) -> Result<Vec<Sidecar>, ClientError> {
        Ok(self
            .call::<Sidecars>(Method::GET, &format!("/api/items/{}/sidecars", encode(item_id)), Body::None)
            .await?
            .sidecars)
    }

    /// One sidecar as WebVTT text, by its `index`.
    pub async fn sidecar_vtt(&self, item_id: &str, index: i64) -> Result<String, ClientError> {
        let path = format!("/api/items/{}/sidecars/{index}", encode(item_id));
        Ok(String::from_utf8_lossy(&self.raw(Method::GET, &path, Body::None, true).await?).into_owned())
    }

    // ---- core administration and read ------------------------------

    /// The server's web UI palette (no session needed).
    pub async fn theme(&self) -> Result<Palette, ClientError> {
        decode(&self.raw(Method::GET, "/api/theme", Body::None, false).await?)
    }

    /// The operator's integration settings (admin); secrets are never
    /// returned, only `secret_set`.
    pub async fn integrations(&self) -> Result<IntegrationsView, ClientError> {
        self.call(Method::GET, "/api/admin/settings/integrations", Body::None).await
    }

    /// Saves integration settings (admin): absent fields keep their
    /// value and an empty secret keeps the stored one.
    pub async fn set_integrations(&self, update: IntegrationsUpdate) -> Result<IntegrationsView, ClientError> {
        self.call(Method::PUT, "/api/admin/settings/integrations", json_body(&update)).await
    }

    /// The page index of a comic or manga item.
    pub async fn comic_pages(&self, item_id: &str) -> Result<ComicPages, ClientError> {
        self.call(Method::GET, &format!("/api/items/{}/pages", encode(item_id)), Body::None).await
    }

    /// One page's image bytes; `index` is the page's `index`.
    pub async fn comic_page(&self, item_id: &str, index: i64) -> Result<Vec<u8>, ClientError> {
        let path = format!("/api/items/{}/pages/{index}", encode(item_id));
        Ok(self.raw(Method::GET, &path, Body::None, true).await?.to_vec())
    }

    /// Uploads an avatar image (`content_type` such as `image/png`).
    pub async fn set_avatar(&self, image: Vec<u8>, content_type: &str) -> Result<User, ClientError> {
        self.call(Method::PUT, "/api/me/avatar", Body::Raw(image, content_type.to_string())).await
    }

    pub async fn clear_avatar(&self) -> Result<User, ClientError> {
        self.call(Method::DELETE, "/api/me/avatar", Body::None).await
    }

    /// A user's avatar image bytes.
    pub async fn avatar(&self, user_id: &str) -> Result<Vec<u8>, ClientError> {
        Ok(self.raw(Method::GET, &format!("/api/users/{}/avatar", encode(user_id)), Body::None, true).await?.to_vec())
    }

    /// A URL a player opens for one sidecar (served as WebVTT). It
    /// carries the token, like [`Client::stream_url`].
    pub fn sidecar_url(&self, item_id: &str, index: i64) -> Result<String, ClientError> {
        self.media_url(&format!("/api/items/{}/sidecars/{index}", encode(item_id)), &[])
    }

    /// The server's backup archive (admin): a `lain.backup@1` tar of a
    /// consistent store snapshot and the data dir's documents (D-197).
    pub async fn backup(&self) -> Result<Vec<u8>, ClientError> {
        Ok(self.raw(Method::GET, "/api/admin/backup", Body::None, true).await?.to_vec())
    }

    // ---- the watch module (D-190) ------------------------------------

    /// How to play an item; `client` is `mpv`, `desktop` or `browser`.
    pub async fn playback_plan(&self, item_id: &str, client: &str) -> Result<PlaybackPlan, ClientError> {
        self.playback_plan_for(item_id, client, None).await
    }

    /// [`Client::playback_plan`] with the client's decode claim (D-058):
    /// container tokens (`mkv`) and container/codec pairs (`mkv/h264`).
    /// `None` is "unknown"; an empty list claims nothing.
    pub async fn playback_plan_for(
        &self,
        item_id: &str,
        client: &str,
        caps: Option<Vec<String>>,
    ) -> Result<PlaybackPlan, ClientError> {
        let mut q = query(&[("client", client.to_string())]);
        if let Some(c) = caps {
            q.push(if q.is_empty() { '?' } else { '&' });
            q.push_str(&format!("caps={}", encode(&c.join(","))));
        }
        self.call(Method::GET, &format!("/api/items/{}/playback{q}", encode(item_id)), Body::None).await
    }

    /// The URL an external player opens: Range-capable, authenticated
    /// by `?token=` because players cannot send headers. It carries the
    /// token: never log or share it.
    pub fn stream_url(&self, item_id: &str) -> Result<String, ClientError> {
        let t = self.token().ok_or(ClientError::Unauthorized)?;
        Ok(format!("{}{}?token={}", self.inner.server, stream_path(item_id), encode(&t)))
    }

    /// The item's cover or frame, as image bytes.
    pub async fn thumbnail(&self, item_id: &str) -> Result<Vec<u8>, ClientError> {
        Ok(self
            .raw(Method::GET, &format!("/api/items/{}/thumbnail", encode(item_id)), Body::None, true)
            .await?
            .to_vec())
    }

    /// What the player's quality menu offers.
    pub async fn playback_options(&self) -> Result<PlaybackOptions, ClientError> {
        self.call(Method::GET, "/api/playback/options", Body::None).await
    }

    /// Starts (or joins) a transcode session; ready or not, the answer
    /// is its status (poll [`Client::transcode_status`] until `ready`).
    pub async fn start_transcode(
        &self,
        item_id: &str,
        sel: TranscodeSelection,
    ) -> Result<TranscodeStatus, ClientError> {
        self.call(Method::POST, &format!("/api/items/{}/transcode", encode(item_id)), json_body(&sel)).await
    }

    pub async fn transcode_status(&self, item_id: &str, session: &str) -> Result<TranscodeStatus, ClientError> {
        let path = format!("/api/items/{}/transcode/status{}", encode(item_id), query(&[("session", session.into())]));
        self.call(Method::GET, &path, Body::None).await
    }

    /// Stops a session this account started.
    pub async fn cancel_transcode(&self, item_id: &str, session: &str) -> Result<TranscodeStatus, ClientError> {
        let path = format!("/api/items/{}/transcode{}", encode(item_id), query(&[("session", session.into())]));
        self.call(Method::DELETE, &path, Body::None).await
    }

    fn media_url(&self, path: &str, pairs: &[(&str, String)]) -> Result<String, ClientError> {
        let t = self.token().ok_or(ClientError::Unauthorized)?;
        let mut p: Vec<(&str, String)> = pairs.to_vec();
        p.push(("token", t));
        Ok(format!("{}{path}{}", self.inner.server, query(&p)))
    }

    /// The progressive MP4 of a ready session, for a media element
    /// (carries the token).
    pub fn transcode_url(&self, item_id: &str, session: &str) -> Result<String, ClientError> {
        self.media_url(&format!("/api/items/{}/transcode", encode(item_id)), &[("session", session.into())])
    }

    /// The HLS playlist of a session; its media URIs come back signed
    /// with the same token (carries the token).
    pub fn hls_url(&self, item_id: &str, session: &str) -> Result<String, ClientError> {
        self.media_url(
            &format!("/api/items/{}/transcode/hls/index.m3u8", encode(item_id)),
            &[("session", session.into())],
        )
    }

    /// WebVTT of embedded subtitle stream `stream` (an index from the
    /// plan's `streams`), extracted on demand (carries the token).
    pub fn subtitles_url(&self, item_id: &str, stream: i64) -> Result<String, ClientError> {
        self.media_url(&format!("/api/items/{}/subtitles", encode(item_id)), &[("stream", stream.to_string())])
    }

    /// WebVTT of embedded stream `stream`, as text.
    pub async fn subtitles(&self, item_id: &str, stream: i64) -> Result<String, ClientError> {
        let path = format!("/api/items/{}/subtitles{}", encode(item_id), query(&[("stream", stream.to_string())]));
        let b = self.raw(Method::GET, &path, Body::None, true).await?;
        Ok(String::from_utf8_lossy(&b).into_owned())
    }

    /// Every tracked transcode session (admin).
    pub async fn admin_transcodes(&self) -> Result<AdminTranscodes, ClientError> {
        self.call(Method::GET, "/api/admin/transcodes", Body::None).await
    }

    /// Stops any session (admin).
    pub async fn admin_cancel_transcode(&self, session: &str) -> Result<TranscodeStatus, ClientError> {
        self.call(Method::DELETE, &format!("/api/admin/transcodes/{}", encode(session)), Body::None).await
    }

    /// The transcode policy and the probed ffmpeg surface (admin).
    pub async fn transcode_settings(&self) -> Result<TranscodeSettingsView, ClientError> {
        self.call(Method::GET, "/api/admin/settings/transcode", Body::None).await
    }

    /// Replaces the transcode policy (admin); fields left at their zero
    /// value take the server's defaults, `None` switches stay as saved.
    pub async fn update_transcode_settings(&self, s: TranscodeSettings) -> Result<TranscodeSettingsView, ClientError> {
        self.call(Method::PUT, "/api/admin/settings/transcode", json_body(&s)).await
    }

    /// Players that can open on the server's own screen (empty unless
    /// this client runs on the server's machine).
    pub async fn local_players(&self) -> Result<Vec<String>, ClientError> {
        let p: LocalPlayers = self.call(Method::GET, "/api/localplay", Body::None).await?;
        Ok(p.players.unwrap_or_default())
    }

    /// Opens the item (and the rest of its series) in mpv or VLC on the
    /// server's machine; an empty `player` picks the first.
    pub async fn play_local(&self, item_id: &str, player: &str) -> Result<PlayLocalReply, ClientError> {
        let body = PlayLocalRequest { player: player.to_string() };
        self.call(Method::POST, &format!("/api/items/{}/play-local", encode(item_id)), json_body(&body)).await
    }

    // ---- lists (D-078..D-083) -----------------------------------------

    /// The tracking list, optionally narrowed by media type and status.
    pub async fn list(&self, media_type: &str, status: &str) -> Result<Vec<ListEntry>, ClientError> {
        let path = format!("/api/list{}", query(&[("type", media_type.into()), ("status", status.into())]));
        let e: ListEntries = self.call(Method::GET, &path, Body::None).await?;
        Ok(e.entries)
    }

    /// The linked external accounts (never their tokens).
    pub async fn links(&self) -> Result<Vec<Link>, ClientError> {
        let l: Links = self.call(Method::GET, "/api/me/links", Body::None).await?;
        Ok(l.links)
    }

    /// The URL that starts an OAuth link under the operator's client.
    pub async fn link_authorize_url(&self, platform: &str) -> Result<String, ClientError> {
        let u: LinkUrl =
            self.call(Method::GET, &format!("/api/me/links/{}/authorize", encode(platform)), Body::None).await?;
        Ok(u.url)
    }

    /// The URL of the zero-setup pin flow (D-083).
    pub async fn link_pin_url(&self, platform: &str) -> Result<String, ClientError> {
        let u: LinkUrl = self.call(Method::GET, &format!("/api/me/links/{}/pin", encode(platform)), Body::None).await?;
        Ok(u.url)
    }

    /// Links with the code (or the whole pin URL) the pin page showed.
    pub async fn link_with_code(&self, platform: &str, code: &str) -> Result<Link, ClientError> {
        let r: LinkReply = self
            .call(Method::POST, &format!("/api/me/links/{}/code", encode(platform)), json_body(&json!({"code": code})))
            .await?;
        Ok(r.link)
    }

    /// Turns scrobbling to the platform on or off.
    pub async fn set_link_scrobble(&self, platform: &str, scrobble: bool) -> Result<Link, ClientError> {
        let body = LinkPatch { scrobble: Some(scrobble) };
        self.call(Method::PATCH, &format!("/api/me/links/{}", encode(platform)), json_body(&body)).await
    }

    /// Re-imports the platform's list now.
    pub async fn sync_link(&self, platform: &str) -> Result<ListSyncStats, ClientError> {
        let r: LinkSyncReply =
            self.call(Method::POST, &format!("/api/me/links/{}/sync", encode(platform)), Body::None).await?;
        Ok(r.stats)
    }

    /// Unlinks; returns how many imported entries went with it.
    pub async fn unlink(&self, platform: &str) -> Result<i64, ClientError> {
        let r: UnlinkReply =
            self.call(Method::DELETE, &format!("/api/me/links/{}", encode(platform)), Body::None).await?;
        Ok(r.removed)
    }

    // ---- social ---------------------------------------------------------

    pub async fn social_settings(&self) -> Result<SocialSettings, ClientError> {
        self.call(Method::GET, "/api/social/me/settings", Body::None).await
    }

    pub async fn update_social_settings(&self, patch: SocialSettingsPatch) -> Result<SocialSettings, ClientError> {
        self.call(Method::PATCH, "/api/social/me/settings", json_body(&patch)).await
    }

    /// Accounts by username or display name (at most 50).
    pub async fn find_users(&self, q: &str) -> Result<Vec<UserWithRelation>, ClientError> {
        let s: UserSearch =
            self.call(Method::GET, &format!("/api/social/users{}", query(&[("q", q.into())])), Body::None).await?;
        Ok(s.users)
    }

    pub async fn social_profile(&self, user_id: &str) -> Result<SocialProfile, ClientError> {
        self.call(Method::GET, &format!("/api/social/users/{}", encode(user_id)), Body::None).await
    }

    /// One account's activity; `before` (unix seconds) pages back.
    pub async fn user_activity(&self, user_id: &str, before: i64, limit: i64) -> Result<ActivityPage, ClientError> {
        let q = query(&[("before", num(before)), ("limit", num(limit))]);
        self.call(Method::GET, &format!("/api/social/users/{}/activity{q}", encode(user_id)), Body::None).await
    }

    pub async fn user_ratings(&self, user_id: &str) -> Result<Vec<Rating>, ClientError> {
        let r: Ratings =
            self.call(Method::GET, &format!("/api/social/users/{}/ratings", encode(user_id)), Body::None).await?;
        Ok(r.ratings)
    }

    pub async fn user_collections(&self, user_id: &str) -> Result<Collections, ClientError> {
        self.call(Method::GET, &format!("/api/social/users/{}/collections", encode(user_id)), Body::None).await
    }

    pub async fn friends(&self) -> Result<Friends, ClientError> {
        self.call(Method::GET, "/api/social/friends", Body::None).await
    }

    /// Sends a friend request, or accepts theirs.
    pub async fn befriend(&self, user_id: &str) -> Result<RelateReply, ClientError> {
        self.call(Method::POST, &format!("/api/social/friends/{}", encode(user_id)), Body::None).await
    }

    pub async fn decline_friend(&self, user_id: &str) -> Result<RelateReply, ClientError> {
        self.call(Method::POST, &format!("/api/social/friends/{}/decline", encode(user_id)), Body::None).await
    }

    /// Unfriends, or cancels an outgoing request.
    pub async fn unfriend(&self, user_id: &str) -> Result<RelateReply, ClientError> {
        self.call(Method::DELETE, &format!("/api/social/friends/{}", encode(user_id)), Body::None).await
    }

    pub async fn block(&self, user_id: &str) -> Result<RelateReply, ClientError> {
        self.call(Method::POST, &format!("/api/social/blocks/{}", encode(user_id)), Body::None).await
    }

    pub async fn unblock(&self, user_id: &str) -> Result<RelateReply, ClientError> {
        self.call(Method::DELETE, &format!("/api/social/blocks/{}", encode(user_id)), Body::None).await
    }

    /// The friends feed, newest first; `before` pages back.
    pub async fn feed(&self, before: i64, limit: i64) -> Result<ActivityPage, ClientError> {
        let q = query(&[("before", num(before)), ("limit", num(limit))]);
        self.call(Method::GET, &format!("/api/social/feed{q}"), Body::None).await
    }

    pub async fn notifications(&self, limit: i64) -> Result<Notifications, ClientError> {
        self.call(Method::GET, &format!("/api/social/notifications{}", query(&[("limit", num(limit))])), Body::None)
            .await
    }

    /// Marks the listed notifications (or all) read.
    pub async fn mark_notifications_read(&self, mark: MarkRead) -> Result<i64, ClientError> {
        let c: Changed = self.call(Method::POST, "/api/social/notifications/read", json_body(&mark)).await?;
        Ok(c.changed)
    }

    fn work_query(w: &WorkRef) -> String {
        query(&[("item", w.item_id.clone()), ("kind", w.kind.clone()), ("title", w.title.clone())])
    }

    /// A work's ratings, average and comments as this account may see
    /// them (named by catalog item, or by kind and title).
    pub async fn work(&self, work: &WorkRef) -> Result<WorkView, ClientError> {
        self.call(Method::GET, &format!("/api/social/work{}", Self::work_query(work)), Body::None).await
    }

    pub async fn rate(&self, rating: RatingInput) -> Result<Rating, ClientError> {
        self.call(Method::PUT, "/api/social/work/rating", json_body(&rating)).await
    }

    pub async fn delete_rating(&self, work: &WorkRef) -> Result<bool, ClientError> {
        let r: Removed = self
            .call(Method::DELETE, &format!("/api/social/work/rating{}", Self::work_query(work)), Body::None)
            .await?;
        Ok(r.removed)
    }

    pub async fn comment(&self, comment: CommentInput) -> Result<Comment, ClientError> {
        self.call(Method::POST, "/api/social/work/comments", json_body(&comment)).await
    }

    pub async fn delete_comment(&self, id: &str) -> Result<(), ClientError> {
        let _: Removed = self.call(Method::DELETE, &format!("/api/social/comments/{}", encode(id)), Body::None).await?;
        Ok(())
    }

    /// Recommends a work to friends; returns who received it.
    pub async fn share_work(&self, share: ShareInput) -> Result<Vec<String>, ClientError> {
        let r: ShareReply = self.call(Method::POST, "/api/social/share", json_body(&share)).await?;
        Ok(r.sent)
    }

    /// This account's collections plus those shared with it.
    pub async fn collections(&self) -> Result<Collections, ClientError> {
        self.call(Method::GET, "/api/social/collections", Body::None).await
    }

    pub async fn create_collection(&self, c: NewCollection) -> Result<CollectionReply, ClientError> {
        self.call(Method::POST, "/api/social/collections", json_body(&c)).await
    }

    pub async fn collection(&self, id: &str) -> Result<CollectionReply, ClientError> {
        self.call(Method::GET, &format!("/api/social/collections/{}", encode(id)), Body::None).await
    }

    pub async fn update_collection(&self, id: &str, patch: CollectionPatch) -> Result<CollectionReply, ClientError> {
        self.call(Method::PATCH, &format!("/api/social/collections/{}", encode(id)), json_body(&patch)).await
    }

    pub async fn delete_collection(&self, id: &str) -> Result<(), ClientError> {
        let _: Removed =
            self.call(Method::DELETE, &format!("/api/social/collections/{}", encode(id)), Body::None).await?;
        Ok(())
    }

    /// Adds a work (or updates its note).
    pub async fn add_to_collection(&self, id: &str, item: CollectionItemInput) -> Result<CollectionReply, ClientError> {
        self.call(Method::POST, &format!("/api/social/collections/{}/items", encode(id)), json_body(&item)).await
    }

    pub async fn remove_from_collection(&self, id: &str, work: &WorkRef) -> Result<CollectionReply, ClientError> {
        let path = format!("/api/social/collections/{}/items{}", encode(id), Self::work_query(work));
        self.call(Method::DELETE, &path, Body::None).await
    }

    /// Gives friends read access; returns who received it.
    pub async fn share_collection(&self, id: &str, share: CollectionShareInput) -> Result<Vec<String>, ClientError> {
        let r: ShareReply = self
            .call(Method::POST, &format!("/api/social/collections/{}/share", encode(id)), json_body(&share))
            .await?;
        Ok(r.sent)
    }

    pub async fn unshare_collection(&self, id: &str, user_id: &str) -> Result<CollectionReply, ClientError> {
        let path = format!("/api/social/collections/{}/share/{}", encode(id), encode(user_id));
        self.call(Method::DELETE, &path, Body::None).await
    }
}

pub(crate) fn stream_path(item_id: &str) -> String {
    format!("/api/items/{}/stream", encode(item_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_query_values_are_left_out() {
        assert_eq!(query(&[("limit", num(0)), ("sort", "".into())]), "");
        assert_eq!(query(&[("limit", "5".into()), ("q", "a b".into())]), "?limit=5&q=a%20b");
    }

    #[test]
    fn secrets_stay_out_of_debug() {
        assert!(!format!("{:?}", Unlock::Pin("2468".into())).contains("2468"));
    }
}
