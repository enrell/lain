//! The release-name tokenizer (`lain-release-tokenizer`): hand-written
//! rules, no model, no regex. The always-available provider of
//! `lain.release.parse@1` and the source of technical tags (source,
//! codec, proper) a model's records lack. A port of
//! `internal/plugins/release/tokenizer.go`.
//!
//! Grammar, in any separator style (spaces, dots, underscores):
//!
//! ```text
//! [Group] Title - 05 (1080p) [CRC32]        absolute anime episode
//! [Group] Title - 01-12 [Batch]             absolute range
//! Title.S01E02E03.1080p.WEB-DL.x264-GROUP   season/episodes, suffix group
//! Title S02 Complete 1080p BluRay           season pack
//! Title Season 2 Episode 5
//! Title (2019) 2160p UHD BluRay             movie with year
//! Title v01 c001 / Title Vol. 2 Ch. 15      volumes and chapters
//! ```

use lain_api::Release;

use crate::text::{atoi, base, ext, fields_join, is_number, lower, strip_controls, trim_set};

pub const TOKENIZER_ID: &str = "lain-release-tokenizer";

const MEDIA_EXTS: &[&str] = &["mkv", "mp4", "avi", "m4v", "webm", "mov", "ts", "cbz", "cbr", "cb7", "torrent", "nzb"];

/// The canonical resolution for a lower-case tag (`4k` → `2160p`,
/// `1080i` → `1080p`), or `None` when the tag is not one. The model
/// bridge (`lain-bridge`) maps the model's label through it, as Go's
/// `merge` does.
pub fn resolution(l: &str) -> Option<&'static str> {
    Some(match l {
        "2160p" | "4k" | "uhd" => "2160p",
        "1080p" | "1080i" => "1080p",
        "720p" => "720p",
        "576p" => "576p",
        "540p" => "540p",
        "480p" => "480p",
        _ => return None,
    })
}

fn source(l: &str) -> Option<&'static str> {
    Some(match l {
        "bluray" | "blu-ray" | "bdrip" | "brrip" | "bd" | "bdremux" | "remux" => "bluray",
        "web" | "webdl" | "web-dl" | "webrip" | "amzn" | "nf" => "web",
        "hdtv" => "hdtv",
        "dvd" | "dvdrip" | "dvd5" | "dvd9" => "dvd",
        _ => return None,
    })
}

fn codec(l: &str) -> Option<&'static str> {
    Some(match l {
        "x264" | "h264" | "h.264" | "avc" => "h264",
        "x265" | "h265" | "h.265" | "hevc" => "h265",
        "av1" => "av1",
        "xvid" => "xvid",
        _ => return None,
    })
}

const OTHER_TECH: &[&str] = &[
    "10bit",
    "8bit",
    "hi10p",
    "hdr",
    "hdr10",
    "dv",
    "dolby",
    "vision",
    "aac",
    "ac3",
    "eac3",
    "flac",
    "opus",
    "dts",
    "truehd",
    "atmos",
    "ddp5.1",
    "dual-audio",
    "dual",
    "audio",
    "multi",
    "subs",
    "multisub",
    "dubbed",
    "subbed",
    "proper",
    "repack",
    "extended",
    "uncut",
    "remastered",
    "dl",
    "internal",
];

/// Whether a word is a technical release tag.
pub fn is_tech_tag(l: &str) -> bool {
    let l = lower(l);
    resolution(&l).is_some()
        || source(&l).is_some()
        || codec(&l).is_some()
        || OTHER_TECH.contains(&l.as_str())
        || l.starts_with("ddp")
        || l.starts_with("aac2")
}

fn apply_tag(l: &str, r: &mut Release) {
    if let Some(v) = resolution(l)
        && r.resolution.is_empty()
    {
        r.resolution = v.into();
    }
    if let Some(v) = source(l)
        && r.source.is_empty()
    {
        r.source = v.into();
    }
    if let Some(v) = codec(l)
        && r.codec.is_empty()
    {
        r.codec = v.into();
    }
    match l {
        "proper" => r.proper = true,
        "repack" | "rerip" => r.repack = true,
        _ => {}
    }
}

fn strip_ext(name: &str) -> String {
    let unix = name.replace('\\', "/");
    let mut name = base(&unix).to_string();
    loop {
        let e = ext(&name);
        if e.is_empty() || !MEDIA_EXTS.contains(&lower(&e[1..]).as_str()) {
            return name;
        }
        name.truncate(name.len() - e.len());
    }
}

fn is_year_token(s: &str) -> bool {
    let s = trim_set(s, "()");
    s.len() == 4 && is_number(s) && (1900..=2100).contains(&atoi(s))
}

