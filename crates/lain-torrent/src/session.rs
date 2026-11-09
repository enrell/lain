//! One swarm membership: state, the check, announce/dial/tick loops and
//! the blocking public handle.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::sync::{Notify, watch};

use crate::client::ClientInner;
use crate::metainfo::valid_tracker_url;
use crate::peer::{Peer, PieceState};
use crate::storage::Storage;
use crate::sync::{Cancel, WaitGroup};
use crate::tracker::{self, AnnounceRequest, EVENT_COMPLETED, EVENT_NONE, EVENT_STARTED, EVENT_STOPPED};
use crate::wire::BLOCK_SIZE;
use crate::{Bitfield, Error, Info, InfoHash, MetaInfo};

/// A torrent's lifecycle position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Metadata,
    Checking,
    Downloading,
    Seeding,
    Paused,
    Error,
}

impl State {
    /// Go's names: metadata, checking, downloading, seeding, paused, error.
    pub fn as_str(&self) -> &'static str {
        match self {
            State::Metadata => "metadata",
            State::Checking => "checking",
            State::Downloading => "downloading",
            State::Seeding => "seeding",
            State::Paused => "paused",
            State::Error => "error",
        }
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A callback fired once, on the blocking pool.
pub type Callback = Arc<dyn Fn(&Torrent) + Send + Sync>;

/// A torrent to add: from a `.torrent` (`meta_info`) or a magnet
/// (`info_hash` plus trackers; metadata is fetched from peers).
#[derive(Clone, Default)]
pub struct Spec {
    pub meta_info: Option<MetaInfo>,
    pub info_hash: Option<InfoHash>,
    pub name: String,
    pub trackers: Vec<String>,
    /// Dialed in addition to tracker results.
    pub peers: Vec<SocketAddr>,
    /// Receives the torrent's files.
    pub dir: PathBuf,
    /// A saved resume bitfield, trusted only when every file is already
    /// on disk at full size; otherwise present data is rechecked.
    pub have: Option<Bitfield>,
    pub paused: bool,
    /// Runs once, on its own thread, after the last piece verifies.
    pub on_complete: Option<Callback>,
    /// Runs once when a magnet's info dictionary arrives.
    pub on_metadata: Option<Callback>,
}

/// A torrent file on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    /// Absolute.
    pub path: PathBuf,
    pub rel: String,
    pub length: i64,
}

/// A snapshot for the queue view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stats {
    pub info_hash: String,
    pub name: String,
    pub state: State,
    pub error: String,
    pub size: i64,
    pub completed: i64,
    pub downloaded: i64,
    pub uploaded: i64,
    pub peers: usize,
    pub seeders: i64,
    pub leechers: i64,
    pub down_rate: i64,
    pub up_rate: i64,
    pub pieces: usize,
    pub have: usize,
}

/// Mutable torrent state, under one lock (Go's `t.mu`).
pub(crate) struct Core {
    pub name: String,
    pub info: Option<Arc<Info>>,
    pub st: Option<Arc<Storage>>,
    pub trackers: Vec<String>,
    pub have: Bitfield,
    pub pieces: Vec<PieceState>,
    /// Pieces with requested or received blocks.
    pub active: BTreeSet<usize>,
    pub avail: Vec<i64>,
    pub peers: HashMap<u64, Peer>,
    /// Earliest time an address may be dialed (`None`: now); failures
    /// double the wait (5 s up to 5 min), a connection resets it.
    pub known: HashMap<SocketAddr, Option<Instant>>,
    pub backoff: HashMap<SocketAddr, Duration>,
    pub state: State,
    pub err: String,
    pub downloaded: i64,
    pub uploaded: i64,
    pub seeders: i64,
    pub leechers: i64,
    pub completed: bool,
    pub resume: Option<Bitfield>,
    // magnet metadata assembly
    pub meta_size: usize,
    pub meta_pieces: Vec<Option<Vec<u8>>>,
    pub meta_asked: HashMap<usize, Instant>,
    // rates sampled by the ticker
    last_down: i64,
    last_up: i64,
    down_rate: i64,
    up_rate: i64,
    pub running: bool,
    pub cancel: Arc<Cancel>,
    announce_now: Option<watch::Sender<u64>>,
}

