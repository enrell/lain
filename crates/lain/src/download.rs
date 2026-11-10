//! `lain download`: offline copies on this machine (D-094, D-237).
//!
//! The store is `lain-client`'s ([`OfflineStore`]): resumable transfers
//! through the Range-capable stream endpoint, a byte budget and a
//! free-space floor, and LRU eviction of watched, synced copies. Copies
//! play with the server unreachable; their positions wait in the
//! progress queue until `lain download sync` (or the next online play)
//! pushes them.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lain_client::{Client, ClientError, FetchObserver, OfflineEntry, OfflineState, OfflineStore, Resolution};
use lain_downloads::human_bytes;

use crate::args::{byte_size, flag, has, positional};
use crate::client::{CliConfig, Home, MAX_RETRIES, Session, runtime};
use crate::watch::{choose_player, episode_queue, label, play_local, recorder, report, resolve, resume_at};

const USAGE: &str = "usage: lain download <command>

  add <query> [--id ID] [--rest] [--pick N] [--no-run]
                 queue an item (--rest: it and every later episode/chapter)
                 and download now; Ctrl+C stops, nothing is lost
  run            download everything queued, resuming partial files and
                 retrying dropped transfers with backoff (Ctrl+C stops)
  list           show the offline store and its usage
  pause|resume <id>
  cancel <id>    stop a download and delete its partial bytes
  rm <id> [--force]
                 delete a local copy (--force drops unsynced progress)
  play <id|title> [--player mpv|vlc]
                 play a local copy without the server
  sync           push offline progress and refresh watched state
  gc [--watched] delete orphaned partials and forgotten files;
                 --watched also deletes every watched, synced copy
  config [--dir DIR] [--max SIZE|0] [--min-free SIZE|0] [--evict-watched on|off]
         [--retries N]
                 show or change where copies live and how much they may use

<id> accepts a unique id prefix or a unique part of the title.";

fn hb(n: u64) -> String {
    human_bytes(n.min(i64::MAX as u64) as i64)
}

fn entry_label(e: &OfflineEntry) -> String {
    label(&e.title, e.season, e.episode)
}

fn state_name(s: OfflineState) -> &'static str {
    match s {
        OfflineState::Queued => "queued",
        OfflineState::Paused => "paused",
        OfflineState::Done => "done",
        OfflineState::Failed => "failed",
    }
}

pub fn download(args: &[String]) -> Result<(), String> {
    let Some(sub) = args.first() else {
        eprintln!("{USAGE}");
        return Err("missing download command".into());
    };
    let rest = &args[1..];
    match sub.as_str() {
        "config" => config(rest),
        "add" => add(rest),
        "run" => {
            let s = Session::open(rest)?;
            let db = s.home.db()?;
            let store = s.home.offline(&s.cfg, &db)?;
            run_queue(&store, &s.client, s.cfg.offline.retries)
        }
        "list" | "ls" => list(),
        "pause" | "resume" | "cancel" | "rm" => entry_command(sub, rest),
        "play" => play(rest),
        "sync" => {
            let s = Session::open(rest)?;
            let db = s.home.db()?;
            let store = s.home.offline(&s.cfg, &db)?;
            sync(&s, &store, &db, true)
        }
        "gc" => gc(rest),
        "help" | "--help" | "-h" => {
            println!("{USAGE}");
            Ok(())
        }
        other => {
            eprintln!("{USAGE}");
            Err(format!("unknown download command {other:?}"))
        }
    }
}

