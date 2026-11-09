//! The provider contracts of `lain.metadata.search@1` and
//! `lain.metadata.resolve@1` (merge-many), the Go `contracts` shapes.

use serde::{Deserialize, Serialize};

fn is_zero(v: &i64) -> bool {
    *v == 0
}

fn is_zero_f(v: &f64) -> bool {
    *v == 0.0
}

/// One provider's search hit. `score` is set by the merge, never by a
/// provider.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MetadataCandidate {
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub remote_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub synonyms: Vec<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub year: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub poster: String,
    #[serde(default, skip_serializing_if = "is_zero_f")]
    pub score: f64,
}

/// A search. `dir` hints local sources (NFO sidecars); remote providers
/// ignore it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataSearchInput {
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub limit: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub dir: String,
}

/// A resolved entry. Artwork rides as URLs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataRecord {
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub remote_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub synonyms: Vec<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub year: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub synopsis: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub poster: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cover: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub episodes: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataResolveInput {
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub remote_id: String,
}
