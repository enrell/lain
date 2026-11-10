//! The reader's data (D-248): a comic's pages from this device's
//! offline copy when it has one (CBZ, through the read module's in-tree
//! ZIP reader), else from the server (`/api/items/{id}/pages`). CBR and
//! CB7 copies need external tools the device does not have, so they
//! read from the server only.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use lain_read::{ComicArchive, PageIndex, archive};

use crate::LainCore;
use crate::core::Backend;
use crate::error::LainError;
use crate::types::{ComicBook, PageInfo};

/// Indexed local copies, by item.
#[derive(Default)]
pub(crate) struct Books {
    archive: ComicArchive,
    open: Mutex<HashMap<String, (PathBuf, PageIndex)>>,
}

impl Books {
    pub(crate) fn clear(&self) {
        self.open.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    pub(crate) fn forget(&self, item_id: &str) {
        self.open.lock().unwrap_or_else(|e| e.into_inner()).remove(item_id);
    }

    fn index(&self, item_id: &str, path: &Path) -> Result<PageIndex, LainError> {
        let idx = self.archive.index(path)?;
        self.open
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(item_id.to_string(), (path.to_path_buf(), idx.clone()));
        Ok(idx)
    }

    fn page(&self, item_id: &str, index: i64) -> Option<Result<Vec<u8>, LainError>> {
        let g = self.open.lock().unwrap_or_else(|e| e.into_inner());
        let (path, idx) = g.get(item_id)?;
        let Some(p) = idx.pages.iter().find(|p| p.index == index) else {
            return Some(Err(LainError::NotFound { detail: "no such page".into() }));
        };
        let (path, name) = (path.clone(), p.name.clone());
        drop(g);
        Some(archive::open_page(&path, &name).map_err(Into::into))
    }
}

/// The local CBZ copy of an item, if this device has one.
fn local_cbz(b: &Backend, item_id: &str) -> Result<Option<PathBuf>, LainError> {
    Ok(b.offline.local_path(&b.server(), item_id)?.map(PathBuf::from).filter(|p| archive::format(p) == "cbz"))
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
            if let Some(r) = books.page(&item_id, index) {
                return r;
            }
            Ok(b.client.comic_page(&item_id, index).await?)
        })
        .await
    }
}