fn config(args: &[String]) -> Result<(), String> {
    let home = Home::open()?;
    let mut cfg = home.load()?;
    let mut changed = false;
    if let Some(d) = flag(args, "dir") {
        cfg.offline.dir = crate::server::absolute(std::path::Path::new(&d)).to_string_lossy().into_owned();
        changed = true;
    }
    if let Some(v) = flag(args, "max") {
        cfg.offline.max_bytes = byte_size(&v, true).map_err(|e| format!("--max {e}"))?;
        changed = true;
    }
    if let Some(v) = flag(args, "min-free") {
        cfg.offline.min_free_bytes = byte_size(&v, true).map_err(|e| format!("--min-free {e}"))?;
        changed = true;
    }
    if let Some(v) = flag(args, "evict-watched") {
        cfg.offline.evict_watched = match v.as_str() {
            "on" | "true" | "yes" => true,
            "off" | "false" | "no" => false,
            _ => return Err("--evict-watched takes on or off".into()),
        };
        changed = true;
    }
    if let Some(v) = flag(args, "retries") {
        cfg.offline.retries = match v.parse::<u32>() {
            Ok(n) if n <= MAX_RETRIES => n,
            _ => return Err(format!("--retries takes a number from 0 to {MAX_RETRIES}")),
        };
        changed = true;
    }
    if changed {
        home.save(&cfg)?;
    }
    let limit = |n: u64| if n == 0 { "none".to_string() } else { hb(n) };
    let oc = home.offline_config(&cfg);
    println!(
        "dir            {}\nmax            {}\nmin free       {}\nevict watched  {}\nretries        {}",
        oc.dir,
        limit(oc.max_bytes),
        limit(oc.min_free_bytes),
        oc.evict_watched,
        cfg.offline.retries
    );
    Ok(())
}

fn add(args: &[String]) -> Result<(), String> {
    let s = Session::open(args)?;
    let rt = runtime()?;
    let c = &s.client;
    let start = rt.block_on(resolve(c, args, false))?;
    let items = if has(args, "rest") { rt.block_on(episode_queue(c, &start))? } else { vec![start] };
    let db = s.home.db()?;
    let store = s.home.offline(&s.cfg, &db)?;
    for it in items {
        let full = rt.block_on(c.catalog_item(&it.id)).map_err(|e| e.to_string())?;
        let e = store.add(&c.server(), &full).map_err(|e| e.to_string())?;
        println!("queued: {} ({})", entry_label(&e), hb(full.size.max(0) as u64));
    }
    if has(args, "no-run") {
        return Ok(());
    }
    run_queue(&store, c, s.cfg.offline.retries)
}

/// Prints transfer progress at most four times a second.
struct Progress {
    label: String,
    last: Mutex<Option<Instant>>,
}

impl FetchObserver for Progress {
    fn progress(&self, done: u64, total: u64) {
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|t| t.elapsed() < Duration::from_millis(250)) && done != total {
            return;
        }
        *last = Some(Instant::now());
        use std::io::Write;
        print!("\r{}  {}", self.label, progress_text(done, total));
        let _ = std::io::stdout().flush();
    }

    fn evicted(&self, e: &OfflineEntry) {
        println!("\nremoved watched copy to make room: {}", entry_label(e));
    }
}

pub fn progress_text(done: u64, total: u64) -> String {
    if total > 0 {
        format!("{:5.1}%  {} / {}", done as f64 * 100.0 / total as f64, hb(done), hb(total))
    } else {
        hb(done)
    }
}

/// Failures worth another attempt: unreachable, timeouts, 5xx, 408, 429.
pub fn retryable(e: &ClientError) -> bool {
    match e {
        ClientError::Offline { .. } | ClientError::Timeout | ClientError::RateLimited { .. } => true,
        ClientError::Api { status, .. } => *status >= 500 || *status == 408,
        _ => false,
    }
}

enum Outcome {
    Done,
    Failed(ClientError),
}

