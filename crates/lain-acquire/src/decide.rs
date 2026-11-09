//! Quality profiles and the one decision engine (A-19, A-20). Every
//! candidate (manual search, RSS, automatic search) is judged by
//! [`evaluate`]; automation only grabs accepted ones, best score first.

use lain_api::Release;
use lain_api::acquire::{HI_EXCLUDE, HI_INCLUDE, HI_PREFER, PROTOCOL_TORRENT, PROTOCOL_USENET, Profile, SearchResult};

use crate::errf;
use crate::error::{CODE_INVALID, Result};
use crate::util::clean_name;

/// Known resolutions, best first.
pub const KNOWN_RESOLUTIONS: &[&str] = &["2160p", "1080p", "720p", "576p", "540p", "480p"];
const KNOWN_SOURCES: &[&str] = &["bluray", "web", "hdtv", "dvd"];

/// Created on first use: 720p or better, upgrade to 1080p, one seeder.
pub fn default_profile() -> Profile {
    Profile {
        id: "default".into(),
        name: "Default (720p+, upgrade to 1080p)".into(),
        resolutions: vec!["2160p".into(), "1080p".into(), "720p".into()],
        cutoff: "1080p".into(),
        min_seeders: 1,
        prefer_proper: true,
        ..Profile::default()
    }
}

fn clean_list(xs: &[String], max: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for x in xs {
        let x = x.trim();
        if x.is_empty() || out.len() >= max || out.iter().any(|o| o.to_lowercase() == x.to_lowercase()) {
            continue;
        }
        out.push(x.to_string());
    }
    out
}

pub fn rank_of<S: AsRef<str>>(list: &[S], v: &str) -> Option<usize> {
    list.iter().position(|x| x.as_ref().to_lowercase() == v.to_lowercase())
}

/// Normalizes a profile or explains why it cannot be saved.
pub fn validate_profile(mut p: Profile) -> Result<Profile> {
    let name = clean_name(&p.name, 60)?;
    if name.is_empty() {
        return Err(errf!(CODE_INVALID, "profile name required"));
    }
    p.name = name;
    let mut seen: Vec<String> = Vec::new();
    for r in p.resolutions.iter_mut() {
        let v = r.trim().to_lowercase();
        if rank_of(KNOWN_RESOLUTIONS, &v).is_none() {
            return Err(errf!(CODE_INVALID, "unknown resolution {v:?}"));
        }
        if seen.contains(&v) {
            return Err(errf!(CODE_INVALID, "resolution {v} listed twice"));
        }
        seen.push(v.clone());
        *r = v;
    }
    if p.resolutions.is_empty() {
        return Err(errf!(CODE_INVALID, "allow at least one resolution"));
    }
    p.cutoff = p.cutoff.trim().to_lowercase();
    if p.cutoff.is_empty() {
        p.cutoff = p.resolutions[0].clone();
    }
    if !seen.contains(&p.cutoff) {
        return Err(errf!(CODE_INVALID, "the cutoff must be one of the allowed resolutions"));
    }
    p.sources = clean_list(&p.sources, 10);
    for s in p.sources.iter_mut() {
        let v = s.to_lowercase();
        if !KNOWN_SOURCES.contains(&v.as_str()) {
            return Err(errf!(CODE_INVALID, "unknown source {v:?}"));
        }
        *s = v;
    }
    p.preferred_groups = clean_list(&p.preferred_groups, 50);
    p.blocked_groups = clean_list(&p.blocked_groups, 50);
    p.blocked_words = clean_list(&p.blocked_words, 50);
    if p.min_seeders < 0 || p.min_size_mb < 0 || p.max_size_mb < 0 {
        return Err(errf!(CODE_INVALID, "limits cannot be negative"));
    }
    if p.max_size_mb > 0 && p.min_size_mb > p.max_size_mb {
        return Err(errf!(CODE_INVALID, "minimum size is above the maximum"));
    }
    let mut langs: Vec<String> = Vec::new();
    for l in clean_list(&p.subtitle_languages, 10) {
        let code = lain_subtitle::language(&l);
        if code.is_empty() {
            return Err(errf!(CODE_INVALID, "unknown subtitle language {l:?}"));
        }
        if !langs.contains(&code) {
            langs.push(code);
        }
    }
    p.subtitle_languages = langs;
    match p.subtitle_hi.as_str() {
        "" => p.subtitle_hi = HI_INCLUDE.into(),
        HI_INCLUDE | HI_PREFER | HI_EXCLUDE => {}
        _ => return Err(errf!(CODE_INVALID, "subtitle_hi must be include, prefer or exclude")),
    }
    Ok(p)
}