pub(crate) struct TorrentInner {
    pub c: Arc<ClientInner>,
    pub ih: InfoHash,
    dir: PathBuf,
    on_complete: Option<Callback>,
    on_metadata: Option<Callback>,
    pub core: Mutex<Core>,
    pub wg: WaitGroup,
    new_peer: Notify,
    pub next_peer: AtomicU64,
}

/// A cheap handle on one swarm membership.
#[derive(Clone)]
pub struct Torrent(pub(crate) Arc<TorrentInner>);

impl fmt::Debug for Torrent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Torrent({})", self.0.ih.hex())
    }
}

impl Core {
    /// Installs the info dictionary: storage, piece tables.
    pub fn set_info(&mut self, dir: &std::path::Path, info: Arc<Info>) {
        if self.name.is_empty() {
            self.name = info.name.clone();
        }
        let n = info.num_pieces();
        self.st = Some(Arc::new(Storage::new(dir.to_path_buf(), info.clone())));
        self.have = Bitfield::new(n);
        self.avail = vec![0; n];
        self.pieces =
            (0..n).map(|i| PieceState::new((info.piece_size(i) as usize).div_ceil(BLOCK_SIZE).max(1))).collect();
        for p in self.peers.values_mut() {
            let mut bf = Bitfield::new(n);
            if let Some(old) = &p.bf {
                // Haves before the metadata may have grown the set; trim.
                for i in 0..n {
                    if old.has(i) {
                        bf.set(i);
                    }
                }
            }
            for i in 0..n {
                if bf.has(i) {
                    self.avail[i] += 1;
                }
            }
            p.bf = Some(bf);
        }
        self.info = Some(info);
    }

    fn announce_req(&self, c: &ClientInner, ih: InfoHash, event: &str) -> AnnounceRequest {
        let left = match &self.info {
            Some(info) => {
                info.length
                    - (0..info.num_pieces()).filter(|&i| self.have.has(i)).map(|i| info.piece_size(i)).sum::<i64>()
            }
            None => 1, // unknown size: still leeching
        };
        AnnounceRequest {
            info_hash: ih,
            peer_id: c.cfg.peer_id,
            port: c.port,
            uploaded: self.uploaded,
            downloaded: self.downloaded,
            left,
            event: event.to_string(),
            num_want: 50,
        }
    }
}

