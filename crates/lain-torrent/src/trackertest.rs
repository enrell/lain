//! A minimal in-process BitTorrent tracker (HTTP and UDP) for tests.
//! Lain's tests never contact a real tracker: swarms run on loopback
//! against this one. Plain std threads, so any test can use it.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::InfoHash;
use crate::bencode::{self, Value};
use crate::metainfo::unescape_bytes;
use crate::tracker::{EVENT_COMPLETED, EVENT_STARTED, EVENT_STOPPED, encode_compact_peers};

#[derive(Default)]
struct Swarms {
    swarms: HashMap<InfoHash, HashMap<[u8; 20], (SocketAddr, i64)>>,
    announces: HashMap<String, usize>,
    refuse: String,
}

/// Keeps one swarm per info hash.
pub struct Tracker {
    state: Arc<Mutex<Swarms>>,
    http: SocketAddr,
    udp: SocketAddr,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl Tracker {
    /// Starts HTTP and UDP listeners on loopback.
    pub fn new() -> std::io::Result<Tracker> {
        let state = Arc::new(Mutex::new(Swarms::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let ln = TcpListener::bind("127.0.0.1:0")?;
        let http = ln.local_addr()?;
        let udp_sock = UdpSocket::bind("127.0.0.1:0")?;
        udp_sock.set_read_timeout(Some(Duration::from_millis(100)))?;
        let udp = udp_sock.local_addr()?;
        let mut threads = Vec::new();
        let (s, st) = (state.clone(), stop.clone());
        threads.push(std::thread::spawn(move || {
            for conn in ln.incoming() {
                if st.load(Ordering::SeqCst) {
                    return;
                }
                if let Ok(c) = conn {
                    let s = s.clone();
                    std::thread::spawn(move || serve_http(&s, c));
                }
            }
        }));
        let (s, st) = (state.clone(), stop.clone());
        threads.push(std::thread::spawn(move || serve_udp(&s, &st, udp_sock)));
        Ok(Tracker { state, http, udp, stop, threads })
    }

    /// The announce URL of the HTTP tracker.
    pub fn http_url(&self) -> String {
        format!("http://{}/announce", self.http)
    }

    /// The announce URL of the UDP tracker.
    pub fn udp_url(&self) -> String {
        format!("udp://{}", self.udp)
    }

    /// Makes every later announce fail with reason (`""` accepts again).
    pub fn refuse(&self, reason: &str) {
        self.lock().refuse = reason.to_string();
    }

    /// Announces seen with the event (`""` for regular ones).
    pub fn count(&self, event: &str) -> usize {
        self.lock().announces.get(event).copied().unwrap_or(0)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Swarms> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Stops both listeners.
    pub fn close(&mut self) {
        if self.stop.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = TcpStream::connect(self.http);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for Tracker {
    fn drop(&mut self) {
        self.close();
    }
}

fn announce(
    s: &Mutex<Swarms>,
    ih: InfoHash,
    id: [u8; 20],
    addr: SocketAddr,
    left: i64,
    event: &str,
) -> (Vec<SocketAddr>, i64, i64) {
    let mut s = s.lock().unwrap_or_else(|e| e.into_inner());
    *s.announces.entry(event.to_string()).or_default() += 1;
    let sw = s.swarms.entry(ih).or_default();
    if event == EVENT_STOPPED {
        sw.remove(&id);
    } else {
        sw.insert(id, (addr, left));
    }
    let (mut peers, mut seeders, mut leechers) = (Vec::new(), 0, 0);
    for (pid, (a, left)) in sw.iter() {
        if *left == 0 {
            seeders += 1;
        } else {
            leechers += 1;
        }
        if *pid != id {
            peers.push(*a);
        }
    }
    (peers, seeders, leechers)
}

fn serve_http(s: &Mutex<Swarms>, mut c: TcpStream) {
    let _ = c.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 16 << 10 {
        match c.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let head = String::from_utf8_lossy(&buf).into_owned();
    let target = head.split_whitespace().nth(1).unwrap_or("/");
    let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut q: HashMap<String, Vec<u8>> = HashMap::new();
    for p in query.split('&') {
        let (k, v) = p.split_once('=').unwrap_or((p, ""));
        if let (Some(k), Some(v)) = (unescape_bytes(k), unescape_bytes(v)) {
            q.entry(String::from_utf8_lossy(&k).into_owned()).or_insert(v);
        }
    }
    let text = |k: &str| q.get(k).map(|v| String::from_utf8_lossy(v).into_owned()).unwrap_or_default();
    let refuse = s.lock().unwrap_or_else(|e| e.into_inner()).refuse.clone();
    let body = (|| {
        if !refuse.is_empty() {
            return Value::dict([("failure reason", Value::str(&refuse))]);
        }
        let (Some(ih), Some(id)) =
            (q.get("info_hash").filter(|v| v.len() == 20), q.get("peer_id").filter(|v| v.len() == 20))
        else {
            return Value::dict([("failure reason", Value::str("bad info_hash or peer_id"))]);
        };
        let port: u16 = text("port").parse().unwrap_or(0);
        let Ok(remote) = c.peer_addr() else { return Value::dict([("failure reason", Value::str("bad address"))]) };
        if port == 0 {
            return Value::dict([("failure reason", Value::str("bad address"))]);
        }
        let mut h = InfoHash::default();
        h.0.copy_from_slice(ih);
        let mut pid = [0u8; 20];
        pid.copy_from_slice(id);
        let addr = SocketAddr::new(remote.ip().to_canonical(), port);
        let left = text("left").parse().unwrap_or(0);
        let (peers, seeders, leechers) = announce(s, h, pid, addr, left, &text("event"));
        Value::dict([
            ("interval", Value::Int(5)),
            ("complete", Value::Int(seeders)),
            ("incomplete", Value::Int(leechers)),
            ("peers", Value::Bytes(encode_compact_peers(&peers))),
        ])
    })();
    let body = bencode::encode(&body);
    let mut resp =
        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
    resp.extend_from_slice(&body);
    let _ = c.write_all(&resp);
}

fn serve_udp(s: &Mutex<Swarms>, stop: &AtomicBool, sock: UdpSocket) {
    let mut buf = [0u8; 2048];
    let mut conns: Vec<u64> = Vec::new();
    while !stop.load(Ordering::SeqCst) {
        let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
        if n < 16 {
            continue;
        }
        let u32_at = |i: usize| u32::from_be_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]);
        let u64_at = |i: usize| ((u32_at(i) as u64) << 32) | u32_at(i + 4) as u64;
        let (action, tid) = (u32_at(8), u32_at(12));
        let refuse = s.lock().unwrap_or_else(|e| e.into_inner()).refuse.clone();
        let mut out = Vec::new();
        if !refuse.is_empty() {
            out.extend_from_slice(&3u32.to_be_bytes());
            out.extend_from_slice(&tid.to_be_bytes());
            out.extend_from_slice(refuse.as_bytes());
            let _ = sock.send_to(&out, from);
            continue;
        }
        match action {
            0 if u64_at(0) == 0x41727101980 => {
                let cid = (conns.len() as u64 + 1) * 7919;
                conns.push(cid);
                out.extend_from_slice(&0u32.to_be_bytes());
                out.extend_from_slice(&tid.to_be_bytes());
                out.extend_from_slice(&cid.to_be_bytes());
            }
            1 if n >= 98 && conns.contains(&u64_at(0)) => {
                let mut ih = InfoHash::default();
                ih.0.copy_from_slice(&buf[16..36]);
                let mut id = [0u8; 20];
                id.copy_from_slice(&buf[36..56]);
                let left = u64_at(64) as i64;
                let event = match u32_at(80) {
                    1 => EVENT_COMPLETED,
                    2 => EVENT_STARTED,
                    3 => EVENT_STOPPED,
                    _ => "",
                };
                let port = u16::from_be_bytes([buf[96], buf[97]]);
                let ip: IpAddr = from.ip().to_canonical();
                let (peers, seeders, leechers) = announce(s, ih, id, SocketAddr::new(ip, port), left, event);
                out.extend_from_slice(&1u32.to_be_bytes());
                out.extend_from_slice(&tid.to_be_bytes());
                out.extend_from_slice(&5u32.to_be_bytes());
                out.extend_from_slice(&(leechers as u32).to_be_bytes());
                out.extend_from_slice(&(seeders as u32).to_be_bytes());
                out.extend_from_slice(&encode_compact_peers(&peers));
            }
            _ => continue,
        }
        let _ = sock.send_to(&out, from);
    }
}
