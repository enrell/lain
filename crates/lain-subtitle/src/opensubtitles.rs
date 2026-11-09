//! `lain-subtitle-opensubtitles` (`lain.subtitle@1`, D-137/D-150,
//! D-202): the OpenSubtitles.com REST API v1 with the operator's own API
//! key. Ops `search` ([`SubtitleSearchInput`] → [`SubtitleSearchOutput`])
//! and `download` ([`SubtitleDownloadInput`] → [`SubtitleDownloadOutput`],
//! a short-lived link the caller fetches). The account travels in the
//! input; its secrets never reach a log line or an error, and errors name
//! the host only. Another provider kind declines with
//! `unsupported-provider`, so the ordered-many binding moves on.
//! A port of `internal/plugins/subtitles/opensubtitles.go`.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};

use lain_api::{
    SUBTITLE_OPENSUBTITLES, SubtitleCandidate, SubtitleDownloadInput, SubtitleDownloadOutput, SubtitleProvider,
    SubtitleSearchInput, SubtitleSearchOutput,
};
use lain_core::{Error, Provider};
use lain_http::{Client, Method, Url, query_escape};

use crate::sidecar::{language, tag};

pub const OPENSUBTITLES_ID: &str = "lain-subtitle-opensubtitles";
pub const CAP_SUBTITLE: &str = "lain.subtitle@1";

const DEFAULT_BASE: &str = "https://api.opensubtitles.com/api/v1";
const MAX_BODY: usize = 2 << 20;
const MAX_RESULTS: usize = 200;
const TOKEN_TTL: Duration = Duration::from_secs(12 * 3600);
const TIMEOUT: Duration = Duration::from_secs(30);

/// The API asks for `<AppName> v<version>` as the User-Agent.
const USER_AGENT: &str = concat!("Lain v", env!("CARGO_PKG_VERSION"));

/// The provider. Its only state is a login-token cache per account, so
/// a download does not log in every time.
pub struct OpenSubtitles {
    http: Client,
    tokens: Mutex<HashMap<String, (String, Instant)>>,
}

fn fail(base: &Url, code: &str, msg: &str) -> Error {
    Error::new(code, format!("{}: {msg}", base.authority()))
}

