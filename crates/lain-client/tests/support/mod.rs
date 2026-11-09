//! Test harness: an in-process `lain-rs`, a TCP proxy that can drop the
//! network on demand, and a hand-written HTTP stub for answers lain-rs
//! cannot give yet (byte streams, refusals).

#![allow(dead_code)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use lain_client::{Client, ClientConfig, MemoryTokenStore, Session, TokenStore};
use lain_server::app::{App, Config};

pub struct Server {
    pub url: String,
    pub addr: SocketAddr,
    pub app: Arc<App>,
    pub dir: PathBuf,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Server {
    /// Starts lain-rs on `data` (inside a tokio runtime).
    pub async fn start(data: &Path) -> Server {
        let mut cfg = Config::new(data, lain_server::VERSION);
        cfg.keep_alive = Duration::from_millis(300);
        cfg.reload_debounce = Duration::from_millis(100);
        let app = App::open(cfg).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        app.start_background();
        tokio::spawn(lain_server::serve(app.clone(), listener, async move {
            let _ = rx.await;
        }));
        Server { url: format!("http://{addr}"), addr, app, dir: data.to_path_buf(), stop: Some(tx) }
    }

    /// A data dir holding the conformance fixture catalog (two Frieren
    /// episodes in library `lib-e8bb66fd`, root `media`).
    pub async fn with_fixture(root: &Path) -> Server {
        let media = root.join("media");
        std::fs::create_dir_all(media.join("Frieren")).unwrap();
        let data = root.join("data");
        std::fs::create_dir_all(&data).unwrap();
        let roots = serde_json::json!({"format": "lain.roots@1", "roots": {"media": media.to_string_lossy()}});
        std::fs::write(data.join("roots.json"), serde_json::to_vec(&roots).unwrap()).unwrap();
        let raw = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/fixtures/catalog.json"))
            .unwrap();
        assert_eq!(lain_server::import(&data, &serde_json::from_slice(&raw).unwrap()).unwrap(), 2);
        Server::start(&data).await
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        self.app.shutdown();
    }
}

pub fn client(url: &str) -> Client {
    Client::new(ClientConfig { server: url.to_string(), timeout_ms: 5_000 }, Arc::new(MemoryTokenStore::new())).unwrap()
}

/// A fresh server with `admin` (password123) and `bob` (bobpassword).
pub async fn admin_and_bob(url: &str) -> (Client, Client) {
    let admin = client(url);
    admin.setup("admin", "password123").await.unwrap();
    admin.login("admin", "password123").await.unwrap();
    admin.create_user("bob", "bobpassword", "").await.unwrap();
    let bob = client(url);
    bob.login("bob", "bobpassword").await.unwrap();
    (admin, bob)
}

/// The raw JSON of an authenticated GET, for checking that the typed
/// answers mirror the wire exactly.
pub async fn raw_get(url: &str, token: &str, path: &str) -> serde_json::Value {
    let base = lain_conformance::http::Base::parse(url).unwrap();
    let headers = vec![("Authorization".to_string(), format!("Bearer {token}"))];
    let path = path.to_string();
    let r = tokio::task::spawn_blocking(move || lain_conformance::http::request(&base, "GET", &path, &headers, None))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    serde_json::from_slice(&r.body).unwrap()
}

/// Decodes `raw` as `T`, re-encodes it and demands the same JSON:
/// nothing dropped, renamed or retyped. `"pass_hash": null` is the one
/// field the client types drop on purpose.
pub fn assert_mirrors<T: serde::de::DeserializeOwned + serde::Serialize>(raw: &serde_json::Value) -> T {
    fn strip(v: &serde_json::Value) -> serde_json::Value {
        match v {
            serde_json::Value::Object(o) => serde_json::Value::Object(
                o.iter()
                    .filter(|(k, v)| !(k.as_str() == "pass_hash" && v.is_null()))
                    .map(|(k, v)| (k.clone(), strip(v)))
                    .collect(),
            ),
            serde_json::Value::Array(a) => serde_json::Value::Array(a.iter().map(strip).collect()),
            other => other.clone(),
        }
    }
    let typed: T = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(serde_json::to_value(&typed).unwrap(), strip(raw), "{}", std::any::type_name::<T>());
    typed
}

/// A TCP proxy in front of a server; `set_online(false)` refuses new
/// connections and cuts the open ones, like a dropped network.
pub struct Proxy {
    pub url: String,
    online: Arc<AtomicBool>,
    cut: tokio::sync::watch::Sender<u64>,
}

impl Proxy {
    pub async fn start(target: SocketAddr) -> Proxy {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let online = Arc::new(AtomicBool::new(true));
        let (cut, _) = tokio::sync::watch::channel(0u64);
        let (on, cut_tx) = (online.clone(), cut.clone());
        tokio::spawn(async move {
            while let Ok((mut down, _)) = listener.accept().await {
                if !on.load(Ordering::SeqCst) {
                    drop(down);
                    continue;
                }
                let mut rx = cut_tx.subscribe();
                tokio::spawn(async move {
                    let Ok(mut up) = TcpStream::connect(target).await else { return };
                    tokio::select! {
                        _ = tokio::io::copy_bidirectional(&mut down, &mut up) => {}
                        _ = rx.changed() => {}
                    }
                });
            }
        });
        Proxy { url, online, cut }
    }

