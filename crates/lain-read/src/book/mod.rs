//! Books (D-303..D-308): EPUB read as it is, MOBI and AZW3 converted to
//! EPUB in memory, PDF through `lain-pdf`. The provider answers the
//! manifest (`lain.book.manifest@1`) and, as `lain-metadata-book`, the
//! book's own metadata to the enrich pass; chapters, resources and
//! rendered pages are bytes the gateway serves itself, like comic
//! pages. Every cache here is bounded and in memory: nothing is written
//! to the data directory, so there is nothing to back up or clean.

pub mod epub;
pub mod markup;
pub mod sanitize;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use serde::Deserialize;
use serde_json::{Value, json};

use lain_api::{BOOK_COUNT_RULE, BOOK_PAGE_CHARS, BookChapter, BookManifest, BookTocEntry};
use lain_core::{Error, Provider};

use self::epub::{Epub, TocNode};
use crate::zip::Archive;

pub const CAP_BOOK_MANIFEST: &str = "lain.book.manifest@1";
pub const PROVIDER_ID: &str = "lain-book";
pub const METADATA_ID: &str = lain_api::BOOK_METADATA_ID;
pub const KIND_BOOK: &str = "book";

pub const INVALID_BOOK: &str = "invalid-book";
pub const DRM_PROTECTED: &str = "drm-protected";
pub const UNSUPPORTED_BOOK: &str = "unsupported-format";

/// Books opened, at most (each holds its index, and a converted book or
/// a PDF holds its bytes).
const MAX_BOOKS: usize = 24;
/// Bytes of converted books and PDFs kept in memory, at most.
const MAX_BOOK_BYTES: usize = 512 << 20;
/// Bytes of rendered PDF pages kept, at most.
const MAX_PAGE_BYTES: usize = 128 << 20;
/// A book file larger than this is refused.
const MAX_FILE: u64 = 1 << 30;
/// One member (a chapter, a picture, a font) is read up to this size.
const MAX_MEMBER: u64 = 64 << 20;
/// A rendered page is at most this wide.
pub const MAX_PAGE_WIDTH: usize = 4096;
/// The page width used when the reader asks none.
pub const DEFAULT_PAGE_WIDTH: usize = 1400;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookError {
    pub code: &'static str,
    pub message: String,
}

impl BookError {
    pub fn invalid(m: impl Into<String>) -> BookError {
        BookError { code: INVALID_BOOK, message: m.into() }
    }

    pub fn drm() -> BookError {
        BookError { code: DRM_PROTECTED, message: "the book is protected by DRM".into() }
    }

    pub fn not_found(what: &str) -> BookError {
        BookError { code: "not-found", message: format!("{what} not found") }
    }
}

impl From<BookError> for Error {
    fn from(e: BookError) -> Error {
        Error::new(e.code, e.message)
    }
}

/// The format of a book file, by extension.
pub fn format(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("epub") => "epub",
        Some("pdf") => "pdf",
        Some("mobi") => "mobi",
        Some("azw3" | "azw") => "azw3",
        _ => "",
    }
}

enum Content {
    Epub(Arc<Epub>),
    Pdf(Arc<lain_pdf::Document>),
}

/// An opened book: its manifest and what serving it needs.
pub struct Book {
    pub manifest: BookManifest,
    content: Content,
    /// Spine member path → chapter index.
    chapters: HashMap<String, usize>,
    /// Bytes held in memory (converted EPUB or PDF file).
    bytes: usize,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    path: PathBuf,
    mtime: u128,
    size: u64,
}

fn key(path: &Path) -> Result<Key, BookError> {
    let meta = std::fs::metadata(path)
        .map_err(|_| BookError { code: "dependency-unavailable", message: "book unavailable".into() })?;
    let mtime = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
    Ok(Key { path: path.to_path_buf(), mtime, size: meta.len() })
}

/// A small LRU by use counter.
struct Lru<K, V> {
    map: HashMap<K, (V, u64, usize)>,
    tick: u64,
    bytes: usize,
    max_items: usize,
    max_bytes: usize,
}

impl<K: std::hash::Hash + Eq + Clone, V: Clone> Lru<K, V> {
    fn new(max_items: usize, max_bytes: usize) -> Self {
        Lru { map: HashMap::new(), tick: 0, bytes: 0, max_items, max_bytes }
    }

    fn get(&mut self, k: &K) -> Option<V> {
        self.tick += 1;
        let t = self.tick;
        self.map.get_mut(k).map(|e| {
            e.1 = t;
            e.0.clone()
        })
    }

