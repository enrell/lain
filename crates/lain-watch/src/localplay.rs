//! Local playback (D-072, D-076): when the browser sits on the server's
//! machine, the server opens mpv or VLC on the library files itself, so
//! no stream URL or token leaves it. [`Manager`] is the process engine
//! (spawn, supervise, sample progress); [`LocalPlay`] is the
//! `lain.playback.local@1` policy on top (queue, resume, subtitles).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use lain_api::{CatalogItem, MediaInfo, MediaStream, Progress, TrackPreferences};
use lain_core::{Error, Registry};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const ID: &str = "lain-playback-local";
pub const CAP: &str = "lain.playback.local@1";

/// The progress script mpv runs (shared with the CLI): one JSON state
/// file per playlist index, and the track preferences (D-280).
pub const MPV_SCRIPT: &str = include_str!("../assets/mpv_progress.lua");

/// One playlist entry.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Entry {
    pub item_id: String,
    pub path: String,
    /// Seconds; under 5 or non-finite starts from the top.
    pub resume: f64,
    /// VLC only: subtitles off once the track loads.
    pub subs_off: bool,
}

type SaveFn = Box<dyn Fn(&str, f64, f64) + Send + Sync>;
type DoneFn = Box<dyn FnOnce(Result<(), String>) + Send>;

/// One session to start.
pub struct PlayRequest {
    pub player: String,
    pub entries: Vec<Entry>,
    /// The viewer's track preferences (D-280).
    pub tracks: TrackPreferences,
    /// Measured positions, from the monitor thread.
    pub save: SaveFn,
    pub done: Option<DoneFn>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayError {
    /// No player binary or no graphical session.
    NoPlayer,
    /// One screen fits one player at a time.
    Busy,
    Other(String),
}

impl std::fmt::Display for PlayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlayError::NoPlayer => write!(f, "no local player available"),
            PlayError::Busy => write!(f, "a local playback session is already running"),
            PlayError::Other(e) => write!(f, "{e}"),
        }
    }
}

type LookFn = Box<dyn Fn(&str) -> Option<PathBuf> + Send + Sync>;
type EnvFn = Box<dyn Fn(&str) -> String + Send + Sync>;

#[derive(Default)]
struct Running {
    running: bool,
    procs: Vec<(u64, Arc<Mutex<Child>>)>,
    monitors: Vec<std::thread::JoinHandle<()>>,
}

/// Spawns and supervises local player processes.
#[derive(Default)]
pub struct Manager {
    look_path: Mutex<Option<LookFn>>,
    getenv: Mutex<Option<EnvFn>>,
    state: Arc<Mutex<Running>>,
}

static SEQ: AtomicU64 = AtomicU64::new(1);

