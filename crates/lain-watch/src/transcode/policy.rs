//! Session planning: operator settings, probed streams and the client's
//! selection resolve into one [`EncodePlan`] (copy or encode, codecs,
//! filters, hardware, subtitles, reasons). `encode.rs` turns a plan
//! into argv; `hls.rs` owns packaging. Go's `transcode/policy.go`.

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;
use std::time::Duration;

use lain_api::TranscodeSettings;
use lain_core::Error;
use lain_probe::{RawReport, RawStream, exec, image_subtitle_codec, text_subtitle_codec};

use crate::settings::{self as st, allow_stream_copy, encoder_tuning, quality};
use crate::transcode::effects;
use crate::transcode::{SourceSpec, V3Request, unsupported};

/// The probed ffmpeg surface (D-031: probing, never silent switching).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Capabilities {
    pub encoders: BTreeSet<String>,
    pub filters: BTreeSet<String>,
    pub hwaccels: BTreeSet<String>,
    /// The zscale+tonemap chain executes.
    pub tone_map: bool,
    /// The libplacebo BT.2390 path executes.
    pub tone_map_2390: bool,
    /// libplacebo renders a frame on this machine (D-274).
    pub effects: bool,
    pub hardware: BTreeMap<String, bool>,
    pub ffmpeg: String,
}

impl Capabilities {
    pub fn has_encoder(&self, name: &str) -> bool {
        self.encoders.contains(name)
    }
    pub fn has_filter(&self, name: &str) -> bool {
        self.filters.contains(name)
    }
    fn hardware_ok(&self, backend: &str) -> bool {
        self.hardware.get(backend).copied().unwrap_or(false)
    }
}

/// One resolved session policy.
#[derive(Debug, Clone, Default)]
pub struct EncodePlan {
    pub spec: SourceSpec,
    pub delivery: String,
    pub settings: TranscodeSettings,
    pub duration_sec: f64,
    pub video: RawStream,
    pub audio: Option<RawStream>,
    pub subtitle: Option<RawStream>,
    /// `off`, `extract` or `burn`.
    pub subtitle_mode: String,
    pub burn_text: bool,
    pub burn_image: bool,
    pub copy_video: bool,
    pub copy_audio: bool,
    pub video_codec: String,
    pub audio_codec: String,
    pub encoder: String,
    pub hw_backend: String,
    pub hw_decode: bool,
    pub need_tone: bool,
    pub fallback: String,
    pub crf: i64,
    pub preset: String,
    pub filters: Vec<String>,
    pub complex_filter: String,
    pub width: i64,
    pub height: i64,
    pub max_bitrate_kbps: i64,
    pub audio_bitrate_kbps: i64,
    pub downmix: bool,
    pub reasons: Vec<String>,
    pub method: String,
    pub profile: String,
    /// The video effect rendered into this session (D-274), `""` for
    /// none, and its shader file once the runner has written it.
    pub effect: String,
    pub effect_file: String,
    /// The software retry of a failed hardware encode: as HLS it writes
    /// its init segment and segments under their own names
    /// (`hls::RETRY_PREFIX`), never the failed attempt's URLs.
    pub software_retry: bool,
}

impl EncodePlan {
    pub fn is_hls(&self) -> bool {
        self.delivery == st::DELIVERY_HLS
    }

    /// The backend in use for the status: the encode backend, else the
    /// configured one when only decoding is offloaded.
    pub fn hardware_label(&self) -> String {
        if !self.hw_backend.is_empty() {
            return self.hw_backend.clone();
        }
        if self.hw_decode {
            return self.settings.hardware_acceleration.clone();
        }
        String::new()
    }
}

pub(crate) fn join_notes(a: &str, b: &str) -> String {
    match (a.is_empty(), b.is_empty()) {
        (true, _) => b.to_string(),
        (_, true) => a.to_string(),
        _ => format!("{a}; {b}"),
    }
}

pub(crate) fn append_reason(list: &mut Vec<String>, r: &str) {
    if !list.iter().any(|x| x == r) {
        list.push(r.to_string());
    }
}

fn select_audio(report: &RawReport, want: Option<i64>) -> Result<Option<RawStream>, Error> {
    let audio = || report.streams.iter().filter(|s| s.codec_type == "audio");
    if let Some(w) = want {
        return audio()
            .find(|s| s.index == w)
            .cloned()
            .map(Some)
            .ok_or_else(|| Error::invalid("audio_stream is not an audio track"));
    }
    Ok(audio().find(|s| s.is_default()).or_else(|| audio().next()).cloned())
}

fn select_subtitle(report: &RawReport, want: Option<i64>) -> Result<Option<RawStream>, Error> {
    let Some(w) = want else { return Ok(None) };
    report
        .streams
        .iter()
        .find(|s| s.index == w && s.codec_type == "subtitle")
        .cloned()
        .map(Some)
        .ok_or_else(|| Error::invalid("subtitle_stream is not a subtitle track"))
}

fn source_kbps(bps: i64) -> i64 {
    if bps <= 0 { 0 } else { bps / 1000 }
}

/// Resolves one v3 session. Pure apart from the report and the
/// capabilities it is handed: nothing here starts work.
pub fn plan_v3(
    spec: &SourceSpec,
    req: &V3Request,
    caps: &Capabilities,
    report: &RawReport,
) -> Result<EncodePlan, Error> {
    let settings = st::normalize(&req.settings);
    st::validate(&settings).map_err(Error::invalid)?;
    let mut p = EncodePlan {
        spec: spec.clone(),
        settings: settings.clone(),
        duration_sec: report.duration_sec(),
        delivery: if req.delivery.is_empty() { settings.default_delivery.clone() } else { req.delivery.clone() },
        ..EncodePlan::default()
    };
    if p.delivery != st::DELIVERY_PROGRESSIVE && p.delivery != st::DELIVERY_HLS {
        return Err(Error::invalid("delivery must be progressive or hls"));
    }
    let Some(video) = report.first_video().cloned() else {
        return Err(unsupported("source has no video stream"));
    };
    p.width = video.width;
    p.height = video.height;
    p.video = video;
    p.audio = select_audio(report, req.audio_stream)?;
    p.subtitle = select_subtitle(report, req.subtitle_stream)?;
    let mode = if req.subtitle_mode.is_empty() { settings.subtitle_mode.clone() } else { req.subtitle_mode.clone() };
    p.resolve_subtitle(&mode, caps)?;

    if !req.quality.is_empty() && quality(&settings, &req.quality).is_none() {
        return Err(Error::invalid(format!("unknown quality {:?}", req.quality)));
    }
    let mut max_bitrate = req.max_bitrate_kbps;
    if let Some(q) = quality(&settings, &req.quality) {
        if q.max_width > 0 {
            p.width = p.width.min(q.max_width);
        }
        if q.max_height > 0 {
            p.height = p.height.min(q.max_height);
        }
        if q.bitrate_kbps > 0 && (max_bitrate == 0 || q.bitrate_kbps < max_bitrate) {
            max_bitrate = q.bitrate_kbps;
        }
    }
    if max_bitrate > 0 {
        p.max_bitrate_kbps = max_bitrate;
    }
    p.resolve_video(req, caps)?;
    p.resolve_audio(req, &settings)?;
    p.resolve_filters(caps);
    p.method = if p.copy_video && p.copy_audio { "remux".into() } else { "transcode".into() };
    if let Some(pol) = &req.policy {
        if p.method == "remux" && !pol.allow_remux {
            return Err(forbidden("remuxing is disabled for this user"));
        }
        if !p.copy_video && !pol.allow_video_transcode {
            return Err(forbidden("video transcoding is disabled for this user"));
        }
        if !p.copy_audio && !pol.allow_audio_transcode {
            return Err(forbidden("audio transcoding is disabled for this user"));
        }
    }
    p.profile = st::profile_key(&settings, req);
    for r in p.transcode_reasons(req) {
        append_reason(&mut p.reasons, &r);
    }
    Ok(p)
}

