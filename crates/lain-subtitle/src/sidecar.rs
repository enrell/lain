//! Sidecar discovery: `<stem>.<tag>[.forced][.sdh].<srt|ass|ssa|vtt>`.

use std::path::{Path, PathBuf};

use lain_api::Sidecar;

const FORMATS: &[&str] = &["srt", "ass", "ssa", "vtt"];

/// ISO 639-1 to ISO 639-2/T (D-071).
const ISO1: &[(&str, &str)] = &[
    ("af", "afr"),
    ("ar", "ara"),
    ("az", "aze"),
    ("be", "bel"),
    ("bg", "bul"),
    ("bn", "ben"),
    ("bs", "bos"),
    ("ca", "cat"),
    ("cs", "ces"),
    ("cy", "cym"),
    ("da", "dan"),
    ("de", "deu"),
    ("el", "ell"),
    ("en", "eng"),
    ("eo", "epo"),
    ("es", "spa"),
    ("et", "est"),
    ("eu", "eus"),
    ("fa", "fas"),
    ("fi", "fin"),
    ("fr", "fra"),
    ("ga", "gle"),
    ("gl", "glg"),
    ("he", "heb"),
    ("hi", "hin"),
    ("hr", "hrv"),
    ("hu", "hun"),
    ("hy", "hye"),
    ("id", "ind"),
    ("is", "isl"),
    ("it", "ita"),
    ("ja", "jpn"),
    ("ka", "kat"),
    ("kk", "kaz"),
    ("km", "khm"),
    ("kn", "kan"),
    ("ko", "kor"),
    ("lt", "lit"),
    ("lv", "lav"),
    ("mk", "mkd"),
    ("ml", "mal"),
    ("mn", "mon"),
    ("mr", "mar"),
    ("ms", "msa"),
    ("my", "mya"),
    ("nb", "nob"),
    ("ne", "nep"),
    ("nl", "nld"),
    ("nn", "nno"),
    ("no", "nor"),
    ("pa", "pan"),
    ("pl", "pol"),
    ("pt", "por"),
    ("ro", "ron"),
    ("ru", "rus"),
    ("si", "sin"),
    ("sk", "slk"),
    ("sl", "slv"),
    ("sq", "sqi"),
    ("sr", "srp"),
    ("sv", "swe"),
    ("sw", "swa"),
    ("ta", "tam"),
    ("te", "tel"),
    ("th", "tha"),
    ("tl", "tgl"),
    ("tr", "tur"),
    ("uk", "ukr"),
    ("ur", "urd"),
    ("uz", "uzb"),
    ("vi", "vie"),
    ("zh", "zho"),
];

/// ISO 639-2/B to /T.
const BIBLIOGRAPHIC: &[(&str, &str)] = &[
    ("fre", "fra"),
    ("ger", "deu"),
    ("chi", "zho"),
    ("dut", "nld"),
    ("gre", "ell"),
    ("rum", "ron"),
    ("cze", "ces"),
    ("slo", "slk"),
    ("per", "fas"),
    ("may", "msa"),
    ("alb", "sqi"),
    ("arm", "hye"),
    ("baq", "eus"),
    ("bur", "mya"),
    ("ice", "isl"),
    ("mac", "mkd"),
    ("geo", "kat"),
    ("wel", "cym"),
];

/// Folds a language tag (639-1, 639-2/T or /B, with or without a
/// region) to ISO 639-2/T; empty when it is not a known language.
pub fn language(tag: &str) -> String {
    let t = tag.trim().replace('_', "-").to_lowercase();
    let t = t.split('-').next().unwrap_or("");
    match t.len() {
        2 => ISO1.iter().find(|(a, _)| *a == t).map(|(_, b)| b.to_string()).unwrap_or_default(),
        3 => {
            if let Some((_, b)) = BIBLIOGRAPHIC.iter().find(|(a, _)| *a == t) {
                return b.to_string();
            }
            if t == "fil" || ISO1.iter().any(|(_, b)| *b == t) { t.to_string() } else { String::new() }
        }
        _ => String::new(),
    }
}

/// One discovered sidecar: the wire view plus where it is (never sent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub path: PathBuf,
    pub view: Sidecar,
}

