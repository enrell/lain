//! The network metadata providers (D-201): Kitsu, AniList, Jikan and
//! TVMaze, in-process over the shared [`lain_http::Client`], each a
//! `lain.metadata.search@1` + `lain.metadata.resolve@1` provider in the
//! merge-many bindings after the NFO provider, as in Go. A port of
//! `internal/plugins/metadata/{client,kitsu,anilist,jikan,tvmaze}.go`.
//!
//! Every answer is decoded tolerantly: a `null` reads as empty and an
//! unknown field is ignored, so an upstream change degrades one field,
//! never the search. A failing provider fails its own call only; the
//! merge skips it.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};

use lain_core::{Error, Provider};
use lain_http::{Client, path_escape, query_escape};

use crate::nfo::{CAP_METADATA_RESOLVE, CAP_METADATA_SEARCH};
use crate::types::{MetadataCandidate, MetadataRecord, MetadataResolveInput, MetadataSearchInput};

pub const KITSU_ID: &str = "lain-metadata-kitsu";
pub const ANILIST_ID: &str = "lain-metadata-anilist";
pub const JIKAN_ID: &str = "lain-metadata-jikan";
pub const TVMAZE_ID: &str = "lain-metadata-tvmaze";

/// Where each provider's API lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    pub kitsu: String,
    pub anilist: String,
    pub jikan: String,
    pub tvmaze: String,
}

impl Endpoints {
    /// The public services (Go's defaults).
    pub fn public() -> Endpoints {
        Endpoints {
            kitsu: "https://kitsu.io".into(),
            anilist: "https://graphql.anilist.co".into(),
            jikan: "https://api.jikan.moe".into(),
            tvmaze: "https://api.tvmaze.com".into(),
        }
    }

    /// The public services, each overridable by
    /// `LAIN_METADATA_{KITSU,ANILIST,JIKAN,TVMAZE}_URL` (as in Go; the
    /// conformance suite points them at recorded fixtures).
    pub fn from_env() -> Endpoints {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Endpoints {
        let p = Endpoints::public();
        let pick = |key: &str, public: String| match get(key).map(|v| v.trim().trim_end_matches('/').to_string()) {
            Some(v) if !v.is_empty() => v,
            _ => public,
        };
        Endpoints {
            kitsu: pick("LAIN_METADATA_KITSU_URL", p.kitsu),
            anilist: pick("LAIN_METADATA_ANILIST_URL", p.anilist),
            jikan: pick("LAIN_METADATA_JIKAN_URL", p.jikan),
            tvmaze: pick("LAIN_METADATA_TVMAZE_URL", p.tvmaze),
        }
    }

    /// Every provider under one fixture server: `<base>/kitsu`, ...
    pub fn under(base: &str) -> Endpoints {
        let b = base.trim_end_matches('/');
        Endpoints {
            kitsu: format!("{b}/kitsu"),
            anilist: format!("{b}/anilist"),
            jikan: format!("{b}/jikan"),
            tvmaze: format!("{b}/tvmaze"),
        }
    }
}

/// The network providers' wiring: one HTTP client, the endpoints, and
/// whether to pace requests per upstream host (Go's intervals: Kitsu
/// 1 s, AniList 700 ms, Jikan 400 ms, TVMaze 1 s). Loopback endpoints
/// (fixture servers, a local mirror) are never paced: with every
/// provider on one host, pacing would serialize them all at the
/// longest interval.
#[derive(Debug, Clone)]
pub struct Remote {
    pub client: Client,
    pub endpoints: Endpoints,
    pub pace: bool,
}

impl Remote {
    pub fn new(client: Client, endpoints: Endpoints) -> Remote {
        Remote { client, endpoints, pace: true }
    }

    /// No network at all: every remote call fails at once with
    /// `offline`, so only local providers answer.
    pub fn offline() -> Remote {
        Remote { client: Client::offline(), endpoints: Endpoints::public(), pace: false }
    }

