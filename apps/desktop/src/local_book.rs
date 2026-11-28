//! Books read from this device's offline copy, with no server (D-306):
//! the read module's own book reader (`lain_read::book`, the code the
//! server answers with) opens the downloaded file, so a local chapter,
//! resource or PDF page is the same bytes the server would send. The
//! readers ask [`path`] first and fall back to the client when the item
//! has no copy here.

use std::path::{Path, PathBuf};

use lain_client::ClientError;
use lain_client::types::{BookManifest, ComicPage, ComicPages};
use lain_read::book::BookError;

use crate::backend::Backend;

/// Width a local PDF page is drawn at: sharp on a large window, and one
/// size for every page so the book module's page cache answers a page
/// seen again.
pub const PDF_WIDTH: usize = 1536;

/// The finished offline copy of an item in a book format, if this
/// device has one.
pub fn path(b: &Backend, item_id: &str) -> Option<PathBuf> {
    let o = b.offline.as_ref()?;
    let p = PathBuf::from(o.local_path(&b.client.server(), item_id).ok().flatten()?);
    (!lain_read::book::format(&p).is_empty()).then_some(p)
}

/// Whether `path` is a PDF (read by pages, not chapters).
pub fn is_pdf(path: &Path) -> bool {
    lain_read::book::format(path) == "pdf"
}

/// The resource prefix the server writes, so `crate::book::parse` finds
/// the same member paths in a local chapter as in a served one.
fn res_prefix(item_id: &str) -> String {
    format!("/api/items/{item_id}/book/res/")
}

/// A book error in the client's terms, so the screens report it like
/// any other failed load.
fn err(e: BookError) -> ClientError {
    match e.code {
        "not-found" => ClientError::NotFound { message: e.message },
        lain_read::book::UNSUPPORTED_BOOK => ClientError::Unsupported { message: e.message },
        code => ClientError::Api { status: 0, code: code.into(), message: e.message },
    }
}

pub fn manifest(path: &Path) -> Result<BookManifest, ClientError> {
    lain_read::book::shared().manifest(path).map_err(err)
}

/// Chapter `index` (the manifest's chapter index, its reading order)
/// sanitized, resources under the server's prefix.
pub fn chapter(path: &Path, item_id: &str, index: i64) -> Result<String, ClientError> {
    let n = usize::try_from(index).map_err(|_| ClientError::Invalid { message: "bad chapter".into() })?;
    lain_read::book::shared().chapter(path, n, &res_prefix(item_id)).map_err(err)
}

/// A member's bytes (an image of a chapter).
pub fn resource(path: &Path, item_id: &str, member: &str) -> Result<Vec<u8>, ClientError> {
    lain_read::book::shared().resource(path, member, &res_prefix(item_id)).map(|(b, _)| b).map_err(err)
}

/// A PDF's pages as the server lists them (sizes in points: the page
/// reader only needs each page's shape).
pub fn pdf_pages(path: &Path) -> Result<ComicPages, ClientError> {
    let sizes = lain_read::book::shared().page_sizes(path).map_err(err)?;
    Ok(ComicPages {
        kind: lain_read::book::KIND_BOOK.into(),
        format: "pdf".into(),
        direction: "ltr".into(),
        pages: sizes
            .iter()
            .enumerate()
            .map(|(i, (w, h))| ComicPage {
                index: i as i64,
                mime: "image/png".into(),
                size: 0,
                width: w.round().max(1.0) as i64,
                height: h.round().max(1.0) as i64,
            })
            .collect(),
        info: None,
    })
}

/// PDF page `index` drawn [`PDF_WIDTH`] pixels wide, as an image file.
pub fn pdf_page(path: &Path, index: i64) -> Result<Vec<u8>, ClientError> {
    let n = usize::try_from(index).map_err(|_| ClientError::NotFound { message: "no such page".into() })?;
    Ok(lain_read::book::shared().page(path, n, PDF_WIDTH).map_err(err)?.0.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/fixtures/read/Read").join(name)
    }

    #[test]
    fn an_epub_reads_from_the_file() {
        let p = fixture("Tiny Book.epub");
        assert!(!is_pdf(&p));
        let m = manifest(&p).unwrap();
        assert_eq!((m.title.as_str(), m.layout.as_str(), m.chapters.len()), ("Tiny Book", "reflowable", 2));
        let html = chapter(&p, "item-read-epub", 0).unwrap();
        // The desktop's own parser finds the chapter's image as it does in
        // a served chapter, and the image loads from the same file.
        let doc = crate::book::parse(&html);
        assert!(
            doc.blocks
                .iter()
                .any(|b| matches!(&b.kind, crate::book::Kind::Image { src, .. } if src == "OEBPS/cover.png")),
            "{html}"
        );
        assert_eq!(doc.chars as i64, m.chapters[0].chars, "the same count rule as the manifest");
        let png = resource(&p, "item-read-epub", "OEBPS/cover.png").unwrap();
        assert!(png.starts_with(b"\x89PNG"));
        assert!(matches!(chapter(&p, "item-read-epub", 9), Err(ClientError::NotFound { .. })));
        assert!(matches!(chapter(&p, "item-read-epub", -1), Err(ClientError::Invalid { .. })));
        assert!(matches!(resource(&p, "item-read-epub", "../etc/passwd"), Err(ClientError::NotFound { .. })));
    }

    #[test]
    fn a_pdf_reads_by_pages_from_the_file() {
        let p = fixture("Tiny Pages.pdf");
        assert!(is_pdf(&p));
        assert_eq!(manifest(&p).unwrap().layout, "pages");
        let pages = pdf_pages(&p).unwrap();
        assert_eq!((pages.format.as_str(), pages.pages.len()), ("pdf", 2));
        assert!(pages.pages.iter().all(|pg| pg.width > 0 && pg.height > 0));
        let img = pdf_page(&p, 1).unwrap();
        assert!(image::load_from_memory(&img).is_ok_and(|i| i.width() as usize == PDF_WIDTH));
        assert!(matches!(pdf_page(&p, 2), Err(ClientError::NotFound { .. })));
        assert!(matches!(pdf_page(&p, -1), Err(ClientError::NotFound { .. })));
    }

    #[test]
    fn a_broken_file_is_an_error_not_a_panic() {
        assert!(manifest(&fixture("broken.epub")).is_err());
        assert!(manifest(&fixture("missing.epub")).is_err());
    }
}