impl Manager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test seam: how binaries are found.
    pub fn set_look_path(&self, f: impl Fn(&str) -> Option<PathBuf> + Send + Sync + 'static) {
        *self.look_path.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(f));
    }

    /// Test seam: the environment (`DISPLAY`, `WAYLAND_DISPLAY`).
    pub fn set_getenv(&self, f: impl Fn(&str) -> String + Send + Sync + 'static) {
        *self.getenv.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(f));
    }

    fn find(&self, bin: &str) -> Option<PathBuf> {
        match self.look_path.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            Some(f) => f(bin),
            None => lain_probe::exec::look_path(bin),
        }
    }

    /// A graphical session exists for a window. Windows and macOS always
    /// have one; Linux and the BSDs need X11 or Wayland.
    fn display(&self) -> bool {
        let g = self.getenv.lock().unwrap_or_else(|e| e.into_inner());
        let get = |k: &str| match g.as_ref() {
            Some(f) => f(k),
            None => std::env::var(k).unwrap_or_default(),
        };
        if g.is_none() && cfg!(any(windows, target_os = "macos")) {
            return true;
        }
        !get("DISPLAY").is_empty() || !get("WAYLAND_DISPLAY").is_empty()
    }

    /// The players that can launch right now.
    pub fn players(&self) -> Vec<String> {
        if !self.display() {
            return Vec::new();
        }
        ["mpv", "vlc"].iter().filter(|p| self.find(p).is_some()).map(|p| p.to_string()).collect()
    }

    /// Kills every running player and waits for their monitors.
    pub fn close(&self) {
        let (procs, monitors) = {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            (s.procs.clone(), std::mem::take(&mut s.monitors))
        };
        for (_, c) in procs {
            if let Ok(mut c) = c.lock() {
                let _ = c.kill();
            }
        }
        for m in monitors {
            let _ = m.join();
        }
    }

    /// Writes the playlist and resume state into a private temp dir,
    /// spawns the player and returns; progress flows through `save`
    /// until it exits.
    pub fn play(&self, req: PlayRequest) -> Result<(), PlayError> {
        if req.entries.is_empty() {
            return Err(PlayError::Other("empty playlist".into()));
        }
        let player = if req.player.is_empty() {
            self.players().into_iter().next().unwrap_or_default()
        } else {
            req.player.clone()
        };
        if player != "mpv" && player != "vlc" {
            return Err(PlayError::Other(format!("unsupported player {player:?}")));
        }
        let bin = self.find(&player).ok_or_else(|| PlayError::Other(format!("{player} not found in PATH")))?;
        if !self.display() {
            return Err(PlayError::NoPlayer);
        }
        {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if s.running {
                return Err(PlayError::Busy);
            }
            s.running = true;
            s.monitors.retain(|m| !m.is_finished());
        }
        let reset = || self.state.lock().unwrap_or_else(|e| e.into_inner()).running = false;
        let dir = std::env::temp_dir().join(format!(
            "lain-play-{}-{}-{}",
            std::process::id(),
            lain_core::now_unix_millis(),
            SEQ.fetch_add(1, Ordering::SeqCst)
        ));
        let prepared = prepare(&dir, &player, &req);
        let mut cmd = match prepared {
            Ok(args) => {
                let mut c = Command::new(&bin);
                c.args(args);
                c
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                reset();
                return Err(PlayError::Other(e));
            }
        };
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                reset();
                return Err(PlayError::Other(e.to_string()));
            }
        };
        let stderr = child.stderr.take();
        let child = Arc::new(Mutex::new(child));
        let id = SEQ.fetch_add(1, Ordering::SeqCst);
        self.state.lock().unwrap_or_else(|e| e.into_inner()).procs.push((id, child.clone()));
        let state = self.state.clone();
        let m = std::thread::Builder::new()
            .name("localplay-monitor".into())
            .spawn(move || {
                let head = stderr.map(|mut s| {
                    std::thread::spawn(move || {
                        use std::io::Read;
                        let mut keep = Vec::new();
                        let mut buf = [0u8; 4096];
                        while let Ok(n) = s.read(&mut buf) {
                            if n == 0 {
                                break;
                            }
                            let room = 8192usize.saturating_sub(keep.len());
                            keep.extend_from_slice(&buf[..n.min(room)]);
                        }
                        keep
                    })
                });
                monitor(&child, &player, &dir, req, head);
                let _ = std::fs::remove_dir_all(&dir);
                let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
                s.procs.retain(|(i, _)| *i != id);
                s.running = !s.procs.is_empty();
            })
            .map_err(|e| PlayError::Other(e.to_string()))?;
        self.state.lock().unwrap_or_else(|e| e.into_inner()).monitors.push(m);
        Ok(())
    }
}

fn finite(n: f64) -> bool {
    n.is_finite() && n >= 0.0
}

