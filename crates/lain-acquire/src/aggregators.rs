//! Prowlarr and Jackett as presets (D-290): given an aggregator's base
//! URL and API key, ask it which indexers it serves and answer the
//! Torznab/Newznab feed of each, so the owner adds them without copying
//! feed URLs by hand. Read-only towards the aggregator; nothing is
//! stored here.
//!
//! - Prowlarr: `GET {base}/api/v1/indexer` with `X-Api-Key`; each
//!   indexer's feed is `{base}/{id}/api` (Newznab for usenet, Torznab
//!   for torrents), served with the same key.
//! - Jackett: the Torznab `t=indexers&configured=true` call on its
//!   `all` indexer (the key-authenticated listing; the JSON one needs
//!   the admin cookie); each feed is
//!   `{base}/api/v2.0/indexers/{id}/results/torznab/`.
//!
//! Jackett takes its key only in the query, so errors name the host
//! and never the URL.

use std::time::Duration;

use serde::Deserialize;

use lain_api::acquire::{
    DiscoverInput, DiscoveredIndexer, INDEXER_NEWZNAB, INDEXER_TORZNAB, SOURCE_JACKETT, SOURCE_PROWLARR,
};
use lain_downloads::http::{Http, Request};

use crate::errf;
use crate::error::{CODE_FETCH, CODE_INVALID, Result};
use crate::torznab::query_escape;
use crate::util::{clip, http_url, url_host};
use crate::xml;

const MAX_RESPONSE: usize = 4 << 20;
/// More than any aggregator setup we know of; a bound, not a target.
const MAX_INDEXERS: usize = 500;
const TIMEOUT: Duration = Duration::from_secs(20);

/// The base URL as the aggregator's own UI shows it: scheme, host,
/// port and an optional URL base, no query and no trailing slash.
pub fn base_url(url: &str) -> Result<String> {
    let base = url.trim().split(['?', '#']).next().unwrap_or("").trim_end_matches('/').to_string();
    if !http_url(&base) {
        return Err(errf!(CODE_INVALID, "the base URL must be http(s), like http://127.0.0.1:9696"));
    }
    Ok(base)
}

fn check(input: &DiscoverInput) -> Result<String> {
    if input.source != SOURCE_PROWLARR && input.source != SOURCE_JACKETT {
        return Err(errf!(CODE_INVALID, "source must be prowlarr or jackett"));
    }
    let key = input.api_key.trim();
    if key.is_empty() {
        return Err(errf!(CODE_INVALID, "api key required (Settings › General in {})", input.source));
    }
    if key.len() > 512 {
        return Err(errf!(CODE_INVALID, "api key too long"));
    }
    base_url(&input.url)
}

/// The indexers `input`'s aggregator serves, in its order.
pub fn discover(http: &dyn Http, input: &DiscoverInput) -> Result<Vec<DiscoveredIndexer>> {
    let base = check(input)?;
    let key = input.api_key.trim();
    let (url, headers) = if input.source == SOURCE_PROWLARR {
        (format!("{base}/api/v1/indexer"), vec![("X-Api-Key".to_string(), key.to_string())])
    } else {
        (
            format!(
                "{base}/api/v2.0/indexers/all/results/torznab/api?apikey={}&configured=true&t=indexers",
                query_escape(key)
            ),
            Vec::new(),
        )
    };
    let body = get(http, &url, headers, &base)?;
    let mut out =
        if input.source == SOURCE_PROWLARR { parse_prowlarr(&body, &base) } else { parse_jackett(&body, &base) }
            .ok_or_else(|| {
                errf!(CODE_FETCH, "{}: the answer is not a {} indexer list", url_host(&base), input.source)
            })?;
    out.truncate(MAX_INDEXERS);
    Ok(out)
}