pub(crate) fn forbidden(msg: &str) -> Error {
    Error::new("forbidden", msg)
}

impl EncodePlan {
    fn resolve_subtitle(&mut self, mode: &str, caps: &Capabilities) -> Result<(), Error> {
        if mode == st::SUB_OFF {
            self.subtitle_mode = st::SUB_OFF.into();
            self.subtitle = None;
            return Ok(());
        }
        let Some(sub) = &self.subtitle else {
            self.subtitle_mode = st::SUB_OFF.into();
            return Ok(());
        };
        let image = image_subtitle_codec(&sub.codec_name);
        let text = text_subtitle_codec(&sub.codec_name);
        match mode {
            st::SUB_EXTRACT => {
                if !text {
                    return Err(unsupported("selected subtitle cannot be converted to WebVTT"));
                }
                self.subtitle_mode = st::SUB_EXTRACT.into();
                Ok(())
            }
            st::SUB_BURN => self.enable_burn(image, text, caps),
            st::SUB_AUTO => {
                if text {
                    self.subtitle_mode = st::SUB_EXTRACT.into();
                    Ok(())
                } else if image {
                    self.enable_burn(image, text, caps)
                } else {
                    Err(unsupported("selected subtitle cannot be converted to WebVTT"))
                }
            }
            _ => Err(Error::invalid("subtitle_mode must be auto, extract, burn or off")),
        }
    }

    fn enable_burn(&mut self, image: bool, text: bool, caps: &Capabilities) -> Result<(), Error> {
        if image {
            if !caps.has_filter("overlay") {
                return Err(unsupported("this ffmpeg cannot burn image subtitles (overlay filter missing)"));
            }
            self.subtitle_mode = st::SUB_BURN.into();
            self.burn_image = true;
            return Ok(());
        }
        if text {
            if !caps.has_filter("subtitles") && !caps.has_filter("ass") {
                return Err(unsupported("this ffmpeg cannot burn text subtitles (libass missing)"));
            }
            self.subtitle_mode = st::SUB_BURN.into();
            self.burn_text = true;
            return Ok(());
        }
        Err(unsupported("selected subtitle cannot be burned"))
    }

    fn resolve_video(&mut self, req: &V3Request, caps: &Capabilities) -> Result<(), Error> {
        let codec = if req.video_codec.is_empty() { st::VIDEO_H264.to_string() } else { req.video_codec.clone() };
        match codec.as_str() {
            st::VIDEO_H264 => {}
            st::VIDEO_HEVC if !self.settings.allow_hevc => {
                return Err(Error::invalid("HEVC encoding is not allowed by server settings"));
            }
            st::VIDEO_AV1 if !self.settings.allow_av1 => {
                return Err(Error::invalid("AV1 encoding is not allowed by server settings"));
            }
            st::VIDEO_HEVC | st::VIDEO_AV1 => {}
            _ => return Err(Error::invalid("video_codec must be h264, hevc or av1")),
        }
        self.video_codec = codec.clone();
        self.resolve_effect(req, caps)?;
        let pixel = self.video.pix_fmt.to_ascii_lowercase();
        let mut web_video =
            self.video.codec_name.eq_ignore_ascii_case("h264") && (pixel == "yuv420p" || pixel == "yuvj420p");
        if !allow_stream_copy(req.allow_video_stream_copy) {
            web_video = false;
            append_reason(&mut self.reasons, "video stream copy disabled by the client");
        }
        if self.max_bitrate_kbps > 0 && source_kbps(self.video.bit_rate_bps()) > self.max_bitrate_kbps {
            web_video = false;
        }
        // An HEVC source the client decodes is copied too (D-273), 8 or
        // 10 bit 4:2:0, into fMP4 only: the MPEG-TS segment container is
        // not a place browsers read HEVC from.
        let hevc_copy = st::accepts_hevc_copy(&req.video_copy_codecs)
            && allow_stream_copy(req.allow_video_stream_copy)
            && matches!(self.video.codec_name.to_ascii_lowercase().as_str(), "hevc" | "h265")
            && matches!(pixel.as_str(), "yuv420p" | "yuvj420p" | "yuv420p10le")
            && !(self.max_bitrate_kbps > 0 && source_kbps(self.video.bit_rate_bps()) > self.max_bitrate_kbps)
            && !(self.is_hls() && self.settings.hls_segment_container == st::HLS_TS);
        let v = &self.video;
        self.copy_video = ((web_video && codec == st::VIDEO_H264) || (hevc_copy && codec != st::VIDEO_AV1))
            && self.effect.is_empty()
            && !v.is_hdr()
            && !v.is_interlaced()
            && !self.burn_image
            && !self.burn_text
            && (self.width == 0 || self.width >= v.width)
            && (self.height == 0 || self.height >= v.height);
        if self.copy_video {
            self.encoder.clear();
            if hevc_copy {
                // The status names what is on the wire, not what was asked.
                self.video_codec = st::VIDEO_HEVC.into();
            }
            return Ok(());
        }
        let hdr = v.is_hdr();
        let mode = self.settings.tone_mapping_mode.clone();
        let need_tone = hdr && self.settings.tone_mapping && mode != st::TONEMAP_MODE_NEVER;
        let always = self.settings.tone_mapping && mode == st::TONEMAP_MODE_ALWAYS;
        if hdr && !self.settings.tone_mapping {
            return Err(unsupported("HDR source and tone mapping is disabled in server settings"));
        }
        if hdr && mode == st::TONEMAP_MODE_NEVER {
            return Err(unsupported("HDR source and tone mapping mode is never"));
        }
        if need_tone || always {
            if !caps.tone_map {
                return Err(unsupported("HDR tone mapping is unavailable on this server (probe failed)"));
            }
            if self.settings.tone_mapping_algorithm == st::TONEMAP_BT2390 && !caps.tone_map_2390 {
                self.fallback = join_notes(&self.fallback, "bt2390 tone mapping unavailable; used hable");
            }
        }
        self.need_tone = need_tone || always;
        let (encoder, backend, hw_decode, fallback) = choose_encoder(&self.settings, &codec, caps, &self.video)?;
        self.encoder = encoder;
        self.hw_backend = backend;
        self.hw_decode = hw_decode;
        if !self.effect.is_empty() && self.hw_backend == st::HW_VAAPI {
            // The VA-API encode owns the filter device libplacebo would
            // need as a Vulkan one.
            self.drop_effect("video effects are not rendered with a VA-API encode");
        }
        if !fallback.is_empty() {
            self.fallback = join_notes(&self.fallback, &fallback);
        }
        let (crf, preset) = encoder_tuning(&self.settings, &codec);
        self.crf = crf;
        self.preset = preset;
        Ok(())
    }

