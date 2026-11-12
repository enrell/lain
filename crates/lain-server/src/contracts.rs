//! The server-side JSON shapes: paging input and the catalog export.
//! The response types clients read live in `lain-api` (D-179), shared
//! with `lain-client`, and are re-exported here.

use std::collections::BTreeMap;

pub use lain_api::{CatalogItem, CatalogPage, Library, Progress};
use serde::{Deserialize, Serialize};

/// Validated paging input. Go's struct has no JSON tags, so its wire
/// names are the Go field names.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageParams {
    #[serde(rename = "Limit", default)]
    pub limit: i64,
    #[serde(rename = "Offset", default)]
    pub offset: i64,
    #[serde(rename = "Sort", default)]
    pub sort: String,
    #[serde(rename = "LibraryID", default)]
    pub library_id: String,
    /// Libraries the caller may not see (D-156): filtered before paging,
    /// so totals do not count them. Rust-only field.
    #[serde(rename = "ExcludeLibraries", default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_libraries: Vec<String>,
    /// `title` (D-283): the page counts titles, not files, and holds every
    /// file of each title on it. Rust-only field.
    #[serde(rename = "Group", default, skip_serializing_if = "String::is_empty")]
    pub group: String,
}

/// The one `group` a catalog page knows (D-283).
pub const GROUP_TITLE: &str = "title";

impl PageParams {
    /// Clamps raw query input like `contracts.NormalizePage`.
    pub fn normalize(limit: i64, offset: i64, sort: &str) -> PageParams {
        let limit = if limit <= 0 { 50 } else { limit.min(500) };
        PageParams {
            limit,
            offset: offset.max(0),
            sort: if sort == "recent" { "recent".into() } else { "title".into() },
            library_id: String::new(),
            exclude_libraries: Vec::new(),
            group: String::new(),
        }
    }

    /// The `group` query value a page honours; anything else pages files.
    pub fn grouping(raw: &str) -> String {
        if raw.trim() == GROUP_TITLE { GROUP_TITLE.into() } else { String::new() }
    }
}

/// `lain.catalog-export@3` (D-175, D-177): the portable catalog
/// document. Items carry aliases and `root` + `rel_path`; `roots` names
/// the roots they use. It is the only version lain-rs reads or writes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExportDoc {
    pub format: String,
    pub version: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots: Vec<String>,
    #[serde(default)]
    pub items: BTreeMap<String, CatalogItem>,
}

pub const EXPORT_FORMAT: &str = "lain.catalog-export";

/// The one export version (numbered after Go's @1 and @2, so the two
/// servers never write different documents under one number).
pub const EXPORT_VERSION: i64 = 3;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paging_is_clamped() {
        assert_eq!(
            PageParams::normalize(900, -5, "zzz"),
            PageParams { limit: 500, offset: 0, sort: "title".into(), ..PageParams::default() }
        );
        assert_eq!(PageParams::normalize(0, 3, "recent").limit, 50);
    }
}
