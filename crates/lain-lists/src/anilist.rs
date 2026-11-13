//! The AniList connector (`lain-listlink-anilist`): the OAuth2
//! authorization-code grant for linking (AniList has no PKCE and no
//! refresh tokens; access tokens are year-lived JWTs), GraphQL
//! `MediaListCollection` reads for imports and `SaveMediaListEntry` for
//! scrobbles. It serves only the `anilist` platform and declines the
//! rest with `unsupported-platform`. Go: `internal/plugins/listlink`.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use lain_api::{list_media, list_status};
use lain_core::{Error, Provider};

use crate::link::{
    self, AuthorizeInput, AuthorizeOutput, ExchangeInput, FetchInput, FetchOutput, Identity, PushInput, PushOutput,
};
use crate::{CAP_LIST_LINK, ListEntry, code, encode_query};

pub const PROVIDER_ID: &str = "lain-listlink-anilist";
/// The platform key the gateway and the account store use.
pub const PLATFORM: &str = "anilist";

pub const GRAPHQL_URL: &str = "https://graphql.anilist.co";
pub const OAUTH_URL: &str = "https://anilist.co/api/v2/oauth";

/// The API roots; tests and the conformance suite point them at a
/// fixture server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    pub graphql: String,
    pub oauth: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints { graphql: GRAPHQL_URL.into(), oauth: OAUTH_URL.into() }
    }
}

impl Endpoints {
    /// `LAIN_LISTLINK_ANILIST_GRAPHQL_URL` and
    /// `LAIN_LISTLINK_ANILIST_OAUTH_URL` when set, else the public API.
    pub fn from_env() -> Endpoints {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Endpoints {
        let pick = |k: &str, d: &str| {
            get(k).map(|v| v.trim().trim_end_matches('/').to_string()).filter(|v| !v.is_empty()).unwrap_or(d.into())
        };
        Endpoints {
            graphql: pick("LAIN_LISTLINK_ANILIST_GRAPHQL_URL", GRAPHQL_URL),
            oauth: pick("LAIN_LISTLINK_ANILIST_OAUTH_URL", OAUTH_URL),
        }
    }
}

pub struct AniList {
    endpoints: Endpoints,
    http: lain_http::Client,
}

/// Go's interval between two AniList calls; loopback hosts (fixture
/// servers) are never paced.
const PACE: Duration = Duration::from_millis(700);

fn is_loopback(url: &str) -> bool {
    lain_http::Url::parse(url)
        .is_ok_and(|u| u.host == "localhost" || u.host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback()))
}

fn sync_failed(msg: impl Into<String>) -> Error {
    Error::new(code::SYNC_FAILED, msg)
}

fn unsupported(p: &str) -> Error {
    Error::new(code::UNSUPPORTED_PLATFORM, format!("anilist connector cannot serve {p}"))
}

impl AniList {
    pub fn new(endpoints: Endpoints, http: lain_http::Client) -> AniList {
        for base in [&endpoints.graphql, &endpoints.oauth] {
            http.pace(base, if is_loopback(base) { Duration::ZERO } else { PACE });
        }
        AniList { endpoints, http }
    }

    /// One JSON POST. Errors name the host only (D-200); a 429 the
    /// client did not retry is Go's `rate limited (429)`.
    fn post(&self, url: &str, body: &Value, bearer: &str, idempotent: bool) -> Result<(u16, Vec<u8>), Error> {
        let mut req = self.http.post(url).json(body).header("Accept", "application/json").idempotent(idempotent);
        if !bearer.is_empty() {
            req = req.bearer(bearer);
        }
        let r = req.send_blocking().map_err(|e| sync_failed(e.to_string()))?;
        if r.status == 429 {
            return Err(Error::new(code::RATE_LIMITED, "rate limited (429)"));
        }
        Ok((r.status, r.body))
    }

    pub fn authorize(&self, i: &AuthorizeInput) -> Result<AuthorizeOutput, Error> {
        if i.client_id.is_empty() || i.redirect_uri.is_empty() || i.state.is_empty() {
            return Err(Error::invalid("client_id, redirect_uri and state required"));
        }
        let q = encode_query(&[
            ("client_id", &i.client_id),
            ("redirect_uri", &i.redirect_uri),
            ("response_type", "code"),
            ("state", &i.state),
        ]);
        Ok(AuthorizeOutput { url: format!("{}/authorize?{q}", self.endpoints.oauth) })
    }

