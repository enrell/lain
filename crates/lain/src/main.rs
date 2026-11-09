//! `lain`: the lain media server and its command-line client in one
//! binary (D-171, D-237). The server commands run where the data dir
//! is; the client commands talk to a server through `lain-client`.

mod args;
mod backup;
mod client;
mod download;
mod prompt;
mod server;
mod watch;

use lain_server::VERSION;

fn usage() {
    eprintln!(
        "usage: lain <command> [flags]

server
  serve     run the server (--data-dir, --port {port}, --bind 127.0.0.1, --log-level info, --log-file,
            --watch=0 or LAIN_WATCH=0 stops library watching, --list-sync=0 or LAIN_LIST_SYNC=0 the
            scheduled list import, LAIN_AUTO_ENRICH=0 the post-scan metadata pass)
  service   install|uninstall|start|stop|status|logs the background service
            (--name lain, --system, --linger, --port, --bind, --data-dir, --dry-run; logs: --follow, --lines N)
  export    write the catalog as lain.catalog-export@3 (--data-dir, --out FILE; default stdout)
  import    replace the catalog with a lain.catalog-export@3 document (--data-dir, --in FILE)
  backup    snapshot the server: online with a saved login, else from a stopped data dir
            (--out DIR|FILE, default backups/; --data-dir, --server)
  restore   restore a backup into an empty data dir (restore BACKUP.tar [--data-dir])
  component list | run <id> --sock S [--k v...] | install <id> [--data-dir] [--k v...] | uninstall <id>
            the bridges to standalone apps (lain-parser, AnimeSubs, Anything) as components; install writes
            <data-dir>/plugins/<id>.json, which the running server picks up
  plugins   list the providers and the composition of a stopped data dir (--data-dir)
  doctor    report what this machine offers: ffmpeg/ffprobe, players, transcode probe (--data-dir)

client
  login     sign in to a server (--server http://127.0.0.1:{port}, --username; password from
            LAIN_PASSWORD or a prompt)
  logout    forget the saved session (--server)
  watch     play in mpv or VLC ([query] [--id ID] [--next] [--once] [--pick N] [--player mpv|vlc]
            [--server URL] [--dry-run]); episodes queue the rest of their title
  download  keep offline copies (add, run, list, pause, resume, cancel, rm, play, sync, gc, config)
  player    show or set the default player (player [mpv|vlc])
  language  show or set your account's preferred language (language [ISO 639-2 code|--clear])
  open-url  play a lain://play link from the web UI
  install-player-handler  register this binary for the web UI's external-player links

  version   print version

The data dir is --data-dir, else $LAIN_HOME, else the per-OS default
(~/.local/share/lain, ~/Library/Application Support/lain,
%LOCALAPPDATA%\\lain). Client state (sessions, offline copies, cli.json)
lives in $LAIN_CLIENT_HOME, else the per-OS lain-client directory.",
        port = server::DEFAULT_PORT
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        usage();
        std::process::exit(2);
    };
    let rest = &args[1..];
    let result = match cmd.as_str() {
        "serve" => server::serve(rest),
        "service" => server::service(rest),
        "import" => server::import(rest),
        "export" => server::export(rest),
        "component" => server::component(rest),
        "plugins" => server::plugins(rest),
        "doctor" => server::doctor(rest),
        "backup" => backup::backup(rest),
        "restore" => backup::restore(rest),
        "login" => client::login(rest),
        "logout" => client::logout(rest),
        "player" => client::player(rest),
        "language" => client::language(rest),
        "watch" => watch::watch(rest),
        "download" => download::download(rest),
        "open-url" => watch::open_url(rest),
        "install-player-handler" => watch::install_player_handler(rest),
        "version" | "--version" | "-V" => {
            println!("lain {VERSION}");
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
