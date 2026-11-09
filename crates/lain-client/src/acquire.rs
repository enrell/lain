//! The acquire module and the download manager (`/api/acquire/*`,
//! `/api/downloads*`), all admin-only on the server.

use hyper::Method;
use serde::Deserialize;
use serde_json::{Value, json};

use lain_api::Release;
use lain_api::acquire::*;
use lain_api::downloads::{CleanupReport, DownloadAdd, DownloadSettings, DownloadsView, Job};

use crate::api::{Body, Client, json_body, query};
use crate::error::ClientError;
use crate::http::encode;

/// A manual indexer search (`GET /api/acquire/search`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReleaseSearch {
    pub query: String,
    /// The library type: anime, series, movie, manga, comic.
    pub kind: String,
    pub season: i64,
    pub episode: i64,
    pub year: i64,
    /// Only these indexers; empty asks every enabled one.
    pub indexers: Vec<String>,
}

#[derive(Deserialize)]
struct Written {
    written: i64,
}

#[derive(Deserialize)]
struct Cleared {
    cleared: i64,
}

#[derive(Deserialize)]
struct Files {
    files: Vec<FileSubtitles>,
}

fn n(v: i64) -> String {
    if v == 0 { String::new() } else { v.to_string() }
}

impl Client {
    // ---- settings and indexers --------------------------------------

    pub async fn acquire_settings(&self) -> Result<AcquireView, ClientError> {
        self.call(Method::GET, "/api/acquire/settings", Body::None).await
    }

    /// Applies the fields of `patch` (a JSON object) over the current
    /// policy, like the server's own merge.
    pub async fn update_acquire_settings(&self, patch: Value) -> Result<AcquireView, ClientError> {
        self.call(Method::PUT, "/api/acquire/settings", json_body(&patch)).await
    }

    pub async fn indexers(&self) -> Result<Vec<IndexerView>, ClientError> {
        Ok(self.call::<IndexerList>(Method::GET, "/api/acquire/indexers", Body::None).await?.indexers)
    }

    pub async fn create_indexer(&self, input: IndexerInput) -> Result<IndexerView, ClientError> {
        self.call(Method::POST, "/api/acquire/indexers", json_body(&input)).await
    }

    pub async fn update_indexer(&self, id: &str, input: IndexerInput) -> Result<IndexerView, ClientError> {
        self.call(Method::PATCH, &format!("/api/acquire/indexers/{}", encode(id)), json_body(&input)).await
    }

    pub async fn delete_indexer(&self, id: &str) -> Result<(), ClientError> {
        self.call::<Removed>(Method::DELETE, &format!("/api/acquire/indexers/{}", encode(id)), Body::None)
            .await
            .map(|_| ())
    }

    /// Probes an indexer; a failing indexer is `ok: false`, not an error.
    pub async fn test_indexer(&self, id: &str) -> Result<IndexerTest, ClientError> {
        self.call(Method::POST, &format!("/api/acquire/indexers/{}/test", encode(id)), Body::None).await
    }

    pub async fn search_releases(&self, q: ReleaseSearch) -> Result<SearchResults, ClientError> {
        let path = format!(
            "/api/acquire/search{}",
            query(&[
                ("q", q.query),
                ("kind", q.kind),
                ("season", n(q.season)),
                ("episode", n(q.episode)),
                ("year", n(q.year)),
                ("indexers", q.indexers.join(",")),
            ])
        );
        self.call(Method::GET, &path, Body::None).await
    }

    pub async fn parse_release(&self, name: &str, kind: &str) -> Result<Release, ClientError> {
        let path = format!("/api/acquire/parse{}", query(&[("name", name.to_string()), ("kind", kind.to_string())]));
        self.call(Method::GET, &path, Body::None).await
    }

    // ---- grabs ------------------------------------------------------

    pub async fn grabs(&self) -> Result<GrabList, ClientError> {
        self.call(Method::GET, "/api/acquire/grabs", Body::None).await
    }

    pub async fn grab(&self, input: GrabInput) -> Result<Grab, ClientError> {
        self.call(Method::POST, "/api/acquire/grabs", json_body(&input)).await
    }

    /// `pause`, `resume` or `import` (retry a failed import).
    pub async fn grab_action(&self, id: &str, action: &str) -> Result<Grab, ClientError> {
        self.call(Method::POST, &format!("/api/acquire/grabs/{}/{}", encode(id), encode(action)), Body::None).await
    }

    /// Forgets a grab; `delete_data` also deletes the torrent's copy
    /// (never the files imported into the library).
    pub async fn remove_grab(&self, id: &str, delete_data: bool) -> Result<(), ClientError> {
        let q = if delete_data { "?data=1" } else { "" };
        self.call::<Removed>(Method::DELETE, &format!("/api/acquire/grabs/{}{q}", encode(id)), Body::None)
            .await
            .map(|_| ())
    }

    // ---- profiles, monitored titles, automation -----------------------

