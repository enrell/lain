//! The built-in identifiers of `lain.media.identify@1`, ports of
//! `internal/plugins/identify`: the anime release-name specialist (one
//! tokenizer pass, no regex), the comic/manga archive reader (D-085)
//! and the generic extension fallback that always accepts.

use serde::{Deserialize, Serialize};

use crate::text::{base, dir, ext, fields_join, lower, strip_controls, trim_set};

pub const ANIME_ID: &str = "lain-identify-anime";
pub const GENERIC_ID: &str = "lain-identify-generic";
pub const COMIC_ID: &str = "lain-identify-comic";

/// A discovered file handed to identifiers: no interpretation, only
/// where it is and what library produced it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub path: String,
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub mod_time: i64,
    #[serde(default)]
    pub library_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub library_type: String,
}

/// An identifier's hypothesis. The scan takes the first accepted one
/// (non-empty title, positive confidence) in binding order.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub season: i64,
    #[serde(default)]
    pub episode: i64,
    #[serde(default)]
    pub year: i64,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default, deserialize_with = "null_as_empty")]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub plugin_id: String,
}

fn null_as_empty<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    Ok(Option::<Vec<String>>::deserialize(d)?.unwrap_or_default())
}

impl Proposal {
    pub fn accepted(&self) -> bool {
        !self.title.is_empty() && self.confidence > 0.0
    }
}

const VIDEO_EXTS: &[&str] = &["mkv", "mp4", "avi", "mov", "m4v", "webm"];

const TECH: &[&str] = &[
    "2160p",
    "1080p",
    "720p",
    "480p",
    "1080i",
    "720i",
    "bluray",
    "webdl",
    "web",
    "dl",
    "webrip",
    "hdtv",
    "dvdrip",
    "dvd",
    "bdrip",
    "brrip",
    "x264",
    "x265",
    "h264",
    "h265",
    "hevc",
    "avc",
    "av1",
    "xvid",
    "divx",
    "flac",
    "aac",
    "ac3",
    "opus",
    "dts",
    "hd",
    "vorbis",
    "truehd",
    "atmos",
    "dual",
    "audio",
    "multi",
    "subs",
    "sub",
    "subtitle",
    "subtitles",
    "multiple",
    "dubbed",
    "subbed",
    "dublado",
    "hdr",
    "hdr10",
    "dolby",
    "vision",
    "10bit",
    "8bit",
    "remastered",
    "extended",
    "unrated",
    "repack",
    "proper",
    "uncut",
];
const RES: &[&str] = &["2160p", "1080p", "720p", "480p"];
const CODEC: &[&str] = &["x264", "x265", "h264", "h265", "hevc", "avc", "av1"];
const SRC: &[&str] = &["bluray", "webdl", "web", "webrip", "hdtv", "dvd", "dvdrip", "bdrip", "brrip"];

/// Release tags, numbered variants included ("multi4", "hdr10"); pure
/// numbers never qualify, so "24" is never erased as a tag.
fn is_tech(l: &str) -> bool {
    if TECH.contains(&l) {
        return true;
    }
    let digits = l.bytes().rev().take_while(u8::is_ascii_digit).count();
    let i = l.len() - digits;
    i > 0 && i < l.len() && TECH.contains(&&l[..i])
}

/// The name without its extension and any further video extensions
/// ("file.mkv.mp4").
fn stem_of(b: &str) -> String {
    let mut name = b[..b.len() - ext(b).len()].to_string();
    loop {
        let e = ext(&name);
        if e.is_empty() || !VIDEO_EXTS.contains(&lower(&e[1..]).as_str()) {
            return name;
        }
        name.truncate(name.len() - e.len());
    }
}

struct Token<'a> {
    text: &'a str,
    lower: String,
    bracketed: bool,
}

fn is_sep(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'.' | b'_' | b'-' | b'[' | b']' | b'(' | b')')
}

