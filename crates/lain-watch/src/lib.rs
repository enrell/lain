//! `lain-watch` (D-190): the watch module's domain for the Rust server.
//! Playback plans ([`plan`]), the transcoder over external ffmpeg
//! ([`transcode`]), thumbnails ([`thumbnail`]), local playback
//! ([`localplay`]) and the operator's transcode settings ([`settings`]),
//! each also served as an in-process provider under the Go provider id
//! so compositions, swaps and module suspend work unchanged.
//!
//! Nothing here touches HTTP or the async runtime: calls block (they
//! run ffprobe or wait on the cache) and the server makes them from its
//! blocking pool; transcodes run on their own threads.

pub mod localplay;
pub mod plan;
pub mod settings;
pub mod thumbnail;
pub mod transcode;

use std::path::Path;
use std::sync::Arc;

use lain_core::{Binding, BindingMode, Error, Provider, Registry};
use lain_store::Store;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

pub use localplay::LocalPlay;
pub use settings::SettingsStore;
pub use thumbnail::Generator;
pub use transcode::Transcoder;

pub const CAP_PLAYBACK_PLAN: &str = "lain.playback.plan@1";
pub const CAP_MEDIA_PROBE: &str = "lain.media.probe@1";
pub const CAP_TRANSCODE_SETTINGS: &str = "lain.settings.transcode@1";

pub const PLANNER_ID: &str = "lain-playback-default";
pub const PROBE_ID: &str = "lain-probe-ffprobe";
pub const SETTINGS_ID: &str = "lain-settings-redb";

fn input<T: DeserializeOwned>(v: Value, what: &str) -> Result<T, Error> {
    serde_json::from_value(v).map_err(|_| Error::invalid(format!("{what} required")))
}

fn out<T: serde::Serialize>(v: &T) -> Result<Value, Error> {
    serde_json::to_value(v).map_err(|e| Error::internal(e.to_string()))
}

fn wrong_cap(cap: &str) -> Error {
    Error::invalid(format!("unsupported cap {cap}"))
}

/// `lain-playback-default`: the planner.
pub struct Planner;

impl Provider for Planner {
    fn id(&self) -> &str {
        PLANNER_ID
    }
    fn capabilities(&self) -> Vec<String> {
        vec![CAP_PLAYBACK_PLAN.into()]
    }
    fn health(&self) -> Result<(), Error> {
        Ok(())
    }
    fn invoke(&self, cap: &str, _: &str, v: Value) -> Result<Value, Error> {
        if cap != CAP_PLAYBACK_PLAN {
            return Err(wrong_cap(cap));
        }
        out(&plan::plan(&input(v, "PlanInput")?))
    }
}

/// `lain-probe-ffprobe`: ffprobe on a gateway-resolved file.
pub struct Probe;

#[derive(Deserialize)]
struct ProbeInput {
    file_path: String,
    /// The configured binary (`ffprobe_path`); empty is PATH.
    #[serde(default)]
    ffprobe_path: String,
}

impl Provider for Probe {
    fn id(&self) -> &str {
        PROBE_ID
    }
    fn capabilities(&self) -> Vec<String> {
        vec![CAP_MEDIA_PROBE.into()]
    }
    fn health(&self) -> Result<(), Error> {
        lain_probe::exec::look_path("ffprobe").map(|_| ()).ok_or_else(|| Error::unavailable("ffprobe not found"))
    }
    fn invoke(&self, cap: &str, _: &str, v: Value) -> Result<Value, Error> {
        if cap != CAP_MEDIA_PROBE {
            return Err(wrong_cap(cap));
        }
        let i: ProbeInput = input(v, "MediaProbeRequest")?;
        if i.file_path.is_empty() {
            return Err(Error::invalid("file_path required"));
        }
        let r = lain_probe::probe(lain_probe::binary(&i.ffprobe_path), Path::new(&i.file_path), lain_probe::TIMEOUT)
            .map_err(|_| Error::unavailable("media probe failed"))?;
        let info = lain_probe::media_info(&r).map_err(|_| Error::unavailable("invalid ffprobe output"))?;
        out(&info)
    }
}

struct ThumbProvider(Arc<Generator>);

impl Provider for ThumbProvider {
    fn id(&self) -> &str {
        thumbnail::ID
    }
    fn capabilities(&self) -> Vec<String> {
        vec![thumbnail::CAP.into()]
    }
    fn health(&self) -> Result<(), Error> {
        self.0.health()
    }
    fn invoke(&self, cap: &str, _: &str, v: Value) -> Result<Value, Error> {
        if cap != thumbnail::CAP {
            return Err(wrong_cap(cap));
        }
        out(&self.0.generate(&input(v, "ThumbnailRequest")?)?)
    }
}

struct TranscodeProvider(Transcoder);

#[derive(Deserialize)]
struct V1Input {
    #[serde(default)]
    file_path: String,
}

