//! Linux (and any Unix) playback: an external `mpv` process in its own
//! window, driven over its JSON IPC socket (`--input-ipc-server`).
//!
//! Why not embedded: GPUI 0.2.2 exposes no external texture or surface
//! hook, libmpv's render API needs a GL context GPUI does not share, and
//! on Wayland one process cannot place a surface inside another
//! process's window. So the app hands playback to mpv and keeps the
//! session: it observes `time-pos`, `duration`, `pause`, `track-list`
//! and `eof-reached` over the socket, saves progress through
//! `ProgressSync` exactly like the embedded player, and returns to the
//! item when mpv exits. The app's control bar stays usable as a remote
//! (pause, seek, tracks), and mpv's own OSC and keys work in its window.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::engine::{Engine, PlayerState, Track, track_label};

pub struct ExtMpv {
    state: Arc<Mutex<PlayerState>>,
    writer: Mutex<Option<UnixStream>>,
    child: Mutex<Child>,
    socket: PathBuf,
}

/// Options for one external run.
pub struct ExtOptions {
    pub url: String,
    pub title: String,
    pub start: f64,
    pub mute: bool,
    pub alang: String,
    pub slang: String,
    pub sub_files: Vec<String>,
}

const OBSERVED: &[&str] = &[
    "time-pos",
    "duration",
    "pause",
    "volume",
    "mute",
    "demuxer-cache-time",
    "track-list",
    "eof-reached",
    "vo-configured",
];