    fn put(&mut self, k: K, v: V, size: usize) {
        self.tick += 1;
        if let Some(old) = self.map.insert(k, (v, self.tick, size)) {
            self.bytes -= old.2;
        }
        self.bytes += size;
        while self.map.len() > self.max_items || (self.bytes > self.max_bytes && self.map.len() > 1) {
            let Some(oldest) = self.map.iter().min_by_key(|(_, e)| e.1).map(|(k, _)| k.clone()) else { break };
            if let Some(e) = self.map.remove(&oldest) {
                self.bytes -= e.2;
            }
        }
    }
}

/// A rendered page: the image file and its media type.
pub type RenderedPage = Arc<(Vec<u8>, &'static str)>;

/// The book module: opened books and rendered pages, cached.
pub struct Books {
    books: Mutex<Lru<Key, Arc<Book>>>,
    pages: Mutex<Lru<(Key, usize, usize), RenderedPage>>,
}

impl Default for Books {
    fn default() -> Books {
        Books::new()
    }
}

/// The process-wide cache every user of books shares (the routes, the
/// providers and the thumbnail maker).
pub fn shared() -> Arc<Books> {
    static SHARED: std::sync::OnceLock<Arc<Books>> = std::sync::OnceLock::new();
    SHARED.get_or_init(|| Arc::new(Books::new())).clone()
}

impl Books {
    pub fn new() -> Books {
        Books {
            books: Mutex::new(Lru::new(MAX_BOOKS, MAX_BOOK_BYTES)),
            pages: Mutex::new(Lru::new(4096, MAX_PAGE_BYTES)),
        }
    }

    /// The opened book at `path`, cached by file identity.
    pub fn open(&self, path: &Path) -> Result<Arc<Book>, BookError> {
        let k = key(path)?;
        if let Some(b) = self.books.lock().unwrap_or_else(|e| e.into_inner()).get(&k) {
            return Ok(b);
        }
        if k.size > MAX_FILE {
            return Err(BookError::invalid("the file is too large"));
        }
        let book = Arc::new(load(path)?);
        let size = book.bytes + 4096;
        self.books.lock().unwrap_or_else(|e| e.into_inner()).put(k, book.clone(), size);
        Ok(book)
    }

    pub fn manifest(&self, path: &Path) -> Result<BookManifest, BookError> {
        Ok(self.open(path)?.manifest.clone())
    }

    /// Chapter `n` sanitized, its resources under `prefix`.
    pub fn chapter(&self, path: &Path, n: usize, prefix: &str) -> Result<String, BookError> {
        let book = self.open(path)?;
        let Content::Epub(e) = &book.content else { return Err(BookError::not_found("chapter")) };
        let (member, _) = e.spine.get(n).ok_or_else(|| BookError::not_found("chapter"))?;
        let src = e.member(member, MAX_MEMBER)?;
        Ok(sanitize_chapter(e, &book.chapters, &src, member, prefix).html)
    }

    /// A member of an EPUB (or converted) book: bytes and media type.
    /// Stylesheets come back with their URLs rewritten under `prefix`.
    pub fn resource(&self, path: &Path, member: &str, prefix: &str) -> Result<(Vec<u8>, String), BookError> {
        let book = self.open(path)?;
        let Content::Epub(e) = &book.content else { return Err(BookError::not_found("resource")) };
        if member.starts_with("META-INF/") || member.contains("..") || e.archive.find(member).is_none() {
            return Err(BookError::not_found("resource"));
        }
        let data = e.member(member, MAX_MEMBER)?;
        let mime = e.media_type(member);
        if mime == "text/css" {
            let css = String::from_utf8_lossy(&data);
            let has = |p: &str| e.archive.find(p).is_some();
            let read_css = |_: &str| None;
            let b = sanitize::Book { chapters: &book.chapters, has: &has, css: &read_css, prefix };
            return Ok((sanitize::rewrite_css(&css, &b, markup::dir_of(member)).into_bytes(), mime));
        }
        Ok((data, mime))
    }

    /// PDF page sizes in points, rotation applied.
    pub fn page_sizes(&self, path: &Path) -> Result<Vec<(f64, f64)>, BookError> {
        let book = self.open(path)?;
        let Content::Pdf(d) = &book.content else { return Err(BookError::not_found("pages")) };
        Ok((0..d.page_count()).map(|i| d.page(i).map(|p| p.size()).unwrap_or((612.0, 792.0))).collect())
    }

    /// PDF page `n` rendered `width` pixels wide, as an image file.
    pub fn page(&self, path: &Path, n: usize, width: usize) -> Result<Arc<(Vec<u8>, &'static str)>, BookError> {
        let width = width.clamp(64, MAX_PAGE_WIDTH);
        let k = key(path)?;
        let pk = (k.clone(), n, width);
        if let Some(p) = self.pages.lock().unwrap_or_else(|e| e.into_inner()).get(&pk) {
            return Ok(p);
        }
        let book = self.open(path)?;
        let Content::Pdf(d) = &book.content else { return Err(BookError::not_found("page")) };
        let opts = lain_pdf::render::RenderOptions {
            width,
            deadline: Some(std::time::Instant::now() + std::time::Duration::from_secs(20)),
            ..Default::default()
        };
        let r = lain_pdf::render::render_page(d, n, &opts).map_err(pdf_err)?;
        if !r.stats.unsupported.is_empty() || r.stats.truncated || r.stats.missing_fonts {
            tracing::info!(file = %path.display(), page = n + 1, unsupported = ?r.stats.unsupported, truncated = r.stats.truncated, missing_fonts = r.stats.missing_fonts, "pdf page drawn partly");
        }
        let (bytes, mime) = lain_pdf::render::encode(&r).map_err(pdf_err)?;
        let out = Arc::new((bytes, mime));
        let size = out.0.len();
        self.pages.lock().unwrap_or_else(|e| e.into_inner()).put(pk, out.clone(), size);
        Ok(out)
    }

    /// The book's cover as an image file (bytes, media type): the EPUB's
    /// cover image, or a PDF's first page.
    pub fn cover(&self, path: &Path) -> Result<(Vec<u8>, String), BookError> {
        let book = self.open(path)?;
        match &book.content {
            Content::Epub(e) => {
                let c = e.cover.as_ref().ok_or_else(|| BookError::not_found("cover"))?;
                let data = e.member(c, MAX_MEMBER)?;
                Ok((data, e.media_type(c)))
            }
            Content::Pdf(_) => {
                let p = self.page(path, 0, 600)?;
                Ok((p.0.clone(), p.1.to_string()))
            }
        }
    }
}

fn pdf_err(e: lain_pdf::PdfError) -> BookError {
    use lain_pdf::ErrorKind;
    match e.kind {
        ErrorKind::Encrypted => BookError { code: DRM_PROTECTED, message: e.message },
        ErrorKind::Unsupported => BookError { code: UNSUPPORTED_BOOK, message: e.message },
        ErrorKind::NoSuchPage => BookError::not_found("page"),
        _ => BookError::invalid(e.message),
    }
}

fn sanitize_chapter(
    e: &Epub,
    chapters: &HashMap<String, usize>,
    src: &[u8],
    member: &str,
    prefix: &str,
) -> sanitize::Chapter {
    let has = |p: &str| e.archive.find(p).is_some();
    let css = |p: &str| e.member(p, 8 << 20).ok().map(|b| String::from_utf8_lossy(&b).into_owned());
    let b = sanitize::Book { chapters, has: &has, css: &css, prefix };
    sanitize::chapter(src, member, &b)
}

fn load(path: &Path) -> Result<Book, BookError> {
    match format(path) {
        "epub" => {
            let a = Archive::open(path).map_err(|e| BookError::invalid(e.to_string()))?;
            from_epub(Epub::open(a)?, "epub", 0)
        }
        f @ ("mobi" | "azw3") => {
            let data = std::fs::read(path).map_err(|e| BookError::invalid(e.to_string()))?;
            let meta = crate::mobi::metadata(&data).map_err(mobi_err)?;
            let epub = crate::mobi::to_epub(&data).map_err(mobi_err)?;
            let size = epub.len();
            let a = Archive::from_bytes(Arc::from(epub)).map_err(|e| BookError::invalid(e.to_string()))?;
            let format = if meta.format.is_empty() {
                f
            } else if meta.format == "azw3" {
                "azw3"
            } else {
                "mobi"
            };
            from_epub(Epub::open(a)?, format, size)
        }
        "pdf" => {
            let data = std::fs::read(path).map_err(|e| BookError::invalid(e.to_string()))?;
            let size = data.len();
            let doc = lain_pdf::Document::open(data).map_err(pdf_err)?;
            Ok(from_pdf(doc, size))
        }
        _ => Err(BookError { code: UNSUPPORTED_BOOK, message: "not a book format".into() }),
    }
}

fn mobi_err(e: crate::mobi::MobiError) -> BookError {
    match e {
        crate::mobi::MobiError::Drm => BookError::drm(),
        crate::mobi::MobiError::Unsupported(m) => BookError { code: UNSUPPORTED_BOOK, message: m },
        crate::mobi::MobiError::Invalid(m) => BookError::invalid(m),
    }
}

fn from_epub(e: Epub, format: &str, bytes: usize) -> Result<Book, BookError> {
    let chapters: HashMap<String, usize> = e.spine.iter().enumerate().map(|(i, (p, _))| (p.clone(), i)).collect();
    // Counting needs every chapter sanitized once (the same rule the
    // reader's text is, D-307); the prefix does not change the count.
    let mut list = Vec::with_capacity(e.spine.len());
    let mut total: i64 = 0;
    for (i, (p, linear)) in e.spine.iter().enumerate() {
        let chars = match e.member(p, MAX_MEMBER) {
            Ok(src) => sanitize_chapter(&e, &chapters, &src, p, "").chars,
            Err(_) => 0,
        };
        total += chars;
        list.push(BookChapter { index: i as i64, chars, linear: *linear });
    }
    let toc = toc_entries(&e.toc, &chapters, 0);
    let m = &e.meta;
    let manifest = BookManifest {
        format: format.into(),
        layout: "reflowable".into(),
        title: m.title.clone(),
        authors: m.authors.clone(),
        language: m.language.clone(),
        publisher: m.publisher.clone(),
        description: m.description.clone(),
        subjects: m.subjects.clone(),
        published: m.published.clone(),
        direction: if e.rtl { "rtl" } else { "ltr" }.into(),
        chapters: list,
        toc,
        pages: ((total + BOOK_PAGE_CHARS - 1) / BOOK_PAGE_CHARS).max(1),
        count_rule: BOOK_COUNT_RULE.into(),
        warnings: Vec::new(),
    };
    Ok(Book { manifest, content: Content::Epub(Arc::new(e)), chapters, bytes })
}

fn toc_entries(nodes: &[TocNode], chapters: &HashMap<String, usize>, depth: usize) -> Vec<BookTocEntry> {
    if depth > 8 {
        return Vec::new();
    }
    nodes
        .iter()
        .map(|n| BookTocEntry {
            title: n.title.clone(),
            chapter: chapters.get(&n.path).map_or(-1, |&c| c as i64),
            anchor: n.anchor.clone(),
            page: -1,
            children: toc_entries(&n.children, chapters, depth + 1),
        })
        .collect()
}

fn from_pdf(doc: lain_pdf::Document, bytes: usize) -> Book {
    let info = doc.info();
    let toc = outline_entries(&doc.outline(), 0);
    let split = |s: &str, seps: &[char]| -> Vec<String> {
        s.split(seps).map(str::trim).filter(|x| !x.is_empty()).map(|x| x.chars().take(100).collect()).take(20).collect()
    };
    let mut warnings = Vec::new();
    if !lain_pdf::font::system::available() && pdf_needs_fonts(&doc) {
        warnings.push("fonts-missing".to_string());
    }
    let manifest = BookManifest {
        format: "pdf".into(),
        layout: "pages".into(),
        title: info.title.clone(),
        authors: split(&info.author, &[';', '&']),
        language: info.language.clone(),
        publisher: String::new(),
        description: info.subject.clone(),
        subjects: split(&info.keywords, &[',', ';']),
        published: if info.year > 0 { info.year.to_string() } else { String::new() },
        direction: "ltr".into(),
        chapters: Vec::new(),
        toc,
        pages: doc.page_count() as i64,
        count_rule: String::new(),
        warnings,
    };
    Book { manifest, content: Content::Pdf(Arc::new(doc)), chapters: HashMap::new(), bytes }
}

fn outline_entries(items: &[lain_pdf::OutlineItem], depth: usize) -> Vec<BookTocEntry> {
    if depth > 8 {
        return Vec::new();
    }
    items
        .iter()
        .map(|o| BookTocEntry {
            title: o.title.clone(),
            chapter: -1,
            anchor: String::new(),
            page: o.page.map_or(-1, |p| p as i64),
            children: outline_entries(&o.children, depth + 1),
        })
        .collect()
}

/// Whether the first pages use a font that is not embedded.
fn pdf_needs_fonts(doc: &lain_pdf::Document) -> bool {
    use lain_pdf::object::Object;
    for i in 0..doc.page_count().min(5) {
        let Ok(p) = doc.page(i) else { continue };
        let Some(res) = &p.resources else { continue };
        let Some(fonts) = doc.dict_in(res, "Font") else { continue };
        for (_, f) in &fonts.entries {
            let Object::Dict(fd) = doc.resolve(f) else { continue };
            if fd.name("Subtype") == Some("Type3") {
                continue;
            }
            let desc = if fd.name("Subtype") == Some("Type0") {
                doc.array_in(&fd, "DescendantFonts")
                    .first()
                    .and_then(|d| d.as_dict().and_then(|d| doc.dict_in(d, "FontDescriptor")))
            } else {
                doc.dict_in(&fd, "FontDescriptor")
            };
            let embedded = desc.is_some_and(|d| ["FontFile", "FontFile2", "FontFile3"].iter().any(|k| d.contains(k)));
            if !embedded {
                return true;
            }
        }
    }
    false
}

// ---- providers ----------------------------------------------------------------

#[derive(Deserialize, Default)]
struct PathInput {
    #[serde(default)]
    path: String,
}

/// `lain.book.manifest@1`: the manifest of the book at `path`.
pub struct BookProvider {
    pub books: Arc<Books>,
}

impl Provider for BookProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn capabilities(&self) -> Vec<String> {
        vec![CAP_BOOK_MANIFEST.into()]
    }

    fn health(&self) -> Result<(), Error> {
        Ok(())
    }

    fn invoke(&self, cap: &str, _op: &str, input: Value) -> Result<Value, Error> {
        if cap != CAP_BOOK_MANIFEST {
            return Err(Error::invalid(format!("unsupported cap {cap}")));
        }
        let i: PathInput = serde_json::from_value(input).map_err(|_| Error::invalid("PathInput required"))?;
        if i.path.is_empty() {
            return Err(Error::invalid("path required"));
        }
        Ok(json!(self.books.manifest(Path::new(&i.path))?))
    }
}

