//! `lain-indexer-torznab` (`lain.indexer@1`, A-7): the Newznab API
//! family, so direct Torznab/Newznab indexers and aggregators such as
//! Jackett and Prowlarr all work through it. It holds no indexer state:
//! definitions, credentials and rate limits come from the caller. Ops
//! `caps` ([`IndexerCapsInput`] → [`IndexerCaps`]) and `search`
//! ([`IndexerSearchInput`] → [`IndexerSearchOutput`]). Request URLs
//! carry the API key, so errors name the host only.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use lain_api::acquire::{
    CAP_INDEXER, INDEXER_NEWZNAB, INDEXER_TORZNAB, Indexer, IndexerCaps, IndexerCapsInput, IndexerCategory,
    IndexerSearchInput, IndexerSearchOutput, PROTOCOL_TORRENT, PROTOCOL_USENET, SEARCH_BOOK, SEARCH_MOVIE, SEARCH_TV,
    SearchResult,
};
use lain_core::{Error, Provider};
use lain_downloads::http::{Http, Request};

use crate::util::{clip, http_url, parse_rfc1123, url_host};
use crate::xml::{self, Node};

pub const TORZNAB_ID: &str = "lain-indexer-torznab";

const MAX_RESPONSE: usize = 8 << 20;
const MAX_RESULTS: usize = 1000;
const TIMEOUT: Duration = Duration::from_secs(30);

pub struct Torznab {
    http: Arc<dyn Http>,
}

impl Torznab {
    pub fn new(http: Arc<dyn Http>) -> Torznab {
        Torznab { http }
    }
}

fn check(ix: &Indexer) -> Result<(), Error> {
    if ix.protocol != INDEXER_TORZNAB && ix.protocol != INDEXER_NEWZNAB {
        return Err(Error::new("unsupported-protocol", "not a torznab/newznab indexer"));
    }
    if !http_url(&ix.url) {
        return Err(Error::invalid("indexer url must be http(s)"));
    }
    Ok(())
}

/// Go's `url.QueryEscape`.
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

/// `{url}/api?…` with sorted parameters (Go `url.Values.Encode`); a URL
/// already ending in `/api` is kept.
fn endpoint(ix: &Indexer, mut q: Vec<(String, String)>) -> String {
    let base = ix.url.split(['?', '#']).next().unwrap_or("");
    let (scheme, rest) = base.split_once("://").unwrap_or(("http", base));
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let path = if path.ends_with("/api") { path.to_string() } else { format!("{}/api", path.trim_end_matches('/')) };
    if !ix.api_key.is_empty() {
        q.retain(|(k, _)| k != "apikey");
        q.push(("apikey".into(), ix.api_key.clone()));
    }
    q.sort_by(|a, b| a.0.cmp(&b.0));
    let query = q.iter().map(|(k, v)| format!("{}={}", query_escape(k), query_escape(v))).collect::<Vec<_>>();
    format!("{scheme}://{authority}{path}?{}", query.join("&"))
}

fn fail(ix: &Indexer, code: &str, msg: &str) -> Error {
    Error::new(code, format!("{}: {msg}", url_host(&ix.url)))
}

fn sanitize(s: &str, secret: &str) -> String {
    let s = if secret.is_empty() { s.to_string() } else { s.replace(secret, "***") };
    clip(&s, 200)
}

/// The Newznab `<error code=… description=…/>` answer, if the body is one.
fn api_error(body: &str) -> Option<(String, String)> {
    let head = &body[..body.len().min(512)];
    let head = &head[..(0..=head.len()).rev().find(|i| head.is_char_boundary(*i)).unwrap_or(0)];
    if !head.trim().contains("<error") {
        return None;
    }
    let root = xml::parse(body)?;
    (root.name == "error").then(|| (root.attr("code").to_string(), root.attr("description").to_string()))
}

