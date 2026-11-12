//! The peer wire protocol: per-peer read and write tasks, piece
//! picking, choking, block transfer and ut_metadata (BEP 9/10).

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc;

use crate::bencode::{self, Value};
use crate::error::wire;
use crate::metainfo::sha1;
use crate::session::{Core, State, Torrent};
use crate::sync::Cancel;
use crate::wire::*;
use crate::{Bitfield, Error, parse_info};

const MAX_INFLIGHT: usize = 32;
const MAX_UPLOAD_QUEUE: usize = 256;
const MAX_METADATA_SIZE: i64 = 8 << 20;
const META_PIECE_SIZE: usize = 16 << 10;
/// The id peers use to send us ut_metadata.
const LOCAL_UT_METADATA: u8 = 1;
const ENDGAME_BLOCKS: usize = 32;

type BlockKey = (usize, usize);

pub(crate) struct PieceState {
    got: Vec<bool>,
    got_n: usize,
    /// Block index -> requesting peers.
    reqs: HashMap<usize, Vec<u64>>,
    verifying: bool,
    contribute: HashSet<u64>,
}

impl PieceState {
    pub fn new(blocks: usize) -> PieceState {
        PieceState {
            got: vec![false; blocks],
            got_n: 0,
            reqs: HashMap::new(),
            verifying: false,
            contribute: HashSet::new(),
        }
    }
}

pub(crate) struct Peer {
    id: [u8; 20],
    pub addr: Option<SocketAddr>,
    out: mpsc::Sender<Vec<u8>>,
    upload: mpsc::Sender<BlockRef>,
    pub quit: Arc<Cancel>,
    am_choking: bool,
    am_interested: bool,
    peer_choking: bool,
    peer_interested: bool,
    pub bf: Option<Bitfield>,
    inflight: HashMap<BlockKey, Instant>,
    canceled: HashSet<BlockRef>,
    ut_meta: u8,
    unchoked_at: Instant,
    interested_at: Instant,
    bad: u32,
}

impl Peer {
    /// Queues a control frame; a peer that cannot keep up is dropped.
    fn send(&self, b: Vec<u8>) {
        if let Err(mpsc::error::TrySendError::Full(_)) = self.out.try_send(b) {
            self.quit.cancel();
        }
    }

    fn set_choking(&mut self, choke: bool) {
        if self.am_choking == choke {
            return;
        }
        self.am_choking = choke;
        if choke {
            self.send(frame(MSG_CHOKE, &[]));
        } else {
            self.unchoked_at = Instant::now();
            self.send(frame(MSG_UNCHOKE, &[]));
        }
    }
}

fn ext_frame(id: u8, dict: &Value, tail: &[u8]) -> Vec<u8> {
    frame(MSG_EXTENDED, &[&[id], &bencode::encode(dict), tail])
}

impl Core {
    fn block_ref(&self, piece: usize, b: usize) -> BlockRef {
        let begin = b * BLOCK_SIZE;
        let size = self.info.as_ref().map(|i| i.piece_size(piece)).unwrap_or(0);
        BlockRef { piece, begin, length: (BLOCK_SIZE as i64).min(size - begin as i64).max(0) as usize }
    }

    pub fn drop_request(&mut self, pid: u64, k: BlockKey) {
        if let Some(p) = self.peers.get_mut(&pid) {
            p.inflight.remove(&k);
        }
        if self.info.is_none() || k.0 >= self.pieces.len() {
            return;
        }
        let ps = &mut self.pieces[k.0];
        let b = k.1 / BLOCK_SIZE;
        if let Some(reqs) = ps.reqs.get_mut(&b) {
            if let Some(i) = reqs.iter().position(|&q| q == pid) {
                reqs.remove(i);
            }
            if reqs.is_empty() {
                ps.reqs.remove(&b);
            }
        }
    }