/// Separator-delimited words with their bracket context.
fn split_tokens(s: &str) -> Vec<Token<'_>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let (mut depth, mut i) = (0usize, 0usize);
    while i < b.len() {
        let c = b[i];
        if c == b'[' || c == b'(' {
            depth += 1;
            i += 1;
            continue;
        }
        if c == b']' || c == b')' {
            depth = depth.saturating_sub(1);
            i += 1;
            continue;
        }
        if is_sep(c) {
            i += 1;
            continue;
        }
        let start = i;
        while i < b.len() && !is_sep(b[i]) {
            i += 1;
        }
        let t = &s[start..i];
        out.push(Token { text: t, lower: lower(t), bracketed: depth > 0 });
    }
    out
}

/// A small ASCII digit run (at most four digits).
fn small(s: &str) -> Option<i64> {
    if s.is_empty() || s.len() > 4 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(s.bytes().fold(0, |n, b| n * 10 + (b - b'0') as i64))
}

/// `s01e05`, any padding.
fn parse_sxxexx(t: &str) -> Option<(i64, i64)> {
    let b = t.as_bytes();
    if b.len() < 4 || b[0] != b's' {
        return None;
    }
    let mut e = None;
    for (i, &c) in b.iter().enumerate().skip(1) {
        if c == b'e' {
            e = Some(i);
            break;
        }
        if !c.is_ascii_digit() {
            return None;
        }
    }
    let e = e?;
    if e + 1 >= b.len() || t[1..e].len() > 2 || t[e + 1..].len() > 3 {
        return None;
    }
    Some((small(&t[1..e])?, small(&t[e + 1..])?))
}

/// `2x13`.
fn parse_nxm(t: &str) -> Option<(i64, i64)> {
    let b = t.as_bytes();
    let mut x = None;
    for (i, &c) in b.iter().enumerate() {
        if c == b'x' {
            x = Some(i);
            break;
        }
        if !c.is_ascii_digit() {
            return None;
        }
    }
    let x = x?;
    if x == 0 || x + 1 >= b.len() || x > 2 || t[x + 1..].len() > 3 {
        return None;
    }
    Some((small(&t[..x])?, small(&t[x + 1..])?))
}

/// "24v2" -> "24".
fn strip_version_suffix(t: &str) -> &str {
    let b = t.as_bytes();
    for i in (0..b.len()).rev() {
        if b[i] == b'v' && i > 0 {
            if small(&t[..i]).is_some() && small(&t[i + 1..]).is_some() {
                return &t[..i];
            }
            return t;
        }
        if !b[i].is_ascii_digit() {
            return t;
        }
    }
    t
}

fn is_year(t: &str) -> bool {
    t.len() == 4 && small(t).is_some_and(|y| (1900..=2099).contains(&y))
}

/// "4th" -> 4.
fn parse_ordinal(t: &str) -> Option<i64> {
    ["st", "nd", "rd", "th"].iter().find_map(|suf| t.strip_suffix(suf)).and_then(small)
}

/// Bare numbers ("12", "24v2") and E-prefixed ones ("E05", "EP12").
fn weak_episode(l: &str) -> Option<i64> {
    if let Some(rest) = l.strip_prefix("ep") {
        return small(rest);
    }
    if l.len() > 1
        && let Some(n) = l.strip_prefix('e').and_then(small)
    {
        return Some(n);
    }
    small(strip_version_suffix(l))
}

fn next_is(toks: &[Token], i: usize, want: &str) -> bool {
    toks.get(i + 1).is_some_and(|t| !t.bracketed && t.lower == want)
}

