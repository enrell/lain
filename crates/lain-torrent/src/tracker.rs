//! HTTP (BEP 3/23/7) and UDP (BEP 15) tracker announces.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;

use crate::bencode::{self, Value};
use crate::error::tracker;
use crate::metainfo::{parse_query, query_escape, split_url};
use crate::{Error, HttpGet, InfoHash};

pub const EVENT_NONE: &str = "";
pub const EVENT_STARTED: &str = "started";
pub const EVENT_COMPLETED: &str = "completed";
pub const EVENT_STOPPED: &str = "stopped";

const MAX_TRACKER_BODY: usize = 1 << 20;
const MAX_PEERS_PER_ANNOUNCE: usize = 2000;
const UDP_PROTOCOL_ID: u64 = 0x41727101980;

/// What a client tells a tracker.
#[derive(Debug, Clone, Default)]
pub struct AnnounceRequest {
    pub info_hash: InfoHash,
    pub peer_id: [u8; 20],
    pub port: u16,
    pub uploaded: i64,
    pub downloaded: i64,
    pub left: i64,
    pub event: String,
    pub num_want: i32,
}

/// The tracker's answer.
#[derive(Debug, Clone, Default)]
pub struct AnnounceResponse {
    pub interval: Duration,
    pub peers: Vec<SocketAddr>,
    pub seeders: i64,
    pub leechers: i64,
}

/// Contacts one tracker (http, https or udp) within `timeout`.
pub async fn announce(
    http: &Arc<dyn HttpGet>,
    url: &str,
    req: &AnnounceRequest,
    timeout: Duration,
) -> Result<AnnounceResponse, Error> {
    let Some((scheme, host, _)) = split_url(url) else { return Err(tracker("bad url")) };
    let deadline = Instant::now() + timeout;
    match scheme.as_str() {
        "http" | "https" => announce_http(http.clone(), url, req, timeout).await,
        "udp" => match tokio::time::timeout_at(deadline.into(), announce_udp(&host, req, deadline)).await {
            Ok(r) => r,
            Err(_) => Err(tracker("timeout")),
        },
        s => Err(tracker(format!("unsupported scheme {s:?}"))),
    }
}

fn announce_url(url: &str, req: &AnnounceRequest) -> String {
    let base = url.split('#').next().unwrap_or(url);
    let (path, query) = base.split_once('?').unwrap_or((base, ""));
    // Go: `u.Query()` then `Encode()`, which sorts every key. info_hash
    // and peer_id are raw bytes.
    let mut q: Vec<(String, Vec<u8>)> = parse_query(query)
        .into_iter()
        .filter(|(k, _)| {
            !matches!(
                k.as_str(),
                "info_hash" | "peer_id" | "port" | "uploaded" | "downloaded" | "left" | "compact" | "numwant" | "event"
            )
        })
        .map(|(k, v)| (k, v.into_bytes()))
        .collect();
    q.push(("info_hash".into(), req.info_hash.0.to_vec()));
    q.push(("peer_id".into(), req.peer_id.to_vec()));
    q.push(("port".into(), req.port.to_string().into_bytes()));
    q.push(("uploaded".into(), req.uploaded.to_string().into_bytes()));
    q.push(("downloaded".into(), req.downloaded.to_string().into_bytes()));
    q.push(("left".into(), req.left.to_string().into_bytes()));
    q.push(("compact".into(), b"1".to_vec()));
    if req.num_want > 0 {
        q.push(("numwant".into(), req.num_want.to_string().into_bytes()));
    }
    if !req.event.is_empty() {
        q.push(("event".into(), req.event.as_bytes().to_vec()));
    }
    q.sort_by(|a, b| a.0.cmp(&b.0));
    let enc: Vec<String> =
        q.iter().map(|(k, v)| format!("{}={}", query_escape(k.as_bytes()), query_escape(v))).collect();
    format!("{path}?{}", enc.join("&"))
}

async fn announce_http(
    http: Arc<dyn HttpGet>,
    url: &str,
    req: &AnnounceRequest,
    timeout: Duration,
) -> Result<AnnounceResponse, Error> {
    let full = announce_url(url, req);
    let res =
        tokio::task::spawn_blocking(move || http.get(&full, timeout)).await.map_err(|_| tracker("request aborted"))?;
    let (status, body) = res.map_err(tracker)?;
    if status != 200 {
        return Err(tracker(format!("http {status}")));
    }
    if body.len() > MAX_TRACKER_BODY {
        return Err(tracker("bad body"));
    }
    parse_http_announce(&body)
}