    pub async fn quality_profiles(&self) -> Result<Vec<Profile>, ClientError> {
        Ok(self.call::<ProfileList>(Method::GET, "/api/acquire/profiles", Body::None).await?.profiles)
    }

    pub async fn create_quality_profile(&self, p: Profile) -> Result<Profile, ClientError> {
        self.call(Method::POST, "/api/acquire/profiles", json_body(&p)).await
    }

    pub async fn update_quality_profile(&self, id: &str, p: Profile) -> Result<Profile, ClientError> {
        self.call(Method::PUT, &format!("/api/acquire/profiles/{}", encode(id)), json_body(&p)).await
    }

    pub async fn delete_quality_profile(&self, id: &str) -> Result<(), ClientError> {
        self.call::<Removed>(Method::DELETE, &format!("/api/acquire/profiles/{}", encode(id)), Body::None)
            .await
            .map(|_| ())
    }

    pub async fn monitored(&self) -> Result<Vec<Monitored>, ClientError> {
        Ok(self.call::<MonitoredList>(Method::GET, "/api/acquire/monitored", Body::None).await?.monitored)
    }

    pub async fn monitored_title(&self, id: &str) -> Result<Monitored, ClientError> {
        self.call(Method::GET, &format!("/api/acquire/monitored/{}", encode(id)), Body::None).await
    }

    pub async fn create_monitored(&self, m: Monitored) -> Result<Monitored, ClientError> {
        self.call(Method::POST, "/api/acquire/monitored", json_body(&m)).await
    }

    pub async fn update_monitored(&self, id: &str, m: Monitored) -> Result<Monitored, ClientError> {
        self.call(Method::PUT, &format!("/api/acquire/monitored/{}", encode(id)), json_body(&m)).await
    }

    pub async fn delete_monitored(&self, id: &str) -> Result<(), ClientError> {
        self.call::<Removed>(Method::DELETE, &format!("/api/acquire/monitored/{}", encode(id)), Body::None)
            .await
            .map(|_| ())
    }

    pub async fn wanted_for(&self, id: &str) -> Result<Wanted, ClientError> {
        self.call(Method::GET, &format!("/api/acquire/monitored/{}/wanted", encode(id)), Body::None).await
    }

    pub async fn search_monitored(&self, id: &str) -> Result<Report, ClientError> {
        self.call(Method::POST, &format!("/api/acquire/monitored/{}/search", encode(id)), Body::None).await
    }

    pub async fn wanted(&self) -> Result<WantedOverview, ClientError> {
        self.call(Method::GET, "/api/acquire/wanted", Body::None).await
    }

    pub async fn rss_sync(&self) -> Result<Report, ClientError> {
        self.call(Method::POST, "/api/acquire/rss", Body::None).await
    }

    pub async fn automation(&self) -> Result<AutomationState, ClientError> {
        self.call(Method::GET, "/api/acquire/automation", Body::None).await
    }

    pub async fn blocklist(&self) -> Result<Vec<BlockEntry>, ClientError> {
        Ok(self.call::<Blocklist>(Method::GET, "/api/acquire/blocklist", Body::None).await?.blocklist)
    }

    pub async fn unblock_release(&self, id: &str) -> Result<(), ClientError> {
        self.call::<Removed>(Method::DELETE, &format!("/api/acquire/blocklist/{}", encode(id)), Body::None)
            .await
            .map(|_| ())
    }

    /// Library files upgrades moved to the holding folder.
    pub async fn replaced_files(&self) -> Result<Vec<HeldFile>, ClientError> {
        Ok(self.call::<HeldFiles>(Method::GET, "/api/acquire/replaced", Body::None).await?.files)
    }

    /// Deletes every held file; answers how many.
    pub async fn purge_replaced(&self) -> Result<i64, ClientError> {
        Ok(self.call::<Deleted>(Method::POST, "/api/acquire/replaced/purge", Body::None).await?.deleted)
    }

    // ---- subtitles --------------------------------------------------

    pub async fn subtitle_providers(&self) -> Result<Vec<SubtitleProviderView>, ClientError> {
        Ok(self
            .call::<SubtitleProviderList>(Method::GET, "/api/acquire/subtitle-providers", Body::None)
            .await?
            .providers)
    }

    pub async fn create_subtitle_provider(
        &self,
        i: SubtitleProviderInput,
    ) -> Result<SubtitleProviderView, ClientError> {
        self.call(Method::POST, "/api/acquire/subtitle-providers", json_body(&i)).await
    }

    pub async fn update_subtitle_provider(
        &self,
        id: &str,
        i: SubtitleProviderInput,
    ) -> Result<SubtitleProviderView, ClientError> {
        self.call(Method::PUT, &format!("/api/acquire/subtitle-providers/{}", encode(id)), json_body(&i)).await
    }