fn get(http: &dyn Http, url: &str, mut headers: Vec<(String, String)>, base: &str) -> Result<String> {
    let host = url_host(base);
    headers.push(("User-Agent".into(), "Lain".into()));
    headers.push(("Accept".into(), "application/json, application/xml".into()));
    let req = Request {
        method: "GET".into(),
        url: url.into(),
        headers,
        timeout: TIMEOUT,
        follow_redirects: true,
        cancel: None,
    };
    // The URL may carry the key: report the host only.
    let res = http.send(&req).map_err(|_| errf!(CODE_FETCH, "{host} did not answer"))?;
    match res.status {
        200 => {}
        401 | 403 => return Err(errf!("auth-failed", "{host}: the API key was rejected")),
        404 => return Err(errf!(CODE_FETCH, "{host}: no such API here; check the base URL and the preset")),
        s => return Err(errf!(CODE_FETCH, "{host} answered {s}")),
    }
    let body =
        res.read_all(MAX_RESPONSE).map_err(|_| errf!(CODE_FETCH, "{host}: the answer is unreadable or too large"))?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

fn privacy(s: &str) -> String {
    match s.to_ascii_lowercase().replace(['-', '_'], "").as_str() {
        "public" => "public".into(),
        "private" => "private".into(),
        "semiprivate" => "semi-private".into(),
        _ => String::new(),
    }
}

/// A source id goes into a URL path: digits, letters and `-_.` only.
fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 100
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        && id != "."
        && id != ".."
}

fn parse_prowlarr(body: &str, base: &str) -> Option<Vec<DiscoveredIndexer>> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Row {
        id: i64,
        #[serde(default)]
        name: String,
        #[serde(default)]
        protocol: String,
        #[serde(default)]
        enable: bool,
        #[serde(default)]
        privacy: String,
    }
    let rows: Vec<Row> = serde_json::from_str(body).ok()?;
    Some(
        rows.into_iter()
            .filter(|r| r.id > 0 && (r.protocol == "torrent" || r.protocol == "usenet"))
            .map(|r| DiscoveredIndexer {
                source_id: r.id.to_string(),
                name: clip(r.name.trim(), 60),
                protocol: if r.protocol == "usenet" { INDEXER_NEWZNAB } else { INDEXER_TORZNAB }.into(),
                url: format!("{base}/{}/api", r.id),
                privacy: privacy(&r.privacy),
                enabled: r.enable,
                added_as: String::new(),
            })
            .collect(),
    )
}

fn parse_jackett(body: &str, base: &str) -> Option<Vec<DiscoveredIndexer>> {
    let root = xml::parse(body)?;
    if root.name == "error" {
        return None;
    }
    if root.name != "indexers" {
        return None;
    }
    Some(
        root.all("indexer")
            .filter(|n| !n.attr("configured").eq_ignore_ascii_case("false"))
            .filter(|n| safe_id(n.attr("id")))
            .map(|n| {
                let id = n.attr("id").to_string();
                let title = n.text_of("title");
                DiscoveredIndexer {
                    name: clip(if title.trim().is_empty() { &id } else { title.trim() }, 60),
                    protocol: INDEXER_TORZNAB.into(),
                    url: format!("{base}/api/v2.0/indexers/{id}/results/torznab/"),
                    privacy: privacy(&n.text_of("type")),
                    enabled: true,
                    added_as: String::new(),
                    source_id: id,
                }
            })
            .collect(),
    )
}

/// Two feed URLs name one feed when they differ only by a trailing
/// `/` or `/api` (the Torznab client adds `/api` itself).
pub fn same_feed(a: &str, b: &str) -> bool {
    fn norm(u: &str) -> String {
        let u = u.trim().split(['?', '#']).next().unwrap_or("").trim_end_matches('/');
        u.strip_suffix("/api").unwrap_or(u).trim_end_matches('/').to_ascii_lowercase()
    }
    norm(a) == norm(b)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::testsupport::{Reply, serve};
    use lain_downloads::http::PlainHttp;

    // Placeholder names only (AGENTS.md naming hygiene).
    const PROWLARR: &str = r#"[
      {"id": 1, "name": "tracker-exemplo", "protocol": "torrent", "enable": true, "privacy": "public"},
      {"id": 7, "name": "usenet-exemplo", "protocol": "usenet", "enable": false, "privacy": "private"},
      {"id": 9, "name": "odd", "protocol": "unknown", "enable": true}
    ]"#;
    const JACKETT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<indexers>
  <indexer id="tracker-exemplo" configured="true"><title>Tracker Exemplo</title><type>semi-private</type></indexer>
  <indexer id="../evil" configured="true"><title>Bad id</title></indexer>
  <indexer id="other" configured="false"><title>Not set up</title></indexer>
