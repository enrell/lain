//! The transcode provider (`lain-transcode-ffmpeg`): the synchronous v1
//! contract and the v3 session contract over one private cache
//! (D-023/D-028/D-042..D-045, Go's `plugins/transcode`).
//!
//! ffmpeg and ffprobe are external processes. Work runs on its own
//! threads, never on the async runtime: a dispatcher admits queued jobs
//! into at most `max_concurrent` runner threads, and a ticker keeps HLS
//! playlists fresh, throttles ffmpeg, deletes consumed segments and
//! stops abandoned sessions. Ready output is an LRU cache bounded by
//! `cache_bytes`, keyed by source identity and every option that
//! changes the bytes. Paths are trusted-plane: the gateway never shows
//! them to a client.
//!
//! `lain.playback.transcode@2` is not ported: no route reaches it
//! (D-190).

pub mod effects;
pub mod encode;
pub mod hls;
pub mod policy;
pub mod process;

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant, UNIX_EPOCH};

use lain_api::{TranscodeSettings, TranscodeStatus};
use lain_core::Error;
use lain_probe::{RawReport, RawStream, exec, text_subtitle_codec};
use serde::{Deserialize, Serialize};

use crate::settings::{self as st, sha256_hex};
pub use policy::{Capabilities, EncodePlan, plan_v3};

pub const ID: &str = "lain-transcode-ffmpeg";
pub const CAP_V1: &str = "lain.playback.transcode@1";
pub const CAP_V3: &str = "lain.playback.transcode@3";

pub const IDLE: &str = "idle";
pub const QUEUED: &str = "queued";
pub const RUNNING: &str = "running";
pub const READY: &str = "ready";
pub const FAILED: &str = "failed";

pub const ACTION_INSPECT: &str = "inspect";
pub const ACTION_START: &str = "start";
pub const ACTION_STATUS: &str = "status";
pub const ACTION_RESOLVE: &str = "resolve";
pub const ACTION_CANCEL: &str = "cancel";
pub const ACTION_POSITION: &str = "position";
pub const ACTION_LIST: &str = "list";
pub const ACTION_SUBTITLES: &str = "subtitles";

const LEGACY_PROFILE: &str = "mp4-compat-v1";
const SUBTITLE_PROFILE: &str = "subs-vtt-v1";
const PROGRESSIVE_TIMEOUT: Duration = Duration::from_secs(3600);
const SUBTITLE_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_QUEUE_CAPACITY: i64 = 64;
const MAX_TRACKED_JOBS: usize = 128;
const RECENT_ACCESS_GRACE: i64 = 60;
/// A probed capability set is current this long. Past it a caller still
/// gets it at once while one background probe replaces it: the ffmpeg
/// build and the GPUs rarely change, and a probe costs seconds on a
/// machine with a GPU (D-031).
const CAP_FRESH: Duration = Duration::from_secs(600);
/// How long an answer in doubt stays current: one where a check timed
/// out, or one that lost something the previous probe found. A check
/// can miss its timeout on a busy machine (the libplacebo one takes
/// seconds even when idle), and the first probe runs while the server
/// starts, so such an answer is probed again this soon instead of
/// standing for [`CAP_FRESH`]. A loss the next probe confirms with no
/// timeout is then current as usual.
const CAP_RECHECK: Duration = Duration::from_secs(30);
/// Capability sets kept, one per ffmpeg binary and VA-API device; more
/// only comes from an operator trying paths.
const CAP_KEYS: usize = 8;

pub(crate) fn unsupported(msg: &str) -> Error {
    Error::new("unsupported-media", msg)
}

fn unavailable(msg: &str) -> Error {
    Error::unavailable(msg)
}

/// The requesting account's limits (D-042); absent is unrestricted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    pub allow_video_transcode: bool,
    pub allow_audio_transcode: bool,
    pub allow_remux: bool,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub max_streams: i64,
}

fn is_zero(v: &i64) -> bool {
    *v == 0
}

/// One v3 call. `file_path` always comes from the gateway's catalog
/// lookup; `settings` is the effective operator policy.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct V3Request {
    pub action: String,
    pub file_path: String,
    pub session: String,
    pub delivery: String,
    pub quality: String,
    pub max_bitrate_kbps: i64,
    pub video_codec: String,
    pub audio_codec: String,
    pub audio_stream: Option<i64>,
    pub subtitle_stream: Option<i64>,
    pub subtitle_mode: String,
    pub start_sec: f64,
    pub segment_index: i64,
    pub allow_video_stream_copy: Option<bool>,
    pub allow_audio_stream_copy: Option<bool>,
    /// Codecs beyond H.264 the client decodes in fMP4 (D-273).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub video_copy_codecs: Vec<String>,
    /// A video effect rendered by the server (D-274): a preset id.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub video_effect: String,
    pub policy: Option<Policy>,
    /// Opaque account reference: per-user stream counts and the admin
    /// list.
    pub user_id: String,
    /// Rust-only label for `transcode-changed` events.
    pub item_id: String,
    pub settings: TranscodeSettings,
}

/// The provider-side status. `path`, `playlist_path` and
/// `subtitle_path` are trusted-plane; [`V3Status::public`] drops them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct V3Status {
    pub session: String,
    pub state: String,
    pub profile: String,
    pub delivery: String,
    pub path: String,
    pub playlist_path: String,
    pub subtitle_path: String,
    pub method: String,
    pub cached: bool,
    pub playable: bool,
    pub video_direct: bool,
    pub audio_direct: bool,
    pub encoder: String,
    pub hardware: String,
    pub fallback: String,
    pub reasons: Vec<String>,
    pub video_codec: String,
    pub audio_codec: String,
    pub width: i64,
    pub height: i64,
    pub bitrate_kbps: i64,
    pub fps: f64,
    pub output_bitrate_kbps: i64,
    pub user_id: String,
    pub item_id: String,
    pub error_code: String,
    pub error: String,
    pub queued_at: i64,
    pub started_at: i64,
    pub finished_at: i64,
    pub progress: f64,
}

impl V3Status {
    fn idle(session: &str, profile: &str) -> V3Status {
        V3Status { session: session.into(), state: IDLE.into(), profile: profile.into(), ..V3Status::default() }
    }

    /// The client-safe projection: no path ever leaves the server.
    pub fn public(&self) -> TranscodeStatus {
        TranscodeStatus {
            session: self.session.clone(),
            state: self.state.clone(),
            profile: self.profile.clone(),
            delivery: self.delivery.clone(),
            method: self.method.clone(),
            cached: self.cached,
            playable: self.playable,
            encoder: self.encoder.clone(),
            hardware: self.hardware.clone(),
            fallback: self.fallback.clone(),
            reasons: self.reasons.clone(),
            video_codec: self.video_codec.clone(),
            audio_codec: self.audio_codec.clone(),
            width: self.width,
            height: self.height,
            bitrate_kbps: self.bitrate_kbps,
            video_direct: self.video_direct,
            audio_direct: self.audio_direct,
            fps: self.fps,
            output_bitrate_kbps: self.output_bitrate_kbps,
            progress: self.progress,
            has_subtitle: !self.subtitle_path.is_empty(),
            error_code: self.error_code.clone(),
            error: self.error.clone(),
            queued_at: self.queued_at,
            started_at: self.started_at,
            finished_at: self.finished_at,
        }
    }
}

/// The v1 result: a complete MP4 on disk.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyResult {
    pub path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub method: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cached: bool,
}

/// The identity of one session's output.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SourceSpec {
    pub path: String,
    pub size: i64,
    pub mtime_ns: i64,
    pub profile: String,
    pub audio_stream: Option<i64>,
    pub subtitle_stream: Option<i64>,
    pub start_sec: f64,
    pub session: String,
    pub settings: TranscodeSettings,
}

fn session_key(s: &SourceSpec) -> String {
    let a = s.audio_stream.map(|n| n.to_string()).unwrap_or_else(|| "default".into());
    let sub = s.subtitle_stream.map(|n| n.to_string()).unwrap_or_else(|| "off".into());
    sha256_hex(&format!("{}|{}|{}|{:.3}|{}|a:{a}|s:{sub}", s.path, s.mtime_ns, s.size, s.start_sec, s.profile))
}

