//! The engine: a tokio runtime of its own, the inbound listener and the
//! torrent table, behind a blocking API.

use std::collections::HashMap;
use std::fmt;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::runtime::{Handle, Runtime};

use crate::session::Torrent;
use crate::sync::{Cancel, WaitGroup};
use crate::wire::{handshake_bytes, read_handshake};
use crate::{Error, HttpGet, InfoHash, Limiter, PlainHttp, Spec};

/// Tunes the engine. Zero values take Go's defaults.
#[derive(Clone, Default)]
pub struct Config {
    /// Accepts inbound peers (`None` disables inbound; tests use
    /// `127.0.0.1:0`).
    pub listen_addr: Option<String>,
    /// Our identity; all zeros picks a random `-LN0001-` id.
    pub peer_id: [u8; 20],
    pub max_peers_per_torrent: usize,
    pub upload_slots: usize,
    /// Bytes per second, 0 = unlimited.
    pub upload_rate: i64,
    pub download_rate: i64,
    /// The tracker HTTP client; `None` is [`PlainHttp`] (http:// only).
    pub http: Option<Arc<dyn HttpGet>>,
    pub dial_timeout: Duration,
    /// Floors the tracker's interval (default 60 s).
    pub min_announce_interval: Duration,
    /// Returns a block request to the pool (default 45 s).
    pub request_timeout: Duration,
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("listen_addr", &self.listen_addr)
            .field("max_peers_per_torrent", &self.max_peers_per_torrent)
            .field("upload_slots", &self.upload_slots)
            .field("upload_rate", &self.upload_rate)
            .field("download_rate", &self.download_rate)
            .finish_non_exhaustive()
    }
}

impl Config {
    fn with_defaults(mut self) -> Config {
        if self.max_peers_per_torrent == 0 {
            self.max_peers_per_torrent = 40;
        }
        if self.upload_slots == 0 {
            self.upload_slots = 4;
        }
        if self.http.is_none() {
            self.http = Some(Arc::new(PlainHttp::default()));
        }
        if self.dial_timeout.is_zero() {
            self.dial_timeout = Duration::from_secs(10);
        }
        if self.min_announce_interval.is_zero() {
            self.min_announce_interval = Duration::from_secs(60);
        }
        if self.request_timeout.is_zero() {
            self.request_timeout = Duration::from_secs(45);
        }
        if self.peer_id == [0; 20] {
            self.peer_id[..8].copy_from_slice(b"-LN0001-");
            if getrandom::fill(&mut self.peer_id[8..]).is_err() {
                let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                self.peer_id[8..].copy_from_slice(&t.as_nanos().to_be_bytes()[4..]);
            }
        }
        self
    }
}

struct Table {
    torrents: HashMap<InfoHash, Torrent>,
    closed: bool,
}

pub(crate) struct ClientInner {
    pub cfg: Config,
    pub handle: Handle,
    rt: Mutex<Option<Runtime>>,
    pub port: u16,
    pub up: Limiter,
    pub down: Limiter,
    table: Mutex<Table>,
    cancel: Arc<Cancel>,
    wg: WaitGroup,
}

impl ClientInner {
    pub fn cfg_http(&self) -> Arc<dyn HttpGet> {
        self.cfg.http.clone().unwrap_or_else(|| Arc::new(PlainHttp::default()))
    }

    fn table(&self) -> std::sync::MutexGuard<'_, Table> {
        self.table.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn torrent(&self, ih: &InfoHash) -> Option<Torrent> {
        self.table().torrents.get(ih).cloned()
    }

    pub fn forget(&self, t: &Torrent) {
        let mut tb = self.table();
        if tb.torrents.get(&t.info_hash()).is_some_and(|x| Arc::ptr_eq(&x.0, &t.0)) {
            tb.torrents.remove(&t.info_hash());
        }
    }
}

/// Runs torrents and the inbound listener.
pub struct Client {
    inner: Arc<ClientInner>,
}

