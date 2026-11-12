//! The operator's transcode policy (D-045): the shipped defaults, the
//! fill-and-validate rules, the cache identity of a session's output,
//! and its store (`meta/transcode_settings`, like the Go meta bucket).
//! Behavior is Go's `contracts.TranscodeSettings`, rule for rule.

use std::sync::Arc;

use lain_api::{TranscodeQuality, TranscodeSettings};
use lain_core::Error;
use lain_store::{Bucket, Store};
use sha2::{Digest, Sha256};

use crate::transcode::V3Request;

pub const DELIVERY_PROGRESSIVE: &str = "progressive";
pub const DELIVERY_HLS: &str = "hls";
/// A subtitle sidecar extracted for direct play (not a transcode).
pub const DELIVERY_SUBTITLE: &str = "subtitle";

pub const HLS_FMP4: &str = "fmp4";
pub const HLS_TS: &str = "mpegts";

pub const SUB_AUTO: &str = "auto";
pub const SUB_EXTRACT: &str = "extract";
pub const SUB_BURN: &str = "burn";
pub const SUB_OFF: &str = "off";

pub const VIDEO_H264: &str = "h264";
pub const VIDEO_HEVC: &str = "hevc";
pub const VIDEO_AV1: &str = "av1";

pub const AUDIO_AAC: &str = "aac";
pub const AUDIO_AC3: &str = "ac3";
pub const AUDIO_EAC3: &str = "eac3";

pub const HW_NONE: &str = "none";
pub const HW_VAAPI: &str = "vaapi";
pub const HW_NVENC: &str = "nvenc";
pub const HW_QSV: &str = "qsv";
pub const HW_AMF: &str = "amf";
pub const HW_V4L2M2M: &str = "v4l2m2m";
pub const HW_VIDEOTOOLBOX: &str = "videotoolbox";
pub const HW_RKMPP: &str = "rkmpp";

/// Every hardware backend, in probe order.
pub const HW_BACKENDS: [&str; 7] = [HW_VAAPI, HW_NVENC, HW_QSV, HW_AMF, HW_V4L2M2M, HW_VIDEOTOOLBOX, HW_RKMPP];

/// The VA-API render node used when none is configured.
pub const DEFAULT_HW_DEVICE: &str = "/dev/dri/renderD128";

/// Codecs with hardware decoding support; settings pick a subset.
pub const HW_DECODE_CODECS: [&str; 7] = ["h264", "hevc", "mpeg2", "vc1", "vp8", "vp9", "av1"];

pub const TONEMAP_BT2390: &str = "bt2390";
pub const TONEMAP_HABLE: &str = "hable";
pub const TONEMAP_MODE_AUTO: &str = "auto";
pub const TONEMAP_MODE_ALWAYS: &str = "always";
pub const TONEMAP_MODE_NEVER: &str = "never";

pub const DEINTERLACE_AUTO: &str = "auto";
pub const DEINTERLACE_OFF: &str = "off";
pub const DEINTERLACE_YADIF: &str = "yadif";
pub const DEINTERLACE_BWDIF: &str = "bwdif";

pub const DOWNMIX_NONE: &str = "none";
pub const DOWNMIX_NIGHTMODE: &str = "nightmode";

/// The x264/x265 preset ladder.
pub const ENCODER_PRESETS: [&str; 9] =
    ["ultrafast", "superfast", "veryfast", "faster", "fast", "medium", "slow", "slower", "veryslow"];

/// The automatic hardware pick on first boot: vendor-native APIs before
/// generic Linux ones, SoC paths before V4L2's wrapper.
pub const AUTOMATIC_HARDWARE: [&str; 7] = [HW_NVENC, HW_QSV, HW_VAAPI, HW_VIDEOTOOLBOX, HW_AMF, HW_RKMPP, HW_V4L2M2M];

fn q(name: &str, w: i64, h: i64, kbps: i64) -> TranscodeQuality {
    TranscodeQuality { name: name.into(), max_width: w, max_height: h, bitrate_kbps: kbps }
}