/// `lain-metadata-book`: a book's embedded metadata for the enrich pass
/// (`lain.metadata.search@1`/`resolve@1`), local like `.nfo` files.
pub struct BookMetadata {
    pub books: Arc<Books>,
}

#[derive(Deserialize, Default)]
struct SearchInput {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    path: String,
}

#[derive(Deserialize, Default)]
struct ResolveInput {
    #[serde(default)]
    remote_id: String,
}

const CAP_SEARCH: &str = "lain.metadata.search@1";
const CAP_RESOLVE: &str = "lain.metadata.resolve@1";

impl BookMetadata {
    fn record(&self, path: &str) -> Result<Value, Error> {
        let m = self.books.manifest(Path::new(path))?;
        let year =
            m.published.get(..4).and_then(|y| y.parse::<i64>().ok()).filter(|y| (1000..=2999).contains(y)).unwrap_or(0);
        let description = m.description.clone();
        Ok(json!({
            "provider": METADATA_ID,
            "remote_id": path,
            "title": m.title,
            "year": year,
            "genres": m.subjects,
            "synopsis": description,
        }))
    }
}

impl Provider for BookMetadata {
    fn id(&self) -> &str {
        METADATA_ID
    }

    fn capabilities(&self) -> Vec<String> {
        vec![CAP_SEARCH.into(), CAP_RESOLVE.into()]
    }

    fn health(&self) -> Result<(), Error> {
        Ok(())
    }

    fn invoke(&self, cap: &str, _op: &str, input: Value) -> Result<Value, Error> {
        match cap {
            CAP_SEARCH => {
                let i: SearchInput = serde_json::from_value(input).unwrap_or_default();
                // Only books, only by their own file; a book with no
                // embedded title is "no match".
                if i.kind != KIND_BOOK || i.path.is_empty() || format(Path::new(&i.path)).is_empty() {
                    return Ok(json!([]));
                }
                let Ok(m) = self.books.manifest(Path::new(&i.path)) else { return Ok(json!([])) };
                if m.title.trim().is_empty() {
                    return Ok(json!([]));
                }
                Ok(json!([{ "provider": METADATA_ID, "remote_id": i.path, "title": m.title, "kind": KIND_BOOK }]))
            }
            CAP_RESOLVE => {
                let i: ResolveInput = serde_json::from_value(input).unwrap_or_default();
                if format(Path::new(&i.remote_id)).is_empty() {
                    return Err(Error::not_found("not a book"));
                }
                self.record(&i.remote_id)
            }
            _ => Err(Error::invalid(format!("unsupported cap {cap}"))),
        }
    }
}

#[cfg(test)]
mod tests;