impl Torznab {
    fn get(&self, ix: &Indexer, q: Vec<(String, String)>) -> Result<String, Error> {
        let req = Request {
            method: "GET".into(),
            url: endpoint(ix, q),
            headers: vec![("User-Agent".into(), "Lain".into())],
            timeout: TIMEOUT,
            follow_redirects: true,
            cancel: None,
        };
        let res = self.http.send(&req).map_err(|_| fail(ix, "dependency-unavailable", "unreachable"))?;
        let status = res.status;
        let body = res.read_all(MAX_RESPONSE).map_err(|e| {
            let msg = if e.to_string().contains("too large") { "response too large" } else { "read failed" };
            fail(ix, "dependency-unavailable", msg)
        })?;
        let body = String::from_utf8_lossy(&body).into_owned();
        if let Some((code, desc)) = api_error(&body) {
            let code = if matches!(code.as_str(), "100" | "101" | "102") { "auth-failed" } else { "indexer-error" };
            return Err(fail(ix, code, &sanitize(&desc, &ix.api_key)));
        }
        match status {
            401 | 403 => Err(fail(ix, "auth-failed", "credentials rejected")),
            429 => Err(fail(ix, "rate-limited", "too many requests")),
            200 => Ok(body),
            s => Err(fail(ix, "dependency-unavailable", &format!("http {s}"))),
        }
    }

    pub fn caps(&self, ix: &Indexer) -> Result<IndexerCaps, Error> {
        let body = self.get(ix, vec![("t".into(), "caps".into())])?;
        parse_caps(&body).ok_or_else(|| fail(ix, "indexer-error", "caps response is not valid XML"))
    }

    pub fn search(&self, i: &IndexerSearchInput) -> Result<IndexerSearchOutput, Error> {
        let ix = &i.indexer;
        let mut q: Vec<(String, String)> = Vec::new();
        let mut func = "search";
        match i.kind.as_str() {
            SEARCH_TV => {
                func = "tvsearch";
                if i.season > 0 {
                    q.push(("season".into(), i.season.to_string()));
                }
                if i.episode > 0 {
                    q.push(("ep".into(), i.episode.to_string()));
                }
            }
            SEARCH_MOVIE => {
                func = "movie";
                if i.year > 0 {
                    q.push(("year".into(), i.year.to_string()));
                }
            }
            SEARCH_BOOK => func = "book",
            _ => {}
        }
        if let Some(c) = &ix.caps {
            // A plain search when the indexer lacks the function.
            if (func == "tvsearch" && !c.tv_search)
                || (func == "movie" && !c.movie_search)
                || (func == "book" && !c.book_search)
            {
                func = "search";
                q.clear();
            }
        }
        q.push(("t".into(), func.into()));
        let text = i.query.trim();
        if !text.is_empty() {
            q.push(("q".into(), clip(text, 200)));
        }
        let cats = if i.categories.is_empty() { &ix.categories } else { &i.categories };
        if !cats.is_empty() {
            q.push(("cat".into(), cats.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(",")));
        }
        if i.limit > 0 {
            q.push(("limit".into(), i.limit.min(MAX_RESULTS as i64).to_string()));
        }
        q.push(("extended".into(), "1".into()));
        let body = self.get(ix, q)?;
        let results = parse_rss(&body, ix).ok_or_else(|| fail(ix, "indexer-error", "results are not valid RSS"))?;
        Ok(IndexerSearchOutput { results })
    }
}

fn yes(n: Option<&Node>) -> bool {
    n.is_some_and(|n| n.attr("available").eq_ignore_ascii_case("yes"))
}

fn parse_caps(body: &str) -> Option<IndexerCaps> {
    let root = xml::parse(body)?;
    let s = root.child("searching");
    let lim = root.child("limits");
    let mut out = IndexerCaps {
        search: yes(s.and_then(|s| s.child("search"))),
        tv_search: yes(s.and_then(|s| s.child("tv-search"))),
        movie_search: yes(s.and_then(|s| s.child("movie-search"))),
        book_search: yes(s.and_then(|s| s.child("book-search"))),
        limit: match lim {
            Some(l) => xml::int(l.attr("max"))?,
            None => 0,
        },
        categories: Vec::new(),
    };
    if let Some(cats) = root.child("categories") {
        for c in cats.all("category") {
            let name = c.attr("name");
            out.categories.push(IndexerCategory { id: xml::int(c.attr("id"))?, name: clip(name, 100) });
            for sub in c.all("subcat") {
                out.categories.push(IndexerCategory {
                    id: xml::int(sub.attr("id"))?,
                    name: clip(&format!("{name}/{}", sub.attr("name")), 100),
                });
            }
            if out.categories.len() > 2000 {
                break;
            }
        }
    }
    Some(out)
}

fn safe_url(s: &str) -> String {
    let s = s.trim();
    if http_url(s) && s.len() <= 4096 { s.to_string() } else { String::new() }
}

