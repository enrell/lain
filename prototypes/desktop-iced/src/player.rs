//! Player state and the mpv glue: engine setup, the callbacks that become
//! iced messages (through a subscription), event pumping and tracks.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

use iced::Animation;
use iced::futures::channel::mpsc;
use iced::futures::stream::{BoxStream, StreamExt};
use mpv_engine::{Engine, ExportOptions, PlaybackEvent, PropertyFormat, PropertyValue};

use crate::data::Item;
use crate::video::VideoShared;

#[derive(Debug, Clone, Copy)]
pub enum VideoEvt {
    Frame,
    Wake,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    Audio,
    Subs,
}

#[derive(Debug, Clone)]
pub struct Track {
    pub kind: String,
    pub id: i64,
    pub label: String,
    pub selected: bool,
}

pub struct Player {
    pub item: Item,
    pub media: PathBuf,
    pub video: Option<Arc<VideoShared>>,
    pub position: f64,
    pub duration: f64,
    pub buffered: f64,
    pub paused: bool,
    pub volume: f64,
    pub muted: bool,
    pub tracks: Vec<Track>,
    pub menu: Option<Menu>,
    pub last_activity: Instant,
    pub chrome: Animation<bool>,
    pub error: Option<String>,
    pub last_saved: Instant,
    pub hwdec: String,
    pub codec: String,
    pub loaded: bool,
    /// Bench: controls stay visible.
    pub pinned: bool,
    /// `current-ao` logged once playback is under way.
    pub ao_logged: bool,
}

impl Player {
    pub fn new(item: Item, media: PathBuf, resume_from: f64, chrome: Animation<bool>, now: Instant) -> Self {
        Player {
            item,
            media,
            video: None,
            position: resume_from,
            duration: 0.0,
            buffered: 0.0,
            paused: false,
            volume: 80.0,
            muted: false,
            tracks: Vec::new(),
            menu: None,
            last_activity: now,
            chrome,
            error: None,
            last_saved: now,
            hwdec: String::new(),
            codec: String::new(),
            loaded: false,
            pinned: false,
            ao_logged: false,
        }
    }

    pub fn engine(&self) -> Option<&Arc<Engine>> {
        self.video.as_ref().map(|v| &v.engine)
    }

    /// Drain mpv's queued events into the state.
    pub fn pump(&mut self) {
        let Some(v) = self.video.clone() else { return };
        v.wake_pending.store(false, std::sync::atomic::Ordering::SeqCst);
        for ev in v.engine.pump_events() {
            match ev {
                PlaybackEvent::PropertyChanged { name, value, .. } => {
                    let f = match value {
                        PropertyValue::Double(d) => Some(d),
                        PropertyValue::Int(i) => Some(i as f64),
                        _ => None,
                    };
                    let b = match value {
                        PropertyValue::Flag(b) => Some(b),
                        _ => None,
                    };
                    match name.as_str() {
                        "time-pos" => self.position = f.unwrap_or(self.position),
                        "duration" => self.duration = f.unwrap_or(self.duration),
                        "demuxer-cache-time" => self.buffered = f.unwrap_or(self.buffered),
                        "pause" => self.paused = b.unwrap_or(self.paused),
                        "volume" => self.volume = f.unwrap_or(self.volume),
                        "mute" => self.muted = b.unwrap_or(self.muted),
                        "hwdec-current" => {
                            if let PropertyValue::Str(s) = &value {
                                self.hwdec = s.clone();
                            }
                        }
                        "aid" | "sid" => self.read_tracks(),
                        _ => {}
                    }
                }
                PlaybackEvent::Loaded => {
                    self.loaded = true;
                    if let Some(e) = self.engine() {
                        let ao = e.get_property::<String>("current-ao").unwrap_or_else(|_| "none".into());
                        crate::placement::log(&format!("mpv loaded {}: current-ao={ao}", self.media.display()));
                    }
                    self.read_tracks();
                    // Show the first subtitle track when the file flags none
                    // (the test file's ASS typesetting must be on screen).
                    if !self.tracks.iter().any(|t| t.kind == "sub" && t.selected) {
                        if let Some(id) = self.tracks.iter().find(|t| t.kind == "sub").map(|t| t.id) {
                            self.select_track("sub", Some(id));
                        }
                    }
                    if let Some(e) = self.engine() {
                        self.codec = e.get_property::<String>("video-codec").unwrap_or_default();
                    }
                }
                PlaybackEvent::Failed { message, .. } => self.error = Some(message),
                PlaybackEvent::Ended { .. } => self.paused = true,
                _ => {}
            }
        }
    }

    fn read_tracks(&mut self) {
        let Some(e) = self.engine().cloned() else { return };
        let n = e.get_property::<i64>("track-list/count").unwrap_or(0);
        let mut out = Vec::new();
        for i in 0..n {
            let g = |k: &str| e.get_property::<String>(&format!("track-list/{i}/{k}")).unwrap_or_default();
            let kind = g("type");
            if kind != "audio" && kind != "sub" {
                continue;
            }
            let id = e.get_property::<i64>(&format!("track-list/{i}/id")).unwrap_or(0);
            let selected = e.get_property::<bool>(&format!("track-list/{i}/selected")).unwrap_or(false);
            let (title, lang, codec) = (g("title"), g("lang"), g("codec"));
            let mut label = if !title.is_empty() { title } else if !lang.is_empty() { lang.clone() } else { format!("Track {id}") };
            if !lang.is_empty() && !label.contains(&lang) {
                label = format!("{label} · {lang}");
            }
            if !codec.is_empty() {
                label = format!("{label} ({codec})");
            }
            out.push(Track { kind, id, label, selected });
        }
        self.tracks = out;
    }