/// The verdict on one candidate.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Decision {
    pub accepted: bool,
    pub rejections: Vec<String>,
    pub score: i64,
    pub quality: String,
}

fn has_group(groups: &[String], rel: &Release, title: &str) -> Option<String> {
    let lt = title.to_lowercase();
    groups
        .iter()
        .find(|g| {
            let lg = g.to_lowercase();
            rel.group.to_lowercase() == lg || lt.ends_with(&format!("-{lg}")) || lt.contains(&format!("[{lg}]"))
        })
        .cloned()
}

/// `w` as a whole token of `s` (separators: anything not a letter or
/// digit), so "cam" does not hit "camera".
pub fn contains_word(s: &str, w: &str) -> bool {
    if w.is_empty() {
        return false;
    }
    let b = s.as_bytes();
    let mut i = 0;
    while let Some(j) = s[i..].find(w) {
        let j = i + j;
        let before = j == 0 || !b[j - 1].is_ascii_alphanumeric();
        let end = j + w.len();
        let after = end == b.len() || !b[end].is_ascii_alphanumeric();
        if before && after {
            return true;
        }
        i = j + 1;
        while i < s.len() && !s.is_char_boundary(i) {
            i += 1;
        }
        if i >= s.len() {
            return false;
        }
    }
    false
}

fn log2_score(factor: f64, n: i64) -> i64 {
    (factor * ((n.max(0) + 1) as f64).log2()) as i64
}

/// Judges a candidate against a profile. `units` is how many episodes,
/// chapters or movies the release covers (size bounds are per unit).
pub fn evaluate(p: &Profile, r: &SearchResult, rel: &Release, units: i64) -> Decision {
    let mut d = Decision { quality: rel.resolution.clone(), ..Decision::default() };
    let units = units.max(1);
    if r.protocol == PROTOCOL_USENET {
        d.rejections.push("usenet: no usenet client yet".into());
    }
    let rank = rank_of(&p.resolutions, &rel.resolution);
    if rel.resolution.is_empty() {
        d.rejections.push("unknown resolution".into());
    } else if rank.is_none() {
        d.rejections.push(format!("resolution {} not allowed", rel.resolution));
    }
    if !p.sources.is_empty() && rank_of(&p.sources, &rel.source).is_none() {
        let src = if rel.source.is_empty() { "unstated" } else { rel.source.as_str() };
        d.rejections.push(format!("source {src} not allowed"));
    }
    if let Some(g) = has_group(&p.blocked_groups, rel, &r.title) {
        d.rejections.push(format!("blocked group {g}"));
    }
    let lt = r.title.to_lowercase();
    for w in &p.blocked_words {
        if contains_word(&lt, &w.to_lowercase()) {
            d.rejections.push(format!("blocked word {w:?}"));
        }
    }
    if r.protocol == PROTOCOL_TORRENT && r.seeders < p.min_seeders.max(0) {
        d.rejections.push(format!("too few seeders ({} < {})", r.seeders, p.min_seeders));
    }
    if r.size > 0 {
        let per = (r.size / units) >> 20;
        if p.min_size_mb > 0 && per < p.min_size_mb {
            d.rejections.push(format!("too small: {per} MB per unit, minimum {}", p.min_size_mb));
        }
        if p.max_size_mb > 0 && per > p.max_size_mb {
            d.rejections.push(format!("too large: {per} MB per unit, maximum {}", p.max_size_mb));
        }
    }
    if let Some(rank) = rank {
        d.score += (p.resolutions.len() - rank) as i64 * 100;
    }
    if has_group(&p.preferred_groups, rel, &r.title).is_some() {
        d.score += 150;
    }
    if p.prefer_proper && (rel.proper || rel.repack || rel.version > 1) {
        d.score += 20;
    }
    d.score += log2_score(5.0, r.seeders);
    if r.freeleech {
        d.score += 2;
    }
    d.accepted = d.rejections.is_empty();
    d
}