    pub fn exchange(&self, i: &ExchangeInput) -> Result<Identity, Error> {
        if i.code.is_empty() || i.client_id.is_empty() || i.client_secret.is_empty() || i.redirect_uri.is_empty() {
            return Err(Error::invalid("code, client_id, client_secret and redirect_uri required"));
        }
        // Go marshals a map: keys sorted, which json! also does.
        let body = json!({
            "client_id": i.client_id,
            "client_secret": i.client_secret,
            "code": i.code,
            "grant_type": "authorization_code",
            "redirect_uri": i.redirect_uri,
        });
        let (status, raw) = self.post(&format!("{}/token", self.endpoints.oauth), &body, "", false)?;
        if status / 100 != 2 {
            return Err(Error::new(code::INVALID_GRANT, grant_error(&raw, status)));
        }
        #[derive(Deserialize)]
        struct Tok {
            #[serde(default)]
            access_token: String,
        }
        let tok = serde_json::from_slice::<Tok>(&raw).ok().map(|t| t.access_token).unwrap_or_default();
        if tok.is_empty() {
            return Err(Error::new(code::INVALID_GRANT, "token response carried no access_token"));
        }
        let (id, name) = self.viewer(&tok)?;
        Ok(Identity {
            token_expires_at: jwt_expiry(&tok),
            token: tok,
            remote_user_id: id.to_string(),
            remote_username: name,
        })
    }

    fn viewer(&self, token: &str) -> Result<(i64, String), Error> {
        let doc = self.graphql(token, "query { Viewer { id name } }", Value::Null)?;
        let v = &doc["data"]["Viewer"];
        let id = v["id"].as_i64().unwrap_or(0);
        if id == 0 {
            return Err(Error::new(code::INVALID_GRANT, "viewer query returned no account"));
        }
        Ok((id, v["name"].as_str().unwrap_or("").to_string()))
    }

    /// One authenticated query. A 401/403 is a dead or revoked
    /// credential (`token-invalid`), never a transient failure (D-081).
    fn graphql(&self, token: &str, query: &str, variables: Value) -> Result<Value, Error> {
        let body = json!({"query": query, "variables": variables});
        let idempotent = !query.trim_start().starts_with("mutation");
        let (status, raw) = self.post(&self.endpoints.graphql, &body, token, idempotent)?;
        if status == 401 || status == 403 {
            return Err(Error::new(code::TOKEN_INVALID, "anilist rejected the access token"));
        }
        if status / 100 != 2 {
            let text = String::from_utf8_lossy(&raw);
            return Err(sync_failed(format!(
                "anilist graphql: status {status}: {}",
                text.chars().take(200).collect::<String>()
            )));
        }
        serde_json::from_slice(&raw).map_err(|e| sync_failed(e.to_string()))
    }

    pub fn fetch(&self, i: &FetchInput) -> Result<FetchOutput, Error> {
        if i.token.is_empty() || i.remote_user_id.is_empty() {
            return Err(Error::invalid("token and remote_user_id required"));
        }
        let uid = leading_int(&i.remote_user_id)
            .filter(|n| *n > 0)
            .ok_or_else(|| Error::invalid("remote_user_id must be numeric"))?;
        let mut out = FetchOutput::default();
        for media_type in ["ANIME", "MANGA"] {
            out.entries.extend(self.collection(&i.token, uid, media_type)?);
        }
        Ok(out)
    }

    fn collection(&self, token: &str, uid: i64, media_type: &str) -> Result<Vec<ListEntry>, Error> {
        let doc = self.graphql(token, COLLECTION_QUERY, json!({"userId": uid, "type": media_type}))?;
        let col: CollectionDoc = serde_json::from_value(doc["data"]["MediaListCollection"].clone()).unwrap_or_default();
        // One media can repeat across custom lists; the standard list's
        // copy is canonical (D-081 remote authority).
        let mut standard: BTreeMap<i64, ListEntry> = BTreeMap::new();
        let mut custom: BTreeMap<i64, ListEntry> = BTreeMap::new();
        for l in col.lists.unwrap_or_default() {
            for e in l.entries.unwrap_or_default() {
                if e.media.id == 0 {
                    continue;
                }
                let id = e.media_id;
                let entry = map_entry(e);
                if l.is_custom_list {
                    custom.entry(id).or_insert(entry);
                } else {
                    standard.insert(id, entry);
                }
            }
        }
        for (id, e) in custom {
            standard.entry(id).or_insert(e);
        }
        Ok(standard.into_values().collect())
    }

