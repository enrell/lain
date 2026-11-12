//! Small shared helpers: names, title keys, ids, clocks.

use std::path::{Component, Path};
use std::time::{SystemTime, UNIX_EPOCH};

use lain_api::Release;

use crate::error::{AcqError, CODE_INVALID, Result};

/// Unix seconds.
pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// 16 random hex digits.
pub fn new_id() -> String {
    let mut b = [0u8; 8];
    let _ = getrandom::fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Collapses whitespace and refuses control characters or more than
/// `max` characters.
pub fn clean_name(s: &str, max: usize) -> Result<String> {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() > max {
        return Err(AcqError::new(CODE_INVALID, format!("name must be text up to {max} characters")));
    }
    if s.chars().any(|c| (c as u32) < 0x20) {
        return Err(AcqError::new(CODE_INVALID, "name must not contain control characters"));
    }
    Ok(s)
}

/// The catalog's title key: lower case, whitespace collapsed.
pub fn title_key(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Matches titles across punctuation and case: "Show: Name" and "Show
/// Name" are one title.
pub fn loose_key(s: &str) -> String {
    title_key(s)
        .split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || (c as u32) > 127))
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Reads `rel` as the first of its alternatives whose title `known`
/// accepts, when its own title is not one (D-279). A parser cannot tell
/// "Show 2" the sequel from episode 2 of "Show"; the caller that searched
/// for a title, monitors one or imports into a library can. The identity
/// moves as a whole (title, year, season, episodes, numbering) and every
/// other field stays. The alternatives are dropped either way: they are
/// advice for this decision, never part of a stored release. An
/// alternative without a word in its title names nothing, so a `known`
/// that matches loose keys cannot take it for an empty one.
pub fn settle(mut rel: Release, known: impl Fn(&str) -> bool) -> Release {
    let alternatives = std::mem::take(&mut rel.alternatives);
    if rel.title.is_empty() || known(&rel.title) {
        return rel;
    }
    if let Some(alt) = alternatives.into_iter().find(|a| !loose_key(&a.title).is_empty() && known(&a.title)) {
        if !alt.episodes.is_empty() {
            rel.season_pack = false;
        }
        rel.title = alt.title;
        rel.year = alt.year;
        rel.season = alt.season;
        rel.episodes = alt.episodes;
        rel.absolute = alt.absolute;
    }
    rel
}

/// At most `n` bytes, cut at a character boundary (Go cuts bytes and can
/// split a UTF-8 sequence).
pub fn clip(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut end = n;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// Lower-case extension with its dot (`".mkv"`), `""` without one.
pub fn ext_lower(p: &str) -> String {
    let base = p.rsplit(['/', '\\']).next().unwrap_or(p);
    match base.rfind('.') {
        Some(i) => base[i..].to_lowercase(),
        None => String::new(),
    }
}

/// The last path element (Go `filepath.Base` on a clean path).
pub fn base_name(p: &str) -> String {
    Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string())
}

/// Lexical clean: no `.` elements, `..` resolved where possible.
pub fn clean_path(p: &Path) -> std::path::PathBuf {
    let mut out = std::path::PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `p` strictly below `root` (Go `within`).
pub fn within(root: &Path, p: &Path) -> bool {
    let (root, p) = (clean_path(root), clean_path(p));
    p != root && p.starts_with(&root)
}

/// `p` is `root` or below it.
pub fn within_or_equal(root: &Path, p: &Path) -> bool {
    let (root, p) = (clean_path(root), clean_path(p));
    p.starts_with(&root)
}

/// Whether `s` is an absolute http(s) URL with a host and no user info.
pub fn http_url(s: &str) -> bool {
    let Some((scheme, rest)) = s.split_once("://") else { return false };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return false;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    !authority.is_empty() && !authority.contains('@') && !authority.starts_with(':')
}

/// The host (with port) of a URL, for error texts that must never carry
/// the query (API keys).
pub fn url_host(s: &str) -> String {
    let rest = s.split_once("://").map(|(_, r)| r).unwrap_or(s);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    authority.rsplit('@').next().unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lain_api::ReleaseIdentity;

    fn alt(title: &str, season: i64, episodes: &[i64]) -> ReleaseIdentity {
        ReleaseIdentity {
            title: title.into(),
            season,
            episodes: episodes.to_vec(),
            absolute: season == 0 && !episodes.is_empty(),
            ..Default::default()
        }
    }

    #[test]
    fn settle_takes_the_first_alternative_with_a_known_title() {
        let wanted = |t: &str| loose_key(t) == "quiet harbor";
        let read = Release {
            title: "Quiet Harbor 2".into(),
            year: 2014,
            group: "Fansub-A".into(),
            resolution: "1080p".into(),
            season_pack: true,
            alternatives: vec![alt("Quiet", 0, &[]), alt("Quiet.Harbor", 0, &[2]), alt("Quiet Harbor", 2, &[])],
            ..Default::default()
        };
        let r = settle(read.clone(), wanted);
        assert_eq!((r.title.as_str(), r.season, r.episodes.clone(), r.absolute), ("Quiet.Harbor", 0, vec![2], true));
        assert_eq!(r.year, 0, "the identity moves as a whole");
        assert!(!r.season_pack, "an episode is not a pack");
        assert_eq!((r.group.as_str(), r.resolution.as_str()), ("Fansub-A", "1080p"), "the rest stays");
        assert!(r.alternatives.is_empty());

        // A season without episodes leaves what the name said about a pack.
        let r = settle(read.clone(), |t| t == "Quiet Harbor");
        assert_eq!((r.title.as_str(), r.season, r.episodes.len(), r.season_pack), ("Quiet Harbor", 2, 0, true));

        // A reading without a word names nothing, even to a caller whose
        // own key is empty.
        let odd = Release { title: "Quiet Harbor 2".into(), alternatives: vec![alt(" - ", 0, &[3])], ..read.clone() };
        let r = settle(odd, |t| loose_key(t).is_empty());
        assert_eq!((r.title.as_str(), r.episodes.len()), ("Quiet Harbor 2", 0));

        // The stated title is known: nothing moves, even with a known alternative.
        let r = settle(read.clone(), |t| t.starts_with("Quiet Harbor"));
        assert_eq!((r.title.as_str(), r.year, r.season_pack), ("Quiet Harbor 2", 2014, true));
        assert!(r.alternatives.is_empty());

        // No alternative is known: the answer stands, the advice is dropped.
        let r = settle(read.clone(), |t| t == "Silver Tide");
        assert_eq!(r, Release { alternatives: Vec::new(), ..read.clone() });

        // A decline stays a decline.
        let declined = Release { alternatives: vec![alt("Quiet Harbor", 0, &[2])], ..Default::default() };
        assert_eq!(settle(declined, wanted), Release::default());
    }

    #[test]
    fn keys_and_names() {
        assert_eq!(loose_key("Show: Name"), loose_key("show name"));
        assert_eq!(loose_key("Frieren - Beyond"), "frieren beyond");
        assert_eq!(clean_name("  a \t b ", 60).unwrap(), "a b");
        assert!(clean_name("a\u{1}b", 60).is_err());
        assert!(clean_name(&"x".repeat(61), 60).is_err());
        assert_eq!(clip("aé", 2), "a");
        assert!(http_url("http://h:1/x?y"));
        assert!(!http_url("http://u:p@h/x"));
        assert!(!http_url("ftp://h/"));
        assert!(!http_url("http:///x"));
        assert_eq!(url_host("https://h.example:9/api?apikey=s"), "h.example:9");
        assert!(within(Path::new("/m/lib"), Path::new("/m/lib/a/b")));
        assert!(!within(Path::new("/m/lib"), Path::new("/m/lib")));
        assert!(!within(Path::new("/m/lib"), Path::new("/m/lib2/a")));
        assert!(!within(Path::new("/m/lib"), Path::new("/m/lib/../x")));
        assert!(within_or_equal(Path::new("/m/lib/"), Path::new("/m/lib")));
        assert_eq!(ext_lower("/a/B.MKV"), ".mkv");
        assert_eq!(ext_lower("/a.b/c"), "");
    }
}

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// RFC 1123 dates as RSS feeds write them (`Mon, 02 Jan 2006 15:04:05
/// -0700`, or a zone name, read as UTC like Go does for unknown
/// abbreviations). Unix seconds.
pub fn parse_rfc1123(s: &str) -> Option<i64> {
    let s = s.trim();
    let (_, rest) = s.split_once(", ")?;
    let f: Vec<&str> = rest.split_whitespace().collect();
    if f.len() != 5 {
        return None;
    }
    let day: i64 = f[0].parse().ok().filter(|d| (1..=31).contains(d) && f[0].len() == 2)?;
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let month = MONTHS.iter().position(|m| *m == f[1])? as i64 + 1;
    let year: i64 = f[2].parse().ok().filter(|_| f[2].len() == 4)?;
    let hms: Vec<i64> = f[3].split(':').map(|x| x.parse().ok()).collect::<Option<Vec<_>>>()?;
    if hms.len() != 3 || hms[0] > 23 || hms[1] > 59 || hms[2] > 59 || hms.iter().any(|x| *x < 0) {
        return None;
    }
    let zone = f[4];
    let offset = if let Some(sign) = zone.chars().next().filter(|c| *c == '+' || *c == '-') {
        let digits = &zone[1..];
        if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let v = digits[..2].parse::<i64>().ok()? * 3600 + digits[2..].parse::<i64>().ok()? * 60;
        if sign == '-' { -v } else { v }
    } else if zone.len() >= 3 && zone.bytes().all(|b| b.is_ascii_uppercase()) {
        0
    } else {
        return None;
    };
    Some(days_from_civil(year, month, day) * 86400 + hms[0] * 3600 + hms[1] * 60 + hms[2] - offset)
}

#[cfg(test)]
mod date_tests {
    use super::parse_rfc1123;

    #[test]
    fn rfc1123() {
        assert_eq!(parse_rfc1123("Mon, 02 Jan 2006 15:04:05 -0700"), Some(1136239445));
        assert_eq!(parse_rfc1123("Mon, 02 Jan 2006 22:04:05 GMT"), Some(1136239445));
        assert_eq!(parse_rfc1123("Thu, 01 Jan 1970 00:00:00 +0000"), Some(0));
        assert_eq!(parse_rfc1123("yesterday"), None);
        assert_eq!(parse_rfc1123("Mon, 2 Jan 2006 15:04:05 -0700"), None);
    }
}
