//! Everything the app asks the server, through lain-client.

use std::path::Path;
use std::sync::Arc;

use lain_client::types::{CatalogItem, Progress, ProgressUpdate};
use lain_client::{CatalogQuery, Client, ClientConfig, FileTokenStore};

pub struct Creds {
    pub user: String,
    pub password: String,
}

/// `LAIN_USER`/`LAIN_PASSWORD`, else the shared `conta-slint.txt`
/// (`usuario: ...` / `senha: ...`).
pub fn creds(conta: &Path) -> Option<Creds> {
    if let (Ok(user), Ok(password)) = (std::env::var("LAIN_USER"), std::env::var("LAIN_PASSWORD")) {
        return Some(Creds { user, password });
    }
    let raw = std::fs::read_to_string(conta).ok()?;
    let field = |k: &str| {
        raw.lines().find_map(|l| l.strip_prefix(k).map(|v| v.trim_start_matches(':').trim().to_string()))
    };
    Some(Creds { user: field("usuario")?, password: field("senha")? })
}

pub fn client(server: &str) -> Result<Client, String> {
    // own session file, so this prototype never shares a login with the
    // other clients on the machine
    let dir = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from).ok_or("no LOCALAPPDATA")?;
    let store = FileTokenStore::new(dir.join("lain-slint").join("sessions.json"));
    Client::new(ClientConfig::new(server), Arc::new(store)).map_err(|e| e.to_string())
}

/// Reuses the stored session when it still works, else logs in.
pub async fn ensure_login(c: &Client, creds: Option<&Creds>) -> Result<String, String> {
    let same_user = |u: &str| creds.is_none_or(|c| c.user == u);
    if c.session().is_some()
        && let Ok(me) = c.me().await
        && same_user(&me.username)
    {
        return Ok(me.username);
    }
    let creds = creds.ok_or("no stored session and no credentials (LAIN_USER/LAIN_PASSWORD or conta-slint.txt)")?;
    let u = c.login(&creds.user, &creds.password).await.map_err(|e| format!("login: {e}"))?;
    Ok(u.username)
}

/// The whole catalog, paged at the server's cap.
pub async fn catalog(c: &Client) -> Result<Vec<CatalogItem>, String> {
    let mut out = Vec::new();
    loop {
        let page = c
            .catalog(CatalogQuery { limit: 500, offset: out.len() as i64, ..Default::default() })
            .await
            .map_err(|e| format!("catalog: {e}"))?;
        let n = page.items.len();
        out.extend(page.items);
        if n == 0 || out.len() as i64 >= page.total {
            break;
        }
    }
    Ok(out)
}

pub async fn continue_list(c: &Client) -> Result<Vec<Progress>, String> {
    c.continue_watching().await.map_err(|e| format!("continue: {e}"))
}

pub async fn progress(c: &Client, item: &str) -> Option<Progress> {
    tokio::time::timeout(std::time::Duration::from_secs(3), c.progress(item)).await.ok()?.ok()
}

pub async fn save(c: &Client, item: &str, pos: f64, dur: f64) {
    if dur <= 0.0 {
        return;
    }
    let completed = pos >= dur * 0.92;
    if let Err(e) = c.put_progress(item, ProgressUpdate { position_sec: pos, duration_sec: dur, completed }).await {
        eprintln!("progress save {item}: {e}");
    }
}