/// The shipped policy: HLS by default (it plays before preparation
/// ends), the D-030 conservative encode, everything else opt-in.
pub fn defaults() -> TranscodeSettings {
    TranscodeSettings {
        default_delivery: DELIVERY_HLS.into(),
        hls_segment_seconds: 6,
        hls_segment_container: HLS_FMP4.into(),
        throttle: true,
        throttle_ahead_sec: 30,
        segment_deletion: false,
        segment_keep_sec: 60,
        idle_timeout_sec: 120,
        cache_bytes: 20 << 30,
        queue_size: 8,
        max_concurrent: 2,
        thread_count: 0,
        muxing_queue_size: 2048,
        transcode_temp_path: String::new(),
        remote_bitrate_limit_kbps: 0,
        encoder_preset: "veryfast".into(),
        crf: 23,
        h264_preset: String::new(),
        h265_preset: String::new(),
        av1_preset: String::new(),
        h264_crf: 23,
        h265_crf: 28,
        av1_crf: 32,
        allow_hevc: false,
        allow_av1: false,
        qualities: vec![
            q("2160p", 3840, 2160, 40000),
            q("1080p", 1920, 1080, 8000),
            q("720p", 1280, 720, 4000),
            q("480p", 854, 480, 1500),
            q("360p", 640, 360, 700),
        ],
        audio_bitrate_kbps: 160,
        audio_vbr: false,
        downmix_audio: true,
        downmix_audio_boost: 2.0,
        downmix_stereo_algorithm: DOWNMIX_NONE.into(),
        subtitle_mode: SUB_AUTO.into(),
        allow_subtitle_extraction: None,
        fallback_font_path: String::new(),
        fallback_font_name: String::new(),
        fallback_font_enabled: None,
        tone_mapping: true,
        tone_mapping_algorithm: TONEMAP_BT2390.into(),
        tone_mapping_mode: TONEMAP_MODE_AUTO.into(),
        tone_mapping_peak_nits: 100,
        deinterlace: DEINTERLACE_AUTO.into(),
        deinterlace_method: DEINTERLACE_YADIF.into(),
        deinterlace_double_rate: false,
        hardware_acceleration: HW_NONE.into(),
        hardware_decode_codecs: HW_DECODE_CODECS.iter().map(|s| s.to_string()).collect(),
        hardware_encode: false,
        hardware_low_power: false,
        hardware_decode_10bit_hevc: false,
        hardware_decode_10bit_vp9: false,
        hardware_device: DEFAULT_HW_DEVICE.into(),
        ffmpeg_path: String::new(),
        ffprobe_path: String::new(),
    }
}

/// Fills zero values with defaults, so a partial document still yields
/// a full policy.
pub fn normalize(s: &TranscodeSettings) -> TranscodeSettings {
    let d = defaults();
    let mut s = s.clone();
    fn fill_s(v: &mut String, d: &str) {
        if v.is_empty() {
            *v = d.to_string();
        }
    }
    fn fill_i(v: &mut i64, d: i64) {
        if *v <= 0 {
            *v = d;
        }
    }
    fill_s(&mut s.default_delivery, &d.default_delivery);
    fill_i(&mut s.hls_segment_seconds, d.hls_segment_seconds);
    fill_s(&mut s.hls_segment_container, &d.hls_segment_container);
    fill_i(&mut s.throttle_ahead_sec, d.throttle_ahead_sec);
    fill_i(&mut s.segment_keep_sec, d.segment_keep_sec);
    fill_i(&mut s.idle_timeout_sec, d.idle_timeout_sec);
    fill_i(&mut s.cache_bytes, d.cache_bytes);
    fill_i(&mut s.queue_size, d.queue_size);
    fill_i(&mut s.max_concurrent, d.max_concurrent);
    fill_s(&mut s.encoder_preset, &d.encoder_preset);
    fill_i(&mut s.crf, d.crf);
    if s.qualities.is_empty() {
        s.qualities = d.qualities.clone();
    }
    fill_i(&mut s.audio_bitrate_kbps, d.audio_bitrate_kbps);
    if s.downmix_audio_boost <= 0.0 || s.downmix_audio_boost.is_nan() {
        s.downmix_audio_boost = d.downmix_audio_boost;
    }
    fill_s(&mut s.downmix_stereo_algorithm, &d.downmix_stereo_algorithm);
    fill_i(&mut s.muxing_queue_size, d.muxing_queue_size);
    fill_i(&mut s.h264_crf, d.h264_crf);
    fill_i(&mut s.h265_crf, d.h265_crf);
    fill_i(&mut s.av1_crf, d.av1_crf);
    fill_s(&mut s.subtitle_mode, &d.subtitle_mode);
    fill_s(&mut s.tone_mapping_algorithm, &d.tone_mapping_algorithm);
    fill_s(&mut s.tone_mapping_mode, &d.tone_mapping_mode);
    fill_i(&mut s.tone_mapping_peak_nits, d.tone_mapping_peak_nits);
    fill_s(&mut s.deinterlace, &d.deinterlace);
    fill_s(&mut s.deinterlace_method, &d.deinterlace_method);
    fill_s(&mut s.hardware_acceleration, &d.hardware_acceleration);
    fill_s(&mut s.hardware_device, &d.hardware_device);
    s
}