/// Exactly the 64-hex ids [`session_key`] makes; client-supplied ids
/// are checked before ever joining a path.
pub fn valid_session_id(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn file_identity(path: &str) -> Option<(i64, i64)> {
    let m = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
    let mtime = m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos() as i64;
    Some((m.len() as i64, mtime))
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct CacheEntry {
    session: String,
    source_path: String,
    source_size: i64,
    #[serde(rename = "source_mod_time_ns")]
    source_mtime: i64,
    profile: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    delivery: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    artifact_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    audio_stream: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    subtitle_stream: Option<i64>,
    #[serde(default)]
    has_subtitle: bool,
    method: String,
    size: i64,
    created_at: i64,
    accessed_at: i64,
    #[serde(default)]
    encoder: String,
    #[serde(default)]
    hardware: String,
    #[serde(default)]
    fallback: String,
    #[serde(default)]
    reasons: Vec<String>,
    #[serde(default)]
    video_codec: String,
    #[serde(default)]
    audio_codec: String,
    #[serde(default)]
    width: i64,
    #[serde(default)]
    height: i64,
    #[serde(default)]
    bitrate_kbps: i64,
    #[serde(default)]
    video_direct: bool,
    #[serde(default)]
    audio_direct: bool,
}

/// One session's artifact locations.
#[derive(Debug, Clone, PartialEq)]
pub struct Paths {
    pub media: PathBuf,
    pub hls_dir: PathBuf,
    pub playlist: PathBuf,
    pub subtitle: PathBuf,
    pub burn: PathBuf,
}

/// The owned subdirectory of a configured temp path.
fn relocate_root(s: &TranscodeSettings) -> Option<PathBuf> {
    let r = s.transcode_temp_path.trim();
    (!r.is_empty()).then(|| Path::new(r).join("lain-transcode"))
}

fn relocated_paths(root: &Path, session: &str) -> Paths {
    let d = root.join(session);
    Paths {
        media: d.join("media.mp4"),
        playlist: d.join(hls::INDEX_PLAYLIST),
        subtitle: d.join("subs.vtt"),
        burn: d.join("burn.ass"),
        hls_dir: d,
    }
}

/// Creates and checks the temp path so a bad volume fails admission,
/// not an encode.
fn ensure_relocated_root(s: &TranscodeSettings) -> Result<(), String> {
    let Some(root) = relocate_root(s) else { return Ok(()) };
    std::fs::create_dir_all(&root).map_err(|e| format!("transcode temp path: {e}"))?;
    let probe = root.join(".writable");
    std::fs::write(&probe, b"").map_err(|e| format!("transcode temp path: {e}"))?;
    let _ = std::fs::remove_file(probe);
    Ok(())
}

struct Job {
    epoch: u64,
    spec: SourceSpec,
    state: String,
    method: String,
    progress: f64,
    err_code: String,
    err_msg: String,
    queued_at: i64,
    started_at: i64,
    finished_at: i64,
    v3: bool,
    delivery: String,
    user_id: String,
    item_id: String,
    settings: TranscodeSettings,
    plan: Option<EncodePlan>,
    handle: Option<process::Handle>,
    paused: bool,
    stopped: bool,
    fps: f64,
    output_kbps: i64,
    client_segment: i64,
    last_touch: i64,
    playable: bool,
    has_subtitle: bool,
    encoder: String,
    hw_backend: String,
    fallback: String,
}

impl Job {
    fn new(epoch: u64, spec: SourceSpec, now: i64) -> Job {
        Job {
            epoch,
            settings: spec.settings.clone(),
            spec,
            state: QUEUED.into(),
            method: String::new(),
            progress: 0.0,
            err_code: String::new(),
            err_msg: String::new(),
            queued_at: now,
            started_at: 0,
            finished_at: 0,
            v3: false,
            delivery: String::new(),
            user_id: String::new(),
            item_id: String::new(),
            plan: None,
            handle: None,
            paused: false,
            stopped: false,
            fps: 0.0,
            output_kbps: 0,
            client_segment: 0,
            last_touch: now,
            playable: false,
            has_subtitle: false,
            encoder: String::new(),
            hw_backend: String::new(),
            fallback: String::new(),
        }
    }

    fn active(&self) -> bool {
        self.state == QUEUED || self.state == RUNNING
    }
}

struct State {
    jobs: HashMap<String, Job>,
    queue: VecDeque<(String, u64)>,
    closed: bool,
    active: i64,
    pending: i64,
    queue_limit: i64,
    max_concurrent: i64,
    max_cache_bytes: i64,
    next_gen: u64,
    ffmpeg: String,
}

/// What a test run hook sees (it replaces the ffmpeg work).
pub struct RunCtx {
    pub session: String,
    pub delivery: String,
    pub paths: Paths,
}

type ProbeFn = Arc<dyn Fn(&Path) -> Result<RawReport, String> + Send + Sync>;
type CapsFn = Arc<dyn Fn(&TranscodeSettings) -> Capabilities + Send + Sync>;
type RunFn = Arc<dyn Fn(&RunCtx) -> Result<(), String> + Send + Sync>;
type ClockFn = Arc<dyn Fn() -> i64 + Send + Sync>;
type ChangeFn = Box<dyn Fn(&V3Status) + Send + Sync>;

/// Test seams: a fake probe, capability set, run or clock.
#[derive(Default, Clone)]
pub struct Hooks {
    pub probe: Option<ProbeFn>,
    /// Stands in for the ffmpeg capability probe; its answers are cached
    /// and shared like the real probe's.
    pub caps: Option<CapsFn>,
    pub run: Option<RunFn>,
    pub clock: Option<ClockFn>,
}

struct Inner {
    dir: PathBuf,
    state: Mutex<State>,
    cv: Condvar,
    caps: Mutex<CapsCache>,
    /// Signalled whenever a capability probe lands.
    caps_cv: Condvar,
    hooks: Hooks,
    on_change: Mutex<Option<ChangeFn>>,
    init_err: Option<String>,
}

/// Probed capability sets by [`caps_key`].
struct CapsCache {
    slots: HashMap<String, CapsSlot>,
    /// [`CAP_FRESH`] and [`CAP_RECHECK`]; tests shorten them.
    fresh: Duration,
    recheck: Duration,
    /// Each ffmpeg run's timeout in a probe ([`policy::PROBE_TIMEOUT`]);
    /// tests shorten it.
    limit: Duration,
}

#[derive(Default)]
struct CapsSlot {
    /// The last probe's answer and when it landed.
    value: Option<(Instant, Capabilities)>,
    /// The answer is in doubt (a check timed out, or it lost something
    /// the one before had): current only for [`CAP_RECHECK`].
    doubtful: bool,
    /// A probe for this key is running. Other callers keep the last
    /// answer or wait for this one; none starts a second (single flight).
    running: bool,
    /// A forced probe was asked for while one ran. The running one may
    /// have started before what prompted it (a driver installed, ffmpeg
    /// replaced in place, then the settings saved), so one more runs
    /// when it lands.
    again: bool,
}

/// Whether the probe `now` lost an execution probe `before` passed (a
/// tone mapping path, the video effects, a hardware backend).
fn lost(before: &Capabilities, now: &Capabilities) -> bool {
    (before.tone_map && !now.tone_map)
        || (before.tone_map_2390 && !now.tone_map_2390)
        || (before.effects && !now.effects)
        || before.hardware.iter().any(|(b, ok)| *ok && !now.hardware.get(b).copied().unwrap_or(false))
}

/// What publishing a probe's answer found.
struct Stored {
    /// It differs from the answer before.
    changed: bool,
    /// A check timed out, or it lost something the answer before had.
    doubtful: bool,
    /// A forced probe was asked for while this one ran: the key stays
    /// claimed and the prober probes again.
    again: bool,
}

impl CapsCache {
    /// Whether the caller starts a probe for `key`: none is running, and
    /// there is no answer yet, a stale one, or `force`. Claiming marks
    /// the probe running. A forced claim while one runs queues one more
    /// after it instead.
    fn claim(&mut self, key: &str, force: bool) -> bool {
        let (fresh, recheck) = (self.fresh, self.recheck);
        let slot = self.slots.entry(key.to_string()).or_default();
        if slot.running {
            slot.again |= force;
            return false;
        }
        let life = if slot.doubtful { fresh.min(recheck) } else { fresh };
        slot.running = force || slot.value.as_ref().is_none_or(|(at, _)| at.elapsed() >= life);
        slot.running
    }

    fn current(&self, key: &str) -> Option<Capabilities> {
        self.slots.get(key)?.value.as_ref().map(|(_, c)| c.clone())
    }

    /// Publishes a probe's answer as probed (D-031: what the page shows is
    /// what ffmpeg did). `timed_out`: one of its ffmpeg runs was killed at
    /// its timeout, so a no in it may be a yes on a calmer machine.
    fn store(&mut self, key: &str, caps: Capabilities, timed_out: bool) -> Stored {
        let slot = self.slots.entry(key.to_string()).or_default();
        let mut stored = Stored {
            changed: slot.value.as_ref().is_none_or(|(_, old)| *old != caps),
            doubtful: timed_out || slot.value.as_ref().is_some_and(|(_, old)| lost(old, &caps)),
            again: false,
        };
        slot.value = Some((Instant::now(), caps));
        slot.doubtful = stored.doubtful;
        slot.running = std::mem::take(&mut slot.again);
        stored.again = slot.running;
        while self.slots.len() > CAP_KEYS {
            // The idle set probed longest ago goes first.
            let oldest = self
                .slots
                .iter()
                .filter(|(_, s)| !s.running)
                .min_by_key(|(_, s)| s.value.as_ref().map(|(at, _)| *at))
                .map(|(k, _)| k.clone());
            let Some(oldest) = oldest else { break };
            self.slots.remove(&oldest);
        }
        stored
    }
}

/// What a capability probe reads from the settings: the ffmpeg binary
/// and the VA-API device. Changing any other setting keeps the set.
fn caps_key(s: &TranscodeSettings) -> String {
    format!("{}|{}", policy::ffmpeg_binary(s), policy::hardware_device(s))
}

enum Fail {
    Core(Error),
    Plain(String),
}

impl From<Error> for Fail {
    fn from(e: Error) -> Self {
        Fail::Core(e)
    }
}

impl From<String> for Fail {
    fn from(e: String) -> Self {
        Fail::Plain(e)
    }
}

/// The transcoder. Clones share one cache and one set of sessions.
#[derive(Clone)]
pub struct Transcoder {
    inner: Arc<Inner>,
}

fn write_json_atomic(path: &Path, v: &impl Serialize) -> std::io::Result<()> {
    let raw = serde_json::to_vec(v).map_err(std::io::Error::other)?;
    hls::write_atomic(path, &raw)
}

fn read_entry(path: &Path) -> Option<CacheEntry> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn file_len(p: &Path) -> Option<i64> {
    std::fs::metadata(p).ok().filter(|m| m.is_file()).map(|m| m.len() as i64)
}

impl Transcoder {
    /// A transcoder over `dir` (created 0700) with production behavior.
    pub fn new(dir: &Path) -> Transcoder {
        Transcoder::with_hooks(dir, Hooks::default())
    }

    pub fn with_hooks(dir: &Path, hooks: Hooks) -> Transcoder {
        let init_err =
            lain_store::ensure_private_dir(dir).err().map(|e| format!("transcode cache {}: {e}", dir.display()));
        let d = st::defaults();
        let t = Transcoder {
            inner: Arc::new(Inner {
                dir: dir.to_path_buf(),
                state: Mutex::new(State {
                    jobs: HashMap::new(),
                    queue: VecDeque::new(),
                    closed: false,
                    active: 0,
                    pending: 0,
                    queue_limit: d.queue_size,
                    max_concurrent: d.max_concurrent,
                    max_cache_bytes: d.cache_bytes,
                    next_gen: 1,
                    ffmpeg: "ffmpeg".into(),
                }),
                cv: Condvar::new(),
                caps: Mutex::new(CapsCache {
                    slots: HashMap::new(),
                    fresh: CAP_FRESH,
                    recheck: CAP_RECHECK,
                    limit: policy::PROBE_TIMEOUT,
                }),
                caps_cv: Condvar::new(),
                hooks,
                on_change: Mutex::new(None),
                init_err,
            }),
        };
        if t.inner.init_err.is_none()
            && let Err(e) = t.cleanup(true, &Default::default())
        {
            tracing::warn!(err = %e, "transcode cache cleanup failed");
        }
        t.spawn_background();
        t
    }

    /// Called on every terminal transition and cancel (events).
    pub fn on_change(&self, f: impl Fn(&V3Status) + Send + Sync + 'static) {
        *self.inner.on_change.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(f));
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.inner.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn now(&self) -> i64 {
        match &self.inner.hooks.clock {
            Some(c) => c(),
            None => lain_core::now_unix(),
        }
    }

    fn changed(&self, s: &V3Status) {
        if let Some(f) = self.inner.on_change.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            f(s);
        }
    }

    fn spawn_background(&self) {
        let weak = Arc::downgrade(&self.inner);
        std::thread::Builder::new().name("transcode-dispatch".into()).spawn(move || dispatch(weak)).ok();
        let weak = Arc::downgrade(&self.inner);
        std::thread::Builder::new()
            .name("transcode-tick".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_secs(1));
                    let Some(inner) = weak.upgrade() else { return };
                    let t = Transcoder { inner };
                    if t.lock().closed {
                        return;
                    }
                    t.tick();
                }
            })
            .ok();
    }

    /// Fails queued work, kills running ffmpeg, stops the threads.
    pub fn close(&self) {
        let mut handles = Vec::new();
        let mut changed = Vec::new();
        {
            let mut g = self.lock();
            if g.closed {
                return;
            }
            g.closed = true;
            let now = self.now();
            for j in g.jobs.values_mut() {
                if j.state == QUEUED {
                    j.state = FAILED.into();
                    j.err_code = "dependency-unavailable".into();
                    j.err_msg = "transcoder stopped".into();
                    j.finished_at = now;
                }
                if let Some(h) = j.handle.take() {
                    handles.push(h);
                }
            }
            for j in g.jobs.values() {
                if j.state == FAILED && j.err_msg == "transcoder stopped" {
                    changed.push(self.status_from_job(j));
                }
            }
            self.inner.cv.notify_all();
        }
        for h in handles {
            h.resume();
            h.kill();
        }
        for s in changed {
            self.changed(&s);
        }
    }

    /// Cheap and side-effect free: the cache is usable and ffmpeg exists.
    pub fn health(&self) -> Result<(), Error> {
        if let Some(e) = &self.inner.init_err {
            return Err(unavailable(e));
        }
        if self.inner.hooks.run.is_none() {
            let ffmpeg = self.lock().ffmpeg.clone();
            if exec::look_path(&ffmpeg).is_none() {
                return Err(unavailable(&format!("{ffmpeg} not found")));
            }
        }
        if !self.inner.dir.is_dir() {
            return Err(unavailable(&format!("transcode cache {} unusable", self.inner.dir.display())));
        }
        Ok(())
    }

    // ---- paths ----------------------------------------------------------

    fn default_paths(&self, session: &str) -> Paths {
        let dir = &self.inner.dir;
        let hls = dir.join(format!("{session}.hls"));
        Paths {
            media: dir.join(format!("{session}.mp4")),
            playlist: hls.join(hls::INDEX_PLAYLIST),
            subtitle: dir.join(format!("{session}.vtt")),
            burn: dir.join(format!("{session}.burn.ass")),
            hls_dir: hls,
        }
    }

    pub fn paths_for(&self, session: &str, s: &TranscodeSettings) -> Paths {
        match relocate_root(s) {
            Some(root) => relocated_paths(&root, session),
            None => self.default_paths(session),
        }
    }

    fn paths_for_entry(&self, e: &CacheEntry) -> Paths {
        if e.artifact_root.is_empty() {
            self.default_paths(&e.session)
        } else {
            relocated_paths(Path::new(&e.artifact_root), &e.session)
        }
    }

    fn meta_path(&self, session: &str) -> PathBuf {
        self.inner.dir.join(format!("{session}.json"))
    }

    // ---- probing --------------------------------------------------------

    fn caps_lock(&self) -> MutexGuard<'_, CapsCache> {
        self.inner.caps.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The probed ffmpeg surface for these settings, kept per ffmpeg
    /// binary and VA-API device. A caller with a set for its key never
    /// waits for a probe: one older than [`CAP_FRESH`] comes back at once
    /// while a background probe replaces it. Only a key with no set yet
    /// (the first boot before the warm-up lands, a new ffmpeg path)
    /// waits, and then for the one probe in flight.
    pub fn capabilities(&self, s: &TranscodeSettings) -> Capabilities {
        let key = caps_key(s);
        let mut g = self.caps_lock();
        loop {
            if g.claim(&key, false) {
                let cached = g.current(&key);
                drop(g);
                self.launch_probe(&key, s);
                if let Some(caps) = cached {
                    return caps;
                }
                g = self.caps_lock();
                continue;
            }
            if let Some(caps) = g.current(&key) {
                return caps;
            }
            g = self.inner.caps_cv.wait(g).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Starts a probe for these settings in the background when there is
    /// no current set and none is running, and returns at once: the
    /// server warms the cache when it starts.
    pub fn warm(&self, s: &TranscodeSettings) {
        self.start_probe(s, false);
    }

    /// Starts a fresh probe for these settings in the background even when
    /// the set is current. One already running may have started before
    /// what prompted this call, so another runs once it lands. Callers
    /// keep the last set meanwhile.
    pub fn reprobe(&self, s: &TranscodeSettings) {
        self.start_probe(s, true);
    }

    fn start_probe(&self, s: &TranscodeSettings, force: bool) {
        let key = caps_key(s);
        let start = self.caps_lock().claim(&key, force);
        if start {
            self.launch_probe(&key, s);
        }
    }

    /// Runs the probe claimed for `key` on a thread of its own, so no
    /// caller pays for more than waiting on its answer; on this thread
    /// only when that one cannot start.
    fn launch_probe(&self, key: &str, s: &TranscodeSettings) {
        let (t, k, settings) = (self.clone(), key.to_string(), s.clone());
        let spawned =
            std::thread::Builder::new().name("transcode-caps".into()).spawn(move || t.run_probe(&k, &settings));
        if let Err(e) = spawned {
            tracing::warn!(err = %e, "capability probe thread not started; probing inline");
            self.run_probe(key, s);
        }
    }

    /// Runs the probe claimed for `key`, and again while a forced probe
    /// was asked for during the last one.
    fn run_probe(&self, key: &str, s: &TranscodeSettings) {
        while self.probe_once(key, s) {}
    }

    /// Probes and hands the answer to every waiter; true when the key
    /// stays claimed for one more probe. A probe that panics answers what
    /// an ffmpeg that does not run would, in doubt, so no waiter is left
    /// behind and the next caller after [`CAP_RECHECK`] probes again.
    fn probe_once(&self, key: &str, s: &TranscodeSettings) -> bool {
        let began = Instant::now();
        let limit = self.caps_lock().limit;
        let probed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match &self.inner.hooks.caps {
            Some(f) => (f(s), false),
            None => {
                let p = policy::probe_build_within(s, limit);
                (p.caps, p.timed_out)
            }
        }));
        let (caps, timed_out) = probed.unwrap_or_else(|_| {
            tracing::error!(ffmpeg = %policy::ffmpeg_binary(s), "transcode capability probe panicked");
            let caps = Capabilities {
                ffmpeg: policy::ffmpeg_binary(s),
                hardware: st::HW_BACKENDS.iter().map(|b| (b.to_string(), false)).collect(),
                ..Capabilities::default()
            };
            (caps, true)
        });
        let ms = began.elapsed().as_millis() as u64;
        let ready: Vec<&str> = caps.hardware.iter().filter(|(_, ok)| **ok).map(|(b, _)| b.as_str()).collect();
        let (ffmpeg, hardware, tone, effects) = (caps.ffmpeg.clone(), ready.join(","), caps.tone_map, caps.effects);
        let stored = self.caps_lock().store(key, caps, timed_out);
        self.inner.caps_cv.notify_all();
        // What the probe found is visible (D-031): a first or a changed
        // answer is logged, an unchanged refresh only at debug.
        if stored.doubtful {
            tracing::info!(ffmpeg = %ffmpeg, ms, hardware = %hardware, tone_mapping = tone, video_effects = effects, timed_out, "transcode capabilities in doubt (a check timed out or lost what the last probe found); probing again soon");
        } else if stored.changed {
            tracing::info!(ffmpeg = %ffmpeg, ms, hardware = %hardware, tone_mapping = tone, video_effects = effects, "transcode capabilities probed");
        } else {
            tracing::debug!(ffmpeg = %ffmpeg, ms, "transcode capabilities unchanged");
        }
        stored.again
    }

    /// The settings UI's report (D-031: probing is visible).
    pub fn probe(&self, s: &TranscodeSettings) -> lain_api::TranscodeCapabilities {
        policy::report(&self.capabilities(&st::normalize(s)))
    }

    fn probe_report(&self, s: &TranscodeSettings, path: &str) -> Result<RawReport, String> {
        match &self.inner.hooks.probe {
            Some(f) => f(Path::new(path)),
            None => lain_probe::probe(lain_probe::binary(&s.ffprobe_path), Path::new(path), lain_probe::TIMEOUT)
                .map_err(|e| e.to_string()),
        }
    }

    // ---- the v3 contract ----------------------------------------------

    fn spec_v3(&self, req: &V3Request) -> Result<(SourceSpec, TranscodeSettings), Error> {
        let Some((size, mtime)) = file_identity(&req.file_path) else {
            return Err(unavailable("source file unavailable"));
        };
        let settings = st::normalize(&req.settings);
        st::validate(&settings).map_err(Error::invalid)?;
        ensure_relocated_root(&settings).map_err(|e| unavailable(&e))?;
        if req.start_sec < 0.0 || req.start_sec.is_nan() {
            return Err(Error::invalid("start_sec must not be negative"));
        }
        let mut spec = SourceSpec {
            path: req.file_path.clone(),
            size,
            mtime_ns: mtime,
            profile: st::profile_key(&settings, req),
            audio_stream: req.audio_stream,
            subtitle_stream: req.subtitle_stream,
            start_sec: req.start_sec,
            session: String::new(),
            settings: settings.clone(),
        };
        spec.session = session_key(&spec);
        Ok((spec, settings))
    }

    /// Routes one v3 call by its action.
    pub fn invoke_v3(&self, req: &V3Request) -> Result<V3Status, Error> {
        let need_session = || {
            if req.session.is_empty() { Err(Error::invalid("session required")) } else { Ok(()) }
        };
        match req.action.as_str() {
            ACTION_SUBTITLES => return self.subtitles_v3(req),
            ACTION_CANCEL => {
                need_session()?;
                return Ok(self.cancel_v3(&req.session, &req.file_path, &req.user_id));
            }
            ACTION_POSITION => {
                need_session()?;
                return self.position_v3(&req.session, &req.file_path, req.segment_index);
            }
            ACTION_STATUS | ACTION_RESOLVE => {
                need_session()?;
                return self.by_session_v3(&req.file_path, &req.session, req.action == ACTION_RESOLVE);
            }
            _ => {}
        }
        if req.file_path.is_empty() {
            return Err(Error::invalid("file_path required"));
        }
        let (spec, settings) = self.spec_v3(req)?;
        if !req.session.is_empty() && req.session != spec.session {
            return Err(Error::invalid("session does not match source, profile and options"));
        }
        match req.action.as_str() {
            ACTION_INSPECT => Ok(self.inspect_v3(&spec, &settings)),
            ACTION_START => self.start_v3(&spec, req, &settings),
            _ => Err(Error::invalid("action must be inspect, start, status, resolve, position, cancel or list")),
        }
    }

    /// Every tracked session (admin view), oldest first.
    pub fn list_v3(&self) -> Vec<V3Status> {
        let g = self.lock();
        let mut out: Vec<V3Status> = g.jobs.values().filter(|j| j.v3).map(|j| self.status_from_job(j)).collect();
        out.sort_by(|a, b| (a.queued_at, &a.session).cmp(&(b.queued_at, &b.session)));
        out
    }

    fn inspect_v3(&self, spec: &SourceSpec, settings: &TranscodeSettings) -> V3Status {
        if let Some(e) = self.ready_entry(spec) {
            return self.status_from_entry(&e);
        }
        if let Some(j) = self.lock().jobs.get(&spec.session) {
            return self.status_from_job(j);
        }
        V3Status { delivery: settings.default_delivery.clone(), ..V3Status::idle(&spec.session, &spec.profile) }
    }

    fn by_session_v3(&self, path: &str, session: &str, touch: bool) -> Result<V3Status, Error> {
        {
            let mut g = self.lock();
            let now = self.now();
            if let Some(j) = g.jobs.get_mut(session) {
                if !path.is_empty() && j.spec.path != path {
                    return Err(Error::invalid("session does not match source"));
                }
                // Only a media fetch counts as watching; a status poll
                // must never keep an abandoned session alive.
                if touch {
                    j.last_touch = now;
                }
                return Ok(self.status_from_job(j));
            }
        }
        let not_found = || Error::not_found("unknown transcode session");
        if !valid_session_id(session) {
            return Err(not_found());
        }
        let e = read_entry(&self.meta_path(session)).ok_or_else(not_found)?;
        if !path.is_empty() && e.source_path != path {
            return Err(Error::invalid("session does not match source"));
        }
        if touch {
            self.touch(&e);
        }
        Ok(self.status_from_entry(&e))
    }

    /// Records the segment the client fetched last; the latest report
    /// wins (after a rewind the client is genuinely behind).
    fn position_v3(&self, session: &str, path: &str, segment: i64) -> Result<V3Status, Error> {
        let status = {
            let mut g = self.lock();
            let now = self.now();
            let Some(j) = g.jobs.get_mut(session) else {
                return Err(Error::not_found("unknown transcode session"));
            };
            if !path.is_empty() && j.spec.path != path {
                return Err(Error::invalid("session does not match source"));
            }
            if segment >= 0 {
                j.client_segment = segment;
            }
            j.last_touch = now;
            self.status_from_job(j)
        };
        self.apply_throttle(session);
        self.delete_consumed_segments(session);
        Ok(status)
    }

    /// Stops a session and drops its artifacts. Only its owner (or an
    /// admin cancel, which names no user) may.
    fn cancel_v3(&self, session: &str, path: &str, user_id: &str) -> V3Status {
        let (handle, paths, status) = {
            let mut g = self.lock();
            let Some(j) = g.jobs.get(session) else {
                drop(g);
                return self.cancel_finished_v3(session, path);
            };
            let refuse = (!path.is_empty() && j.spec.path != path)
                || (!user_id.is_empty() && !j.user_id.is_empty() && j.user_id != user_id)
                || !j.v3;
            if refuse {
                return V3Status::idle(session, "");
            }
            let mut j = g.jobs.remove(session).unwrap_or_else(|| unreachable!("checked above"));
            self.inner.cv.notify_all();
            let paths = self.paths_for(session, &j.settings);
            let status = V3Status {
                user_id: j.user_id.clone(),
                item_id: j.item_id.clone(),
                ..V3Status::idle(session, &j.spec.profile)
            };
            (j.handle.take(), paths, status)
        };
        if let Some(h) = handle {
            h.resume();
            h.kill();
        }
        let _ = std::fs::remove_file(&paths.media);
        let _ = std::fs::remove_dir_all(&paths.hls_dir);
        let _ = std::fs::remove_file(&paths.subtitle);
        let _ = std::fs::remove_file(&paths.burn);
        let _ = std::fs::remove_file(self.meta_path(session));
        tracing::info!(session = %session, "transcode cancelled");
        self.changed(&status);
        V3Status { user_id: String::new(), item_id: String::new(), ..status }
    }

    fn cancel_finished_v3(&self, session: &str, path: &str) -> V3Status {
        if !valid_session_id(session) {
            return V3Status::idle(session, "");
        }
        let Some(e) = read_entry(&self.meta_path(session)) else { return V3Status::idle(session, "") };
        if !path.is_empty() && e.source_path != path {
            return V3Status::idle(session, "");
        }
        self.remove_artifacts(&e, &self.paths_for_entry(&e));
        tracing::info!(session = %session, "transcode cancelled");
        V3Status::idle(session, &e.profile)
    }

    /// Extracts one subtitle stream to a cached WebVTT sidecar without a
    /// transcode (direct play's "subtitle extraction on the fly").
    fn subtitles_v3(&self, req: &V3Request) -> Result<V3Status, Error> {
        if req.file_path.is_empty() {
            return Err(Error::invalid("file_path required"));
        }
        let Some(index) = req.subtitle_stream else { return Err(Error::invalid("subtitle_stream required")) };
        let settings = st::normalize(&req.settings);
        st::validate(&settings).map_err(Error::invalid)?;
        if !st::allows_subtitle_extraction(&settings) {
            return Err(policy::forbidden("on-the-fly subtitle extraction is disabled by server settings"));
        }
        ensure_relocated_root(&settings).map_err(|e| unavailable(&e))?;
        let (mut spec, _) = self.spec_v3(req)?;
        spec.profile = SUBTITLE_PROFILE.into();
        spec.subtitle_stream = Some(index);
        spec.session = session_key(&spec);
        if let Some(e) = self.ready_entry(&spec) {
            self.touch(&e);
            return Ok(self.status_from_entry(&e));
        }
        let report =
            self.probe_report(&settings, &spec.path).map_err(|_| unavailable("ffprobe could not read the source"))?;
        let Some(sel) = report.streams.iter().find(|s| s.index == index && s.codec_type == "subtitle").cloned() else {
            return Err(Error::invalid("subtitle_stream is not a subtitle track"));
        };
        if !text_subtitle_codec(&sel.codec_name) {
            return Err(unsupported("selected subtitle cannot be converted to WebVTT"));
        }
        let paths = self.paths_for(&spec.session, &settings);
        if let Some(parent) = paths.subtitle.parent() {
            lain_store::ensure_private_dir(parent).map_err(|_| unavailable("transcode cache unavailable"))?;
        }
        if let Err(e) = extract_subtitle(&spec, &sel, &paths.subtitle, "webvtt") {
            tracing::warn!(session = %spec.session, err = %e, "subtitle extraction failed");
            return Err(unavailable("subtitle extraction failed"));
        }
        let size = file_len(&paths.subtitle).filter(|n| *n > 0).ok_or_else(|| unavailable("no WebVTT produced"))?;
        let now = self.now();
        let e = CacheEntry {
            session: spec.session.clone(),
            source_path: spec.path.clone(),
            source_size: spec.size,
            source_mtime: spec.mtime_ns,
            profile: spec.profile.clone(),
            delivery: st::DELIVERY_SUBTITLE.into(),
            artifact_root: relocate_root(&settings).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
            subtitle_stream: Some(index),
            has_subtitle: true,
            method: "extract".into(),
            size,
            created_at: now,
            accessed_at: now,
            ..CacheEntry::default()
        };
        write_json_atomic(&self.meta_path(&spec.session), &e)
            .map_err(|_| unavailable("transcode cache unavailable"))?;
        let _ = self.cleanup(false, &self.active_sessions_with(&spec.session));
        tracing::info!(session = %spec.session, codec = %sel.codec_name, "subtitle extracted");
        Ok(self.status_from_entry(&e))
    }

    fn active_streams_for(g: &State, user: &str, except: &str) -> i64 {
        if user.is_empty() {
            return 0;
        }
        g.jobs.iter().filter(|(s, j)| s.as_str() != except && j.user_id == user && j.active()).count() as i64
    }

    fn start_v3(&self, spec: &SourceSpec, req: &V3Request, settings: &TranscodeSettings) -> Result<V3Status, Error> {
        if let Some(e) = self.ready_entry(spec) {
            // The cache is keyed by source and options, not account: a
            // restricted user must not be served another's derivative.
            policy_allows_method(req.policy.as_ref(), &e.method)?;
            self.touch(&e);
            return Ok(self.status_from_entry(&e));
        }
        self.cleanup(false, &self.active_sessions_with("")).map_err(|_| unavailable("transcode cache unavailable"))?;
        if let Some(root) = relocate_root(settings) {
            self.sweep_relocated_orphans(&root, &self.active_sessions_with(&spec.session));
        }
        let report =
            self.probe_report(settings, &spec.path).map_err(|_| unavailable("ffprobe could not read the source"))?;
        let caps = self.capabilities(settings);
        let plan = plan_v3(spec, req, &caps, &report)?;
        let mut g = self.lock();
        if g.closed {
            return Err(unavailable("transcoder stopped"));
        }
        g.ffmpeg = policy::ffmpeg_binary(settings);
        if let Some(old) = g.jobs.get(&spec.session) {
            if old.state != FAILED {
                policy_allows_method(req.policy.as_ref(), &old.method)?;
                return Ok(self.status_from_job(old));
            }
            g.jobs.remove(&spec.session);
        }
        // The request carries the operator's bounds: they apply now.
        g.max_cache_bytes = settings.cache_bytes;
        g.max_concurrent = settings.max_concurrent;
        g.queue_limit = settings.queue_size.min(MAX_QUEUE_CAPACITY);
        if g.pending >= g.queue_limit {
            return Err(Error::new("queue-full", "transcode queue is full"));
        }
        if let Some(p) = &req.policy
            && p.max_streams > 0
            && Self::active_streams_for(&g, &req.user_id, &spec.session) >= p.max_streams
        {
            return Err(Error::new("too-many-streams", "simultaneous stream limit reached for this account"));
        }
        let now = self.now();
        let epoch = g.next_gen;
        g.next_gen += 1;
        let mut j = Job::new(epoch, spec.clone(), now);
        j.v3 = true;
        j.delivery = plan.delivery.clone();
        j.settings = settings.clone();
        j.user_id = req.user_id.clone();
        j.item_id = req.item_id.clone();
        j.encoder = plan.encoder.clone();
        j.hw_backend = plan.hardware_label();
        j.fallback = plan.fallback.clone();
        j.method = plan.method.clone();
        j.plan = Some(plan);
        let status = self.status_from_job(&j);
        g.jobs.insert(spec.session.clone(), j);
        g.pending += 1;
        g.queue.push_back((spec.session.clone(), epoch));
        self.inner.cv.notify_all();
        Ok(status)
    }

    fn status_from_job(&self, j: &Job) -> V3Status {
        let plan = j.plan.as_ref();
        let mut s = V3Status {
            session: j.spec.session.clone(),
            state: j.state.clone(),
            profile: j.spec.profile.clone(),
            delivery: j.delivery.clone(),
            method: j.method.clone(),
            encoder: j.encoder.clone(),
            hardware: j.hw_backend.clone(),
            fallback: j.fallback.clone(),
            reasons: plan.map(|p| p.reasons.clone()).unwrap_or_default(),
            video_codec: plan.map(|p| p.video_codec.clone()).unwrap_or_default(),
            audio_codec: plan.map(|p| p.audio_codec.clone()).unwrap_or_default(),
            width: plan.map(|p| p.width).unwrap_or(0),
            height: plan.map(|p| p.height).unwrap_or(0),
            bitrate_kbps: plan.map(|p| p.max_bitrate_kbps).unwrap_or(0),
            video_direct: plan.is_some_and(|p| p.copy_video),
            audio_direct: plan.is_some_and(|p| p.copy_audio),
            error_code: j.err_code.clone(),
            error: j.err_msg.clone(),
            user_id: j.user_id.clone(),
            item_id: j.item_id.clone(),
            queued_at: j.queued_at,
            started_at: j.started_at,
            finished_at: j.finished_at,
            ..V3Status::default()
        };
        if j.active() {
            s.progress = j.progress;
            s.fps = j.fps;
            s.output_bitrate_kbps = j.output_kbps;
        }
        let paths = self.paths_for(&j.spec.session, &j.settings);
        if j.has_subtitle {
            s.subtitle_path = paths.subtitle.to_string_lossy().into_owned();
        }
        if j.delivery == st::DELIVERY_HLS {
            if j.state == RUNNING || j.state == READY {
                s.playlist_path = paths.playlist.to_string_lossy().into_owned();
                s.playable = j.playable || j.state == READY;
            }
        } else if j.state == READY {
            s.path = paths.media.to_string_lossy().into_owned();
            s.playable = true;
        }
        s
    }

    fn status_from_entry(&self, e: &CacheEntry) -> V3Status {
        let paths = self.paths_for_entry(e);
        let mut s = V3Status {
            session: e.session.clone(),
            state: READY.into(),
            profile: e.profile.clone(),
            delivery: e.delivery.clone(),
            method: e.method.clone(),
            cached: true,
            playable: true,
            encoder: e.encoder.clone(),
            hardware: e.hardware.clone(),
            fallback: e.fallback.clone(),
            reasons: e.reasons.clone(),
            video_codec: e.video_codec.clone(),
            audio_codec: e.audio_codec.clone(),
            width: e.width,
            height: e.height,
            bitrate_kbps: e.bitrate_kbps,
            video_direct: e.video_direct,
            audio_direct: e.audio_direct,
            finished_at: e.created_at,
            ..V3Status::default()
        };
        if e.has_subtitle {
            s.subtitle_path = paths.subtitle.to_string_lossy().into_owned();
        }
        match e.delivery.as_str() {
            st::DELIVERY_SUBTITLE => {
                s.playable = false;
                s.method = "extract".into();
            }
            st::DELIVERY_HLS => s.playlist_path = paths.playlist.to_string_lossy().into_owned(),
            _ => s.path = paths.media.to_string_lossy().into_owned(),
        }
        s
    }

    // ---- the cache ------------------------------------------------------

    fn ready_entry(&self, spec: &SourceSpec) -> Option<CacheEntry> {
        let mut e = read_entry(&self.meta_path(&spec.session))?;
        if e.session != spec.session
            || e.source_path != spec.path
            || e.source_size != spec.size
            || e.source_mtime != spec.mtime_ns
            || e.profile != spec.profile
        {
            return None;
        }
        let paths = self.paths_for_entry(&e);
        let sub = || file_len(&paths.subtitle).filter(|n| *n > 0);
        e.size = match e.delivery.as_str() {
            st::DELIVERY_SUBTITLE => return sub().map(|n| CacheEntry { size: n, ..e }),
            st::DELIVERY_HLS => {
                if !hls::playable(&paths.hls_dir) {
                    return None;
                }
                hls::dir_size(&paths.hls_dir)
            }
            _ => file_len(&paths.media).filter(|n| *n > 0)?,
        };
        if e.has_subtitle {
            e.size += sub()?;
        }
        Some(e)
    }

    fn touch(&self, e: &CacheEntry) {
        let e = CacheEntry { accessed_at: self.now(), ..e.clone() };
        let _ = write_json_atomic(&self.meta_path(&e.session), &e);
    }

    fn active_sessions_with(&self, extra: &str) -> std::collections::HashSet<String> {
        let g = self.lock();
        let mut out: std::collections::HashSet<String> =
            g.jobs.iter().filter(|(_, j)| j.active()).map(|(s, _)| s.clone()).collect();
        if !extra.is_empty() {
            out.insert(extra.to_string());
        }
        out
    }

    fn remove_artifacts(&self, e: &CacheEntry, p: &Paths) {
        let _ = std::fs::remove_file(&p.media);
        let _ = std::fs::remove_file(p.media.with_extension("mp4.tmp"));
        let _ = std::fs::remove_file(&p.subtitle);
        let _ = std::fs::remove_file(&p.burn);
        let _ = std::fs::remove_dir_all(&p.hls_dir);
        let _ = std::fs::remove_file(self.meta_path(&e.session));
    }

    /// Removes stale and orphaned derivatives, then evicts the least
    /// recently used ready entries until the budget holds. A session
    /// fetched within the last minute is never the victim.
    fn cleanup(&self, startup: bool, exclude: &std::collections::HashSet<String>) -> std::io::Result<()> {
        let dir = &self.inner.dir;
        let entries: Vec<_> = std::fs::read_dir(dir)?.flatten().collect();
        let budget = self.lock().max_cache_bytes;
        let mut meta = std::collections::HashSet::new();
        let mut ready = Vec::new();
        let mut total = 0i64;
        for de in &entries {
            let name = de.file_name().to_string_lossy().into_owned();
            let path = de.path();
            if !name.ends_with(".json") {
                if startup && name.ends_with(".tmp") {
                    let _ = std::fs::remove_file(&path);
                }
                continue;
            }
            let Some(mut e) = read_entry(&path).filter(|e| !e.session.is_empty()) else {
                let _ = std::fs::remove_file(&path);
                continue;
            };
            meta.insert(e.session.clone());
            let paths = self.paths_for_entry(&e);
            let sub_len = file_len(&paths.subtitle).filter(|n| *n > 0);
            let media_len = file_len(&paths.media).filter(|n| *n > 0);
            let source_ok = file_identity(&e.source_path) == Some((e.source_size, e.source_mtime));
            let mut stale = match e.delivery.as_str() {
                st::DELIVERY_SUBTITLE => sub_len.is_none(),
                st::DELIVERY_HLS => !hls::playable(&paths.hls_dir),
                _ => media_len.is_none(),
            } || !source_ok;
            if e.has_subtitle && e.delivery != st::DELIVERY_SUBTITLE && sub_len.is_none() {
                stale = true;
            }
            if stale {
                if !exclude.contains(&e.session) {
                    self.remove_artifacts(&e, &paths);
                }
                continue;
            }
            e.size = match e.delivery.as_str() {
                st::DELIVERY_SUBTITLE => sub_len.unwrap_or(0),
                st::DELIVERY_HLS => hls::dir_size(&paths.hls_dir),
                _ => media_len.unwrap_or(0),
            };
            if e.has_subtitle && e.delivery != st::DELIVERY_SUBTITLE {
                e.size += sub_len.unwrap_or(0);
            }
            total += e.size;
            ready.push(e);
        }
        // Artifacts with no record (a crash between rename and record).
        for de in &entries {
            let name = de.file_name().to_string_lossy().into_owned();
            let id = name.strip_suffix(".mp4").or_else(|| name.strip_suffix(".hls"));
            if let Some(id) = id
                && !meta.contains(id)
                && !exclude.contains(id)
            {
                if name.ends_with(".hls") {
                    if de.path().is_dir() {
                        let _ = std::fs::remove_dir_all(de.path());
                    }
                } else {
                    let _ = std::fs::remove_file(de.path());
                }
            }
        }
        ready.sort_by_key(|e| (e.accessed_at, e.created_at));
        let grace = self.now() - RECENT_ACCESS_GRACE;
        for e in &ready {
            if total <= budget {
                break;
            }
            if exclude.contains(&e.session) || e.accessed_at >= grace {
                continue;
            }
            self.remove_artifacts(e, &self.paths_for_entry(e));
            total -= e.size;
            tracing::debug!(session = %e.session, bytes = e.size, "transcode cache evict");
        }
        if total > budget {
            tracing::warn!(bytes = total, max_bytes = budget, "transcode cache over quota");
        }
        Ok(())
    }

    /// Removes session directories under a temp root that no record
    /// claims, only when this data dir owns the root (`.owner`).
    fn sweep_relocated_orphans(&self, root: &Path, exclude: &std::collections::HashSet<String>) {
        let owner = root.join(".owner");
        let me = self.inner.dir.to_string_lossy().into_owned();
        match std::fs::read_to_string(&owner) {
            Ok(got) => {
                let got = got.trim().to_string();
                if got != me {
                    if Path::new(&got).exists() {
                        tracing::warn!(root = %root.display(), owner = %got, "transcode temp path is owned by another data dir; skipping orphan sweep");
                        return;
                    }
                    let _ = std::fs::write(&owner, &me);
                }
            }
            Err(_) => match std::fs::OpenOptions::new().write(true).create_new(true).open(&owner) {
                Ok(mut f) => {
                    use std::io::Write;
                    let _ = f.write_all(me.as_bytes());
                }
                Err(_) => {
                    if std::fs::read_to_string(&owner).map(|s| s.trim() != me).unwrap_or(true) {
                        return;
                    }
                }
            },
        }
        let Ok(rd) = std::fs::read_dir(root) else { return };
        for e in rd.flatten() {
            if !e.path().is_dir() {
                continue;
            }
            let session = e.file_name().to_string_lossy().into_owned();
            if exclude.contains(&session) || self.meta_path(&session).exists() {
                continue;
            }
            let _ = std::fs::remove_dir_all(e.path());
        }
    }

    // ---- running --------------------------------------------------------

    /// Runs `f` on the job `(session, epoch)` if it is still that job.
    fn with_job<R>(&self, session: &str, epoch: u64, f: impl FnOnce(&mut Job) -> R) -> Option<R> {
        let mut g = self.lock();
        g.jobs.get_mut(session).filter(|j| j.epoch == epoch).map(f)
    }

    fn sample_sink(&self, session: &str, epoch: u64) -> Box<dyn FnMut(process::Sample) + Send> {
        let t = self.clone();
        let session = session.to_string();
        Box::new(move |s: process::Sample| {
            t.with_job(&session, epoch, |j| {
                if let Some(f) = s.fraction {
                    j.progress = f;
                }
                if s.fps > 0.0 {
                    j.fps = s.fps;
                }
                if s.bitrate_kbps > 0 {
                    j.output_kbps = s.bitrate_kbps;
                }
            });
        })
    }

    /// One ffmpeg invocation of a plan, killable through the job.
    fn exec_plan(&self, session: &str, epoch: u64, p: &EncodePlan, output: &Path, sub: &Path) -> Result<(), Fail> {
        let args = p.ffmpeg_args(output, sub, true);
        if !p.is_hls() {
            let _ = std::fs::remove_file(output);
        }
        let mut cmd = Command::new(policy::ffmpeg_binary(&p.settings));
        cmd.args(&args);
        let deadline = (!p.is_hls()).then_some(PROGRESSIVE_TIMEOUT);
        let t = self.clone();
        let res = process::run(
            cmd,
            p.duration_sec,
            deadline,
            |h| {
                let kept = t.with_job(session, epoch, |j| {
                    if j.stopped {
                        return false;
                    }
                    j.handle = Some(h.clone());
                    true
                });
                if kept != Some(true) {
                    h.kill();
                }
            },
            Some(self.sample_sink(session, epoch)),
            || {
                let g = t.lock();
                g.closed || g.jobs.get(session).is_none_or(|j| j.epoch != epoch || j.stopped)
            },
        );
        self.with_job(session, epoch, |j| {
            j.handle = None;
            j.paused = false;
        });
        res.map_err(|e| Fail::Plain(e.to_string()))?;
        if !p.is_hls() && file_len(output).is_none_or(|n| n == 0) {
            return Err(Fail::Plain("no output produced".into()));
        }
        Ok(())
    }

    fn exec_with_fallback(
        &self,
        session: &str,
        epoch: u64,
        p: &EncodePlan,
        output: &Path,
        sub: &Path,
    ) -> Result<(), Fail> {
        let first = self.exec_plan(session, epoch, p, output, sub);
        let Err(Fail::Plain(e)) = &first else { return first };
        if p.hw_backend.is_empty() && !p.hw_decode {
            return first;
        }
        if self.with_job(session, epoch, |j| j.stopped).unwrap_or(true) {
            return first;
        }
        tracing::warn!(session = %session, err = %e, "hardware transcode failed; retrying in software");
        let has_svt = self.capabilities(&p.settings).has_encoder("libsvtav1");
        let fb = p.software_fallback(has_svt);
        self.with_job(session, epoch, |j| {
            j.fallback = fb.fallback.clone();
            j.encoder = fb.encoder.clone();
            j.hw_backend.clear();
        });
        self.exec_plan(session, epoch, &fb, output, sub)
    }

    fn run_plan(&self, session: &str, epoch: u64, p: &EncodePlan, paths: &Paths) -> Result<(), Fail> {
        if self.with_job(session, epoch, |j| j.stopped).unwrap_or(true) {
            return Err(Fail::Plain("session stopped".into()));
        }
        // The effect's shader is a file ffmpeg reads; it lives beside the
        // sessions, one per preset.
        let with_effect;
        let p = if p.effect.is_empty() {
            p
        } else {
            let file = effects::materialize(&self.inner.dir, &p.effect)?;
            with_effect = EncodePlan { effect_file: file.to_string_lossy().into_owned(), ..p.clone() };
            &with_effect
        };
        let mut sub_file = PathBuf::new();
        if p.burn_text
            && let Some(sub) = &p.subtitle
        {
            sub_file = paths.burn.clone();
            ensure_parent(&sub_file)?;
            extract_subtitle(&p.spec, sub, &sub_file, "ass")?;
        }
        // The sidecar comes before the encode, so HLS can offer it from
        // the first segment.
        if p.subtitle_mode == st::SUB_EXTRACT
            && let Some(sub) = &p.subtitle
        {
            ensure_parent(&paths.subtitle)?;
            extract_subtitle(&p.spec, sub, &paths.subtitle, "webvtt")?;
            self.with_job(session, epoch, |j| j.has_subtitle = true);
        }
        if p.is_hls() {
            lain_store::ensure_private_dir(&paths.hls_dir).map_err(|e| e.to_string())?;
            let raw = paths.hls_dir.join(hls::RAW_PLAYLIST);
            self.exec_with_fallback(session, epoch, p, &raw, &sub_file)?;
            hls::write_index(&paths.hls_dir, p.settings.hls_segment_seconds).map_err(|e| e.to_string())?;
            let ok = hls::playable(&paths.hls_dir);
            self.with_job(session, epoch, |j| j.playable = ok);
            return Ok(());
        }
        ensure_parent(&paths.media)?;
        let tmp = paths.media.with_extension("mp4.tmp");
        let res = self.exec_with_fallback(session, epoch, p, &tmp, &sub_file);
        if res.is_err() {
            let _ = std::fs::remove_file(&tmp);
            return res;
        }
        std::fs::rename(&tmp, &paths.media).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            Fail::Plain(e.to_string())
        })
    }

    fn finalize_v3(&self, session: &str, epoch: u64, p: &EncodePlan, paths: &Paths) -> Result<(), Fail> {
        if p.subtitle_mode == st::SUB_EXTRACT
            && let Some(sub) = &p.subtitle
            && !self.with_job(session, epoch, |j| j.has_subtitle).unwrap_or(true)
        {
            ensure_parent(&paths.subtitle)?;
            extract_subtitle(&p.spec, sub, &paths.subtitle, "webvtt")?;
            self.with_job(session, epoch, |j| j.has_subtitle = true);
        }
        if p.burn_text {
            let _ = std::fs::remove_file(&paths.burn);
        }
        self.record_v3(session, epoch, p, paths)
    }

    fn record_v3(&self, session: &str, epoch: u64, p: &EncodePlan, paths: &Paths) -> Result<(), Fail> {
        let Some((has_sub, encoder, hw, fallback)) = self.with_job(session, epoch, |j| {
            (j.has_subtitle, j.encoder.clone(), j.hw_backend.clone(), j.fallback.clone())
        }) else {
            return Err(Fail::Plain("session cancelled".into()));
        };
        let mut size = if p.is_hls() {
            if !hls::playable(&paths.hls_dir) {
                let mut names: Vec<String> = std::fs::read_dir(&paths.hls_dir)
                    .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
                    .unwrap_or_default();
                names.sort();
                return Err(Fail::Plain(format!("no playable HLS output produced (files: {})", names.join(", "))));
            }
            hls::dir_size(&paths.hls_dir)
        } else {
            file_len(&paths.media).filter(|n| *n > 0).ok_or_else(|| Fail::Plain("no mp4 produced".into()))?
        };
        if has_sub {
            size +=
                file_len(&paths.subtitle).filter(|n| *n > 0).ok_or_else(|| Fail::Plain("no WebVTT produced".into()))?;
        }
        let now = self.now();
        let e = CacheEntry {
            session: session.to_string(),
            source_path: p.spec.path.clone(),
            source_size: p.spec.size,
            source_mtime: p.spec.mtime_ns,
            profile: p.spec.profile.clone(),
            delivery: p.delivery.clone(),
            artifact_root: relocate_root(&p.settings).map(|r| r.to_string_lossy().into_owned()).unwrap_or_default(),
            audio_stream: p.spec.audio_stream,
            subtitle_stream: p.spec.subtitle_stream,
            has_subtitle: has_sub,
            method: p.method.clone(),
            size,
            created_at: now,
            accessed_at: now,
            encoder,
            hardware: hw,
            fallback,
            reasons: p.reasons.clone(),
            video_codec: p.video_codec.clone(),
            audio_codec: p.audio_codec.clone(),
            width: p.width,
            height: p.height,
            bitrate_kbps: p.max_bitrate_kbps,
            video_direct: p.copy_video,
            audio_direct: p.copy_audio,
        };
        write_json_atomic(&self.meta_path(session), &e).map_err(|e| Fail::Plain(e.to_string()))
    }

    fn remove_v3_artifacts(&self, delivery: &str, paths: &Paths) {
        if delivery == st::DELIVERY_HLS {
            let _ = std::fs::remove_dir_all(&paths.hls_dir);
            return;
        }
        let _ = std::fs::remove_file(&paths.media);
        let _ = std::fs::remove_file(paths.media.with_extension("mp4.tmp"));
        let _ = std::fs::remove_file(&paths.subtitle);
        let _ = std::fs::remove_file(&paths.burn);
    }

    fn run_v3(&self, session: &str, epoch: u64) {
        let started = {
            let mut g = self.lock();
            if g.closed {
                return;
            }
            let now = self.now();
            match g.jobs.get_mut(session).filter(|j| j.epoch == epoch) {
                Some(j) if j.state == QUEUED => {
                    j.state = RUNNING.into();
                    j.started_at = now;
                    j.last_touch = now;
                    j.plan.clone().map(|p| (p, j.delivery.clone(), j.settings.clone()))
                }
                _ => None,
            }
        };
        let Some((plan, delivery, settings)) = started else {
            self.inner.cv.notify_all();
            return;
        };
        tracing::info!(session = %session, profile = %plan.spec.profile, delivery = %delivery, "transcode running");
        let paths = self.paths_for(session, &settings);
        let res = match &self.inner.hooks.run {
            Some(f) => f(&RunCtx { session: session.into(), delivery: delivery.clone(), paths: paths.clone() })
                .map_err(Fail::Plain)
                .map(|_| {
                    if delivery == st::DELIVERY_HLS {
                        let _ = hls::write_index(&paths.hls_dir, settings.hls_segment_seconds);
                    }
                }),
            None => self.run_plan(session, epoch, &plan, &paths),
        };
        let res = res.and_then(|_| self.finalize_v3(session, epoch, &plan, &paths));
        let (ok, status, detail) = {
            let mut g = self.lock();
            let now = self.now();
            let mut out = (false, None, String::new());
            if let Some(j) = g.jobs.get_mut(session).filter(|j| j.epoch == epoch) {
                j.finished_at = now;
                let res = match res {
                    Ok(()) if j.stopped => Err(Fail::Plain("session stopped".into())),
                    other => other,
                };
                match res {
                    Ok(()) => {
                        j.state = READY.into();
                        j.playable = true;
                        out.0 = true;
                    }
                    Err(f) => {
                        self.remove_v3_artifacts(&j.delivery, &paths);
                        j.state = FAILED.into();
                        if j.err_code.is_empty() {
                            j.err_code = "dependency-unavailable".into();
                            j.err_msg = "transcode failed".into();
                        }
                        match f {
                            Fail::Core(e) => {
                                out.2 = e.to_string();
                                j.err_code = e.code;
                                j.err_msg = e.msg;
                            }
                            Fail::Plain(e) => out.2 = e,
                        }
                    }
                }
                out.1 = Some(self.status_from_job(j));
            }
            self.inner.cv.notify_all();
            out
        };
        // A missing job was cancelled: its artifacts are already gone.
        let Some(status) = status else { return };
        if ok {
            let _ = self.cleanup(false, &self.active_sessions_with(session));
            tracing::info!(session = %session, method = %status.method, "transcode ready");
        } else {
            let detail = detail.replace(&plan.spec.path, "<media>");
            tracing::warn!(session = %session, err = %detail, "transcode failed");
        }
        self.changed(&status);
    }

    // ---- the v1 contract ----------------------------------------------

    /// Prepares a complete MP4 (`mp4-compat-v1`) and waits for it.
    pub fn prepare_sync(&self, path: &str) -> Result<LegacyResult, Error> {
        if path.is_empty() {
            return Err(Error::invalid("file_path required"));
        }
        let Some((size, mtime)) = file_identity(path) else { return Err(unavailable("source file unavailable")) };
        let mut spec = SourceSpec {
            path: path.into(),
            size,
            mtime_ns: mtime,
            profile: LEGACY_PROFILE.into(),
            ..SourceSpec::default()
        };
        spec.session = session_key(&spec);
        if let Some(e) = self.ready_entry(&spec) {
            self.touch(&e);
            return Ok(LegacyResult { path: self.status_from_entry(&e).path, method: e.method, cached: true });
        }
        self.cleanup(false, &self.active_sessions_with("")).map_err(|_| unavailable("transcode cache unavailable"))?;
        let epoch = {
            let mut g = self.lock();
            if g.closed {
                return Err(unavailable("transcoder stopped"));
            }
            match g.jobs.get(&spec.session) {
                Some(j) if j.state != FAILED => j.epoch,
                _ => {
                    g.jobs.remove(&spec.session);
                    if g.pending >= g.queue_limit {
                        return Err(Error::new("queue-full", "transcode queue is full"));
                    }
                    let epoch = g.next_gen;
                    g.next_gen += 1;
                    let now = self.now();
                    g.jobs.insert(spec.session.clone(), Job::new(epoch, spec.clone(), now));
                    g.pending += 1;
                    g.queue.push_back((spec.session.clone(), epoch));
                    self.inner.cv.notify_all();
                    epoch
                }
            }
        };
        let mut g = self.lock();
        loop {
            if g.closed {
                return Err(unavailable("transcoder stopped"));
            }
            match g.jobs.get(&spec.session).filter(|j| j.epoch == epoch) {
                Some(j) if j.state == READY => {
                    let p = self.default_paths(&spec.session).media.to_string_lossy().into_owned();
                    return Ok(LegacyResult { path: p, method: j.method.clone(), cached: false });
                }
                Some(j) if j.state == FAILED => return Err(unavailable(&j.err_msg)),
                Some(_) => {}
                None => return Err(unavailable("transcode job unavailable")),
            }
            g = self.inner.cv.wait_timeout(g, Duration::from_millis(500)).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    fn run_legacy(&self, session: &str, epoch: u64) {
        let spec = {
            let mut g = self.lock();
            if g.closed {
                return;
            }
            let now = self.now();
            match g.jobs.get_mut(session).filter(|j| j.epoch == epoch) {
                Some(j) if j.state == QUEUED => {
                    j.state = RUNNING.into();
                    j.started_at = now;
                    Some(j.spec.clone())
                }
                _ => None,
            }
        };
        let Some(spec) = spec else {
            self.inner.cv.notify_all();
            return;
        };
        let paths = self.default_paths(session);
        let res: Result<String, String> = match &self.inner.hooks.run {
            Some(f) => {
                f(&RunCtx { session: session.into(), delivery: st::DELIVERY_PROGRESSIVE.into(), paths: paths.clone() })
                    .map(|_| "transcode".to_string())
            }
            None => self.convert_legacy(session, epoch, &spec, &paths.media),
        };
        let res = res.and_then(|method| {
            let len = file_len(&paths.media).filter(|n| *n > 0).ok_or("no mp4 produced")?;
            let now = self.now();
            let e = CacheEntry {
                session: session.into(),
                source_path: spec.path.clone(),
                source_size: spec.size,
                source_mtime: spec.mtime_ns,
                profile: spec.profile.clone(),
                method: method.clone(),
                size: len,
                created_at: now,
                accessed_at: now,
                ..CacheEntry::default()
            };
            write_json_atomic(&self.meta_path(session), &e).map_err(|e| e.to_string())?;
            let _ = self.cleanup(false, &self.active_sessions_with(session));
            Ok(method)
        });
        let mut g = self.lock();
        let now = self.now();
        if let Some(j) = g.jobs.get_mut(session).filter(|j| j.epoch == epoch) {
            j.finished_at = now;
            match res {
                Ok(m) => {
                    j.state = READY.into();
                    j.method = m;
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&paths.media);
                    let _ = std::fs::remove_file(paths.media.with_extension("mp4.tmp"));
                    j.state = FAILED.into();
                    j.err_code = "dependency-unavailable".into();
                    j.err_msg = "transcode failed".into();
                    tracing::warn!(session = %session, err = %e.replace(&spec.path, "<media>"), "transcode failed");
                }
            }
        }
        self.inner.cv.notify_all();
    }

    fn convert_legacy(&self, session: &str, epoch: u64, spec: &SourceSpec, out: &Path) -> Result<String, String> {
        let report = self.probe_report(&TranscodeSettings::default(), &spec.path).ok();
        let tmp = out.with_extension("mp4.tmp");
        let _ = std::fs::remove_file(&tmp);
        let (args, method) = encode::legacy_args(&spec.path, report.as_ref(), &tmp, true);
        let mut cmd = Command::new("ffmpeg");
        cmd.args(&args);
        let total = report.as_ref().map(|r| r.duration_sec()).unwrap_or(0.0);
        let t = self.clone();
        let res = process::run(
            cmd,
            total,
            Some(PROGRESSIVE_TIMEOUT),
            |_| {},
            Some(self.sample_sink(session, epoch)),
            || t.lock().closed,
        );
        if let Err(e) = res {
            let _ = std::fs::remove_file(&tmp);
            return Err(e.to_string());
        }
        if file_len(&tmp).is_none_or(|n| n == 0) {
            let _ = std::fs::remove_file(&tmp);
            return Err("no mp4 produced".into());
        }
        std::fs::rename(&tmp, out).map_err(|e| e.to_string())?;
        Ok(method)
    }

    // ---- the ticker -----------------------------------------------------

    fn tick(&self) {
        let now = self.now();
        self.prune_jobs();
        let hls: Vec<(String, u64, String, i64, i64, TranscodeSettings)> = {
            let g = self.lock();
            g.jobs
                .iter()
                .filter(|(_, j)| j.v3 && j.delivery == st::DELIVERY_HLS)
                .map(|(s, j)| {
                    (s.clone(), j.epoch, j.state.clone(), j.last_touch, j.settings.idle_timeout_sec, j.settings.clone())
                })
                .collect()
        };
        for (session, epoch, state, last_touch, idle, settings) in hls {
            let dir = self.paths_for(&session, &settings).hls_dir;
            match state.as_str() {
                RUNNING => {
                    // The idle check runs even before ffmpeg wrote a
                    // playlist: a stuck encoder must still time out.
                    if hls::write_index(&dir, settings.hls_segment_seconds).is_ok() {
                        let ok = hls::playable(&dir);
                        self.with_job(&session, epoch, |j| j.playable = ok);
                        self.apply_throttle(&session);
                        self.delete_consumed_segments(&session);
                    }
                    if idle > 0 && last_touch > 0 && now - last_touch > idle {
                        self.stop_job(&session, epoch, "idle-timeout", "session stopped after inactivity");
                    }
                }
                READY => {
                    let _ = hls::write_index(&dir, settings.hls_segment_seconds);
                }
                _ => {}
            }
        }
    }

    /// Bounds the session map: finished sessions go first, oldest first
    /// (the record on disk keeps a later play instant).
    fn prune_jobs(&self) {
        let mut g = self.lock();
        if g.jobs.len() <= MAX_TRACKED_JOBS {
            return;
        }
        let mut done: Vec<(i64, String)> = g
            .jobs
            .iter()
            .filter(|(_, j)| j.state == READY || j.state == FAILED)
            .map(|(s, j)| (j.finished_at, s.clone()))
            .collect();
        done.sort();
        for (_, s) in done {
            if g.jobs.len() <= MAX_TRACKED_JOBS {
                break;
            }
            g.jobs.remove(&s);
        }
    }

    /// Kills a running session and marks it failed with `code`; a stop
    /// is terminal, a late finish never resurrects it.
    fn stop_job(&self, session: &str, epoch: u64, code: &str, msg: &str) {
        let now = self.now();
        let Some((h, status)) = self
            .with_job(session, epoch, |j| {
                if !j.active() {
                    return None;
                }
                j.stopped = true;
                j.state = FAILED.into();
                j.err_code = code.into();
                j.err_msg = msg.into();
                j.finished_at = now;
                j.paused = false;
                Some((j.handle.clone(), self.status_from_job(j)))
            })
            .flatten()
        else {
            return;
        };
        if let Some(h) = h {
            h.resume();
            h.kill();
        }
        tracing::info!(session = %session, reason = %code, "transcode stopped");
        self.changed(&status);
    }

    /// Pauses ffmpeg when it is far ahead of the client and resumes it
    /// once the client catches up (output never changes).
    fn apply_throttle(&self, session: &str) {
        let Some((epoch, h, paused, seg, settings)) = ({
            let g = self.lock();
            g.jobs.get(session).and_then(|j| {
                (j.settings.throttle && j.delivery == st::DELIVERY_HLS)
                    .then(|| (j.epoch, j.handle.clone(), j.paused, j.client_segment, j.settings.clone()))
            })
        }) else {
            return;
        };
        let Some(h) = h else { return };
        let dir = self.paths_for(session, &settings).hls_dir;
        let (_, produced) = hls::produced(&dir);
        let ahead = produced - hls::client_position_sec(&dir, seg);
        let limit = settings.throttle_ahead_sec as f64;
        if !paused && ahead > limit {
            if h.pause() {
                self.with_job(session, epoch, |j| j.paused = true);
                tracing::debug!(session = %session, ahead_sec = ahead as i64, "transcode throttled");
            }
        } else if paused && ahead < limit / 2.0 && h.resume() {
            self.with_job(session, epoch, |j| j.paused = false);
            tracing::debug!(session = %session, ahead_sec = ahead as i64, "transcode resumed");
        }
    }

    fn delete_consumed_segments(&self, session: &str) {
        let Some((seg, settings)) = ({
            let g = self.lock();
            g.jobs.get(session).and_then(|j| {
                (j.settings.segment_deletion && j.delivery == st::DELIVERY_HLS)
                    .then(|| (j.client_segment, j.settings.clone()))
            })
        }) else {
            return;
        };
        let dir = self.paths_for(session, &settings).hls_dir;
        hls::delete_consumed(&dir, seg, settings.segment_keep_sec, settings.hls_segment_seconds);
    }

    /// Whether `(session, epoch)` is paused (tests).
    pub fn is_paused(&self, session: &str) -> bool {
        self.lock().jobs.get(session).is_some_and(|j| j.paused)
    }
}