</indexers>"#;

    fn http() -> Arc<dyn Http> {
        Arc::new(PlainHttp::new())
    }

    #[test]
    fn prowlarr_lists_feeds_with_its_key_in_a_header() {
        let fx = serve(|s| {
            let key = s.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("x-api-key")).map(|(_, v)| v.as_str());
            if s.path == "/prowlarr/api/v1/indexer" && key == Some("k-1") {
                Reply::ok(PROWLARR)
            } else {
                Reply::status(401)
            }
        });
        let input =
            DiscoverInput { source: "prowlarr".into(), url: format!("{}/prowlarr/", fx.url), api_key: "k-1".into() };
        let got = discover(http().as_ref(), &input).unwrap();
        assert_eq!(got.len(), 2, "unknown protocols are left out");
        assert_eq!(got[0].url, format!("{}/prowlarr/1/api", fx.url));
        assert_eq!((got[0].protocol.as_str(), got[0].privacy.as_str(), got[0].enabled), ("torznab", "public", true));
        assert_eq!((got[1].protocol.as_str(), got[1].source_id.as_str(), got[1].enabled), ("newznab", "7", false));
        assert_eq!(fx.last_query(), "", "the key never goes in Prowlarr's URL");

        let wrong = DiscoverInput { api_key: "nope".into(), ..input };
        let e = discover(http().as_ref(), &wrong).unwrap_err();
        assert_eq!(e.code, "auth-failed");
        assert!(!e.msg.contains("nope"));
    }

    #[test]
    fn jackett_lists_configured_indexers_with_safe_ids() {
        let fx = serve(|s| {
            if s.path == "/api/v2.0/indexers/all/results/torznab/api"
                && s.param("t") == "indexers"
                && s.param("apikey") == "k-2"
            {
                Reply::ok(JACKETT)
            } else {
                Reply::ok(r#"<error code="100" description="Invalid API Key"/>"#)
            }
        });
        let input = DiscoverInput { source: "jackett".into(), url: fx.url.clone(), api_key: "k-2".into() };
        let got = discover(http().as_ref(), &input).unwrap();
        assert_eq!(got.len(), 1, "unconfigured and unsafe ids are left out: {got:?}");
        assert_eq!(got[0].url, format!("{}/api/v2.0/indexers/tracker-exemplo/results/torznab/", fx.url));
        assert_eq!((got[0].name.as_str(), got[0].privacy.as_str()), ("Tracker Exemplo", "semi-private"));
        assert_eq!(fx.last.lock().unwrap()[0].param("configured"), "true");

        let wrong = DiscoverInput { api_key: "secret-wrong".into(), ..input };
        let e = discover(http().as_ref(), &wrong).unwrap_err();
        assert!(!e.msg.contains("secret-wrong") && e.msg.contains("jackett"), "{e}");
    }

    #[test]
    fn inputs_are_checked_before_any_call() {
        let ok = |s: &str, u: &str, k: &str| {
            discover(http().as_ref(), &DiscoverInput { source: s.into(), url: u.into(), api_key: k.into() })
        };
        assert_eq!(ok("sonarr", "http://127.0.0.1:9", "k").unwrap_err().code, CODE_INVALID);
        assert_eq!(ok("prowlarr", "ftp://127.0.0.1:9", "k").unwrap_err().code, CODE_INVALID);
        assert_eq!(ok("prowlarr", "http://user@127.0.0.1:9", "k").unwrap_err().code, CODE_INVALID);
        assert_eq!(ok("jackett", "http://127.0.0.1:9", " ").unwrap_err().code, CODE_INVALID);
        assert_eq!(ok("jackett", "http://127.0.0.1:9", &"k".repeat(513)).unwrap_err().code, CODE_INVALID);
        // A closed port: reachable error, host only.
        let e = ok("jackett", "http://127.0.0.1:9", "secret").unwrap_err();
        assert_eq!(e.code, CODE_FETCH);
        assert!(!e.msg.contains("secret"));
    }

    #[test]
    fn feeds_compare_without_the_api_suffix() {
        assert!(same_feed("http://h:1/x/1/api", "http://h:1/x/1/"));
        assert!(same_feed(
            "http://h:1/api/v2.0/indexers/a/results/torznab/",
            "http://H:1/api/v2.0/indexers/a/results/torznab/api?apikey=k"
        ));
        assert!(!same_feed("http://h:1/x/1/api", "http://h:1/x/2/api"));
        assert_eq!(base_url(" http://h:9696/prowlarr/?x=1 ").unwrap(), "http://h:9696/prowlarr");
    }
}
