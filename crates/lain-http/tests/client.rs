//! The client against local servers only: plain HTTP from a scripted
//! std server, HTTPS from a rustls server with a committed test CA
//! (`tests/tls/`, valid for `localhost` and 127.0.0.1 until 2126).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lain_http::{Client, Config, ErrorKind};

#[derive(Debug, Clone, Default)]
struct Seen {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

struct Reply {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
    delay: Duration,
}

fn reply(status: u16, body: &str) -> Reply {
    Reply { status, headers: Vec::new(), body: body.as_bytes().to_vec(), delay: Duration::ZERO }
}

type Handler = dyn Fn(&Seen, usize) -> Reply + Send + Sync;

struct Server {
    url: String,
    hits: Arc<AtomicUsize>,
    conns: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<Seen>>>,
}

fn read_request(r: &mut BufReader<TcpStream>) -> Option<Seen> {
    let mut line = String::new();
    if r.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let mut parts = line.split_whitespace();
    let mut seen = Seen { method: parts.next()?.into(), target: parts.next()?.into(), ..Seen::default() };
    loop {
        let mut h = String::new();
        r.read_line(&mut h).ok()?;
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        let (k, v) = h.split_once(':')?;
        seen.headers.push((k.trim().to_string(), v.trim().to_string()));
    }
    let n: usize = seen.header("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    seen.body = vec![0; n];
    r.read_exact(&mut seen.body).ok()?;
    Some(seen)
}

fn serve(handler: impl Fn(&Seen, usize) -> Reply + Send + Sync + 'static) -> Server {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    let handler: Arc<Handler> = Arc::new(handler);
    let (hits, conns, seen) =
        (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)), Arc::new(Mutex::new(vec![])));
    let (h2, c2, s2) = (hits.clone(), conns.clone(), seen.clone());
    std::thread::spawn(move || {
        for stream in l.incoming() {
            let Ok(stream) = stream else { return };
            c2.fetch_add(1, Ordering::SeqCst);
            let (handler, hits, seen) = (handler.clone(), h2.clone(), s2.clone());
            std::thread::spawn(move || {
                let mut w = stream.try_clone().unwrap();
                let mut r = BufReader::new(stream);
                while let Some(req) = read_request(&mut r) {
                    let n = hits.fetch_add(1, Ordering::SeqCst);
                    seen.lock().unwrap().push(req.clone());
                    let rep = handler(&req, n);
                    std::thread::sleep(rep.delay);
                    let mut out = format!("HTTP/1.1 {} X\r\nContent-Length: {}\r\n", rep.status, rep.body.len());
                    for (k, v) in &rep.headers {
                        out.push_str(&format!("{k}: {v}\r\n"));
                    }
                    out.push_str("\r\n");
                    if w.write_all(out.as_bytes()).and_then(|_| w.write_all(&rep.body)).is_err() {
                        return;
                    }
                }
            });
        }
    });
    Server { url, hits, conns, seen }
}

fn fast() -> Config {
    Config { backoff: Duration::from_millis(10), ..Config::default() }
}