/// The anime release-name parser. `None` declines (no release
/// evidence: a plain "movie.mp4"), so the next identifier runs.
pub fn identify_anime(c: &Candidate) -> Option<Proposal> {
    let stem = stem_of(base(&c.path));
    let mut evidence: Vec<String> = Vec::new();
    let add = |ev: &mut Vec<String>, e: &str| {
        if !ev.iter().any(|x| x == e) {
            ev.push(e.to_string());
        }
    };

    // A leading "[Group]" is the release group, not the title.
    let mut group = "";
    let mut rest = stem.trim_start_matches([' ', '\t']);
    if rest.starts_with('[')
        && let Some(end) = rest.find(']')
        && end > 1
    {
        group = &rest[1..end];
        add(&mut evidence, "release-group");
        rest = &rest[end + 1..];
    }

    let toks = split_tokens(rest);
    let (mut season, mut episode, mut year) = (0i64, 0i64, 0i64);
    let mut strong = false;
    let (mut has_tech, mut has_res) = (false, false);
    let mut ep_out: Option<usize> = None;
    let mut season_skip = vec![false; toks.len() + 1];
    let mut out_count = 0usize;

    for (i, tk) in toks.iter().enumerate() {
        let l = tk.lower.as_str();
        if RES.contains(&l) {
            has_res = true;
            has_tech = true;
            add(&mut evidence, "resolution");
        } else if CODEC.contains(&l) {
            has_tech = true;
            add(&mut evidence, "codec");
        } else if SRC.contains(&l) {
            has_tech = true;
            add(&mut evidence, "source");
        } else if is_tech(l) {
            has_tech = true;
        }
        if is_year(l) && year == 0 {
            year = small(l).unwrap_or(0);
            add(&mut evidence, "year");
        }
        if tk.bracketed {
            continue;
        }
        // Season words with one-token lookahead ("4th Season", "Season 2").
        if season == 0 {
            if let Some(n) = parse_ordinal(l).filter(|n| *n <= 30)
                && next_is(&toks, i, "season")
            {
                season = n;
                season_skip[i] = true;
                season_skip[i + 1] = true;
                add(&mut evidence, "season-word");
            } else if l == "season"
                && let Some(n) = toks.get(i + 1).and_then(|t| small(&t.lower)).filter(|n| *n <= 30)
            {
                season = n;
                season_skip[i] = true;
                season_skip[i + 1] = true;
                add(&mut evidence, "season-word");
            }
        }
        if ep_out.is_none() || !strong {
            if let Some((sn, en)) = parse_sxxexx(l) {
                (season, episode, strong, ep_out) = (sn, en, true, Some(out_count));
                add(&mut evidence, "sxxexx");
            } else if let Some((a, b)) = parse_nxm(l) {
                (season, episode, strong, ep_out) = (a, b, true, Some(out_count));
                add(&mut evidence, "nxm");
            } else if ep_out.is_none()
                && let Some(en) = weak_episode(l)
            {
                episode = en;
                ep_out = Some(out_count);
                add(&mut evidence, "episode-number");
            }
        }
        out_count += 1;
    }
    if ep_out.is_none() {
        // Unambiguous markers hidden inside brackets ("Show [S01E05]").
        for tk in toks.iter().filter(|t| t.bracketed) {
            if let Some((sn, en)) = parse_sxxexx(&tk.lower) {
                (season, episode, strong, ep_out) = (sn, en, true, Some(out_count));
                add(&mut evidence, "sxxexx");
                break;
            }
            if let Some((a, b)) = parse_nxm(&tk.lower) {
                (season, episode, strong, ep_out) = (a, b, true, Some(out_count));
                add(&mut evidence, "nxm");
                break;
            }
        }
    }
    if year > 0 && episode == year && !strong {
        // A lone 4-digit number is a year, not episode 1995.
        episode = 0;
        ep_out = None;
        evidence.retain(|e| e != "episode-number");
    }
    if group.is_empty() && !strong && !(year > 0 && has_tech) {
        return None;
    }

    // Title: unbracketed words before the episode marker, minus tags.
    let mut words: Vec<String> = Vec::new();
    let mut n = 0usize;
    for (i, tk) in toks.iter().enumerate() {
        if tk.bracketed {
            continue;
        }
        if season_skip[i] {
            n += 1;
            continue;
        }
        if ep_out.is_some_and(|e| n >= e) {
            break;
        }
        let l = tk.lower.as_str();
        let skip = is_tech(l)
            || RES.contains(&l)
            || CODEC.contains(&l)
            || SRC.contains(&l)
            || is_year(l)
            || parse_ordinal(l).is_some();
        n += 1;
        if skip {
            continue;
        }
        let w = strip_controls(tk.text);
        if !w.is_empty() {
            words.push(w);
        }
    }
    if words.is_empty() {
        return None;
    }
    let mut conf = 0.7;
    if !group.is_empty() {
        conf += 0.1;
    }
    if has_res {
        conf += 0.08;
    }
    if conf > 0.96 {
        conf = 0.96;
    }
    let kind = if season == 0 && episode == 0 { "video" } else { "episode" };
    Some(Proposal {
        kind: kind.into(),
        title: words.join(" "),
        season,
        episode,
        year,
        confidence: conf,
        evidence,
        plugin_id: ANIME_ID.into(),
    })
}