    pub fn expire_requests(&mut self, timeout: Duration) {
        let Some(deadline) = Instant::now().checked_sub(timeout) else { return };
        let stale: Vec<(u64, BlockKey)> = self
            .peers
            .iter()
            .flat_map(|(id, p)| p.inflight.iter().filter(|(_, at)| **at < deadline).map(move |(k, _)| (*id, *k)))
            .collect();
        for (id, k) in stale {
            self.drop_request(id, k);
        }
    }

    /// Unchokes up to `slots` interested peers, rotating one slot every
    /// period so newcomers get a turn.
    pub fn rechoke(&mut self, rotate: bool, slots: usize) {
        let mut unchoked = Vec::new();
        let mut waiting = Vec::new();
        for (id, p) in self.peers.iter_mut() {
            if !p.peer_interested {
                if !p.am_choking {
                    p.set_choking(true);
                }
                continue;
            }
            if p.am_choking {
                waiting.push((p.interested_at, *id));
            } else {
                unchoked.push((p.unchoked_at, *id));
            }
        }
        if rotate && !waiting.is_empty() && unchoked.len() >= slots && !unchoked.is_empty() {
            unchoked.sort();
            let (_, id) = unchoked.remove(0);
            if let Some(p) = self.peers.get_mut(&id) {
                p.set_choking(true);
            }
        }
        waiting.sort();
        for (_, id) in waiting {
            if unchoked.len() >= slots {
                break;
            }
            if let Some(p) = self.peers.get_mut(&id) {
                p.set_choking(false);
                unchoked.push((p.unchoked_at, id));
            }
        }
    }

    fn update_interest(&mut self, pid: u64) {
        let Some(p) = self.peers.get(&pid) else { return };
        let want = match (&self.info, &p.bf) {
            (Some(info), Some(bf)) if !self.completed => (0..info.num_pieces()).any(|i| bf.has(i) && !self.have.has(i)),
            _ => false,
        };
        let Some(p) = self.peers.get_mut(&pid) else { return };
        if want != p.am_interested {
            p.am_interested = want;
            p.send(frame(if want { MSG_INTERESTED } else { MSG_NOT_INTERESTED }, &[]));
        }
    }

    fn update_interest_all(&mut self) {
        let ids: Vec<u64> = self.peers.keys().copied().collect();
        for id in ids {
            self.update_interest(id);
        }
    }

    fn fill_all(&mut self) {
        let ids: Vec<u64> = self.peers.keys().copied().collect();
        for id in ids {
            self.fill(id);
        }
    }

    /// Keeps the peer's request pipeline full: partial pieces first, then
    /// the rarest piece it has; endgame duplicates the last blocks.
    pub fn fill(&mut self, pid: u64) {
        loop {
            let Some(p) = self.peers.get(&pid) else { return };
            if self.info.is_none() || self.completed || p.peer_choking || !p.am_interested || p.bf.is_none() {
                return;
            }
            if p.inflight.len() >= MAX_INFLIGHT {
                return;
            }
            let Some(r) = self.pick(pid) else { return };
            let k = (r.piece, r.begin);
            self.pieces[r.piece].reqs.entry(r.begin / BLOCK_SIZE).or_default().push(pid);
            self.active.insert(r.piece);
            if let Some(p) = self.peers.get_mut(&pid) {
                p.inflight.insert(k, Instant::now());
                p.send(request_frame(MSG_REQUEST, r));
            }
        }
    }