#[test]
fn json_user_agent_keep_alive_and_errors() {
    let s = serve(|req, _| match req.target.as_str() {
        "/ok?q=a+b" => reply(200, r#"{"n":1}"#),
        "/missing" => reply(404, "nope"),
        _ => reply(500, "boom"),
    });
    let c = Client::new(fast()).unwrap();
    #[derive(serde::Deserialize)]
    struct N {
        n: i64,
    }
    let r = c.get(&format!("{}/ok?q=a+b", s.url)).send_blocking().unwrap().ok().unwrap();
    assert_eq!(r.json::<N>().unwrap().n, 1);
    assert!(r.json::<Vec<i64>>().unwrap_err().to_string().starts_with("127.0.0.1: response is not"));
    assert_eq!(s.seen.lock().unwrap()[0].method, "GET");
    let ua = s.seen.lock().unwrap()[0].header("user-agent").unwrap().to_string();
    assert_eq!(ua, lain_http::USER_AGENT);
    assert!(ua.starts_with("lain/"));
    let e = c.get(&format!("{}/missing", s.url)).send_blocking().unwrap().ok().unwrap_err();
    assert_eq!(
        (e.kind, e.status(), e.to_string().as_str()),
        (ErrorKind::Status(404), Some(404), "127.0.0.1: status 404")
    );
    // A 500 is an answer, not retried.
    let before = s.hits.load(Ordering::SeqCst);
    let r = c.get(&format!("{}/secret?api_key=k", s.url)).send_blocking().unwrap();
    assert_eq!((r.status, s.hits.load(Ordering::SeqCst) - before), (500, 1));
    assert!(!r.ok().unwrap_err().to_string().contains("api_key"), "errors never carry the query");
    assert_eq!(s.conns.load(Ordering::SeqCst), 1, "one kept-alive connection served every request");

    // The async API on someone else's runtime.
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let r = rt.block_on(c.get(&format!("{}/ok?q=a+b", s.url)).user_agent("Lain v0.1").send()).unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(s.seen.lock().unwrap().last().unwrap().header("user-agent"), Some("Lain v0.1"));
}

#[test]
fn retries_429_and_5xx_with_backoff() {
    // 429 without Retry-After: terminal, one attempt (Go's rule).
    let s = serve(|_, _| reply(429, "slow down"));
    let c = Client::new(fast()).unwrap();
    let e = c.get(&s.url).send_blocking().unwrap().ok().unwrap_err();
    assert_eq!((e.status(), s.hits.load(Ordering::SeqCst)), (Some(429), 1));
    assert!(e.to_string().contains("rate limited (429)"));

    // 429 with a short Retry-After is honored, then succeeds.
    let s = serve(|_, n| {
        if n == 0 { Reply { headers: vec![("Retry-After", "1".into())], ..reply(429, "") } } else { reply(200, "ok") }
    });
    let t = Instant::now();
    let r = c.get(&s.url).send_blocking().unwrap();
    assert_eq!((r.status, r.text().as_str(), s.hits.load(Ordering::SeqCst)), (200, "ok", 2));
    assert!(t.elapsed() >= Duration::from_secs(1));

    // Too long a Retry-After fails at once.
    let s = serve(|_, _| Reply { headers: vec![("Retry-After", "120".into())], ..reply(429, "") });
    assert_eq!(c.get(&s.url).send_blocking().unwrap().status, 429);
    assert_eq!(s.hits.load(Ordering::SeqCst), 1);

    // 503 on a GET: retried `retries` times; a POST is not repeated
    // unless it says it is idempotent.
    let s = serve(|_, _| reply(503, "down"));
    assert_eq!(c.get(&s.url).send_blocking().unwrap().status, 503);
    assert_eq!(s.hits.load(Ordering::SeqCst), 3);
    assert_eq!(c.post(&s.url).json(&serde_json::json!({})).send_blocking().unwrap().status, 503);
    assert_eq!(s.hits.load(Ordering::SeqCst), 4);
    let _ = c.post(&s.url).idempotent(true).send_blocking();
    assert_eq!(s.hits.load(Ordering::SeqCst), 7);
    let recovering = serve(|_, n| if n < 2 { reply(502, "") } else { reply(200, "up") });
    assert_eq!(c.get(&recovering.url).send_blocking().unwrap().text(), "up");

    // A refused connection is retried, then reported.
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let e = c.get(&format!("http://127.0.0.1:{port}/")).send_blocking().unwrap_err();
    assert_eq!(e.kind, ErrorKind::Connect);
}

#[test]
fn paces_requests_per_host() {
    let s = serve(|_, _| reply(200, "ok"));
    let c = Client::new(fast()).unwrap();
    c.pace(&s.url, Duration::from_millis(80));
    let t = Instant::now();
    for _ in 0..3 {
        c.get(&s.url).send_blocking().unwrap();
    }
    assert!(t.elapsed() >= Duration::from_millis(160), "{:?}", t.elapsed());
    c.pace(&s.url, Duration::ZERO);
    let t = Instant::now();
    c.get(&s.url).send_blocking().unwrap();
    assert!(t.elapsed() < Duration::from_millis(80));
}

#[test]
fn caps_size_and_time() {
    let s = serve(|req, _| match req.target.as_str() {
        "/big" => Reply { body: vec![b'x'; 2048], ..reply(200, "") },
        _ => Reply { delay: Duration::from_millis(400), ..reply(200, "late") },
    });
    let c = Client::new(Config { retries: 0, ..fast() }).unwrap();
    let e = c.get(&format!("{}/big", s.url)).max_body(1024).send_blocking().unwrap_err();
    assert_eq!(e.kind, ErrorKind::TooLarge);
    assert_eq!(c.get(&format!("{}/big", s.url)).send_blocking().unwrap().body.len(), 2048);
    let e = c.get(&format!("{}/slow", s.url)).timeout(Duration::from_millis(100)).send_blocking().unwrap_err();
    assert_eq!(e.kind, ErrorKind::Timeout);
}

#[test]
fn follows_redirects_safely() {
    let s = serve(|req, _| match req.target.as_str() {
        "/old" => Reply { headers: vec![("Location", "/new?x=1".into())], ..reply(301, "") },
        "/loop" => Reply { headers: vec![("Location", "/loop".into())], ..reply(302, "") },
        "/new?x=1" => reply(200, "moved"),
        _ => reply(404, ""),
    });
    let c = Client::new(fast()).unwrap();
    assert_eq!(c.get(&format!("{}/old", s.url)).send_blocking().unwrap().text(), "moved");
    assert_eq!(c.get(&format!("{}/old", s.url)).no_redirects().send_blocking().unwrap().status, 301);
    assert_eq!(c.post(&format!("{}/old", s.url)).send_blocking().unwrap().status, 301, "POST never follows");
    let e = c.get(&format!("{}/loop", s.url)).send_blocking().unwrap_err();
    assert!(e.to_string().contains("too many redirects"), "{e}");
}

#[test]
fn offline_and_bad_urls_never_connect() {
    let s = serve(|_, _| reply(200, "ok"));
    let e = Client::offline().get(&s.url).send_blocking().unwrap_err();
    assert_eq!(e.kind, ErrorKind::Offline);
    assert_eq!(s.hits.load(Ordering::SeqCst), 0);
    let c = Client::new(fast()).unwrap();
    for bad in ["file:///etc/passwd", "ftp://h/", "http://user:pw@127.0.0.1/", "not a url"] {
        assert_eq!(c.get(bad).send_blocking().unwrap_err().kind, ErrorKind::InvalidUrl, "{bad}");
    }
    assert_eq!(c.get(&s.url).header("Bad Name", "x").send_blocking().unwrap_err().kind, ErrorKind::InvalidUrl);
}

fn tls_file(name: &str) -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/tls").join(name)).unwrap()
}