/// Removes `[...]`/`(...)` groups (an opener, anything but a closer,
/// a closer: the Go `[\[\(][^\]\)]*[\]\)]`), each replaced by `with`.
fn replace_groups<'a>(s: &'a str, with: &str) -> (String, Vec<&'a str>) {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut groups = Vec::new();
    let mut i = 0;
    let mut last = 0;
    while i < b.len() {
        if b[i] == b'[' || b[i] == b'(' {
            if let Some(off) = b[i + 1..].iter().position(|&c| c == b']' || c == b')') {
                let end = i + 1 + off;
                out.push_str(&s[last..i]);
                out.push_str(with);
                groups.push(&s[i..=end]);
                i = end + 1;
                last = i;
                continue;
            }
            // No closer anywhere after: no further match is possible.
            break;
        }
        i += 1;
    }
    out.push_str(&s[last..]);
    (out, groups)
}

/// Classifies by extension and never declines.
pub fn identify_generic(c: &Candidate) -> Proposal {
    let b = base(&c.path);
    let e = ext(b);
    let lext = lower(e.strip_prefix('.').unwrap_or(e));
    let name = &b[..b.len() - e.len()];
    let (clean, _) = replace_groups(name, " ");
    let clean = clean.replace('-', " ").replace(['.', '_'], " ");
    let mut title = fields_join(&clean);
    if title.is_empty() {
        title = b.to_string();
    }
    let kind = match lext.as_str() {
        "mp3" | "flac" | "m4a" | "aac" | "ogg" | "opus" | "wav" => "audio",
        "pdf" | "epub" | "mobi" | "azw3" => "book",
        "cbz" | "cbr" | "cb7" => "comic",
        "jpg" | "jpeg" | "png" | "webp" | "gif" | "heic" | "avif" => "photo",
        _ => "video",
    };
    Proposal {
        kind: kind.into(),
        title: strip_controls(&title),
        confidence: 0.4,
        evidence: vec!["extension".into()],
        plugin_id: GENERIC_ID.into(),
        ..Proposal::default()
    }
}

// ---- comics and manga (D-085) ----------------------------------------------

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0c)
}

/// The tail `[\s._]*0*(\d{1,4})\b` from `p`: the matched digit range.
/// With RE2's leftmost-first semantics the separators and the digit run
/// are maximal, and the digits are the run without its leading zeros
/// (one zero when it is all zeros), at most four of them.
fn number_tail(b: &[u8], mut p: usize, seps: bool) -> Option<(usize, usize)> {
    if seps {
        while p < b.len() && (is_space(b[p]) || b[p] == b'.' || b[p] == b'_') {
            p += 1;
        }
    }
    let run = b[p..].iter().take_while(|c| c.is_ascii_digit()).count();
    if run == 0 {
        return None;
    }
    let end = p + run;
    if end < b.len() && crate::text::is_word(b[end]) {
        return None;
    }
    let zeros = b[p..end].iter().take_while(|&&c| c == b'0').count();
    let k0 = if zeros == run { zeros - 1 } else { zeros };
    if run - k0 > 4 {
        return None;
    }
    Some((p + k0, end))
}

/// Case-insensitive keyword at `p`.
fn kw(b: &[u8], p: usize, word: &str) -> Option<usize> {
    let w = word.as_bytes();
    (b.len() >= p + w.len() && b[p..p + w.len()].eq_ignore_ascii_case(w)).then_some(p + w.len())
}

