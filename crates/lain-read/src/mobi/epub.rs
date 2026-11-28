//! The EPUB 3 writer (with an NCX, so EPUB 2 readers work too).

use super::Metadata;
use super::html::escape;
use crate::zip::Writer;

/// A non-document file of the book (image, font, stylesheet, SVG).
pub(crate) struct Item {
    /// Relative to OEBPS.
    pub path: String,
    pub mime: String,
    pub data: Vec<u8>,
    pub deflate: bool,
    pub cover: bool,
}

impl Item {
    pub fn new(path: &str, mime: &str, data: Vec<u8>, deflate: bool) -> Item {
        Item { path: path.to_string(), mime: mime.to_string(), data, deflate, cover: false }
    }
}

/// A content document (XHTML body plus head extras).
pub(crate) struct Doc {
    /// Relative to OEBPS (always at its top level).
    pub path: String,
    pub title: Option<String>,
    pub head: String,
    pub body_attrs: String,
    pub body: String,
    pub svg: bool,
}

pub(crate) struct SpineItem {
    pub doc: usize,
    pub props: Option<&'static str>,
}

pub(crate) struct TocEntry {
    pub label: String,
    /// Relative to OEBPS.
    pub href: String,
    pub children: Vec<TocEntry>,
}

pub(crate) struct Book {
    pub docs: Vec<Doc>,
    pub spine: Vec<SpineItem>,
    pub items: Vec<Item>,
    pub toc: Vec<TocEntry>,
}

/// Nesting deeper than this is flattened in the navigation.
const MAX_NAV_DEPTH: usize = 8;

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    escape(&mut out, s, true);
    out
}

/// The `lang`/`xml:lang` attributes for `meta.language`, if known.
fn lang_attrs(meta: &Metadata) -> String {
    if meta.language.is_empty() || meta.language == "und" {
        return String::new();
    }
    let l = esc(&meta.language);
    format!(" lang=\"{l}\" xml:lang=\"{l}\"")
}

/// The XHTML file for `doc`.
pub(crate) fn xhtml(doc: &Doc, meta: &Metadata) -> String {
    let title = doc.title.as_deref().filter(|t| !t.is_empty()).unwrap_or(&meta.title);
    let mut out = String::with_capacity(doc.body.len() + doc.head.len() + 512);
    out.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n");
    out.push_str("<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\"");
    out.push_str(&lang_attrs(meta));
    out.push_str(">\n<head>\n<meta charset=\"utf-8\"/>\n<title>");
    out.push_str(&esc(title));
    out.push_str("</title>\n");
    out.push_str(&doc.head);
    out.push_str("</head>\n<body");
    out.push_str(&doc.body_attrs);
    out.push_str(">\n");
    out.push_str(&doc.body);
    out.push_str("\n</body>\n</html>\n");
    out
}

/// Writes the EPUB.
pub(crate) fn write(book: &Book, meta: &Metadata, identifier: &str, modified: &str, rtl: bool) -> Vec<u8> {
    let mut z = Writer::new();
    z.add("mimetype", b"application/epub+zip", false);
    z.add(
        "META-INF/container.xml",
        b"<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\n<rootfiles>\n<rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/>\n</rootfiles>\n</container>\n",
        true,
    );
    z.add("OEBPS/content.opf", opf(book, meta, identifier, modified, rtl).as_bytes(), true);
    z.add("OEBPS/nav.xhtml", nav(book, meta).as_bytes(), true);
    z.add("OEBPS/toc.ncx", ncx(book, meta, identifier).as_bytes(), true);
    for s in &book.spine {
        if let Some(d) = book.docs.get(s.doc) {
            z.add(&format!("OEBPS/{}", d.path), xhtml(d, meta).as_bytes(), true);
        }
    }
    for i in &book.items {
        z.add(&format!("OEBPS/{}", i.path), &i.data, i.deflate);
    }
    z.finish()
}