    /// Writes the fields the input carries to one entry, creating it
    /// when the list does not hold the media yet. GraphQL answers 200
    /// with `errors` for validation failures, so the body is checked,
    /// not just the status. Only the variables that are set are sent:
    /// AniList leaves an argument that is not given as it was.
    pub fn push(&self, i: &PushInput) -> Result<PushOutput, Error> {
        if i.token.is_empty() {
            return Err(Error::invalid("token required"));
        }
        let media =
            leading_int(&i.remote_id).filter(|n| *n > 0).ok_or_else(|| Error::invalid("remote_id must be numeric"))?;
        if i.progress.is_some_and(|p| p < 0) || i.progress_volumes.is_some_and(|p| p < 0) {
            return Err(Error::invalid("progress must not be negative"));
        }
        if i.score.is_some_and(|s| !(0.0..=10.0).contains(&s)) {
            return Err(Error::invalid("score must be between 0 and 10"));
        }
        let mut vars = json!({"mediaId": media});
        if let Some(p) = i.progress {
            vars["progress"] = json!(p);
        }
        if let Some(v) = i.progress_volumes {
            vars["progressVolumes"] = json!(v);
        }
        if !i.status.is_empty() {
            vars["status"] = json!(remote_status(&i.status));
        }
        // `scoreRaw` is the 100-point scale whatever format the user
        // chose on AniList, so 8.5 here is 8.5 on a 10-point list and
        // 85 on a 100-point one.
        if let Some(sc) = i.score {
            vars["scoreRaw"] = json!((sc * 10.0).round() as i64);
        }
        let doc = self.graphql(&i.token, SAVE_ENTRY_MUTATION, vars)?;
        if let Some(e) = doc["errors"].as_array().and_then(|a| a.first()) {
            let msg = e["message"].as_str().unwrap_or("");
            return Err(sync_failed(format!(
                "anilist rejected the entry: {}",
                msg.chars().take(200).collect::<String>()
            )));
        }
        let save = &doc["data"]["SaveMediaListEntry"];
        let entry = serde_json::from_value::<AlEntry>(save.clone()).ok().filter(|e| e.media.id != 0).map(map_entry);
        Ok(PushOutput {
            progress: save["progress"].as_i64().unwrap_or(0),
            status: map_status(save["status"].as_str().unwrap_or("")),
            entry,
        })
    }
}

/// The shared vocabulary onto AniList's statuses (the inverse of
/// [`map_status`]).
pub fn remote_status(s: &str) -> String {
    match s {
        list_status::CURRENT => "CURRENT".into(),
        list_status::PLANNING => "PLANNING".into(),
        list_status::COMPLETED => "COMPLETED".into(),
        list_status::PAUSED => "PAUSED".into(),
        list_status::DROPPED => "DROPPED".into(),
        list_status::REPEATING => "REPEATING".into(),
        other => other.to_uppercase(),
    }
}

/// Go's `fmt.Sscanf("%d")`: an optional sign and the leading digits.
fn leading_int(s: &str) -> Option<i64> {
    let s = s.trim_start();
    let (neg, rest) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let n: i64 = digits.parse().ok()?;
    Some(if neg { -n } else { n })
}

/// AniList's OAuth error description, never echoing credentials.
fn grant_error(raw: &[u8], status: u16) -> String {
    let v: Value = serde_json::from_slice(raw).unwrap_or(Value::Null);
    ["error_description", "error", "message"]
        .iter()
        .filter_map(|k| v[*k].as_str())
        .find(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("status {status}"))
}

/// The `exp` claim of an access token (a JWT); 0 when unreadable, which
/// means "no known expiry".
pub fn jwt_expiry(token: &str) -> i64 {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return 0;
    }
    lain_auth::crypto::b64_url_decode(parts[1])
        .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
        .and_then(|v| v["exp"].as_i64())
        .unwrap_or(0)
}

/// One entry's fields, shared by the collection read and the save's
/// answer so both map through [`map_entry`].
macro_rules! entry_fields {
    () => {
        "mediaId status progress progressVolumes
				score(format: POINT_10_DECIMAL) repeat notes
				startedAt { year month day }
				completedAt { year month day }
				media {
					id type format episodes chapters
					title { romaji english native }
					coverImage { extraLarge large }
				}"
    };
}

