//! `lain watch`, `lain open-url` and `lain install-player-handler`
//! (D-237): play library items in mpv or VLC on this machine.
//!
//! The item resolves through the API (an id, the next unfinished
//! continue-watching entry, or a search); an episode starts a queue of
//! the rest of its title in watch order (D-056). Each entry plays from
//! its offline copy when one is finished, else from the authenticated
//! stream URL (the token rides the player's argv while it runs). The
//! player runs under `lain-watch`'s local-play manager, the same code
//! the server's local playback uses: mpv reports through the bundled
//! Lua script, VLC through its private RC socket, and each measured
//! position goes through `lain-client`'s progress sync, so a position
//! taken while the server is unreachable is queued and pushed later.

use std::sync::{Mutex, mpsc};

use lain_api::{CatalogItem, ProgressUpdate, TrackPreferences};
use lain_client::{Client, RecordOutcome, SearchQuery};
use lain_watch::localplay::{Entry, Manager, PlayError, PlayRequest, subtitle_off};

use crate::args::{flag, has, positional};
use crate::client::{Session, runtime};

pub fn label(title: &str, season: i64, episode: i64) -> String {
    if episode > 0 { format!("{title} S{season:02}E{episode:02}") } else { title.to_string() }
}

pub fn item_label(i: &CatalogItem) -> String {
    label(&i.title, i.season, i.episode)
}

/// `mpv` or `vlc`, from `--player` or the saved default.
pub fn choose_player(args: &[String], saved: &str) -> Result<String, String> {
    let p = flag(args, "player").unwrap_or_else(|| saved.to_string());
    let p = if p.is_empty() { "mpv".to_string() } else { p };
    if p != "mpv" && p != "vlc" {
        return Err(format!("unsupported player {p:?} (choose mpv or vlc)"));
    }
    if p == "vlc" && cfg!(windows) {
        return Err("VLC reports progress through a Unix socket, which Windows lacks; use mpv".into());
    }
    Ok(p)
}

/// One pick from several search hits: `--pick N`, else asked on stdin.
pub fn pick(items: &[CatalogItem], pick: Option<usize>) -> Result<CatalogItem, String> {
    if let Some(n) = pick.filter(|n| *n >= 1 && *n <= items.len()) {
        return Ok(items[n - 1].clone());
    }
    let shown = &items[..items.len().min(20)];
    for (i, it) in shown.iter().enumerate() {
        let mut line = format!("{:2}  {}", i + 1, it.title);
        if it.episode > 0 {
            line.push_str(&format!(" S{:02}E{:02}", it.season, it.episode));
        }
        if it.year > 0 {
            line.push_str(&format!(" ({})", it.year));
        }
        println!("{line}");
    }
    use std::io::Write;
    print!("pick [1-{}]: ", shown.len());
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).map_err(|e| e.to_string())?;
    match line.trim().parse::<usize>() {
        Ok(n) if n >= 1 && n <= shown.len() => Ok(shown[n - 1].clone()),
        _ => Err("invalid pick".into()),
    }
}

/// A search that resolves to one item.
pub async fn resolve_query(c: &Client, query: &str, n: Option<usize>) -> Result<CatalogItem, String> {
    let page = c
        .search(SearchQuery { q: query.to_string(), limit: 500, ..Default::default() })
        .await
        .map_err(|e| e.to_string())?;
    match page.items.len() {
        0 => Err(format!("no results for {query:?}")),
        1 => Ok(page.items[0].clone()),
        _ => pick(&page.items, n),
    }
}

/// The first unfinished continue-watching entry.
pub async fn resolve_next(c: &Client) -> Result<CatalogItem, String> {
    for p in c.continue_watching().await.map_err(|e| e.to_string())? {
        if p.completed {
            continue;
        }
        if let Ok(item) = c.catalog_item(&p.item_id).await {
            return Ok(item);
        }
    }
    Err("nothing to continue (watch something first, or pass a search)".into())
}