    fn pick(&self, pid: u64) -> Option<BlockRef> {
        let p = self.peers.get(&pid)?;
        let bf = p.bf.as_ref()?;
        let info = self.info.as_ref()?;
        let free = |i: usize| -> Option<BlockRef> {
            let ps = &self.pieces[i];
            if ps.verifying || self.have.has(i) || !bf.has(i) {
                return None;
            }
            let b = ps.got.iter().enumerate().position(|(b, got)| !got && ps.reqs.get(&b).is_none_or(Vec::is_empty))?;
            Some(self.block_ref(i, b))
        };
        for &i in &self.active {
            if let Some(r) = free(i) {
                return Some(r);
            }
        }
        let mut best: Option<(usize, i64)> = None;
        for i in 0..info.num_pieces() {
            if self.active.contains(&i) || self.have.has(i) || !bf.has(i) {
                continue;
            }
            if best.is_none_or(|(_, a)| self.avail[i] < a) {
                best = Some((i, self.avail[i]));
            }
        }
        if let Some((i, _)) = best {
            return free(i);
        }
        // Endgame: everything is requested; duplicate the remaining
        // blocks this peer is not already fetching.
        let remaining: usize = self.active.iter().map(|&i| self.pieces[i].got.len() - self.pieces[i].got_n).sum();
        if remaining > ENDGAME_BLOCKS {
            return None;
        }
        for &i in &self.active {
            let ps = &self.pieces[i];
            if ps.verifying || !bf.has(i) {
                continue;
            }
            for (b, got) in ps.got.iter().enumerate() {
                if !got && !p.inflight.contains_key(&(i, b * BLOCK_SIZE)) {
                    return Some(self.block_ref(i, b));
                }
            }
        }
        None
    }

    fn peer_has(&mut self, pid: u64, i: usize) -> Result<(), Error> {
        let n = self.info.as_ref().map(|info| info.num_pieces());
        let Some(p) = self.peers.get_mut(&pid) else { return Ok(()) };
        match n {
            Some(n) => {
                if i >= n {
                    return Err(wire(format!("have {i} out of range")));
                }
                let bf = p.bf.get_or_insert_with(|| Bitfield::new(n));
                if !bf.has(i) {
                    bf.set(i);
                    self.avail[i] += 1;
                }
                self.update_interest(pid);
                self.fill(pid);
            }
            None => {
                if i >= 1 << 24 {
                    return Err(wire(format!("have {i} out of range")));
                }
                let bf = p.bf.get_or_insert_with(Bitfield::default);
                while bf.0.len() * 8 <= i {
                    bf.0.push(0);
                }
                bf.set(i);
            }
        }
        Ok(())
    }

    fn request_meta(&mut self) {
        if self.info.is_some() || self.meta_size == 0 {
            return;
        }
        let sources: Vec<u64> = self.peers.iter().filter(|(_, p)| p.ut_meta != 0).map(|(id, _)| *id).collect();
        if sources.is_empty() {
            return;
        }
        let mut n = 0;
        for i in 0..self.meta_pieces.len() {
            if self.meta_pieces[i].is_some() || self.meta_asked.contains_key(&i) {
                continue;
            }
            let q = &self.peers[&sources[n % sources.len()]];
            n += 1;
            let d = Value::dict([("msg_type", Value::Int(0)), ("piece", Value::Int(i as i64))]);
            q.send(ext_frame(q.ut_meta, &d, &[]));
            self.meta_asked.insert(i, Instant::now());
        }
    }

    pub fn expire_meta(&mut self) {
        self.meta_asked.retain(|_, at| at.elapsed() <= Duration::from_secs(20));
        self.request_meta();
    }

    /// Installs the metadata once every piece arrived; true when it did.
    fn try_metadata(&mut self, dir: &std::path::Path, ih: &crate::InfoHash) -> bool {
        if self.meta_pieces.iter().any(Option::is_none) {
            return false;
        }
        let raw: Vec<u8> = self.meta_pieces.iter().flatten().flatten().copied().collect();
        if sha1(&raw) != ih.0 {
            // A peer lied; start over.
            self.meta_pieces = vec![None; self.meta_pieces.len()];
            self.meta_asked.clear();
            return false;
        }
        match parse_info(&raw) {
            Err(e) => {
                self.state = State::Error;
                self.err = format!("metadata: {e}");
                false
            }
            Ok(info) => {
                self.set_info(dir, Arc::new(info));
                self.state = State::Downloading;
                self.meta_pieces.clear();
                self.meta_asked.clear();
                self.update_interest_all();
                self.fill_all();
                true
            }
        }
    }
}