    /// The requested effect, when this session can carry it. An unknown
    /// preset is the client's error; a server or a source that cannot
    /// render one still plays, and says why in `fallback`.
    fn resolve_effect(&mut self, req: &V3Request, caps: &Capabilities) -> Result<(), Error> {
        let preset = req.video_effect.as_str();
        if preset.is_empty() {
            return Ok(());
        }
        if !effects::known(preset) {
            return Err(Error::invalid(format!("unknown video_effect {preset:?}")));
        }
        self.effect = preset.to_string();
        if !caps.effects {
            self.drop_effect("video effects are unavailable on this server");
        } else if self.video.is_hdr() {
            self.drop_effect("video effects are not rendered on HDR sources");
        } else if self.burn_image {
            self.drop_effect("video effects are not rendered with an image subtitle burned in");
        }
        if self.effect.is_empty() {
            return Ok(());
        }
        // `width`/`height` already hold the quality box when it is
        // smaller than the source; otherwise the preset decides.
        let boxed = self.width < self.video.width || self.height < self.video.height;
        let (max_w, max_h) = if boxed { (self.width, self.height) } else { (0, 0) };
        (self.width, self.height) = effects::output_size(preset, self.video.width, self.video.height, max_w, max_h);
        Ok(())
    }

    fn drop_effect(&mut self, why: &str) {
        self.effect.clear();
        self.fallback = join_notes(&self.fallback, why);
        self.width = self.width.min(self.video.width);
        self.height = self.height.min(self.video.height);
    }

    fn resolve_audio(&mut self, req: &V3Request, settings: &TranscodeSettings) -> Result<(), Error> {
        let codec = if req.audio_codec.is_empty() { st::AUDIO_AAC.to_string() } else { req.audio_codec.clone() };
        if ![st::AUDIO_AAC, st::AUDIO_AC3, st::AUDIO_EAC3].contains(&codec.as_str()) {
            return Err(Error::invalid("audio_codec must be aac, ac3 or eac3"));
        }
        self.audio_codec = codec.clone();
        self.audio_bitrate_kbps = settings.audio_bitrate_kbps;
        let Some(a) = &self.audio else {
            self.copy_audio = true;
            return Ok(());
        };
        self.copy_audio = a.codec_name.eq_ignore_ascii_case(&codec);
        if !allow_stream_copy(req.allow_audio_stream_copy) {
            self.copy_audio = false;
            append_reason(&mut self.reasons, "audio stream copy disabled by the client");
        }
        if a.channels > 2 && settings.downmix_audio {
            self.downmix = true;
            self.copy_audio = false;
        }
        Ok(())
    }

    fn resolve_filters(&mut self, caps: &Capabilities) {
        let mut filters = Vec::new();
        let subtitles = self.burn_text.then(|| {
            format!("subtitles={}{}", super::encode::SUBTITLE_PLACEHOLDER, subtitle_font_options(&self.settings))
        });
        // Text is drawn after an effect, at the output size, so the
        // shaders never redraw the letters.
        if self.effect.is_empty()
            && let Some(s) = &subtitles
        {
            filters.push(s.clone());
        }
        if self.settings.deinterlace == st::DEINTERLACE_AUTO && self.video.is_interlaced() {
            let mut method = self.settings.deinterlace_method.clone();
            if method == st::DEINTERLACE_BWDIF && !caps.has_filter("bwdif") {
                method = st::DEINTERLACE_YADIF.into();
                self.fallback = join_notes(&self.fallback, "bwdif is unavailable; used yadif");
            }
            if method.is_empty() {
                method = st::DEINTERLACE_YADIF.into();
            }
            if self.settings.deinterlace_double_rate {
                filters.push(format!("{method}=mode=send_field"));
            } else {
                filters.push(method);
            }
        }
        if self.need_tone {
            filters.push(tone_map_chain(&self.settings, caps));
        }
        let has_width = self.width > 0 && self.video.width > 0;
        let narrower = self.width < self.video.width;
        let shorter = self.height > 0 && self.height < self.video.height;
        if !self.effect.is_empty() {
            filters.push(effects::filter(self.width, self.height));
            filters.extend(subtitles);
        } else if has_width && (narrower || shorter) {
            filters.push(scale_filter(self.width, self.height));
        }
        if self.hw_backend == st::HW_VAAPI && !self.copy_video {
            filters.push("format=nv12".into());
            filters.push("hwupload".into());
        }
        if self.burn_image {
            let base = filters.join(",");
            let label = if base.is_empty() { "[0:v:0]".to_string() } else { format!("[0:v:0]{base}[base]") };
            let idx = self.subtitle.as_ref().map(|s| s.index).unwrap_or(0);
            self.complex_filter = format!("{label}[0:{idx}]overlay:shortest=0[vout]");
            self.filters.clear();
            return;
        }
        self.filters = filters;
    }

    /// Why this session cannot be a direct play (Jellyfin's
    /// TranscodeReasons idea).
    fn transcode_reasons(&self, req: &V3Request) -> Vec<String> {
        let mut r = Vec::new();
        let ext = file_ext(&self.spec.path);
        if !ext.is_empty() && ext != "mp4" && ext != "m4v" {
            append_reason(&mut r, &format!("container ({ext})"));
        }
        if !self.copy_video {
            append_reason(&mut r, &format!("video codec ({})", self.video.codec_name));
            if self.video.is_hdr() {
                append_reason(&mut r, &format!("HDR ({})", self.video.color_transfer.to_ascii_lowercase()));
            }
            if self.video.is_interlaced() && self.settings.deinterlace == st::DEINTERLACE_AUTO {
                append_reason(&mut r, "interlaced video");
            }
            if self.video_codec != st::VIDEO_H264 {
                append_reason(&mut r, &format!("output codec ({})", self.video_codec));
            }
            if !self.effect.is_empty() {
                append_reason(&mut r, &format!("video effect ({})", self.effect));
            }
        }
        if let Some(a) = &self.audio
            && !self.copy_audio
        {
            append_reason(&mut r, &format!("audio codec ({})", a.codec_name));
        }
        if self.downmix {
            append_reason(&mut r, "audio downmix");
            if self.settings.downmix_stereo_algorithm == st::DOWNMIX_NIGHTMODE
                && self.audio.as_ref().is_none_or(|a| a.channels != 6)
            {
                append_reason(&mut r, "nightmode needs a 5.1 source");
            }
        }
        if self.max_bitrate_kbps > 0 && !self.copy_video {
            append_reason(&mut r, &format!("bitrate limit ({} kbps)", self.max_bitrate_kbps));
        }
        if let Some(q) = quality(&self.settings, &req.quality) {
            append_reason(&mut r, &format!("quality ({})", q.name));
        }
        if self.burn_image {
            let codec = self.subtitle.as_ref().map(|s| s.codec_name.clone()).unwrap_or_default();
            append_reason(&mut r, &format!("image subtitle ({codec})"));
        }
        if self.burn_text {
            append_reason(&mut r, "subtitle burn-in");
        }
        if r.is_empty() {
            r.push("browser compatibility".into());
        }
        r
    }
}

