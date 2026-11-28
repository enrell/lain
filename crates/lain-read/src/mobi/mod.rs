//! MOBI (Mobipocket 6/7) and AZW3 (KF8) books, read from scratch and
//! converted to EPUB 3 (with an NCX for EPUB 2 readers).
//!
//! A `.mobi`/`.azw3` file is a PalmDB: record 0 holds the PalmDOC and
//! MOBI headers and the EXTH metadata; then come the compressed text
//! records, index records and resources (images, fonts). A joint file
//! carries a MOBI 7 book and a KF8 book after a BOUNDARY record; the
//! KF8 half is preferred. The format is detected from the headers.
//!
//! Every byte here is hostile input: reads are bounds-checked,
//! arithmetic is checked, sizes are capped, recursion is bounded, and
//! damage is a [`MobiError`], never a panic. Output is deterministic:
//! the same input gives the same EPUB, byte for byte.

pub(crate) mod encoding;
pub(crate) mod entities;
mod epub;
mod exth;
mod header;
pub(crate) mod html;
mod huffcdic;
mod indx;
mod kf8;
mod mobi7;
mod palmdb;
mod palmdoc;
mod resources;
mod text;
mod toc;

#[cfg(test)]
mod tests;

use std::fmt;

use epub::{Book, Doc, Item, SpineItem, TocEntry};
use header::Header;
use palmdb::PalmDb;
use resources::Resources;

/// Why a book could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MobiError {
    /// The book is encrypted.
    Drm,
    /// The file is damaged or not a MOBI book.
    Invalid(String),
    /// A valid book in a variant this reader does not handle.
    Unsupported(String),
}

impl fmt::Display for MobiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MobiError::Drm => write!(f, "mobi: the book is DRM-protected"),
            MobiError::Invalid(m) => write!(f, "mobi: invalid book: {m}"),
            MobiError::Unsupported(m) => write!(f, "mobi: unsupported book: {m}"),
        }
    }
}

impl std::error::Error for MobiError {}

/// A book's metadata, from its headers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metadata {
    /// "mobi" or "azw3" (the part that is converted).
    pub format: String,
    pub title: String,
    pub authors: Vec<String>,
    pub publisher: String,
    /// Plain text, tags stripped.
    pub description: String,
    pub subjects: Vec<String>,
    /// BCP 47 (such as "en") when known.
    pub language: String,
    pub isbn: String,
    pub published: String,
    pub rtl: bool,
    /// The cover image and its media type.
    pub cover: Option<(Vec<u8>, String)>,
}

/// Files larger than this are refused.
pub(crate) const MAX_FILE: usize = 512 << 20;
/// Decompressed text is capped at this.
pub(crate) const MAX_TEXT: usize = 256 << 20;
/// Records, and one record's decompressed text, are capped at this.
pub(crate) const MAX_RECORD: usize = 64 << 20;
/// Content documents per book (the EPUB's ZIP holds at most 65535 files).
pub(crate) const MAX_DOCS: usize = 60_000;

pub(crate) fn invalid<T>(msg: impl Into<String>) -> Result<T, MobiError> {
    Err(MobiError::Invalid(msg.into()))
}

pub(crate) fn be16(d: &[u8], at: usize) -> Option<u16> {
    let b = d.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([b[0], b[1]]))
}

pub(crate) fn be32(d: &[u8], at: usize) -> Option<u32> {
    let b = d.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// A converted book, before the cover and the package are added.
pub(crate) struct Content {
    pub docs: Vec<Doc>,
    pub spine: Vec<SpineItem>,
    pub items: Vec<Item>,
    pub toc: Vec<TocEntry>,
    /// The book's own spine says right to left.
    pub rtl: bool,
}

/// An `a href` that is kept as is: web and mail links.
pub(crate) fn external_link(v: &str) -> html::Url {
    let v = v.trim();
    let l = v.to_ascii_lowercase();
    if ["http://", "https://", "mailto:", "ftp://"].iter().any(|p| l.starts_with(p)) && v.len() > 8 {
        html::Url::Keep(v.to_string())
    } else {
        html::Url::Drop
    }
}

/// An id as a URL fragment (percent-encoding what a fragment cannot hold).
pub(crate) fn fragment(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    for c in id.chars() {
        match c {
            ' ' | '"' | '#' | '%' | '<' | '>' | '\\' | '^' | '`' | '{' | '|' | '}' => {
                out.push_str(&format!("%{:02X}", c as u32));
            }
            _ => out.push(c),
        }
    }
    out
}

/// The headers of a book: the header to convert (KF8 when there is one)
/// and, in a joint file, the MOBI 6/7 header of the other half.
struct Source<'a> {
    db: PalmDb<'a>,
    main: Header<'a>,
    /// The MOBI 7 header of a joint file.
    joint: Option<Header<'a>>,
    kf8: bool,
}

