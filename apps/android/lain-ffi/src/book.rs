//! The native book reader's data (D-306, D-307): a reflowable book's
//! manifest, its chapters made ready for a WebView, and the mapping
//! between a place in the book and the shared progress record, the same
//! rule as the web reader's `web/src/lib/reader/book.ts`.
//!
//! A chapter comes from `GET /api/items/{id}/book/chapters/{n}` already
//! sanitized by the server (no script, nothing fetched off the server).
//! [`prepare_chapter`] then gives every resource URL the server's
//! address and the session token (a WebView sends no bearer header) and
//! turns links to other chapters into `lain-chapter://<n>#<anchor>`, so
//! the app sees them as navigations it handles itself.
//!
//! When this device holds an offline copy of the book, the manifest and
//! chapters come from the file through the read module's book reader
//! (the server's own code), with no server at all: [`inline_chapter`]
//! puts each resource in the page as a `data:` URI, since a WebView
//! offline has nothing to fetch it from.

use std::collections::HashMap;
use std::path::PathBuf;

use lain_api::{BOOK_PAGE_CHARS, BookManifest, BookTocEntry};
use lain_client::http::encode;
use lain_read::book::BookError;
use lain_read::book::markup::{self, Ev};

use crate::LainCore;
use crate::core::Backend;
use crate::error::LainError;
use crate::task::blocking;
use crate::types::{BookInfo, BookPlace, BookProgress, BookTocItem};

/// A resource larger than this stays a URL (and fails to load offline)
/// rather than bloat the chapter string handed across the bindings.
pub const MAX_INLINE: usize = 4 << 20;
/// All of a chapter's inlined resources together, at most.
pub const MAX_INLINE_TOTAL: usize = 32 << 20;

pub(crate) fn book_err(e: BookError) -> LainError {
    lain_core::Error::from(e).into()
}

/// The local copy of an item in a book format, if this device has one.
pub(crate) fn local_book(b: &Backend, item_id: &str) -> Result<Option<PathBuf>, LainError> {
    Ok(b.offline
        .local_path(&b.server(), item_id)?
        .map(PathBuf::from)
        .filter(|p| !lain_read::book::format(p).is_empty()))
}

/// The resource route's prefix, the same the server writes.
fn res_prefix(item_id: &str) -> String {
    format!("/api/items/{item_id}/book/res/")
}

/// The scheme of a link to another chapter, after [`prepare_chapter`].
pub const CHAPTER_SCHEME: &str = "lain-chapter";

fn page_chars() -> f64 {
    BOOK_PAGE_CHARS as f64
}

/// Characters before each chapter, and the total at the end.
fn offsets(chapters: &[i64]) -> Vec<f64> {
    let mut out = Vec::with_capacity(chapters.len() + 1);
    out.push(0.0);
    for c in chapters {
        let last = out[out.len() - 1];
        out.push(last + (*c).max(0) as f64);
    }
    out
}

/// Virtual pages of the whole book (at least one).
fn total_pages(chapters: &[i64]) -> f64 {
    let total = offsets(chapters).last().copied().unwrap_or(0.0);
    (total / page_chars()).ceil().max(1.0)
}

/// The progress record of a place: `position` is the 1-based virtual
/// page, fractional so the exact place survives (D-307), `duration` the
/// page count. The last 2 % of the last chapter finishes the book.
#[uniffi::export]
pub fn book_progress_at(chapters: Vec<i64>, chapter: i64, fraction: f64) -> BookProgress {
    let off = offsets(&chapters);
    let n = chapters.len() as i64;
    let ch = chapter.clamp(0, (n - 1).max(0));
    let f = if fraction.is_nan() { 0.0 } else { fraction.clamp(0.0, 1.0) };
    let len = chapters.get(ch as usize).copied().unwrap_or(0).max(0) as f64;
    let chars = off.get(ch as usize).copied().unwrap_or(0.0) + f * len;
    let pages = total_pages(&chapters);
    let position = (1.0 + chars / page_chars()).min(pages);
    BookProgress {
        position: (position * 1000.0).round() / 1000.0,
        duration: pages,
        completed: n > 0 && ch == n - 1 && f >= 0.98,
    }
}