impl Torrent {
    pub(crate) fn new(c: Arc<ClientInner>, s: Spec) -> Result<Torrent, Error> {
        if s.dir.as_os_str().is_empty() {
            return Err(Error::Spec("spec needs a directory".into()));
        }
        let mut trackers = s.trackers.clone();
        let mut name = s.name.clone();
        let ih = match &s.meta_info {
            Some(mi) => {
                trackers = mi.trackers.iter().chain(s.trackers.iter()).cloned().collect();
                if name.is_empty() {
                    name = mi.info.name.clone();
                }
                mi.info_hash
            }
            None => match s.info_hash {
                Some(h) if h != InfoHash::default() => h,
                _ => return Err(Error::Spec("spec needs metainfo or an info hash".into())),
            },
        };
        let mut seen = std::collections::HashSet::new();
        trackers.retain(|t| valid_tracker_url(t) && seen.insert(t.clone()));
        let mut core = Core {
            name,
            info: None,
            st: None,
            trackers,
            have: Bitfield::default(),
            pieces: Vec::new(),
            active: BTreeSet::new(),
            avail: Vec::new(),
            peers: HashMap::new(),
            known: s.peers.iter().map(|p| (*p, None)).collect(),
            backoff: HashMap::new(),
            state: State::Paused,
            err: String::new(),
            downloaded: 0,
            uploaded: 0,
            seeders: 0,
            leechers: 0,
            completed: false,
            resume: s.have.clone(),
            meta_size: 0,
            meta_pieces: Vec::new(),
            meta_asked: HashMap::new(),
            last_down: 0,
            last_up: 0,
            down_rate: 0,
            up_rate: 0,
            running: false,
            cancel: Cancel::new(),
            announce_now: None,
        };
        if let Some(mi) = &s.meta_info {
            core.set_info(&s.dir, mi.info.clone());
        }
        Ok(Torrent(Arc::new(TorrentInner {
            c,
            ih,
            dir: s.dir,
            on_complete: s.on_complete,
            on_metadata: s.on_metadata,
            core: Mutex::new(core),
            wg: WaitGroup::default(),
            new_peer: Notify::new(),
            next_peer: AtomicU64::new(1),
        })))
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, Core> {
        self.0.core.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn dir(&self) -> &std::path::Path {
        &self.0.dir
    }

    pub fn info_hash(&self) -> InfoHash {
        self.0.ih
    }

    /// The metainfo once known (`None` while a magnet fetches it).
    pub fn info(&self) -> Option<Arc<Info>> {
        self.lock().info.clone()
    }

    /// Spawns a task counted by the torrent's wait group.
    pub(crate) fn spawn<F>(&self, fut: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let g = self.0.wg.add();
        self.0.c.handle.spawn(async move {
            let _g = g;
            fut.await;
        });
    }

    /// Starts (or restarts) the torrent.
    pub fn resume(&self) {
        let mut c = self.lock();
        if c.running {
            return;
        }
        c.running = true;
        c.err.clear();
        c.cancel = Cancel::new();
        let (tx, _) = watch::channel(0u64);
        c.announce_now = Some(tx);
        let need_check = c.info.is_some();
        c.state = if need_check { State::Checking } else { State::Metadata };
        let cancel = c.cancel.clone();
        drop(c);
        let t = self.clone();
        self.spawn(async move {
            if need_check {
                let t2 = t.clone();
                let _ = tokio::task::spawn_blocking(move || t2.check()).await;
            }
            if cancel.is_cancelled() {
                return;
            }
            let (trackers, rxs) = {
                let c = t.lock();
                let rxs: Vec<_> =
                    c.trackers.iter().filter_map(|_| c.announce_now.as_ref().map(|tx| tx.subscribe())).collect();
                (c.trackers.clone(), rxs)
            };
            for (tr, rx) in trackers.into_iter().zip(rxs) {
                let t2 = t.clone();
                let cancel = cancel.clone();
                t.spawn(async move { t2.announce_loop(tr, rx, cancel).await });
            }
            let (t2, c2) = (t.clone(), cancel.clone());
            t.spawn(async move { t2.dial_loop(c2).await });
            let (t2, c2) = (t.clone(), cancel.clone());
            t.spawn(async move { t2.tick_loop(c2).await });
        });
    }

    /// Establishes which pieces are on disk (blocking).
    fn check(&self) {
        let (st, info, resume, cancel) = {
            let c = self.lock();
            let (Some(st), Some(info)) = (c.st.clone(), c.info.clone()) else { return };
            (st, info, c.resume.clone(), c.cancel.clone())
        };
        let n = info.num_pieces();
        let mut have = Bitfield::new(n);
        match resume {
            Some(r) if r.valid(n) && st.complete() => have = r,
            _ => {
                for i in 0..n {
                    if cancel.is_cancelled() {
                        return;
                    }
                    if st.piece_on_disk(i) && st.verify(i).unwrap_or(false) {
                        have.set(i);
                    }
                }
            }
        }
        let done = {
            let mut c = self.lock();
            c.completed = have.count(n) == n;
            c.have = have;
            c.state = if c.completed { State::Seeding } else { State::Downloading };
            c.completed
        };
        if done {
            self.finalize(false);
        }
    }

    /// Disconnects every peer and tells trackers we stopped.
    pub fn pause(&self) {
        self.stop(true);
    }

    pub(crate) fn stop(&self, announce_stopped: bool) {
        let quits: Vec<Arc<Cancel>> = {
            let mut c = self.lock();
            if !c.running {
                return;
            }
            c.running = false;
            c.cancel.cancel();
            c.peers.values().map(|p| p.quit.clone()).collect()
        };
        for q in quits {
            q.cancel();
        }
        self.0.wg.wait(Duration::from_secs(30));
        let (trackers, req, has_info, st) = {
            let mut c = self.lock();
            if c.state != State::Error {
                c.state = State::Paused;
            }
            c.announce_now = None;
            (c.trackers.clone(), c.announce_req(&self.0.c, self.0.ih, EVENT_STOPPED), c.info.is_some(), c.st.clone())
        };
        if announce_stopped && has_info && !trackers.is_empty() {
            let (tx, rx) = std::sync::mpsc::channel();
            let http = self.0.c.cfg_http();
            self.0.c.handle.spawn(async move {
                let all = async {
                    for tr in &trackers {
                        let _ = tracker::announce(&http, tr, &req, Duration::from_secs(3)).await;
                    }
                };
                let _ = tokio::time::timeout(Duration::from_secs(3), all).await;
                let _ = tx.send(());
            });
            let _ = rx.recv_timeout(Duration::from_secs(4));
        }
        if let Some(st) = st {
            st.close();
        }
    }

    /// Stops the torrent, forgets it and optionally deletes its data.
    pub fn remove(&self, delete_data: bool) -> Result<(), Error> {
        self.stop(true);
        self.0.c.forget(self);
        let st = self.lock().st.clone();
        match st {
            Some(st) if delete_data => st.remove().map_err(Error::from),
            _ => Ok(()),
        }
    }

    /// The torrent's files (empty until metadata is known).
    pub fn files(&self) -> Vec<FileInfo> {
        let c = self.lock();
        let (Some(info), Some(st)) = (&c.info, &c.st) else { return Vec::new() };
        info.files
            .iter()
            .enumerate()
            .map(|(i, f)| FileInfo { path: st.path(i), rel: f.rel_path(), length: f.length })
            .collect()
    }

    pub fn stats(&self) -> Stats {
        let c = self.lock();
        let mut s = Stats {
            info_hash: self.0.ih.hex(),
            name: c.name.clone(),
            state: c.state,
            error: c.err.clone(),
            size: 0,
            completed: 0,
            downloaded: c.downloaded,
            uploaded: c.uploaded,
            peers: c.peers.len(),
            seeders: c.seeders,
            leechers: c.leechers,
            down_rate: c.down_rate,
            up_rate: c.up_rate,
            pieces: 0,
            have: 0,
        };
        if let Some(info) = &c.info {
            s.size = info.length;
            s.pieces = info.num_pieces();
            for i in 0..s.pieces {
                if c.have.has(i) {
                    s.have += 1;
                    s.completed += info.piece_size(i);
                }
            }
        }
        s
    }

    /// A copy of the verified piece set (for resume).
    pub fn bitfield(&self) -> Bitfield {
        self.lock().have.clone()
    }

    /// Dials extra peers.
    pub fn add_peers(&self, peers: &[SocketAddr]) {
        {
            let mut c = self.lock();
            for p in peers {
                c.known.entry(*p).or_insert(None);
            }
        }
        self.poke();
    }

    pub(crate) fn poke(&self) {
        self.0.new_peer.notify_one();
    }

    pub(crate) fn fail(&self, msg: &str) {
        {
            let mut c = self.lock();
            c.state = State::Error;
            c.err = msg.to_string();
        }
        tracing::warn!(torrent = %self.0.ih, err = msg, "torrent failed");
        let t = self.clone();
        self.0.c.handle.spawn_blocking(move || t.stop(true));
    }

    async fn announce_loop(self, tracker_url: String, mut now: watch::Receiver<u64>, cancel: Arc<Cancel>) {
        let mut event = EVENT_STARTED;
        // A torrent that starts complete never sends "completed":
        // trackers only want it for a download finished in this session.
        let mut sent_completed = self.lock().completed;
        let mut backoff = Duration::from_secs(15);
        let http = self.0.c.cfg_http();
        loop {
            let req = {
                let c = self.lock();
                if event == EVENT_NONE && c.completed && !sent_completed {
                    event = EVENT_COMPLETED;
                }
                c.announce_req(&self.0.c, self.0.ih, event)
            };
            let res = tokio::select! {
                _ = cancel.cancelled() => return,
                r = tracker::announce(&http, &tracker_url, &req, Duration::from_secs(30)) => r,
            };
            let mut wait = backoff;
            match res {
                Err(e) => {
                    tracing::info!(torrent = %self.0.ih, err = %e, "announce failed");
                    backoff = (backoff * 2).min(Duration::from_secs(30 * 60));
                }
                Ok(r) => {
                    backoff = Duration::from_secs(15);
                    if event == EVENT_COMPLETED {
                        sent_completed = true;
                    }
                    event = EVENT_NONE;
                    wait = r.interval.max(self.0.c.cfg.min_announce_interval);
                    {
                        let mut c = self.lock();
                        c.seeders = r.seeders;
                        c.leechers = r.leechers;
                    }
                    self.add_peers(&r.peers);
                }
            }
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(wait) => {}
                r = now.changed() => if r.is_err() { return },
            }
        }
    }

    /// Connects to known peers while below the peer limit.
    async fn dial_loop(self, cancel: Arc<Cancel>) {
        loop {
            self.dial_some();
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                _ = self.0.new_peer.notified() => {}
            }
        }
    }