impl<'a> Source<'a> {
    fn open(data: &'a [u8]) -> Result<Source<'a>, MobiError> {
        if data.len() > MAX_FILE {
            return invalid("file exceeds 512 MB");
        }
        let db = PalmDb::parse(data)?;
        let first = Header::parse(db.need(0, "header")?, 0)?;
        if first.encryption != 0 {
            return Err(MobiError::Drm);
        }
        if first.version >= 8 {
            return Ok(Source { db, main: first, joint: None, kf8: true });
        }
        // A joint file names the KF8 record 0, right after BOUNDARY.
        let kf8 = first
            .exth
            .number(exth::KF8_BOUNDARY)
            .map(|v| v as usize)
            .filter(|&v| v > 0 && db.record(v - 1).is_some_and(|r| r.starts_with(b"BOUNDARY")))
            .and_then(|v| Header::parse(db.record(v)?, v).ok())
            .filter(|h| h.version >= 8);
        match kf8 {
            Some(k) if k.encryption != 0 => Err(MobiError::Drm),
            Some(k) => Ok(Source { db, main: k, joint: Some(first), kf8: true }),
            None => Ok(Source { db, main: first, joint: None, kf8: false }),
        }
    }

    /// The resource records, numbered from 1. In a joint file they sit
    /// in the MOBI 7 half and KF8 numbers them the same way.
    fn resources(&self) -> Resources<'a> {
        let n = self.db.len();
        let mut recs = Vec::new();
        let mut push = |from: usize, to: usize| {
            recs.extend((from..to.min(n)).filter_map(|i| self.db.record(i)));
        };
        match &self.joint {
            Some(m) => {
                if let Some(first) = m.first_resource {
                    let mut end = self.main.start.saturating_sub(1);
                    if let Some(c) = m.exth.number(exth::RESOURCE_COUNT).filter(|&c| c > 0) {
                        end = end.min(first.saturating_add(c as usize));
                    }
                    push(first, end);
                }
                if let Some(first) = self.main.first_resource.filter(|&f| f > self.main.start) {
                    push(first, n);
                }
            }
            None => {
                if let Some(first) = self.main.first_resource {
                    push(first, n);
                }
            }
        }
        Resources::new(recs)
    }
}

/// The book's metadata and its cover's resource number.
fn read_metadata(src: &Source<'_>, res: &Resources<'_>) -> (Metadata, Option<usize>) {
    let main = &src.main;
    let other = src.joint.as_ref();
    let utf8 = main.utf8();
    let get = |t: u32| main.exth.get(t).or_else(|| other.and_then(|o| o.exth.get(t)));
    let text = |t: u32| get(t).map(|b| clean(&encoding::decode(b, utf8))).unwrap_or_default();
    let list = |t: u32| {
        let mut v: Vec<String> = main.exth.all(t).map(|b| clean(&encoding::decode(b, utf8))).collect();
        if v.is_empty()
            && let Some(o) = other
        {
            v = o.exth.all(t).map(|b| clean(&encoding::decode(b, o.utf8()))).collect();
        }
        let mut seen = std::collections::HashSet::new();
        v.retain(|s| !s.is_empty() && seen.insert(s.clone()));
        v
    };
    let mut title = text(exth::UPDATED_TITLE);
    if title.is_empty() {
        title = clean(&encoding::decode(main.full_name, utf8));
    }
    if title.is_empty() {
        title = clean(&encoding::decode(src.db.name, false).replace('_', " "));
    }
    let description = get(exth::DESCRIPTION).map(|b| html::plain_text(b, utf8)).unwrap_or_default();
    let mut language = normalize_language(&text(exth::LANGUAGE));
    if language.is_empty() {
        language = locale_language(main.locale).to_string();
    }
    let mode = text(exth::WRITING_MODE).to_ascii_lowercase();
    let rtl = text(exth::PAGE_PROGRESSION).eq_ignore_ascii_case("rtl") || mode.ends_with("-rl");
    let cover_num = cover_number(src).filter(|&n| res.image(n).is_some());
    let cover = cover_num.and_then(|n| res.image(n)).map(|(d, _, mime)| (d.to_vec(), mime.to_string()));
    let meta = Metadata {
        format: if src.kf8 { "azw3" } else { "mobi" }.to_string(),
        title,
        authors: list(exth::AUTHOR),
        publisher: text(exth::PUBLISHER),
        description,
        subjects: list(exth::SUBJECT),
        language,
        isbn: text(exth::ISBN),
        published: text(exth::PUBLISHED),
        rtl,
        cover,
    };
    (meta, cover_num)
}

