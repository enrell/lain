//! What the player UI needs from a playback engine, whichever it is:
//! libmpv embedded in the window (Windows, `mpv.rs`) or an external mpv
//! process driven over its JSON IPC socket (Linux, `mpv_ipc.rs`).

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Track {
    pub id: i64,
    pub kind: String,
    pub label: String,
    pub lang: String,
    pub external: bool,
    pub selected: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlayerState {
    pub time: f64,
    pub duration: f64,
    pub paused: bool,
    pub volume: f64,
    pub muted: bool,
    /// Seconds of demuxed data ahead of the start (`demuxer-cache-time`).
    pub cache_end: f64,
    pub tracks: Vec<Track>,
    pub loaded: bool,
    /// The VO is configured: the first frame is (about to be) on screen.
    pub has_video: bool,
    pub eof: bool,
    pub error: Option<String>,
    /// The external player process is gone (closed by the user).
    pub exited: bool,
}

/// A running engine. Commands never block the UI thread.
pub trait Engine: Send + Sync {
    fn snapshot(&self) -> PlayerState;
    /// Fire and forget (`seek`, `cycle`, `set`, `sub-add`, ...).
    fn command(&self, args: &[&str]);
    fn get_string(&self, name: &str) -> Option<String>;
    /// Video shows in its own window, not under the app's controls.
    fn external(&self) -> bool;
}

#[cfg(windows)]
impl Engine for crate::mpv::Mpv {
    fn snapshot(&self) -> PlayerState {
        crate::mpv::Mpv::snapshot(self)
    }
    fn command(&self, args: &[&str]) {
        self.command_async(args)
    }
    fn get_string(&self, name: &str) -> Option<String> {
        crate::mpv::Mpv::get_string(self, name)
    }
    fn external(&self) -> bool {
        false
    }
}

/// A track label from mpv's `track-list` fields.
pub fn track_label(id: i64, title: &str, lang: &str, codec: &str, external: bool) -> String {
    let mut label = match (title.is_empty(), lang.is_empty()) {
        (false, false) => format!("{title} ({lang})"),
        (false, true) => title.to_string(),
        (true, false) => lang.to_string(),
        (true, true) => format!("Track {id}"),
    };
    if !codec.is_empty() {
        label = format!("{label} · {codec}");
    }
    if external {
        label = format!("{label} · sidecar");
    }
    label
}
