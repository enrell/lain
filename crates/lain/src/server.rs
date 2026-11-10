//! The server-side commands: `serve`, `export`, `import`, `service`,
//! `plugins` and `doctor`. They run on the machine that holds the data
//! dir; the client commands live in `client.rs` and its siblings.

use std::path::{Path, PathBuf};

use serde_json::json;

use lain_server::VERSION;
use lain_server::app::{App, Config};
use lain_server::datadir::{Os, default_data_dir};
use lain_server::logging::{self, Sink};
use lain_server::service::{self, Action, Host, Spec};
use lain_server::webui::WebAssets;

use crate::args::{flag, has};

pub const DEFAULT_PORT: &str = "9360";

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/webui.rs"));
}

/// The web SPA compiled into this binary (D-235).
pub fn web() -> WebAssets {
    WebAssets::new(embedded::WEB_FILES)
}

pub fn data_dir(args: &[String]) -> Result<PathBuf, String> {
    match flag(args, "data-dir") {
        Some(d) => Ok(PathBuf::from(d)),
        None => default_data_dir(),
    }
}

pub fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().map(|c| c.join(p)).unwrap_or_else(|_| p.to_path_buf())
    }
}

fn off(v: Option<String>) -> bool {
    v.is_some_and(|v| v == "0" || v == "false" || v == "FALSE")
}

pub fn serve(args: &[String]) -> Result<(), String> {
    let dir = data_dir(args)?;
    let port: u16 = flag(args, "port").unwrap_or_else(|| DEFAULT_PORT.into()).parse().map_err(|_| "bad --port")?;
    let bind = flag(args, "bind").unwrap_or_else(|| "127.0.0.1".into());
    let level = logging::parse_level(
        &flag(args, "log-level").or_else(|| std::env::var("LAIN_LOG_LEVEL").ok()).unwrap_or_else(|| "info".into()),
    )?;
    let sink = match flag(args, "log-file") {
        Some(f) => {
            let f = PathBuf::from(f);
            if let Some(parent) = f.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            Sink::File(
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&f)
                    .map_err(|e| format!("{}: {e}", f.display()))?,
            )
        }
        None => Sink::Stderr,
    };
    logging::init(level, sink);
    let mut cfg = Config::new(&dir, VERSION);
    cfg.auto_enrich = !off(std::env::var("LAIN_AUTO_ENRICH").ok());
    cfg.watch = !off(flag(args, "watch").or_else(|| std::env::var("LAIN_WATCH").ok()));
    cfg.metadata_endpoints = lain_metadata::Endpoints::from_env();
    cfg.anilist_endpoints = lain_lists::anilist::Endpoints::from_env();
    cfg.list_sync = !off(flag(args, "list-sync").or_else(|| std::env::var("LAIN_LIST_SYNC").ok()));
    cfg.web = web();
    if !cfg.web.is_built() {
        tracing::warn!("this binary was built without the web UI (just web, then rebuild)");
    }
    let app = App::open(cfg)?;
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(async move {
        app.start_background();
        let listener = tokio::net::TcpListener::bind((bind.as_str(), port))
            .await
            .map_err(|e| format!("listen {bind}:{port}: {e}"))?;
        eprintln!("lain {VERSION} on http://{bind}:{port} (data {})", dir.display());
        lain_server::serve(app, listener, shutdown_signal()).await.map_err(|e| e.to_string())
    })
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
    tracing::info!("shutting down");
}

pub fn import(args: &[String]) -> Result<(), String> {
    let file = flag(args, "in").ok_or("import needs --in <export.json>")?;
    let dir = data_dir(args)?;
    let raw = std::fs::read(&file).map_err(|e| format!("{file}: {e}"))?;
    let doc = serde_json::from_slice(&raw).map_err(|e| format!("{file}: {e}"))?;
    let n = lain_server::import(&dir, &doc)?;
    println!("imported {n} items into {}", dir.display());
    Ok(())
}

pub fn export(args: &[String]) -> Result<(), String> {
    let dir = data_dir(args)?;
    let doc = lain_server::export(&dir)?;
    let raw = serde_json::to_vec_pretty(&doc).map_err(|e| e.to_string())?;
    match flag(args, "out") {
        Some(out) => std::fs::write(&out, raw).map_err(|e| format!("{out}: {e}")),
        None => {
            use std::io::Write;
            std::io::stdout().write_all(&raw).map_err(|e| e.to_string())
        }
    }
}