    pub async fn delete_subtitle_provider(&self, id: &str) -> Result<(), ClientError> {
        let path = format!("/api/acquire/subtitle-providers/{}", encode(id));
        self.call::<Removed>(Method::DELETE, &path, Body::None).await.map(|_| ())
    }

    /// Candidates for an item; empty `languages` uses its profile's.
    pub async fn search_item_subtitles(
        &self,
        item_id: &str,
        languages: &[String],
    ) -> Result<SubtitleSearch, ClientError> {
        let path =
            format!("/api/acquire/items/{}/subtitles{}", encode(item_id), query(&[("languages", languages.join(","))]));
        self.call(Method::GET, &path, Body::None).await
    }

    pub async fn take_subtitle(&self, item_id: &str, take: SubtitleTake) -> Result<SubtitleRecord, ClientError> {
        self.call(Method::POST, &format!("/api/acquire/items/{}/subtitles", encode(item_id)), json_body(&take)).await
    }

    pub async fn item_subtitle_status(&self, item_id: &str) -> Result<ItemSubtitleStatus, ClientError> {
        self.call(Method::GET, &format!("/api/acquire/items/{}/subtitles/status", encode(item_id)), Body::None).await
    }

    /// Moves a sidecar of the item to the holding folder.
    pub async fn remove_sidecar(&self, item_id: &str, name: &str) -> Result<HeldFile, ClientError> {
        let path = format!("/api/acquire/items/{}/sidecars/{}", encode(item_id), encode(name));
        self.call(Method::DELETE, &path, Body::None).await
    }

    pub async fn monitored_subtitles(&self, id: &str) -> Result<Vec<FileSubtitles>, ClientError> {
        let path = format!("/api/acquire/monitored/{}/subtitles", encode(id));
        Ok(self.call::<Files>(Method::GET, &path, Body::None).await?.files)
    }

    /// Fetches every missing language for a monitored title; answers how
    /// many sidecars were written.
    pub async fn fetch_monitored_subtitles(&self, id: &str) -> Result<i64, ClientError> {
        let path = format!("/api/acquire/monitored/{}/subtitles", encode(id));
        Ok(self.call::<Written>(Method::POST, &path, Body::None).await?.written)
    }

    pub async fn subtitle_ledger(&self) -> Result<Vec<SubtitleRecord>, ClientError> {
        Ok(self.call::<SubtitleLedger>(Method::GET, "/api/acquire/subtitles", Body::None).await?.subtitles)
    }

    pub async fn subtitle_refusals(&self) -> Result<Vec<SubtitleRefusal>, ClientError> {
        Ok(self.call::<SubtitleRefusals>(Method::GET, "/api/acquire/subtitle-refusals", Body::None).await?.refusals)
    }

    pub async fn clear_subtitle_refusal(&self, provider: &str, file: &str) -> Result<(), ClientError> {
        let path = format!("/api/acquire/subtitle-refusals/{}/{}", encode(provider), encode(file));
        self.call::<Removed>(Method::DELETE, &path, Body::None).await.map(|_| ())
    }

    /// Clears every refusal; answers how many.
    pub async fn clear_subtitle_refusals(&self) -> Result<i64, ClientError> {
        Ok(self.call::<Cleared>(Method::DELETE, "/api/acquire/subtitle-refusals", Body::None).await?.cleared)
    }

    // ---- downloads --------------------------------------------------

    pub async fn downloads(&self) -> Result<DownloadsView, ClientError> {
        self.call(Method::GET, "/api/downloads", Body::None).await
    }

    pub async fn add_download(&self, add: DownloadAdd) -> Result<Job, ClientError> {
        self.call(Method::POST, "/api/downloads", json_body(&add)).await
    }

    pub async fn download(&self, id: &str) -> Result<Job, ClientError> {
        self.call(Method::GET, &format!("/api/downloads/{}", encode(id)), Body::None).await
    }

    /// `pause`, `resume` or `cancel`.
    pub async fn download_action(&self, id: &str, action: &str) -> Result<Job, ClientError> {
        self.call(Method::POST, &format!("/api/downloads/{}/{}", encode(id), encode(action)), Body::None).await
    }

    /// Forgets a finished, failed or canceled download (the file stays).
    pub async fn remove_download(&self, id: &str) -> Result<(), ClientError> {
        self.call::<Value>(Method::DELETE, &format!("/api/downloads/{}", encode(id)), Body::None).await.map(|_| ())
    }

    pub async fn download_settings(&self) -> Result<DownloadSettings, ClientError> {
        self.call(Method::GET, "/api/downloads/settings", Body::None).await
    }

    pub async fn update_download_settings(&self, s: DownloadSettings) -> Result<DownloadSettings, ClientError> {
        self.call(Method::PUT, "/api/downloads/settings", json_body(&s)).await
    }

    pub async fn cleanup_downloads(&self) -> Result<CleanupReport, ClientError> {
        self.call(Method::POST, "/api/downloads/cleanup", json_body(&json!({}))).await
    }
}