impl Client {
    /// Starts the engine.
    pub fn new(cfg: Config) -> Result<Client, Error> {
        let cfg = cfg.with_defaults();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("lain-torrent")
            .enable_io()
            .enable_time()
            .build()?;
        let mut port = 0;
        let mut listener = None;
        if let Some(addr) = &cfg.listen_addr {
            let ln = std::net::TcpListener::bind(addr.as_str())?;
            ln.set_nonblocking(true)?;
            port = ln.local_addr()?.port();
            let _g = rt.enter();
            listener = Some(tokio::net::TcpListener::from_std(ln)?);
        }
        let inner = Arc::new(ClientInner {
            up: Limiter::new(cfg.upload_rate),
            down: Limiter::new(cfg.download_rate),
            cfg,
            handle: rt.handle().clone(),
            rt: Mutex::new(Some(rt)),
            port,
            table: Mutex::new(Table { torrents: HashMap::new(), closed: false }),
            cancel: Cancel::new(),
            wg: WaitGroup::default(),
        });
        if let Some(ln) = listener {
            let g = inner.wg.add();
            let c = inner.clone();
            inner.handle.spawn(async move {
                let _g = g;
                accept_loop(c, ln).await;
            });
        }
        Ok(Client { inner })
    }

    /// The inbound TCP port (0 without a listener).
    pub fn port(&self) -> u16 {
        self.inner.port
    }

    /// Changes the transfer caps (bytes/second, 0 = unlimited).
    pub fn set_rates(&self, upload: i64, download: i64) {
        self.inner.up.set_rate(upload);
        self.inner.down.set_rate(download);
    }

    pub fn torrent(&self, ih: &InfoHash) -> Option<Torrent> {
        self.inner.torrent(ih)
    }

    pub fn torrents(&self) -> Vec<Torrent> {
        self.inner.table().torrents.values().cloned().collect()
    }

    /// Registers and (unless `spec.paused`) starts a torrent.
    pub fn add(&self, spec: Spec) -> Result<Torrent, Error> {
        let paused = spec.paused;
        let t = Torrent::new(self.inner.clone(), spec)?;
        {
            let mut tb = self.inner.table();
            if tb.closed {
                return Err(Error::Closed);
            }
            if tb.torrents.contains_key(&t.info_hash()) {
                return Err(Error::Exists);
            }
            tb.torrents.insert(t.info_hash(), t.clone());
        }
        if !paused {
            t.resume();
        }
        Ok(t)
    }

    /// Stops every torrent and the listener. Idempotent; blocks, so it
    /// must not run on an async task.
    pub fn close(&self) {
        let ts: Vec<Torrent> = {
            let mut tb = self.inner.table();
            if tb.closed {
                return;
            }
            tb.closed = true;
            tb.torrents.drain().map(|(_, t)| t).collect()
        };
        for t in ts {
            t.stop(false);
        }
        self.inner.cancel.cancel();
        self.inner.wg.wait(Duration::from_secs(5));
        if let Some(rt) = self.inner.rt.lock().unwrap_or_else(|e| e.into_inner()).take() {
            rt.shutdown_background();
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.close();
    }
}

async fn accept_loop(c: Arc<ClientInner>, ln: tokio::net::TcpListener) {
    loop {
        let res = tokio::select! {
            _ = c.cancel.cancelled() => return,
            r = ln.accept() => r,
        };
        match res {
            Ok((conn, addr)) => {
                let c2 = c.clone();
                c.handle.spawn(async move { handle_inbound(c2, conn, addr).await });
            }
            Err(e) => {
                tracing::warn!(err = %e, "torrent accept failed");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

async fn handle_inbound(c: Arc<ClientInner>, conn: tokio::net::TcpStream, addr: SocketAddr) {
    let _ = conn.set_nodelay(true);
    let id = c.cfg.peer_id;
    let c2 = c.clone();
    let hs = async move {
        let (r, mut w) = conn.into_split();
        let mut r = tokio::io::BufReader::with_capacity(64 << 10, r);
        let hs = read_handshake(&mut r).await.ok()?;
        let t = c2.torrent(&hs.info_hash)?;
        if hs.peer_id == id {
            return None;
        }
        tokio::io::AsyncWriteExt::write_all(&mut w, &handshake_bytes(&hs.info_hash, &id)).await.ok()?;
        Some((t, r, w, hs))
    };
    if let Ok(Some((t, r, w, hs))) = tokio::time::timeout(c.cfg.dial_timeout, hs).await {
        t.attach(r, w, hs, Some(SocketAddr::new(addr.ip().to_canonical(), addr.port())));
    }
}