/// The playlist, resume file and argv for one session.
fn prepare(dir: &Path, player: &str, req: &PlayRequest) -> Result<Vec<String>, String> {
    std::fs::create_dir(dir).map_err(|e| e.to_string())?;
    lain_store::ensure_private_dir(dir).map_err(|e| e.to_string())?;
    let mut playlist = String::from("#EXTM3U\n");
    let mut resumes = BTreeMap::new();
    for (i, e) in req.entries.iter().enumerate() {
        if e.resume >= 5.0 && finite(e.resume) {
            resumes.insert(i.to_string(), e.resume);
        }
        playlist.push_str(&format!("#EXTINF:-1,lain-episode-{i:06}\n{}\n", e.path));
    }
    let m3u = dir.join("episodes.m3u");
    std::fs::write(&m3u, playlist).map_err(|e| e.to_string())?;
    let resume = dir.join("resume.json");
    std::fs::write(&resume, serde_json::to_vec(&resumes).unwrap_or_default()).map_err(|e| e.to_string())?;
    let p = |x: &Path| x.to_string_lossy().into_owned();
    if player == "mpv" {
        let script = dir.join("lain-progress.lua");
        std::fs::write(&script, MPV_SCRIPT).map_err(|e| e.to_string())?;
        let mut a = vec![
            format!("--script={}", p(&script)),
            format!("--script-opt=lain-state_dir={}", p(dir)),
            format!("--script-opt=lain-resume_file={}", p(&resume)),
        ];
        if !req.tracks.is_empty() {
            let t = &req.tracks;
            a.push(format!("--script-opt=lain-audio_language={}", t.audio_language));
            a.push(format!("--script-opt=lain-subtitle_language={}", t.subtitle_language));
            a.push(format!("--script-opt=lain-subtitle_mode={}", t.subtitle_mode));
        }
        a.push(format!("--playlist={}", p(&m3u)));
        return Ok(a);
    }
    let mut a = vec![
        "--no-one-instance".to_string(),
        "--play-and-exit".into(),
        "--extraintf=rc".into(),
        format!("--rc-unix={}", p(&dir.join("vlc.sock"))),
    ];
    if !req.tracks.audio_language.is_empty() {
        a.push(format!("--audio-language={}", req.tracks.audio_language));
    }
    if !req.tracks.subtitle_language.is_empty() && req.tracks.subtitle_mode != "off" {
        a.push(format!("--sub-language={}", req.tracks.subtitle_language));
    }
    if let Some(at) = resumes.get("0") {
        a.push(format!("--start-time={at:.0}"));
    }
    a.push(p(&m3u));
    Ok(a)
}

#[derive(Deserialize)]
struct StateFile {
    #[serde(default)]
    position_sec: f64,
    #[serde(default)]
    duration_sec: f64,
}

fn read_state(path: &Path) -> Option<(f64, f64)> {
    let s: StateFile = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    Some((s.position_sec, s.duration_sec))
}

fn monitor(
    child: &Arc<Mutex<Child>>,
    player: &str,
    dir: &Path,
    mut req: PlayRequest,
    head: Option<std::thread::JoinHandle<Vec<u8>>>,
) {
    let mut saved: BTreeMap<usize, (f64, f64)> = BTreeMap::new();
    let mut observed = false;
    let mut current: Option<usize> = None;
    let mut last = (0.0, 0.0);
    let save = |saved: &mut BTreeMap<usize, (f64, f64)>, i: usize, pos: f64, dur: f64, req: &PlayRequest| {
        if i >= req.entries.len() || !finite(pos) || !finite(dur) || dur <= 0.0 {
            return;
        }
        if saved.get(&i) == Some(&(pos, dur)) {
            return;
        }
        saved.insert(i, (pos, dur));
        (req.save)(&req.entries[i].item_id, pos, dur);
    };
    let mut sample = |saved: &mut BTreeMap<usize, (f64, f64)>, req: &PlayRequest, observed: &mut bool| {
        if player == "mpv" {
            for i in 0..req.entries.len() {
                if let Some((pos, dur)) = read_state(&dir.join(format!("{i}.json"))) {
                    *observed = true;
                    save(saved, i, pos, dur, req);
                }
            }
            return;
        }
        let sock = dir.join("vlc.sock");
        let Some(index) = vlc::queue_index(&sock, req.entries.len()) else { return };
        if current != Some(index) {
            current = Some(index);
            if req.entries[index].subs_off && vlc::rc(&sock, "strack -1\n").is_none() {
                tracing::debug!("localplay: could not disable subtitles");
            }
            if index > 0 && req.entries[index].resume >= 5.0 {
                let _ = vlc::rc(&sock, &format!("seek {:.0}\n", req.entries[index].resume));
                return;
            }
        }
        if let Some((pos, dur)) = vlc::position(&sock)
            && dur > 0.0
        {
            last = (pos, dur);
            *observed = true;
            save(saved, index, pos, dur, req);
        }
    };
    let mut ticks = 0u32;
    let status = loop {
        let polled = child.lock().map(|mut c| c.try_wait());
        match polled {
            Ok(Ok(Some(st))) => break Some(st),
            Ok(Ok(None)) => {}
            _ => break None,
        }
        std::thread::sleep(Duration::from_millis(250));
        ticks += 1;
        if ticks.is_multiple_of(4) {
            sample(&mut saved, &req, &mut observed);
        }
    };
    sample(&mut saved, &req, &mut observed);
    if player == "vlc"
        && let Some(i) = current
        && observed
    {
        save(&mut saved, i, last.0, last.1, &req);
    }
    let detail =
        head.and_then(|h| h.join().ok()).map(|b| String::from_utf8_lossy(&b).trim().to_string()).unwrap_or_default();
    let Some(done) = req.done.take() else { return };
    if !observed {
        let msg = if detail.is_empty() {
            format!("{player} closed without measurable progress")
        } else {
            format!("{player} closed without measurable progress: {detail}")
        };
        done(Err(msg));
        return;
    }
    match status {
        Some(st) if !st.success() => done(Err(format!("{player} exited: {st}"))),
        _ => done(Ok(())),
    }
}

