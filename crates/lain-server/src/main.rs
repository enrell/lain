//! `lain-rs`: the lain server (D-171).

use std::path::{Path, PathBuf};

use lain_server::VERSION;
use lain_server::app::{App, Config};
use lain_server::datadir::{Os, default_data_dir};
use lain_server::logging::{self, Sink};
use lain_server::service::{self, Action, Host, Spec};

const DEFAULT_PORT: &str = "9360";

fn usage() {
    eprintln!(
        "usage: lain-rs <command> [flags]

  serve     run the server (--data-dir, --port {DEFAULT_PORT}, --bind 127.0.0.1, --log-level info, --log-file,
            --watch=0 or LAIN_WATCH=0 stops library watching, LAIN_AUTO_ENRICH=0 the post-scan metadata pass)
  export    write the catalog as lain.catalog-export@3 (--data-dir, --out FILE; default stdout)
  import    replace the catalog with a lain.catalog-export@3 document (--data-dir, --in FILE)
  service   install|uninstall|start|stop|status|logs the background service
            (--name lain, --system, --linger, --port, --bind, --data-dir, --dry-run; logs: --follow, --lines N)
  version   print version

The data dir is --data-dir, else $LAIN_HOME, else the per-OS default
(~/.local/share/lain, ~/Library/Application Support/lain,
%LOCALAPPDATA%\\lain)."
    );
}

/// `--name value` or `--name=value`, like the Go CLI.
fn flag(args: &[String], name: &str) -> Option<String> {
    let long = format!("--{name}");
    let eq = format!("--{name}=");
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if *a == long {
            return it.next().cloned();
        }
        if let Some(v) = a.strip_prefix(&eq) {
            return Some(v.to_string());
        }
    }
    None
}

fn has(args: &[String], name: &str) -> bool {
    args.iter().any(|a| *a == format!("--{name}"))
}

fn data_dir(args: &[String]) -> Result<PathBuf, String> {
    match flag(args, "data-dir") {
        Some(d) => Ok(PathBuf::from(d)),
        None => default_data_dir(),
    }
}

fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().map(|c| c.join(p)).unwrap_or_else(|_| p.to_path_buf())
    }
}

fn cmd_serve(args: &[String]) -> Result<(), String> {
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
    let off = |v: Option<String>| v.is_some_and(|v| v == "0" || v == "false" || v == "FALSE");
    cfg.auto_enrich = !off(std::env::var("LAIN_AUTO_ENRICH").ok());
    cfg.watch = !off(flag(args, "watch").or_else(|| std::env::var("LAIN_WATCH").ok()));
    cfg.metadata_endpoints = lain_metadata::Endpoints::from_env();
    let app = App::open(cfg)?;
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(async move {
        app.start_background();
        let listener = tokio::net::TcpListener::bind((bind.as_str(), port))
            .await
            .map_err(|e| format!("listen {bind}:{port}: {e}"))?;
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

fn cmd_import(args: &[String]) -> Result<(), String> {
    let file = flag(args, "in").ok_or("import needs --in <export.json>")?;
    let dir = data_dir(args)?;
    let raw = std::fs::read(&file).map_err(|e| format!("{file}: {e}"))?;
    let doc = serde_json::from_slice(&raw).map_err(|e| format!("{file}: {e}"))?;
    let n = lain_server::import(&dir, &doc)?;
    println!("imported {n} items into {}", dir.display());
    Ok(())
}

fn cmd_export(args: &[String]) -> Result<(), String> {
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

fn cmd_service(args: &[String]) -> Result<(), String> {
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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        usage();
        std::process::exit(2);
    };
    let rest = &args[1..];
    let result = match cmd.as_str() {
        "serve" => cmd_serve(rest),
        "import" => cmd_import(rest),
        "export" => cmd_export(rest),
        "service" => cmd_service(rest),
        "version" | "--version" | "-V" => {
            println!("lain-rs {VERSION}");
            Ok(())
        }
        "help" | "-h" | "--help" => {
            usage();
            Ok(())
        }
        _ => {
            usage();
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