fn policy_allows_method(p: Option<&Policy>, method: &str) -> Result<(), Error> {
    let Some(p) = p else { return Ok(()) };
    match method {
        "extract" => Ok(()),
        "remux" if !p.allow_remux => Err(policy::forbidden("remuxing is disabled for this user")),
        "remux" => Ok(()),
        _ if !p.allow_video_transcode => Err(policy::forbidden("video transcoding is disabled for this user")),
        _ => Ok(()),
    }
}

fn ensure_parent(p: &Path) -> Result<(), Fail> {
    if let Some(d) = p.parent() {
        lain_store::ensure_private_dir(d).map_err(|e| Fail::Plain(e.to_string()))?;
    }
    Ok(())
}

/// Converts one subtitle stream to a standalone file (`webvtt` sidecar
/// or `ass` for burn-in), atomically.
fn extract_subtitle(spec: &SourceSpec, s: &RawStream, out: &Path, format: &str) -> Result<(), String> {
    let tmp = out.with_extension(format!("{}.tmp", out.extension().map(|e| e.to_string_lossy()).unwrap_or_default()));
    let _ = std::fs::remove_file(&tmp);
    let mut cmd = Command::new(policy::ffmpeg_binary(&spec.settings));
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i", &spec.path, "-map", &format!("0:{}", s.index)]);
    cmd.args(["-f", format, "-y"]).arg(&tmp);
    let out_ = exec::run(cmd, None, SUBTITLE_TIMEOUT).map_err(|e| e.to_string())?;
    if !out_.status.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err(out_.describe());
    }
    if file_len(&tmp).is_none_or(|n| n == 0) {
        let _ = std::fs::remove_file(&tmp);
        return Err("no subtitle file produced".into());
    }
    std::fs::rename(&tmp, out).map_err(|e| e.to_string())
}

