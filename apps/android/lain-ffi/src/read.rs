//! The reader's data (D-248): a comic's pages from this device's
//! offline copy when it has one (CBZ, through the read module's in-tree
//! ZIP reader; a PDF, drawn by the book module), else from the server
//! (`/api/items/{id}/pages`). CBR and CB7 copies need external tools
//! the device does not have, so they read from the server only.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use lain_read::{ComicArchive, PageIndex, archive};

use crate::LainCore;
use crate::book::{book_err, local_book};
use crate::core::Backend;
use crate::error::LainError;
use crate::task::blocking;
use crate::types::{ComicBook, PageInfo};

/// Width of a PDF page drawn from a local copy: sharp on a phone or
/// tablet held upright, and the same for every page so the book
/// module's page cache answers a page seen again.
pub const LOCAL_PDF_WIDTH: usize = 1536;

/// A local copy the page reader opened.
#[derive(Clone)]
enum Local {
    Cbz(PathBuf, Arc<PageIndex>),
    Pdf(PathBuf),
}

/// Indexed local copies, by item.
#[derive(Default)]
pub(crate) struct Books {
    archive: ComicArchive,
    open: Mutex<HashMap<String, Local>>,
}

impl Books {
    pub(crate) fn clear(&self) {
        self.open.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    pub(crate) fn forget(&self, item_id: &str) {
        self.open.lock().unwrap_or_else(|e| e.into_inner()).remove(item_id);
    }

    fn remember(&self, item_id: &str, local: Local) {
        self.open.lock().unwrap_or_else(|e| e.into_inner()).insert(item_id.to_string(), local);
    }

    fn index(&self, item_id: &str, path: &Path) -> Result<PageIndex, LainError> {
        let idx = self.archive.index(path)?;
        self.remember(item_id, Local::Cbz(path.to_path_buf(), Arc::new(idx.clone())));
        Ok(idx)
    }

    /// The page from the local copy [`LainCore::comic`] opened, `None`
    /// when the pages come from the server.
    async fn page(&self, item_id: &str, index: i64) -> Option<Result<Vec<u8>, LainError>> {
        let local = self.open.lock().unwrap_or_else(|e| e.into_inner()).get(item_id).cloned()?;
        let no_page = || LainError::NotFound { detail: "no such page".into() };
        Some(match local {
            Local::Cbz(path, idx) => match idx.pages.iter().find(|p| p.index == index) {
                Some(p) => archive::open_page(&path, &p.name).map_err(Into::into),
                None => Err(no_page()),
            },
            Local::Pdf(path) => match usize::try_from(index) {
                // Drawing a page is seconds of CPU at worst: off the
                // runtime's workers, which also carry sync and downloads.
                Ok(n) => {
                    blocking(move || {
                        let p = lain_read::book::shared().page(&path, n, LOCAL_PDF_WIDTH).map_err(book_err)?;
                        Ok(p.0.clone())
                    })
                    .await
                }
                Err(_) => Err(no_page()),
            },
        })
    }
}

/// The local CBZ copy of an item, if this device has one.
fn local_cbz(b: &Backend, item_id: &str) -> Result<Option<PathBuf>, LainError> {
    Ok(b.offline.local_path(&b.server(), item_id)?.map(PathBuf::from).filter(|p| archive::format(p) == "cbz"))
}

/// The local PDF copy of an item, if this device has one.
fn local_pdf(b: &Backend, item_id: &str) -> Result<Option<PathBuf>, LainError> {
    Ok(local_book(b, item_id)?.filter(|p| lain_read::book::format(p) == "pdf"))
}

async fn book(b: Arc<Backend>, books: Arc<Books>, item_id: String) -> Result<ComicBook, LainError> {
    if let Some(path) = local_cbz(&b, &item_id)? {
        let kind = b.offline.get(&b.server(), &item_id)?.map(|e| e.kind).unwrap_or_default();
        let idx = books.index(&item_id, &path)?;
        b.offline.touch(&b.server(), &item_id)?;
        return Ok(ComicBook {
            direction: lain_read::default_direction(&kind, idx.info.as_ref()),
            pages: idx
                .pages
                .iter()
                .map(|p| PageInfo {
                    index: p.index,
                    width: i64::from(p.width),
                    height: i64::from(p.height),
                    mime: p.mime.clone(),
                })
                .collect(),
            item_id,
            offline: true,
        });
    }
    if let Some(path) = local_pdf(&b, &item_id)? {
        let p = path.clone();
        let sizes = blocking(move || lain_read::book::shared().page_sizes(&p).map_err(book_err)).await?;
        books.remember(&item_id, Local::Pdf(path));
        b.offline.touch(&b.server(), &item_id)?;
        // Sizes in points, as the server answers them: the reader only
        // needs each page's shape.
        return Ok(ComicBook {
            direction: "ltr".into(),
            pages: sizes
                .iter()
                .enumerate()
                .map(|(i, (w, h))| PageInfo {
                    index: i as i64,
                    width: w.round().max(1.0) as i64,
                    height: h.round().max(1.0) as i64,
                    mime: "image/png".into(),
                })
                .collect(),
            item_id,
            offline: true,
        });
    }
    // Pages come from the server now: a local copy opened before (since
    // removed) must not answer them.
    books.forget(&item_id);
    let pages = b.client.comic_pages(&item_id).await?;
    Ok(ComicBook {
        direction: pages.direction,
        pages: pages
            .pages
            .into_iter()
            .map(|p| PageInfo { index: p.index, width: p.width, height: p.height, mime: p.mime })
            .collect(),
        item_id,
        offline: false,
    })
}

#[uniffi::export]
impl LainCore {
    /// A comic or manga's page list (offline copy first).
    pub async fn comic(&self, item_id: String) -> Result<ComicBook, LainError> {
        let b = self.backend()?;
        let books = self.books.clone();
        self.run(book(b, books, item_id)).await
    }

    /// One page's image bytes, by its `index`. Call [`LainCore::comic`]
    /// first: it decides whether pages come from the device.
    pub async fn comic_page(&self, item_id: String, index: i64) -> Result<Vec<u8>, LainError> {
        let b = self.backend()?;
        let books = self.books.clone();
        self.run(async move {
            if let Some(r) = books.page(&item_id, index).await {
                return r;
            }
            Ok(b.client.comic_page(&item_id, index).await?)
        })
        .await
    }
}
