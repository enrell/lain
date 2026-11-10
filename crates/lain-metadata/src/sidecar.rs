//! `lain-metadata-sidecar`: the download sidecar. A downloader that
//! files each item in its own folder (the Anything bridge's does, D-230)
//! leaves what it knew about the item next to it:
//!
//! ```text
//! <item>/<item>.mp4
//! <item>/.metadata/meta.toml
//! <item>/.metadata/thumbs/thumbnail.jpg
//! ```
//!
//! Like an `.nfo`, this is offline and already about this exact file, so
//! it is local, curated metadata: the title, the year of the upload
//! date, the tags as genres and the description. The remote id is the
//! sidecar's path. The still next to it is the item's artwork
//! ([`artwork`]): the gateway serves it where it would otherwise cut a
//! frame out of the video.
//!
//! The file is TOML, but only the flat part is read: top-level strings,
//! numbers and string arrays. Tables (`[remote]`, `[images]`, `[local]`)
//! are skipped; they hold URLs and absolute paths of the machine that
//! downloaded the item.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use lain_core::{Error, Provider};
use lain_identify::{CAP_MEDIA_IDENTIFY, Candidate, Proposal};

use crate::nfo::{CAP_METADATA_RESOLVE, CAP_METADATA_SEARCH};
use crate::types::{MetadataCandidate, MetadataRecord, MetadataResolveInput, MetadataSearchInput};

pub const SIDECAR_ID: &str = "lain-metadata-sidecar";
/// The identifier that reads the same file (D-270).
pub const SIDECAR_IDENTIFY_ID: &str = "lain-identify-sidecar";

const DIR: &str = ".metadata";
const FILE: &str = "meta.toml";
/// The largest sidecar read.
const MAX_SIDECAR: u64 = 1 << 20;
/// Tags kept as genres: a scraped page can carry hundreds.
const MAX_GENRES: usize = 48;
const MAX_SYNOPSIS: usize = 8000;

/// The sidecar of the item whose media lives in `dir`.
pub fn sidecar_path(dir: &Path) -> PathBuf {
    dir.join(DIR).join(FILE)
}

/// The item's own still, when its folder has one: a non-empty regular
/// file, never a link out of the folder.
pub fn artwork(media_file: &Path) -> Option<PathBuf> {
    let thumbs = media_file.parent()?.join(DIR).join("thumbs");
    ["thumbnail.jpg", "banner.jpg"]
        .iter()
        .map(|n| thumbs.join(n))
        .find(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_file() && m.len() > 0))
}

/// The content type of a still by its first bytes: downloaders keep the
/// `.jpg` name whatever the site served.
pub fn image_type(path: &Path) -> &'static str {
    use std::io::Read;
    let mut head = [0u8; 12];
    let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    match &head[..n] {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [b'G', b'I', b'F', b'8', ..] => "image/gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P'] => "image/webp",
        _ => "image/jpeg",
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Val {
    Str(String),
    Int(i64),
    List(Vec<String>),
}

/// One basic (`"…"`) or literal (`'…'`) string at the start of `s`; the
/// rest of the input follows it.
fn string(s: &str) -> Option<(String, &str)> {
    let mut chars = s.char_indices();
    match chars.next()?.1 {
        '\'' => {
            let end = s[1..].find(['\'', '\n']).filter(|i| s[1 + i..].starts_with('\''))?;
            Some((s[1..1 + end].to_string(), &s[end + 2..]))
        }
        '"' => {
            let mut out = String::new();
            while let Some((i, c)) = chars.next() {
                match c {
                    '"' => return Some((out, &s[i + 1..])),
                    // A basic string ends on its line.
                    '\n' => return None,
                    '\\' => match chars.next()?.1 {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'r' => out.push('\r'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        e @ ('u' | 'U') => {
                            let n = if e == 'u' { 4 } else { 8 };
                            let hex: String = chars.by_ref().take(n).map(|(_, h)| h).collect();
                            out.push(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32)?);
                        }
                        other => out.push(other),
                    },
                    c => out.push(c),
                }
            }
            None
        }
        _ => None,
    }
}

/// The strings of an array body (after `[`), across lines, and what
/// follows the closing bracket; `None` when the array never closes.
fn list<'a>(mut s: &'a str, into: &mut Vec<String>) -> Option<&'a str> {
    loop {
        s = s.trim_start_matches(|c: char| c.is_whitespace() || c == ',');
        if let Some(rest) = s.strip_prefix('#') {
            s = rest.find('\n').map(|i| &rest[i..]).unwrap_or("");
            continue;
        }
        if let Some(rest) = s.strip_prefix(']') {
            return Some(rest);
        }
        let (item, rest) = string(s)?;
        into.push(item);
        s = rest;
    }
}