/// Parses an HTTP tracker's bencoded answer.
pub fn parse_http_announce(body: &[u8]) -> Result<AnnounceResponse, Error> {
    let v = bencode::decode(body).map_err(|e| tracker(e.to_string()))?;
    if !matches!(v, Value::Dict(_)) {
        return Err(tracker("response is not a dictionary"));
    }
    if let Some(reason) = v.get("failure reason").and_then(Value::as_bytes) {
        let reason = String::from_utf8_lossy(&reason[..reason.len().min(200)]).into_owned();
        return Err(tracker(format!("refused: {reason}")));
    }
    let mut out = AnnounceResponse { interval: Duration::from_secs(30 * 60), ..Default::default() };
    if let Some(n) = v.get("interval").and_then(Value::as_int).filter(|&n| n > 0 && n < 86400) {
        out.interval = Duration::from_secs(n as u64);
    }
    out.seeders = v.get("complete").and_then(Value::as_int).unwrap_or(0);
    out.leechers = v.get("incomplete").and_then(Value::as_int).unwrap_or(0);
    match v.get("peers") {
        Some(Value::Bytes(b)) => out.peers = compact_peers(b, 4),
        Some(Value::List(l)) => {
            for e in l {
                let ip = e.get("ip").and_then(Value::as_str).and_then(|s| s.parse::<IpAddr>().ok());
                let port = e.get("port").and_then(Value::as_int).filter(|&p| p > 0 && p <= 65535);
                if let (Some(ip), Some(port)) = (ip, port) {
                    out.peers.push(SocketAddr::new(ip.to_canonical(), port as u16));
                }
                if out.peers.len() > MAX_PEERS_PER_ANNOUNCE {
                    break;
                }
            }
        }
        _ => {}
    }
    if let Some(p6) = v.get("peers6").and_then(Value::as_bytes) {
        out.peers.extend(compact_peers(p6, 16));
    }
    out.peers.truncate(MAX_PEERS_PER_ANNOUNCE);
    Ok(out)
}

/// Decodes BEP 23 (`ip_len` 4) and BEP 7 (`ip_len` 16) peer lists.
pub fn compact_peers(b: &[u8], ip_len: usize) -> Vec<SocketAddr> {
    let step = ip_len + 2;
    let mut out = Vec::new();
    for c in b.chunks_exact(step) {
        let port = u16::from_be_bytes([c[ip_len], c[ip_len + 1]]);
        if port == 0 {
            continue;
        }
        let ip = match ip_len {
            4 => IpAddr::V4(Ipv4Addr::new(c[0], c[1], c[2], c[3])),
            16 => {
                let mut a = [0u8; 16];
                a.copy_from_slice(&c[..16]);
                IpAddr::V6(Ipv6Addr::from(a)).to_canonical()
            }
            _ => continue,
        };
        out.push(SocketAddr::new(ip, port));
    }
    out
}

/// Encodes IPv4 peers (the test tracker uses it).
pub fn encode_compact_peers(peers: &[SocketAddr]) -> Vec<u8> {
    let mut b = Vec::new();
    for p in peers {
        if let IpAddr::V4(a) = p.ip().to_canonical() {
            b.extend_from_slice(&a.octets());
            b.extend_from_slice(&p.port().to_be_bytes());
        }
    }
    b
}

fn udp_event(e: &str) -> u32 {
    match e {
        EVENT_COMPLETED => 1,
        EVENT_STARTED => 2,
        EVENT_STOPPED => 3,
        _ => 0,
    }
}

fn rand_u32() -> u32 {
    getrandom::u32().unwrap_or(0x5eed_1a17)
}