fn stem(media: &Path) -> String {
    let name = media.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match name.rfind('.') {
        Some(i) => name[..i].to_string(),
        None => name,
    }
}

/// The sidecars of a media file, by name only, sorted by file name and
/// indexed in that order. Symlinks are skipped: they could point
/// anywhere.
pub fn discover(media: &Path) -> Vec<Found> {
    let Some(dir) = media.parent() else { return Vec::new() };
    let prefix = format!("{}.", stem(media));
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<Found> = Vec::new();
    for e in rd.flatten() {
        let Ok(name) = e.file_name().into_string() else { continue };
        let Ok(ft) = e.file_type() else { continue };
        if !ft.is_file() || !name.starts_with(&prefix) {
            continue;
        }
        let Some(dot) = name.rfind('.') else { continue };
        let format = name[dot + 1..].to_lowercase();
        if !FORMATS.contains(&format.as_str()) {
            continue;
        }
        let mut s = Sidecar { name: name.clone(), format, language: "und".into(), ..Sidecar::default() };
        // "<stem>.srt" has no middle; "<stem>.en.forced.srt" has "en.forced".
        let rest = &name[prefix.len()..];
        let middle = if rest.contains('.') { &rest[..rest.len() - (name.len() - dot)] } else { "" };
        for tok in middle.split('.') {
            match tok.to_lowercase().as_str() {
                "" | "default" => {}
                "forced" => s.forced = true,
                "sdh" | "hi" | "cc" => s.hi = true,
                _ => {
                    if s.tag.is_empty() {
                        s.tag = tok.to_string();
                        let l = language(tok);
                        if !l.is_empty() {
                            s.language = l;
                        }
                    }
                }
            }
        }
        out.push(Found { path: dir.join(&name), view: s });
    }
    out.sort_by(|a, b| a.view.name.cmp(&b.view.name));
    for (i, f) in out.iter_mut().enumerate() {
        f.view.index = i as i64;
    }
    out
}

/// The language tag Lain writes in a sidecar name (A-35): the region
/// form when the provider gave one (`pt-BR`), else ISO 639-1 when it
/// exists, else the ISO 639-2 code.
pub fn tag(lang: &str, region: &str) -> String {
    if !region.is_empty() {
        return region.to_string();
    }
    ISO1.iter().find(|(_, three)| *three == lang).map(|(two, _)| two.to_string()).unwrap_or_else(|| lang.to_string())
}

/// Where Lain writes a subtitle for `media` (A-35):
/// `<stem>.<tag>[.forced][.sdh].<format>`, next to it.
pub fn sidecar_path(media: &Path, lang: &str, region: &str, forced: bool, hi: bool, format: &str) -> PathBuf {
    let mut parts = vec![stem(media), tag(lang, region)];
    if forced {
        parts.push("forced".into());
    }
    if hi {
        parts.push("sdh".into());
    }
    let name = format!("{}.{format}", parts.join("."));
    media.parent().map(|d| d.join(&name)).unwrap_or_else(|| PathBuf::from(name))
}