#[cfg(unix)]
mod vlc {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::time::Duration;

    /// One RC exchange over VLC's private socket.
    pub fn rc(sock: &Path, command: &str) -> Option<String> {
        let mut c = UnixStream::connect(sock).ok()?;
        c.set_read_timeout(Some(Duration::from_millis(700))).ok()?;
        c.set_write_timeout(Some(Duration::from_millis(700))).ok()?;
        c.write_all(command.as_bytes()).ok()?;
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match c.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(_) => break,
            }
        }
        (!out.is_empty()).then(|| String::from_utf8_lossy(&out).into_owned())
    }

    /// The entry VLC plays, by its `lain-episode-NNNNNN` title.
    pub fn queue_index(sock: &Path, count: usize) -> Option<usize> {
        let title = rc(sock, "get_title\n")?;
        (0..count).find(|i| title.contains(&format!("lain-episode-{i:06}")))
    }

    pub fn position(sock: &Path) -> Option<(f64, f64)> {
        let r = rc(sock, "get_time\nget_length\n")?;
        let v: Vec<f64> = r
            .lines()
            .filter_map(|l| l.trim_start_matches(['>', ' ']).trim().parse::<f64>().ok())
            .filter(|n| *n >= 0.0)
            .take(2)
            .collect();
        (v.len() == 2).then(|| (v[0], v[1]))
    }
}

#[cfg(not(unix))]
mod vlc {
    use std::path::Path;
    pub fn rc(_: &Path, _: &str) -> Option<String> {
        None
    }
    pub fn queue_index(_: &Path, _: usize) -> Option<usize> {
        None
    }
    pub fn position(_: &Path) -> Option<(f64, f64)> {
        None
    }
}

pub use crate::tracks::same_language;

/// VLC under the account's preferences (D-071, D-280): its language
/// flags pick the tracks, and this says when subtitles go off instead,
/// which no VLC flag can say per file.
pub fn subtitle_off(streams: &[MediaStream], prefs: &TrackPreferences) -> bool {
    !prefs.is_empty() && crate::tracks::choose(streams, prefs).subtitle == crate::tracks::Subtitle::Off
}

/// What `play` is asked.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlayInput {
    #[serde(default)]
    pub player: String,
    pub user_id: String,
    /// D-071's one language, for a caller before D-280: both languages
    /// when `tracks` names neither.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub language: String,
    /// The viewer's track preferences (D-280), flat in the input.
    #[serde(default, flatten)]
    pub tracks: TrackPreferences,
    pub item: CatalogItem,
    /// Libraries this session may not see (D-156): their episodes never
    /// join the queue.
    #[serde(default)]
    pub hidden_libraries: Vec<String>,
}