/// The cover's resource number: EXTH 201 (an offset from the first
/// resource), else KF8's `kindle:embed` cover URI.
fn cover_number(src: &Source<'_>) -> Option<usize> {
    let headers = [Some(&src.main), src.joint.as_ref()];
    for h in headers.iter().flatten() {
        if let Some(off) = h.exth.number(exth::COVER_OFFSET).filter(|&o| o != u32::MAX) {
            return (off as usize).checked_add(1);
        }
    }
    for h in headers.iter().flatten() {
        if let Some(uri) = h.exth.get(exth::KF8_COVER_URI) {
            let uri = String::from_utf8_lossy(uri);
            let id = uri.trim().strip_prefix("kindle:embed:")?.split('?').next()?;
            return resources::base32(id);
        }
    }
    None
}

/// Trims and collapses whitespace, dropping control characters.
fn clean(s: &str) -> String {
    let s: String = s.chars().filter(|c| !c.is_control() || c.is_whitespace()).collect();
    html::collapse_ws(&s)
}

/// EXTH 524 as BCP 47: `en_US` becomes `en-US`, junk becomes "".
fn normalize_language(s: &str) -> String {
    let s = s.trim().replace('_', "-");
    let ok = !s.is_empty()
        && s.len() <= 35
        && s.as_bytes()[0].is_ascii_alphabetic()
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if !ok {
        return String::new();
    }
    match s.split_once('-') {
        Some((l, r)) => format!("{}-{r}", l.to_ascii_lowercase()),
        None => s.to_ascii_lowercase(),
    }
}

/// The language of a MOBI locale (a Windows LANGID's primary language).
fn locale_language(locale: u32) -> &'static str {
    match locale & 0xff {
        0x01 => "ar",
        0x02 => "bg",
        0x03 => "ca",
        0x04 => "zh",
        0x05 => "cs",
        0x06 => "da",
        0x07 => "de",
        0x08 => "el",
        0x09 => "en",
        0x0a => "es",
        0x0b => "fi",
        0x0c => "fr",
        0x0d => "he",
        0x0e => "hu",
        0x0f => "is",
        0x10 => "it",
        0x11 => "ja",
        0x12 => "ko",
        0x13 => "nl",
        0x14 => "nb",
        0x15 => "pl",
        0x16 => "pt",
        0x17 => "rm",
        0x18 => "ro",
        0x19 => "ru",
        0x1a => "hr",
        0x1b => "sk",
        0x1c => "sq",
        0x1d => "sv",
        0x1e => "th",
        0x1f => "tr",
        0x20 => "ur",
        0x21 => "id",
        0x22 => "uk",
        0x23 => "be",
        0x24 => "sl",
        0x25 => "et",
        0x26 => "lv",
        0x27 => "lt",
        0x29 => "fa",
        0x2a => "vi",
        0x2b => "hy",
        0x2c => "az",
        0x2d => "eu",
        0x2f => "mk",
        0x36 => "af",
        0x37 => "ka",
        0x38 => "fo",
        0x39 => "hi",
        0x3a => "mt",
        0x3c => "ga",
        0x3e => "ms",
        0x3f => "kk",
        0x41 => "sw",
        0x43 => "uz",
        0x44 => "tt",
        0x45 => "bn",
        0x46 => "pa",
        0x47 => "gu",
        0x48 => "or",
        0x49 => "ta",
        0x4a => "te",
        0x4b => "kn",
        0x4c => "ml",
        0x4d => "as",
        0x4e => "mr",
        0x4f => "sa",
        0x50 => "mn",
        0x52 => "cy",
        0x56 => "gl",
        0x5a => "syr",
        0x61 => "ne",
        0x62 => "fy",
        0x63 => "ps",
        0x64 => "fil",
        _ => "",
    }
}

/// Fast metadata: the PalmDB, the headers, EXTH and the cover record.
/// The text is never decompressed.
pub fn metadata(data: &[u8]) -> Result<Metadata, MobiError> {
    let src = Source::open(data)?;
    src.main.check_text()?;
    let res = src.resources();
    Ok(read_metadata(&src, &res).0)
}

