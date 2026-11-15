//! The sidecar identifier (`lain.media.identify@1`): a video that has a
//! `.metadata/meta.toml` next to it (the layout the owner's downloader
//! writes, one folder per file) is identified from it. The file names
//! the series and the episode, which the name alone cannot: with one
//! file per folder, "Some Title 2" has no siblings to be numbered with,
//! so every episode stayed a title of its own.
//!
//! Only the flat keys before the first `[table]` are read, as
//! `key = "string"`; anything else is ignored, and a file that is
//! missing, too large or says nothing useful declines, so the next
//! identifier runs.

use std::io::Read;
use std::path::Path;

use serde_json::{Value, json};

use lain_core::{Error, Provider};
use lain_identify::{CAP_MEDIA_IDENTIFY, Candidate, Proposal};

pub const SIDECAR_ID: &str = "lain-identify-sidecar";

/// Where the sidecar sits, relative to the media file's folder.
const SIDECAR_PATH: &str = ".metadata/meta.toml";
/// A sidecar larger than this is not read.
const MAX_BYTES: u64 = 256 * 1024;
const MAX_TITLE: usize = 300;

/// The value of `key = "…"` on this line, with `\"` and `\\` unescaped.
fn string_value<'a>(line: &'a str, key: &str) -> Option<String> {
    let rest = line.strip_prefix(key)?.trim_start();
    let rest: &'a str = rest.strip_prefix('=')?.trim_start();
    let body = rest.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' | 't' | 'r' => out.push(' '),
                other => out.push(other),
            },
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    None
}

/// The proposal a sidecar's text makes; `None` declines.
pub fn parse(text: &str) -> Option<Proposal> {
    let (mut series, mut episode, mut media) = (None, None, None);
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            break;
        }
        if series.is_none() {
            series = string_value(line, "series");
        }
        if episode.is_none() {
            episode = string_value(line, "episode");
        }
        if media.is_none() {
            media = string_value(line, "media_type");
        }
    }
    if media.is_some_and(|m| m != "video") {
        return None;
    }
    let title = series?.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() || title.len() > MAX_TITLE {
        return None;
    }
    let episode = episode.and_then(|e| e.trim().parse::<i64>().ok()).filter(|n| (1..=9999).contains(n)).unwrap_or(0);
    Some(Proposal {
        kind: if episode > 0 { "episode" } else { "video" }.into(),
        title,
        episode,
        confidence: 0.9,
        evidence: vec!["sidecar".into()],
        plugin_id: SIDECAR_ID.into(),
        ..Proposal::default()
    })
}

fn read(path: &Path) -> Option<String> {
    let f = std::fs::File::open(path).ok()?;
    if f.metadata().ok()?.len() > MAX_BYTES {
        return None;
    }
    let mut text = String::new();
    f.take(MAX_BYTES).read_to_string(&mut text).ok()?;
    Some(text)
}

/// Identifies from the sidecar next to the file.
pub struct Sidecar;

impl Sidecar {
    pub fn identify(&self, c: &Candidate) -> Proposal {
        Path::new(&c.path)
            .parent()
            .and_then(|dir| read(&dir.join(SIDECAR_PATH)))
            .and_then(|text| parse(&text))
            .unwrap_or_default()
    }
}

impl Provider for Sidecar {
    fn id(&self) -> &str {
        SIDECAR_ID
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
        Ok(json!(self.identify(&c)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EP: &str = "schema_version = 2\nmedia_type = \"video\"\ntitle = \"Some Show 2\"\nseries = \"Some  Show\"\nepisode = \"2\"\nsource_site = \"site-exemplo\"\ntags = [\n    \"a\",\n]\n\n[remote]\nseries = \"Other\"\nepisode = \"9\"\n";

    #[test]
    fn a_sidecar_names_the_series_and_the_episode() {
        let p = parse(EP).unwrap();
        assert_eq!((p.kind.as_str(), p.title.as_str(), p.episode), ("episode", "Some Show", 2));
        assert_eq!((p.plugin_id.as_str(), p.evidence.as_slice()), (SIDECAR_ID, &["sidecar".to_string()][..]));
        assert!(p.accepted());
        // No episode: one video under the series' name.
        let single = parse("series = \"Name: With \\\"Quotes\\\"\"\n").unwrap();
        assert_eq!(
            (single.kind.as_str(), single.title.as_str(), single.episode),
            ("video", "Name: With \"Quotes\"", 0)
        );
        // An episode that is not a small positive number is no episode.
        for odd in ["0", "-1", "two", "99999", ""] {
            let p = parse(&format!("series = \"S\"\nepisode = \"{odd}\"\n")).unwrap();
            assert_eq!((p.kind.as_str(), p.episode), ("video", 0), "{odd}");
        }
    }

    #[test]
    fn a_sidecar_that_says_nothing_declines() {
        for text in [
            "",
            "title = \"Only A Title\"\n",
            "series = \"\"\nepisode = \"1\"\n",
            "series = \"   \"\n",
            "series = unquoted\n",
            "series = \"unterminated\n",
            "media_type = \"image\"\nseries = \"S\"\n",
            "[remote]\nseries = \"S\"\n",
            "seriesx = \"S\"\n",
        ] {
            assert!(parse(text).is_none(), "{text:?}");
        }
        assert!(parse(&format!("series = \"{}\"\n", "x".repeat(MAX_TITLE + 1))).is_none());
    }

    #[test]
    fn the_file_is_read_from_the_folder_of_the_media_file() {
        let dir = std::env::temp_dir().join(format!("lain-sidecar-{}", std::process::id()));
        let show = dir.join("Some Show 2");
        std::fs::create_dir_all(show.join(".metadata")).unwrap();
        std::fs::write(show.join(".metadata/meta.toml"), EP).unwrap();
        let c = |p: &Path| Candidate { path: p.to_string_lossy().into_owned(), ..Candidate::default() };
        let p = Sidecar.identify(&c(&show.join("Some Show 2.mp4")));
        assert_eq!((p.title.as_str(), p.episode), ("Some Show", 2));
        // No sidecar, and a path with no folder: declined, never an error.
        assert!(!Sidecar.identify(&c(&dir.join("Bare.mp4"))).accepted());
        assert!(!Sidecar.identify(&c(Path::new("x.mp4"))).accepted());
        let out = Sidecar.invoke(CAP_MEDIA_IDENTIFY, "identify", json!({"path": show.join("a.mp4")})).unwrap();
        assert_eq!(out["episode"], 2);
        assert_eq!(
            Sidecar.invoke(CAP_MEDIA_IDENTIFY, "identify", json!({"path": 4})).unwrap_err().code,
            "invalid-message"
        );
        assert_eq!(Sidecar.invoke("lain.nope@1", "", json!({})).unwrap_err().code, "invalid-message");
        std::fs::remove_dir_all(dir).ok();
    }
}