/// Reads a Torznab/Newznab result feed; `None` when it is not one.
pub fn parse_rss(body: &str, ix: &Indexer) -> Option<Vec<SearchResult>> {
    let root = xml::parse(body)?;
    let mut out = Vec::new();
    let Some(channel) = root.child("channel") else { return Some(out) };
    for it in channel.all("item") {
        // Go decodes every item before filtering: a bad number anywhere
        // fails the feed.
        let size = xml::int(&it.text_of("size"))?;
        let enclosure = it.child("enclosure");
        let enc_len = match enclosure {
            Some(e) => xml::int(e.attr("length"))?,
            None => 0,
        };
        if out.len() >= MAX_RESULTS {
            continue;
        }
        let mut r = SearchResult {
            indexer_id: ix.id.clone(),
            title: clip(it.text_of("title").trim(), 300),
            guid: clip(&it.text_of("guid"), 500),
            comments_url: safe_url(&it.text_of("comments")),
            size,
            ..SearchResult::default()
        };
        if r.title.is_empty() {
            continue;
        }
        let enc_type = enclosure.map(|e| e.attr("type")).unwrap_or("");
        r.protocol = if ix.protocol == INDEXER_NEWZNAB || enc_type.contains("nzb") {
            PROTOCOL_USENET.into()
        } else {
            PROTOCOL_TORRENT.into()
        };
        let mut link = enclosure.map(|e| e.attr("url").to_string()).unwrap_or_default();
        if link.is_empty() {
            link = it.text_of("link");
        }
        if link.starts_with("magnet:") {
            r.magnet = clip(&link, 4096);
        } else {
            r.link = safe_url(&link);
        }
        if r.size == 0 {
            r.size = enc_len;
        }
        if let Some(ts) = parse_rfc1123(&it.text_of("pubDate")) {
            r.published_at = ts;
        }
        for a in it.all("attr") {
            let v = a.attr("value").trim();
            match a.attr("name").to_lowercase().as_str() {
                "seeders" => r.seeders = v.parse().unwrap_or(0),
                "peers" => r.peers = v.parse().unwrap_or(0),
                "size" => {
                    if let Ok(n) = v.parse::<i64>()
                        && r.size == 0
                    {
                        r.size = n;
                    }
                }
                "infohash" => {
                    if v.len() == 40 {
                        r.info_hash = v.to_lowercase();
                    }
                }
                "magneturl" => {
                    if v.starts_with("magnet:") {
                        r.magnet = clip(v, 4096);
                    }
                }
                "category" => {
                    if let Ok(n) = v.parse::<i64>()
                        && r.categories.len() < 20
                    {
                        r.categories.push(n);
                    }
                }
                "downloadvolumefactor" => r.freeleech = v == "0",
                _ => {}
            }
        }
        r.size = r.size.max(0);
        if r.link.is_empty() && r.magnet.is_empty() {
            continue;
        }
        out.push(r);
    }
    Some(out)
}

impl Provider for Torznab {
    fn id(&self) -> &str {
        TORZNAB_ID
    }

    fn capabilities(&self) -> Vec<String> {
        vec![CAP_INDEXER.into()]
    }

    fn health(&self) -> Result<(), Error> {
        Ok(())
    }

