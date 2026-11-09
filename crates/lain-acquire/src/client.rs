//! The download client the manager drives (A-3): the in-house engine
//! (`lain-torrent`) with its resume records in `acq_torrents`. A
//! qBittorrent or Transmission adapter could implement [`DownloadClient`]
//! later without touching import.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use lain_torrent::{Bitfield, Client, Config, InfoHash, Spec, Torrent, parse_magnet, parse_metainfo};

use crate::errf;
use crate::error::{CODE_FETCH, CODE_IMPORT, CODE_INVALID, CODE_NOT_FOUND, CODE_STATE, CODE_UNSUPPORTED, Result};
use crate::importer::File;
use crate::store::{AcqStore, TorrentState};

/// A client's view of one torrent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientStatus {
    /// metadata, checking, downloading, seeding, paused, error
    pub state: String,
    pub size: i64,
    pub completed: i64,
    pub downloaded: i64,
    pub uploaded: i64,
    pub peers: i64,
    pub down_rate: i64,
    pub up_rate: i64,
    pub error: String,
}

/// A `.torrent` body or a magnet.
#[derive(Debug, Clone, Default)]
pub struct AddRequest {
    pub torrent: Vec<u8>,
    pub magnet: String,
    pub dir: PathBuf,
    pub paused: bool,
}

pub trait DownloadClient: Send + Sync {
    fn name(&self) -> &str;
    /// The info hash (hex) and the size, 0 while a magnet's metadata is
    /// unknown.
    fn add(&self, req: AddRequest) -> Result<(String, i64)>;
    fn status(&self, ih: &str) -> Option<ClientStatus>;
    fn files(&self, ih: &str) -> Vec<File>;
    fn pause(&self, ih: &str) -> Result<()>;
    fn resume(&self, ih: &str) -> Result<()>;
    fn remove(&self, ih: &str, delete_data: bool) -> Result<()>;
    fn set_rates(&self, up_bps: i64, down_bps: i64);
    /// Persists resume state (periodically and on close).
    fn checkpoint(&self);
    fn port(&self) -> u16;
    fn close(&self);
}

pub type MetadataHook = Arc<dyn Fn(&str, i64) + Send + Sync>;
pub type CompleteHook = Arc<dyn Fn(&str) + Send + Sync>;

/// Events a client reports back.
#[derive(Clone, Default)]
pub struct ClientEvents {
    /// A magnet's size became known.
    pub on_metadata: Option<MetadataHook>,
    pub on_complete: Option<CompleteHook>,
}

#[derive(Clone, Default)]
pub struct NativeConfig {
    /// 0 picks a free port at each start.
    pub listen_port: u16,
    pub listen_host: String,
    pub max_peers: usize,
    pub upload_bps: i64,
    pub download_bps: i64,
    /// Tests override timings.
    pub engine: Option<Config>,
}

/// The in-house engine as a [`DownloadClient`].
pub struct Native {
    st: AcqStore,
    c: Client,
    events: ClientEvents,
    mu: Mutex<()>,
}

impl Native {
    /// Starts the engine and restores persisted torrents.
    pub fn new(st: AcqStore, cfg: NativeConfig, events: ClientEvents) -> Result<Arc<Native>> {
        let mut ec = cfg.engine.clone().unwrap_or_default();
        // Always listen: peers must reach a seeder, and trackers reject a
        // port of 0. Port 0 lets the OS pick a free one.
        let ip: IpAddr = if cfg.listen_host.is_empty() {
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        } else {
            cfg.listen_host.parse().map_err(|_| errf!(CODE_INVALID, "bad listen host {}", cfg.listen_host))?
        };
        ec.listen_addr = Some(SocketAddr::new(ip, cfg.listen_port).to_string());
        if cfg.max_peers > 0 {
            ec.max_peers_per_torrent = cfg.max_peers;
        }
        ec.upload_rate = cfg.upload_bps.max(0);
        ec.download_rate = cfg.download_bps.max(0);
        let c = Client::new(ec).map_err(|e| errf!(CODE_STATE, "{e}"))?;
        let n = Arc::new(Native { st, c, events, mu: Mutex::new(()) });
        for (ih, ts) in n.st.torrents() {
            if let Err(e) = n.restore(&ih, &ts) {
                tracing::warn!(torrent = %ih, err = %e, "torrent not restored");
            }
        }
        Ok(n)
    }