/// One entry, retried with backoff up to `limit` times; a retry resumes
/// from the partial bytes and any progress resets the count.
async fn fetch_with_retries(store: &OfflineStore, c: &Client, e: &OfflineEntry, limit: u32) -> Outcome {
    let label = entry_label(e);
    let observer: Arc<dyn FetchObserver> = Arc::new(Progress { label: label.clone(), last: Mutex::new(None) });
    let (mut prev, mut attempts) = (e.bytes, 0u32);
    loop {
        let res = store.fetch(c, &e.item_id, Some(observer.clone())).await;
        println!();
        let err = match res {
            Ok(_) => return Outcome::Done,
            Err(err) => err,
        };
        if !retryable(&err) {
            return Outcome::Failed(err);
        }
        let got = store.get(&e.server, &e.item_id).ok().flatten().map(|x| x.bytes).unwrap_or(0);
        if got > prev {
            attempts = 0;
        }
        prev = got;
        if attempts >= limit {
            return Outcome::Failed(err);
        }
        attempts += 1;
        let wait = lain_downloads::default_backoff(attempts);
        println!("retrying: {label} in {}s (attempt {attempts}/{limit}): {err}", wait.as_secs());
        if let Err(e) = store.resume(&e.server, &e.item_id) {
            return Outcome::Failed(e);
        }
        tokio::time::sleep(wait).await;
    }
}

/// Downloads every queued entry of `c`'s server in order. Ctrl+C stops
/// the run; the entry stays queued with its bytes kept.
pub fn run_queue(store: &OfflineStore, c: &Client, retries: u32) -> Result<(), String> {
    let rt = runtime()?;
    let current: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let cur = current.clone();
    let server = c.server();
    let run = async {
        let mut failed = 0;
        for e in store.list().map_err(|e| e.to_string())? {
            if e.state != OfflineState::Queued {
                continue;
            }
            if e.server != server {
                println!("skipped: {} belongs to {}", entry_label(&e), e.server);
                continue;
            }
            *cur.lock().unwrap_or_else(|x| x.into_inner()) = Some(e.item_id.clone());
            match fetch_with_retries(store, c, &e, retries).await {
                Outcome::Done => println!("done: {}", entry_label(&e)),
                Outcome::Failed(err) => {
                    failed += 1;
                    println!("failed: {}: {err}", entry_label(&e));
                    if matches!(err, ClientError::NoRoom { .. }) {
                        println!(
                            "hint: `lain download gc --watched` frees watched copies; `lain download config --max` raises the limit"
                        );
                        return Err("offline store is full".to_string());
                    }
                }
            }
        }
        *cur.lock().unwrap_or_else(|x| x.into_inner()) = None;
        if failed > 0 {
            return Err(format!("{failed} download(s) failed; `lain download resume <id>` retries"));
        }
        Ok(())
    };
    rt.block_on(async {
        tokio::select! {
            r = run => r,
            _ = tokio::signal::ctrl_c() => {
                let id = current.lock().unwrap_or_else(|x| x.into_inner()).clone();
                if let Some(e) = id.and_then(|id| store.get(&server, &id).ok().flatten()) {
                    // Interrupted mid-transfer: the part file holds what
                    // arrived (its length is the resume point).
                    if e.state != OfflineState::Done && e.state != OfflineState::Queued {
                        let _ = store.resume(&e.server, &e.item_id);
                    }
                    println!("\nstopped: {} (partial bytes kept; `lain download run` resumes)", entry_label(&e));
                }
                Ok(())
            }
        }
    })
}

type Db = Arc<lain_client::ClientDb>;

fn open_store() -> Result<(CliConfig, Db, OfflineStore), String> {
    let home = Home::open()?;
    let cfg = home.load()?;
    let db = home.db()?;
    let store = home.offline(&cfg, &db)?;
    Ok((cfg, db, store))
}

fn list() -> Result<(), String> {
    let (_, db, store) = open_store()?;
    // Unsynced marks need the queue of the signed-in user.
    let pending: Vec<String> = match Session::open(&[]) {
        Ok(s) => s.sync(&db).queued().unwrap_or_default(),
        Err(_) => Vec::new(),
    }
    .into_iter()
    .map(|q| q.item_id)
    .collect();
    print!("{}", render_list(&store, &pending)?);
    Ok(())
}

