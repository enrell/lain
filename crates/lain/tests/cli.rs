//! The `lain` binary end to end: `serve` with the SPA fallback, then
//! the client commands against it (login, language, player, offline
//! copies, dry-run watch, online backup and restore, logout), and the
//! service plan. No player is launched and nothing reaches the network.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use lain_client::types::NewLibrary;
use lain_client::{Client, ClientConfig, MemoryTokenStore};

const BIN: &str = env!("CARGO_BIN_EXE_lain");

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lain-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A password made at run time (no credential literal in the source).
fn pw() -> String {
    format!("{:08}x", std::process::id())
}

struct Serve {
    child: Child,
    url: String,
}

impl Drop for Serve {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// GET over plain HTTP/1.0: (status, headers, body).
fn get(url: &str, path: &str) -> (u16, String, Vec<u8>) {
    let addr = url.trim_start_matches("http://");
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(s, "GET {path} HTTP/1.0\r\nHost: {addr}\r\n\r\n").unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&raw[..split]).to_lowercase();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, head, raw[split + 4..].to_vec())
}

fn serve(data: &Path) -> Serve {
    let port = free_port();
    let child = Command::new(BIN)
        .args(["serve", "--data-dir"])
        .arg(data)
        .args(["--port", &port.to_string()])
        .env("LAIN_WATCH", "0")
        .env("LAIN_AUTO_ENRICH", "0")
        .env("LAIN_LIST_SYNC", "0")
        .env("LAIN_ACQUIRE_LISTEN_PORT", "0")
        .env("LAIN_OMARCHY_COLORS", data.join("no-colors.toml"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let s = Serve { child, url: format!("http://127.0.0.1:{port}") };
    let deadline = Instant::now() + Duration::from_secs(30);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "lain serve did not start");
        std::thread::sleep(Duration::from_millis(50));
    }
    s
}

struct Cli {
    home: PathBuf,
}