/// Rejects out-of-range values with Go's stable messages.
pub fn validate(s: &TranscodeSettings) -> Result<(), String> {
    let one_of = |v: &str, set: &[&str]| set.contains(&v);
    if !one_of(&s.default_delivery, &[DELIVERY_PROGRESSIVE, DELIVERY_HLS]) {
        return Err(format!("default_delivery must be {DELIVERY_PROGRESSIVE} or {DELIVERY_HLS}"));
    }
    if !(1..=30).contains(&s.hls_segment_seconds) {
        return Err("hls_segment_seconds must be 1..30".into());
    }
    if !one_of(&s.hls_segment_container, &[HLS_FMP4, HLS_TS]) {
        return Err(format!("hls_segment_container must be {HLS_FMP4} or {HLS_TS}"));
    }
    if !(5..=600).contains(&s.throttle_ahead_sec) {
        return Err("throttle_ahead_sec must be 5..600".into());
    }
    if !(0..=3600).contains(&s.segment_keep_sec) {
        return Err("segment_keep_sec must be 0..3600".into());
    }
    if !(15..=3600).contains(&s.idle_timeout_sec) {
        return Err("idle_timeout_sec must be 15..3600".into());
    }
    if s.cache_bytes < 1 << 30 {
        return Err("cache_bytes must be at least 1 GiB".into());
    }
    if !(1..=64).contains(&s.queue_size) {
        return Err("queue_size must be 1..64".into());
    }
    if !(1..=16).contains(&s.max_concurrent) {
        return Err("max_concurrent must be 1..16".into());
    }
    if !ENCODER_PRESETS.contains(&s.encoder_preset.as_str()) {
        return Err(format!("encoder_preset must be one of {}", ENCODER_PRESETS.join(", ")));
    }
    if !(0..=51).contains(&s.crf) {
        return Err("crf must be 0..51".into());
    }
    for q in &s.qualities {
        if q.name.trim().is_empty() {
            return Err("quality entries need a name".into());
        }
        if q.max_width < 0 || q.max_height < 0 || q.bitrate_kbps < 0 {
            return Err(format!("quality {} has negative bounds", q.name));
        }
    }
    if !(32..=640).contains(&s.audio_bitrate_kbps) {
        return Err("audio_bitrate_kbps must be 32..640".into());
    }
    if !(0.5..=8.0).contains(&s.downmix_audio_boost) {
        return Err("downmix_audio_boost must be 0.5..8".into());
    }
    if !one_of(&s.downmix_stereo_algorithm, &[DOWNMIX_NONE, DOWNMIX_NIGHTMODE]) {
        return Err("downmix_stereo_algorithm must be none or nightmode".into());
    }
    if !(0..=64).contains(&s.thread_count) {
        return Err("thread_count must be 0..64".into());
    }
    if !(128..=65536).contains(&s.muxing_queue_size) {
        return Err("max_muxing_queue_size must be 128..65536".into());
    }
    if !(0..=1_000_000).contains(&s.remote_bitrate_limit_kbps) {
        return Err("remote_bitrate_limit_kbps must be 0..1000000".into());
    }
    for (name, crf) in [("av1_crf", s.av1_crf), ("h264_crf", s.h264_crf), ("h265_crf", s.h265_crf)] {
        if !(0..=51).contains(&crf) {
            return Err(format!("{name} must be 0..51"));
        }
    }
    for (name, preset) in
        [("av1_preset", &s.av1_preset), ("h264_preset", &s.h264_preset), ("h265_preset", &s.h265_preset)]
    {
        if !preset.is_empty() && !ENCODER_PRESETS.contains(&preset.as_str()) {
            return Err(format!("{name} must be one of {}", ENCODER_PRESETS.join(", ")));
        }
    }
    if !one_of(&s.subtitle_mode, &[SUB_AUTO, SUB_EXTRACT, SUB_BURN, SUB_OFF]) {
        return Err("subtitle_mode must be auto, extract, burn or off".into());
    }
    if !one_of(&s.tone_mapping_algorithm, &[TONEMAP_BT2390, TONEMAP_HABLE, "reinhard", "mobius", "clip", "linear"]) {
        return Err("tone_mapping_algorithm is not recognized".into());
    }
    if !one_of(&s.tone_mapping_mode, &[TONEMAP_MODE_AUTO, TONEMAP_MODE_ALWAYS, TONEMAP_MODE_NEVER]) {
        return Err("tone_mapping_mode must be auto, always or never".into());
    }
    if !(50..=10000).contains(&s.tone_mapping_peak_nits) {
        return Err("tone_mapping_peak_nits must be 50..10000".into());
    }
    if !one_of(&s.deinterlace, &[DEINTERLACE_AUTO, DEINTERLACE_OFF]) {
        return Err("deinterlace must be auto or off".into());
    }
    if !one_of(&s.deinterlace_method, &[DEINTERLACE_YADIF, DEINTERLACE_BWDIF]) {
        return Err("deinterlace_method must be yadif or bwdif".into());
    }
    if s.hardware_acceleration != HW_NONE && !HW_BACKENDS.contains(&s.hardware_acceleration.as_str()) {
        return Err("hardware_acceleration is not recognized".into());
    }
    for c in &s.hardware_decode_codecs {
        if !HW_DECODE_CODECS.contains(&c.as_str()) {
            return Err(format!("hardware_decode_codecs contains unknown codec {c}"));
        }
    }
    // The device is one argv element: reject only what breaks argv.
    let device = s.hardware_device.trim();
    if !device.is_empty() && (device.len() > 512 || device.contains(['\0', '\n', '\r'])) {
        return Err("hardware_device is not a usable device path".into());
    }
    Ok(())
}