pub fn render_list(store: &OfflineStore, pending: &[String]) -> Result<String, String> {
    let entries = store.list().map_err(|e| e.to_string())?;
    let mut out = String::new();
    if entries.is_empty() {
        out.push_str("offline store is empty (`lain download add <query>`)\n");
    }
    for e in &entries {
        let size = if e.size > 0 { e.size } else { e.bytes };
        let pct = if e.state != OfflineState::Done && size > 0 {
            format!("{:3}%", e.bytes * 100 / size)
        } else {
            "    ".into()
        };
        let mut marks = String::new();
        if e.watched {
            marks.push_str(" watched");
        }
        if pending.contains(&e.item_id) {
            marks.push_str(" unsynced");
        }
        let id: String = e.item_id.chars().take(8).collect();
        out.push_str(&format!(
            "{:<8} {pct} {:>10}  {id:<8}  {}{marks}\n",
            state_name(e.state),
            hb(size),
            entry_label(e)
        ));
    }
    let cfg = store.config();
    let limit = if cfg.max_bytes > 0 { hb(cfg.max_bytes) } else { "no limit".into() };
    out.push_str(&format!("used {} of {limit} in {}\n", hb(store.used().map_err(|e| e.to_string())?), cfg.dir));
    Ok(out)
}

/// An id, a unique id prefix or a unique part of the title.
pub fn find_entry(store: &OfflineStore, reference: &str) -> Result<OfflineEntry, String> {
    let r = reference.trim();
    if r.is_empty() {
        return Err("name an entry (id prefix or title)".into());
    }
    let all = store.list().map_err(|e| e.to_string())?;
    if let Some(e) = all.iter().find(|e| e.item_id == r) {
        return Ok(e.clone());
    }
    let low = r.to_lowercase();
    let hits: Vec<&OfflineEntry> =
        all.iter().filter(|e| e.item_id.starts_with(r) || entry_label(e).to_lowercase().contains(&low)).collect();
    match hits.as_slice() {
        [] => Err(format!("nothing in the offline store matches {r:?}")),
        [one] => Ok((*one).clone()),
        many => {
            let names: Vec<String> = many
                .iter()
                .map(|h| format!("{} {}", h.item_id.chars().take(8).collect::<String>(), entry_label(h)))
                .collect();
            Err(format!("{r:?} matches {} entries:\n  {}", many.len(), names.join("\n  ")))
        }
    }
}

fn entry_command(sub: &str, args: &[String]) -> Result<(), String> {
    let (_, _db, store) = open_store()?;
    let e = find_entry(&store, &positional(args).join(" "))?;
    let l = entry_label(&e);
    match sub {
        "pause" => {
            store.pause(&e.server, &e.item_id).map_err(|e| e.to_string())?;
            println!("paused: {l}");
        }
        "resume" => {
            store.resume(&e.server, &e.item_id).map_err(|e| e.to_string())?;
            println!("queued: {l} (`lain download run` downloads it)");
        }
        "cancel" => {
            if e.state == OfflineState::Done {
                return Err(format!("{l} is already downloaded; `lain download rm` deletes the copy"));
            }
            let freed = store.remove(&e.server, &e.item_id, true).map_err(|e| e.to_string())?;
            println!("canceled: {l} (freed {})", hb(freed));
        }
        _ => {
            let freed = store.remove(&e.server, &e.item_id, has(args, "force")).map_err(|e| e.to_string())?;
            println!("removed: {l} (freed {})", hb(freed));
        }
    }
    Ok(())
}

/// Pushes queued offline positions (the progress sync's conflict rules
/// decide each one), then refreshes the watched flag of every finished
/// copy so eviction knows what is safe.
fn sync(s: &Session, store: &OfflineStore, db: &Db, verbose: bool) -> Result<(), String> {
    let rt = runtime()?;
    let labels = |id: &str| {
        store.get(&s.client.server(), id).ok().flatten().map(|e| entry_label(&e)).unwrap_or_else(|| id.to_string())
    };
    let rep = rt.block_on(s.sync(db).flush()).map_err(|e| e.to_string())?;
    for i in &rep.items {
        if !verbose {
            continue;
        }
        match i.resolution {
            Resolution::Pushed => println!("synced: {} at {:.0}s", labels(&i.item_id), i.progress.position_sec),
            Resolution::KeptServer => {
                println!("kept the server's position: {} at {:.0}s", labels(&i.item_id), i.progress.position_sec)
            }
            Resolution::Refused => println!("refused: {}: {}", labels(&i.item_id), i.message),
        }
    }
    if let Some(e) = rep.stopped {
        return Err(format!("sync stopped with {} still queued: {e}", rep.remaining));
    }
    rt.block_on(store.refresh_watched(&s.client)).map_err(|e| e.to_string())?;
    Ok(())
}

