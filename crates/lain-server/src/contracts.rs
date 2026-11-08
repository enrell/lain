//! The JSON shapes of `internal/contracts` the P1a routes serve. Field
//! names, `omitempty` behavior and number types follow the Go structs;
//! unknown fields of stored records are kept (`extra`) so an import
//! and P1b's additive fields round-trip.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

fn is_false(v: &bool) -> bool {
    !*v
}

/// A configured media root.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Library {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub path: String,
    pub source: String,
    pub created_at: i64,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// One catalog item: canonical identity plus provenance.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CatalogItem {
    pub id: String,
    pub library_id: String,
    pub kind: String,
    pub title: String,
    pub season: i64,
    pub episode: i64,
    pub year: i64,
    pub file_path: String,
    pub size: i64,
    pub confidence: f64,
    pub origin: String,
    pub provenance: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub missing: bool,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// The paged envelope of catalog reads (default limit 50, cap 500).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CatalogPage {
    pub items: Vec<CatalogItem>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

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
}

impl PageParams {
    /// Clamps raw query input like `contracts.NormalizePage`.
    pub fn normalize(limit: i64, offset: i64, sort: &str) -> PageParams {
        let limit = if limit <= 0 { 50 } else { limit.min(500) };
        PageParams {
            limit,
            offset: offset.max(0),
            sort: if sort == "recent" { "recent".into() } else { "title".into() },
            library_id: String::new(),
        }
    }
}

/// One user's progress on one item.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    #[serde(default)]
    pub item_id: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub position_sec: f64,
    #[serde(default)]
    pub duration_sec: f64,
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub updated_at: i64,
}

/// `lain.catalog-export`: the portable catalog document. Version 1 has
/// no aliases; version 2 adds them (identity v2). Both import.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExportDoc {
    pub format: String,
    pub version: i64,
    #[serde(default)]
    pub items: BTreeMap<String, CatalogItem>,
}

pub const EXPORT_FORMAT: &str = "lain.catalog-export";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_item_round_trips() {
        let raw = r#"{"id":"e4622164393ba829","library_id":"lib-1","kind":"episode","title":"Frieren","season":0,"episode":1,"year":0,"file_path":"/m/a.mkv","size":0,"confidence":0.8799999999999999,"origin":"lain-identify-anime","provenance":"identify:lain-identify-anime","updated_at":1794173709}"#;
        let it: CatalogItem = serde_json::from_str(raw).unwrap();
        let back = serde_json::to_value(&it).unwrap();
        assert_eq!(back, serde_json::from_str::<Value>(raw).unwrap());
        let with_future: CatalogItem =
            serde_json::from_str(&raw.replace("}", r#","root":"anime","rel_path":"a.mkv"}"#)).unwrap();
        assert_eq!(with_future.extra["root"], "anime");
        assert_eq!(serde_json::to_value(&with_future).unwrap()["rel_path"], "a.mkv");
    }

    #[test]
    fn paging_is_clamped() {
        assert_eq!(
            PageParams::normalize(900, -5, "zzz"),
            PageParams { limit: 500, offset: 0, sort: "title".into(), library_id: String::new() }
        );
        assert_eq!(PageParams::normalize(0, 3, "recent").limit, 50);
    }
}