    fn dial_some(&self) {
        let targets: Vec<SocketAddr> = {
            let mut c = self.lock();
            let slots = self.0.c.cfg.max_peers_per_torrent.saturating_sub(c.peers.len());
            let connected: std::collections::HashSet<SocketAddr> = c.peers.values().filter_map(|p| p.addr).collect();
            let now = Instant::now();
            let ready: Vec<SocketAddr> = c
                .known
                .iter()
                .filter(|(a, next)| !connected.contains(a) && next.is_none_or(|n| now >= n))
                .map(|(a, _)| *a)
                .take(slots)
                .collect();
            for a in &ready {
                let wait = c.backoff.get(a).copied().unwrap_or(Duration::from_secs(5));
                c.known.insert(*a, Some(now + wait));
                c.backoff.insert(*a, (wait * 2).min(Duration::from_secs(300)));
            }
            ready
        };
        for a in targets {
            let t = self.clone();
            self.spawn(async move { t.dial(a).await });
        }
    }

    async fn dial(self, a: SocketAddr) {
        let cancel = self.lock().cancel.clone();
        let cfg = &self.0.c.cfg;
        let (ih, id) = (self.0.ih, cfg.peer_id);
        let hs = async {
            let stream = tokio::net::TcpStream::connect(a).await.ok()?;
            let _ = stream.set_nodelay(true);
            let (r, mut w) = stream.into_split();
            tokio::io::AsyncWriteExt::write_all(&mut w, &crate::wire::handshake_bytes(&ih, &id)).await.ok()?;
            let mut r = tokio::io::BufReader::with_capacity(64 << 10, r);
            let hs = crate::wire::read_handshake(&mut r).await.ok()?;
            (hs.info_hash == ih && hs.peer_id != id).then_some((r, w, hs))
        };
        let got = tokio::select! {
            _ = cancel.cancelled() => None,
            r = tokio::time::timeout(cfg.dial_timeout, hs) => r.ok().flatten(),
        };
        if let Some((r, w, hs)) = got {
            self.attach(r, w, hs, Some(a));
        }
    }