/// Where a progress record points: the chapter holding that character,
/// and how far into it. A finished or unread book opens at the start.
#[uniffi::export]
pub fn book_position_of(chapters: Vec<i64>, position: f64, completed: bool) -> BookPlace {
    let start = BookPlace { chapter: 0, fraction: 0.0 };
    if completed || position.is_nan() || position <= 1.0 || chapters.is_empty() {
        return start;
    }
    let off = offsets(&chapters);
    let chars = (position - 1.0) * page_chars();
    for (i, c) in chapters.iter().enumerate() {
        let len = (*c).max(0) as f64;
        // A saved place right at a chapter's start, rounded on the way,
        // must not reopen at the end of the chapter before it.
        if chars < off[i] + len - 0.75 || i == chapters.len() - 1 {
            if len == 0.0 {
                continue;
            }
            return BookPlace { chapter: i as i64, fraction: ((chars - off[i]) / len).clamp(0.0, 1.0) };
        }
    }
    BookPlace { chapter: chapters.len() as i64 - 1, fraction: 1.0 }
}

fn flatten(toc: &[BookTocEntry], depth: i64, out: &mut Vec<BookTocItem>) {
    for e in toc {
        out.push(BookTocItem {
            title: e.title.clone(),
            chapter: e.chapter,
            anchor: e.anchor.clone(),
            page: e.page,
            depth,
        });
        flatten(&e.children, depth + 1, out);
    }
}

pub(crate) fn info(item_id: String, m: BookManifest) -> BookInfo {
    let mut toc = Vec::new();
    flatten(&m.toc, 0, &mut toc);
    let mut chapters: Vec<(i64, i64)> = m.chapters.iter().map(|c| (c.index, c.chars)).collect();
    chapters.sort_by_key(|c| c.0);
    BookInfo {
        item_id,
        format: m.format,
        layout: m.layout,
        title: m.title,
        authors: m.authors,
        direction: m.direction,
        pages: m.pages,
        chapters: chapters.into_iter().map(|c| c.1).collect(),
        toc,
    }
}

/// The end of a resource URL the server wrote: the path is
/// percent-encoded, so it holds none of these.
fn url_end(b: u8) -> bool {
    matches!(b, b'"' | b'\'' | b'(' | b')' | b'<' | b'>' | b'?' | b'#') || b.is_ascii_whitespace()
}

/// Every resource URL (`/api/items/{id}/book/res/…`, as the server
/// writes them at the start of an attribute value or a CSS `url()`)
/// replaced by what `to` answers for it; `None` leaves it as it is.
fn rewrite_resources(html: &str, item_id: &str, mut to: impl FnMut(&str) -> Option<String>) -> String {
    let prefix = res_prefix(item_id);
    let mut out = String::with_capacity(html.len() + 256);
    let mut rest = html;
    while let Some(at) = rest.find(&prefix) {
        let (head, tail) = rest.split_at(at);
        // Only a URL that starts an attribute value or a url(): never one
        // inside another site's link that happens to hold the same path.
        let starts = head.trim_end().as_bytes().last().is_some_and(|b| matches!(b, b'"' | b'\'' | b'('));
        let len = tail.bytes().position(url_end).unwrap_or(tail.len());
        let url = &tail[..len];
        out.push_str(head);
        match starts.then(|| to(url)).flatten() {
            Some(new) => out.push_str(&new),
            None => out.push_str(url),
        }
        rest = &tail[len..];
    }
    out.push_str(rest);
    out
}

/// A chapter made ready for a WebView loaded with the server as its base
/// URL: resource URLs become absolute and carry `?token=`, and links to
/// other chapters (`href="#a" data-lain-chapter="n"`) become
/// `lain-chapter://n#a`. Same-chapter `#anchor` links and external links
/// are left alone.
pub fn prepare_chapter(html: &str, server: &str, item_id: &str, token: &str) -> String {
    let server = server.trim_end_matches('/');
    let token = encode(token);
    chapter_links(&rewrite_resources(html, item_id, |url| Some(format!("{server}{url}?token={token}"))))
}