type ProgressFn = Box<dyn Fn(&str, &str, Value) + Send + Sync>;

/// The `lain.playback.local@1` policy: queue a series from this
/// episode on (D-056), resume each entry, VLC subtitle policy (D-071),
/// progress into userstate.
pub struct LocalPlay {
    pub manager: Manager,
    registry: Mutex<Weak<Registry>>,
    on_progress: Arc<Mutex<Option<ProgressFn>>>,
}

impl Default for LocalPlay {
    fn default() -> Self {
        LocalPlay { manager: Manager::new(), registry: Mutex::new(Weak::new()), on_progress: Arc::default() }
    }
}

impl LocalPlay {
    /// The registry it reads catalog, userstate and probe through.
    pub fn attach(&self, reg: &Arc<Registry>) {
        *self.registry.lock().unwrap_or_else(|e| e.into_inner()) = Arc::downgrade(reg);
    }

    /// Called after each saved position (events).
    pub fn on_progress(&self, f: impl Fn(&str, &str, Value) + Send + Sync + 'static) {
        *self.on_progress.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(f));
    }

    fn reg(&self) -> Option<Arc<Registry>> {
        self.registry.lock().unwrap_or_else(|e| e.into_inner()).upgrade()
    }

    pub fn play(&self, input: &PlayInput) -> Result<Value, Error> {
        let players = self.manager.players();
        if players.is_empty() {
            return Err(Error::unavailable(
                "no local player: install mpv or VLC on the server machine inside a graphical session",
            ));
        }
        let player = if input.player.is_empty() { players[0].clone() } else { input.player.clone() };
        if !players.contains(&player) {
            return Err(Error::invalid(format!("player must be one of: {}", players.join(", "))));
        }
        let items = self.queue(&input.item, &input.hidden_libraries)?;
        let reg = self.reg();
        let tracks = if input.tracks.audio_language.is_empty()
            && input.tracks.subtitle_language.is_empty()
            && !input.language.is_empty()
        {
            TrackPreferences::legacy(&input.language)
        } else {
            input.tracks.clone()
        };
        let entries: Vec<Entry> = items
            .iter()
            .map(|it| {
                let mut e = Entry { item_id: it.id.clone(), path: it.file_path.clone(), ..Entry::default() };
                if let Some(p) = reg.as_ref().and_then(|r| progress(r, &input.user_id, &it.id))
                    && !p.completed
                {
                    e.resume = p.position_sec;
                }
                if player == "vlc" && !tracks.is_empty() {
                    e.subs_off = tracks.subtitle_mode == "off"
                        || reg.as_ref().is_some_and(|r| subs_off(r, &it.file_path, &tracks));
                }
                e
            })
            .collect();
        let count = entries.len();
        let weak = reg.as_ref().map(Arc::downgrade).unwrap_or_default();
        let user = input.user_id.clone();
        let sink = self.on_progress.clone();
        let req = PlayRequest {
            player: player.clone(),
            entries,
            tracks,
            save: Box::new(move |item, pos, dur| {
                let Some(r) = weak.upgrade() else { return };
                let p =
                    json!({"item_id": item, "position_sec": pos, "duration_sec": dur, "completed": pos / dur >= 0.95});
                match r.call_one("lain.userstate.progress@1", "put", json!({"user_id": user, "progress": p})) {
                    Ok((out, _)) => {
                        if let Some(f) = sink.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                            f(&user, item, out);
                        }
                    }
                    Err(e) => tracing::warn!(err = %e, "local playback progress not saved"),
                }
            }),
            done: Some(Box::new(|r| {
                if let Err(e) = r {
                    tracing::warn!(err = %e, "local playback ended abnormally");
                }
            })),
        };
        self.manager.play(req).map_err(|e| match e {
            PlayError::Busy => Error::new("busy", e.to_string()),
            PlayError::NoPlayer => Error::unavailable(e.to_string()),
            PlayError::Other(m) => Error::internal(m),
        })?;
        Ok(json!({"status": "playing", "player": player, "count": count}))
    }

    /// A series plays from this episode onward (D-056); missing entries
    /// and vanished files are skipped (D-068).
    fn queue(&self, start: &CatalogItem, hidden: &[String]) -> Result<Vec<CatalogItem>, Error> {
        let mut items = Vec::new();
        if start.episode > 0
            && let Some(reg) = self.reg()
            && let Ok((out, _)) = reg.call_one("lain.catalog.read@1", "episodes", json!({"id": start.id}))
        {
            let eps: Vec<CatalogItem> = serde_json::from_value(out).unwrap_or_default();
            let mut found = false;
            for e in eps {
                if e.id == start.id {
                    found = true;
                }
                if found && !e.missing && !hidden.contains(&e.library_id) {
                    items.push(e);
                }
            }
        }
        if items.is_empty() {
            items.push(start.clone());
        }
        items.retain(|it| Path::new(&it.file_path).is_file());
        if items.is_empty() {
            return Err(Error::not_found("the file is no longer on disk"));
        }
        Ok(items)
    }
}