/// The extension after the last dot of the whole path, lowercased
/// (Go's `fileExt`).
fn file_ext(path: &str) -> String {
    match path.rfind('.') {
        Some(i) => path[i + 1..].to_ascii_lowercase(),
        None => String::new(),
    }
}

/// The burn-in font fallback options (fontsdir, forced family).
pub(crate) fn subtitle_font_options(s: &TranscodeSettings) -> String {
    if !st::allows_fallback_fonts(s) {
        return String::new();
    }
    let mut out = String::new();
    if !s.fallback_font_path.trim().is_empty() {
        out.push_str(&format!(":fontsdir={}", super::encode::FONTS_PLACEHOLDER));
    }
    let name = sanitize_font_name(&s.fallback_font_name);
    if !name.is_empty() {
        out.push_str(&format!(":force_style='Fontname={name}'"));
    }
    out
}

/// Keeps only characters a font family can contain, so a setting cannot
/// inject filter-graph syntax.
pub(crate) fn sanitize_font_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '+'))
        .collect::<String>()
        .trim()
        .to_string()
}

/// Caps both dimensions, keeping aspect ratio and even sizes.
pub fn scale_filter(w: i64, h: i64) -> String {
    if w > 0 && h > 0 {
        format!("scale=w='min(iw,{w})':h='min(ih,{h})':force_original_aspect_ratio=decrease:force_divisible_by=2")
    } else if h > 0 {
        format!("scale=-2:'min(ih,{h})'")
    } else {
        format!("scale='min(iw,{w})':-2")
    }
}

/// The HDR to SDR chain.
pub fn tone_map_chain(s: &TranscodeSettings, caps: &Capabilities) -> String {
    let mut alg = s.tone_mapping_algorithm.as_str();
    if alg == st::TONEMAP_BT2390 && caps.tone_map_2390 {
        return "libplacebo=tonemapping=bt.2390:colorspace=bt709:color_primaries=bt709:color_trc=bt709:range=tv:format=yuv420p".into();
    }
    if alg == st::TONEMAP_BT2390 {
        alg = st::TONEMAP_HABLE;
    }
    format!(
        "format=gbrpf32le,zscale=transferin=smpte2084:primariesin=bt2020:matrixin=bt2020nc:transfer=linear:primaries=bt2020:matrix=bt2020nc:npl={},tonemap=tonemap={alg}:desat=0,zscale=transferin=linear:primariesin=bt2020:matrixin=bt2020nc:transfer=bt709:primaries=bt709:matrix=bt709:range=limited,format=yuv420p",
        s.tone_mapping_peak_nits
    )
}

fn hw_encoder(backend: &str, codec: &str) -> &'static str {
    match (backend, codec) {
        (st::HW_VAAPI, st::VIDEO_H264) => "h264_vaapi",
        (st::HW_VAAPI, st::VIDEO_HEVC) => "hevc_vaapi",
        (st::HW_VAAPI, st::VIDEO_AV1) => "av1_vaapi",
        (st::HW_NVENC, st::VIDEO_H264) => "h264_nvenc",
        (st::HW_NVENC, st::VIDEO_HEVC) => "hevc_nvenc",
        (st::HW_NVENC, st::VIDEO_AV1) => "av1_nvenc",
        (st::HW_QSV, st::VIDEO_H264) => "h264_qsv",
        (st::HW_QSV, st::VIDEO_HEVC) => "hevc_qsv",
        (st::HW_QSV, st::VIDEO_AV1) => "av1_qsv",
        (st::HW_AMF, st::VIDEO_H264) => "h264_amf",
        (st::HW_AMF, st::VIDEO_HEVC) => "hevc_amf",
        (st::HW_AMF, st::VIDEO_AV1) => "av1_amf",
        (st::HW_V4L2M2M, st::VIDEO_H264) => "h264_v4l2m2m",
        (st::HW_V4L2M2M, st::VIDEO_HEVC) => "hevc_v4l2m2m",
        (st::HW_VIDEOTOOLBOX, st::VIDEO_H264) => "h264_videotoolbox",
        (st::HW_VIDEOTOOLBOX, st::VIDEO_HEVC) => "hevc_videotoolbox",
        (st::HW_RKMPP, st::VIDEO_H264) => "h264_rkmpp",
        (st::HW_RKMPP, st::VIDEO_HEVC) => "hevc_rkmpp",
        _ => "",
    }
}

/// Picks the video encoder under the hardware policy. A configured but
/// unusable backend never fails the session: it falls back to software
/// with a visible note (D-031). Returns (encoder, backend, hw decode,
/// fallback note).
pub fn choose_encoder(
    s: &TranscodeSettings,
    codec: &str,
    caps: &Capabilities,
    video: &RawStream,
) -> Result<(String, String, bool, String), Error> {
    let mut backend =
        if s.hardware_acceleration == st::HW_NONE { String::new() } else { s.hardware_acceleration.clone() };
    let mut fallback = String::new();
    let source = video.codec_name.to_ascii_lowercase();
    let mut decode_allowed = s.hardware_decode_codecs.contains(&source);
    if decode_allowed && video.is_10bit() {
        match source.as_str() {
            "hevc" | "h265" if !s.hardware_decode_10bit_hevc => {
                decode_allowed = false;
                fallback = join_notes(&fallback, "10-bit HEVC decoding is disabled; decoded in software");
            }
            "vp9" if !s.hardware_decode_10bit_vp9 => {
                decode_allowed = false;
                fallback = join_notes(&fallback, "10-bit VP9 decoding is disabled; decoded in software");
            }
            _ => {}
        }
    }
    if !backend.is_empty() && s.hardware_encode {
        let want = hw_encoder(&backend, codec);
        if want.is_empty() {
            fallback = join_notes(&fallback, &format!("hardware {backend} cannot encode {codec}; used software"));
        } else if !caps.has_encoder(want) {
            fallback = join_notes(&fallback, &format!("hardware encoder {want} unavailable; used software"));
        } else if !caps.hardware_ok(&backend) {
            fallback = join_notes(&fallback, &format!("hardware backend {backend} failed its probe; used software"));
        } else {
            return Ok((want.to_string(), backend, decode_allowed, fallback));
        }
        backend.clear();
    }
    let candidates: &[&str] = match codec {
        st::VIDEO_H264 => &["libx264"],
        st::VIDEO_HEVC => &["libx265"],
        st::VIDEO_AV1 => &["libsvtav1", "libaom-av1"],
        _ => return Err(Error::invalid("video_codec must be h264, hevc or av1")),
    };
    // Decode-only hardware still offloads the decode half when the
    // operator enabled acceleration but not hardware encode.
    for c in candidates {
        if caps.has_encoder(c) {
            let hw_decode = !backend.is_empty() && decode_allowed && caps.hardware_ok(&backend);
            return Ok((c.to_string(), String::new(), hw_decode, fallback));
        }
    }
    Ok((candidates[0].to_string(), String::new(), false, fallback))
}