/// Byte-bounded, cut at a character boundary.
fn clip(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut end = n;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// Lower-case, alphabetically sorted parameters: the API redirects
/// otherwise.
fn encode_sorted(q: &BTreeMap<String, String>) -> String {
    q.iter()
        .map(|(k, v)| format!("{}={}", query_escape(&k.to_lowercase()), query_escape(&v.to_lowercase())))
        .collect::<Vec<_>>()
        .join("&")
}

/// ISO 639-2 codes to the API's tags, asking for both regional variants
/// where the API splits a language (D-150).
pub fn api_languages(langs: &[String]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut add = |t: &str| {
        if !out.iter().any(|o| o == t) {
            out.push(t.to_string());
        }
    };
    for l in langs {
        match l.as_str() {
            "por" => {
                add("pt-br");
                add("pt-pt");
            }
            "zho" => {
                add("zh-cn");
                add("zh-tw");
            }
            _ => {
                let t = tag(l, "");
                if !t.is_empty() {
                    add(&t.to_lowercase());
                }
            }
        }
    }
    out.sort();
    out.join(",")
}

/// The account's API root, or the typed refusal.
fn check(p: &SubtitleProvider) -> Result<Url, Error> {
    if p.kind != SUBTITLE_OPENSUBTITLES {
        return Err(Error::new("unsupported-provider", "not an OpenSubtitles account"));
    }
    let raw = if p.base_url.is_empty() { DEFAULT_BASE } else { p.base_url.as_str() };
    let u = Url::parse(raw).map_err(|_| Error::invalid("provider url must be http(s)"))?;
    if u.target.contains('?') {
        return Err(Error::invalid("provider url must be http(s)"));
    }
    if p.api_key.trim().is_empty() {
        return Err(Error::invalid("an OpenSubtitles API key is required"));
    }
    Ok(u)
}

#[derive(Deserialize, Default)]
struct SearchResponse {
    #[serde(default)]
    data: Vec<Entry>,
}

#[derive(Deserialize, Default)]
struct Entry {
    #[serde(default)]
    attributes: Attrs,
}

fn nullable<'de, D: serde::Deserializer<'de>, T: Deserialize<'de> + Default>(d: D) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[derive(Deserialize, Default)]
struct Attrs {
    #[serde(default, deserialize_with = "nullable")]
    language: String,
    #[serde(default, deserialize_with = "nullable")]
    download_count: i64,
    #[serde(default, deserialize_with = "nullable")]
    hearing_impaired: bool,
    #[serde(default, deserialize_with = "nullable")]
    foreign_parts_only: bool,
    #[serde(default, deserialize_with = "nullable")]
    release: String,
    #[serde(default, deserialize_with = "nullable")]
    moviehash_match: bool,
    #[serde(default, deserialize_with = "nullable")]
    feature_details: Feature,
    #[serde(default, deserialize_with = "nullable")]
    files: Vec<FileRef>,
}

#[derive(Deserialize, Default)]
struct Feature {
    #[serde(default, deserialize_with = "nullable")]
    season_number: i64,
    #[serde(default, deserialize_with = "nullable")]
    episode_number: i64,
}

#[derive(Deserialize, Default)]
struct FileRef {
    #[serde(default, deserialize_with = "nullable")]
    file_id: i64,
    #[serde(default, deserialize_with = "nullable")]
    file_name: String,
}

impl OpenSubtitles {
    pub fn new(http: Client) -> OpenSubtitles {
        OpenSubtitles { http, tokens: Mutex::new(HashMap::new()) }
    }

    /// One API call: the account's key, the bearer token when given,
    /// the status mapped to a stable code, the body as JSON.
    fn call<T: serde::de::DeserializeOwned>(
        &self,
        base: &Url,
        p: &SubtitleProvider,
        method: Method,
        path: &str,
        body: Option<Value>,
        bearer: &str,
    ) -> Result<T, Error> {
        // `path` may carry the query.
        let target = format!("{}{path}", base.target.trim_end_matches('/'));
        let url = format!("{}://{}{target}", if base.tls { "https" } else { "http" }, base.authority());
        let mut req = self
            .http
            .request(method, &url)
            .header("Api-Key", &p.api_key)
            .header("Accept", "application/json")
            .user_agent(USER_AGENT)
            .timeout(TIMEOUT)
            .max_body(MAX_BODY);
        if let Some(b) = body {
            req = req.json(&b);
        }
        if !bearer.is_empty() {
            req = req.bearer(bearer);
        }
        let res = req.send_blocking().map_err(|e| match e.kind {
            lain_http::ErrorKind::TooLarge => fail(base, "dependency-unavailable", "unreadable response"),
            lain_http::ErrorKind::InvalidUrl => fail(base, "invalid-message", "bad request"),
            _ => fail(base, "dependency-unavailable", "unreachable"),
        })?;
        match res.status {
            401 | 403 => return Err(fail(base, "auth-failed", "credentials rejected")),
            429 => return Err(fail(base, "rate-limited", "too many requests or daily download quota used")),
            404 => return Err(fail(base, "not-found", "no such subtitle")),
            s if s >= 300 => return Err(fail(base, "dependency-unavailable", &format!("http {s}"))),
            _ => {}
        }
        serde_json::from_slice(&res.body)
            .map_err(|_| fail(base, "dependency-unavailable", "response is not valid JSON"))
    }

    pub fn search(&self, i: &SubtitleSearchInput) -> Result<SubtitleSearchOutput, Error> {
        let base = check(&i.provider)?;
        let mut q = BTreeMap::new();
        if !i.hash.is_empty() {
            if i.hash.len() != 16 {
                return Err(Error::invalid("hash must be 16 hex digits"));
            }
            q.insert("moviehash".to_string(), i.hash.clone());
        }
        let query = i.query.trim();
        if !query.is_empty() {
            q.insert("query".into(), clip(query, 200));
        }
        if q.is_empty() {
            return Err(Error::invalid("a hash or a query is required"));
        }
        let langs = api_languages(&i.languages);
        if !langs.is_empty() {
            q.insert("languages".into(), langs);
        }
        if i.movie {
            q.insert("type".into(), "movie".into());
            if i.year > 0 {
                q.insert("year".into(), i.year.to_string());
            }
        } else if i.episode > 0 {
            q.insert("type".into(), "episode".into());
            q.insert("episode_number".into(), i.episode.to_string());
            if i.season > 0 {
                q.insert("season_number".into(), i.season.to_string());
            }
        }
        let res: SearchResponse =
            self.call(&base, &i.provider, Method::GET, &format!("/subtitles?{}", encode_sorted(&q)), None, "")?;
        let mut out = SubtitleSearchOutput { candidates: Vec::new() };
        for d in res.data {
            let a = d.attributes;
            let lang = language(&a.language);
            let Some(file) = a.files.first() else { continue };
            if lang.is_empty() || out.candidates.len() >= MAX_RESULTS {
                continue;
            }
            out.candidates.push(SubtitleCandidate {
                provider_id: i.provider.id.clone(),
                file_id: file.file_id.to_string(),
                language: lang,
                region: if a.language.contains('-') { clip(&a.language, 10) } else { String::new() },
                release: clip(&a.release, 300),
                file_name: clip(&file.file_name, 300),
                hash_match: a.moviehash_match,
                hi: a.hearing_impaired,
                forced: a.foreign_parts_only,
                downloads: a.download_count,
                season: a.feature_details.season_number,
                episode: a.feature_details.episode_number,
            });
        }
        Ok(out)
    }

    /// Logs in once per account (when a username is set) and keeps the
    /// token 12 h; anonymous accounts use the API key's own quota.
    fn token(&self, base: &Url, p: &SubtitleProvider) -> Result<String, Error> {
        if p.username.is_empty() {
            return Ok(String::new());
        }
        let key = format!("{base}\0{}", p.username);
        {
            let g = self.tokens.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((t, until)) = g.get(&key)
                && Instant::now() < *until
            {
                return Ok(t.clone());
            }
        }
        #[derive(Deserialize)]
        struct Login {
            #[serde(default, deserialize_with = "nullable")]
            token: String,
        }
        let body = json!({"username": p.username, "password": p.password});
        let res: Login = self.call(base, p, Method::POST, "/login", Some(body), "")?;
        if res.token.is_empty() {
            return Err(fail(base, "auth-failed", "login returned no token"));
        }
        self.tokens
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, (res.token.clone(), Instant::now() + TOKEN_TTL));
        Ok(res.token)
    }

    pub fn download(&self, i: &SubtitleDownloadInput) -> Result<SubtitleDownloadOutput, Error> {
        let base = check(&i.provider)?;
        let id: i64 = match i.file_id.parse() {
            Ok(n) if n > 0 => n,
            _ => return Err(Error::invalid("file_id must be a positive number")),
        };
        let tok = self.token(&base, &i.provider)?;
        #[derive(Deserialize)]
        struct Link {
            #[serde(default, deserialize_with = "nullable")]
            link: String,
            #[serde(default, deserialize_with = "nullable")]
            file_name: String,
            #[serde(default)]
            remaining: Option<i64>,
        }
        let res: Link = self.call(&base, &i.provider, Method::POST, "/download", Some(json!({"file_id": id})), &tok)?;
        if Url::parse(&res.link).is_err() {
            return Err(fail(&base, "dependency-unavailable", "no usable download link"));
        }
        Ok(SubtitleDownloadOutput {
            link: res.link,
            file_name: clip(&res.file_name, 300),
            remaining: res.remaining.unwrap_or(-1),
        })
    }
}