    /// The four providers, in Go's precedence.
    pub fn providers(&self) -> Vec<Arc<dyn Provider>> {
        let e = &self.endpoints;
        let gap = |base: &str, ms: u64| {
            let local = lain_http::Url::parse(base).is_ok_and(|u| is_loopback(&u.host));
            if self.pace && !local { Duration::from_millis(ms) } else { Duration::ZERO }
        };
        for (base, ms) in [(&e.kitsu, 1000), (&e.anilist, 700), (&e.jikan, 400), (&e.tvmaze, 1000)] {
            self.client.pace(base, gap(base, ms));
        }
        vec![
            Arc::new(Kitsu::new(self.client.clone(), &e.kitsu)),
            Arc::new(AniList::new(self.client.clone(), &e.anilist)),
            Arc::new(Jikan::new(self.client.clone(), &e.jikan)),
            Arc::new(TvMaze::new(self.client.clone(), &e.tvmaze)),
        ]
    }
}

// ---- shared helpers ---------------------------------------------------------

fn is_loopback(host: &str) -> bool {
    host == "localhost" || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// `null` reads as the type's empty value (Go's decoder leaves the zero
/// value for `null`).
fn nullable<'de, D: Deserializer<'de>, T: Deserialize<'de> + Default>(d: D) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

fn upstream(id: &str, e: lain_http::Error) -> Error {
    Error::unavailable(format!("{id}: {e}"))
}

fn decode<T: serde::de::DeserializeOwned>(id: &str, raw: &[u8]) -> Result<T, Error> {
    serde_json::from_slice(raw).map_err(|e| Error::unavailable(format!("{id}: unexpected answer: {e}")))
}

/// 1..=10, else 5.
fn clamp_limit(l: i64) -> i64 {
    if (1..=10).contains(&l) { l } else { 5 }
}

/// The first four characters as a number, Go's `strconv.Atoi(s[:4])`.
fn year4(s: &str) -> i64 {
    s.get(..4).and_then(|y| y.parse().ok()).unwrap_or(0)
}

/// The first non-blank value, trimmed.
fn first_of(v: &[&str]) -> String {
    v.iter().map(|s| s.trim()).find(|s| !s.is_empty()).unwrap_or("").to_string()
}

/// Drops `<br>`/`<i>` markup (a bare `>` is consumed too, as in Go).
pub fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}

/// The string values of a JSON object (non-strings are ignored, so
/// Kitsu's image `meta` object never breaks decoding).
fn string_map(v: &Value) -> BTreeMap<String, String> {
    v.as_object()
        .map(|m| m.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
        .unwrap_or_default()
}

/// The shared `invoke`: capability and op dispatch, the kind gate and
/// typed input errors.
fn dispatch(
    cap: &str,
    op: &str,
    input: Value,
    kinds: &[&str],
    search: impl FnOnce(&MetadataSearchInput) -> Result<Vec<MetadataCandidate>, Error>,
    resolve: impl FnOnce(&str) -> Result<MetadataRecord, Error>,
) -> Result<Value, Error> {
    match cap {
        CAP_METADATA_SEARCH if op.is_empty() || op == "search" => {
            let i: MetadataSearchInput = serde_json::from_value(input)
                .map_err(|e| Error::invalid(format!("MetadataSearchInput required: {e}")))?;
            if !i.kind.is_empty() && !kinds.contains(&i.kind.as_str()) {
                return Ok(json!([]));
            }
            Ok(json!(search(&i)?))
        }
        CAP_METADATA_RESOLVE if op.is_empty() || op == "resolve" => {
            let i: MetadataResolveInput = serde_json::from_value(input)
                .map_err(|e| Error::invalid(format!("MetadataResolveInput required: {e}")))?;
            Ok(json!(resolve(&i.remote_id)?))
        }
        CAP_METADATA_SEARCH | CAP_METADATA_RESOLVE => Err(Error::invalid(format!("capability {cap} has no op {op}"))),
        _ => Err(Error::invalid(format!("unsupported cap {cap}"))),
    }
}

const ANIME_KINDS: &[&str] = &["anime", "episode", "video"];

macro_rules! provider {
    ($ty:ident, $id:expr, $kinds:expr) => {
        impl Provider for $ty {
            fn id(&self) -> &str {
                $id
            }
            fn capabilities(&self) -> Vec<String> {
                vec![CAP_METADATA_SEARCH.into(), CAP_METADATA_RESOLVE.into()]
            }
            fn health(&self) -> Result<(), Error> {
                Ok(())
            }
            fn invoke(&self, cap: &str, op: &str, input: Value) -> Result<Value, Error> {
                dispatch(cap, op, input, $kinds, |i| self.search(i), |id| self.resolve(id))
            }
        }
    };
}

// ---- Kitsu (JSON:API, no key) -----------------------------------------------

pub struct Kitsu {
    http: Client,
    base: String,
}

#[derive(Deserialize, Default)]
struct KitsuEntry {
    #[serde(default, deserialize_with = "nullable")]
    id: String,
    #[serde(default, deserialize_with = "nullable")]
    attributes: KitsuAttrs,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct KitsuAttrs {
    #[serde(default, deserialize_with = "nullable")]
    canonical_title: String,
    #[serde(default)]
    titles: Value,
    #[serde(default, deserialize_with = "nullable")]
    synopsis: String,
    #[serde(default, deserialize_with = "nullable")]
    start_date: String,
    #[serde(default, deserialize_with = "nullable")]
    episode_count: i64,
    #[serde(default)]
    poster_image: Value,
    #[serde(default)]
    cover_image: Value,
}

impl KitsuAttrs {
    fn titles(&self) -> BTreeMap<String, String> {
        string_map(&self.titles)
    }

    fn synonyms(&self) -> Vec<String> {
        self.titles().into_values().map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).collect()
    }
}

/// The best image tier, else any non-empty one.
fn first_image(v: &Value) -> String {
    let m = string_map(v);
    for k in ["original", "large", "medium", "small", "tiny"] {
        if let Some(v) = m.get(k).filter(|v| !v.is_empty()) {
            return v.clone();
        }
    }
    m.into_values().find(|v| !v.is_empty()).unwrap_or_default()
}

impl Kitsu {
    pub fn new(http: Client, base: &str) -> Kitsu {
        Kitsu { http, base: base.trim_end_matches('/').to_string() }
    }

    fn get(&self, url: &str) -> Result<Vec<u8>, Error> {
        let r = self.http.get(url).send_blocking().and_then(|r| r.ok()).map_err(|e| upstream(KITSU_ID, e))?;
        Ok(r.body)
    }

    fn search(&self, i: &MetadataSearchInput) -> Result<Vec<MetadataCandidate>, Error> {
        let url = format!(
            "{}/api/edge/anime?filter%5Btext%5D={}&page%5Blimit%5D={}",
            self.base,
            query_escape(&i.query),
            clamp_limit(i.limit)
        );
        #[derive(Deserialize)]
        struct Doc {
            #[serde(default, deserialize_with = "nullable")]
            data: Vec<KitsuEntry>,
        }
        let doc: Doc = decode(KITSU_ID, &self.get(&url)?)?;
        Ok(doc.data.iter().map(kitsu_candidate).collect())
    }

    fn resolve(&self, id: &str) -> Result<MetadataRecord, Error> {
        #[derive(Deserialize)]
        struct Doc {
            #[serde(default, deserialize_with = "nullable")]
            data: KitsuEntry,
        }
        let doc: Doc = decode(KITSU_ID, &self.get(&format!("{}/api/edge/anime/{}", self.base, path_escape(id)))?)?;
        Ok(kitsu_record(&doc.data))
    }
}

fn kitsu_candidate(d: &KitsuEntry) -> MetadataCandidate {
    let a = &d.attributes;
    let t = a.titles();
    let title = first_nonempty(&[&a.canonical_title, t.get("en").map_or("", |s| s), t.get("en_jp").map_or("", |s| s)]);
    MetadataCandidate {
        provider: KITSU_ID.into(),
        remote_id: d.id.clone(),
        title,
        synonyms: a.synonyms(),
        year: year4(&a.start_date),
        kind: "anime".into(),
        poster: first_image(&a.poster_image),
        score: 0.0,
    }
}

fn kitsu_record(d: &KitsuEntry) -> MetadataRecord {
    let a = &d.attributes;
    let t = a.titles();
    MetadataRecord {
        provider: KITSU_ID.into(),
        remote_id: d.id.clone(),
        title: first_nonempty(&[&a.canonical_title, t.get("en").map_or("", |s| s)]),
        synonyms: a.synonyms(),
        year: year4(&a.start_date),
        genres: Vec::new(),
        synopsis: a.synopsis.trim().to_string(),
        poster: first_image(&a.poster_image),
        cover: first_image(&a.cover_image),
        episodes: a.episode_count,
    }
}

/// The first non-empty value as given (Go's `if title == ""` chain).
fn first_nonempty(v: &[&str]) -> String {
    v.iter().find(|s| !s.is_empty()).unwrap_or(&"").to_string()
}

provider!(Kitsu, KITSU_ID, ANIME_KINDS);

// ---- AniList (GraphQL, no key) ----------------------------------------------

pub struct AniList {
    http: Client,
    base: String,
}

const ANILIST_FIELDS: &str = "id title { romaji english native } description coverImage { large extraLarge } bannerImage episodes startDate { year } genres synonyms";

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AniListMedia {
    #[serde(default, deserialize_with = "nullable")]
    id: i64,
    #[serde(default, deserialize_with = "nullable")]
    title: AniListTitle,
    #[serde(default, deserialize_with = "nullable")]
    description: String,
    #[serde(default, deserialize_with = "nullable")]
    cover_image: AniListCover,
    #[serde(default, deserialize_with = "nullable")]
    banner_image: String,
    #[serde(default, deserialize_with = "nullable")]
    episodes: i64,
    #[serde(default, deserialize_with = "nullable")]
    start_date: AniListDate,
    #[serde(default, deserialize_with = "nullable")]
    genres: Vec<String>,
    #[serde(default, deserialize_with = "nullable")]
    synonyms: Vec<String>,
}

#[derive(Deserialize, Default)]
struct AniListTitle {
    #[serde(default, deserialize_with = "nullable")]
    romaji: String,
    #[serde(default, deserialize_with = "nullable")]
    english: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AniListCover {
    #[serde(default, deserialize_with = "nullable")]
    large: String,
    #[serde(default, deserialize_with = "nullable")]
    extra_large: String,
}

#[derive(Deserialize, Default)]
struct AniListDate {
    #[serde(default, deserialize_with = "nullable")]
    year: i64,
}

impl AniListMedia {
    fn best_title(&self) -> String {
        if self.title.english.is_empty() { self.title.romaji.clone() } else { self.title.english.clone() }
    }
}

/// The leading integer of `s`, as Go's `fmt.Sscanf(s, "%d")` reads it.
fn leading_int(s: &str) -> Option<i64> {
    let t = s.trim_start();
    let end = t
        .char_indices()
        .find(|&(i, c)| !(c.is_ascii_digit() || (i == 0 && (c == '+' || c == '-'))))
        .map_or(t.len(), |(i, _)| i);
    t[..end].parse().ok()
}

impl AniList {
    pub fn new(http: Client, base: &str) -> AniList {
        AniList { http, base: base.trim_end_matches('/').to_string() }
    }

    fn query(&self, body: Value) -> Result<Vec<u8>, Error> {
        // A GraphQL read: safe to repeat.
        let r = self
            .http
            .post(&self.base)
            .json(&body)
            .idempotent(true)
            .send_blocking()
            .and_then(|r| r.ok())
            .map_err(|e| upstream(ANILIST_ID, e))?;
        Ok(r.body)
    }

    fn search(&self, i: &MetadataSearchInput) -> Result<Vec<MetadataCandidate>, Error> {
        let raw = self.query(json!({
            "query": format!("query ($search: String, $perPage: Int) {{ Page(page: 1, perPage: $perPage) {{ media(search: $search, type: ANIME) {{ {ANILIST_FIELDS} }} }} }}"),
            "variables": {"search": i.query, "perPage": clamp_limit(i.limit)},
        }))?;
        // GraphQL answers `Page`/`Media`; Go's decoder matches field
        // names case-insensitively, serde needs the alias.
        #[derive(Deserialize)]
        struct Doc {
            #[serde(default, deserialize_with = "nullable")]
            data: Data,
        }
        #[derive(Deserialize, Default)]
        struct Data {
            #[serde(default, alias = "Page", deserialize_with = "nullable")]
            page: Page,
        }
        #[derive(Deserialize, Default)]
        struct Page {
            #[serde(default, deserialize_with = "nullable")]
            media: Vec<AniListMedia>,
        }
        let doc: Doc = decode(ANILIST_ID, &raw)?;
        Ok(doc
            .data
            .page
            .media
            .iter()
            .map(|m| MetadataCandidate {
                provider: ANILIST_ID.into(),
                remote_id: m.id.to_string(),
                title: m.best_title(),
                synonyms: m.synonyms.clone(),
                year: m.start_date.year,
                kind: "anime".into(),
                poster: first_of(&[&m.cover_image.extra_large, &m.cover_image.large]),
                score: 0.0,
            })
            .collect())
    }

    fn resolve(&self, id: &str) -> Result<MetadataRecord, Error> {
        let aid = leading_int(id).ok_or_else(|| Error::invalid("numeric id required"))?;
        let raw = self.query(json!({
            "query": format!("query ($id: Int) {{ Media(id: $id) {{ {ANILIST_FIELDS} }} }}"),
            "variables": {"id": aid},
        }))?;
        #[derive(Deserialize)]
        struct Doc {
            #[serde(default, deserialize_with = "nullable")]
            data: Data,
        }
        #[derive(Deserialize, Default)]
        struct Data {
            #[serde(default, alias = "Media", deserialize_with = "nullable")]
            media: AniListMedia,
        }
        let m = decode::<Doc>(ANILIST_ID, &raw)?.data.media;
        Ok(MetadataRecord {
            provider: ANILIST_ID.into(),
            remote_id: m.id.to_string(),
            title: m.best_title(),
            synonyms: m.synonyms.clone(),
            year: m.start_date.year,
            genres: m.genres.clone(),
            synopsis: strip_html(&m.description),
            poster: first_of(&[&m.cover_image.extra_large, &m.cover_image.large]),
            cover: m.banner_image.clone(),
            episodes: m.episodes,
        })
    }
}

provider!(AniList, ANILIST_ID, ANIME_KINDS);

// ---- Jikan (MyAnimeList over REST, no key) ----------------------------------

pub struct Jikan {
    http: Client,
    base: String,
}

#[derive(Deserialize, Default)]
struct JikanEntry {
    #[serde(default, deserialize_with = "nullable")]
    mal_id: i64,
    #[serde(default, deserialize_with = "nullable")]
    title: String,
    #[serde(default, deserialize_with = "nullable")]
    title_english: String,
    #[serde(default, deserialize_with = "nullable")]
    title_japanese: String,
    #[serde(default, deserialize_with = "nullable")]
    synopsis: String,
    #[serde(default, deserialize_with = "nullable")]
    episodes: i64,
    #[serde(default, deserialize_with = "nullable")]
    aired: JikanAired,
    #[serde(default, deserialize_with = "nullable")]
    images: JikanImages,
    #[serde(default, deserialize_with = "nullable")]
    genres: Vec<JikanGenre>,
}

#[derive(Deserialize, Default)]
struct JikanAired {
    #[serde(default, deserialize_with = "nullable")]
    from: String,
}

#[derive(Deserialize, Default)]
struct JikanImages {
    #[serde(default, deserialize_with = "nullable")]
    jpg: JikanJpg,
}

#[derive(Deserialize, Default)]
struct JikanJpg {
    #[serde(default, deserialize_with = "nullable")]
    image_url: String,
    #[serde(default, deserialize_with = "nullable")]
    large_image_url: String,
}

#[derive(Deserialize, Default)]
struct JikanGenre {
    #[serde(default, deserialize_with = "nullable")]
    name: String,
}

/// A plausible year (1900..=2099) from an ISO date, else 0.
pub fn year_of(iso: &str) -> i64 {
    match year4(iso) {
        y @ 1900..=2099 => y,
        _ => 0,
    }
}

impl JikanEntry {
    fn best_title(&self) -> String {
        if self.title_english.is_empty() { self.title.clone() } else { self.title_english.clone() }
    }

    fn synonyms(&self) -> Vec<String> {
        let best = self.best_title();
        [&self.title, &self.title_japanese]
            .into_iter()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty() && *t != best)
            .collect()
    }

    fn poster(&self) -> String {
        first_of(&[&self.images.jpg.large_image_url, &self.images.jpg.image_url])
    }
}

impl Jikan {
    pub fn new(http: Client, base: &str) -> Jikan {
        Jikan { http, base: base.trim_end_matches('/').to_string() }
    }

    fn get(&self, url: &str) -> Result<Vec<u8>, Error> {
        let r = self.http.get(url).send_blocking().and_then(|r| r.ok()).map_err(|e| upstream(JIKAN_ID, e))?;
        Ok(r.body)
    }

    fn search(&self, i: &MetadataSearchInput) -> Result<Vec<MetadataCandidate>, Error> {
        let url = format!(
            "{}/v4/anime?q={}&limit={}&order_by=members&sort=desc&sfw=true",
            self.base,
            query_escape(&i.query),
            clamp_limit(i.limit)
        );
        #[derive(Deserialize)]
        struct Doc {
            #[serde(default, deserialize_with = "nullable")]
            data: Vec<JikanEntry>,
        }
        let doc: Doc = decode(JIKAN_ID, &self.get(&url)?)?;
        Ok(doc
            .data
            .iter()
            .map(|e| MetadataCandidate {
                provider: JIKAN_ID.into(),
                remote_id: e.mal_id.to_string(),
                title: e.best_title(),
                synonyms: e.synonyms(),
                year: year_of(&e.aired.from),
                kind: "anime".into(),
                poster: e.poster(),
                score: 0.0,
            })
            .collect())
    }

    fn resolve(&self, id: &str) -> Result<MetadataRecord, Error> {
        if id.parse::<i64>().is_err() {
            return Err(Error::invalid("numeric id required"));
        }
        #[derive(Deserialize)]
        struct Doc {
            #[serde(default, deserialize_with = "nullable")]
            data: JikanEntry,
        }
        let e = decode::<Doc>(JIKAN_ID, &self.get(&format!("{}/v4/anime/{}", self.base, path_escape(id)))?)?.data;
        Ok(MetadataRecord {
            provider: JIKAN_ID.into(),
            remote_id: e.mal_id.to_string(),
            title: e.best_title(),
            synonyms: e.synonyms(),
            year: year_of(&e.aired.from),
            genres: e.genres.iter().map(|g| g.name.clone()).collect(),
            synopsis: e.synopsis.trim().to_string(),
            poster: e.poster(),
            cover: String::new(),
            episodes: e.episodes,
        })
    }
}

provider!(Jikan, JIKAN_ID, ANIME_KINDS);

// ---- TVMaze (series, no key) ------------------------------------------------

/// TV-first: every video kind but movies, which TVMaze does not
/// catalogue. Anime keeps the anime providers ahead in the binding.
pub struct TvMaze {
    http: Client,
    base: String,
}

#[derive(Deserialize, Default)]
struct TvMazeShow {
    #[serde(default, deserialize_with = "nullable")]
    id: i64,
    #[serde(default, deserialize_with = "nullable")]
    name: String,
    #[serde(default, deserialize_with = "nullable")]
    genres: Vec<String>,
    #[serde(default, deserialize_with = "nullable")]
    premiered: String,
    #[serde(default, deserialize_with = "nullable")]
    summary: String,
    #[serde(default)]
    image: Option<TvMazeImage>,
}

#[derive(Deserialize, Default)]
struct TvMazeImage {
    #[serde(default, deserialize_with = "nullable")]
    medium: String,
    #[serde(default, deserialize_with = "nullable")]
    original: String,
}

impl TvMazeShow {
    /// The full-size portrait, else the medium thumbnail.
    fn poster(&self) -> String {
        match &self.image {
            Some(i) if !i.original.is_empty() => i.original.clone(),
            Some(i) => i.medium.clone(),
            None => String::new(),
        }
    }
}

impl TvMaze {
    pub fn new(http: Client, base: &str) -> TvMaze {
        TvMaze { http, base: base.trim_end_matches('/').to_string() }
    }

    fn get(&self, url: &str) -> Result<Vec<u8>, Error> {
        let r = self.http.get(url).send_blocking().and_then(|r| r.ok()).map_err(|e| upstream(TVMAZE_ID, e))?;
        Ok(r.body)
    }

    fn search(&self, i: &MetadataSearchInput) -> Result<Vec<MetadataCandidate>, Error> {
        #[derive(Deserialize)]
        struct Hit {
            #[serde(default, deserialize_with = "nullable")]
            show: TvMazeShow,
        }
        let hits: Vec<Hit> =
            decode(TVMAZE_ID, &self.get(&format!("{}/search/shows?q={}", self.base, query_escape(&i.query)))?)?;
        Ok(hits
            .iter()
            .map(|h| &h.show)
            .filter(|s| !s.name.trim().is_empty())
            .take(clamp_limit(i.limit) as usize)
            .map(|s| MetadataCandidate {
                provider: TVMAZE_ID.into(),
                remote_id: s.id.to_string(),
                title: s.name.trim().to_string(),
                synonyms: Vec::new(),
                year: year4(&s.premiered),
                kind: "series".into(),
                poster: s.poster(),
                score: 0.0,
            })
            .collect())
    }

    fn resolve(&self, id: &str) -> Result<MetadataRecord, Error> {
        let s: TvMazeShow = decode(TVMAZE_ID, &self.get(&format!("{}/shows/{}", self.base, path_escape(id)))?)?;
        if s.name.trim().is_empty() {
            return Err(Error::unavailable("tvmaze: empty show"));
        }
        // No cover: TVMaze has one portrait per show. Episode counts
        // need a second call and stay out, as in Go.
        Ok(MetadataRecord {
            provider: TVMAZE_ID.into(),
            remote_id: s.id.to_string(),
            title: s.name.trim().to_string(),
            synonyms: Vec::new(),
            year: year4(&s.premiered),
            genres: s.genres.clone(),
            synopsis: strip_html(&s.summary),
            poster: s.poster(),
            cover: String::new(),
            episodes: 0,
        })
    }
}

provider!(TvMaze, TVMAZE_ID, &["anime", "series", "episode", "video"]);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_parsers_match_go() {
        for (i, w) in
            [("a<b>x</b>c", "axc"), ("plain", "plain"), ("<br>hi", "hi"), ("unclosed <i", "unclosed"), ("1>0", "10")]
        {
            assert_eq!(strip_html(i), w, "{i}");
        }
        for (i, w) in [
            ("", 0),
            ("abc", 0),
            ("1899-01-01", 0),
            ("1900-01-01", 1900),
            ("2099-12-31", 2099),
            ("2100-01-01", 0),
            ("xxxx-01-01", 0),
            ("2023", 2023),
            ("2023-09-29T0", 2023),
        ] {
            assert_eq!(year_of(i), w, "{i}");
        }
        assert_eq!((year4("202"), year4("2020"), year4("bad")), (0, 2020, 0));
        for (l, w) in [(0, 5), (-3, 5), (11, 5), (10, 10), (3, 3)] {
            assert_eq!(clamp_limit(l), w);
        }
        assert_eq!((leading_int("20965"), leading_int("12abc"), leading_int("abc")), (Some(20965), Some(12), None));
        assert_eq!(first_image(&json!({"weird": "w.jpg"})), "w.jpg", "an unknown tier still answers");
        assert_eq!(first_image(&json!({"original": "", "tiny": "t.jpg", "meta": {"x": 1}})), "t.jpg");
        assert_eq!(first_image(&json!({"a": "", "b": ""})), "");
        let e = Endpoints::from_lookup(|k| (k == "LAIN_METADATA_TVMAZE_URL").then(|| "http://127.0.0.1:9/tv/".into()));
        assert_eq!((e.tvmaze.as_str(), e.kitsu.as_str()), ("http://127.0.0.1:9/tv", "https://kitsu.io"));
    }

    #[test]
    fn kitsu_falls_back_and_tolerates_nulls() {
        let e: KitsuEntry = serde_json::from_value(json!({"id": "5", "attributes": {
            "canonicalTitle": "", "titles": {"en": "English Title", "en_jp": "Romanized", "bad": "  ", "ja_jp": null},
            "synopsis": null, "startDate": "xx", "episodeCount": null,
            "posterImage": {"tiny": "https://t.jpg", "meta": {"dimensions": {}}}, "coverImage": null}}))
        .unwrap();
        let c = kitsu_candidate(&e);
        assert_eq!((c.title.as_str(), c.year, c.poster.as_str()), ("English Title", 0, "https://t.jpg"));
        assert_eq!(c.synonyms, vec!["English Title", "Romanized"], "blank and null titles are dropped");
        let only_jp: KitsuEntry =
            serde_json::from_value(json!({"id": "6", "attributes": {"titles": {"en_jp": "Romanized"}}})).unwrap();
        assert_eq!(kitsu_candidate(&only_jp).title, "Romanized");
        assert_eq!(kitsu_record(&only_jp).title, "", "resolve falls back to en only, as in Go");
    }

    #[test]
    fn jikan_synonyms_skip_the_title() {
        let e: JikanEntry = serde_json::from_value(json!({"mal_id": 7, "title": "Romaji", "title_english": "Eng Title",
            "title_japanese": "Romaji", "aired": {"from": "1899-01-01"}, "images": {"jpg": {"image_url": "s.jpg", "large_image_url": ""}}}))
        .unwrap();
        assert_eq!(e.best_title(), "Eng Title");
        assert_eq!(e.synonyms(), vec!["Romaji", "Romaji"]);
        assert_eq!((e.poster().as_str(), year_of(&e.aired.from)), ("s.jpg", 0));
        let plain: JikanEntry =
            serde_json::from_value(json!({"mal_id": 8, "title": "Only Romaji", "title_english": null})).unwrap();
        assert_eq!((plain.best_title().as_str(), plain.synonyms().len()), ("Only Romaji", 0));
    }
}