/// Extracts the tool names of `ffmpeg -encoders/-filters/-hwaccels`.
pub fn parse_ffmpeg_list(out: &str, set: &mut BTreeSet<String>) {
    for line in out.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        match fields.len() {
            0 => {}
            1 => {
                set.insert(fields[0].to_string());
            }
            _ => {
                let name = fields[1];
                if name == "=" || name.contains(':') {
                    continue;
                }
                set.insert(name.to_string());
            }
        }
    }
}

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
/// One small frame for the hardware probes. Not smaller: NVENC refuses
/// H.264 below 145x49 ("Invalid argument", measured on an RTX 3050 Ti
/// at 128x128), which read as "no NVENC" on a working card.
const TESTSRC: &str = "testsrc2=size=256x144:rate=5:duration=0.2";

fn ffmpeg_ok(ffmpeg: &str, args: &[&str]) -> bool {
    let mut c = Command::new(ffmpeg);
    c.args(["-hide_banner", "-loglevel", "error"]).args(args);
    exec::run(c, None, PROBE_TIMEOUT).is_ok_and(|o| o.status.success())
}

fn ffmpeg_list(ffmpeg: &str, flag: &str, set: &mut BTreeSet<String>) {
    let mut c = Command::new(ffmpeg);
    c.args(["-hide_banner", flag]);
    if let Ok(o) = exec::run(c, None, PROBE_TIMEOUT)
        && o.status.success()
    {
        parse_ffmpeg_list(&String::from_utf8_lossy(&o.stdout), set);
    }
}

/// The VA-API render node: configured, else the default.
pub fn hardware_device(s: &TranscodeSettings) -> String {
    let d = s.hardware_device.trim();
    if d.is_empty() { st::DEFAULT_HW_DEVICE.to_string() } else { d.to_string() }
}

/// The ffmpeg binary: configured, else `ffmpeg` on PATH.
pub fn ffmpeg_binary(s: &TranscodeSettings) -> String {
    if s.ffmpeg_path.trim().is_empty() { "ffmpeg".into() } else { s.ffmpeg_path.clone() }
}

fn hardware_candidate(backend: &str, caps: &Capabilities) -> bool {
    let acc = |n: &str| caps.hwaccels.contains(n);
    match backend {
        st::HW_VAAPI => caps.has_encoder("h264_vaapi") && acc("vaapi"),
        st::HW_NVENC => caps.has_encoder("h264_nvenc") && acc("cuda"),
        st::HW_QSV => caps.has_encoder("h264_qsv") && acc("qsv"),
        st::HW_AMF => caps.has_encoder("h264_amf") && acc("amf"),
        st::HW_V4L2M2M => caps.has_encoder("h264_v4l2m2m"),
        st::HW_VIDEOTOOLBOX => caps.has_encoder("h264_videotoolbox"),
        st::HW_RKMPP => caps.has_encoder("h264_rkmpp") && acc("rkmpp"),
        _ => false,
    }
}

fn probe_hardware(ffmpeg: &str, backend: &str, device: &str) -> bool {
    let va = format!("vaapi=va:{device}");
    let args: Vec<&str> = match backend {
        st::HW_VAAPI => vec![
            "-init_hw_device",
            &va,
            "-f",
            "lavfi",
            "-i",
            TESTSRC,
            "-vf",
            "format=nv12,hwupload",
            "-c:v",
            "h264_vaapi",
            "-frames:v",
            "1",
            "-f",
            "null",
            "-",
        ],
        st::HW_NVENC => {
            vec![
                "-init_hw_device",
                "cuda",
                "-f",
                "lavfi",
                "-i",
                TESTSRC,
                "-c:v",
                "h264_nvenc",
                "-frames:v",
                "1",
                "-f",
                "null",
                "-",
            ]
        }
        st::HW_QSV => {
            vec![
                "-init_hw_device",
                "qsv=hw",
                "-f",
                "lavfi",
                "-i",
                TESTSRC,
                "-c:v",
                "h264_qsv",
                "-frames:v",
                "1",
                "-f",
                "null",
                "-",
            ]
        }
        st::HW_AMF => vec!["-f", "lavfi", "-i", TESTSRC, "-c:v", "h264_amf", "-frames:v", "1", "-f", "null", "-"],
        st::HW_V4L2M2M => {
            vec!["-f", "lavfi", "-i", TESTSRC, "-c:v", "h264_v4l2m2m", "-frames:v", "1", "-f", "null", "-"]
        }
        st::HW_VIDEOTOOLBOX => {
            vec!["-f", "lavfi", "-i", TESTSRC, "-c:v", "h264_videotoolbox", "-frames:v", "1", "-f", "null", "-"]
        }
        st::HW_RKMPP => vec![
            "-init_hw_device",
            "rkmpp=hw",
            "-f",
            "lavfi",
            "-i",
            TESTSRC,
            "-vf",
            "format=nv12,hwupload",
            "-c:v",
            "h264_rkmpp",
            "-frames:v",
            "1",
            "-f",
            "null",
            "-",
        ],
        _ => return false,
    };
    // A driver can accept the graph and still encode nothing (seen with
    // VA-API under WSL): the probe must see one frame come out.
    let mut args: Vec<&str> = args;
    let n = args.len();
    args.truncate(n - 3);
    args.extend(["-f", "framecrc", "-"]);
    let mut c = Command::new(ffmpeg);
    c.args(["-hide_banner", "-loglevel", "error"]).args(&args);
    exec::run(c, None, PROBE_TIMEOUT)
        .is_ok_and(|o| o.status.success() && framecrc_has_data(&String::from_utf8_lossy(&o.stdout)))
}

/// A `framecrc` listing with at least one non-empty packet (the fifth
/// column is the packet size).
fn framecrc_has_data(out: &str) -> bool {
    out.lines()
        .filter(|l| !l.starts_with('#'))
        .any(|l| l.split(',').nth(4).and_then(|n| n.trim().parse::<u64>().ok()).is_some_and(|n| n > 0))
}

fn probe_tone_map(ffmpeg: &str, bt2390: bool) -> bool {
    let src = "testsrc2=size=64x64:rate=5:duration=0.2";
    if bt2390 {
        ffmpeg_ok(
            ffmpeg,
            &[
                "-init_hw_device",
                "vulkan=vk",
                "-f",
                "lavfi",
                "-i",
                src,
                "-vf",
                "libplacebo=tonemapping=bt.2390:colorspace=bt709:color_primaries=bt709:color_trc=bt709:range=tv:format=yuv420p",
                "-frames:v",
                "1",
                "-f",
                "null",
                "-",
            ],
        )
    } else {
        ffmpeg_ok(
            ffmpeg,
            &[
                "-f",
                "lavfi",
                "-i",
                src,
                "-vf",
                "format=gbrpf32le,zscale=transferin=smpte2084:primariesin=bt2020:matrixin=bt2020nc:transfer=linear:primaries=bt2020:matrix=bt2020nc,tonemap=tonemap=hable:desat=0,zscale=transferin=linear:primariesin=bt2020:matrixin=bt2020nc:transfer=bt709:primaries=bt709:matrix=bt709:range=limited,format=yuv420p",
                "-frames:v",
                "1",
                "-f",
                "null",
                "-",
            ],
        )
    }
}