/// The episode and every later one of its title, in watch order;
/// missing files stay out of the player.
pub async fn episode_queue(c: &Client, start: &CatalogItem) -> Result<Vec<CatalogItem>, String> {
    if start.episode == 0 {
        return Ok(vec![start.clone()]);
    }
    let all = c.episodes(&start.id).await.map_err(|e| e.to_string())?;
    let Some(at) = all.iter().position(|i| i.id == start.id) else {
        return Err("selected episode is not available in its title".into());
    };
    let q: Vec<CatalogItem> = all[at..].iter().filter(|i| !i.missing).cloned().collect();
    if q.is_empty() {
        return Err("selected episode is not available in its title".into());
    }
    Ok(q)
}

/// The item `lain watch` / `lain download add` starts from.
pub async fn resolve(c: &Client, args: &[String], allow_next: bool) -> Result<CatalogItem, String> {
    if let Some(id) = flag(args, "id") {
        return c.catalog_item(&id).await.map_err(|e| e.to_string());
    }
    let query = positional(args).join(" ").trim().to_string();
    if allow_next && (query.is_empty() || has(args, "next")) {
        return resolve_next(c).await;
    }
    if query.is_empty() {
        return Err("name what you want (a search query or --id)".into());
    }
    let n = flag(args, "pick").and_then(|v| v.parse().ok());
    resolve_query(c, &query, n).await
}

/// Where a resumed play starts: completed or under 5 s starts over.
pub fn resume_at(position: f64, completed: bool) -> f64 {
    if completed || !position.is_finite() || position < 5.0 { 0.0 } else { position }
}

/// One planned playlist entry.
pub struct Planned {
    pub item: CatalogItem,
    pub entry: Entry,
    /// The offline copy that plays, if any.
    pub local: Option<String>,
}

/// What a play session saved for one item.
#[derive(Debug, Clone, PartialEq)]
pub struct Saved {
    pub item_id: String,
    pub update: ProgressUpdate,
    /// Queued locally because the server did not answer.
    pub queued: bool,
}

pub fn completed(pos: f64, dur: f64) -> bool {
    dur > 0.0 && pos / dur >= 0.95
}

enum Msg {
    Save(String, f64, f64),
    Done(Result<(), String>),
}

/// Runs the player over `entries` until it exits, recording every
/// measured position through `record`. Returns the last record per item.
pub fn run_player(
    player: &str,
    entries: Vec<Entry>,
    tracks: &TrackPreferences,
    mut record: impl FnMut(&str, ProgressUpdate) -> bool,
) -> Result<Vec<Saved>, String> {
    let (tx, rx) = mpsc::channel();
    let save_tx = Mutex::new(tx.clone());
    let done_tx = tx;
    let manager = Manager::new();
    let req = PlayRequest {
        player: player.to_string(),
        entries,
        tracks: tracks.clone(),
        save: Box::new(move |id, pos, dur| {
            let _ = save_tx.lock().unwrap_or_else(|e| e.into_inner()).send(Msg::Save(id.to_string(), pos, dur));
        }),
        done: Some(Box::new(move |r| {
            let _ = done_tx.send(Msg::Done(r));
        })),
    };
    manager.play(req).map_err(|e| match e {
        PlayError::NoPlayer => {
            "no graphical session to open a player in (DISPLAY and WAYLAND_DISPLAY are unset)".into()
        }
        other => other.to_string(),
    })?;
    let mut saved: Vec<Saved> = Vec::new();
    let result = loop {
        match rx.recv() {
            Ok(Msg::Save(id, pos, dur)) => {
                let update = ProgressUpdate { position_sec: pos, duration_sec: dur, completed: completed(pos, dur) };
                let queued = !record(&id, update);
                match saved.iter_mut().find(|s| s.item_id == id) {
                    Some(s) => (s.update, s.queued) = (update, queued),
                    None => saved.push(Saved { item_id: id, update, queued }),
                }
            }
            Ok(Msg::Done(r)) => break r,
            Err(_) => break Err(format!("{player} monitor stopped")),
        }
    };
    manager.close();
    // A position was recorded: the session did its job even if the
    // player exited non-zero (mpv does on a plain quit).
    match result {
        Err(e) if saved.is_empty() => Err(e),
        _ => Ok(saved),
    }
}