/// HTTPS end to end: a rustls server with the test CA's certificate,
/// trusted through `extra_roots`; without it the handshake fails.
#[test]
fn https_with_a_trusted_root() {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    let cfg = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(tls_file("server.der"))],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(tls_file("server.key.der"))),
        )
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(cfg));
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
    let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let port = listener.local_addr().unwrap().port();
    rt.spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        loop {
            let Ok((tcp, _)) = listener.accept().await else { return };
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else { return };
                let mut buf = vec![0u8; 4096];
                let mut got = Vec::new();
                while !got.windows(4).any(|w| w == b"\r\n\r\n") {
                    match tls.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => got.extend_from_slice(&buf[..n]),
                    }
                }
                let _ = tls.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nsecure").await;
                let _ = tls.shutdown().await;
            });
        }
    });
    let trusted = Client::new(Config { extra_roots: vec![tls_file("ca.der")], ..fast() }).unwrap();
    for host in ["localhost", "127.0.0.1"] {
        let r = trusted.get(&format!("https://{host}:{port}/")).send_blocking().unwrap();
        assert_eq!((r.status, r.text().as_str()), (200, "secure"), "{host}");
    }
    let stranger = Client::new(Config { retries: 0, ..fast() }).unwrap();
    let e = stranger.get(&format!("https://localhost:{port}/")).send_blocking().unwrap_err();
    assert_eq!(e.kind, ErrorKind::Tls, "{e}");
    drop(rt);
}

/// The real thing, opt-in: `cargo test -p lain-http -- --ignored`.
#[test]
#[ignore = "reaches the network"]
fn live_https() {
    let c = Client::new(Config::default()).unwrap();
    let r = c.get("https://api.tvmaze.com/shows/1").send_blocking().unwrap().ok().unwrap();
    assert!(r.text().contains("\"id\":1"));
}