const COLLECTION_QUERY: &str = concat!(
    r#"query ($userId: Int, $type: MediaType) {
	MediaListCollection(userId: $userId, type: $type) {
		lists {
			name
			isCustomList
			entries {
				"#,
    entry_fields!(),
    r#"
			}
		}
	}
}"#
);

const SAVE_ENTRY_MUTATION: &str = concat!(
    r#"mutation ($mediaId: Int, $progress: Int, $progressVolumes: Int, $status: MediaListStatus, $scoreRaw: Int) {
	SaveMediaListEntry(mediaId: $mediaId, progress: $progress, progressVolumes: $progressVolumes, status: $status, scoreRaw: $scoreRaw) {
		"#,
    entry_fields!(),
    r#"
	}
}"#
);

/// Absent and `null` read as empty, as Go's zero values do.
fn nullable<'de, D: serde::Deserializer<'de>, T: Default + Deserialize<'de>>(d: D) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[derive(Deserialize, Default)]
struct CollectionDoc {
    #[serde(default)]
    lists: Option<Vec<AlList>>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AlList {
    #[serde(default, deserialize_with = "nullable")]
    is_custom_list: bool,
    #[serde(default)]
    entries: Option<Vec<AlEntry>>,
}

#[derive(Deserialize, Default)]
struct AlDate {
    #[serde(default, deserialize_with = "nullable")]
    year: i64,
    #[serde(default, deserialize_with = "nullable")]
    month: i64,
    #[serde(default, deserialize_with = "nullable")]
    day: i64,
}

#[derive(Deserialize, Default)]
struct AlTitle {
    #[serde(default, deserialize_with = "nullable")]
    romaji: String,
    #[serde(default, deserialize_with = "nullable")]
    english: String,
    #[serde(default, deserialize_with = "nullable")]
    native: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AlCover {
    #[serde(default, deserialize_with = "nullable")]
    extra_large: String,
    #[serde(default, deserialize_with = "nullable")]
    large: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AlMedia {
    #[serde(default, deserialize_with = "nullable")]
    id: i64,
    #[serde(rename = "type", default, deserialize_with = "nullable")]
    kind: String,
    #[serde(default, deserialize_with = "nullable")]
    format: String,
    #[serde(default, deserialize_with = "nullable")]
    episodes: i64,
    #[serde(default, deserialize_with = "nullable")]
    chapters: i64,
    #[serde(default, deserialize_with = "nullable")]
    title: AlTitle,
    #[serde(default, deserialize_with = "nullable")]
    cover_image: AlCover,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AlEntry {
    #[serde(default, deserialize_with = "nullable")]
    media_id: i64,
    #[serde(default, deserialize_with = "nullable")]
    status: String,
    #[serde(default, deserialize_with = "nullable")]
    progress: i64,
    #[serde(default, deserialize_with = "nullable")]
    progress_volumes: i64,
    #[serde(default, deserialize_with = "nullable")]
    score: f64,
    #[serde(default, deserialize_with = "nullable")]
    repeat: i64,
    #[serde(default, deserialize_with = "nullable")]
    notes: String,
    #[serde(default, deserialize_with = "nullable")]
    started_at: AlDate,
    #[serde(default, deserialize_with = "nullable")]
    completed_at: AlDate,
    #[serde(default, deserialize_with = "nullable")]
    media: AlMedia,
}

fn map_entry(e: AlEntry) -> ListEntry {
    let title = first_non_empty(&[&e.media.title.english, &e.media.title.romaji, &e.media.title.native]);
    let mut alt_titles: Vec<String> = Vec::new();
    for t in [&e.media.title.english, &e.media.title.romaji, &e.media.title.native] {
        let t = t.trim();
        if !t.is_empty() && t != title && !alt_titles.iter().any(|a| a == t) {
            alt_titles.push(t.to_string());
        }
    }
    let (media_type, total) = match e.media.kind.as_str() {
        "ANIME" => (list_media::ANIME.to_string(), e.media.episodes),
        "MANGA" => (list_media::MANGA.to_string(), e.media.chapters),
        other => (other.to_lowercase(), 0),
    };
    ListEntry {
        remote_id: e.media_id.to_string(),
        status: map_status(&e.status),
        progress: e.progress,
        progress_total: total,
        progress_volumes: e.progress_volumes,
        score: e.score,
        repeat: e.repeat,
        notes: e.notes,
        started_at: format_date(&e.started_at),
        completed_at: format_date(&e.completed_at),
        format: e.media.format,
        title,
        alt_titles,
        cover: first_non_empty(&[&e.media.cover_image.extra_large, &e.media.cover_image.large]),
        media_type,
        ..ListEntry::default()
    }
}

/// AniList's statuses onto the shared vocabulary.
pub fn map_status(s: &str) -> String {
    match s.to_uppercase().as_str() {
        "CURRENT" => list_status::CURRENT.into(),
        "PLANNING" => list_status::PLANNING.into(),
        "COMPLETED" => list_status::COMPLETED.into(),
        "PAUSED" => list_status::PAUSED.into(),
        "DROPPED" => list_status::DROPPED.into(),
        "REPEATING" => list_status::REPEATING.into(),
        _ => s.to_lowercase(),
    }
}

fn format_date(d: &AlDate) -> String {
    if d.year <= 0 {
        return String::new();
    }
    format!("{:04}-{:02}-{:02}", d.year, d.month, d.day)
}

fn first_non_empty(vs: &[&str]) -> String {
    vs.iter().map(|v| v.trim()).find(|v| !v.is_empty()).unwrap_or("").to_string()
}

impl Provider for AniList {
    fn id(&self) -> &str {
        PROVIDER_ID
    }
    fn capabilities(&self) -> Vec<String> {
        vec![CAP_LIST_LINK.into()]
    }
    fn health(&self) -> Result<(), Error> {
        Ok(())
    }
    fn invoke(&self, cap: &str, op: &str, input: Value) -> Result<Value, Error> {
        if cap != CAP_LIST_LINK {
            return Err(Error::invalid(format!("unsupported cap {cap}")));
        }
        let platform = input["platform"].as_str().unwrap_or("").to_string();
        let bad = || Error::invalid("LinkAuthorizeInput, LinkExchangeInput, LinkFetchInput or LinkPushInput required");
        let v = input;
        if !matches!(op, link::AUTHORIZE | link::EXCHANGE | link::FETCH | link::PUSH) {
            return Err(bad());
        }
        if platform != PLATFORM {
            return Err(unsupported(&platform));
        }
        let to = |r: Result<Value, serde_json::Error>| r.map_err(|e| Error::internal(e.to_string()));
        match op {
            link::AUTHORIZE => {
                to(serde_json::to_value(self.authorize(&serde_json::from_value(v).map_err(|_| bad())?)?))
            }
            link::EXCHANGE => to(serde_json::to_value(self.exchange(&serde_json::from_value(v).map_err(|_| bad())?)?)),
            link::FETCH => to(serde_json::to_value(self.fetch(&serde_json::from_value(v).map_err(|_| bad())?)?)),
            _ => to(serde_json::to_value(self.push(&serde_json::from_value(v).map_err(|_| bad())?)?)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(leading_int("42"), Some(42));
        assert_eq!(leading_int("42abc"), Some(42));
        assert_eq!(leading_int("x"), None);
        assert_eq!(map_status("CURRENT"), "current");
        assert_eq!(map_status("Hiatus"), "hiatus");
        for s in ["current", "planning", "completed", "paused", "dropped", "repeating"] {
            assert_eq!(map_status(&remote_status(s)), s, "round trip");
        }
        assert_eq!(format_date(&AlDate { year: 2024, month: 3, day: 9 }), "2024-03-09");
        assert_eq!(format_date(&AlDate::default()), "");
        let claims = lain_auth::crypto::b64_url(br#"{"exp":1900000000}"#);
        assert_eq!(jwt_expiry(&format!("h.{claims}.s")), 1_900_000_000);
        for bad in ["", "a.b", "a.!!.c", "a.b.c.d"] {
            assert_eq!(jwt_expiry(bad), 0, "{bad}");
        }
        assert_eq!(grant_error(br#"{"error":"invalid_grant","error_description":"expired"}"#, 400), "expired");
        assert_eq!(grant_error(br#"{"error":"invalid_client"}"#, 401), "invalid_client");
        assert_eq!(grant_error(b"<html>", 500), "status 500");
    }

    #[test]
    fn endpoints_from_env() {
        let e = Endpoints::from_lookup(|k| {
            (k == "LAIN_LISTLINK_ANILIST_OAUTH_URL").then(|| "http://127.0.0.1:9/oauth/ ".into())
        });
        assert_eq!(e, Endpoints { graphql: GRAPHQL_URL.into(), oauth: "http://127.0.0.1:9/oauth".into() });
    }
}
