use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use lain_torrent::trackertest::Tracker;
use lain_torrent::*;

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
}

fn http() -> Arc<dyn HttpGet> {
    Arc::new(PlainHttp::default())
}

#[test]
fn announce_http_and_udp() {
    let tr = Tracker::new().unwrap();
    let ih = InfoHash([1; 20]);
    let rt = rt();
    for url in [tr.http_url(), tr.udp_url()] {
        let mut a = AnnounceRequest { info_hash: ih, port: 7001, event: EVENT_STARTED.into(), ..Default::default() };
        a.peer_id[0] = b'a';
        a.peer_id[1] = url.len() as u8;
        rt.block_on(announce(&http(), &url, &a, Duration::from_secs(5)))
            .unwrap_or_else(|e| panic!("{url} seeder: {e}"));
        let mut b =
            AnnounceRequest { info_hash: ih, port: 7002, left: 100, event: EVENT_STARTED.into(), ..Default::default() };
        b.peer_id[0] = b'b';
        b.peer_id[1] = url.len() as u8;
        let res = rt.block_on(announce(&http(), &url, &b, Duration::from_secs(5))).unwrap();
        let want: SocketAddr = "127.0.0.1:7001".parse().unwrap();
        assert!(
            res.peers.contains(&want) && res.seeders >= 1 && res.interval == Duration::from_secs(5),
            "{url}: {res:?}"
        );
    }
    assert_eq!(tr.count(EVENT_STARTED), 4);
}

#[test]
fn announce_refusal_and_bad_schemes() {
    let tr = Tracker::new().unwrap();
    tr.refuse("unregistered torrent");
    let rt = rt();
    for url in [tr.http_url(), tr.udp_url(), "ftp://x/announce".into()] {
        let r = rt.block_on(announce(&http(), &url, &AnnounceRequest::default(), Duration::from_secs(5)));
        assert!(matches!(r, Err(Error::Tracker(_))), "{url}: {r:?}");
    }
    let r = rt.block_on(announce(&http(), &tr.http_url(), &AnnounceRequest::default(), Duration::from_secs(5)));
    assert_eq!(r.unwrap_err().to_string(), "torrent: tracker: refused: unregistered torrent");
}

#[test]
fn udp_announce_times_out() {
    // A loopback port with nothing listening: reads time out or fail.
    let r =
        rt().block_on(announce(&http(), "udp://127.0.0.1:9", &AnnounceRequest::default(), Duration::from_millis(300)));
    assert!(matches!(r, Err(Error::Tracker(_))), "{r:?}");
}