/// A chapter read from a local copy, made ready for a WebView with no
/// server: each resource `load` finds (by its member path) becomes a
/// `data:` URI, once per chapter however often it is used; one over
/// [`MAX_INLINE`], or past [`MAX_INLINE_TOTAL`], stays a URL that simply
/// fails to load. Chapter links as in [`prepare_chapter`].
pub fn inline_chapter(html: &str, item_id: &str, mut load: impl FnMut(&str) -> Option<(Vec<u8>, String)>) -> String {
    let prefix = res_prefix(item_id);
    let mut seen: HashMap<String, Option<String>> = HashMap::new();
    let mut total = 0usize;
    let out = rewrite_resources(html, item_id, |url| {
        if let Some(done) = seen.get(url) {
            return done.clone();
        }
        let member = markup::percent_decode(&url[prefix.len()..]);
        let uri = load(&member)
            .filter(|(data, _)| data.len() <= MAX_INLINE && total + data.len() <= MAX_INLINE_TOTAL)
            .map(|(data, mime)| {
                total += data.len();
                format!("data:{};base64,{}", data_mime(&mime), base64(&data))
            });
        seen.insert(url.to_string(), uri.clone());
        uri
    });
    chapter_links(&out)
}

/// The media type a `data:` URI may carry: documents never become pages
/// (as the server serves them, as plain text), and anything that could
/// end the attribute or the `url()` it sits in is replaced.
fn data_mime(mime: &str) -> &str {
    let m = mime.trim();
    if m.contains("html") || m.contains("xml") && !m.contains("svg") {
        return "text/plain";
    }
    if m.is_empty() || !m.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'+' | b'-' | b'.')) {
        return "application/octet-stream";
    }
    m
}