impl Provider for OpenSubtitles {
    fn id(&self) -> &str {
        OPENSUBTITLES_ID
    }

    fn capabilities(&self) -> Vec<String> {
        vec![CAP_SUBTITLE.into()]
    }

    fn health(&self) -> Result<(), Error> {
        Ok(())
    }

    fn invoke(&self, cap: &str, op: &str, input: Value) -> Result<Value, Error> {
        if cap != CAP_SUBTITLE {
            return Err(Error::invalid(format!("unsupported cap {cap}")));
        }
        let bad = |e: serde_json::Error| {
            Error::invalid(format!("SubtitleSearchInput or SubtitleDownloadInput required: {e}"))
        };
        match op {
            "search" => Ok(json!(self.search(&serde_json::from_value(input).map_err(bad)?)?)),
            "download" => Ok(json!(self.download(&serde_json::from_value(input).map_err(bad)?)?)),
            _ => Err(Error::invalid(format!("capability {cap} has no op {op}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_and_query_encoding() {
        let l = |v: &[&str]| api_languages(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(l(&["eng", "por"]), "en,pt-br,pt-pt");
        assert_eq!(l(&["zho", "jpn", "eng", "eng"]), "en,ja,zh-cn,zh-tw");
        assert_eq!(l(&["fil"]), "fil");
        let mut q = BTreeMap::new();
        q.insert("query".to_string(), "Show Name".to_string());
        q.insert("languages".to_string(), "en,pt-br".to_string());
        assert_eq!(encode_sorted(&q), "languages=en%2Cpt-br&query=show+name");
        assert_eq!(clip("ação", 2), "a");
        assert_eq!(clip("ação", 3), "aç");
    }
}
