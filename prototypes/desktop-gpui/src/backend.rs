//! Everything that talks to the server goes through `lain-client`, on a
//! small tokio runtime of its own. GPUI's executors can await tokio
//! `JoinHandle`s and tokio channels directly, so the UI never blocks.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lain_client::types::{CatalogItem, Progress, ProgressUpdate};
use lain_client::{CatalogQuery, Client, ClientConfig, ClientError, EventStream, FileTokenStore, ServerEvent, StreamConfig, StreamItem};

pub struct Backend {
    pub rt: tokio::runtime::Runtime,
    pub client: Client,
}

/// One playable card.
#[derive(Debug, Clone)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub season: i64,
    pub episode: i64,
    pub slug: String,
}

impl Item {
    pub fn label(&self) -> String {
        format!("S{} · E{:02}", self.season.max(1), self.episode)
    }
}

pub struct Account {
    pub user: String,
    pub password: String,
    pub server: String,
}

/// `shared/conta.txt`: `usuario:`, `senha:`, `servidor:` lines.
pub fn read_account(path: &Path) -> Result<Account, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut a = Account { user: String::new(), password: String::new(), server: "http://127.0.0.1:9360".into() };
    for line in raw.lines() {
        if let Some((k, v)) = line.split_once(':') {
            let v = v.trim().to_string();
            match k.trim() {
                "usuario" => a.user = v,
                "senha" => a.password = v,
                "servidor" => a.server = v,
                _ => {}
            }
        }
    }
    Ok(a)
}

pub struct Loaded {
    pub items: Vec<Item>,
    pub continues: Vec<Progress>,
    pub library: String,
    pub username: String,
}

impl Backend {
    pub fn new(server: &str, session_file: PathBuf) -> Result<Arc<Backend>, String> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("lain-net")
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let tokens = Arc::new(FileTokenStore::new(session_file));
        let client = Client::new(ClientConfig::new(server), tokens).map_err(|e| e.to_string())?;
        Ok(Arc::new(Backend { rt, client }))
    }

    pub fn spawn<F>(&self, f: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.rt.spawn(f)
    }
}

/// Logs in (reusing a saved session when it still works), then reads the
/// libraries, the whole catalog (paged) and the Continue row.
pub async fn load_all(client: Client, account: Account, series: HashMap<String, String>) -> Result<Loaded, String> {
    let me = match client.session() {
        Some(_) => client.me().await.ok(),
        None => None,
    };
    let username = match me {
        Some(u) => u.username,
        None => client.login(&account.user, &account.password).await.map_err(|e| format!("login: {e}"))?.username,
    };
    let libraries = client.libraries().await.map_err(|e| format!("libraries: {e}"))?;
    let library = libraries.first().map(|l| l.name.clone()).unwrap_or_default();

    let mut raw: Vec<CatalogItem> = Vec::new();
    loop {
        let page = client
            .catalog(CatalogQuery { limit: 500, offset: raw.len() as i64, ..Default::default() })
            .await
            .map_err(|e| format!("catalog: {e}"))?;
        let n = page.items.len();
        raw.extend(page.items);
        if n == 0 || raw.len() as i64 >= page.total {
            break;
        }
    }
    let mut items: Vec<Item> = raw
        .into_iter()
        .map(|c| Item {
            slug: series.get(&c.title).cloned().unwrap_or_default(),
            id: c.id,
            title: c.title,
            season: c.season,
            episode: c.episode,
        })
        .collect();
    items.sort_by(|a, b| (&a.slug, a.season, a.episode).cmp(&(&b.slug, b.season, b.episode)));
    let continues = client.continue_watching().await.map_err(|e| format!("continue: {e}"))?;
    Ok(Loaded { items, continues, library, username })
}

pub async fn continues(client: Client) -> Result<Vec<Progress>, ClientError> {
    client.continue_watching().await
}

pub async fn progress(client: Client, id: String) -> Option<Progress> {
    client.progress(&id).await.ok()
}

pub async fn save_progress(client: Client, id: String, pos: f64, dur: f64) {
    let completed = dur > 0. && pos / dur > 0.92;
    if let Err(e) = client.put_progress(&id, ProgressUpdate { position_sec: pos, duration_sec: dur, completed }).await {
        eprintln!("progress {id}: {e}");
    }
}

/// Forwards `progress-changed` (and reconnects) to the UI.
pub async fn watch_events(client: Client, tx: tokio::sync::mpsc::UnboundedSender<()>) {
    let stream = EventStream::new(client, StreamConfig::default());
    while let Some(item) = stream.next().await {
        let poke = matches!(item, StreamItem::Connected(_) | StreamItem::Event(ServerEvent::ProgressChanged(_)));
        if poke && tx.send(()).is_err() {
            return;
        }
    }
}