impl Cli {
    fn run(&self, args: &[&str]) -> Output {
        Command::new(BIN)
            .args(args)
            .env("LAIN_CLIENT_HOME", &self.home)
            .env("LAIN_PASSWORD", pw())
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    /// Runs and expects success; returns stdout.
    fn ok(&self, args: &[&str]) -> String {
        let o = self.run(args);
        let out = String::from_utf8_lossy(&o.stdout).into_owned();
        assert!(o.status.success(), "lain {args:?} failed: {out}{}", String::from_utf8_lossy(&o.stderr));
        out
    }

    /// Runs and expects failure; returns stderr.
    fn fails(&self, args: &[&str]) -> String {
        let o = self.run(args);
        assert!(!o.status.success(), "lain {args:?} should fail: {}", String::from_utf8_lossy(&o.stdout));
        String::from_utf8_lossy(&o.stderr).into_owned()
    }
}

const EP1: &[u8] = b"episode one bytes, a stand-in for a video file";
const EP2: &[u8] = b"episode two, longer: 0123456789abcdefghijklmnopqrstuvwxyz";

#[test]
fn serve_and_the_client_commands() {
    let root = temp("e2e");
    let media = root.join("media/Frieren");
    std::fs::create_dir_all(&media).unwrap();
    std::fs::write(media.join("[Fansub-A] Frieren - 01 [1080p].mkv"), EP1).unwrap();
    std::fs::write(media.join("[Fansub-A] Frieren - 02 [1080p].mkv"), EP2).unwrap();
    let data = root.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let roots = serde_json::json!({"format": "lain.roots@1", "roots": {"media": root.join("media").to_string_lossy()}});
    std::fs::write(data.join("roots.json"), serde_json::to_vec(&roots).unwrap()).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/fixtures/catalog.json");
    let cli = Cli { home: root.join("client") };
    let out = cli.ok(&["import", "--in", &fixture.to_string_lossy(), "--data-dir", &data.to_string_lossy()]);
    assert!(out.contains("imported 2 items"), "{out}");

    let srv = serve(&data);
    // The SPA owns everything the API does not: the shell (or the
    // not-built notice), JSON for unknown API paths, 405 elsewhere.
    let (status, head, body) = get(&srv.url, "/library/anything");
    let (root_status, _, root_body) = get(&srv.url, "/");
    assert_eq!((status, &body), (root_status, &root_body));
    assert!(status == 200 || status == 503, "{status}");
    assert!(head.contains("content-type: text/html; charset=utf-8"), "{head}");
    if status == 200 {
        assert!(head.contains("cache-control: no-cache"), "{head}");
    }
    let (status, head, body) = get(&srv.url, "/api/no-such-route");
    assert_eq!((status, body.as_slice()), (404, &b"{\"error\":\"not found\"}\n"[..]));
    assert!(head.contains("application/json"));
    assert_eq!(get(&srv.url, "/_app/immutable/missing.js").0, 404);

    // First admin and a library over the fixture, through lain-client.
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let c = Client::new(ClientConfig::new(&srv.url), Arc::new(MemoryTokenStore::new())).unwrap();
        c.setup("admin", &pw()).await.unwrap();
        c.login("admin", &pw()).await.unwrap();
        c.create_library(NewLibrary {
            name: "Anime".into(),
            root: "media".into(),
            path: "Frieren".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    });

    assert!(cli.fails(&["language"]).contains("not logged in"));
    let out = cli.ok(&["login", "--server", &srv.url, "--username", "admin"]);
    assert!(out.contains("logged in as admin on http://127.0.0.1:"), "{out}");
    assert_eq!(cli.ok(&["language", "jpn"]).trim(), "preferred language: jpn");
    assert_eq!(cli.ok(&["language"]).trim(), "jpn");
    assert!(cli.fails(&["language", "ja"]).contains("three-letter"));
    assert_eq!(cli.ok(&["language", "--clear"]).trim(), "preferred language cleared");
    assert_eq!(cli.ok(&["player"]).trim(), "mpv");
    if !cfg!(windows) {
        assert_eq!(cli.ok(&["player", "vlc"]).trim(), "default player: vlc");
        assert_eq!(cli.ok(&["player"]).trim(), "vlc");
        cli.ok(&["player", "mpv"]);
    }
    assert!(cli.fails(&["player", "totem"]).contains("usage"));

    // Offline copies.
    let out = cli.ok(&["download", "config", "--max", "1GiB", "--min-free", "0", "--retries", "2"]);
    assert!(out.contains("max            1.0 GiB") && out.contains("min free       none"), "{out}");
    assert!(out.contains("retries        2"), "{out}");
    assert!(cli.ok(&["download", "list"]).contains("offline store is empty"));
    let out = cli.ok(&["download", "add", "--id", "item-frieren-01"]);
    assert!(out.contains("queued: Frieren S00E01") && out.contains("done: Frieren S00E01"), "{out}");
    let out = cli.ok(&["download", "add", "--id", "item-frieren-01", "--rest", "--no-run"]);
    assert!(out.contains("queued: Frieren S00E02"), "{out}");
    assert!(cli.ok(&["download", "pause", "S00E02"]).contains("paused: Frieren S00E02"));
    assert!(cli.ok(&["download", "list"]).contains("paused"));
    assert!(cli.fails(&["download", "pause", "Frieren"]).contains("matches 2 entries"));
    cli.ok(&["download", "resume", "S00E02"]);
    assert!(cli.ok(&["download", "run"]).contains("done: Frieren S00E02"));
    let list = cli.ok(&["download", "list"]);
    assert_eq!(list.matches("done ").count(), 2, "{list}");
    let copy = cli.home.join("offline/[Fansub-A] Frieren - 02 [1080p].mkv");
    assert_eq!(std::fs::read(&copy).unwrap(), EP2);
    cli.ok(&["download", "sync"]);

    // A dry run names the offline copy, and never the token.
    let out = cli.ok(&["watch", "--id", "item-frieren-01", "--dry-run"]);
    assert!(out.contains("[offline copy]") && out.contains("Frieren S00E01"), "{out}");
    assert!(cli.fails(&["watch", "--player", "totem"]).contains("unsupported player"));
    let out = cli.ok(&["download", "rm", "S00E01"]);
    assert!(out.contains(&format!("freed {} B", EP1.len())), "{out}");
    let out = cli.ok(&["watch", "--id", "item-frieren-01", "--dry-run"]);
    assert!(out.contains("[authenticated stream URL]") && !out.contains("token"), "{out}");
    assert!(cli.ok(&["download", "gc"]).contains("freed"));
    let link = format!("lain://play?server={}&id=a&player=mpv", "http://10.9.9.9:9360");
    assert!(cli.fails(&["open-url", &link]).contains("differs from the local login"));

    // Online backup, then restore into an empty data dir.
    let bk = root.join("backups");
    let out = cli.ok(&["backup", "--out", &bk.to_string_lossy()]);
    assert!(out.contains("backup (online)"), "{out}");
    let tar = std::fs::read_dir(&bk).unwrap().next().unwrap().unwrap().path();
    assert!(tar.to_string_lossy().ends_with(".tar"));
    let restored = root.join("restored");
    assert!(
        cli.ok(&["restore", &tar.to_string_lossy(), "--data-dir", &restored.to_string_lossy()]).contains("restored")
    );
    assert!(restored.join("lain.redb").is_file() && restored.join("roots.json").is_file());
    assert!(
        cli.fails(&["restore", &tar.to_string_lossy(), "--data-dir", &restored.to_string_lossy()])
            .contains("not empty")
    );

    assert_eq!(cli.ok(&["logout"]).trim(), "logged out");
    assert!(cli.fails(&["language"]).contains("not logged in"));
    drop(srv);

    // With the server stopped, a backup reads the data dir directly.
    let out =
        cli.ok(&["backup", "--out", &root.join("offline-bk").to_string_lossy(), "--data-dir", &data.to_string_lossy()]);
    assert!(out.contains("backup (offline)"), "{out}");
    let out = cli.ok(&["export", "--data-dir", &data.to_string_lossy()]);
    assert!(out.contains("item-frieren-02"));

    let out = cli.ok(&["service", "install", "--dry-run", "--os", "linux", "--data-dir", "/srv/lain"]);
    // Windows makes the path absolute on its current drive.
    assert!(out.contains("serve --data-dir ") && out.contains("srv/lain --port 9360 --bind 127.0.0.1"), "{out}");
    assert!(cli.ok(&["version"]).starts_with("lain "));
    let _ = std::fs::remove_dir_all(&root);
}