pub fn service(args: &[String]) -> Result<(), String> {
    let action_name = args.first().ok_or("service needs install|uninstall|start|stop|status|logs")?;
    let lines = flag(args, "lines").map(|n| n.parse().unwrap_or(200)).unwrap_or(200);
    let action = Action::parse(action_name, has(args, "follow"), lines)
        .ok_or_else(|| format!("unknown service action {action_name}"))?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let spec = Spec {
        name: flag(args, "name").unwrap_or_else(|| "lain".into()),
        exe: exe.canonicalize().unwrap_or(exe),
        data_dir: absolute(&data_dir(args)?),
        port: flag(args, "port").unwrap_or_else(|| DEFAULT_PORT.into()).parse().map_err(|_| "bad --port")?,
        bind: flag(args, "bind").unwrap_or_else(|| "127.0.0.1".into()),
        system: has(args, "system"),
        linger: has(args, "linger"),
    };
    let os = match flag(args, "os").as_deref() {
        Some("linux") => Os::Linux,
        Some("macos") => Os::MacOs,
        Some("windows") => Os::Windows,
        Some(other) => return Err(format!("unknown --os {other}")),
        None => Os::current(),
    };
    let plan = service::plan(os, &action, &spec, &Host::detect())?;
    if has(args, "dry-run") || os != Os::current() {
        print!("{}", service::describe(&plan));
        return Ok(());
    }
    service::execute(&plan)
}

/// `--k v` pairs after the bridge id (`--sock` and `--data-dir` of
/// install excluded by the caller).
fn bridge_options(args: &[String], skip: &[&str]) -> Result<lain_bridge::Options, String> {
    let mut opts = lain_bridge::Options::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let Some(k) = a.strip_prefix("--") else { return Err(format!("unexpected argument {a}")) };
        let (k, v) = match k.split_once('=') {
            Some((k, v)) => (k.to_string(), v.to_string()),
            None => (k.to_string(), it.next().cloned().ok_or_else(|| format!("--{k} needs a value"))?),
        };
        if !skip.contains(&k.as_str()) {
            opts.insert(k, v);
        }
    }
    Ok(opts)
}

pub fn component(args: &[String]) -> Result<(), String> {
    let sub = args.first().map(String::as_str).ok_or("component needs list|run|install|uninstall")?;
    if sub == "list" {
        for (id, caps, what) in lain_bridge::catalog() {
            println!("{id}\t{}\t{what}", caps.join(","));
        }
        return Ok(());
    }
    let id = args.get(1).ok_or_else(|| format!("component {sub} needs a bridge id"))?;
    let rest = &args[2..];
    match sub {
        "run" => {
            let sock = flag(rest, "sock").ok_or("component run needs --sock")?;
            logging::init(logging::parse_level("info")?, Sink::Stderr);
            lain_bridge::run(id, &sock, &bridge_options(rest, &["sock"])?)
        }
        "install" => {
            let dir = absolute(&data_dir(rest)?);
            let mut opts = bridge_options(rest, &["data-dir"])?;
            opts.insert("data-dir".into(), dir.to_string_lossy().into_owned());
            // Fail now, not in the server's log, when the options are wrong.
            lain_bridge::build(id, &opts)?;
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let m = lain_bridge::manifest(id, &exe.canonicalize().unwrap_or(exe), VERSION, &opts)?;
            let plugins = dir.join("plugins");
            std::fs::create_dir_all(&plugins).map_err(|e| format!("{}: {e}", plugins.display()))?;
            let path = plugins.join(format!("{id}.json"));
            lain_server::app::save_doc(&path, &m)?;
            println!("installed {id} -> {}", path.display());
            Ok(())
        }
        "uninstall" => {
            let path = absolute(&data_dir(rest)?).join("plugins").join(format!("{id}.json"));
            std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            println!("removed {}", path.display());
            Ok(())
        }
        other => Err(format!("unknown component action {other}")),
    }
}

/// `lain plugins`: the providers and the composition of a data dir,
/// read with the server stopped (the store is locked while it runs;
/// a running server answers `GET /api/plugins`).
pub fn plugins(args: &[String]) -> Result<(), String> {
    let dir = data_dir(args)?;
    let mut cfg = Config::new(&dir, VERSION);
    cfg.offline = true;
    cfg.watch = false;
    cfg.list_sync = false;
    cfg.auto_enrich = false;
    let app = App::open(cfg).map_err(|e| format!("{e} (is the server running? use GET /api/plugins)"))?;
    let out = json!({"composition": app.registry.composition(), "providers": app.registry.providers()});
    app.shutdown();
    println!("{}", serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?);
    Ok(())
}

/// `lain doctor`: what this machine offers the server and the CLI.
pub fn doctor(args: &[String]) -> Result<(), String> {
    let dir = data_dir(args)?;
    let present = |b: &str| lain_probe::exec::look_path(b).is_some();
    let mut rep = json!({
        "version": VERSION,
        "data_dir": dir.to_string_lossy(),
        "data_dir_present": dir.is_dir(),
        "web_ui": web().is_built(),
        "ffprobe": present("ffprobe"),
        "ffmpeg": present("ffmpeg"),
        "mpv": present("mpv"),
        "vlc": present("vlc"),
    });
    // What the local ffmpeg can actually do: hardware is opt-in and its
    // probe result is never hidden (D-031).
    if present("ffmpeg") {
        let caps = lain_watch::transcode::policy::probe_build(&lain_watch::settings::defaults());
        rep["transcode"] = json!(lain_watch::transcode::policy::report(&caps));
    }
    println!("{}", serde_json::to_string_pretty(&rep).map_err(|e| e.to_string())?);
    Ok(())
}