    fn spec(self: &Arc<Self>, dir: PathBuf, have: Option<Bitfield>, paused: bool) -> Spec {
        let weak = Arc::downgrade(self);
        let weak2 = weak.clone();
        Spec {
            dir,
            have,
            paused,
            on_complete: Some(Arc::new(move |t: &Torrent| {
                if let Some(n) = weak.upgrade() {
                    n.save(t);
                    if let Some(f) = &n.events.on_complete {
                        f(&t.info_hash().hex());
                    }
                }
            })),
            on_metadata: Some(Arc::new(move |t: &Torrent| {
                if let Some(n) = weak2.upgrade() {
                    n.save(t);
                    if let (Some(f), Some(info)) = (&n.events.on_metadata, t.info()) {
                        f(&t.info_hash().hex(), info.length);
                    }
                }
            })),
            ..Spec::default()
        }
    }

    fn restore(self: &Arc<Self>, ih: &str, ts: &TorrentState) -> Result<()> {
        let h = InfoHash::parse(ih).map_err(|e| errf!(CODE_INVALID, "{e}"))?;
        let dir = PathBuf::from(&ts.dir);
        if ts.metainfo.is_empty() {
            let m =
                parse_magnet(&ts.magnet).map_err(|_| errf!(CODE_INVALID, "no metainfo or magnet saved for {ih}"))?;
            if m.info_hash != h {
                return Err(errf!(CODE_INVALID, "no metainfo or magnet saved for {ih}"));
            }
            let mut s = self.spec(dir, None, ts.paused);
            s.info_hash = Some(m.info_hash);
            s.name = m.name;
            s.trackers = m.trackers;
            self.c.add(s).map_err(|e| errf!(CODE_STATE, "{e}"))?;
            return Ok(());
        }
        let mi = parse_metainfo(&ts.metainfo).map_err(|_| errf!(CODE_INVALID, "saved metainfo invalid"))?;
        if mi.info_hash != h {
            return Err(errf!(CODE_INVALID, "saved metainfo invalid"));
        }
        let have = (!ts.have.is_empty()).then(|| Bitfield(ts.have.clone()));
        let mut s = self.spec(dir, have, ts.paused);
        s.meta_info = Some(mi);
        self.c.add(s).map_err(|e| errf!(CODE_STATE, "{e}"))?;
        Ok(())
    }

    /// Persists metainfo (synthesized from the info dict for magnets) and
    /// the verified-piece bitfield.
    fn save(&self, t: &Torrent) {
        let _g = self.mu.lock().unwrap_or_else(|e| e.into_inner());
        let ih = t.info_hash().hex();
        let Some(mut ts) = self.st.torrent(&ih) else { return };
        if ts.metainfo.is_empty()
            && let Some(info) = t.info()
        {
            ts.metainfo = lain_torrent::torrent_file(&info.raw);
        }
        ts.have = t.bitfield().0;
        let _ = self.st.put_torrent(&ih, &ts);
    }

    fn get(&self, ih: &str) -> Result<Torrent> {
        let h = InfoHash::parse(ih).map_err(|_| errf!(CODE_NOT_FOUND, "unknown torrent"))?;
        self.c.torrent(&h).ok_or_else(|| errf!(CODE_NOT_FOUND, "unknown torrent"))
    }

    fn set_paused(&self, ih: &str, paused: bool) {
        let _g = self.mu.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(mut ts) = self.st.torrent(ih) {
            ts.paused = paused;
            let _ = self.st.put_torrent(ih, &ts);
        }
    }