fn progress(reg: &Registry, user: &str, item: &str) -> Option<Progress> {
    let (out, _) = reg.call_one("lain.userstate.progress@1", "get", json!({"user_id": user, "item_id": item})).ok()?;
    serde_json::from_value(out).ok()
}

fn subs_off(reg: &Registry, path: &str, tracks: &TrackPreferences) -> bool {
    let Ok((out, _)) = reg.call_one(crate::CAP_MEDIA_PROBE, "probe", json!({"file_path": path})) else { return false };
    serde_json::from_value::<MediaInfo>(out).map(|i| subtitle_off(&i.streams, tracks)).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(kind: &str, lang: &str) -> MediaStream {
        MediaStream { kind: kind.into(), language: lang.into(), ..Default::default() }
    }

    #[test]
    fn subtitle_policy() {
        let por = TrackPreferences::legacy("por");
        assert!(subtitle_off(&[s("audio", "por"), s("subtitle", "eng")], &por));
        assert!(!subtitle_off(&[s("audio", "jpn"), s("subtitle", "por")], &por));
        assert!(subtitle_off(&[s("audio", "jpn"), s("subtitle", "eng")], &por));
        assert!(!subtitle_off(&[s("audio", ""), s("subtitle", "")], &por), "untagged keeps defaults");
        assert!(!subtitle_off(&[s("audio", "por")], &TrackPreferences::default()));
        // D-280: Japanese audio with Portuguese subtitles keeps them on.
        let split = TrackPreferences {
            audio_language: "jpn".into(),
            subtitle_language: "por".into(),
            subtitle_mode: "auto".into(),
        };
        assert!(!subtitle_off(&[s("audio", "jpn"), s("subtitle", "por")], &split));
        assert!(subtitle_off(
            &[s("audio", "jpn"), s("subtitle", "por")],
            &TrackPreferences { subtitle_mode: "off".into(), ..split }
        ));
    }

    #[test]
    fn player_arguments_carry_the_preferences() {
        let dir = lain_store::test_dir("localplay-args");
        let req = |player: &str, tracks: TrackPreferences| PlayRequest {
            player: player.into(),
            entries: vec![Entry { item_id: "i1".into(), path: "/m/a.mkv".into(), ..Entry::default() }],
            tracks,
            save: Box::new(|_, _, _| {}),
            done: None,
        };
        let split = TrackPreferences {
            audio_language: "jpn".into(),
            subtitle_language: "por".into(),
            subtitle_mode: "auto".into(),
        };
        let mpv = prepare(&dir.join("mpv"), "mpv", &req("mpv", split.clone())).unwrap();
        for want in ["lain-audio_language=jpn", "lain-subtitle_language=por", "lain-subtitle_mode=auto"] {
            assert!(mpv.iter().any(|a| a.ends_with(want)), "{want} in {mpv:?}");
        }
        let plain = prepare(&dir.join("mpv-plain"), "mpv", &req("mpv", TrackPreferences::default())).unwrap();
        assert!(!plain.iter().any(|a| a.contains("language")), "{plain:?}");
        let vlc = prepare(&dir.join("vlc"), "vlc", &req("vlc", split.clone())).unwrap();
        assert!(vlc.contains(&"--audio-language=jpn".to_string()) && vlc.contains(&"--sub-language=por".to_string()));
        let off = prepare(
            &dir.join("vlc-off"),
            "vlc",
            &req("vlc", TrackPreferences { subtitle_mode: "off".into(), ..split }),
        )
        .unwrap();
        assert!(!off.iter().any(|a| a.starts_with("--sub-language")), "{off:?}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn an_input_before_d280_names_one_language() {
        let item = serde_json::to_value(CatalogItem::default()).unwrap();
        let old: PlayInput = serde_json::from_value(json!({"user_id": "u", "language": "jpn", "item": item})).unwrap();
        assert_eq!((old.language.as_str(), old.tracks.is_empty()), ("jpn", true));
        let new: PlayInput = serde_json::from_value(json!({
            "user_id": "u", "audio_language": "jpn", "subtitle_language": "por", "subtitle_mode": "always", "item": item
        }))
        .unwrap();
        assert_eq!(new.tracks.subtitle_mode, "always");
        let back = serde_json::to_value(&new).unwrap();
        assert_eq!((back["audio_language"].as_str(), back.get("language")), (Some("jpn"), None));
    }

    #[test]
    fn players_need_a_display() {
        let m = Manager::new();
        m.set_look_path(|_| Some(PathBuf::from("/bin/true")));
        m.set_getenv(|_| String::new());
        assert!(m.players().is_empty());
        m.set_getenv(|k| if k == "WAYLAND_DISPLAY" { "wayland-0".into() } else { String::new() });
        assert_eq!(m.players(), vec!["mpv".to_string(), "vlc".to_string()]);
    }

    #[cfg(unix)]
    #[test]
    fn mpv_session_reports_progress_and_frees_the_slot() {
        let dir = lain_store::test_dir("localplay-mpv");
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("mpv");
        std::fs::write(
            &bin,
            "#!/bin/sh\nstate=\"\"\nfor arg in \"$@\"; do\n  case \"$arg\" in\n    --script-opt=lain-state_dir=*) state=\"${arg#--script-opt=lain-state_dir=}\" ;;\n  esac\ndone\nprintf '{\"position_sec\":42,\"duration_sec\":120}' > \"$state/0.json\"\nsleep 1\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let m = Manager::new();
        let b = bin.clone();
        m.set_look_path(move |n| (n == "mpv").then(|| b.clone()));
        m.set_getenv(|k| if k == "DISPLAY" { ":0".into() } else { String::new() });
        let (tx, rx) = std::sync::mpsc::channel();
        let (dtx, drx) = std::sync::mpsc::channel();
        let req = |tx: std::sync::mpsc::Sender<(String, f64, f64)>,
                   dtx: Option<std::sync::mpsc::Sender<Result<(), String>>>| PlayRequest {
            player: "mpv".into(),
            entries: vec![Entry { item_id: "i1".into(), path: "/m/a.mkv".into(), ..Entry::default() }],
            tracks: TrackPreferences::default(),
            save: Box::new(move |i, p, d| {
                let _ = tx.send((i.to_string(), p, d));
            }),
            done: dtx.map(|d| -> DoneFn {
                Box::new(move |r| {
                    let _ = d.send(r);
                })
            }),
        };
        m.play(req(tx.clone(), Some(dtx))).unwrap();
        assert_eq!(m.play(req(tx.clone(), None)).unwrap_err(), PlayError::Busy);
        let got = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(got, ("i1".to_string(), 42.0, 120.0));
        assert_eq!(drx.recv_timeout(Duration::from_secs(10)).unwrap(), Ok(()));
        // The slot frees once the player exits.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            match m.play(req(tx.clone(), None)) {
                Ok(()) => break,
                Err(PlayError::Busy) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50))
                }
                Err(e) => panic!("{e}"),
            }
        }
        m.close();
        assert_eq!(
            m.play(PlayRequest { player: "vlc".into(), ..req(tx, None) }).unwrap_err(),
            PlayError::Other("vlc not found in PATH".into())
        );
        std::fs::remove_dir_all(dir).ok();
    }
}