impl Torrent {
    /// Registers a handshaken connection and starts its tasks.
    pub(crate) fn attach(
        &self,
        r: BufReader<OwnedReadHalf>,
        w: OwnedWriteHalf,
        hs: Handshake,
        addr: Option<SocketAddr>,
    ) {
        let max = self.0.c.cfg.max_peers_per_torrent;
        let mut c = self.lock();
        // Peers arriving during the hash check are refused: the bitfield
        // is only sent on attach and would announce pieces still unknown.
        if !c.running || c.state == State::Checking || c.peers.len() >= max {
            return;
        }
        if c.peers.values().any(|q| q.id == hs.peer_id) {
            return;
        }
        let (out_tx, out_rx) = mpsc::channel(512);
        let (up_tx, up_rx) = mpsc::channel(MAX_UPLOAD_QUEUE);
        let now = Instant::now();
        let p = Peer {
            id: hs.peer_id,
            addr,
            out: out_tx,
            upload: up_tx,
            quit: Cancel::new(),
            am_choking: true,
            am_interested: false,
            peer_choking: true,
            peer_interested: false,
            bf: None,
            inflight: HashMap::new(),
            canceled: HashSet::new(),
            ut_meta: 0,
            unchoked_at: now,
            interested_at: now,
            bad: 0,
        };
        if let Some(a) = addr {
            match c.known.entry(a) {
                std::collections::hash_map::Entry::Occupied(_) => {
                    c.backoff.remove(&a); // reachable again: retry quickly if it drops
                }
                std::collections::hash_map::Entry::Vacant(v) => {
                    // Inbound peers' source ports are ephemeral; never redial.
                    v.insert(Some(now + Duration::from_secs(365 * 24 * 3600)));
                }
            }
        }
        if hs.supports_extensions() {
            let mut d = vec![
                ("m", Value::dict([("ut_metadata", Value::Int(LOCAL_UT_METADATA as i64))])),
                ("v", Value::str("Lain")),
                ("reqq", Value::Int(MAX_UPLOAD_QUEUE as i64)),
            ];
            if let Some(info) = &c.info {
                d.push(("metadata_size", Value::Int(info.raw.len() as i64)));
            }
            p.send(ext_frame(0, &Value::dict(d), &[]));
        }
        if let Some(info) = &c.info
            && c.have.count(info.num_pieces()) > 0
        {
            p.send(frame(MSG_BITFIELD, &[c.have.as_bytes()]));
        }
        let quit = p.quit.clone();
        let run = c.cancel.clone();
        let pid = self.0.next_peer.fetch_add(1, Ordering::Relaxed);
        c.peers.insert(pid, p);
        drop(c);
        let t = self.clone();
        let (q2, r2) = (quit.clone(), run.clone());
        self.spawn(async move { t.write_loop(pid, w, out_rx, up_rx, q2, r2).await });
        let t = self.clone();
        self.spawn(async move { t.read_loop(pid, r, quit, run).await });
    }

    async fn write_loop(
        self,
        pid: u64,
        mut w: OwnedWriteHalf,
        mut out: mpsc::Receiver<Vec<u8>>,
        mut up: mpsc::Receiver<BlockRef>,
        quit: Arc<Cancel>,
        run: Arc<Cancel>,
    ) {
        async fn write(w: &mut OwnedWriteHalf, b: &[u8]) -> bool {
            matches!(tokio::time::timeout(Duration::from_secs(60), w.write_all(b)).await, Ok(Ok(())))
        }
        let keep = Duration::from_secs(90);
        let mut keepalive = tokio::time::interval_at(tokio::time::Instant::now() + keep, keep);
        loop {
            tokio::select! {
                _ = quit.cancelled() => break,
                _ = run.cancelled() => break,
                b = out.recv() => match b {
                    Some(b) => if !write(&mut w, &b).await { break },
                    None => break,
                },
                r = up.recv() => {
                    let Some(req) = r else { break };
                    let Some(data) = self.read_block_for_upload(pid, req) else { continue };
                    tokio::select! {
                        _ = quit.cancelled() => break,
                        _ = run.cancelled() => break,
                        _ = self.0.c.up.wait(data.len()) => {}
                    }
                    let f = frame(MSG_PIECE, &[&u32s(&[req.piece as u32, req.begin as u32]), &data]);
                    if !write(&mut w, &f).await {
                        break;
                    }
                    self.lock().uploaded += data.len() as i64;
                }
                _ = keepalive.tick() => if !write(&mut w, &[0, 0, 0, 0]).await { break },
            }
        }
        quit.cancel();
    }