fn skip_line(s: &str) -> &str {
    s.find('\n').map(|i| &s[i + 1..]).unwrap_or("")
}

/// The top-level keys of a sidecar. Anything this reader does not
/// understand is skipped, never an error: a sidecar from a newer
/// downloader still gives what it shares with this one.
fn parse(src: &str) -> Vec<(String, Val)> {
    let mut out = Vec::new();
    let mut s = src;
    loop {
        s = s.trim_start();
        // The first table ends the part this reader keeps.
        if s.is_empty() || s.starts_with('[') {
            break;
        }
        let line = &s[..s.find('\n').unwrap_or(s.len())];
        let Some(eq) = line.find('=').filter(|_| !line.starts_with('#')) else {
            s = skip_line(s);
            continue;
        };
        let key = line[..eq].trim().to_string();
        let value = s[eq + 1..].trim_start_matches([' ', '\t']);
        if let Some(body) = value.strip_prefix('[') {
            let mut items = Vec::new();
            match list(body, &mut items) {
                Some(rest) => {
                    out.push((key, Val::List(items)));
                    s = skip_line(rest);
                }
                None => break,
            }
        } else if let Some((text, rest)) = string(value) {
            out.push((key, Val::Str(text)));
            s = skip_line(rest);
        } else {
            let number = value[..value.find(['\n', '#']).unwrap_or(value.len())].trim().replace('_', "");
            if let Ok(n) = number.parse::<i64>() {
                out.push((key, Val::Int(n)));
            }
            s = skip_line(s);
        }
    }
    out
}

fn text<'a>(doc: &'a [(String, Val)], key: &str) -> &'a str {
    doc.iter()
        .find_map(|(k, v)| match v {
            Val::Str(s) if k == key => Some(s.trim()),
            _ => None,
        })
        .unwrap_or("")
}

fn strings<'a>(doc: &'a [(String, Val)], key: &str) -> &'a [String] {
    doc.iter()
        .find_map(|(k, v)| match v {
            Val::List(l) if k == key => Some(l.as_slice()),
            _ => None,
        })
        .unwrap_or(&[])
}

/// A plausible year at the start of a date (`2024-02-21…`, `20240221`).
fn year_of(date: &str) -> i64 {
    date.get(..4).and_then(|y| y.parse::<i64>().ok()).filter(|y| (1900..=2099).contains(y)).unwrap_or(0)
}

/// Reads one sidecar into a record.
pub fn resolve_sidecar(path: &Path) -> Result<MetadataRecord, Error> {
    // The remote id comes back from a client: it must name a sidecar.
    if path.file_name().and_then(|n| n.to_str()) != Some(FILE)
        || path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()) != Some(DIR)
    {
        return Err(Error::invalid("not a download sidecar"));
    }
    let meta = std::fs::metadata(path).map_err(|_| Error::not_found(format!("no sidecar at {}", path.display())))?;
    if !meta.is_file() || meta.len() > MAX_SIDECAR {
        return Err(Error::invalid(format!("{}: not a sidecar this reader takes", path.display())));
    }
    let raw = std::fs::read(path).map_err(|e| Error::unavailable(format!("{}: {e}", path.display())))?;
    let doc = parse(&String::from_utf8_lossy(&raw));
    let mut title = text(&doc, "title").to_string();
    if title.is_empty() {
        title = text(&doc, "remote_title").to_string();
    }
    if title.is_empty() {
        return Err(Error::invalid(format!("{}: no title", path.display())));
    }
    let mut genres: Vec<String> = Vec::new();
    for tag in strings(&doc, "tags").iter().chain(strings(&doc, "categories")) {
        let tag = tag.trim();
        if !tag.is_empty() && tag.len() <= 64 && !genres.iter().any(|g| g.eq_ignore_ascii_case(tag)) {
            genres.push(tag.to_string());
        }
        if genres.len() == MAX_GENRES {
            break;
        }
    }
    let mut synopsis = text(&doc, "description").to_string();
    if synopsis.len() > MAX_SYNOPSIS {
        let cut = (0..=MAX_SYNOPSIS).rev().find(|i| synopsis.is_char_boundary(*i)).unwrap_or(0);
        synopsis.truncate(cut);
    }
    Ok(MetadataRecord {
        provider: SIDECAR_ID.into(),
        remote_id: path.to_string_lossy().into_owned(),
        title,
        year: year_of(text(&doc, "upload_date")),
        genres,
        synopsis,
        ..MetadataRecord::default()
    })
}