/// BEP 15: connect, then announce, three tries each with growing
/// timeouts inside the caller's deadline.
async fn announce_udp(host: &str, req: &AnnounceRequest, deadline: Instant) -> Result<AnnounceResponse, Error> {
    let addr = tokio::net::lookup_host(host)
        .await
        .map_err(|e| tracker(e.to_string()))?
        .next()
        .ok_or_else(|| tracker("no address"))?;
    let bind: SocketAddr =
        if addr.is_ipv4() { "0.0.0.0:0".parse() } else { "[::]:0".parse() }.map_err(|_| tracker("bind"))?;
    let sock = UdpSocket::bind(bind).await.map_err(|e| tracker(e.to_string()))?;
    sock.connect(addr).await.map_err(|e| tracker(e.to_string()))?;
    let mut buf = vec![0u8; 16 + 6 * MAX_PEERS_PER_ANNOUNCE];

    async fn exchange(
        sock: &UdpSocket,
        buf: &mut [u8],
        pkt: &[u8],
        action: u32,
        tid: u32,
        min_len: usize,
        deadline: Instant,
    ) -> Result<Vec<u8>, Error> {
        for attempt in 0..3 {
            let try_until = (Instant::now() + Duration::from_secs(2 << attempt)).min(deadline);
            sock.send(pkt).await.map_err(|e| tracker(e.to_string()))?;
            loop {
                let n = match tokio::time::timeout_at(try_until.into(), sock.recv(buf)).await {
                    Err(_) => break,
                    Ok(Err(e)) => return Err(tracker(e.to_string())),
                    Ok(Ok(n)) => n,
                };
                if n < 8 || u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]) != tid {
                    continue; // stale or foreign datagram
                }
                let act = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
                if act == 3 {
                    let msg = String::from_utf8_lossy(&buf[8..n.min(208)]).into_owned();
                    return Err(tracker(format!("refused: {msg}")));
                }
                if act != action || n < min_len {
                    return Err(tracker("bad response"));
                }
                return Ok(buf[..n].to_vec());
            }
            if Instant::now() >= deadline {
                return Err(tracker("timeout"));
            }
        }
        Err(tracker("timeout"))
    }

    let tid = rand_u32();
    let mut pkt = UDP_PROTOCOL_ID.to_be_bytes().to_vec();
    pkt.extend_from_slice(&0u32.to_be_bytes());
    pkt.extend_from_slice(&tid.to_be_bytes());
    let resp = exchange(&sock, &mut buf, &pkt, 0, tid, 16, deadline).await?;
    let conn_id = &resp[8..16];

    let tid = rand_u32();
    let num_want: i32 = if req.num_want > 0 { req.num_want } else { -1 };
    let mut pkt = conn_id.to_vec();
    pkt.extend_from_slice(&1u32.to_be_bytes());
    pkt.extend_from_slice(&tid.to_be_bytes());
    pkt.extend_from_slice(&req.info_hash.0);
    pkt.extend_from_slice(&req.peer_id);
    pkt.extend_from_slice(&(req.downloaded as u64).to_be_bytes());
    pkt.extend_from_slice(&(req.left as u64).to_be_bytes());
    pkt.extend_from_slice(&(req.uploaded as u64).to_be_bytes());
    pkt.extend_from_slice(&udp_event(&req.event).to_be_bytes());
    pkt.extend_from_slice(&0u32.to_be_bytes()); // IP: default
    pkt.extend_from_slice(&rand_u32().to_be_bytes());
    pkt.extend_from_slice(&(num_want as u32).to_be_bytes());
    pkt.extend_from_slice(&req.port.to_be_bytes());
    let resp = exchange(&sock, &mut buf, &pkt, 1, tid, 20, deadline).await?;
    let at = |i: usize| u32::from_be_bytes([resp[i], resp[i + 1], resp[i + 2], resp[i + 3]]);
    let mut interval = Duration::from_secs(at(8) as u64);
    if interval.is_zero() || interval > Duration::from_secs(24 * 3600) {
        interval = Duration::from_secs(30 * 60);
    }
    Ok(AnnounceResponse {
        interval,
        leechers: at(12) as i64,
        seeders: at(16) as i64,
        peers: compact_peers(&resp[20..], 4),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::Rng;

    #[test]
    fn announce_url_sorts_and_escapes() {
        let req =
            AnnounceRequest { info_hash: InfoHash([0xff; 20]), port: 7, event: "started".into(), ..Default::default() };
        let u = announce_url("http://tracker-exemplo.invalid/announce?passkey=a%20b", &req);
        assert!(
            u.starts_with("http://tracker-exemplo.invalid/announce?compact=1&downloaded=0&event=started&info_hash=%FF")
        );
        assert!(u.contains("&passkey=a+b&"));
    }

    #[test]
    fn parses_bodies_and_never_panics() {
        let ok = parse_http_announce(b"d8:intervali60e5:peers6:\x7f\x00\x00\x01\x1b\x59e").unwrap();
        assert_eq!(ok.interval, Duration::from_secs(60));
        assert_eq!(ok.peers, vec!["127.0.0.1:7001".parse().unwrap()]);
        let dict = parse_http_announce(b"d5:peersld2:ip9:127.0.0.14:porti6881eeee").unwrap();
        assert_eq!(dict.peers, vec!["127.0.0.1:6881".parse().unwrap()]);
        assert!(
            matches!(parse_http_announce(b"d14:failure reason4:nopee"), Err(Error::Tracker(m)) if m == "refused: nope")
        );
        let seeds: [&[u8]; 3] = [
            b"d8:intervali60e5:peers6:\x7f\x00\x00\x01\x1b\x59e",
            b"d14:failure reason5:nopee",
            b"d5:peersld2:ip9:127.0.0.14:porti6881eeee",
        ];
        let mut rng = Rng::new(3);
        for i in 0..20_000 {
            let mut body = seeds[i % 3].to_vec();
            rng.mutate(&mut body);
            if let Ok(r) = parse_http_announce(&body) {
                assert!(r.peers.len() <= MAX_PEERS_PER_ANNOUNCE && !r.interval.is_zero());
                assert!(r.peers.iter().all(|p| p.port() != 0), "zero port accepted");
            }
        }
    }
}