impl From<Fail> for String {
    fn from(f: Fail) -> String {
        match f {
            Fail::Core(e) => e.to_string(),
            Fail::Plain(s) => s,
        }
    }
}

/// The dispatcher: admits queued jobs while a slot is free. It holds
/// the transcoder only briefly, so dropping every handle ends it.
fn dispatch(weak: Weak<Inner>) {
    loop {
        let Some(inner) = weak.upgrade() else { return };
        let t = Transcoder { inner };
        let next = {
            let mut g = t.lock();
            if g.closed {
                return;
            }
            if !g.queue.is_empty() && g.active < g.max_concurrent {
                let (s, epoch) = g.queue.pop_front().unwrap_or_default();
                g.pending = (g.pending - 1).max(0);
                let v3 = g.jobs.get(&s).filter(|j| j.epoch == epoch && j.state == QUEUED).map(|j| j.v3);
                if let Some(v3) = v3 {
                    g.active += 1;
                    Some((s, epoch, v3))
                } else {
                    None
                }
            } else {
                let _unused = t.inner.cv.wait_timeout(g, Duration::from_millis(300));
                None
            }
        };
        if let Some((s, epoch, v3)) = next {
            let runner = t.clone();
            let spawned = std::thread::Builder::new().name("transcode-run".into()).spawn(move || {
                if v3 {
                    runner.run_v3(&s, epoch);
                } else {
                    runner.run_legacy(&s, epoch);
                }
                let mut g = runner.lock();
                g.active = (g.active - 1).max(0);
                runner.inner.cv.notify_all();
            });
            if spawned.is_err() {
                let mut g = t.lock();
                g.active = (g.active - 1).max(0);
            }
        }
    }
}

/// Writes an executable stub for a test. A file some process holds open
/// for writing cannot be executed (ETXTBSY), and a child that another
/// test thread forks while this one writes inherits that descriptor
/// until it execs, so a stub written by this process can fail to start
/// on a busy machine (every list of a probe then comes back empty). A
/// `cat` child writes it instead: no descriptor of this process ever
/// points at the file.
#[cfg(all(test, unix))]
pub(crate) fn write_stub(path: &Path, script: &str) {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    let mut sh = std::process::Command::new("/bin/sh")
        .args(["-c", "cat > \"$1\"", "sh"])
        .arg(path)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    sh.stdin.take().unwrap().write_all(script.as_bytes()).unwrap();
    assert!(sh.wait().unwrap().success(), "stub {} not written", path.display());
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(test)]
mod tests;