    async fn read_loop(self, pid: u64, mut r: BufReader<OwnedReadHalf>, quit: Arc<Cancel>, run: Arc<Cancel>) {
        loop {
            let limit = {
                let c = self.lock();
                if c.info.is_some() { MAX_MESSAGE.max(c.have.len_bytes() + 1) } else { MAX_MESSAGE }
            };
            let m = tokio::select! {
                _ = quit.cancelled() => break,
                _ = run.cancelled() => break,
                m = tokio::time::timeout(Duration::from_secs(180), read_message(&mut r, limit)) => match m {
                    Ok(Ok(m)) => m,
                    _ => break,
                },
            };
            if let Err(e) = self.handle(pid, m, &quit, &run).await {
                tracing::info!(torrent = %self.0.ih, err = %e, "dropping peer");
                break;
            }
        }
        quit.cancel();
        self.detach(pid);
    }

    /// Forgets a closed peer and releases its requests.
    fn detach(&self, pid: u64) {
        let slots = self.0.c.cfg.upload_slots;
        let mut c = self.lock();
        let Some(p) = c.peers.remove(&pid) else { return };
        p.quit.cancel();
        // drop_request needs the peer gone from `peers` only for its own
        // inflight map, which leaves with it.
        for k in p.inflight.keys() {
            let ps = c.pieces.get_mut(k.0);
            if let Some(ps) = ps
                && let Some(reqs) = ps.reqs.get_mut(&(k.1 / BLOCK_SIZE))
            {
                reqs.retain(|&q| q != pid);
                if reqs.is_empty() {
                    ps.reqs.remove(&(k.1 / BLOCK_SIZE));
                }
            }
        }
        if let (Some(info), Some(bf)) = (c.info.clone(), &p.bf) {
            for (i, avail) in c.avail.iter_mut().enumerate().take(info.num_pieces()) {
                if bf.has(i) {
                    *avail -= 1;
                }
            }
        }
        if !p.am_choking {
            c.rechoke(false, slots);
        }
        drop(c);
        self.poke();
    }

