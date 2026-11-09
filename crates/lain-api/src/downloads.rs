//! `/api/downloads*`: the server download manager's records (Go
//! `internal/downloads`).

use serde::{Deserialize, Serialize};

fn is_false(v: &bool) -> bool {
    !*v
}

fn is_zero_i64(v: &i64) -> bool {
    *v == 0
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

/// A job's lifecycle position: `queued`, `running`, `paused`, `done`,
/// `failed` or `canceled`.
pub const JOB_QUEUED: &str = "queued";
pub const JOB_RUNNING: &str = "running";
pub const JOB_PAUSED: &str = "paused";
pub const JOB_DONE: &str = "done";
pub const JOB_FAILED: &str = "failed";
pub const JOB_CANCELED: &str = "canceled";

/// One server-side download. `path` is where the finished file lives.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub library_id: String,
    pub dir: String,
    pub name: String,
    /// The origin may still name it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub name_auto: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
    pub state: String,
    pub bytes: i64,
    /// -1 or 0 while unknown.
    pub total: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub validator: String,
    /// The origin serves byte ranges.
    #[serde(default, skip_serializing_if = "is_false")]
    pub ranges: bool,
    /// Consecutive transient failures; `retry_at` (unix) is when a
    /// queued job that failed transiently may run again.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub attempts: u32,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub retry_at: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub code: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub created_by: String,
    /// Insertion order: the queue is FIFO.
    pub seq: u64,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub finished_at: i64,
}

/// The disk budget a download store may use. 0 disables a bound.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    #[serde(default)]
    pub max_bytes: i64,
    #[serde(default)]
    pub min_free_bytes: i64,
}

/// The operator's download policy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadSettings {
    #[serde(default)]
    pub dir: String,
    #[serde(flatten)]
    pub limits: Limits,
    #[serde(default)]
    pub concurrency: i64,
    #[serde(default)]
    pub keep_finished_days: i64,
    #[serde(default)]
    pub max_retries: i64,
}

/// The store's footprint against its limits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadUsage {
    pub used_bytes: i64,
    pub max_bytes: i64,
    /// -1 when unknown.
    pub free_bytes: i64,
    pub min_free_bytes: i64,
}

/// What a cleanup pass removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupReport {
    pub parts: i64,
    pub records: i64,
    pub freed_bytes: i64,
}

/// `GET /api/downloads`: the queue page in one read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadsView {
    pub jobs: Vec<Job>,
    pub usage: DownloadUsage,
    pub settings: DownloadSettings,
}

/// `POST /api/downloads`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadAdd {
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub library_id: String,
}