/// Removes `[..]`, `(..)` and `{..}` groups and returns their contents;
/// a parenthesized year stays in the text so it can mark a movie.
fn pull_brackets(s: &str) -> (String, Vec<String>) {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut tags = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let close = match c {
            b'[' => b']',
            b'(' => b')',
            b'{' => b'}',
            _ => {
                out.push(c);
                i += 1;
                continue;
            }
        };
        let Some(end) = b[i + 1..].iter().position(|&x| x == close) else {
            out.push(b' ');
            i += 1;
            continue;
        };
        let inner = &s[i + 1..i + 1 + end];
        if c == b'(' && is_year_token(inner) {
            out.push(b' ');
            out.extend_from_slice(inner.as_bytes());
            out.push(b' ');
        } else {
            tags.push(inner.to_string());
            out.push(b' ');
        }
        i += end + 2;
    }
    (String::from_utf8_lossy(&out).into_owned(), tags)
}

/// Scene separators to spaces, keeping decimals ("5.1") and codec dots
/// ("H.264").
fn dots_to_spaces(s: &str) -> String {
    let mut b = s.as_bytes().to_vec();
    let n = b.len();
    let d = |x: u8| x.is_ascii_digit();
    for i in 0..n {
        if b[i] != b'.' {
            continue;
        }
        let digits_around =
            i > 0 && i < n - 1 && d(b[i - 1]) && d(b[i + 1]) && (i < 2 || !d(b[i - 2])) && (i + 2 >= n || !d(b[i + 2]));
        let codec_dot = i > 0 && i < n - 1 && (b[i - 1] == b'H' || b[i - 1] == b'h') && d(b[i + 1]);
        if !digits_around && !codec_dot {
            b[i] = b' ';
        }
    }
    String::from_utf8_lossy(&b).into_owned()
}

/// `SxxEyy[Ezz|-Ezz|-zz]...`
fn parse_season_episodes(l: &str, r: &mut Release) -> bool {
    let b = l.as_bytes();
    if b.len() < 4 || b[0] != b's' {
        return false;
    }
    let mut i = 1;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == 1 || i >= b.len() || b[i] != b'e' || i > 4 {
        return false;
    }
    let season = atoi(&l[1..i]);
    let mut eps: Vec<i64> = Vec::new();
    let mut rest = &l[i..];
    while !rest.is_empty() {
        let first = rest.as_bytes()[0];
        let is_range = first == b'-';
        if first == b'e' || is_range {
            rest = rest.strip_prefix('-').unwrap_or(rest);
            rest = rest.strip_prefix('e').unwrap_or(rest);
        } else {
            return false;
        }
        let j = rest.bytes().take_while(u8::is_ascii_digit).count();
        if j == 0 || j > 4 {
            return false;
        }
        let n = atoi(&rest[..j]);
        match eps.last().copied() {
            Some(last) if is_range && n > last && n - last < 500 => eps.extend(last + 1..=n),
            _ => eps.push(n),
        }
        rest = &rest[j..];
    }
    r.season = season;
    r.episodes = eps;
    r.season_pack = false;
    true
}

/// `05`, `05v2`, `01-12`, `1100`.
fn parse_absolute(s: &str) -> Option<(Vec<i64>, i64)> {
    let mut s = s;
    let mut ver = 0;
    if let Some(i) = s.find('v')
        && i > 0
        && is_number(&s[i + 1..])
    {
        ver = atoi(&s[i + 1..]);
        s = &s[..i];
    }
    if let Some((a, b)) = s.split_once('-')
        && is_number(a)
        && is_number(b)
    {
        let (x, y) = (atoi(a), atoi(b));
        if y < x || y - x > 2000 {
            return None;
        }
        return Some(((x..=y).collect(), ver));
    }
    if is_number(s) && s.len() <= 4 && !is_year_token(s) {
        return Some((vec![atoi(s)], ver));
    }
    None
}

fn mark(at: usize, title_end: &mut usize) {
    if at < *title_end {
        *title_end = at;
    }
}

fn followed_by_tags(rest: &[&str]) -> bool {
    for t in rest {
        let l = lower(t);
        if is_tech_tag(&l) {
            return true;
        }
        if l != "-" {
            return false;
        }
    }
    false
}

fn trailing_number(rest: &[&str]) -> bool {
    rest.iter().all(|t| {
        let l = lower(t);
        is_year_token(&l) || is_tech_tag(&l) || l == "-"
    })
}

/// Collapses whitespace, drops control characters, caps at 200 bytes
/// (on a character boundary).
pub fn clean_title(s: &str) -> String {
    let mut s = fields_join(&strip_controls(s));
    if s.len() > 200 {
        let mut cut = 200;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
    }
    s
}