/// The sidecar of `input.dir`, if it has one: it is about the file
/// being enriched, so the query does not have to match its title.
pub fn search_sidecar(input: &MetadataSearchInput) -> Vec<MetadataCandidate> {
    if input.dir.is_empty() {
        return Vec::new();
    }
    let path = sidecar_path(Path::new(&input.dir));
    match resolve_sidecar(&path) {
        Ok(rec) => vec![MetadataCandidate {
            provider: SIDECAR_ID.into(),
            remote_id: rec.remote_id,
            title: rec.title,
            year: rec.year,
            kind: input.kind.clone(),
            ..MetadataCandidate::default()
        }],
        Err(_) => Vec::new(),
    }
}

/// What the downloader said the file is: the series and the episode
/// number when it knew both (an episode of that series), else the
/// item's title (a standalone video). The default proposal, which the
/// scan does not accept, when the folder has no sidecar or the file is
/// not a video: the name-based identifiers then have their turn.
pub fn identify_sidecar(c: &Candidate) -> Proposal {
    let path = Path::new(&c.path);
    let video = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["mkv", "mp4", "avi", "mov", "m4v", "webm"].contains(&e.to_ascii_lowercase().as_str()));
    let Some(dir) = path.parent().filter(|_| video) else { return Proposal::default() };
    let file = sidecar_path(dir);
    let Ok(meta) = std::fs::metadata(&file) else { return Proposal::default() };
    if !meta.is_file() || meta.len() > MAX_SIDECAR {
        return Proposal::default();
    }
    let Ok(raw) = std::fs::read(&file) else { return Proposal::default() };
    let doc = parse(&String::from_utf8_lossy(&raw));
    let series = text(&doc, "series");
    // "3", "03" or the number itself; anything else is not an episode.
    let episode = doc
        .iter()
        .find_map(|(k, v)| match v {
            Val::Str(s) if k == "episode" => s.trim().parse::<i64>().ok(),
            Val::Int(n) if k == "episode" => Some(*n),
            _ => None,
        })
        .filter(|n| (1..=9999).contains(n))
        .unwrap_or(0);
    let (kind, title) = if !series.is_empty() && episode > 0 {
        ("episode", series)
    } else {
        ("video", if text(&doc, "title").is_empty() { text(&doc, "remote_title") } else { text(&doc, "title") })
    };
    if title.is_empty() {
        return Proposal::default();
    }
    Proposal {
        kind: kind.into(),
        title: title.to_string(),
        episode: if kind == "episode" { episode } else { 0 },
        year: year_of(text(&doc, "upload_date")),
        confidence: 0.95,
        evidence: vec!["sidecar".into()],
        plugin_id: SIDECAR_IDENTIFY_ID.into(),
        ..Proposal::default()
    }
}

/// `lain-identify-sidecar` (`lain.media.identify@1`), first in the
/// binding order: what was written down beats what a name suggests.
pub struct SidecarIdentifier;

impl Provider for SidecarIdentifier {
    fn id(&self) -> &str {
        SIDECAR_IDENTIFY_ID
    }

    fn capabilities(&self) -> Vec<String> {
        vec![CAP_MEDIA_IDENTIFY.into()]
    }

    fn health(&self) -> Result<(), Error> {
        Ok(())
    }

    fn invoke(&self, cap: &str, op: &str, input: Value) -> Result<Value, Error> {
        if cap != CAP_MEDIA_IDENTIFY {
            return Err(Error::invalid(format!("unsupported cap {cap}")));
        }
        if !op.is_empty() && op != "identify" {
            return Err(Error::invalid(format!("capability {cap} has no op {op}")));
        }
        let c: Candidate =
            serde_json::from_value(input).map_err(|e| Error::invalid(format!("Candidate required: {e}")))?;
        Ok(json!(identify_sidecar(&c)))
    }
}