    /// Processes one message from a peer.
    async fn handle(&self, pid: u64, m: Message, quit: &Cancel, run: &Cancel) -> Result<(), Error> {
        let slots = self.0.c.cfg.upload_slots;
        match m.id {
            MSG_CHOKE => {
                let mut c = self.lock();
                let keys: Vec<BlockKey> = match c.peers.get_mut(&pid) {
                    Some(p) => {
                        p.peer_choking = true;
                        p.inflight.keys().copied().collect()
                    }
                    None => return Ok(()),
                };
                for k in keys {
                    c.drop_request(pid, k);
                }
            }
            MSG_UNCHOKE => {
                let mut c = self.lock();
                if let Some(p) = c.peers.get_mut(&pid) {
                    p.peer_choking = false;
                }
                c.fill(pid);
            }
            MSG_INTERESTED => {
                let mut c = self.lock();
                if let Some(p) = c.peers.get_mut(&pid)
                    && !p.peer_interested
                {
                    p.peer_interested = true;
                    p.interested_at = Instant::now();
                    c.rechoke(false, slots);
                }
            }
            MSG_NOT_INTERESTED => {
                let mut c = self.lock();
                if let Some(p) = c.peers.get_mut(&pid) {
                    p.peer_interested = false;
                }
                c.rechoke(false, slots);
            }
            MSG_HAVE => {
                let Ok(b) = <[u8; 4]>::try_from(m.payload.as_slice()) else {
                    return Err(wire(format!("have of {} bytes", m.payload.len())));
                };
                return self.lock().peer_has(pid, u32::from_be_bytes(b) as usize);
            }
            MSG_BITFIELD => {
                let mut c = self.lock();
                let late = c.peers.get(&pid).and_then(|p| p.bf.as_ref()).is_some_and(|bf| bf.count(bf.0.len() * 8) > 0);
                if late {
                    return Err(wire("late bitfield"));
                }
                let bf = Bitfield::from_bytes(&m.payload);
                if let Some(info) = c.info.clone() {
                    let n = info.num_pieces();
                    if !bf.valid(n) {
                        return Err(wire(format!("bitfield does not fit {n} pieces")));
                    }
                    for (i, avail) in c.avail.iter_mut().enumerate().take(n) {
                        if bf.has(i) {
                            *avail += 1;
                        }
                    }
                } else if bf.len_bytes() > 1 << 21 {
                    return Err(wire("bitfield too large"));
                }
                if let Some(p) = c.peers.get_mut(&pid) {
                    p.bf = Some(bf);
                }
                c.update_interest(pid);
                c.fill(pid);
            }
            MSG_REQUEST => {
                let r = parse_block_ref(&m.payload)?;
                let mut c = self.lock();
                let ok = c.info.as_ref().is_some_and(|info| {
                    r.piece < info.num_pieces()
                        && r.length > 0
                        && r.length <= MAX_BLOCK_REQUEST
                        && (r.begin + r.length) as i64 <= info.piece_size(r.piece)
                });
                if !ok {
                    return Err(wire(format!("bad request {r:?}")));
                }
                let have = c.have.has(r.piece);
                if let Some(p) = c.peers.get_mut(&pid) {
                    // Ignored, not fatal: races with choke are normal.
                    if p.am_choking || !have {
                        return Ok(());
                    }
                    p.canceled.remove(&r);
                    // A full queue drops it; the peer re-requests later.
                    let _ = p.upload.try_send(r);
                }
            }
            MSG_CANCEL => {
                let r = parse_block_ref(&m.payload)?;
                let mut c = self.lock();
                if let Some(p) = c.peers.get_mut(&pid)
                    && p.canceled.len() < 4 * MAX_UPLOAD_QUEUE
                {
                    p.canceled.insert(r);
                }
            }
            MSG_PIECE => return self.receive_block(pid, &m.payload, quit, run).await,
            MSG_EXTENDED => return self.handle_extended(pid, &m.payload),
            // Unknown and unsupported messages (port, fast extension) are
            // ignored, as BEP 3 allows.
            _ => {}
        }
        Ok(())
    }