/// Parses a release name. `kind` (`anime`, `series`, `movie`, `manga`,
/// `comic`) picks the reading grammar for archives.
pub fn tokenize(name: &str, kind: &str) -> Release {
    let mut r = Release { parser: TOKENIZER_ID.into(), ..Release::default() };
    let mut name = strip_ext(name).trim().to_string();
    if name.is_empty() {
        return r;
    }

    // Leading [Group].
    if name.starts_with('[')
        && let Some(end) = name.find(']')
        && end > 1
    {
        let g = name[1..end].trim();
        if !g.is_empty() && !is_tech_tag(g) {
            r.group = g.to_string();
        }
        name = name[end + 1..].trim().to_string();
    }

    // Bracketed tags anywhere else are technical metadata, never title.
    let (pulled, tags) = pull_brackets(&name);
    let mut name = pulled.replace('_', " ");
    if !name.contains(' ') && name.matches('.').count() >= 2 {
        name = dots_to_spaces(&name);
    }
    // A trailing "-GROUP" after the last technical token is the group.
    if r.group.is_empty()
        && let Some(i) = name.rfind('-')
        && i > 0
        && i < name.len() - 1
    {
        let cand = &name[i + 1..];
        if !cand.contains(' ') && !is_number(cand) && cand.len() <= 32 {
            let before: Vec<&str> = name[..i].split_whitespace().collect();
            if let Some(last) = before.last()
                && is_tech_tag(&lower(last))
            {
                r.group = cand.to_string();
                name.truncate(i);
            }
        }
    }

    let toks: Vec<&str> = name.split_whitespace().collect();
    let mut title_end = toks.len();
    let reading = kind == "manga" || kind == "comic";
    let n = toks.len();

    let mut i = 0;
    while i < n {
        let raw = toks[i];
        let l = lower(trim_set(raw, ",;"));
        let lb = l.as_bytes();
        if parse_season_episodes(&l, &mut r) {
            mark(i, &mut title_end);
        } else if lb.len() >= 2 && lb[0] == b's' && is_number(&l[1..]) && l.len() <= 4 {
            r.season = atoi(&l[1..]);
            r.season_pack = true;
            mark(i, &mut title_end);
        } else if (l == "season" || l == "saison") && i + 1 < n && is_number(toks[i + 1]) {
            r.season = atoi(toks[i + 1]);
            r.season_pack = true;
            mark(i, &mut title_end);
            i += 1;
        } else if (l == "episode" || l == "ep") && i + 1 < n && is_number(toks[i + 1]) {
            r.episodes = vec![atoi(toks[i + 1])];
            r.season_pack = false;
            mark(i, &mut title_end);
            i += 1;
        } else if l == "-" && i + 1 < n && i > 0 {
            // "Title - 05", "Title - 01-12", "Title - 05v2".
            match parse_absolute(&lower(toks[i + 1])) {
                Some((eps, ver)) if !reading => {
                    r.episodes = eps;
                    r.absolute = r.season == 0;
                    if ver > 0 {
                        r.version = ver;
                    }
                    mark(i, &mut title_end);
                    i += 1;
                }
                // A dash ends the title: an episode title or tags follow.
                _ => mark(i, &mut title_end),
            }
        } else if reading
            && (l == "vol" || l == "vol." || l == "volume")
            && i + 1 < n
            && is_number(trim_set(toks[i + 1], "."))
        {
            r.volume = atoi(trim_set(toks[i + 1], "."));
            mark(i, &mut title_end);
            i += 1;
        } else if reading
            && (l == "ch" || l == "ch." || l == "chapter")
            && i + 1 < n
            && is_number(trim_set(toks[i + 1], "."))
        {
            r.chapter = atoi(trim_set(toks[i + 1], "."));
            mark(i, &mut title_end);
            i += 1;
        } else if reading && lb.len() >= 2 && (lb[0] == b'v' || lb[0] == b'c') && is_number(&l[1..]) && i > 0 {
            let v = atoi(&l[1..]);
            if lb[0] == b'v' {
                r.volume = v;
            } else {
                r.chapter = v;
            }
            mark(i, &mut title_end);
        } else if reading && lb.len() >= 2 && lb[0] == b'#' && is_number(&l[1..]) && i > 0 {
            r.chapter = atoi(&l[1..]);
            mark(i, &mut title_end);
        } else if is_year_token(&l) && i > 0 {
            // The last plausible year wins: "1917 2019" is a title then a year.
            r.year = atoi(trim_set(&l, "()"));
            mark(i, &mut title_end);
        } else if l == "complete" || l == "batch" {
            if r.episodes.is_empty() {
                r.season_pack = true;
            }
            mark(i, &mut title_end);
        } else if is_tech_tag(&l) {
            apply_tag(&l, &mut r);
            mark(i, &mut title_end);
        } else if reading
            && i > 0
            && is_number(&l)
            && l.len() <= 4
            && r.chapter == 0
            && !is_year_token(&l)
            && trailing_number(&toks[i + 1..])
        {
            // "Saga 054 (2018)": a trailing bare number is the issue.
            r.chapter = atoi(&l);
            mark(i, &mut title_end);
        } else if !reading
            && i > 0
            && is_number(&l)
            && l.len() <= 4
            && l != "0"
            && r.episodes.is_empty()
            && r.season == 0
            && !is_year_token(&l)
        {
            // "Title 05 [1080p]": a bare number is an absolute episode
            // only when tags follow it.
            if followed_by_tags(&toks[i + 1..]) || (i == n - 1 && !tags.is_empty()) {
                r.episodes = vec![atoi(&l)];
                r.absolute = true;
                mark(i, &mut title_end);
            }
        }
        // Words after the first marker can still carry tags.
        if i >= title_end {
            apply_tag(&l, &mut r);
        }
        i += 1;
    }
    for tg in &tags {
        let words = lower(&tg.replace(['-', ',', '_'], " "));
        for w in words.split_whitespace() {
            apply_tag(w, &mut r);
        }
        let l = lower(tg);
        if (l == "batch" || l == "complete") && r.episodes.is_empty() {
            r.season_pack = true;
        }
    }
    if reading && r.volume == 0 && r.chapter == 0 && r.season > 0 {
        r.volume = r.season;
    }

    let title = toks[..title_end].join(" ");
    let mut title = trim_set(&title, " -–:.").to_string();
    if r.year > 0 && title_end > 0 {
        title = title.trim().strip_suffix('(').unwrap_or(title.trim()).to_string();
    }
    r.title = clean_title(&title);
    if r.title.is_empty() && r.year > 0 {
        // "1917 (2019)": the "year" that ended the title was the title.
        r.title = r.year.to_string();
        r.year = 0;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(f: impl FnOnce(&mut Release)) -> Release {
        let mut r = Release { parser: TOKENIZER_ID.into(), ..Release::default() };
        f(&mut r);
        r
    }

    #[test]
    fn the_go_table() {
        let cases: Vec<(&str, &str, Release)> = vec![
            (
                "[Fansub-A] Sousou no Frieren - 05 (1080p) [ABCD1234].mkv",
                "anime",
                rel(|r| {
                    r.title = "Sousou no Frieren".into();
                    r.episodes = vec![5];
                    r.absolute = true;
                    r.group = "Fansub-A".into();
                    r.resolution = "1080p".into();
                }),
            ),
            (
                "[Fansub-A] Sousou no Frieren - 01-04 [1080p] (Batch)",
                "anime",
                rel(|r| {
                    r.title = "Sousou no Frieren".into();
                    r.episodes = vec![1, 2, 3, 4];
                    r.absolute = true;
                    r.group = "Fansub-A".into();
                    r.resolution = "1080p".into();
                }),
            ),
            (
                "Show.Name.S01E05E06.PROPER.720p.HDTV.x265-GRP",
                "series",
                rel(|r| {
                    r.title = "Show Name".into();
                    r.season = 1;
                    r.episodes = vec![5, 6];
                    r.proper = true;
                    r.group = "GRP".into();
                    r.resolution = "720p".into();
                    r.source = "hdtv".into();
                    r.codec = "h265".into();
                }),
            ),
            (
                "1917 (2019) 1080p",
                "movie",
                rel(|r| {
                    r.title = "1917".into();
                    r.year = 2019;
                    r.resolution = "1080p".into();
                }),
            ),
            (
                "Saga #054 (2018).cbr",
                "comic",
                rel(|r| {
                    r.title = "Saga".into();
                    r.chapter = 54;
                    r.year = 2018;
                }),
            ),
        ];
        for (name, kind, want) in cases {
            assert_eq!(tokenize(name, kind), want, "{name}");
        }
    }

    #[test]
    fn odd_input_never_panics() {
        for s in ["", " ", "-", "[", "[]", "((((", "S", "S01E", "s1e999999", "- - -", "x.y.z", "é.é.é", "[é]-é"] {
            let _ = tokenize(s, "anime");
            let _ = tokenize(s, "manga");
        }
        let _ = tokenize(&"[a]".repeat(300), "");
        let long = tokenize(&"é".repeat(300), "");
        assert!(long.title.len() <= 200);
    }
}