/// Prints what a session saved.
pub fn report(saved: &[Saved], labels: &dyn Fn(&str) -> String) {
    for s in saved {
        let u = s.update;
        let mark = if u.completed { " (completed)" } else { "" };
        let what = if s.queued { "kept offline" } else { "saved" };
        println!("{what}: {} at {:.0}s / {:.0}s{mark}", labels(&s.item_id), u.position_sec, u.duration_sec);
    }
    if saved.iter().any(|s| s.queued) {
        println!("server unreachable; `lain download sync` pushes the position later");
    }
}

/// Records one position through the progress sync; `false` when it was
/// queued (or refused) instead of written.
pub fn recorder<'a>(
    rt: &'a tokio::runtime::Runtime,
    sync: &'a lain_client::ProgressSync,
) -> impl FnMut(&str, ProgressUpdate) -> bool + 'a {
    move |id, u| match rt.block_on(sync.record(id, u)) {
        Ok(RecordOutcome::Synced { .. }) => true,
        Ok(RecordOutcome::KeptServer { .. }) => true,
        Ok(RecordOutcome::Queued { .. }) => false,
        Err(e) => {
            eprintln!("warning: could not record progress: {e}");
            false
        }
    }
}

pub fn watch(args: &[String]) -> Result<(), String> {
    let s = Session::open(args)?;
    let player = choose_player(args, s.cfg.player())?;
    let dry = has(args, "dry-run");
    let once = has(args, "once");
    let rt = runtime()?;
    let c = &s.client;
    let tracks = if dry {
        TrackPreferences::default()
    } else {
        rt.block_on(c.me()).map_err(|e| e.to_string())?.track_preferences()
    };
    let start = rt.block_on(resolve(c, args, true))?;
    let items = if !once && !dry && start.episode > 0 { rt.block_on(episode_queue(c, &start))? } else { vec![start] };
    let db = s.home.db()?;
    let store = s.home.offline(&s.cfg, &db)?;
    let sync = s.sync(&db);
    let server = c.server();
    let mut planned = Vec::new();
    for item in items {
        let local = store.local_path(&server, &item.id).map_err(|e| e.to_string())?;
        let path = match &local {
            Some(p) => p.clone(),
            None => c.stream_url(&item.id).map_err(|e| e.to_string())?,
        };
        let (resume, subs_off) = if dry {
            (0.0, false)
        } else {
            let p = rt.block_on(sync.fetch(&item.id)).map_err(|e| e.to_string())?;
            let off = player == "vlc"
                && !tracks.is_empty()
                && (tracks.subtitle_mode == "off"
                    || rt
                        .block_on(c.playback_plan(&item.id, "vlc"))
                        .map(|plan| subtitle_off(&plan.streams, &tracks))
                        .map_err(|e| e.to_string())?);
            (resume_at(p.position_sec, p.completed), off)
        };
        planned.push(Planned { entry: Entry { item_id: item.id.clone(), path, resume, subs_off }, item, local });
    }
    if dry {
        print!("{}", dry_run(&player, &planned));
        return Ok(());
    }
    let first = &planned[0].item;
    if planned.len() > 1 {
        println!("playing {} episodes from {}", planned.len(), item_label(first));
    } else {
        println!("playing: {}", item_label(first));
    }
    let labels: Vec<(String, String)> = planned.iter().map(|p| (p.item.id.clone(), item_label(&p.item))).collect();
    let entries = planned.into_iter().map(|p| p.entry).collect();
    let saved = run_player(&player, entries, &tracks, recorder(&rt, &sync))?;
    report(&saved, &|id| labels.iter().find(|(i, _)| i == id).map(|(_, l)| l.clone()).unwrap_or_default());
    Ok(())
}

/// What `--dry-run` prints: the player and each entry, with the offline
/// copy's path when one would play and a placeholder for the stream URL
/// (it carries the token).
pub fn dry_run(player: &str, planned: &[Planned]) -> String {
    let mut out = format!("{player}: {} entr{}\n", planned.len(), if planned.len() == 1 { "y" } else { "ies" });
    for (i, p) in planned.iter().enumerate() {
        let media = match &p.local {
            Some(path) => format!("{path} [offline copy]"),
            None => "[authenticated stream URL]".into(),
        };
        out.push_str(&format!("{:3}  {}  {media}\n", i + 1, item_label(&p.item)));
    }
    out
}