/// The Phase 1 manual-search score (Go `rank`): fit first, then swarm
/// health and quality.
pub fn search_score(rel: &Release, r: &SearchResult) -> i64 {
    let res = match rel.resolution.as_str() {
        "2160p" => 40,
        "1080p" => 30,
        "720p" => 20,
        "576p" => 10,
        "480p" => 5,
        _ => 0,
    };
    let mut score = res + log2_score(10.0, r.seeders);
    if rel.proper || rel.repack {
        score += 5;
    }
    if r.freeleech {
        score += 2;
    }
    score
}

pub(crate) fn log2_points(factor: f64, n: i64) -> i64 {
    log2_score(factor, n)
}

/// A quality at or above the profile cutoff.
pub fn meets_cutoff(p: &Profile, res: &str) -> bool {
    match (rank_of(&p.resolutions, res), rank_of(&p.resolutions, &p.cutoff)) {
        (Some(r), Some(c)) => r <= c,
        _ => false,
    }
}

/// `cand` is worth grabbing over `have`: `have` is below the cutoff
/// (unknown counts as worst) and `cand` is allowed and strictly better.
pub fn is_upgrade(p: &Profile, have: &str, cand: &str) -> bool {
    if meets_cutoff(p, have) {
        return false;
    }
    let Some(c) = rank_of(&p.resolutions, cand) else { return false };
    rank_of(&p.resolutions, have).is_none_or(|h| c < h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CODE_INVALID;

    fn cand(title: &str, seeders: i64, size: i64) -> SearchResult {
        SearchResult { title: title.into(), seeders, size, protocol: PROTOCOL_TORRENT.into(), ..Default::default() }
    }

    fn parse(s: &str, kind: &str) -> Release {
        lain_identify::tokenize(s, kind)
    }

    #[test]
    fn profile_validate() {
        validate_profile(default_profile()).unwrap();
        let bads: Vec<fn(&mut Profile)> = vec![
            |p| p.name = String::new(),
            |p| p.resolutions = vec!["1080p".into(), "potato".into()],
            |p| p.resolutions = vec!["1080p".into(), "1080p".into()],
            |p| {
                p.cutoff = "2160p".into();
                p.resolutions = vec!["1080p".into(), "720p".into()];
            },
            |p| p.sources = vec!["vhs".into()],
            |p| {
                p.min_size_mb = 500;
                p.max_size_mb = 100;
            },
            |p| p.min_seeders = -1,
            |p| p.subtitle_languages = vec!["klingon".into()],
            |p| p.subtitle_hi = "maybe".into(),
        ];
        for bad in bads {
            let mut p = default_profile();
            bad(&mut p);
            assert_eq!(validate_profile(p.clone()).unwrap_err().code, CODE_INVALID, "accepted {p:?}");
        }
        let mut p = default_profile();
        p.subtitle_languages = vec!["en".into(), "eng".into(), "pt-BR".into()];
        let p = validate_profile(p).unwrap();
        assert_eq!(p.subtitle_languages, vec!["eng", "por"]);
        assert_eq!(p.subtitle_hi, HI_INCLUDE);
    }

    #[test]
    fn evaluate_accepts_and_ranks() {
        let mut p = default_profile();
        p.preferred_groups = vec!["Fansub-A".into()];
        let ev = |t: &str, s, z| evaluate(&p, &cand(t, s, z), &parse(t, "anime"), 1);
        let a = ev("[Fansub-A] Show - 05 [1080p]", 10, 700 << 20);
        let b = ev("[Fansub-B] Show - 05 [1080p]", 10, 700 << 20);
        let c = ev("[Fansub-B] Show - 05 [720p]", 500, 300 << 20);
        for d in [&a, &b, &c] {
            assert!(d.accepted, "rejected: {d:?}");
        }
        assert!(a.score > b.score && b.score > c.score, "{} {} {}", a.score, b.score, c.score);
        assert_eq!((a.quality.as_str(), c.quality.as_str()), ("1080p", "720p"));
    }

    #[test]
    fn evaluate_rejections() {
        let mut p = default_profile();
        p.sources = vec!["web".into(), "bluray".into()];
        p.blocked_groups = vec!["Fansub-X".into()];
        p.blocked_words = vec!["cam".into()];
        p.min_seeders = 3;
        p.min_size_mb = 100;
        p.max_size_mb = 2000;
        let cases = [
            ("resolution not allowed", "Show.S01E01.480p.WEB.x264-GRP", 9, 500 << 20),
            ("unknown resolution", "Show.S01E01.WEB.x264-GRP", 9, 500 << 20),
            ("source not allowed", "Show.S01E01.1080p.HDTV.x264-GRP", 9, 500 << 20),
            ("blocked group", "Show.S01E01.1080p.WEB.x264-Fansub-X", 9, 500 << 20),
            ("blocked word", "Show.S01E01.CAM.1080p.WEB.x264-GRP", 9, 500 << 20),
            ("too few seeders", "Show.S01E01.1080p.WEB.x264-GRP", 1, 500 << 20),
            ("too small", "Show.S01E01.1080p.WEB.x264-GRP", 9, 10 << 20),
            ("too large", "Show.S01E01.1080p.WEB.x264-GRP", 9, 5000 << 20),
        ];
        for (name, title, seeders, size) in cases {
            let d = evaluate(&p, &cand(title, seeders, size), &parse(title, "series"), 1);
            assert!(!d.accepted, "{name}: accepted");
            let first = name.split(' ').next().unwrap();
            assert!(d.rejections.join("; ").contains(first), "{name}: {:?}", d.rejections);
        }
        let pack = "Show.S01.1080p.WEB.x264-GRP";
        let d = evaluate(&p, &cand(pack, 9, 6000 << 20), &parse(pack, "series"), 12);
        assert!(d.accepted, "{:?}", d.rejections);
        let mut u = cand("Show.S01E01.1080p.WEB.x264-GRP", 0, 500 << 20);
        u.protocol = PROTOCOL_USENET.into();
        assert!(!evaluate(&p, &u, &parse(&u.title, "series"), 1).accepted);
    }

    #[test]
    fn proper_preference() {
        let p = default_profile();
        let ev = |t: &str| evaluate(&p, &cand(t, 5, 500 << 20), &parse(t, "series"), 1);
        assert!(ev("Show.S01E01.PROPER.1080p.WEB.x264-GRP").score > ev("Show.S01E01.1080p.WEB.x264-GRP").score);
    }

    #[test]
    fn upgrade_rules() {
        let p = default_profile();
        for (have, cand, want) in [
            ("720p", "1080p", true),
            ("720p", "720p", false),
            ("1080p", "2160p", false),
            ("", "720p", true),
            ("720p", "480p", false),
        ] {
            assert_eq!(is_upgrade(&p, have, cand), want, "have {have:?} cand {cand:?}");
        }
        assert!(meets_cutoff(&p, "2160p") && !meets_cutoff(&p, "720p"));
    }

    #[test]
    fn whole_words() {
        assert!(contains_word("show cam 1080p", "cam"));
        assert!(!contains_word("show camera", "cam"));
        assert!(contains_word("x.cam", "cam"));
        assert!(!contains_word("ação", ""));
        assert!(contains_word("ação cam", "cam"));
    }
}