pub struct Sidecar;

impl Provider for Sidecar {
    fn id(&self) -> &str {
        SIDECAR_ID
    }

    fn capabilities(&self) -> Vec<String> {
        vec![CAP_METADATA_SEARCH.into(), CAP_METADATA_RESOLVE.into()]
    }

    fn health(&self) -> Result<(), Error> {
        Ok(())
    }

    fn invoke(&self, cap: &str, op: &str, input: Value) -> Result<Value, Error> {
        match cap {
            CAP_METADATA_SEARCH if op.is_empty() || op == "search" => {
                let i: MetadataSearchInput = serde_json::from_value(input)
                    .map_err(|e| Error::invalid(format!("MetadataSearchInput required: {e}")))?;
                Ok(json!(search_sidecar(&i)))
            }
            CAP_METADATA_RESOLVE if op.is_empty() || op == "resolve" => {
                let i: MetadataResolveInput = serde_json::from_value(input)
                    .map_err(|e| Error::invalid(format!("MetadataResolveInput required: {e}")))?;
                Ok(json!(resolve_sidecar(Path::new(&i.remote_id))?))
            }
            CAP_METADATA_SEARCH | CAP_METADATA_RESOLVE => {
                Err(Error::invalid(format!("capability {cap} has no op {op}")))
            }
            _ => Err(Error::invalid(format!("unsupported cap {cap}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"schema_version = 2
media_type = "video"
title = "Lighthouse at Dusk - Studio"
remote_title = "Lighthouse at Dusk"
tags = ["Short Film", "4K", "short film", ""]
categories = [
    "Landscape",
    "Sea \"Storm\"",  # a comment
]
extracted_at_ms = 1_700_000_000_000
description = "Line one.\nLine é."
upload_date = "2024-02-21T03:33:41.000Z"

[remote]
scrape_url = "https://example.test/watch/1"
title = "not this one"

[local]
video_path = "/somewhere/else.mp4"
"#;

    fn item_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lain-sidecar-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(DIR).join("thumbs")).unwrap();
        dir
    }

    #[test]
    fn reads_the_flat_part_and_skips_tables() {
        let doc = parse(SAMPLE);
        assert_eq!(text(&doc, "title"), "Lighthouse at Dusk - Studio");
        assert_eq!(strings(&doc, "tags"), ["Short Film", "4K", "short film", ""]);
        assert_eq!(strings(&doc, "categories"), ["Landscape", "Sea \"Storm\""]);
        assert_eq!(text(&doc, "description"), "Line one.\nLine é.");
        assert!(doc.contains(&("extracted_at_ms".into(), Val::Int(1_700_000_000_000))));
        // Nothing under a table is read.
        assert_eq!(text(&doc, "scrape_url"), "");
        assert_eq!(text(&doc, "video_path"), "");
        // A literal string, a string that never ends, an array that never closes.
        assert_eq!(parse("a = 'lit\\eral'\nb = \"no end\nc = [\"x\"").len(), 1);
        assert_eq!(text(&parse("a = 'lit\\eral'"), "a"), "lit\\eral");
        assert!(parse("\0\0\0").is_empty());
    }

    #[test]
    fn resolves_and_answers_any_query_for_its_folder() {
        let dir = item_dir("ok");
        std::fs::write(sidecar_path(&dir), SAMPLE).unwrap();
        let rec = resolve_sidecar(&sidecar_path(&dir)).unwrap();
        assert_eq!(
            (rec.provider.as_str(), rec.title.as_str(), rec.year),
            (SIDECAR_ID, "Lighthouse at Dusk - Studio", 2024)
        );
        assert_eq!(rec.genres, ["Short Film", "4K", "Landscape", "Sea \"Storm\""]);
        assert_eq!(rec.synopsis, "Line one.\nLine é.");
        assert!(rec.poster.is_empty() && rec.cover.is_empty());

        let hits = search_sidecar(&MetadataSearchInput {
            query: "a file name that says something else".into(),
            kind: "anime".into(),
            limit: 5,
            dir: dir.to_string_lossy().into(),
        });
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].remote_id, sidecar_path(&dir).to_string_lossy());
        assert!(search_sidecar(&MetadataSearchInput::default()).is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn refuses_what_is_not_a_sidecar() {
        let dir = item_dir("bad");
        // A path a client made up, a sidecar without a title, an empty folder.
        std::fs::write(dir.join("secrets.toml"), "title = \"x\"").unwrap();
        assert_eq!(resolve_sidecar(&dir.join("secrets.toml")).unwrap_err().code, "invalid-message");
        std::fs::write(sidecar_path(&dir), "tags = [\"a\"]\n").unwrap();
        assert!(resolve_sidecar(&sidecar_path(&dir)).is_err());
        assert!(
            search_sidecar(&MetadataSearchInput { dir: dir.to_string_lossy().into(), ..Default::default() }).is_empty()
        );
        std::fs::remove_file(sidecar_path(&dir)).unwrap();
        assert_eq!(
            Sidecar.invoke(CAP_METADATA_RESOLVE, "", json!({"remote_id": sidecar_path(&dir)})).unwrap_err().code,
            "not-found"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn identity_comes_from_the_sidecar_when_there_is_one() {
        let dir = item_dir("ident");
        let cand = |name: &str| Candidate { path: dir.join(name).to_string_lossy().into(), ..Default::default() };
        // No sidecar: declined, the name-based identifiers decide.
        assert!(!identify_sidecar(&cand("Some Show - 03.mkv")).accepted());

        std::fs::write(
            sidecar_path(&dir),
            "title = \"Some Show - EP3\"\nseries = \"Some Show\"\nepisode = \"03\"\nupload_date = \"2021-07-01\"\n",
        )
        .unwrap();
        let p = identify_sidecar(&cand("whatever the file is called.mp4"));
        assert_eq!((p.kind.as_str(), p.title.as_str(), p.episode, p.year), ("episode", "Some Show", 3, 2021));
        assert_eq!((p.plugin_id.as_str(), p.confidence), (SIDECAR_IDENTIFY_ID, 0.95));
        // Not a video: declined, so an archive still goes to the comic identifier.
        assert!(!identify_sidecar(&cand("extras.cbz")).accepted());

        // A series without an episode number, or no series: one video by its title.
        std::fs::write(sidecar_path(&dir), "title = \"A Short\"\nseries = \"A Short\"\nepisode = \"special\"\n")
            .unwrap();
        let p = identify_sidecar(&cand("x.mp4"));
        assert_eq!((p.kind.as_str(), p.title.as_str(), p.episode), ("video", "A Short", 0));
        std::fs::write(sidecar_path(&dir), "tags = [\"a\"]\n").unwrap();
        assert!(!identify_sidecar(&cand("x.mp4")).accepted());
        let out = SidecarIdentifier.invoke(CAP_MEDIA_IDENTIFY, "identify", json!(cand("x.mp4"))).unwrap();
        assert_eq!(out["title"], "");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn artwork_is_the_still_in_the_item_folder() {
        let dir = item_dir("art");
        let media = dir.join("item.mp4");
        assert_eq!(artwork(&media), None);
        std::fs::write(dir.join(DIR).join("thumbs").join("thumbnail.jpg"), b"").unwrap();
        assert_eq!(artwork(&media), None, "an empty file is not artwork");
        std::fs::write(dir.join(DIR).join("thumbs").join("banner.jpg"), b"jpg").unwrap();
        assert_eq!(artwork(&media), Some(dir.join(DIR).join("thumbs").join("banner.jpg")));
        std::fs::write(dir.join(DIR).join("thumbs").join("thumbnail.jpg"), b"jpg").unwrap();
        let still = dir.join(DIR).join("thumbs").join("thumbnail.jpg");
        assert_eq!(artwork(&media), Some(still.clone()));
        assert_eq!(image_type(&still), "image/jpeg");
        std::fs::write(&still, b"RIFF\x10\0\0\0WEBPVP8 ").unwrap();
        assert_eq!(image_type(&still), "image/webp");
        std::fs::write(&still, b"\x89PNG\r\n\x1a\n").unwrap();
        assert_eq!(image_type(&still), "image/png");
        assert_eq!(image_type(&dir.join("missing.jpg")), "image/jpeg");
        std::fs::remove_dir_all(dir).ok();
    }
}