/// The whole book as an EPUB 3 file.
pub fn to_epub(data: &[u8]) -> Result<Vec<u8>, MobiError> {
    let src = Source::open(data)?;
    let res = src.resources();
    let (mut meta, cover_num) = read_metadata(&src, &res);
    let content = if src.kf8 {
        match kf8::convert(&src.db, &src.main, &res) {
            Ok(c) => c,
            // A damaged KF8 half of a joint file: fall back to MOBI 7.
            Err(MobiError::Invalid(why)) => match &src.joint {
                Some(m) => {
                    meta.format = "mobi".to_string();
                    mobi7::convert(&src.db, m, &res)?
                }
                None => return Err(MobiError::Invalid(why)),
            },
            Err(e) => return Err(e),
        }
    } else {
        mobi7::convert(&src.db, &src.main, &res)?
    };
    if meta.title.is_empty() {
        meta.title = "Untitled".to_string();
    }
    let rtl = meta.rtl || content.rtl;
    let book = package(content, &res, cover_num)?;
    let identifier = identifier(&meta);
    let modified = modified(&src.db, &meta);
    Ok(epub::write(&book, &meta, &identifier, &modified, rtl))
}

/// Adds the cover (and a cover page when the text has none).
fn package(mut c: Content, res: &Resources<'_>, cover: Option<usize>) -> Result<Book, MobiError> {
    if let Some(n) = cover
        && let Some((data, ext, mime)) = res.image(n)
    {
        let path = resources::image_path(n, ext);
        match c.items.iter_mut().find(|i| i.path == path) {
            Some(i) => i.cover = true,
            None => {
                let mut item = Item::new(&path, mime, data.to_vec(), false);
                item.cover = true;
                c.items.push(item);
                c.items.sort_by(|a, b| a.path.cmp(&b.path));
            }
        }
        let quoted = format!("\"{path}\"");
        if !c.docs.iter().any(|d| d.body.contains(&quoted)) {
            let mut body = String::from("<div style=\"text-align: center; page-break-after: always\"><img src=\"");
            html::escape(&mut body, &path, true);
            body.push_str("\" alt=\"Cover\" style=\"max-width: 100%; max-height: 100%\"/></div>");
            c.docs.push(Doc {
                path: "cover.xhtml".to_string(),
                title: Some("Cover".to_string()),
                head: String::new(),
                body_attrs: String::new(),
                body,
                svg: false,
            });
            c.spine.insert(0, SpineItem { doc: c.docs.len() - 1, props: None });
        }
    }
    if c.docs.len() + c.items.len() > MAX_DOCS {
        return invalid("too many files for one EPUB");
    }
    Ok(Book { docs: c.docs, spine: c.spine, items: c.items, toc: c.toc })
}

/// `urn:isbn:...`, else a `urn:lain:` hash of the title and authors.
fn identifier(meta: &Metadata) -> String {
    let isbn: String = meta.isbn.chars().filter(|c| c.is_ascii_digit() || *c == 'X' || *c == 'x').collect();
    if isbn.len() == 10 || isbn.len() == 13 {
        return format!("urn:isbn:{}", isbn.to_ascii_uppercase());
    }
    // FNV-1a, 64 bits.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |b: &[u8]| {
        for &x in b {
            h ^= u64::from(x);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        h ^= 0xff;
        h = h.wrapping_mul(0x0100_0000_01b3);
    };
    feed(meta.title.as_bytes());
    for a in &meta.authors {
        feed(a.as_bytes());
    }
    format!("urn:lain:{h:016x}")
}

/// `dcterms:modified`, fixed by the book: the PalmDB modification (or
/// creation) date, else the publication date, else 2000-01-01.
fn modified(db: &PalmDb<'_>, meta: &Metadata) -> String {
    for t in [db.modified, db.created] {
        if t == 0 {
            continue;
        }
        // PalmOS counts from 1904 when the high bit is set, else from 1970.
        let unix = if t & 0x8000_0000 != 0 { i64::from(t) - 2_082_844_800 } else { i64::from(t) };
        let (y, m, d, hh, mm, ss) = civil(unix);
        if (1990..2100).contains(&y) {
            return format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z");
        }
    }
    if let Some(d) = epub::w3c_date(&meta.published)
        && d.len() == 10
    {
        return format!("{d}T00:00:00Z");
    }
    "2000-01-01T00:00:00Z".to_string()
}

/// Seconds since 1970 as a UTC date and time.
fn civil(unix: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, (secs / 3600) as u32, (secs % 3600 / 60) as u32, (secs % 60) as u32)
}