impl ExtMpv {
    pub fn spawn(o: ExtOptions) -> Result<ExtMpv, String> {
        let socket = std::env::temp_dir().join(format!("lain-mpv-{}-{}.sock", std::process::id(), now_ms()));
        let _ = std::fs::remove_file(&socket);
        let mut cmd = Command::new(std::env::var("LAIN_MPV").unwrap_or_else(|_| "mpv".into()));
        cmd.arg(format!("--input-ipc-server={}", socket.display()))
            .arg("--force-window=yes")
            .arg("--keep-open=yes")
            .arg("--idle=no")
            .arg(format!("--start={:.3}", o.start))
            .arg(format!("--force-media-title={}", o.title))
            .arg("--hwdec=auto-safe");
        if o.mute {
            cmd.arg("--ao=null").arg("--mute=yes");
        }
        if !o.alang.is_empty() {
            cmd.arg(format!("--alang={}", o.alang));
        }
        if !o.slang.is_empty() {
            cmd.arg(format!("--slang={}", o.slang));
        }
        for s in &o.sub_files {
            cmd.arg(format!("--sub-file={s}"));
        }
        // `--` so a URL can never be read as an option.
        cmd.arg("--").arg(&o.url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        let child = cmd.spawn().map_err(|e| format!("could not start mpv ({e}); install mpv or set LAIN_MPV"))?;
        Self::attach(child, socket)
    }

    /// Connects to the IPC socket of a started player process (waits up
    /// to 5 s for it to appear) and starts observing its properties.
    pub fn attach(child: Child, socket: PathBuf) -> Result<ExtMpv, String> {
        let state = Arc::new(Mutex::new(PlayerState { volume: 100., ..Default::default() }));
        let me =
            ExtMpv { state: state.clone(), writer: Mutex::new(None), child: Mutex::new(child), socket: socket.clone() };
        let deadline = Instant::now() + Duration::from_secs(5);
        let stream = loop {
            match UnixStream::connect(&socket) {
                Ok(s) => break s,
                Err(_) if Instant::now() < deadline => {
                    if let Ok(Some(st)) = me.child.lock().unwrap_or_else(|e| e.into_inner()).try_wait() {
                        return Err(format!("mpv exited at start ({st})"));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(format!("mpv IPC socket: {e}")),
            }
        };
        let reader = stream.try_clone().map_err(|e| e.to_string())?;
        *me.writer.lock().unwrap_or_else(|e| e.into_inner()) = Some(stream);
        for (i, name) in OBSERVED.iter().enumerate() {
            me.send(json!({"command": ["observe_property", i + 1, name]}));
        }
        std::thread::Builder::new()
            .name("mpv-ipc".into())
            .spawn(move || read_loop(reader, &state))
            .map_err(|e| e.to_string())?;
        Ok(me)
    }

    fn send(&self, v: Value) {
        let mut w = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = w.as_mut() {
            let mut line = v.to_string();
            line.push('\n');
            if s.write_all(line.as_bytes()).is_err() {
                *w = None;
            }
        }
    }
}

fn now_ms() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

fn read_loop(stream: UnixStream, state: &Mutex<PlayerState>) {
    let mut lines = BufReader::new(stream).lines();
    while let Some(Ok(line)) = lines.next() {
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
        match v.get("event").and_then(Value::as_str) {
            Some("property-change") => {
                let data = v.get("data").cloned().unwrap_or(Value::Null);
                let f = data.as_f64();
                let b = data.as_bool();
                match v.get("name").and_then(Value::as_str).unwrap_or("") {
                    "time-pos" => st.time = f.unwrap_or(st.time),
                    "duration" => st.duration = f.unwrap_or(st.duration),
                    "pause" => st.paused = b.unwrap_or(st.paused),
                    "volume" => st.volume = f.unwrap_or(st.volume),
                    "mute" => st.muted = b.unwrap_or(st.muted),
                    "demuxer-cache-time" => st.cache_end = f.unwrap_or(0.),
                    "eof-reached" => st.eof = b.unwrap_or(false),
                    "vo-configured" => st.has_video = b.unwrap_or(st.has_video),
                    "track-list" => st.tracks = parse_tracks(&data),
                    _ => {}
                }
            }
            Some("file-loaded") => {
                st.loaded = true;
                st.has_video = true;
                st.error = None;
            }
            Some("end-file") if v.get("reason").and_then(Value::as_str) == Some("error") => {
                st.error = Some(v.get("file_error").and_then(Value::as_str).unwrap_or("playback failed").to_string());
            }
            Some("shutdown") => break,
            _ => {}
        }
    }
    state.lock().unwrap_or_else(|e| e.into_inner()).exited = true;
}

pub fn parse_tracks(v: &Value) -> Vec<Track> {
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|t| {
                    let s = |k: &str| t.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                    let id = t.get("id").and_then(Value::as_i64).unwrap_or(0);
                    let external = t.get("external").and_then(Value::as_bool).unwrap_or(false);
                    Track {
                        id,
                        kind: s("type"),
                        label: track_label(id, &s("title"), &s("lang"), &s("codec"), external),
                        lang: s("lang"),
                        external,
                        selected: t.get("selected").and_then(Value::as_bool).unwrap_or(false),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Engine for ExtMpv {
    fn snapshot(&self) -> PlayerState {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Ok(Some(_)) = self.child.lock().unwrap_or_else(|e| e.into_inner()).try_wait() {
            st.exited = true;
        }
        st
    }
    fn command(&self, args: &[&str]) {
        self.send(json!({ "command": args }));
    }
    fn get_string(&self, _name: &str) -> Option<String> {
        None
    }
    fn external(&self) -> bool {
        true
    }
}

impl Drop for ExtMpv {
    fn drop(&mut self) {
        self.send(json!({"command": ["quit"]}));
        let mut c = self.child.lock().unwrap_or_else(|e| e.into_inner());
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = c.try_wait() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = c.kill();
        let _ = c.wait();
        let _ = std::fs::remove_file(&self.socket);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in for mpv's IPC server: it records the commands it gets
    /// and answers with property changes, then closes like mpv on quit.
    #[test]
    fn session_follows_ipc_events_and_sends_commands() {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixListener;
        let sock = std::env::temp_dir().join(format!("lain-ipc-test-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&sock);
        let listener = UnixListener::bind(&sock).unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let server = std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            let mut w = s.try_clone().unwrap();
            let events = [
                r#"{"event":"property-change","id":2,"name":"duration","data":1440.5}"#,
                r#"{"event":"property-change","id":1,"name":"time-pos","data":61.25}"#,
                r#"{"event":"property-change","id":3,"name":"pause","data":true}"#,
                r#"{"event":"file-loaded"}"#,
            ];
            for e in events {
                writeln!(w, "{e}").unwrap();
            }
            for line in BufReader::new(s).lines() {
                let line = line.unwrap();
                let quit = line.contains("\"quit\"");
                tx.send(line).unwrap();
                if quit {
                    writeln!(w, r#"{{"event":"shutdown"}}"#).unwrap();
                    break;
                }
            }
        });
        let child = Command::new("sleep").arg("30").spawn().unwrap();
        let m = ExtMpv::attach(child, sock.clone()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while m.snapshot().time == 0. && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let st = m.snapshot();
        assert_eq!((st.time, st.duration, st.paused, st.loaded), (61.25, 1440.5, true, true));
        m.command(&["seek", "5", "relative"]);
        drop(m); // sends quit, kills the stand-in process
        server.join().unwrap();
        let got: Vec<String> = rx.try_iter().collect();
        assert!(got.iter().any(|l| l.contains(r#""observe_property",1,"time-pos""#)), "{got:?}");
        assert!(got.iter().any(|l| l.contains(r#"["seek","5","relative"]"#)), "{got:?}");
        assert!(!sock.exists());
    }

    #[test]
    fn tracks_parse_from_ipc_json() {
        let v: Value = serde_json::from_str(
            r#"[{"id":1,"type":"audio","lang":"jpn","codec":"flac","selected":true},
                {"id":2,"type":"sub","title":"Signs","lang":"eng","codec":"ass","external":true}]"#,
        )
        .unwrap();
        let t = parse_tracks(&v);
        assert_eq!(t.len(), 2);
        assert!(t[0].selected && t[0].label == "jpn · flac");
        assert!(t[1].external && t[1].label.contains("sidecar"));
    }
}