    /// Adds through an `Arc` (the hooks hold a weak reference).
    pub fn add_arc(self: &Arc<Self>, req: AddRequest) -> Result<(String, i64)> {
        let mut s = self.spec(req.dir.clone(), None, req.paused);
        let mut raw = Vec::new();
        if !req.torrent.is_empty() {
            let mi = parse_metainfo(&req.torrent).map_err(|e| errf!(CODE_FETCH, "not a valid .torrent: {e}"))?;
            s.meta_info = Some(mi);
            raw = req.torrent.clone();
        } else if !req.magnet.is_empty() {
            let m = parse_magnet(&req.magnet).map_err(|e| errf!(CODE_INVALID, "bad magnet: {e}"))?;
            if m.trackers.is_empty() {
                // No DHT (A-2): a magnet needs a tracker to find peers.
                return Err(errf!(
                    CODE_UNSUPPORTED,
                    "this magnet has no tracker; Lain's engine has no DHT yet — use the .torrent link"
                ));
            }
            s.info_hash = Some(m.info_hash);
            s.name = m.name;
            s.trackers = m.trackers;
        } else {
            return Err(errf!(CODE_INVALID, "a .torrent or a magnet is required"));
        }
        let ih = match &s.meta_info {
            Some(mi) => mi.info_hash,
            None => s.info_hash.unwrap_or_default(),
        };
        if self.c.torrent(&ih).is_some() {
            return Err(errf!(CODE_STATE, "this torrent is already in the queue"));
        }
        let mut ts = TorrentState {
            metainfo: raw,
            dir: req.dir.to_string_lossy().into(),
            paused: req.paused,
            ..Default::default()
        };
        if ts.metainfo.is_empty() {
            ts.magnet = req.magnet.clone(); // until metadata arrives
        }
        self.st.put_torrent(&ih.hex(), &ts)?;
        let t = match self.c.add(s) {
            Ok(t) => t,
            Err(e) => {
                self.st.delete_torrent(&ih.hex());
                return Err(errf!(CODE_STATE, "{e}"));
            }
        };
        let size = t.info().map(|i| i.length).unwrap_or(0);
        Ok((ih.hex(), size))
    }
}

/// The client as the manager holds it.
pub struct NativeClient(pub Arc<Native>);

impl DownloadClient for NativeClient {
    fn name(&self) -> &str {
        "native"
    }

    fn add(&self, req: AddRequest) -> Result<(String, i64)> {
        self.0.add_arc(req)
    }

    fn status(&self, ih: &str) -> Option<ClientStatus> {
        let s = self.0.get(ih).ok()?.stats();
        Some(ClientStatus {
            state: s.state.as_str().into(),
            size: s.size,
            completed: s.completed,
            downloaded: s.downloaded,
            uploaded: s.uploaded,
            peers: s.peers as i64,
            down_rate: s.down_rate,
            up_rate: s.up_rate,
            error: s.error.clone(),
        })
    }

    fn files(&self, ih: &str) -> Vec<File> {
        let Ok(t) = self.0.get(ih) else { return Vec::new() };
        t.files().into_iter().map(|f| File { path: f.path, rel: f.rel, length: f.length }).collect()
    }

    fn pause(&self, ih: &str) -> Result<()> {
        let t = self.0.get(ih)?;
        t.pause();
        self.0.save(&t);
        self.0.set_paused(ih, true);
        Ok(())
    }

    fn resume(&self, ih: &str) -> Result<()> {
        let t = self.0.get(ih)?;
        self.0.set_paused(ih, false);
        t.resume();
        Ok(())
    }

    fn remove(&self, ih: &str, delete_data: bool) -> Result<()> {
        if let Ok(t) = self.0.get(ih) {
            t.remove(delete_data).map_err(|e| errf!(CODE_IMPORT, "could not delete torrent data: {e}"))?;
        }
        self.0.st.delete_torrent(ih);
        Ok(())
    }

    fn set_rates(&self, up: i64, down: i64) {
        self.0.c.set_rates(up.max(0), down.max(0));
    }

    fn checkpoint(&self) {
        for t in self.0.c.torrents() {
            self.0.save(&t);
        }
    }

    fn port(&self) -> u16 {
        self.0.c.port()
    }

    fn close(&self) {
        self.checkpoint();
        self.0.c.close();
    }
}