    /// Timeouts, choking and rate sampling.
    async fn tick_loop(self, cancel: Arc<Cancel>) {
        let mut tick =
            tokio::time::interval_at(tokio::time::Instant::now() + Duration::from_secs(1), Duration::from_secs(1));
        let mut n = 0u64;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tick.tick() => {}
            }
            n += 1;
            let mut c = self.lock();
            c.down_rate = c.downloaded - c.last_down;
            c.last_down = c.downloaded;
            c.up_rate = c.uploaded - c.last_up;
            c.last_up = c.uploaded;
            c.expire_requests(self.0.c.cfg.request_timeout);
            if n.is_multiple_of(10) {
                c.rechoke(true, self.0.c.cfg.upload_slots);
            }
            c.expire_meta();
            let ids: Vec<u64> = c.peers.keys().copied().collect();
            for id in ids {
                c.fill(id);
            }
        }
    }

    /// After the last piece: creates never-written (empty) files and
    /// fires `on_complete` once.
    pub(crate) fn finalize(&self, fire: bool) {
        let (info, st) = {
            let c = self.lock();
            (c.info.clone(), c.st.clone())
        };
        if let (Some(info), Some(st)) = (info, st) {
            for (i, f) in info.files.iter().enumerate() {
                if f.length != 0 {
                    continue;
                }
                let p = st.path(i);
                if let Some(parent) = p.parent()
                    && std::fs::create_dir_all(parent).is_ok()
                {
                    let _ = std::fs::OpenOptions::new().create(true).append(true).open(&p);
                }
            }
        }
        if fire {
            if let Some(tx) = &self.lock().announce_now {
                tx.send_modify(|v| *v += 1);
            }
            self.callback(self.0.on_complete.clone());
        }
    }

    pub(crate) fn metadata_arrived(&self) {
        self.callback(self.0.on_metadata.clone());
    }

    fn callback(&self, cb: Option<Callback>) {
        if let Some(cb) = cb {
            let t = self.clone();
            self.0.c.handle.spawn_blocking(move || cb(&t));
        }
    }
}