    pub fn set_online(&self, on: bool) {
        self.online.store(on, Ordering::SeqCst);
        if !on {
            self.cut.send_modify(|g| *g += 1);
        }
    }
}

/// What the stub answers.
#[derive(Clone)]
pub struct StubFile {
    pub item: String,
    pub bytes: Vec<u8>,
    /// Without a `Range`, send only this many bytes and hang up.
    pub cut_after: Option<usize>,
}

pub struct Stub {
    pub url: String,
    /// The status `PUT .../progress` answers: 200, 400 (a refusal) or
    /// 503.
    pub put_status: Arc<std::sync::atomic::AtomicU16>,
}

/// A minimal HTTP/1.1 server: `GET /api/items/{id}/stream` with ETag and
/// `Range: bytes=N-`, `GET .../progress` answering an empty record, and
/// `PUT .../progress` answering [`Stub::put_status`]. One request per
/// connection.
pub async fn stub(files: Vec<StubFile>) -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let files = Arc::new(files);
    let put_status = Arc::new(std::sync::atomic::AtomicU16::new(200));
    let put = put_status.clone();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let files = files.clone();
            let put = put.load(Ordering::SeqCst);
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match s.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                }
                let head = String::from_utf8_lossy(&buf).to_string();
                let mut lines = head.split("\r\n");
                let first: Vec<&str> = lines.next().unwrap_or("").split(' ').collect();
                let (method, path) = (first[0], first.get(1).copied().unwrap_or(""));
                let range: Option<usize> = lines
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("range: bytes=").map(str::to_string))
                    .and_then(|r| r.trim_end_matches('-').parse().ok());
                let reply = |status: &str, headers: &str, body: &[u8]| {
                    let mut out = format!("HTTP/1.1 {status}\r\nconnection: close\r\n{headers}").into_bytes();
                    out.extend_from_slice(b"\r\n");
                    out.extend_from_slice(body);
                    out
                };
                let out = if path.ends_with("/progress") && method == "GET" {
                    let b = br#"{"item_id":"","user_id":"","position_sec":0,"duration_sec":0,"completed":false,"updated_at":0}"#;
                    reply("200 OK", &format!("content-type: application/json\r\ncontent-length: {}\r\n", b.len()), b)
                } else if path.ends_with("/progress") && put == 200 {
                    let b = br#"{"item_id":"x","user_id":"user-stub","position_sec":1,"duration_sec":0,"completed":false,"updated_at":1}"#;
                    reply("200 OK", &format!("content-type: application/json\r\ncontent-length: {}\r\n", b.len()), b)
                } else if path.ends_with("/progress") {
                    let b = br#"{"error":"refused"}"#;
                    let status = if put == 400 { "400 Bad Request" } else { "503 Service Unavailable" };
                    reply(status, &format!("content-length: {}\r\n", b.len()), b)
                } else if let Some(f) = files.iter().find(|f| path == format!("/api/items/{}/stream", f.item)) {
                    let etag = format!("\"v-{}\"", f.item);
                    match range {
                        Some(start) if start < f.bytes.len() => {
                            let body = &f.bytes[start..];
                            let h = format!(
                                "etag: {etag}\r\ncontent-range: bytes {start}-{}/{}\r\ncontent-length: {}\r\n",
                                f.bytes.len() - 1,
                                f.bytes.len(),
                                body.len()
                            );
                            reply("206 Partial Content", &h, body)
                        }
                        _ => {
                            let h = format!(
                                "etag: {etag}\r\naccept-ranges: bytes\r\ncontent-length: {}\r\n",
                                f.bytes.len()
                            );
                            let n = f.cut_after.filter(|_| range.is_none()).unwrap_or(f.bytes.len());
                            reply("200 OK", &h, &f.bytes[..n])
                        }
                    }
                } else {
                    let b = br#"{"error":"not found"}"#;
                    reply("404 Not Found", &format!("content-length: {}\r\n", b.len()), b)
                };
                let _ = s.write_all(&out).await;
                let _ = s.flush().await;
                let _ = s.shutdown().await;
            });
        }
    });
    Stub { url, put_status }
}

/// A client of the stub with a made-up session.
pub fn stub_client(url: &str) -> Client {
    let tokens = Arc::new(MemoryTokenStore::new());
    let c = Client::new(ClientConfig { server: url.to_string(), timeout_ms: 5_000 }, tokens.clone()).unwrap();
    tokens
        .save(&Session { server: c.server(), token: "t".into(), user_id: "user-stub".into(), username: "stub".into() })
        .unwrap();
    c.reload_session().unwrap();
    c
}

pub fn temp(tag: &str) -> PathBuf {
    lain_store::test_dir(&format!("client-{tag}"))
}
