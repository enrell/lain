//! Everything that talks to the server goes through `lain-client`, on a
//! small tokio runtime of its own. GPUI's executors await tokio
//! `JoinHandle`s and channels directly, so the UI thread never blocks.
//!
//! One [`Backend`] per server: the typed client, the progress sync
//! engine (offline queue) and the offline store share one client
//! database, opened once per process.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lain_client::types::{CatalogItem, Progress, ProgressUpdate};
use lain_client::{
    CatalogQuery, Client, ClientConfig, ClientDb, ClientError, EventStream, FileTokenStore, OfflineConfig,
    OfflineStore, ProgressSync, StreamConfig, StreamItem,
};

pub struct Net {
    pub rt: tokio::runtime::Runtime,
    pub db: Option<Arc<ClientDb>>,
    pub db_error: Option<String>,
    pub dir: PathBuf,
}

impl Net {
    pub fn new(dir: &Path) -> Result<Arc<Net>, String> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("lain-net")
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let _ = std::fs::create_dir_all(dir);
        let (db, db_error) = match ClientDb::open(&dir.join("client.redb")) {
            Ok(d) => (Some(Arc::new(d)), None),
            Err(e) => (None, Some(e.to_string())),
        };
        Ok(Arc::new(Net { rt, db, db_error, dir: dir.to_path_buf() }))
    }
}

pub struct Backend {
    pub net: Arc<Net>,
    pub client: Client,
    pub sync: Option<ProgressSync>,
    pub offline: Option<Arc<OfflineStore>>,
    pub offline_error: Option<String>,
}

impl Backend {
    pub fn new(
        net: Arc<Net>,
        server: &str,
        offline_dir: &Path,
        offline_max_gib: u64,
    ) -> Result<Arc<Backend>, ClientError> {
        let tokens = Arc::new(FileTokenStore::new(net.dir.join("sessions.json")));
        let client = Client::new(ClientConfig::new(server), tokens)?;
        let sync = net.db.clone().map(|db| ProgressSync::new(client.clone(), db));
        let (offline, offline_error) = match net.db.clone() {
            Some(db) => {
                let _ = std::fs::create_dir_all(offline_dir);
                let dir = std::fs::canonicalize(offline_dir).unwrap_or_else(|_| offline_dir.to_path_buf());
                let mut cfg = OfflineConfig::new(&dir.to_string_lossy());
                cfg.max_bytes = offline_max_gib << 30;
                match OfflineStore::open(cfg, db) {
                    Ok(o) => (Some(Arc::new(o)), None),
                    Err(e) => (None, Some(e.to_string())),
                }
            }
            None => (None, net.db_error.clone()),
        };
        Ok(Arc::new(Backend { net, client, sync, offline, offline_error }))
    }

    pub fn spawn<F>(&self, f: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.net.rt.spawn(f)
    }

    pub fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
        self.net.rt.block_on(f)
    }
}

/// Every catalog item the user may see, paged.
pub async fn all_items(client: Client) -> Result<Vec<CatalogItem>, ClientError> {
    let mut out: Vec<CatalogItem> = Vec::new();
    loop {
        let page = client.catalog(CatalogQuery { limit: 500, offset: out.len() as i64, ..Default::default() }).await?;
        let n = page.items.len();
        out.extend(page.items);
        if n == 0 || out.len() as i64 >= page.total {
            break;
        }
    }
    Ok(out)
}

/// Progress for many items at once (small concurrency).
pub async fn progress_many(sync: Option<ProgressSync>, client: Client, ids: Vec<String>) -> Vec<(String, Progress)> {
    let mut out = Vec::new();
    for chunk in ids.chunks(8) {
        let mut handles = Vec::new();
        for id in chunk {
            let (sync, client, id) = (sync.clone(), client.clone(), id.clone());
            handles.push(tokio::spawn(async move {
                let p = match sync {
                    Some(s) => s.fetch(&id).await,
                    None => client.progress(&id).await,
                };
                (id, p)
            }));
        }
        for h in handles {
            if let Ok((id, Ok(p))) = h.await {
                out.push((id, p));
            }
        }
    }
    out
}

/// Records a position through the sync engine (queued when offline).
pub async fn save_progress(
    b: Arc<Backend>,
    id: String,
    pos: f64,
    dur: f64,
    completed: bool,
) -> Result<(), ClientError> {
    let update = ProgressUpdate { position_sec: pos.max(0.), duration_sec: dur.max(0.), completed };
    match &b.sync {
        Some(s) => s.record(&id, update).await.map(|_| ()),
        None => b.client.put_progress(&id, update).await.map(|_| ()),
    }
}

/// What the event stream tells the UI.
#[derive(Debug, Clone)]
pub enum Poke {
    Connected,
    Lost,
    AuthRequired,
    Progress(String),
    Library,
    /// `scan-state` (D-188): live scan progress.
    Scan(Box<lain_client::types::ScanStateEvent>),
    Modules,
    Plugins,
    /// `transcode-changed` (D-193): a session ended or was cancelled.
    Transcodes,
}

/// Forwards server events to the UI and feeds the sync engine.
pub async fn watch_events(b: Arc<Backend>, tx: tokio::sync::mpsc::UnboundedSender<Poke>) {
    use lain_client::ServerEvent as E;
    let stream = EventStream::new(b.client.clone(), StreamConfig::default());
    while let Some(item) = stream.next().await {
        let poke = match item {
            StreamItem::Connected(_) => {
                if let Some(s) = &b.sync {
                    let _ = s.flush().await;
                }
                Poke::Connected
            }
            StreamItem::Reconnecting { .. } => Poke::Lost,
            StreamItem::AuthRequired => Poke::AuthRequired,
            StreamItem::Event(ev) => {
                if let Some(s) = &b.sync {
                    let _ = s.observe(&ev);
                }
                match ev {
                    E::ProgressChanged(p) => Poke::Progress(p.item_id),
                    E::LibraryChanged(_) => Poke::Library,
                    E::ModuleState(_) => Poke::Modules,
                    E::PluginChanged(_) => Poke::Plugins,
                    E::TranscodeChanged(_) => Poke::Transcodes,
                    E::ScanState(s) => Poke::Scan(Box::new(s)),
                    E::Other { event, .. } if event == "item-deleted" => Poke::Library,
                    E::Other { .. } => continue,
                }
            }
        };
        if tx.send(poke).is_err() {
            return;
        }
    }
}

/// A short, human reason for a failed call, for inline error text.
pub fn reason(e: &ClientError) -> String {
    match e {
        ClientError::NotPorted { .. } => "not available on this server yet".into(),
        ClientError::ModuleDisabled { module } => format!("the {module} module is disabled on this server"),
        ClientError::Offline { .. } | ClientError::Timeout => "the server is unreachable".into(),
        ClientError::Unauthorized => "your session ended; log in again".into(),
        ClientError::Forbidden { message } if message.is_empty() => "not allowed".into(),
        ClientError::Forbidden { message } => message.clone(),
        ClientError::Api { message, status, .. } if message.is_empty() => format!("server error {status}"),
        ClientError::Api { message, .. } => message.clone(),
        other => other.to_string(),
    }
}