    pub fn select_track(&mut self, kind: &str, id: Option<i64>) {
        if let Some(e) = self.engine() {
            let prop = if kind == "audio" { "aid" } else { "sid" };
            let v = id.map(|i| i.to_string()).unwrap_or_else(|| "no".into());
            let _ = e.set_property(prop, v.as_str());
        }
        self.read_tracks();
    }
}

// ---- engine setup ------------------------------------------------------------

/// `--mute`: no audio device at all (ao=null); test runs are silent.
pub static MUTE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

static VIDEO_RX: Mutex<Option<(u64, mpsc::UnboundedReceiver<VideoEvt>)>> = Mutex::new(None);

/// The subscription stream for session `id`: mpv's frame and wakeup
/// callbacks, already coalesced.
pub fn video_events(id: &u64) -> BoxStream<'static, VideoEvt> {
    let mut g = VIDEO_RX.lock().unwrap_or_else(|e| e.into_inner());
    match g.take() {
        Some((sid, rx)) if sid == *id => rx.boxed(),
        other => {
            *g = other;
            iced::futures::stream::pending().boxed()
        }
    }
}

/// Create the engine, attach the exported renderer at `w`×`h`, and load.
pub fn open(id: u64, media: &Path, start: f64, w: u32, h: u32) -> Result<Arc<VideoShared>, String> {
    let mut b = Engine::video()
        .property("hwdec", "auto-safe")
        .property("keep-open", "yes")
        .property("input-default-bindings", "no")
        .property("input-vo-keyboard", "no")
        .property("osc", "no")
        .property("volume", "80")
        .property("sub-auto", "no")
        .property("msg-level", "all=warn");
    // Experiments without a rebuild: LAIN_MPV_OPTS="hwdec=nvdec-copy,video-sync=display-resample".
    if let Ok(extra) = std::env::var("LAIN_MPV_OPTS") {
        for kv in extra.split(',').filter(|s| !s.is_empty()) {
            if let Some((k, v)) = kv.split_once('=') {
                b = b.property(k.trim(), v.trim());
            }
        }
    }
    if MUTE.load(std::sync::atomic::Ordering::Relaxed) {
        b = b.property("ao", "null").property("mute", "yes");
    }
    if start > 1.0 {
        b = b.property("start", &format!("{start:.3}"));
    }
    let engine = Arc::new(b.build().map_err(|e| format!("mpv: {e}"))?);
    for (name, fmt) in [
        ("time-pos", PropertyFormat::Double),
        ("duration", PropertyFormat::Double),
        ("demuxer-cache-time", PropertyFormat::Double),
        ("pause", PropertyFormat::Flag),
        ("volume", PropertyFormat::Double),
        ("mute", PropertyFormat::Flag),
        ("hwdec-current", PropertyFormat::Str),
        ("aid", PropertyFormat::Str),
        ("sid", PropertyFormat::Str),
    ] {
        let _ = engine.observe(name, fmt);
    }
    let shared = Arc::new(VideoShared::new(id, engine.clone(), w, h));
    let (tx, rx) = mpsc::unbounded();
    *VIDEO_RX.lock().unwrap_or_else(|e| e.into_inner()) = Some((id, rx));

    let weak: Weak<VideoShared> = Arc::downgrade(&shared);
    let tx_wake = tx.clone();
    engine.set_wakeup_callback(move || {
        if let Some(s) = weak.upgrade() {
            if !s.wake_pending.swap(true, std::sync::atomic::Ordering::SeqCst) {
                let _ = tx_wake.unbounded_send(VideoEvt::Wake);
            }
        }
    });
    let weak: Weak<VideoShared> = Arc::downgrade(&shared);
    engine
        .attach_exported_render(ExportOptions::new(w.max(16), h.max(16)).pool_size(5), move || {
            if let Some(s) = weak.upgrade() {
                if !s.frame_pending.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    let _ = tx.unbounded_send(VideoEvt::Frame);
                }
            }
        })
        .map_err(|e| format!("mpv render: {e}"))?;
    if MUTE.load(std::sync::atomic::Ordering::Relaxed) {
        let ao = engine.get_property::<String>("current-ao").unwrap_or_else(|_| "(none yet)".into());
        let opt = engine.get_property::<String>("ao").unwrap_or_default();
        crate::placement::log(&format!("mpv session {id}: --mute ao={opt} current-ao={ao}"));
    }
    engine.load(&media.to_string_lossy()).map_err(|e| format!("mpv load: {e}"))?;
    Ok(shared)
}

/// Tear the session down off the UI thread (mpv's terminate can take a
/// few tens of ms).
pub fn close(video: Arc<VideoShared>) {
    std::thread::spawn(move || {
        let _ = video.engine.stop();
        video.engine.detach_render();
        drop(video);
    });
}

pub fn fmt_time(s: f64) -> String {
    let s = s.max(0.0) as u64;
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}