fn opf(book: &Book, meta: &Metadata, identifier: &str, modified: &str, rtl: bool) -> String {
    let mut o = String::new();
    o.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    o.push_str("<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"bookid\"");
    if !meta.language.is_empty() {
        o.push_str(&format!(" xml:lang=\"{}\"", esc(&meta.language)));
    }
    o.push_str(
        ">\n<metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:opf=\"http://www.idpf.org/2007/opf\">\n",
    );
    o.push_str(&format!("<dc:identifier id=\"bookid\">{}</dc:identifier>\n", esc(identifier)));
    o.push_str(&format!("<dc:title>{}</dc:title>\n", esc(&meta.title)));
    for (i, a) in meta.authors.iter().enumerate() {
        o.push_str(&format!("<dc:creator id=\"creator{}\">{}</dc:creator>\n", i + 1, esc(a)));
        o.push_str(&format!(
            "<meta refines=\"#creator{}\" property=\"role\" scheme=\"marc:relators\">aut</meta>\n",
            i + 1
        ));
    }
    let lang = if meta.language.is_empty() { "und" } else { &meta.language };
    o.push_str(&format!("<dc:language>{}</dc:language>\n", esc(lang)));
    if !meta.publisher.is_empty() {
        o.push_str(&format!("<dc:publisher>{}</dc:publisher>\n", esc(&meta.publisher)));
    }
    if !meta.description.is_empty() {
        o.push_str(&format!("<dc:description>{}</dc:description>\n", esc(&meta.description)));
    }
    for s in &meta.subjects {
        o.push_str(&format!("<dc:subject>{}</dc:subject>\n", esc(s)));
    }
    if let Some(d) = w3c_date(&meta.published) {
        o.push_str(&format!("<dc:date>{d}</dc:date>\n"));
    }
    o.push_str(&format!("<meta property=\"dcterms:modified\">{modified}</meta>\n"));
    if book.items.iter().any(|i| i.cover) {
        o.push_str("<meta name=\"cover\" content=\"cover-image\"/>\n");
    }
    o.push_str("</metadata>\n<manifest>\n");
    o.push_str("<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n");
    o.push_str("<item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>\n");
    for s in &book.spine {
        let Some(d) = book.docs.get(s.doc) else { continue };
        let props = if d.svg { " properties=\"svg\"" } else { "" };
        o.push_str(&format!(
            "<item id=\"doc{}\" href=\"{}\" media-type=\"application/xhtml+xml\"{props}/>\n",
            s.doc,
            esc(&d.path)
        ));
    }
    let mut n = 0;
    for i in &book.items {
        let (id, props) = if i.cover {
            ("cover-image".to_string(), " properties=\"cover-image\"")
        } else {
            n += 1;
            (format!("item{n}"), "")
        };
        o.push_str(&format!("<item id=\"{id}\" href=\"{}\" media-type=\"{}\"{props}/>\n", esc(&i.path), esc(&i.mime)));
    }
    o.push_str("</manifest>\n<spine toc=\"ncx\"");
    if rtl {
        o.push_str(" page-progression-direction=\"rtl\"");
    }
    o.push_str(">\n");
    for s in &book.spine {
        match s.props {
            Some(p) => o.push_str(&format!("<itemref idref=\"doc{}\" properties=\"{p}\"/>\n", s.doc)),
            None => o.push_str(&format!("<itemref idref=\"doc{}\"/>\n", s.doc)),
        }
    }
    o.push_str("</spine>\n</package>\n");
    o
}

/// The entries to show, with a fallback so the navigation is never empty.
fn toc_or_default<'b>(book: &'b Book, meta: &Metadata, fallback: &'b mut Vec<TocEntry>) -> &'b [TocEntry] {
    if !book.toc.is_empty() {
        return &book.toc;
    }
    if let Some(d) = book.spine.first().and_then(|s| book.docs.get(s.doc)) {
        fallback.push(TocEntry { label: meta.title.clone(), href: d.path.clone(), children: Vec::new() });
    }
    fallback
}

fn nav(book: &Book, meta: &Metadata) -> String {
    let mut o = String::new();
    o.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n");
    o.push_str("<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\"");
    o.push_str(&lang_attrs(meta));
    o.push_str(">\n<head>\n<meta charset=\"utf-8\"/>\n<title>");
    o.push_str(&esc(&meta.title));
    o.push_str("</title>\n</head>\n<body>\n<nav epub:type=\"toc\" id=\"toc\">\n<h1>");
    o.push_str(&esc(&meta.title));
    o.push_str("</h1>\n");
    let mut fallback = Vec::new();
    let toc = toc_or_default(book, meta, &mut fallback);
    nav_list(&mut o, toc, 1);
    o.push_str("</nav>\n</body>\n</html>\n");
    o
}