// ---- external-player links -------------------------------------------------

fn query_unescape(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' => {
                out.push(u8::from_str_radix(s.get(i + 1..i + 3)?, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// A browser cannot start a local process, so the web UI hands this CLI
/// a `lain://play?server=..&id=..&player=..` link: an origin, an item id
/// and a player, never a token. Returns (server, id, player); `server`
/// must be the one this computer is logged in to (canonical forms).
pub fn parse_player_url(raw: &str, logged_in: &str) -> Result<(String, String, String), String> {
    let invalid = || "invalid Lain player link".to_string();
    let q = raw.strip_prefix("lain://play?").ok_or_else(invalid)?;
    if q.contains('#') {
        return Err(invalid());
    }
    let bad_params = || "invalid Lain player link parameters".to_string();
    let (mut server, mut id, mut player) = (None, None, None);
    for pair in q.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let (k, v) = (query_unescape(k).ok_or_else(bad_params)?, query_unescape(v).ok_or_else(bad_params)?);
        let slot = match k.as_str() {
            "server" => &mut server,
            "id" => &mut id,
            "player" => &mut player,
            _ => return Err(bad_params()),
        };
        if slot.replace(v).is_some() {
            return Err(bad_params());
        }
    }
    let (Some(server), Some(id), Some(player)) = (server, id, player) else { return Err(bad_params()) };
    let bad_server = || "invalid server in Lain player link".to_string();
    if !(server.starts_with("http://") || server.starts_with("https://")) {
        return Err(bad_server());
    }
    let url = lain_client::ServerUrl::parse(&server).map_err(|_| bad_server())?;
    if !url.base.is_empty() {
        return Err(bad_server());
    }
    let logged = lain_client::ServerUrl::parse(logged_in).map(|u| u.canonical()).unwrap_or_default();
    if url.canonical() != logged {
        return Err("this browser's server differs from the local login; run `lain login --server <web address>` on this computer first".into());
    }
    if id.is_empty() || id.contains(['/', '\\', '\0']) || (player != "mpv" && player != "vlc") {
        return Err("invalid item or player in Lain player link".into());
    }
    Ok((url.canonical(), id, player))
}

pub fn open_url(args: &[String]) -> Result<(), String> {
    let [raw] = args else { return Err("usage: lain open-url 'lain://play?...'".into()) };
    let s = Session::open(&[]).map_err(|_| "not logged in (run `lain login` on this computer)".to_string())?;
    let (server, id, player) = parse_player_url(raw, &s.client.server())?;
    watch(&["--id".into(), id, "--player".into(), player, "--server".into(), server])
}

/// Registers `lain open-url %u` for `lain:` links (freedesktop: a
/// `.desktop` entry and `xdg-mime`).
pub fn install_player_handler(_args: &[String]) -> Result<(), String> {
    if cfg!(any(windows, target_os = "macos")) {
        return Err("player links are registered on Linux and BSD desktops (xdg-mime) only for now".into());
    }
    let bin = std::env::current_exe().and_then(|p| p.canonicalize()).map_err(|e| e.to_string())?;
    let data = match std::env::var("XDG_DATA_HOME") {
        Ok(d) if !d.is_empty() => std::path::PathBuf::from(d),
        _ => std::path::PathBuf::from(std::env::var("HOME").map_err(|_| "HOME is not set")?).join(".local/share"),
    };
    let dir = data.join("applications");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join("lain-player.desktop");
    std::fs::write(&path, desktop_entry(&bin.to_string_lossy())).map_err(|e| format!("{}: {e}", path.display()))?;
    let out = std::process::Command::new("xdg-mime")
        .args(["default", "lain-player.desktop", "x-scheme-handler/lain"])
        .output()
        .map_err(|e| format!("register Lain player handler: xdg-mime: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "register Lain player handler: {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    println!("Lain player links registered for this user");
    Ok(())
}

/// Desktop `Exec` quoting is not shell quoting: backslashes and quotes
/// are escaped, and `%` is doubled so it is not a field code.
pub fn desktop_entry(bin: &str) -> String {
    let quoted = bin.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%");
    format!(
        "[Desktop Entry]\nType=Application\nName=Lain external player\nNoDisplay=true\nTerminal=true\nExec=\"{quoted}\" open-url %u\nMimeType=x-scheme-handler/lain;\n"
    )
}

/// Shared by `lain download play`: plays one local file with no server.
pub fn play_local(
    player: &str,
    item_id: &str,
    path: &str,
    resume: f64,
    record: impl FnMut(&str, ProgressUpdate) -> bool,
) -> Result<Vec<Saved>, String> {
    let entry = Entry { item_id: item_id.to_string(), path: path.to_string(), resume, subs_off: false };
    run_player(player, vec![entry], &TrackPreferences::default(), record)
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: &str = "http://127.0.0.1:9360";

    #[test]
    fn player_links() {
        let ok = parse_player_url("lain://play?server=http%3A%2F%2F127.0.0.1%3A9360&id=abc&player=vlc", S).unwrap();
        assert_eq!(ok, (S.to_string(), "abc".to_string(), "vlc".to_string()));
        // A trailing slash is the same origin.
        assert!(parse_player_url("lain://play?server=http://127.0.0.1:9360/&id=a&player=mpv", S).is_ok());
        for bad in [
            "http://play?server=x&id=a&player=mpv",
            "lain://play/?server=http://127.0.0.1:9360&id=a&player=mpv",
            "lain://play?server=http://127.0.0.1:9360&id=a&player=mpv#x",
            "lain://play?server=http://127.0.0.1:9360&id=a",
            "lain://play?server=http://127.0.0.1:9360&id=a&id=b&player=mpv",
            "lain://play?server=http://127.0.0.1:9360&id=a&player=mpv&token=t",
            "lain://play?server=ftp://127.0.0.1:9360&id=a&player=mpv",
            "lain://play?server=http://u@127.0.0.1:9360&id=a&player=mpv",
            "lain://play?server=http://127.0.0.1:9360/base&id=a&player=mpv",
            "lain://play?server=http://127.0.0.1:9360&id=a%2Fb&player=mpv",
            "lain://play?server=http://127.0.0.1:9360&id=a&player=totem",
            "lain://play?server=http://127.0.0.1:9360&id=&player=mpv",
        ] {
            assert!(parse_player_url(bad, S).is_err(), "{bad}");
        }
        let e = parse_player_url("lain://play?server=http://10.0.0.2:9360&id=a&player=mpv", S).unwrap_err();
        assert!(e.contains("lain login --server"), "{e}");
    }

    #[test]
    fn desktop_entries_quote_the_path() {
        let e = desktop_entry("/opt/my \"lain\"/100%/lain");
        assert!(e.contains("Exec=\"/opt/my \\\"lain\\\"/100%%/lain\" open-url %u\n"), "{e}");
        assert!(e.contains("MimeType=x-scheme-handler/lain;\n"));
    }

    #[test]
    fn resume_and_completion() {
        assert_eq!(resume_at(4.9, false), 0.0);
        assert_eq!(resume_at(120.0, false), 120.0);
        assert_eq!(resume_at(120.0, true), 0.0);
        assert_eq!(resume_at(f64::NAN, false), 0.0);
        assert!(completed(95.0, 100.0) && !completed(94.0, 100.0) && !completed(1.0, 0.0));
        assert_eq!(label("Frieren", 1, 2), "Frieren S01E02");
        assert_eq!(label("Film", 0, 0), "Film");
    }

    #[test]
    fn dry_runs_never_print_the_token() {
        let item = CatalogItem { id: "a".into(), title: "Frieren".into(), season: 1, episode: 3, ..Default::default() };
        let p = vec![
            Planned {
                item: item.clone(),
                entry: Entry {
                    item_id: "a".into(),
                    path: "http://h/api/items/a/stream?token=SECRET".into(),
                    ..Default::default()
                },
                local: None,
            },
            Planned { item, entry: Entry::default(), local: Some("/c/Frieren 03.mkv".into()) },
        ];
        let out = dry_run("mpv", &p);
        assert!(!out.contains("SECRET"), "{out}");
        assert!(
            out.contains("[authenticated stream URL]") && out.contains("/c/Frieren 03.mkv [offline copy]"),
            "{out}"
        );
    }
}