/// Leftmost match of `(?:^|[\s._-])(?:KEYWORDS)TAIL`: the match start
/// and the digits. `alts` lists the keyword alternatives in priority
/// order.
fn find_marker(s: &str, alts: &[&str]) -> Option<(usize, i64)> {
    let b = s.as_bytes();
    for start in 0..=b.len() {
        let mut heads = Vec::with_capacity(2);
        if start == 0 {
            heads.push(0);
        }
        if start < b.len() && (is_space(b[start]) || matches!(b[start], b'.' | b'_' | b'-')) {
            heads.push(start + 1);
        }
        for h in heads {
            for alt in alts {
                if let Some(p) = kw(b, h, alt)
                    && let Some((d0, d1)) = number_tail(b, p, true)
                {
                    return Some((start, crate::text::atoi(&s[d0..d1])));
                }
            }
        }
    }
    None
}

const VOLUME_ALTS: &[&str] = &["volume.", "volume", "vol.", "vol", "v"];
const CHAPTER_ALTS: &[&str] =
    &["chapter.", "chapter", "ch.", "ch", "c", "episode.", "episode", "ep.", "ep", "issue", "no.", "no", "#"];

/// `#[\s._]*0*(\d{1,4})\b`.
fn find_hash(s: &str) -> Option<(usize, i64)> {
    let b = s.as_bytes();
    (0..b.len())
        .filter(|&i| b[i] == b'#')
        .find_map(|i| number_tail(b, i + 1, true).map(|(d0, d1)| (i, crate::text::atoi(&s[d0..d1]))))
}

/// `[\s._-]0*(\d{1,4})$`.
fn find_trailing(s: &str) -> Option<(usize, i64)> {
    let b = s.as_bytes();
    let run = b.iter().rev().take_while(|c| c.is_ascii_digit()).count();
    if run == 0 || run == b.len() {
        return None;
    }
    let i = b.len() - run - 1;
    if !(is_space(b[i]) || matches!(b[i], b'.' | b'_' | b'-')) {
        return None;
    }
    let (d0, d1) = number_tail(b, i + 1, false)?;
    Some((i, crate::text::atoi(&s[d0..d1])))
}

/// `^0*(\d{1,4})$`.
fn bare_number(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && number_tail(s.as_bytes(), 0, false).is_some()
}

/// `\b(19|20)\d{2}\b`, the first one.
fn find_year(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    (0..b.len().saturating_sub(3)).find_map(|i| {
        let w = &b[i..i + 4];
        let ok = (w.starts_with(b"19") || w.starts_with(b"20"))
            && w.iter().all(u8::is_ascii_digit)
            && (i == 0 || !crate::text::is_word(b[i - 1]))
            && (i + 4 == b.len() || !crate::text::is_word(b[i + 4]));
        ok.then(|| crate::text::atoi(&s[i..i + 4]))
    })
}

fn clean_comic_title(s: &str) -> String {
    strip_controls(trim_set(&fields_join(s), " -–—_.:,"))
}