fn nav_list(o: &mut String, entries: &[TocEntry], depth: usize) {
    if entries.is_empty() {
        return;
    }
    o.push_str("<ol>\n");
    for e in entries {
        o.push_str(&format!("<li><a href=\"{}\">{}</a>", esc(&e.href), esc(&e.label)));
        if depth < MAX_NAV_DEPTH {
            if !e.children.is_empty() {
                o.push('\n');
            }
            nav_list(o, &e.children, depth + 1);
        }
        o.push_str("</li>\n");
    }
    o.push_str("</ol>\n");
}

fn ncx(book: &Book, meta: &Metadata, identifier: &str) -> String {
    let mut fallback = Vec::new();
    let toc = toc_or_default(book, meta, &mut fallback);
    let mut points = String::new();
    let mut order = 0usize;
    let mut depth = 1usize;
    ncx_points(&mut points, toc, 1, &mut order, &mut depth);
    let mut o = String::new();
    o.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    o.push_str("<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\"");
    if !meta.language.is_empty() {
        o.push_str(&format!(" xml:lang=\"{}\"", esc(&meta.language)));
    }
    o.push_str(">\n<head>\n");
    o.push_str(&format!("<meta name=\"dtb:uid\" content=\"{}\"/>\n", esc(identifier)));
    o.push_str(&format!("<meta name=\"dtb:depth\" content=\"{depth}\"/>\n"));
    o.push_str("<meta name=\"dtb:totalPageCount\" content=\"0\"/>\n<meta name=\"dtb:maxPageNumber\" content=\"0\"/>\n");
    o.push_str("</head>\n<docTitle><text>");
    o.push_str(&esc(&meta.title));
    o.push_str("</text></docTitle>\n<navMap>\n");
    o.push_str(&points);
    o.push_str("</navMap>\n</ncx>\n");
    o
}

fn ncx_points(o: &mut String, entries: &[TocEntry], level: usize, order: &mut usize, depth: &mut usize) {
    *depth = (*depth).max(level);
    for e in entries {
        *order += 1;
        o.push_str(&format!(
            "<navPoint id=\"np{0}\" playOrder=\"{0}\"><navLabel><text>{1}</text></navLabel><content src=\"{2}\"/>\n",
            *order,
            esc(&e.label),
            esc(&e.href)
        ));
        if level < MAX_NAV_DEPTH {
            ncx_points(o, &e.children, level + 1, order, depth);
        }
        o.push_str("</navPoint>\n");
    }
}

/// The W3C date (`YYYY`, `YYYY-MM` or `YYYY-MM-DD`) at the start of `s`.
pub(crate) fn w3c_date(s: &str) -> Option<String> {
    let b = s.trim().as_bytes();
    let digits = |r: std::ops::Range<usize>| b.get(r.clone()).filter(|d| d.iter().all(u8::is_ascii_digit)).map(|_| r);
    let year = digits(0..4)?;
    let mut out = String::from_utf8_lossy(&b[year]).into_owned();
    if b.get(4) == Some(&b'-')
        && let Some(m) = digits(5..7)
    {
        let month: u32 = String::from_utf8_lossy(&b[m.clone()]).parse().ok()?;
        if !(1..=12).contains(&month) {
            return Some(out);
        }
        out.push('-');
        out.push_str(&String::from_utf8_lossy(&b[m]));
        if b.get(7) == Some(&b'-')
            && let Some(d) = digits(8..10)
        {
            let day: u32 = String::from_utf8_lossy(&b[d.clone()]).parse().ok()?;
            if (1..=31).contains(&day) {
                out.push('-');
                out.push_str(&String::from_utf8_lossy(&b[d]));
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(w3c_date("2001-02-03T00:00:00+00:00").as_deref(), Some("2001-02-03"));
        assert_eq!(w3c_date("1999").as_deref(), Some("1999"));
        assert_eq!(w3c_date("1999-13-01").as_deref(), Some("1999"));
        assert_eq!(w3c_date("May 1999"), None);
        assert_eq!(w3c_date(""), None);
    }
}