/// Standard base64 with padding (RFC 4648), for `data:` URIs; no crate
/// in the tree exposes one.
fn base64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n =
            (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// `href="#a" data-lain-chapter="n"` (the sanitizer writes the two
/// together, in this order) to `href="lain-chapter://n#a" …`.
fn chapter_links(html: &str) -> String {
    const ATTR: &str = " data-lain-chapter=\"";
    let mut out = String::with_capacity(html.len() + 64);
    let mut rest = html;
    while let Some(at) = rest.find(ATTR) {
        let (head, tail) = rest.split_at(at);
        let digits = &tail[ATTR.len()..];
        let n_len = digits.bytes().take_while(u8::is_ascii_digit).count();
        let closed = digits.as_bytes().get(n_len) == Some(&b'"');
        // The href just before: ` href="#…"` ending where this attribute starts.
        let href = head.rfind(" href=\"").filter(|&h| {
            let v = &head[h + 7..];
            v.starts_with('#') && v.ends_with('"') && !v[..v.len() - 1].contains(['"', '<', '>'])
        });
        match (href, n_len > 0 && closed) {
            (Some(h), true) => {
                let anchor = &head[h + 7..head.len() - 1];
                out.push_str(&head[..h]);
                out.push_str(" href=\"");
                out.push_str(CHAPTER_SCHEME);
                out.push_str("://");
                out.push_str(&digits[..n_len]);
                out.push_str(anchor);
                out.push('"');
            }
            _ => out.push_str(head),
        }
        out.push_str(ATTR);
        rest = &tail[ATTR.len()..];
    }
    out.push_str(rest);
    out
}

/// Links inside the chapter (`<a href="#a">`, as the sanitizer writes
/// them) to `lain-chapter://n#a`, `n` being this chapter: with
/// JavaScript off a WebView's own fragment scroll is not something the
/// app can follow or page, so every jump to an anchor goes through the
/// app, which places it by [`anchor_fraction`]. Only on `a` elements: an
/// SVG `use href="#id"` keeps pointing into the document.
pub fn local_links(html: &str, n: i64) -> String {
    const HREF: &str = " href=\"#";
    let mut out = String::with_capacity(html.len() + 64);
    let mut rest = html;
    while let Some(at) = rest.find(HREF) {
        let (head, tail) = rest.split_at(at);
        out.push_str(head);
        // The tag this attribute is in: attribute values and text have
        // their `<` escaped, so the last one starts it.
        let in_a = out.rfind('<').is_some_and(|t| {
            let tag = &out[t..];
            (tag == "<a" || tag.starts_with("<a ")) && !tag.contains('>')
        });
        if in_a {
            out.push_str(" href=\"");
            out.push_str(CHAPTER_SCHEME);
            out.push_str("://");
            out.push_str(&n.to_string());
            out.push('#');
        } else {
            out.push_str(HREF);
        }
        rest = &tail[HREF.len()..];
    }
    out.push_str(rest);
    out
}

/// How far into a chapter the element with id `anchor` starts, from 0
/// to 1: the characters of text before it over the chapter's (the
/// measure the manifest's `chars` uses, whitespace not counted). An
/// unknown or empty anchor, or a chapter with no text, is its start.
/// Approximate where images or headings are large, but deterministic,
/// and with no script: the reader scrolls or pages to this fraction.
pub fn anchor_fraction(html: &str, anchor: &str) -> f64 {
    if anchor.is_empty() {
        return 0.0;
    }
    let decoded = markup::percent_decode(anchor);
    let named = |v: Option<&str>| v.is_some_and(|v| !v.is_empty() && (v == anchor || v == decoded));
    let mut total = 0usize;
    let mut found = None;
    for e in markup::events(html.as_bytes()) {
        match &e {
            Ev::Start { name, .. } if found.is_none() => {
                if named(e.attr("id")) || (markup::local(name) == "a" && named(e.attr("name"))) {
                    found = Some(total);
                }
            }
            Ev::Text(t) => total += t.chars().filter(|c| !c.is_whitespace()).count(),
            _ => {}
        }
    }
    match found {
        Some(before) if total > 0 => (before as f64 / total as f64).clamp(0.0, 1.0),
        _ => 0.0,
    }
}

/// [`anchor_fraction`] of a chapter the app already holds (as
/// [`LainCore::book_chapter`] answered it), so a jump needs no second
/// fetch; works the same on an offline copy's chapter.
#[uniffi::export]
pub fn book_anchor_fraction_in(html: String, anchor: String) -> f64 {
    anchor_fraction(&html, &anchor)
}

/// A length in CSS pixels as the stylesheet writes it.
fn px(v: f64) -> String {
    format!("{}px", (v * 64.0).round() / 64.0)
}

/// The paged layout's rules (the web reader's `readerCss` paged mode),
/// laid over the reader's own stylesheet: the body is one page tall and
/// flows into columns one page wide, so page `k` starts `k * width` CSS
/// pixels to the right and turning a page is a horizontal scroll of the
/// WebView, with no script. `width` and `height` are the WebView's in
/// CSS pixels; the width is floored to the 1/64 px the layout engine
/// counts in, so the app's page stride and the layout's agree exactly.
/// `text_zoom` (percent) widens the margins on a wide screen so a line
/// keeps a readable measure. Books written right to left still page
/// left to right; their blocks keep their direction.
#[uniffi::export]
pub fn book_paged_css(width: f64, height: f64, text_zoom: i64) -> String {
    let sane = |v: f64, d: f64| if v.is_finite() && v > 0.0 { v } else { d };
    let w = ((sane(width, 360.0).max(120.0)) * 64.0).floor() / 64.0;
    let h = ((sane(height, 640.0).max(120.0)) * 64.0).floor() / 64.0;
    let zoom = text_zoom.clamp(50, 300) as f64;
    // About 38em at the text size: past that a line is hard to follow.
    let measure = (38.0 * 16.0 * 1.25 * zoom / 100.0).round();
    let m = 22f64.max(((w - measure) / 2.0).floor()).min((w / 4.0).floor());
    let v = m.clamp(24.0, 48.0);
    [
        format!(
            "html {{ width: {w} !important; height: {h} !important; margin: 0 !important; padding: 0 !important; \
             direction: ltr !important; }}",
            w = px(w),
            h = px(h)
        ),
        format!(
            "body {{ width: {w} !important; height: {h} !important; max-width: none !important; margin: 0 !important; \
             box-sizing: border-box !important; padding: {v} {m} !important; column-width: {cw} !important; \
             column-gap: {gap} !important; column-fill: auto !important; overflow: visible !important; \
             direction: ltr !important; }}",
            w = px(w),
            h = px(h),
            v = px(v),
            m = px(m),
            // A hair under the content width: exactly one column fills it,
            // whatever the rounding, so the stride is the page's width.
            cw = px((w - 2.0 * m - 1.0).max(1.0)),
            gap = px(2.0 * m)
        ),
        "html[dir=\"rtl\"] body > *, body[dir=\"rtl\"] > * { direction: rtl !important; }".to_string(),
        format!(
            "img, svg, video {{ max-width: 100% !important; max-height: {} !important; height: auto; \
             object-fit: contain; break-inside: avoid; }}",
            px((h - 2.0 * v).max(60.0))
        ),
        "h1, h2, h3, h4, h5, h6 { break-after: avoid; }".to_string(),
    ]
    .join("\n")
}

#[uniffi::export]
impl LainCore {
    /// A book's manifest (D-306). A comic or manga answers an
    /// `Api` error (`not-a-book`).
    /// From the offline copy when this device has one.
    pub async fn book(&self, item_id: String) -> Result<BookInfo, LainError> {
        let b = self.backend()?;
        self.run(async move {
            if let Some(path) = local_book(&b, &item_id)? {
                let m = blocking(move || lain_read::book::shared().manifest(&path).map_err(book_err)).await?;
                b.offline.touch(&b.server(), &item_id)?;
                return Ok(info(item_id, m));
            }
            let m = b.client.book(&item_id).await?;
            Ok(info(item_id, m))
        })
        .await
    }

    /// Chapter `n` (0-based, reading order) of a reflowable book as HTML
    /// for a WebView whose base URL is the server ([`prepare_chapter`]),
    /// or, from the offline copy, self-contained ([`inline_chapter`]).
    pub async fn book_chapter(&self, item_id: String, n: i64) -> Result<String, LainError> {
        let b = self.backend()?;
        self.run(async move {
            if n < 0 {
                return Err(LainError::Invalid { detail: "bad chapter".into() });
            }
            if let Some(path) = local_book(&b, &item_id)? {
                let id = item_id.clone();
                let html = blocking(move || {
                    let books = lain_read::book::shared();
                    let prefix = res_prefix(&id);
                    let html = books.chapter(&path, n as usize, &prefix).map_err(book_err)?;
                    Ok(local_links(
                        &inline_chapter(&html, &id, |member| books.resource(&path, member, &prefix).ok()),
                        n,
                    ))
                })
                .await?;
                b.offline.touch(&b.server(), &item_id)?;
                return Ok(html);
            }
            let token = b.client.session().map(|s| s.token).ok_or(LainError::Unauthorized)?;
            let server = b.server();
            let html = b.client.book_chapter(&item_id, n).await?;
            Ok(local_links(&prepare_chapter(&html, &server, &item_id, &token), n))
        })
        .await
    }

    /// How far into chapter `n` the element with id `anchor` starts
    /// ([`anchor_fraction`], 0..1), so a jump to a contents entry or a
    /// link lands there with no script. From the offline copy when this
    /// device has one.
    pub async fn book_anchor_fraction(&self, item_id: String, n: i64, anchor: String) -> Result<f64, LainError> {
        let b = self.backend()?;
        self.run(async move {
            if n < 0 {
                return Err(LainError::Invalid { detail: "bad chapter".into() });
            }
            if anchor.is_empty() {
                return Ok(0.0);
            }
            if let Some(path) = local_book(&b, &item_id)? {
                let id = item_id.clone();
                return blocking(move || {
                    let html =
                        lain_read::book::shared().chapter(&path, n as usize, &res_prefix(&id)).map_err(book_err)?;
                    Ok(anchor_fraction(&html, &anchor))
                })
                .await;
            }
            let html = b.client.book_chapter(&item_id, n).await?;
            Ok(anchor_fraction(&html, &anchor))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn progress_is_the_virtual_page() {
        // 2048 + 1024 characters: three pages.
        let ch = vec![2048, 1024];
        let p = book_progress_at(ch.clone(), 0, 0.0);
        assert_eq!((p.position, p.duration, p.completed), (1.0, 3.0, false));
        let p = book_progress_at(ch.clone(), 0, 0.5);
        assert_eq!(p.position, 2.0);
        let p = book_progress_at(ch.clone(), 1, 0.5);
        assert_eq!(p.position, 3.0, "capped at the page count");
        assert!(!p.completed);
        let p = book_progress_at(ch.clone(), 1, 0.99);
        assert!(p.completed);
        // Out-of-range input is clamped, never a panic.
        assert!(book_progress_at(ch.clone(), 9, 2.0).completed);
        assert_eq!(book_progress_at(ch.clone(), -3, f64::NAN).position, 1.0);
        let empty = book_progress_at(vec![], 0, 0.5);
        assert_eq!((empty.position, empty.duration, empty.completed), (1.0, 1.0, false));
        // Three decimals, as the web reader rounds.
        assert_eq!(book_progress_at(vec![3000], 0, 1.0 / 3.0).position, 1.977);
    }

    #[test]
    fn a_position_maps_back_to_its_place() {
        let ch = vec![2048, 0, 1024];
        assert_eq!(book_position_of(ch.clone(), 0.0, false), BookPlace { chapter: 0, fraction: 0.0 });
        assert_eq!(book_position_of(ch.clone(), 2.5, true), BookPlace { chapter: 0, fraction: 0.0 }, "finished: start");
        assert_eq!(book_position_of(vec![], 4.0, false).chapter, 0);
        let p = book_position_of(ch.clone(), 2.0, false);
        assert_eq!(p.chapter, 0);
        assert!(close(p.fraction, 0.5));
        // An empty chapter is never the place.
        let p = book_position_of(ch.clone(), 3.5, false);
        assert_eq!(p.chapter, 2);
        assert!(close(p.fraction, 0.5));
        // Past the end: the last chapter, at its end.
        let p = book_position_of(ch.clone(), 99.0, false);
        assert_eq!(p, BookPlace { chapter: 2, fraction: 1.0 });
        // A round trip lands where it started (before the last page,
        // which the page count caps, as on the web).
        let ch = vec![2048, 0, 1500];
        for (c, f) in [(0, 0.25), (2, 0.5)] {
            let saved = book_progress_at(ch.clone(), c, f);
            let back = book_position_of(ch.clone(), saved.position, saved.completed);
            assert_eq!(back.chapter, c);
            assert!((back.fraction - f).abs() < 0.01, "{back:?}");
        }
    }

    #[test]
    fn the_manifest_flattens_its_toc() {
        let m = BookManifest {
            format: "epub".into(),
            layout: "reflowable".into(),
            title: "T".into(),
            direction: "ltr".into(),
            pages: 2,
            chapters: vec![
                lain_api::BookChapter { index: 1, chars: 7, linear: true },
                lain_api::BookChapter { index: 0, chars: 900, linear: true },
            ],
            toc: vec![BookTocEntry {
                title: "One".into(),
                chapter: 0,
                page: -1,
                children: vec![BookTocEntry {
                    title: "One, b".into(),
                    chapter: 0,
                    anchor: "b".into(),
                    page: -1,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let i = info("x".into(), m);
        assert_eq!(i.chapters, vec![900, 7], "in reading order");
        assert_eq!(i.toc.iter().map(|t| (t.title.as_str(), t.depth)).collect::<Vec<_>>(), [("One", 0), ("One, b", 1)]);
        assert_eq!(i.toc[1].anchor, "b");
    }

    #[test]
    fn a_chapter_gets_absolute_resources_and_chapter_links() {
        let html = concat!(
            r##"<style>p{background:url("/api/items/I/book/res/OEBPS/a.png")}</style>"##,
            r##"<img src="/api/items/I/book/res/OEBPS/img/a%20b.png" alt="x">"##,
            r##"<a href="#sec" data-lain-chapter="12" data-lain-anchor="sec">next</a>"##,
            r##"<a href="#" data-lain-chapter="3" data-lain-anchor="">top of 3</a>"##,
            r##"<a href="#here">here</a>"##,
            r##"<a href="http://example.com/api/items/I/book/res/x.png" target="_blank">ext</a>"##,
        );
        let out = prepare_chapter(html, "http://10.0.0.2:9360/", "I", "t k+n");
        assert!(
            out.contains(r##"url("http://10.0.0.2:9360/api/items/I/book/res/OEBPS/a.png?token=t%20k%2Bn")"##),
            "{out}"
        );
        assert!(
            out.contains(
                r##"<img src="http://10.0.0.2:9360/api/items/I/book/res/OEBPS/img/a%20b.png?token=t%20k%2Bn" alt="x">"##
            ),
            "{out}"
        );
        assert!(
            out.contains(r##"<a href="lain-chapter://12#sec" data-lain-chapter="12" data-lain-anchor="sec">"##),
            "{out}"
        );
        assert!(out.contains(r##"<a href="lain-chapter://3#" data-lain-chapter="3""##), "{out}");
        assert!(out.contains(r##"<a href="#here">"##));
        assert!(
            out.contains(r##"href="http://example.com/api/items/I/book/res/x.png""##),
            "another site's link is left alone: {out}"
        );
        assert!(!out.contains("x.png?token"));
    }

    #[test]
    fn a_local_chapter_inlines_its_resources() {
        let html = concat!(
            r##"<style>p{background:url("/api/items/I/book/res/OEBPS/a.png")}</style>"##,
            r##"<img src="/api/items/I/book/res/OEBPS/a.png"><img src="/api/items/I/book/res/OEBPS/img/a%20b.svg">"##,
            r##"<img src="/api/items/I/book/res/big.png"><img src="/api/items/I/book/res/gone.png">"##,
            r##"<a href="/api/items/I/book/res/t.xhtml">t</a>"##,
            r##"<a href="#sec" data-lain-chapter="2" data-lain-anchor="sec">next</a>"##,
        );
        let mut asked = Vec::new();
        let out = inline_chapter(html, "I", |m| {
            asked.push(m.to_string());
            match m {
                "OEBPS/a.png" => Some((b"Man".to_vec(), "image/png".into())),
                "OEBPS/img/a b.svg" => Some((b"<svg/>".to_vec(), "image/svg+xml".into())),
                "big.png" => Some((vec![0; MAX_INLINE + 1], "image/png".into())),
                "t.xhtml" => Some((b"x".to_vec(), "application/xhtml+xml".into())),
                _ => None,
            }
        });
        assert_eq!(asked.iter().filter(|m| *m == "OEBPS/a.png").count(), 1, "loaded once: {asked:?}");
        assert!(out.contains(r##"url("data:image/png;base64,TWFu")"##), "{out}");
        assert!(out.contains(r##"<img src="data:image/png;base64,TWFu">"##), "{out}");
        assert!(out.contains(r##"<img src="data:image/svg+xml;base64,PHN2Zy8+">"##), "{out}");
        assert!(out.contains(r##"<img src="/api/items/I/book/res/big.png">"##), "too large: {out}");
        assert!(out.contains(r##"<img src="/api/items/I/book/res/gone.png">"##), "{out}");
        assert!(out.contains(r##"<a href="data:text/plain;base64,eA==">"##), "{out}");
        assert!(out.contains(r##"href="lain-chapter://2#sec""##), "{out}");
    }

    #[test]
    fn base64_and_mime_are_safe() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe, 0xfd]), "//79");
        assert_eq!(data_mime("image/jpeg"), "image/jpeg");
        assert_eq!(data_mime("text/html"), "text/plain");
        assert_eq!(data_mime("image/png\"onload=\"x"), "application/octet-stream");
        assert_eq!(data_mime(""), "application/octet-stream");
    }

    #[test]
    fn odd_markup_passes_through() {
        for html in [
            "",
            "plain text",
            " data-lain-chapter=\"",
            "<a data-lain-chapter=\"1\">no href</a>",
            "<a href=\"#a\" data-lain-chapter=\"x\">",
            "/api/items/I/book/res/",
            "(/api/items/I/book/res/",
        ] {
            let out = prepare_chapter(html, "http://s", "I", "t");
            if !html.contains("(/api") {
                assert_eq!(out, html);
            }
        }
        assert_eq!(
            prepare_chapter("(/api/items/I/book/res/", "http://s", "I", "t"),
            "(http://s/api/items/I/book/res/?token=t"
        );
    }

    #[test]
    fn links_inside_a_chapter_go_through_the_app() {
        let html = concat!(
            r##"<p id="a">x <a href="#fn%201" class="n">1</a> <a href="#">top</a></p>"##,
            r##"<a href="#sec" data-lain-chapter="4" data-lain-anchor="sec">next</a>"##,
            r##"<svg><use href="#glyph"/></svg><a href="http://example.com/#x">ext</a>"##,
            r##"<abbr title="a href=&quot;#q&quot;">q</abbr>"##,
        );
        let out = local_links(&prepare_chapter(html, "http://s", "I", "t"), 2);
        assert!(out.contains(r##"<a href="lain-chapter://2#fn%201" class="n">"##), "{out}");
        assert!(out.contains(r##"<a href="lain-chapter://2#">top</a>"##), "{out}");
        assert!(out.contains(r##"<a href="lain-chapter://4#sec" data-lain-chapter="4""##), "{out}");
        assert!(out.contains(r##"<use href="#glyph"/>"##), "SVG keeps its reference: {out}");
        assert!(out.contains(r##"href="http://example.com/#x""##), "{out}");
        assert!(out.contains(r##"title="a href=&quot;#q&quot;""##), "{out}");
        for odd in ["", " href=\"#", "<a href=\"#", "<b href=\"#x\">"] {
            let out = local_links(odd, 0);
            assert!(!out.contains("lain-chapter") || odd.starts_with("<a"), "{odd:?} -> {out}");
        }
    }

    #[test]
    fn an_anchor_maps_to_its_share_of_the_text() {
        // 10 characters before `mid`, 10 from it on (whitespace not counted).
        let html = concat!(
            r##"<!DOCTYPE html><html><head><meta charset="utf-8"><style>p::before{content:"xxxxxxxx"}</style></head>"##,
            r##"<body><h1 id="top">Abcde</h1><p>fg hij</p><p id="mid">Klmno <span id="deep">pqrst</span></p>"##,
            r##"<p id="end"></p></body></html>"##,
        );
        assert!(close(anchor_fraction(html, "top"), 0.0));
        assert!(close(anchor_fraction(html, "mid"), 0.5));
        assert!(close(anchor_fraction(html, "deep"), 0.75));
        assert!(close(anchor_fraction(html, "end"), 1.0));
        assert_eq!(anchor_fraction(html, "nowhere"), 0.0);
        assert_eq!(anchor_fraction(html, ""), 0.0);
        // A percent-encoded anchor (as a URL fragment carries it) and an
        // old-style `<a name>` both count; the first match wins.
        let html = r#"<p>aaaa</p><a name="n 1"></a><p id="dup">bbbb</p><p id="dup">cccc</p>"#;
        assert!(close(anchor_fraction(html, "n%201"), 1.0 / 3.0));
        assert!(close(anchor_fraction(html, "dup"), 1.0 / 3.0));
        // No text at all, or broken markup: a number, never a panic.
        assert_eq!(anchor_fraction(r#"<img id="i" src="x">"#, "i"), 0.0);
        for odd in ["<", "<p id=", "<p id=\"x", "\u{0}<p id=x>y"] {
            let f = anchor_fraction(odd, "x");
            assert!((0.0..=1.0).contains(&f), "{odd:?}: {f}");
        }
        assert_eq!(book_anchor_fraction_in(html.into(), "dup".into()), anchor_fraction(html, "dup"));
    }

    #[test]
    fn the_paged_layout_is_one_page_wide_columns() {
        // A 1080 px phone at 2.625: 411.43 CSS px, floored to 1/64 px.
        let css = book_paged_css(1080.0 / 2.625, 2000.0 / 2.625, 110);
        let w = (1080.0f64 / 2.625 * 64.0).floor() / 64.0;
        assert_eq!(w, 411.421875);
        assert!(css.contains("html { width: 411.421875px !important;"), "{css}");
        assert!(css.contains("body { width: 411.421875px !important;"), "{css}");
        // Narrow: the 22 px margin; one column a hair under the content
        // width, a gap of two margins: the stride is the page.
        assert!(css.contains("padding: 24px 22px !important"), "{css}");
        assert!(css.contains(&format!("column-width: {} !important", px(w - 44.0 - 1.0))), "{css}");
        assert!(css.contains("column-gap: 44px !important"), "{css}");
        assert!(css.contains("column-fill: auto"), "{css}");
        assert!(css.contains("height: 761.890625px !important"), "{css}");
        // A wide screen keeps a readable measure: the margins grow.
        let wide = book_paged_css(1200.0, 900.0, 100);
        assert!(wide.contains("padding: 48px 220px !important"), "{wide}");
        assert!(wide.contains("column-gap: 440px !important"), "{wide}");
        // Larger text, a longer measure; never more than half the page in margins.
        assert!(book_paged_css(1200.0, 900.0, 200).contains("padding: 24px 22px"));
        assert!(book_paged_css(4000.0, 900.0, 50).contains("padding: 48px 1000px"));
        // Nonsense sizes give a usable sheet, not NaN.
        for (a, b, z) in [(f64::NAN, -1.0, 0), (0.0, f64::INFINITY, i64::MAX), (5.0, 5.0, -9)] {
            let css = book_paged_css(a, b, z);
            assert!(!css.contains("NaN") && !css.contains("inf") && !css.contains(" -"), "{css}");
        }
    }
}
