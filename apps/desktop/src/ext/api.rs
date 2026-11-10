//! The host API's server side: `lain-client` with the user's session
//! (catalog, libraries, progress) and `lain-http` for `net:<host>` GETs.
//! The scope checks are in `runtime.rs`; this only performs calls that
//! passed them. Workers are plain threads, so calls block on the app's
//! network runtime.

use std::time::Duration;

use lain_client::{CatalogQuery, Client, ClientError};

use super::runtime::{HostApi, HostError, HttpResponse, Item, Library, Progress};

pub struct ClientApi {
    pub client: Client,
    pub rt: tokio::runtime::Handle,
    pub http: lain_http::Client,
}

impl ClientApi {
    pub fn new(client: Client, rt: tokio::runtime::Handle) -> Result<ClientApi, String> {
        let http = lain_http::Client::new(lain_http::Config {
            user_agent: concat!("lain-desktop-extension/", env!("CARGO_PKG_VERSION")).into(),
            timeout: Duration::from_secs(8),
            retries: 0,
            max_redirects: 0,
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;
        Ok(ClientApi { client, rt, http })
    }

    fn run<T>(&self, f: impl std::future::Future<Output = Result<T, ClientError>>) -> Result<T, HostError> {
        let r = self.rt.block_on(async { tokio::time::timeout(Duration::from_secs(8), f).await });
        match r {
            Ok(r) => r.map_err(err),
            Err(_) => Err(HostError::Offline),
        }
    }
}

fn err(e: ClientError) -> HostError {
    match e {
        ClientError::NotFound { .. } => HostError::NotFound,
        e if e.is_offline() => HostError::Offline,
        ClientError::Timeout => HostError::Offline,
        e => HostError::Failed(crate::backend::reason(&e)),
    }
}

fn item(c: lain_client::types::CatalogItem) -> Item {
    Item {
        id: c.id,
        library_id: c.library_id,
        kind: c.kind,
        title: c.title,
        season: c.season,
        episode: c.episode,
        year: c.year,
        size: c.size,
        updated_at: c.updated_at,
    }
}

fn progress(p: lain_client::types::Progress) -> Progress {
    Progress {
        item_id: p.item_id,
        position_sec: p.position_sec,
        duration_sec: p.duration_sec,
        completed: p.completed,
        updated_at: p.updated_at,
    }
}

impl HostApi for ClientApi {
    fn libraries(&self) -> Result<Vec<Library>, HostError> {
        let libs = self.run(self.client.libraries())?;
        Ok(libs
            .into_iter()
            .filter(|l| l.locked != Some(true))
            .map(|l| Library { id: l.id, name: l.name, kind: l.kind })
            .collect())
    }

    fn catalog(&self, offset: u32, limit: u32) -> Result<Vec<Item>, HostError> {
        let page = self.run(self.client.catalog(CatalogQuery {
            limit: i64::from(limit),
            offset: i64::from(offset),
            ..Default::default()
        }))?;
        Ok(page.items.into_iter().filter(|i| !i.missing).map(item).collect())
    }

    fn item(&self, id: &str) -> Result<Item, HostError> {
        self.run(self.client.catalog_item(id)).map(item)
    }

    fn progress(&self, item_id: &str) -> Result<Option<Progress>, HostError> {
        match self.run(self.client.progress(item_id)) {
            Ok(p) if p.updated_at == 0 && p.position_sec == 0. && !p.completed => Ok(None),
            Ok(p) => Ok(Some(progress(p))),
            Err(HostError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn continue_watching(&self) -> Result<Vec<Progress>, HostError> {
        let mut v = self.run(self.client.continue_watching())?;
        v.sort_by_key(|p| (p.completed, std::cmp::Reverse(p.updated_at)));
        Ok(v.into_iter().map(progress).collect())
    }

    fn http_get(&self, url: &str, max_body: usize) -> Result<HttpResponse, HostError> {
        // No token, no redirects (a 3xx goes back to the extension, which
        // may follow it only to a host it was granted).
        let r = self.http.get(url).no_redirects().max_body(max_body).send_blocking();
        match r {
            Ok(r) => Ok(HttpResponse { status: r.status, headers: r.headers, body: r.body }),
            Err(e) => Err(match e.kind {
                lain_http::ErrorKind::TooLarge => HostError::TooLarge,
                lain_http::ErrorKind::Connect | lain_http::ErrorKind::Timeout | lain_http::ErrorKind::Offline => {
                    HostError::Offline
                }
                _ => HostError::Failed(e.to_string()),
            }),
        }
    }
}
