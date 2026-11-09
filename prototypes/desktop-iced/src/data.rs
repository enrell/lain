//! Everything that talks to lain-rs (through `lain-client`) or reads the
//! shared art. All of it runs on iced's tokio executor or on the blocking
//! pool, never on the UI thread.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use iced::widget::image::Handle;
use lain_client::types::{CatalogItem, Progress, ProgressUpdate};
use lain_client::{CatalogQuery, Client, ClientConfig, EventStream, FileTokenStore, ServerEvent, StreamConfig, StreamItem};

/// One card's worth of an item.
#[derive(Debug, Clone)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub slug: String,
    pub season: i64,
    pub episode: i64,
}

impl Item {
    pub fn label(&self) -> String {
        format!("S{:02} · E{:02}", self.season, self.episode)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Catalog {
    /// Sorted by series then episode.
    pub items: Vec<Item>,
    pub by_id: HashMap<String, usize>,
    /// (slug, title) in series.json order.
    pub series: Vec<(String, String)>,
}

/// The client every task clones, set once after login (the event-stream
/// subscription needs it from a plain `fn`).
pub static CLIENT: OnceLock<Client> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct Paths {
    pub server: String,
    pub shared: PathBuf,
    pub media: PathBuf,
    pub theme: PathBuf,
}

impl Paths {
    pub fn art(&self, slug: &str, kind: ArtKind) -> PathBuf {
        let k = match kind {
            ArtKind::Poster => "poster",
            ArtKind::Backdrop => "backdrop",
        };
        self.shared.join("art").join(format!("{slug}-{k}.jpg"))
    }

    /// Even episodes play the 4K file, odd ones the 10-bit one.
    pub fn media_for(&self, episode: i64) -> PathBuf {
        if episode % 2 == 0 {
            self.media.join("teste-4k-hevc-60fps.mkv")
        } else {
            let a = self.media.join("teste-1080p-hevc-10bit-legendas.mkv");
            if a.exists() { a } else { self.media.join("teste-1080p-hevc-10bit.mkv") }
        }
    }
}

fn read_account(shared: &Path) -> Result<(String, String), String> {
    // Each prototype has its own account (conta-iced.txt); conta.txt is the fallback.
    let own = shared.join("conta-iced.txt");
    let p = if own.exists() { own } else { shared.join("conta.txt") };
    let raw = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    let mut user = None;
    let mut pass = None;
    for line in raw.lines() {
        if let Some((k, v)) = line.split_once(':') {
            match k.trim() {
                "usuario" | "user" | "username" => user = Some(v.trim().to_string()),
                "senha" | "password" => pass = Some(v.trim().to_string()),
                _ => {}
            }
        }
    }
    Ok((user.ok_or("conta.txt: no user")?, pass.ok_or("conta.txt: no password")?))
}

/// Reuse a saved session when it still verifies, else log in with the
/// shared account.
pub async fn connect(paths: Paths) -> Result<String, String> {
    let tokens = FileTokenStore::open_default().map_err(|e| e.to_string())?;
    let client = Client::new(ClientConfig::new(&paths.server), Arc::new(tokens)).map_err(|e| e.to_string())?;
    // The token store is shared per machine: reuse a session only if it
    // belongs to this prototype's account.
    let (want_user, _) = read_account(&paths.shared)?;
    let user = match client.session() {
        Some(s) if s.username == want_user => client.me().await.ok(),
        _ => None,
    };
    let user = match user {
        Some(u) if u.username == want_user => u,
        _ => {
            let (u, p) = read_account(&paths.shared)?;
            client.login(&u, &p).await.map_err(|e| format!("login: {e}"))?
        }
    };
    let _ = CLIENT.set(client);
    Ok(user.username)
}

fn client() -> Result<Client, String> {
    CLIENT.get().cloned().ok_or_else(|| "not connected".to_string())
}

/// All items, paged through `/api/catalog` (server cap 500 per page).
pub async fn catalog(paths: Paths) -> Result<Catalog, String> {
    let c = client()?;
    let mut all: Vec<CatalogItem> = Vec::new();
    loop {
        let page = c
            .catalog(CatalogQuery { limit: 500, offset: all.len() as i64, ..Default::default() })
            .await
            .map_err(|e| format!("catalog: {e}"))?;
        let n = page.items.len();
        all.extend(page.items);
        if n == 0 || all.len() as i64 >= page.total {
            break;
        }
    }
    let series_raw = std::fs::read_to_string(paths.shared.join("series.json")).map_err(|e| e.to_string())?;
    #[derive(serde::Deserialize)]
    struct S {
        slug: String,
        title: String,
    }
    let series: Vec<S> = serde_json::from_str(&series_raw).map_err(|e| e.to_string())?;
    let slug_of: HashMap<&str, &str> = series.iter().map(|s| (s.title.as_str(), s.slug.as_str())).collect();
    let order: HashMap<&str, usize> = series.iter().enumerate().map(|(i, s)| (s.slug.as_str(), i)).collect();
    let mut items: Vec<Item> = all
        .into_iter()
        .map(|ci| Item {
            slug: slug_of.get(ci.title.as_str()).map(|s| s.to_string()).unwrap_or_default(),
            id: ci.id,
            title: ci.title,
            season: ci.season,
            episode: ci.episode,
        })
        .collect();
    items.sort_by_key(|i| (order.get(i.slug.as_str()).copied().unwrap_or(usize::MAX), i.season, i.episode));
    let by_id = items.iter().enumerate().map(|(i, it)| (it.id.clone(), i)).collect();
    Ok(Catalog { items, by_id, series: series.into_iter().map(|s| (s.slug, s.title)).collect() })
}

pub async fn continue_watching() -> Result<Vec<Progress>, String> {
    let mut v = client()?.continue_watching().await.map_err(|e| format!("continue: {e}"))?;
    v.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(v)
}

pub async fn progress(item_id: String) -> Option<Progress> {
    client().ok()?.progress(&item_id).await.ok()
}

pub async fn save_progress(item_id: String, position: f64, duration: f64) -> Result<(), String> {
    if duration <= 0.0 {
        return Ok(());
    }
    let completed = position >= duration * 0.95;
    let update = ProgressUpdate { position_sec: position, duration_sec: duration, completed };
    client()?.put_progress(&item_id, update).await.map(|_| ()).map_err(|e| e.to_string())
}

/// What the app cares about from `/api/events`.
#[derive(Debug, Clone)]
pub enum Live {
    Connected,
    ProgressChanged,
    Down(String),
}

/// The event stream as a plain stream, for `Subscription::run`.
pub fn events() -> impl iced::futures::Stream<Item = Live> {
    iced::stream::channel(32, async |mut out: iced::futures::channel::mpsc::Sender<Live>| {
        use iced::futures::SinkExt;
        let Some(client) = CLIENT.get().cloned() else { return };
        let stream = EventStream::new(client, StreamConfig::default());
        while let Some(item) = stream.next().await {
            let msg = match item {
                StreamItem::Connected(_) => Live::Connected,
                StreamItem::Event(ServerEvent::ProgressChanged(_)) => Live::ProgressChanged,
                StreamItem::Event(_) => continue,
                StreamItem::Reconnecting { reason, .. } => Live::Down(reason),
                StreamItem::AuthRequired => Live::Down("session refused".into()),
            };
            if out.send(msg).await.is_err() {
                break;
            }
        }
    })
}

// ---- art ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArtKind {
    Poster,
    Backdrop,
}

/// Cache key: the same file decoded at a different size is another entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArtKey {
    pub slug: String,
    pub kind: ArtKind,
    pub w: u32,
    pub h: u32,
    pub blur: u32,
}

/// Decode, crop-to-fill and (optionally) blur one image on the blocking
/// pool. The result is RGBA ready for the GPU atlas.
pub async fn load_art(path: PathBuf, key: ArtKey) -> (ArtKey, Option<Handle>) {
    let k = key.clone();
    let out = tokio::task::spawn_blocking(move || -> Option<Handle> {
        let img = image::open(&path).ok()?;
        let mut img = img.resize_to_fill(k.w.max(1), k.h.max(1), image::imageops::FilterType::Triangle);
        if k.blur > 0 {
            // Blur at quarter size, scale back up: the hero blur costs
            // tens of ms instead of seconds, and it's soft anyway.
            let (w, h) = (img.width(), img.height());
            let small = img.resize_exact((w / 4).max(1), (h / 4).max(1), image::imageops::FilterType::Triangle);
            let small = small.blur(k.blur as f32 / 4.0);
            img = small.resize_exact(w, h, image::imageops::FilterType::Triangle);
        }
        let rgba = img.into_rgba8();
        let (w, h) = rgba.dimensions();
        Some(Handle::from_rgba(w, h, rgba.into_raw()))
    })
    .await
    .ok()
    .flatten();
    (key, out)
}