impl Provider for TranscodeProvider {
    fn id(&self) -> &str {
        transcode::ID
    }
    fn capabilities(&self) -> Vec<String> {
        vec![transcode::CAP_V1.into(), transcode::CAP_V3.into()]
    }
    fn health(&self) -> Result<(), Error> {
        self.0.health()
    }
    fn invoke(&self, cap: &str, op: &str, v: Value) -> Result<Value, Error> {
        match cap {
            transcode::CAP_V1 => out(&self.0.prepare_sync(&input::<V1Input>(v, "TranscodeRequest")?.file_path)?),
            transcode::CAP_V3 => {
                let mut req: transcode::V3Request = input(v, "TranscodeV3Request")?;
                if req.action.is_empty() {
                    req.action = op.to_string();
                }
                if req.action == transcode::ACTION_LIST {
                    return out(&self.0.list_v3());
                }
                out(&self.0.invoke_v3(&req)?)
            }
            _ => Err(wrong_cap(cap)),
        }
    }
}

struct LocalProvider(Arc<LocalPlay>);

impl Provider for LocalProvider {
    fn id(&self) -> &str {
        localplay::ID
    }
    fn capabilities(&self) -> Vec<String> {
        vec![localplay::CAP.into()]
    }
    fn health(&self) -> Result<(), Error> {
        Ok(())
    }
    fn invoke(&self, cap: &str, op: &str, v: Value) -> Result<Value, Error> {
        if cap != localplay::CAP {
            return Err(wrong_cap(cap));
        }
        match op {
            "players" => Ok(json!({"players": self.0.manager.players()})),
            "play" => self.0.play(&input(v, "PlayInput")?),
            _ => Err(Error::invalid("localplay input required")),
        }
    }
}

struct SettingsProvider(Arc<SettingsStore>);

#[derive(Deserialize)]
struct SettingsIn {
    #[serde(default)]
    settings: lain_api::TranscodeSettings,
    #[serde(default)]
    base: lain_api::TranscodeSettings,
}

impl Provider for SettingsProvider {
    fn id(&self) -> &str {
        SETTINGS_ID
    }
    fn capabilities(&self) -> Vec<String> {
        vec![CAP_TRANSCODE_SETTINGS.into()]
    }
    fn health(&self) -> Result<(), Error> {
        Ok(())
    }
    fn invoke(&self, cap: &str, op: &str, v: Value) -> Result<Value, Error> {
        if cap != CAP_TRANSCODE_SETTINGS {
            return Err(wrong_cap(cap));
        }
        let i = || input::<SettingsIn>(v.clone(), "settings input");
        match op {
            "get" => out(&self.0.get()),
            "put" => out(&self.0.put(&i()?.settings)?),
            "has" => Ok(json!({"saved": self.0.has()?})),
            "ensure" => Ok(json!({"saved": self.0.ensure(&i()?.base)?})),
            _ => Err(Error::invalid("settings input required")),
        }
    }
}

/// The watch module's state in one server.
pub struct Watch {
    pub transcoder: Transcoder,
    pub thumbnails: Arc<Generator>,
    pub local: Arc<LocalPlay>,
    pub settings: Arc<SettingsStore>,
}

impl Watch {
    /// Opens the module's caches under `data_dir` (`thumbnails/`,
    /// `transcodes/`) and its settings in `store`.
    pub fn open(data_dir: &Path, store: Arc<Store>) -> Watch {
        Watch::with_transcoder(data_dir, store, Transcoder::new(&data_dir.join("transcodes")))
    }

    /// [`Watch::open`] with a prepared transcoder (tests pass hooks).
    pub fn with_transcoder(data_dir: &Path, store: Arc<Store>, transcoder: Transcoder) -> Watch {
        Watch {
            transcoder,
            thumbnails: Arc::new(Generator::new(&data_dir.join("thumbnails"))),
            local: Arc::new(LocalPlay::default()),
            settings: Arc::new(SettingsStore::new(store)),
        }
    }

    /// Registers every provider (and gives local play the registry).
    pub fn register(&self, reg: &Arc<Registry>) {
        reg.register(Arc::new(Planner));
        reg.register(Arc::new(Probe));
        reg.register(Arc::new(ThumbProvider(self.thumbnails.clone())));
        reg.register(Arc::new(TranscodeProvider(self.transcoder.clone())));
        reg.register(Arc::new(LocalProvider(self.local.clone())));
        reg.register(Arc::new(SettingsProvider(self.settings.clone())));
        self.local.attach(reg);
    }

    /// The default bindings of these providers (Go's composition).
    pub fn bindings() -> Vec<(&'static str, Binding)> {
        vec![
            (CAP_PLAYBACK_PLAN, Binding::new(BindingMode::FirstAccepted, &[PLANNER_ID])),
            (CAP_MEDIA_PROBE, Binding::new(BindingMode::ExactlyOne, &[PROBE_ID])),
            (thumbnail::CAP, Binding::new(BindingMode::ExactlyOne, &[thumbnail::ID])),
            (transcode::CAP_V1, Binding::new(BindingMode::ExactlyOne, &[transcode::ID])),
            (transcode::CAP_V3, Binding::new(BindingMode::ExactlyOne, &[transcode::ID])),
            (localplay::CAP, Binding::new(BindingMode::ExactlyOne, &[localplay::ID])),
            (CAP_TRANSCODE_SETTINGS, Binding::new(BindingMode::ExactlyOne, &[SETTINGS_ID])),
        ]
    }

    /// Stops transcodes and local players.
    pub fn close(&self) {
        self.transcoder.close();
        self.local.manager.close();
    }
}