fn gc(args: &[String]) -> Result<(), String> {
    let (_, db, store) = open_store()?;
    if let Ok(s) = Session::open(&[])
        && let Err(e) = sync(&s, &store, &db, false)
    {
        eprintln!("warning: could not refresh watched state: {e}");
    }
    let rep = store.gc(has(args, "watched")).map_err(|e| e.to_string())?;
    for e in &rep.evicted {
        println!("removed watched copy: {}", entry_label(e));
    }
    println!(
        "freed {} ({} orphaned partials, {} forgotten files, {} watched copies)",
        hb(rep.freed_bytes),
        rep.orphan_parts,
        rep.vanished,
        rep.evicted.len()
    );
    Ok(())
}

/// Plays a local copy with no server involved. The position is recorded
/// through the progress sync: pushed when the server answers, queued
/// otherwise.
fn play(args: &[String]) -> Result<(), String> {
    let home = Home::open()?;
    let cfg = home.load()?;
    let db = home.db()?;
    let store = home.offline(&cfg, &db)?;
    let e = find_entry(&store, &positional(args).join(" "))?;
    let l = entry_label(&e);
    let Some(path) = store.local_path(&e.server, &e.item_id).map_err(|e| e.to_string())? else {
        return Err(format!("{l} has no finished local copy"));
    };
    if e.kind == "comic" || e.kind == "manga" {
        return Err(format!("{l} is an archive; open {path} in a comic reader (the web reader needs the server)"));
    }
    let player = choose_player(args, cfg.player())?;
    // A short timeout: this runs offline more often than not.
    let client = Client::new(lain_client::ClientConfig { server: e.server.clone(), timeout_ms: 5_000 }, home.tokens())
        .map_err(|e| e.to_string())?;
    let rt = runtime()?;
    let sync = (client.session().is_some()).then(|| lain_client::ProgressSync::new(client.clone(), db.clone()));
    let resume = match &sync {
        Some(s) => rt.block_on(s.fetch(&e.item_id)).map(|p| resume_at(p.position_sec, p.completed)).unwrap_or(0.0),
        None => 0.0,
    };
    store.touch(&e.server, &e.item_id).map_err(|e| e.to_string())?;
    println!("playing offline: {l}");
    let saved = match &sync {
        Some(s) => play_local(&player, &e.item_id, &path, resume, recorder(&rt, s))?,
        None => {
            eprintln!("warning: no saved login for {}; the position is not kept", e.server);
            play_local(&player, &e.item_id, &path, resume, |_, _| true)?
        }
    };
    report(&saved, &|_| l.clone());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_policy() {
        assert!(retryable(&ClientError::Timeout));
        assert!(retryable(&ClientError::Offline { message: "x".into() }));
        assert!(retryable(&ClientError::Api { status: 502, code: String::new(), message: String::new() }));
        assert!(retryable(&ClientError::Api { status: 408, code: String::new(), message: String::new() }));
        assert!(!retryable(&ClientError::Api { status: 400, code: String::new(), message: String::new() }));
        assert!(!retryable(&ClientError::NotFound { message: String::new() }));
        assert!(!retryable(&ClientError::NoRoom { code: "quota-exceeded".into(), message: String::new() }));
        assert_eq!(progress_text(512, 1024), " 50.0%  512 B / 1.0 KiB");
        assert_eq!(progress_text(2048, 0), "2.0 KiB");
    }
}