/// libplacebo renders one frame: the filter exists in many builds whose
/// machine has no Vulkan device to run it on.
fn probe_effects(ffmpeg: &str) -> bool {
    ffmpeg_ok(
        ffmpeg,
        &[
            "-f",
            "lavfi",
            "-i",
            TESTSRC,
            "-vf",
            "libplacebo=w=512:h=288:format=yuv420p",
            "-frames:v",
            "1",
            "-f",
            "null",
            "-",
        ],
    )
}

/// Probes the ffmpeg build with no cache: encoder, filter and hwaccel
/// lists plus real execution probes for tone mapping and each hardware
/// backend the build could support. Discovery does not depend on the
/// current selection, so a software install can learn what it may pick.
pub fn probe_build(s: &TranscodeSettings) -> Capabilities {
    let ffmpeg = ffmpeg_binary(s);
    let mut caps = Capabilities { ffmpeg: ffmpeg.clone(), ..Capabilities::default() };
    ffmpeg_list(&ffmpeg, "-encoders", &mut caps.encoders);
    ffmpeg_list(&ffmpeg, "-filters", &mut caps.filters);
    ffmpeg_list(&ffmpeg, "-hwaccels", &mut caps.hwaccels);
    if caps.encoders.is_empty() {
        // No ffmpeg at all: every execution probe would fail too.
        for b in st::HW_BACKENDS {
            caps.hardware.insert(b.to_string(), false);
        }
        return caps;
    }
    caps.tone_map = probe_tone_map(&ffmpeg, false);
    caps.tone_map_2390 = caps.has_filter("libplacebo") && probe_tone_map(&ffmpeg, true);
    caps.effects = caps.has_filter("libplacebo") && probe_effects(&ffmpeg);
    let device = hardware_device(s);
    for b in st::HW_BACKENDS {
        let ok = hardware_candidate(b, &caps) && probe_hardware(&ffmpeg, b, &device);
        caps.hardware.insert(b.to_string(), ok);
    }
    caps
}

/// The encoders the settings UI reports, in Go's order.
pub const REPORTED_ENCODERS: [&str; 20] = [
    "libx264",
    "libx265",
    "libsvtav1",
    "libaom-av1",
    "h264_vaapi",
    "hevc_vaapi",
    "av1_vaapi",
    "h264_nvenc",
    "hevc_nvenc",
    "av1_nvenc",
    "h264_qsv",
    "hevc_qsv",
    "av1_qsv",
    "h264_amf",
    "hevc_amf",
    "av1_amf",
    "h264_v4l2m2m",
    "hevc_v4l2m2m",
    "h264_videotoolbox",
    "hevc_videotoolbox",
];

