//! Every swarm here is in-process on 127.0.0.1: a local tracker, a
//! seeding client and a leeching client.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use lain_torrent::trackertest::Tracker;
use lain_torrent::*;

fn content(n: usize, seed: u8) -> Vec<u8> {
    let mut x = (seed as u32).wrapping_mul(2654435761).wrapping_add(1);
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

struct Fixture {
    files: Vec<SourceFile>,
    mi: MetaInfo,
}

fn fixture(trackers: &[String]) -> Fixture {
    let sf = |p: &[&str], data: Vec<u8>| SourceFile { path: p.iter().map(|s| s.to_string()).collect(), data };
    let files = vec![
        sf(&["[Fansub-A] Show - 01 [1080p].mkv"], content(150_000, 1)),
        sf(&["[Fansub-A] Show - 02 [1080p].mkv"], content(97_001, 2)),
        sf(&["Extras", "empty.txt"], Vec::new()),
        sf(&["Extras", "note.txt"], content(10, 3)),
    ];
    let (_, mi) = build("[Fansub-A] Show", &files, 32 << 10, trackers).unwrap();
    Fixture { files, mi }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let p = std::env::temp_dir().join(format!("lain-torrent-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn file_path(dir: &Path, f: &Fixture, sf: &SourceFile) -> PathBuf {
    let mut p = dir.join(&f.mi.info.name);
    for c in &sf.path {
        p.push(c);
    }
    p
}

fn write_files(dir: &Path, f: &Fixture) {
    for sf in &f.files {
        let p = file_path(dir, f, sf);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, &sf.data).unwrap();
    }
}

fn assert_files(dir: &Path, f: &Fixture) {
    for sf in &f.files {
        let p = file_path(dir, f, sf);
        let got = std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        assert!(got == sf.data, "{} differs ({} vs {} bytes)", p.display(), got.len(), sf.data.len());
    }
}

fn client(mut cfg: Config) -> Client {
    cfg.listen_addr = Some("127.0.0.1:0".into());
    if cfg.min_announce_interval.is_zero() {
        cfg.min_announce_interval = Duration::from_secs(1);
    }
    Client::new(cfg).unwrap()
}

fn wait_state(t: &Torrent, want: State, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if t.stats().state == want {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("state {} after {timeout:?}, want {want} ({:?})", t.stats().state, t.stats());
}

fn signal() -> (Option<Callback>, mpsc::Receiver<()>) {
    let (tx, rx) = mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    (
        Some(Arc::new(move |_: &Torrent| {
            let _ = tx.lock().unwrap().send(());
        })),
        rx,
    )
}

fn seed(f: &Fixture) -> (Client, Torrent, TempDir) {
    let dir = TempDir::new();
    write_files(dir.path(), f);
    let c = client(Config::default());
    let t = c.add(Spec { meta_info: Some(f.mi.clone()), dir: dir.path().into(), ..Default::default() }).unwrap();
    wait_state(&t, State::Seeding, Duration::from_secs(5));
    (c, t, dir)
}

fn local(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}

#[test]
fn swarm_download_over_http_tracker() {
    let tr = Tracker::new().unwrap();
    let f = fixture(&[tr.http_url()]);
    let (_sc, seed_t, _sd) = seed(&f);
    let dir = TempDir::new();
    let (on_complete, done) = signal();
    let c = client(Config::default());
    let leech = c
        .add(Spec { meta_info: Some(f.mi.clone()), dir: dir.path().into(), on_complete, ..Default::default() })
        .unwrap();
    done.recv_timeout(Duration::from_secs(15))
        .unwrap_or_else(|_| panic!("no completion: leech {:?} seed {:?}", leech.stats(), seed_t.stats()));
    assert_files(dir.path(), &f);
    let st = leech.stats();
    assert!(
        st.state == State::Seeding && st.completed == f.mi.info.length && st.downloaded >= f.mi.info.length,
        "{st:?}"
    );
    assert!(seed_t.stats().uploaded >= f.mi.info.length, "seed uploaded {}", seed_t.stats().uploaded);
    let deadline = Instant::now() + Duration::from_secs(5);
    while tr.count(EVENT_COMPLETED) == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(tr.count(EVENT_COMPLETED), 1, "completed announces");
    let files = leech.files();
    assert_eq!(files.len(), 4);
    assert_eq!(files[0].rel, Path::new("[Fansub-A] Show").join("[Fansub-A] Show - 01 [1080p].mkv").to_string_lossy());
    assert!(files[0].path.is_absolute());
    // Pausing announces "stopped".
    leech.pause();
    assert_eq!(leech.stats().state, State::Paused);
    assert!(tr.count(EVENT_STOPPED) >= 1);
}

#[test]
fn magnet_fetches_metadata_over_udp_tracker() {
    let tr = Tracker::new().unwrap();
    let f = fixture(&[tr.udp_url()]);
    let (_sc, _st, _sd) = seed(&f);
    let dir = TempDir::new();
    let (on_metadata, got_meta) = signal();
    let (on_complete, done) = signal();
    let magnet = Magnet { info_hash: f.mi.info_hash, name: "Show".into(), trackers: vec![tr.udp_url()] };
    let m = parse_magnet(&magnet.to_string()).unwrap();
    let c = client(Config::default());
    let leech = c
        .add(Spec {
            info_hash: Some(m.info_hash),
            trackers: m.trackers,
            name: m.name,
            dir: dir.path().into(),
            on_metadata,
            on_complete,
            ..Default::default()
        })
        .unwrap();
    assert!(leech.info().is_none(), "a magnet starts without metadata");
    for rx in [&got_meta, &done] {
        rx.recv_timeout(Duration::from_secs(15)).unwrap_or_else(|_| panic!("stalled: {:?}", leech.stats()));
    }
    assert_eq!(leech.info().unwrap().raw, f.mi.info.raw, "fetched metadata differs");
    assert_files(dir.path(), &f);
}

#[test]
fn direct_peers_without_tracker() {
    let f = fixture(&[]);
    let (sc, _st, _sd) = seed(&f);
    let dir = TempDir::new();
    let (on_complete, done) = signal();
    let c = client(Config::default());
    c.add(Spec {
        meta_info: Some(f.mi.clone()),
        dir: dir.path().into(),
        peers: vec![local(sc.port())],
        on_complete,
        ..Default::default()
    })
    .unwrap();
    done.recv_timeout(Duration::from_secs(15)).expect("no completion with a direct peer");
    assert_files(dir.path(), &f);
}

#[test]
fn corrupt_seeder_is_survived() {
    let f = fixture(&[]);
    let good = TempDir::new();
    write_files(good.path(), &f);
    // The bad seeder claims every piece (resume bitfield + full-size
    // files) but its first file is garbage.
    let bad = TempDir::new();
    write_files(bad.path(), &f);
    std::fs::write(file_path(bad.path(), &f, &f.files[0]), content(f.files[0].data.len(), 99)).unwrap();
    let mut all = Bitfield::new(f.mi.info.num_pieces());
    for i in 0..f.mi.info.num_pieces() {
        all.set(i);
    }
    let cb = client(Config::default());
    cb.add(Spec { meta_info: Some(f.mi.clone()), dir: bad.path().into(), have: Some(all), ..Default::default() })
        .unwrap();
    let cg = client(Config::default());
    cg.add(Spec { meta_info: Some(f.mi.clone()), dir: good.path().into(), ..Default::default() }).unwrap();
    let dir = TempDir::new();
    let (on_complete, done) = signal();
    let c = client(Config::default());
    c.add(Spec {
        meta_info: Some(f.mi.clone()),
        dir: dir.path().into(),
        peers: vec![local(cb.port()), local(cg.port())],
        on_complete,
        ..Default::default()
    })
    .unwrap();
    done.recv_timeout(Duration::from_secs(30)).expect("no completion beside a corrupt seeder");
    assert_files(dir.path(), &f);
}

#[test]
fn resume_and_recheck() {
    let f = fixture(&[]);
    let (cs, _st, _sd) = seed(&f);
    let peers = vec![local(cs.port())];

    // Throttled download, paused part-way.
    let dir = TempDir::new();
    let c1 = client(Config { download_rate: 64 << 10, ..Default::default() });
    let t = c1
        .add(Spec { meta_info: Some(f.mi.clone()), dir: dir.path().into(), peers: peers.clone(), ..Default::default() })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while t.stats().have < 2 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    t.pause();
    let have = t.stats().have;
    assert!(have >= 2 && have < f.mi.info.num_pieces(), "want a partial download, have {have}");
    t.remove(false).unwrap();

    // A fresh client without a resume bitfield rechecks what is on disk.
    let c2 = client(Config::default());
    let t2 = c2
        .add(Spec { meta_info: Some(f.mi.clone()), dir: dir.path().into(), paused: true, ..Default::default() })
        .unwrap();
    assert_eq!(t2.stats().state, State::Paused);
    t2.resume();
    wait_state(&t2, State::Downloading, Duration::from_secs(5));
    assert!(t2.stats().have >= have, "recheck found {}, had {have}", t2.stats().have);
    t2.remove(false).unwrap();
    let (on_complete, done) = signal();
    let t3 = c2
        .add(Spec { meta_info: Some(f.mi.clone()), dir: dir.path().into(), peers, on_complete, ..Default::default() })
        .unwrap();
    done.recv_timeout(Duration::from_secs(15)).unwrap_or_else(|_| panic!("resume stalled: {:?}", t3.stats()));
    assert_files(dir.path(), &f);

    // A completed torrent restarts straight to seeding from its bitfield.
    let bf = t3.bitfield();
    t3.remove(false).unwrap();
    let t4 = c2
        .add(Spec { meta_info: Some(f.mi.clone()), dir: dir.path().into(), have: Some(bf), ..Default::default() })
        .unwrap();
    wait_state(&t4, State::Seeding, Duration::from_secs(5));

    // Remove with data deletes the files and the empty folders.
    t4.remove(true).unwrap();
    assert!(!dir.path().join(&f.mi.info.name).exists(), "torrent folder survived removal");
    assert!(c2.torrent(&f.mi.info_hash).is_none(), "removed torrent still registered");
}

#[test]
fn rate_limit_is_honored() {
    let f = fixture(&[]);
    let (cs, _st, _sd) = seed(&f);
    let dir = TempDir::new();
    let (on_complete, done) = signal();
    let rate = 128 << 10;
    let c = client(Config { download_rate: rate, ..Default::default() });
    let start = Instant::now();
    c.add(Spec {
        meta_info: Some(f.mi.clone()),
        dir: dir.path().into(),
        peers: vec![local(cs.port())],
        on_complete,
        ..Default::default()
    })
    .unwrap();
    done.recv_timeout(Duration::from_secs(20)).expect("no completion under a rate cap");
    // One second of burst, then the cap: (247 KiB - 128 KiB) / 128 KiB.
    let min = Duration::from_secs_f64((f.mi.info.length - rate) as f64 / rate as f64 * 0.8);
    assert!(start.elapsed() >= min, "took {:?}, the cap allows no less than {min:?}", start.elapsed());
}

#[test]
fn add_twice_is_rejected() {
    let f = fixture(&[]);
    let c = client(Config::default());
    let d = TempDir::new();
    let spec = Spec { meta_info: Some(f.mi.clone()), dir: d.path().into(), paused: true, ..Default::default() };
    c.add(spec.clone()).unwrap();
    assert_eq!(c.add(spec.clone()).unwrap_err(), Error::Exists);
    assert!(c.add(Spec { dir: d.path().into(), ..Default::default() }).is_err(), "no metainfo, no hash");
    c.close();
    c.close();
    assert_eq!(c.add(spec).unwrap_err(), Error::Closed);
}

/// Arbitrary bytes after a valid handshake: any input may drop the
/// peer, none may panic or wedge the engine.
#[test]
fn garbage_peer_messages_are_survived() {
    use std::io::{Read, Write};
    let f = fixture(&[]);
    let (sc, seed_t, _sd) = seed(&f);
    let seeds: Vec<Vec<u8>> = vec![
        vec![0, 0, 0, 2, 5, 0xe0],
        vec![0, 0, 0, 1, 1, 0, 0, 0, 13, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x40, 0],
        vec![0, 0, 0, 3, 20, 0, b'd', 0, 0, 0, 5, 4, 0xff, 0xff, 0xff, 0xff],
        b"\x00\x00\x00\x14\x14\x01d8:msg_typei1eexxx".to_vec(),
        b"\x00\x00\x00\x1b\x14\x00d1:md11:ut_metadatai3eee".to_vec(),
        vec![0, 0, 0, 13, 7, 0, 0, 0, 1, 0, 0, 0, 0, 1, 2, 3, 4],
        vec![0, 0, 0, 1, 2, 0, 0, 0, 13, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x40, 0],
    ];
    let mut x: u64 = 0x2545_F491;
    let mut rnd = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for i in 0..150u32 {
        let mut payload = seeds[i as usize % seeds.len()].clone();
        for _ in 0..1 + rnd() % 4 {
            let at = (rnd() as usize) % (payload.len() + 1);
            match rnd() % 3 {
                0 if at < payload.len() => payload[at] ^= 1 << (rnd() % 8),
                1 => payload.insert(at, rnd() as u8),
                _ if at < payload.len() => {
                    payload.remove(at);
                }
                _ => {}
            }
        }
        let Ok(mut s) = std::net::TcpStream::connect(local(sc.port())) else { continue };
        let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
        let mut hs = vec![19u8];
        hs.extend_from_slice(b"BitTorrent protocol");
        hs.extend_from_slice(&[0, 0, 0, 0, 0, 0x10, 0, 0]);
        hs.extend_from_slice(&f.mi.info_hash.0);
        let mut id = [b'f'; 20];
        id[..4].copy_from_slice(&i.to_be_bytes());
        hs.extend_from_slice(&id);
        let _ = s.write_all(&hs);
        let _ = s.write_all(&payload);
        let mut buf = [0u8; 4096];
        let _ = s.read(&mut buf);
    }
    assert_eq!(seed_t.stats().state, State::Seeding);
    let dir = TempDir::new();
    let (on_complete, done) = signal();
    let c = client(Config::default());
    c.add(Spec {
        meta_info: Some(f.mi.clone()),
        dir: dir.path().into(),
        peers: vec![local(sc.port())],
        on_complete,
        ..Default::default()
    })
    .unwrap();
    done.recv_timeout(Duration::from_secs(15)).expect("seed wedged by garbage peers");
    assert_files(dir.path(), &f);
}