    /// Stores a block, and verifies the piece once complete.
    async fn receive_block(&self, pid: u64, payload: &[u8], quit: &Cancel, run: &Cancel) -> Result<(), Error> {
        if payload.len() < 8 {
            return Err(wire("short piece message"));
        }
        let piece = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
        let begin = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]) as usize;
        let data = &payload[8..];
        let (st, info, want, k, b) = {
            let mut c = self.lock();
            let Some(info) = c.info.clone() else { return Err(wire("unexpected block")) };
            if piece >= info.num_pieces() || !begin.is_multiple_of(BLOCK_SIZE) {
                return Err(wire("unexpected block"));
            }
            let b = begin / BLOCK_SIZE;
            let want = c.block_ref(piece, b);
            if begin as i64 >= info.piece_size(piece) || data.len() != want.length {
                return Err(wire("block of wrong size"));
            }
            let k = (piece, begin);
            let requested = c.peers.get(&pid).is_some_and(|p| p.inflight.contains_key(&k));
            let ps = &c.pieces[piece];
            if !requested || ps.got[b] || ps.verifying || c.have.has(piece) {
                // Unrequested, a late duplicate, or after a cancel: discard.
                if requested {
                    c.drop_request(pid, k);
                }
                return Ok(());
            }
            (c.st.clone(), info, want, k, b)
        };
        let Some(st) = st else { return Ok(()) };
        tokio::select! {
            _ = quit.cancelled() => return Err(wire("peer closed")),
            _ = run.cancelled() => return Err(wire("torrent stopped")),
            _ = self.0.c.down.wait(data.len()) => {}
        }
        if let Err(e) = st.write_at(data, piece as i64 * info.piece_length + begin as i64) {
            self.fail(&format!("write failed: {e}"));
            return Err(Error::Io(e.to_string()));
        }
        let complete = {
            let mut c = self.lock();
            c.downloaded += data.len() as i64;
            if c.pieces[piece].got[b] || c.have.has(piece) {
                c.drop_request(pid, k);
                return Ok(());
            }
            let others: Vec<u64> = {
                let ps = &mut c.pieces[piece];
                ps.got[b] = true;
                ps.got_n += 1;
                ps.contribute.insert(pid);
                ps.reqs.remove(&b).unwrap_or_default()
            };
            // Cancel duplicates still out with other peers (endgame).
            for q in others {
                if q != pid
                    && let Some(qp) = c.peers.get_mut(&q)
                {
                    qp.send(request_frame(MSG_CANCEL, want));
                    qp.inflight.remove(&k);
                }
            }
            if let Some(p) = c.peers.get_mut(&pid) {
                p.inflight.remove(&k);
            }
            let ps = &mut c.pieces[piece];
            let complete = ps.got_n == ps.got.len();
            if complete {
                ps.verifying = true;
            }
            c.fill(pid);
            complete
        };
        if complete {
            self.verify_piece(piece, st).await;
        }
        Ok(())
    }

    async fn verify_piece(&self, piece: usize, st: Arc<crate::storage::Storage>) {
        let ok = match tokio::task::spawn_blocking(move || st.verify(piece)).await {
            Ok(Ok(ok)) => ok,
            Ok(Err(e)) => {
                self.fail(&format!("read failed: {e}"));
                return;
            }
            Err(_) => return,
        };
        let done = {
            let mut c = self.lock();
            c.active.remove(&piece);
            let contributors: Vec<u64> = {
                let ps = &mut c.pieces[piece];
                ps.verifying = false;
                std::mem::take(&mut ps.contribute).into_iter().collect()
            };
            if !ok {
                let ps = &mut c.pieces[piece];
                ps.got.iter_mut().for_each(|g| *g = false);
                ps.got_n = 0;
                for q in contributors {
                    if let Some(qp) = c.peers.get_mut(&q) {
                        qp.bad += 1;
                        if qp.bad >= 3 {
                            qp.quit.cancel(); // repeatedly sent corrupt data
                        }
                    }
                }
                c.fill_all();
                return;
            }
            c.have.set(piece);
            let have = frame(MSG_HAVE, &[&u32s(&[piece as u32])]);
            for p in c.peers.values() {
                p.send(have.clone());
            }
            c.update_interest_all();
            let n = c.info.as_ref().map(|i| i.num_pieces()).unwrap_or(0);
            if c.have.count(n) == n && !c.completed {
                c.completed = true;
                c.state = State::Seeding;
                c.update_interest_all();
                true
            } else {
                c.fill_all();
                false
            }
        };
        if done {
            tracing::info!(torrent = %self.0.ih, "torrent complete");
            self.finalize(true);
            self.poke();
        }
    }

    /// Reads a requested block if it is still wanted.
    fn read_block_for_upload(&self, pid: u64, r: BlockRef) -> Option<Vec<u8>> {
        let (st, plen) = {
            let mut c = self.lock();
            let have = c.have.has(r.piece);
            let plen = c.info.as_ref()?.piece_length;
            let st = c.st.clone()?;
            let p = c.peers.get_mut(&pid)?;
            if p.canceled.remove(&r) || p.am_choking || !have {
                return None;
            }
            (st, plen)
        };
        let mut buf = vec![0u8; r.length];
        st.read_at(&mut buf, r.piece as i64 * plen + r.begin as i64).ok()?;
        Some(buf)
    }

    /// BEP 10 and BEP 9 (ut_metadata).
    fn handle_extended(&self, pid: u64, payload: &[u8]) -> Result<(), Error> {
        let Some((&id, body)) = payload.split_first() else { return Err(wire("empty extended message")) };
        if id == 0 {
            let d = bencode::decode(body).map_err(|e| wire(format!("extended handshake: {e}")))?;
            let mut c = self.lock();
            if let Some(ut) = d.get("m").and_then(|m| m.get("ut_metadata")).and_then(Value::as_int)
                && ut > 0
                && ut < 256
                && let Some(p) = c.peers.get_mut(&pid)
            {
                p.ut_meta = ut as u8;
            }
            if let Some(size) = d.get("metadata_size").and_then(Value::as_int)
                && c.info.is_none()
                && size > 0
                && size <= MAX_METADATA_SIZE
                && c.meta_size == 0
            {
                c.meta_size = size as usize;
                c.meta_pieces = vec![None; (size as usize).div_ceil(META_PIECE_SIZE)];
            }
            c.request_meta();
            return Ok(());
        }
        if id != LOCAL_UT_METADATA {
            return Ok(()); // an extension we did not advertise
        }
        let (d, rest) = bencode::decode_prefix(body).map_err(|e| wire(format!("ut_metadata: {e}")))?;
        let typ = d.get("msg_type").and_then(Value::as_int).unwrap_or(0);
        let piece = d.get("piece").and_then(Value::as_int).unwrap_or(0);
        let mut c = self.lock();
        match typ {
            0 => {
                let Some(p) = c.peers.get(&pid) else { return Ok(()) };
                if p.ut_meta == 0 {
                    return Ok(());
                }
                let raw = c.info.as_ref().map(|i| &i.raw[..]);
                match raw {
                    Some(raw) if piece >= 0 && (piece as usize).saturating_mul(META_PIECE_SIZE) < raw.len() => {
                        let start = piece as usize * META_PIECE_SIZE;
                        let end = (start + META_PIECE_SIZE).min(raw.len());
                        let d = Value::dict([
                            ("msg_type", Value::Int(1)),
                            ("piece", Value::Int(piece)),
                            ("total_size", Value::Int(raw.len() as i64)),
                        ]);
                        p.send(ext_frame(p.ut_meta, &d, &raw[start..end]));
                    }
                    _ => {
                        let d = Value::dict([("msg_type", Value::Int(2)), ("piece", Value::Int(piece))]);
                        p.send(ext_frame(p.ut_meta, &d, &[]));
                    }
                }
            }
            1 => {
                if c.info.is_some() || c.meta_size == 0 || piece < 0 || piece as usize >= c.meta_pieces.len() {
                    return Ok(());
                }
                let i = piece as usize;
                let want = META_PIECE_SIZE.min(c.meta_size - i * META_PIECE_SIZE);
                if rest.len() != want {
                    return Err(wire(format!("metadata piece of {} bytes, want {want}", rest.len())));
                }
                c.meta_pieces[i] = Some(rest.to_vec());
                c.meta_asked.remove(&i);
                if c.try_metadata(self.dir(), &self.0.ih) {
                    drop(c);
                    self.metadata_arrived();
                }
            }
            2 if piece >= 0 => {
                c.meta_asked.remove(&(piece as usize));
            }
            _ => {}
        }
        Ok(())
    }
}