/// The public, path-free summary.
pub fn report(caps: &Capabilities) -> lain_api::TranscodeCapabilities {
    let enc: Vec<String> = REPORTED_ENCODERS.iter().filter(|e| caps.has_encoder(e)).map(|e| e.to_string()).collect();
    lain_api::TranscodeCapabilities {
        ffmpeg: caps.ffmpeg.clone(),
        encoders: if enc.is_empty() { None } else { Some(enc) },
        tone_mapping: caps.tone_map,
        tone_mapping_bt2390: caps.tone_map_2390,
        video_effects: caps.effects,
        hardware: caps.hardware.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcode::Policy;

    pub(crate) fn caps_full() -> Capabilities {
        let mut c = Capabilities::default();
        for e in ["libx264", "libx265", "libsvtav1", "h264_vaapi"] {
            c.encoders.insert(e.into());
        }
        for f in ["overlay", "subtitles", "bwdif"] {
            c.filters.insert(f.into());
        }
        c.tone_map = true;
        c
    }

    fn vs(codec: &str, pix: &str, w: i64, h: i64) -> RawStream {
        RawStream {
            index: 0,
            codec_type: "video".into(),
            codec_name: codec.into(),
            pix_fmt: pix.into(),
            width: w,
            height: h,
            r_frame_rate: "24/1".into(),
            ..Default::default()
        }
    }

    fn aus(idx: i64, codec: &str, ch: i64) -> RawStream {
        RawStream {
            index: idx,
            codec_type: "audio".into(),
            codec_name: codec.into(),
            channels: ch,
            ..Default::default()
        }
    }

    fn sub(idx: i64, codec: &str) -> RawStream {
        RawStream { index: idx, codec_type: "subtitle".into(), codec_name: codec.into(), ..Default::default() }
    }

    fn report(streams: Vec<RawStream>) -> RawReport {
        let mut r = RawReport { streams, ..Default::default() };
        r.format.duration = "10".into();
        r
    }

    fn spec(path: &str) -> SourceSpec {
        SourceSpec { path: path.into(), ..Default::default() }
    }

    fn req() -> V3Request {
        V3Request { settings: st::defaults(), ..V3Request::default() }
    }

    #[test]
    fn remuxes_web_safe_source() {
        let r = report(vec![vs("h264", "yuv420p", 1280, 720), aus(1, "aac", 2)]);
        let p = plan_v3(&spec("/m/a.mkv"), &req(), &caps_full(), &r).unwrap();
        assert!(p.copy_video && p.copy_audio);
        assert_eq!(p.method, "remux");
        assert_eq!(p.reasons, vec!["container (mkv)".to_string()]);
        assert_eq!(p.delivery, "hls");
    }

    #[test]
    fn encodes_incompatible_codecs() {
        let r = report(vec![vs("hevc", "yuv420p", 1920, 1080), aus(1, "flac", 2)]);
        let p = plan_v3(&spec("/m/a.mkv"), &req(), &caps_full(), &r).unwrap();
        assert!(!p.copy_video && !p.copy_audio);
        assert_eq!((p.encoder.as_str(), p.crf, p.preset.as_str()), ("libx264", 23, "veryfast"));
        assert!(p.reasons.contains(&"video codec (hevc)".to_string()));
        assert!(p.reasons.contains(&"audio codec (flac)".to_string()));
    }

    #[test]
    fn copies_hevc_for_a_client_that_decodes_it() {
        let mut rq = req();
        rq.video_copy_codecs = vec!["hevc".into()];
        let ten = report(vec![vs("hevc", "yuv420p10le", 1920, 1080), aus(1, "aac", 2)]);
        let p = plan_v3(&spec("/m/a.mkv"), &rq, &caps_full(), &ten).unwrap();
        assert!(p.copy_video && p.copy_audio && p.encoder.is_empty());
        assert_eq!((p.method.as_str(), p.video_codec.as_str()), ("remux", "hevc"));
        assert_eq!(p.reasons, vec!["container (mkv)".to_string()]);

        // The claim licenses nothing else: an H.264 source still copies as
        // H.264, 4:4:4 and HDR are still encoded, and so is a downscale.
        let h264 = report(vec![vs("h264", "yuv420p", 1280, 720), aus(1, "aac", 2)]);
        let p = plan_v3(&spec("/m/a.mkv"), &rq, &caps_full(), &h264).unwrap();
        assert!(p.copy_video && p.video_codec == "h264");
        let wide = report(vec![vs("hevc", "yuv444p", 1920, 1080)]);
        assert!(!plan_v3(&spec("/m/a.mkv"), &rq, &caps_full(), &wide).unwrap().copy_video);
        let mut hdr = vs("hevc", "yuv420p10le", 1920, 1080);
        hdr.color_transfer = "smpte2084".into();
        let p = plan_v3(&spec("/m/a.mkv"), &rq, &caps_full(), &report(vec![hdr])).unwrap();
        assert!(!p.copy_video && p.video_codec == "h264");
        let mut small = rq.clone();
        small.quality = "720p".into();
        assert!(!plan_v3(&spec("/m/a.mkv"), &small, &caps_full(), &ten).unwrap().copy_video);

        // Refused by the client's own flag, and by the MPEG-TS container.
        let mut no_copy = rq.clone();
        no_copy.allow_video_stream_copy = Some(false);
        assert!(!plan_v3(&spec("/m/a.mkv"), &no_copy, &caps_full(), &ten).unwrap().copy_video);
        let mut ts = rq.clone();
        ts.settings.hls_segment_container = st::HLS_TS.into();
        assert!(!plan_v3(&spec("/m/a.mkv"), &ts, &caps_full(), &ten).unwrap().copy_video);
        ts.delivery = "progressive".into();
        assert!(plan_v3(&spec("/m/a.mkv"), &ts, &caps_full(), &ten).unwrap().copy_video);
    }

    #[test]
    fn renders_a_video_effect_on_the_server() {
        let mut caps = caps_full();
        caps.effects = true;
        let mut rq = req();
        rq.video_effect = "anime4k-a".into();
        let src = report(vec![vs("h264", "yuv420p", 1280, 720), aus(1, "aac", 2)]);
        let p = plan_v3(&spec("/m/a.mkv"), &rq, &caps, &src).unwrap();
        // A copyable source is encoded: the effect is the reason.
        assert!(!p.copy_video && p.copy_audio && p.effect == "anime4k-a");
        assert_eq!((p.width, p.height), (1920, 1080));
        assert_eq!(p.filters, vec!["libplacebo=custom_shader_path=__EFFECT_FILE__:w=1920:h=1080:format=yuv420p"]);
        assert!(p.reasons.contains(&"video effect (anime4k-a)".to_string()));

        // A quality box bounds the output; burned text is drawn after it.
        let mut q = rq.clone();
        q.quality = "720p".into();
        q.subtitle_stream = Some(2);
        q.subtitle_mode = "burn".into();
        let subbed = report(vec![vs("h264", "yuv420p", 1920, 1080), aus(1, "aac", 2), sub(2, "ass")]);
        let p = plan_v3(&spec("/m/a.mkv"), &q, &caps, &subbed).unwrap();
        assert_eq!((p.width, p.height), (1280, 720));
        assert!(p.filters[0].starts_with("libplacebo=") && p.filters[1].starts_with("subtitles="), "{:?}", p.filters);

        // A server or a source that cannot render it still plays.
        let p = plan_v3(&spec("/m/a.mkv"), &rq, &caps_full(), &src).unwrap();
        assert!(p.effect.is_empty() && p.copy_video && p.fallback.contains("unavailable on this server"));
        assert_eq!((p.width, p.height), (1280, 720));
        let mut hdr = vs("hevc", "yuv420p10le", 1920, 1080);
        hdr.color_transfer = "smpte2084".into();
        let p = plan_v3(&spec("/m/a.mkv"), &rq, &caps, &report(vec![hdr])).unwrap();
        assert!(p.effect.is_empty() && p.fallback.contains("HDR"));
        let mut va = rq.clone();
        va.settings.hardware_acceleration = st::HW_VAAPI.into();
        va.settings.hardware_encode = true;
        caps.hardware.insert(st::HW_VAAPI.into(), true);
        let p = plan_v3(&spec("/m/a.mkv"), &va, &caps, &src).unwrap();
        assert!(p.effect.is_empty() && p.fallback.contains("VA-API") && (p.width, p.height) == (1280, 720));

        rq.video_effect = "sepia".into();
        assert_eq!(plan_v3(&spec("/m/a.mkv"), &rq, &caps, &src).unwrap_err().code, "invalid-message");
    }

    #[test]
    fn hdr_requires_tone_mapping() {
        let mut v = vs("hevc", "yuv420p10le", 1920, 1080);
        v.color_transfer = "smpte2084".into();
        let r = report(vec![v]);
        let mut rq = req();
        rq.settings.tone_mapping = false;
        assert_eq!(plan_v3(&spec("/a.mkv"), &rq, &caps_full(), &r).unwrap_err().code, "unsupported-media");
        let mut no_tone = caps_full();
        no_tone.tone_map = false;
        assert!(plan_v3(&spec("/a.mkv"), &req(), &no_tone, &r).unwrap_err().msg.contains("probe failed"));
        let p = plan_v3(&spec("/a.mkv"), &req(), &caps_full(), &r).unwrap();
        assert!(p.need_tone && p.fallback.contains("bt2390"));
        assert!(p.filters.iter().any(|f| f.contains("tonemap=tonemap=hable")));
    }

    #[test]
    fn deinterlace_follows_settings() {
        let mut v = vs("h264", "yuv420p", 720, 480);
        v.field_order = "tt".into();
        let r = report(vec![v]);
        let mut rq = req();
        rq.settings.deinterlace_method = "bwdif".into();
        rq.settings.deinterlace_double_rate = true;
        let p = plan_v3(&spec("/a.mp4"), &rq, &caps_full(), &r).unwrap();
        assert_eq!(p.filters, vec!["bwdif=mode=send_field".to_string()]);
        let mut no_bwdif = caps_full();
        no_bwdif.filters.remove("bwdif");
        let p = plan_v3(&spec("/a.mp4"), &rq, &no_bwdif, &r).unwrap();
        assert!(p.filters[0].starts_with("yadif") && p.fallback.contains("yadif"));
    }

    #[test]
    fn quality_caps_resolution_and_bitrate() {
        let r = report(vec![vs("h264", "yuv420p", 1920, 1080), aus(1, "aac", 2)]);
        let mut rq = req();
        rq.quality = "720p".into();
        let p = plan_v3(&spec("/a.mp4"), &rq, &caps_full(), &r).unwrap();
        assert!(!p.copy_video);
        assert_eq!((p.width, p.height, p.max_bitrate_kbps), (1280, 720, 4000));
        assert!(p.filters.iter().any(|f| f.starts_with("scale=w='min(iw,1280)'")));
        assert!(p.reasons.contains(&"quality (720p)".to_string()));
        rq.quality = "nope".into();
        assert_eq!(plan_v3(&spec("/a.mp4"), &rq, &caps_full(), &r).unwrap_err().msg, "unknown quality \"nope\"");
    }

    #[test]
    fn user_policy_is_enforced() {
        let r = report(vec![vs("hevc", "yuv420p", 640, 360), aus(1, "aac", 2)]);
        let mut rq = req();
        rq.policy = Some(Policy {
            allow_video_transcode: false,
            allow_audio_transcode: true,
            allow_remux: true,
            max_streams: 0,
        });
        assert_eq!(plan_v3(&spec("/a.mkv"), &rq, &caps_full(), &r).unwrap_err().code, "forbidden");
        let ok = report(vec![vs("h264", "yuv420p", 640, 360), aus(1, "aac", 2)]);
        rq.policy = Some(Policy {
            allow_video_transcode: true,
            allow_audio_transcode: true,
            allow_remux: false,
            max_streams: 0,
        });
        assert_eq!(
            plan_v3(&spec("/a.mkv"), &rq, &caps_full(), &ok).unwrap_err().msg,
            "remuxing is disabled for this user"
        );
    }

    #[test]
    fn subtitle_modes() {
        let r = report(vec![vs("h264", "yuv420p", 640, 360), sub(2, "subrip"), sub(3, "hdmv_pgs_subtitle")]);
        let mut rq = req();
        rq.subtitle_stream = Some(2);
        let p = plan_v3(&spec("/a.mkv"), &rq, &caps_full(), &r).unwrap();
        assert_eq!(p.subtitle_mode, "extract");
        assert!(p.copy_video);
        rq.subtitle_stream = Some(3);
        let p = plan_v3(&spec("/a.mkv"), &rq, &caps_full(), &r).unwrap();
        assert!(p.burn_image && !p.copy_video);
        assert_eq!(p.complex_filter, "[0:v:0][0:3]overlay:shortest=0[vout]");
        rq.subtitle_mode = "extract".into();
        assert_eq!(plan_v3(&spec("/a.mkv"), &rq, &caps_full(), &r).unwrap_err().code, "unsupported-media");
        rq.subtitle_mode = "burn".into();
        rq.subtitle_stream = Some(2);
        let p = plan_v3(&spec("/a.mkv"), &rq, &caps_full(), &r).unwrap();
        assert!(p.burn_text && p.filters[0].starts_with("subtitles=__SUBTITLE_FILE__"));
        rq.subtitle_stream = Some(9);
        assert_eq!(
            plan_v3(&spec("/a.mkv"), &rq, &caps_full(), &r).unwrap_err().msg,
            "subtitle_stream is not a subtitle track"
        );
        rq.subtitle_mode = "off".into();
        assert!(plan_v3(&spec("/a.mkv"), &rq, &caps_full(), &r).is_err(), "a bad index is refused before the mode");
    }

    #[test]
    fn stream_copy_flags_force_encode_with_reasons() {
        let r = report(vec![vs("h264", "yuv420p", 640, 360), aus(1, "aac", 2)]);
        let mut rq = req();
        rq.allow_video_stream_copy = Some(false);
        rq.allow_audio_stream_copy = Some(false);
        let p = plan_v3(&spec("/a.mp4"), &rq, &caps_full(), &r).unwrap();
        assert!(!p.copy_video && !p.copy_audio);
        assert!(p.reasons.contains(&"video stream copy disabled by the client".to_string()));
        assert!(p.reasons.contains(&"audio stream copy disabled by the client".to_string()));
    }

    #[test]
    fn downmix_and_nightmode_reason() {
        let r = report(vec![vs("h264", "yuv420p", 640, 360), aus(1, "aac", 8)]);
        let mut rq = req();
        rq.settings.downmix_stereo_algorithm = "nightmode".into();
        let p = plan_v3(&spec("/a.mp4"), &rq, &caps_full(), &r).unwrap();
        assert!(p.downmix && !p.copy_audio);
        assert!(p.reasons.contains(&"nightmode needs a 5.1 source".to_string()));
    }

    #[test]
    fn encoder_honors_hardware_policy() {
        let v = vs("h264", "yuv420p", 640, 360);
        let mut s = st::defaults();
        s.hardware_acceleration = "vaapi".into();
        s.hardware_encode = true;
        let mut caps = caps_full();
        let (e, b, _, f) = choose_encoder(&s, "h264", &caps, &v).unwrap();
        assert_eq!((e.as_str(), b.as_str()), ("libx264", ""));
        assert!(f.contains("failed its probe"), "{f}");
        caps.hardware.insert("vaapi".into(), true);
        let (e, b, hd, _) = choose_encoder(&s, "h264", &caps, &v).unwrap();
        assert_eq!((e.as_str(), b.as_str(), hd), ("h264_vaapi", "vaapi", true));
        s.hardware_acceleration = "videotoolbox".into();
        let (_, _, _, f) = choose_encoder(&s, "av1", &caps, &v).unwrap();
        assert!(f.contains("cannot encode av1"), "{f}");
        // Decode-only acceleration.
        s.hardware_acceleration = "vaapi".into();
        s.hardware_encode = false;
        let (e, b, hd, _) = choose_encoder(&s, "h264", &caps, &v).unwrap();
        assert_eq!((e.as_str(), b.as_str(), hd), ("libx264", "", true));
        let ten = vs("hevc", "yuv420p10le", 640, 360);
        let (_, _, hd, f) = choose_encoder(&s, "h264", &caps, &ten).unwrap();
        assert!(!hd && f.contains("10-bit HEVC"));
    }

    #[test]
    fn hardware_probe_needs_a_real_packet() {
        assert!(!framecrc_has_data("#tb 0: 1/5\n0,          0,          0,        1,        0, 0x00000000\n"));
        assert!(framecrc_has_data("#tb 0: 1/5\n0,          0,          0,        1,     1234, 0x1a2b3c4d\n"));
        assert!(!framecrc_has_data(""));
    }

    #[test]
    fn filter_helpers() {
        assert_eq!(scale_filter(0, 720), "scale=-2:'min(ih,720)'");
        assert_eq!(scale_filter(1280, 0), "scale='min(iw,1280)':-2");
        assert!(
            tone_map_chain(&st::defaults(), &Capabilities { tone_map_2390: true, ..Default::default() })
                .starts_with("libplacebo")
        );
        assert!(tone_map_chain(&st::defaults(), &Capabilities::default()).contains("npl=100"));
        assert_eq!(sanitize_font_name(" Noto';Sans:x "), "NotoSansx");
        let mut set = BTreeSet::new();
        parse_ffmpeg_list(
            " V....D libx264  H.264\n ------\n A....D aac  AAC\nHardware acceleration methods:\nvaapi\n = x",
            &mut set,
        );
        assert!(set.contains("libx264") && set.contains("aac") && set.contains("vaapi") && set.contains("------"));
        assert!(!set.contains("="));
    }
}