/// A stream-copy permission: only an explicit `false` forbids copying.
pub fn allow_stream_copy(flag: Option<bool>) -> bool {
    flag != Some(false)
}

/// The client accepts an HEVC stream as it is (D-273). The list is a
/// vocabulary of one: values this server does not know are ignored.
pub fn accepts_hevc_copy(codecs: &[String]) -> bool {
    codecs.iter().any(|c| c.eq_ignore_ascii_case(VIDEO_HEVC))
}

pub fn allows_subtitle_extraction(s: &TranscodeSettings) -> bool {
    s.allow_subtitle_extraction != Some(false)
}

pub fn allows_fallback_fonts(s: &TranscodeSettings) -> bool {
    s.fallback_font_enabled != Some(false)
}

/// The per-codec CRF and preset, falling back to the general values.
pub fn encoder_tuning(s: &TranscodeSettings, codec: &str) -> (i64, String) {
    let (mut crf, mut preset) = (s.crf, s.encoder_preset.clone());
    let (c, p) = match codec {
        VIDEO_HEVC => (s.h265_crf, &s.h265_preset),
        VIDEO_AV1 => (s.av1_crf, &s.av1_preset),
        _ => (s.h264_crf, &s.h264_preset),
    };
    if c > 0 {
        crf = c;
    }
    if !p.is_empty() {
        preset = p.clone();
    }
    (crf, preset)
}

/// A ladder entry by name; empty or unknown is `None`.
pub fn quality<'a>(s: &'a TranscodeSettings, name: &str) -> Option<&'a TranscodeQuality> {
    if name.is_empty() {
        return None;
    }
    s.qualities.iter().find(|q| q.name == name)
}