    fn invoke(&self, cap: &str, op: &str, input: Value) -> Result<Value, Error> {
        if cap != CAP_INDEXER {
            return Err(Error::invalid(format!("unsupported cap {cap}")));
        }
        let op = match op {
            "" if input.as_object().is_some_and(|o| o.keys().any(|k| k != "indexer")) => "search",
            "" => "caps",
            op => op,
        };
        match op {
            "caps" => {
                let i: IndexerCapsInput = serde_json::from_value(input)
                    .map_err(|_| Error::invalid("IndexerCapsInput or IndexerSearchInput required"))?;
                check(&i.indexer)?;
                Ok(json!(self.caps(&i.indexer)?))
            }
            "search" => {
                let i: IndexerSearchInput = serde_json::from_value(input)
                    .map_err(|_| Error::invalid("IndexerCapsInput or IndexerSearchInput required"))?;
                check(&i.indexer)?;
                Ok(json!(self.search(&i)?))
            }
            other => Err(Error::invalid(format!("capability {cap} has no op {other}"))),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;
    use crate::testsupport::{Fixture, Reply, serve};
    use lain_downloads::http::PlainHttp;

    // Fixture feeds use placeholder names only (D-010).
    pub const CAPS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<caps>
  <limits max="100" default="50"/>
  <searching>
    <search available="yes" supportedParams="q"/>
    <tv-search available="yes" supportedParams="q,season,ep"/>
    <movie-search available="no"/>
  </searching>
  <categories>
    <category id="5000" name="TV"><subcat id="5070" name="Anime"/></category>
    <category id="7000" name="Books"/>
  </categories>
</caps>"#;

    pub const FEED_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:torznab="http://torznab.com/schemas/2015/feed">
<channel>
  <item>
    <title>[Fansub-A] Show - 05 [1080p]</title>
    <guid>https://tracker-exemplo.invalid/t/1</guid>
    <link>https://tracker-exemplo.invalid/dl/1.torrent</link>
    <comments>https://tracker-exemplo.invalid/t/1</comments>
    <pubDate>Mon, 02 Jan 2006 15:04:05 -0700</pubDate>
    <size>734003200</size>
    <enclosure url="https://tracker-exemplo.invalid/dl/1.torrent" length="734003200" type="application/x-bittorrent"/>
    <torznab:attr name="seeders" value="42"/>
    <torznab:attr name="peers" value="50"/>
    <torznab:attr name="infohash" value="0123456789ABCDEF0123456789ABCDEF01234567"/>
    <torznab:attr name="category" value="5070"/>
    <torznab:attr name="downloadvolumefactor" value="0"/>
  </item>
  <item>
    <title>Show S01E02 720p</title>
    <link>magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567</link>
    <torznab:attr name="size" value="1000"/>
  </item>
  <item>
    <title>no link at all</title>
  </item>
  <item>
    <title>bad link</title>
    <link>javascript:alert(1)</link>
  </item>
</channel>
</rss>"#;

    pub fn fake(feeds: Arc<Mutex<HashMap<String, String>>>) -> Fixture {
        serve(move |s| {
            if s.path != "/api" {
                return Reply::status(404);
            }
            if s.param("apikey") != "secret-key" {
                return Reply::ok(
                    r#"<?xml version="1.0"?><error code="100" description="Incorrect user credentials secret-key"/>"#,
                );
            }
            match feeds.lock().unwrap().get(&s.param("t")) {
                Some(f) => Reply::ok(f.clone()),
                None => Reply::ok(r#"<error code="203" description="Function not available"/>"#),
            }
        })
    }

    fn default_feeds() -> Arc<Mutex<HashMap<String, String>>> {
        Arc::new(Mutex::new(HashMap::from([
            ("caps".to_string(), CAPS_XML.to_string()),
            ("search".to_string(), FEED_XML.to_string()),
            ("tvsearch".to_string(), FEED_XML.to_string()),
        ])))
    }

    fn ix(f: &Fixture, key: &str) -> Indexer {
        Indexer {
            id: "ix1".into(),
            name: "tracker-exemplo".into(),
            protocol: INDEXER_TORZNAB.into(),
            url: f.url.clone(),
            api_key: key.into(),
            categories: vec![5070],
            ..Default::default()
        }
    }

    fn provider() -> Torznab {
        Torznab::new(Arc::new(PlainHttp::new()))
    }

    #[test]
    fn caps() {
        let f = fake(default_feeds());
        let out = provider().invoke(CAP_INDEXER, "caps", json!({"indexer": ix(&f, "secret-key")})).unwrap();
        let c: IndexerCaps = serde_json::from_value(out).unwrap();
        assert!(c.search && c.tv_search && !c.movie_search && c.limit == 100, "{c:?}");
        assert_eq!(c.categories.len(), 3);
        assert_eq!(c.categories[1].name, "TV/Anime");
    }

    #[test]
    fn search_parses_feed() {
        let f = fake(default_feeds());
        let mut i = ix(&f, "secret-key");
        i.caps = Some(IndexerCaps { tv_search: true, ..Default::default() });
        let input = IndexerSearchInput {
            indexer: i,
            query: "Show".into(),
            kind: SEARCH_TV.into(),
            season: 1,
            episode: 2,
            limit: 20,
            ..Default::default()
        };
        let out = provider().invoke(CAP_INDEXER, "search", json!(input)).unwrap();
        let q = f.last_query();
        for want in ["t=tvsearch", "season=1", "ep=2", "cat=5070", "q=Show", "limit=20", "extended=1"] {
            assert!(q.contains(want), "query {q:?} lacks {want:?}");
        }
        let res: IndexerSearchOutput = serde_json::from_value(out).unwrap();
        assert_eq!(res.results.len(), 2, "{:?}", res.results);
        let a = &res.results[0];
        assert_eq!(a.title, "[Fansub-A] Show - 05 [1080p]");
        assert_eq!((a.seeders, a.peers, a.size), (42, 50, 734003200));
        assert_eq!(a.info_hash, "0123456789abcdef0123456789abcdef01234567");
        assert!(!a.link.is_empty() && a.freeleech && a.protocol == PROTOCOL_TORRENT);
        assert_eq!(a.published_at, 1136239445);
        assert_eq!(a.categories, vec![5070]);
        assert!(!res.results[1].magnet.is_empty());
        assert_eq!(res.results[1].size, 1000);
    }

    #[test]
    fn search_falls_back_without_function() {
        let f = fake(default_feeds());
        let mut i = ix(&f, "secret-key");
        i.caps = Some(IndexerCaps { search: true, ..Default::default() });
        let input = IndexerSearchInput {
            indexer: i,
            query: "Film".into(),
            kind: SEARCH_MOVIE.into(),
            year: 2001,
            ..Default::default()
        };
        provider().invoke(CAP_INDEXER, "search", json!(input)).unwrap();
        let q = f.last_query();
        assert!(q.contains("t=search") && !q.contains("year="), "{q}");
    }

    #[test]
    fn errors_never_leak_the_key() {
        let feeds = default_feeds();
        let f = fake(feeds.clone());
        let e = provider().invoke(CAP_INDEXER, "caps", json!({"indexer": ix(&f, "wrong-key")})).unwrap_err();
        assert_eq!(e.code, "auth-failed");
        assert!(!e.msg.contains("wrong-key"), "{}", e.msg);
        feeds.lock().unwrap().clear();
        let e = provider()
            .invoke(
                CAP_INDEXER,
                "search",
                json!(IndexerSearchInput { indexer: ix(&f, "secret-key"), ..Default::default() }),
            )
            .unwrap_err();
        assert!(!e.to_string().contains("secret-key"), "{e}");
        // Unreachable: the host only.
        let mut dead = ix(&f, "secret-key");
        dead.url = "http://127.0.0.1:9/x".into();
        let e = provider().invoke(CAP_INDEXER, "caps", json!({"indexer": dead})).unwrap_err();
        assert_eq!(e.to_string(), "dependency-unavailable: 127.0.0.1:9: unreachable");
    }

    #[test]
    fn rejects_bad_indexers() {
        let p = provider();
        let e =
            p.invoke(CAP_INDEXER, "caps", json!({"indexer": {"id":"","name":"","protocol":"rss","url":"http://x"}}));
        assert_eq!(e.unwrap_err().code, "unsupported-protocol");
        let e = p.invoke(
            CAP_INDEXER,
            "caps",
            json!({"indexer": {"id":"","name":"","protocol":"torznab","url":"file:///etc/passwd"}}),
        );
        assert_eq!(e.unwrap_err().code, "invalid-message");
        assert_eq!(p.invoke("lain.nope@1", "caps", json!({})).unwrap_err().code, "invalid-message");
        assert_eq!(p.invoke(CAP_INDEXER, "caps", json!(42)).unwrap_err().code, "invalid-message");
        assert_eq!(p.invoke(CAP_INDEXER, "bogus", json!({})).unwrap_err().code, "invalid-message");
        assert!(p.health().is_ok());
    }

    #[test]
    fn rss_never_panics_and_stays_safe() {
        // Go fuzzes parseRSS; a seeded mutation walk over the fixtures.
        let ix = Indexer { id: "x".into(), protocol: INDEXER_TORZNAB.into(), ..Default::default() };
        let seeds = [FEED_XML, "<rss><channel><item><title>x</title><link>http://a/b</link></item></channel></rss>"];
        let mut state = 0x9e3779b97f4a7c15u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for round in 0..3000 {
            let mut b = seeds[round % 2].as_bytes().to_vec();
            for _ in 0..(next() % 8) {
                let i = (next() as usize) % b.len().max(1);
                match next() % 3 {
                    0 => b[i] = next() as u8,
                    1 => {
                        b.remove(i);
                    }
                    _ => b.insert(i, b"<>&\"'/="[(next() % 7) as usize]),
                }
            }
            let s = String::from_utf8_lossy(&b);
            if let Some(res) = parse_rss(&s, &ix) {
                assert!(res.len() <= MAX_RESULTS);
                for r in res {
                    assert!(!r.link.is_empty() || !r.magnet.is_empty());
                    assert!(r.link.is_empty() || r.link.starts_with("http"), "{}", r.link);
                }
            }
            let _ = parse_caps(&s);
        }
    }
}