/// The OpenSubtitles file hash (A-31, D-138): the file size plus the
/// little-endian 64-bit words of the first and last 64 KiB, as 16 hex
/// digits. Files under 64 KiB cannot be hashed.
pub fn movie_hash(path: &Path) -> std::io::Result<String> {
    use std::io::{Read, Seek, SeekFrom};
    const CHUNK: u64 = 65536;
    let mut f = std::fs::File::open(path)?;
    let size = f.metadata()?.len();
    if size < CHUNK {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "subtitle: file too small to hash"));
    }
    let mut h = size;
    let mut buf = vec![0u8; CHUNK as usize];
    for off in [0, size - CHUNK] {
        f.seek(SeekFrom::Start(off))?;
        f.read_exact(&mut buf)?;
        for w in buf.as_chunks::<8>().0 {
            h = h.wrapping_add(u64::from_le_bytes(*w));
        }
    }
    Ok(format!("{h:016x}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_folding() {
        assert_eq!(language("en"), "eng");
        assert_eq!(language("pt-BR"), "por");
        assert_eq!(language("pt_br"), "por");
        assert_eq!(language("ger"), "deu");
        assert_eq!(language("jpn"), "jpn");
        assert_eq!(language("und"), "");
        assert_eq!(language("xx"), "");
        assert_eq!(language("english"), "");
    }

    #[test]
    fn discovery_by_name() {
        let dir = std::env::temp_dir().join(format!("lain-sidecar-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let media = dir.join("Show - 01.mkv");
        for n in [
            "Show - 01.mkv",
            "Show - 01.srt",
            "Show - 01.en.forced.srt",
            "Show - 01.pt-BR.sdh.ass",
            "Show - 01.ja.txt",
            "Show - 02.en.srt",
            "Show - 01.zz.vtt",
        ] {
            std::fs::write(dir.join(n), b"x").unwrap();
        }
        std::fs::create_dir_all(dir.join("Show - 01.dir.srt")).unwrap();
        let got: Vec<_> = discover(&media).into_iter().map(|f| f.view).collect();
        let names: Vec<_> = got.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["Show - 01.en.forced.srt", "Show - 01.pt-BR.sdh.ass", "Show - 01.srt", "Show - 01.zz.vtt"]
        );
        assert_eq!(
            (got[0].language.as_str(), got[0].tag.as_str(), got[0].forced, got[0].index),
            ("eng", "en", true, 0)
        );
        assert_eq!((got[1].language.as_str(), got[1].hi, got[1].format.as_str()), ("por", true, "ass"));
        assert_eq!((got[2].language.as_str(), got[2].tag.as_str()), ("und", ""));
        assert_eq!((got[3].language.as_str(), got[3].tag.as_str()), ("und", "zz"));
        assert!(discover(&dir.join("nope/x.mkv")).is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn sidecar_names_round_trip() {
        let media = Path::new("/l/Show - S01E01.mkv");
        for (lang, region, forced, hi, format, want) in [
            ("eng", "", false, false, "srt", "/l/Show - S01E01.en.srt"),
            ("por", "pt-BR", true, false, "ass", "/l/Show - S01E01.pt-BR.forced.ass"),
            ("jpn", "", false, true, "srt", "/l/Show - S01E01.ja.sdh.srt"),
            ("fil", "", false, false, "vtt", "/l/Show - S01E01.fil.vtt"),
        ] {
            assert_eq!(sidecar_path(media, lang, region, forced, hi, format), Path::new(want));
        }
        let dir = std::env::temp_dir().join(format!("lain-sidecar-rt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let media = dir.join("Film (2001).mkv");
        std::fs::write(sidecar_path(&media, "por", "pt-BR", true, false, "srt"), b"x").unwrap();
        let s = discover(&media);
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].view.language.as_str(), s[0].view.forced, s[0].view.tag.as_str()), ("por", true, "pt-BR"));
        std::fs::remove_dir_all(dir).ok();
    }

    /// The published algorithm written the slow, obvious way.
    fn ref_hash(b: &[u8]) -> String {
        let mut h = b.len() as u64;
        let n = b.len().min(65536);
        for chunk in [&b[..n], &b[b.len() - n..]] {
            for w in chunk.as_chunks::<8>().0 {
                h = h.wrapping_add(u64::from_le_bytes(*w));
            }
        }
        format!("{h:016x}")
    }

    #[test]
    fn movie_hash_reads_head_and_tail() {
        let dir = std::env::temp_dir().join(format!("lain-moviehash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.mkv");
        let mut data: Vec<u8> = (0..300_000u32).map(|i| (i.wrapping_mul(131) + 7) as u8).collect();
        std::fs::write(&p, &data).unwrap();
        let got = movie_hash(&p).unwrap();
        assert_eq!((got.len(), got.clone()), (16, ref_hash(&data)));
        data[150_000] ^= 0xff;
        std::fs::write(&p, &data).unwrap();
        assert_eq!(movie_hash(&p).unwrap(), got, "the middle does not count");
        data[10] ^= 0xff;
        std::fs::write(&p, &data).unwrap();
        assert_ne!(movie_hash(&p).unwrap(), got, "the head does");
        let small = dir.join("small.mkv");
        std::fs::write(&small, vec![0u8; 1000]).unwrap();
        assert!(movie_hash(&small).is_err());
        std::fs::remove_dir_all(dir).ok();
    }
}