/// AAC VBR quality step (`-q:a`) for a target bitrate.
pub fn audio_vbr_quality(kbps: i64) -> f64 {
    match kbps {
        k if k >= 192 => 2.0,
        k if k >= 128 => 1.5,
        k if k >= 96 => 1.0,
        k if k >= 64 => 0.5,
        _ => 0.1,
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The SHA-256 of `s`, hex.
pub fn sha256_hex(s: &str) -> String {
    hex(&Sha256::digest(s.as_bytes()))
}

/// The cache/session identity of everything that changes the produced
/// bytes (D-030/D-045). Operational knobs (threads, muxing queue, temp
/// path, cache and queue bounds) stay out.
pub fn profile_key(s: &TranscodeSettings, req: &V3Request) -> String {
    let or = |v: &str, d: &str| if v.is_empty() { d.to_string() } else { v.to_string() };
    let delivery = or(&req.delivery, &s.default_delivery);
    let video = or(&req.video_codec, VIDEO_H264);
    let audio = or(&req.audio_codec, AUDIO_AAC);
    let sub_mode = or(&req.subtitle_mode, &s.subtitle_mode);
    let quality_spec = quality(s, &req.quality)
        .map(|q| format!("{}/{}/{}", q.max_width, q.max_height, q.bitrate_kbps))
        .unwrap_or_default();
    let a = req.audio_stream.map(|n| n.to_string()).unwrap_or_else(|| "default".into());
    let sub = req.subtitle_stream.map(|n| n.to_string()).unwrap_or_else(|| "off".into());
    let mut fields = vec![
        "v3".to_string(),
        delivery,
        req.quality.clone(),
        quality_spec,
        video,
        audio,
        sub_mode,
        format!("ab:{}", s.audio_bitrate_kbps),
        format!("vbr:{}", s.audio_vbr),
        format!("dm:{}", s.downmix_audio),
        format!("dmb:{:.2}", s.downmix_audio_boost),
        format!("dma:{}", s.downmix_stereo_algorithm),
        format!("preset:{}", s.encoder_preset),
        format!("p:{}/{}/{}", s.h264_preset, s.h265_preset, s.av1_preset),
        format!("crf:{}", s.crf),
        format!("crf3:{}/{}/{}", s.h264_crf, s.h265_crf, s.av1_crf),
        format!("maxrate:{}", req.max_bitrate_kbps),
        format!("hw:{}", s.hardware_acceleration),
        format!("hwe:{}", s.hardware_encode),
        format!("hwlp:{}", s.hardware_low_power),
        format!("hw10:{}/{}", s.hardware_decode_10bit_hevc, s.hardware_decode_10bit_vp9),
        format!("hwdec:{}", s.hardware_decode_codecs.join(",")),
        format!("ffmpeg:{}", s.ffmpeg_path),
        format!(
            "tm:{}:{}:{}:{}",
            s.tone_mapping, s.tone_mapping_algorithm, s.tone_mapping_mode, s.tone_mapping_peak_nits
        ),
        format!("di:{}:{}", s.deinterlace, s.deinterlace_double_rate),
        format!("dim:{}", s.deinterlace_method),
        format!("copyv:{}", allow_stream_copy(req.allow_video_stream_copy)),
        format!("copya:{}", allow_stream_copy(req.allow_audio_stream_copy)),
        format!("seg:{}", s.hls_segment_seconds),
        format!("hlsct:{}", s.hls_segment_container),
        format!("font:{}/{}/{}", s.fallback_font_path, s.fallback_font_name, allows_fallback_fonts(s)),
        format!("a:{a}"),
        format!("s:{sub}"),
    ];
    // Appended only when claimed, so every key made before D-273 stays.
    if accepts_hevc_copy(&req.video_copy_codecs) {
        fields.push("vcopy:hevc".into());
    }
    if !req.video_effect.is_empty() {
        fields.push(format!("fx:{}", req.video_effect));
    }
    let h = Sha256::digest(fields.join("|").as_bytes());
    format!("web-mp4-v3-{}", hex(&h[..6]))
}

/// The meta key of the saved policy.
const KEY: &[u8] = b"transcode_settings";

/// The saved policy in the store's `meta` table.
pub struct SettingsStore {
    store: Arc<Store>,
}

impl SettingsStore {
    pub fn new(store: Arc<Store>) -> Self {
        SettingsStore { store }
    }

    /// The effective settings: defaults when nothing is saved or the
    /// read fails (a broken store degrades to the shipped policy).
    pub fn get(&self) -> TranscodeSettings {
        match self.store.read(|tx| tx.get_json::<TranscodeSettings>(Bucket::Meta, KEY)) {
            Ok(Some(s)) => normalize(&s),
            Ok(None) => defaults(),
            Err(e) => {
                tracing::warn!(err = %e, "transcode settings unreadable; using defaults");
                defaults()
            }
        }
    }

    /// Whether an operator save exists; a corrupt save is an error.
    pub fn has(&self) -> Result<bool, Error> {
        self.store
            .read(|tx| tx.get_json::<TranscodeSettings>(Bucket::Meta, KEY))
            .map(|s| s.is_some())
            .map_err(|e| Error::unavailable(e.to_string()))
    }

    /// Normalizes, validates and stores.
    pub fn put(&self, s: &TranscodeSettings) -> Result<TranscodeSettings, Error> {
        let s = normalize(s);
        validate(&s).map_err(Error::invalid)?;
        self.store.write(|tx| tx.put_json(Bucket::Meta, KEY, &s)).map_err(|e| Error::unavailable(e.to_string()))?;
        Ok(s)
    }

    /// Writes `base` only when no readable save exists; true when one
    /// already did.
    pub fn ensure(&self, base: &TranscodeSettings) -> Result<bool, Error> {
        if self.has().unwrap_or(false) {
            return Ok(true);
        }
        self.put(base)?;
        Ok(false)
    }
}

/// `settings` with the first available backend of [`AUTOMATIC_HARDWARE`]
/// enabled for decode and encode, or software when none probed.
pub fn with_automatic_hardware(
    settings: &TranscodeSettings,
    available: &std::collections::BTreeMap<String, bool>,
) -> TranscodeSettings {
    let mut s = settings.clone();
    s.hardware_acceleration = HW_NONE.into();
    s.hardware_encode = false;
    if let Some(b) = AUTOMATIC_HARDWARE.iter().find(|b| available.get(**b).copied().unwrap_or(false)) {
        s.hardware_acceleration = b.to_string();
        s.hardware_encode = true;
    }
    s
}

/// What a saved policy becomes when the machine's hardware turns out
/// different from what was picked for it: `Some` only for a policy nobody
/// tuned (the shipped one, with whatever backend the first boot chose),
/// and only when today's pick differs. A policy with any setting changed
/// is the operator's and is left alone, hardware included.
pub fn repick_hardware(
    saved: &TranscodeSettings,
    available: &std::collections::BTreeMap<String, bool>,
) -> Option<TranscodeSettings> {
    let now = with_automatic_hardware(&defaults(), available);
    let mut untuned = saved.clone();
    untuned.hardware_acceleration = now.hardware_acceleration.clone();
    untuned.hardware_encode = now.hardware_encode;
    (untuned == now && *saved != now).then_some(now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_untuned_policy_follows_the_hardware() {
        let only = |b: &str| std::collections::BTreeMap::from([(b.to_string(), true)]);
        let both = std::collections::BTreeMap::from([(HW_QSV.to_string(), true), (HW_NVENC.to_string(), true)]);
        // Picked QSV when the NVIDIA GPU did not probe; it does now.
        let saved = with_automatic_hardware(&defaults(), &only(HW_QSV));
        let next = repick_hardware(&saved, &both).unwrap();
        assert_eq!((next.hardware_acceleration.as_str(), next.hardware_encode), (HW_NVENC, true));
        // Already the pick, or the hardware went away entirely.
        assert_eq!(repick_hardware(&next, &both), None);
        assert_eq!(repick_hardware(&saved, &Default::default()).unwrap().hardware_acceleration, HW_NONE);
        // Anything the operator changed makes the policy theirs.
        let mut tuned = saved.clone();
        tuned.hls_segment_seconds += 2;
        assert_eq!(repick_hardware(&tuned, &both), None);
    }

    #[test]
    fn defaults_validate_and_normalize_fills_zeroes() {
        assert_eq!(validate(&defaults()), Ok(()));
        let n = normalize(&TranscodeSettings::default());
        assert_eq!(validate(&n), Ok(()));
        assert_eq!(n.queue_size, 8);
        assert!(!n.throttle, "booleans are not defaulted by normalize, like Go");
        assert_eq!(n.qualities.len(), 5);
    }

    #[test]
    fn validation_messages() {
        let bad = |f: &dyn Fn(&mut TranscodeSettings)| {
            let mut s = defaults();
            f(&mut s);
            validate(&s).unwrap_err()
        };
        assert_eq!(bad(&|s| s.hls_segment_seconds = 31), "hls_segment_seconds must be 1..30");
        assert_eq!(bad(&|s| s.default_delivery = "x".into()), "default_delivery must be progressive or hls");
        assert_eq!(bad(&|s| s.cache_bytes = 5), "cache_bytes must be at least 1 GiB");
        assert_eq!(bad(&|s| s.h265_crf = 60), "h265_crf must be 0..51");
        assert!(bad(&|s| s.h264_preset = "warp".into()).starts_with("h264_preset must be one of ultrafast"));
        assert_eq!(bad(&|s| s.hardware_acceleration = "cuda".into()), "hardware_acceleration is not recognized");
        assert_eq!(bad(&|s| s.hardware_device = "/dev/x\0y".into()), "hardware_device is not a usable device path");
        assert_eq!(
            bad(&|s| s.hardware_decode_codecs = vec!["mpeg4".into()]),
            "hardware_decode_codecs contains unknown codec mpeg4"
        );
        assert_eq!(bad(&|s| s.downmix_audio_boost = 9.0), "downmix_audio_boost must be 0.5..8");
    }

    #[test]
    fn tuning_and_quality() {
        let mut s = defaults();
        assert_eq!(encoder_tuning(&s, VIDEO_HEVC), (28, "veryfast".to_string()));
        s.av1_preset = "slow".into();
        assert_eq!(encoder_tuning(&s, VIDEO_AV1), (32, "slow".to_string()));
        assert_eq!(quality(&s, "720p").unwrap().max_height, 720);
        assert!(quality(&s, "").is_none() && quality(&s, "999p").is_none());
        assert_eq!(audio_vbr_quality(160), 1.5);
        assert!(!allow_stream_copy(Some(false)) && allow_stream_copy(None));
    }

    #[test]
    fn profile_key_tracks_output_changes_only() {
        let s = defaults();
        let req = V3Request::default();
        let base = profile_key(&s, &req);
        assert!(base.starts_with("web-mp4-v3-") && base.len() == "web-mp4-v3-".len() + 12);
        let mut threads = s.clone();
        threads.thread_count = 8;
        threads.transcode_temp_path = "/tmp/x".into();
        assert_eq!(profile_key(&threads, &req), base, "operational knobs keep the key");
        let mut ladder = s.clone();
        ladder.qualities[2].bitrate_kbps = 3000;
        let r720 = V3Request { quality: "720p".into(), ..V3Request::default() };
        assert_ne!(profile_key(&ladder, &r720), profile_key(&s, &r720), "ladder values are in the key");
        let sub = V3Request { subtitle_stream: Some(2), ..V3Request::default() };
        assert_ne!(profile_key(&s, &sub), base);
        let unknown = V3Request { video_copy_codecs: vec!["vvc".into()], ..V3Request::default() };
        assert_eq!(profile_key(&s, &unknown), base, "an unknown copy codec is ignored");
        let hevc = V3Request { video_copy_codecs: vec!["HEVC".into()], ..V3Request::default() };
        assert_ne!(profile_key(&s, &hevc), base);
        let fx = V3Request { video_effect: "anime4k-a".into(), ..V3Request::default() };
        assert_ne!(profile_key(&s, &fx), base);
    }

    #[test]
    fn automatic_hardware_order() {
        let mut avail = std::collections::BTreeMap::new();
        avail.insert(HW_VAAPI.to_string(), true);
        avail.insert(HW_V4L2M2M.to_string(), true);
        let s = with_automatic_hardware(&defaults(), &avail);
        assert_eq!((s.hardware_acceleration.as_str(), s.hardware_encode), (HW_VAAPI, true));
        let s = with_automatic_hardware(&defaults(), &Default::default());
        assert_eq!((s.hardware_acceleration.as_str(), s.hardware_encode), (HW_NONE, false));
    }

    #[test]
    fn store_round_trip() {
        let dir = lain_store::test_dir("watch-settings");
        let store = Arc::new(Store::open(&dir).unwrap());
        let st = SettingsStore::new(store);
        assert!(!st.has().unwrap());
        assert_eq!(st.get(), defaults());
        assert!(!st.ensure(&defaults()).unwrap());
        assert!(st.ensure(&TranscodeSettings::default()).unwrap(), "a save is never overwritten");
        let mut s = defaults();
        s.crf = 18;
        assert_eq!(st.put(&s).unwrap().crf, 18);
        assert_eq!(st.get().crf, 18);
        s.crf = 99;
        assert_eq!(st.put(&s).unwrap_err().msg, "crf must be 0..51");
        drop(st);
        std::fs::remove_dir_all(dir).ok();
    }
}