/// Reads cbz/cbr/cb7 archives; `None` declines every other extension
/// so the anime parser never sees an archive. The series is the title,
/// a volume the season and a chapter or issue the episode, each 0 when
/// absent; the kind follows the library type.
pub fn identify_comic(c: &Candidate) -> Option<Proposal> {
    let b = base(&c.path);
    let e = ext(b);
    let lext = lower(e.strip_prefix('.').unwrap_or(e));
    if !matches!(lext.as_str(), "cbz" | "cbr" | "cb7") {
        return None;
    }
    let kind = if c.library_type.eq_ignore_ascii_case("manga") { "manga" } else { "comic" };
    let stem = &b[..b.len() - e.len()];

    let (without, groups) = replace_groups(stem, " ");
    let year = groups.iter().find_map(|g| find_year(g)).unwrap_or(0);
    let mut name = fields_join(&without).replace('_', " ");
    if !name.contains(' ') {
        name = name.replace('.', " ");
    }

    let (mut vol, mut ch) = (0i64, 0i64);
    let mut evidence = vec!["extension".to_string()];
    let mut cut = name.len();
    if let Some((at, n)) = find_marker(&name, VOLUME_ALTS) {
        vol = n;
        cut = cut.min(at);
        evidence.push("volume".into());
    }
    if let Some((at, n)) = find_marker(&name, CHAPTER_ALTS) {
        ch = n;
        cut = cut.min(at);
        evidence.push("chapter".into());
    } else if let Some((at, n)) = find_hash(&name) {
        ch = n;
        cut = cut.min(at);
        evidence.push("issue".into());
    }
    if vol == 0
        && ch == 0
        && let Some((at, n)) = find_trailing(&name)
    {
        ch = n;
        cut = at;
        evidence.push("trailing-number".into());
    }
    let mut title = clean_comic_title(&name[..cut]);
    if title.is_empty() || bare_number(&title) {
        // "Series/Vol 03.cbz": the folder names the series.
        let d = base(dir(&c.path));
        if d != "." && d != "/" && d != "\\" {
            title = clean_comic_title(d);
            evidence.push("folder".into());
        }
    }
    if title.is_empty() {
        title = strip_controls(stem);
    }
    Some(Proposal {
        kind: kind.into(),
        title,
        season: vol,
        episode: ch,
        year,
        confidence: if vol > 0 || ch > 0 { 0.85 } else { 0.5 },
        evidence,
        plugin_id: COMIC_ID.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(p: &str) -> Candidate {
        Candidate { path: format!("/lib/{p}"), library_id: "lib-1".into(), ..Candidate::default() }
    }

    #[test]
    fn anime_release_names() {
        for (file, title, s, e) in [
            ("[Fansub-A] Frieren - 12 [1080p][HEVC x265 10bit][Multi-Subs].mkv", "Frieren", 0, 12),
            ("[Fansub-B] Solo Leveling S02E05 (1080p) [7A3B2C1D].mkv", "Solo Leveling", 2, 5),
            (
                "[Fansub-A] Tensei Shitara Slime Datta Ken 4th Season - 20 [1080p WEBRip HEVC AAC][MultiSub][B19A6FE6].mkv",
                "Tensei Shitara Slime Datta Ken",
                4,
                20,
            ),
            ("Silo.S01E01.1080p.WEB-DL.mkv.mp4", "Silo", 1, 1),
        ] {
            let p = identify_anime(&cand(file)).unwrap_or_else(|| panic!("{file} declined"));
            assert_eq!((p.title.as_str(), p.season, p.episode), (title, s, e), "{file}");
            assert_eq!(p.plugin_id, ANIME_ID);
        }
        let p = identify_anime(&cand("1a. Ghost in the Shell - The Movie (1995 - 1080p DUAL Audio).mkv")).unwrap();
        assert_eq!((p.title.as_str(), p.year, p.episode), ("1a Ghost in the Shell The Movie", 1995, 0));
        assert!(identify_anime(&cand("movie.mp4")).is_none());
        let p = identify_anime(&cand("[Fansub-A] Frieren - 01 [1080p].mkv")).unwrap();
        assert_eq!(p.confidence, 0.7 + 0.1 + 0.08);
    }

    #[test]
    fn generic_never_declines() {
        let p = identify_generic(&cand("Some Movie (2020) [1080p].mkv"));
        assert_eq!((p.kind.as_str(), p.title.as_str(), p.confidence), ("video", "Some Movie", 0.4));
        assert_eq!(identify_generic(&cand("song.FLAC")).kind, "audio");
        assert_eq!(identify_generic(&cand("[x].mkv")).title, "[x].mkv");
    }

    #[test]
    fn comics_and_manga() {
        let mut c = cand("Frieren Vol. 02 Ch. 015.cbz");
        c.library_type = "manga".into();
        let p = identify_comic(&c).unwrap();
        assert_eq!((p.kind.as_str(), p.title.as_str(), p.season, p.episode), ("manga", "Frieren", 2, 15));
        let p = identify_comic(&cand("Saga #054 (2018).cbr")).unwrap();
        assert_eq!((p.title.as_str(), p.episode, p.year), ("Saga", 54, 2018));
        let p = identify_comic(&cand("Saga/Vol 03.cbz")).unwrap();
        assert_eq!((p.title.as_str(), p.season), ("Saga", 3));
        let p = identify_comic(&cand("Akira_045.cbz")).unwrap();
        assert_eq!((p.title.as_str(), p.episode), ("Akira", 45));
        assert!(identify_comic(&cand("a.mkv")).is_none());
        assert_eq!(identify_comic(&cand("v12345.cbz")).unwrap().season, 0);
    }
}
