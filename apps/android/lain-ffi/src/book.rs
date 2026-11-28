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

use lain_api::{BOOK_PAGE_CHARS, BookManifest, BookTocEntry};
use lain_client::http::encode;

use crate::LainCore;
use crate::error::LainError;
use crate::types::{BookInfo, BookPlace, BookProgress, BookTocItem};

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

/// A chapter made ready for a WebView loaded with the server as its base
/// URL: resource URLs (`/api/items/{id}/book/res/…`, as the server writes
/// them at the start of an attribute value or a CSS `url()`) become
/// absolute and carry `?token=`, and links to other chapters
/// (`href="#a" data-lain-chapter="n"`) become `lain-chapter://n#a`.
/// Same-chapter `#anchor` links and external links are left alone.
pub fn prepare_chapter(html: &str, server: &str, item_id: &str, token: &str) -> String {
    let server = server.trim_end_matches('/');
    let prefix = format!("/api/items/{item_id}/book/res/");
    let token = encode(token);
    let mut out = String::with_capacity(html.len() + 256);
    let mut rest = html;
    while let Some(at) = rest.find(&prefix) {
        let (head, tail) = rest.split_at(at);
        // Only a URL that starts an attribute value or a url(): never one
        // inside another site's link that happens to hold the same path.
        let starts = head.trim_end().as_bytes().last().is_some_and(|b| matches!(b, b'"' | b'\'' | b'('));
        let len = tail.bytes().position(url_end).unwrap_or(tail.len());
        out.push_str(head);
        if starts {
            out.push_str(server);
            out.push_str(&tail[..len]);
            out.push_str("?token=");
            out.push_str(&token);
        } else {
            out.push_str(&tail[..len]);
        }
        rest = &tail[len..];
    }
    out.push_str(rest);
    chapter_links(&out)
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

#[uniffi::export]
impl LainCore {
    /// A book's manifest (D-306). A comic or manga answers an
    /// `Api` error (`not-a-book`).
    pub async fn book(&self, item_id: String) -> Result<BookInfo, LainError> {
        let b = self.backend()?;
        self.run(async move {
            let m = b.client.book(&item_id).await?;
            Ok(info(item_id, m))
        })
        .await
    }

    /// Chapter `n` (0-based, reading order) of a reflowable book as HTML
    /// for a WebView whose base URL is the server ([`prepare_chapter`]).
    pub async fn book_chapter(&self, item_id: String, n: i64) -> Result<String, LainError> {
        let b = self.backend()?;
        self.run(async move {
            if n < 0 {
                return Err(LainError::Invalid { detail: "bad chapter".into() });
            }
            let token = b.client.session().map(|s| s.token).ok_or(LainError::Unauthorized)?;
            let server = b.server();
            let html = b.client.book_chapter(&item_id, n).await?;
            Ok(prepare_chapter(&html, &server, &item_id, &token))
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
}
