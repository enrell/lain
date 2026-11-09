//! ffmpeg argv for a plan (Go's `transcode/encode.go`), and the v1
//! synchronous conversion argv (`ffmpeg.go`).

use std::path::Path;

use lain_probe::RawReport;

use crate::settings as st;
use crate::transcode::policy::{EncodePlan, join_notes};

/// Replaced with the escaped pre-extracted subtitle file.
pub const SUBTITLE_PLACEHOLDER: &str = "__SUBTITLE_FILE__";
/// Replaced with the escaped fallback font directory.
pub const FONTS_PLACEHOLDER: &str = "__FONTS_DIR__";

/// Escapes a filesystem path for ffmpeg filter syntax.
pub fn escape_filter_path(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    for c in p.chars() {
        if matches!(c, '\\' | ':' | '\'' | '[' | ']' | ',' | ';') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn software_encoder_for(codec: &str, caps_has_svt: bool) -> &'static str {
    match codec {
        st::VIDEO_HEVC => "libx265",
        st::VIDEO_AV1 if caps_has_svt => "libsvtav1",
        st::VIDEO_AV1 => "libaom-av1",
        _ => "libx264",
    }
}

fn nvenc_preset(p: &str) -> &'static str {
    match p {
        "ultrafast" | "superfast" | "veryfast" => "p1",
        "faster" | "fast" => "p2",
        "slow" => "p5",
        "slower" => "p6",
        "veryslow" => "p7",
        _ => "p4",
    }
}

fn svt_av1_preset(p: &str) -> &'static str {
    match p {
        "ultrafast" => "13",
        "superfast" => "12",
        "veryfast" => "10",
        "faster" => "9",
        "fast" => "8",
        "medium" => "6",
        "slow" => "4",
        "slower" => "2",
        "veryslow" => "0",
        _ => "8",
    }
}

fn libaom_cpu_used(p: &str) -> &'static str {
    match p {
        "ultrafast" | "superfast" => "8",
        "veryfast" => "7",
        "faster" | "fast" => "6",
        "medium" => "5",
        "slow" => "4",
        "slower" => "3",
        "veryslow" => "2",
        _ => "8",
    }
}

/// lain's own dialogue-lifting 5.1 to stereo matrix (not Jellyfin's).
pub const NIGHTMODE_PAN: &str = "pan=stereo|c0=0.4*c0+0.4*c1+0.8*c2+0.2*c4|c1=0.4*c0+0.4*c1+0.8*c2+0.2*c5";

fn s(v: &str) -> String {
    v.to_string()
}

/// Formats like Go's `strconv.FormatFloat(f, 'f', -1, 64)`.
fn fmt_float(f: f64) -> String {
    let mut t = format!("{f}");
    if t.contains('e') {
        t = format!("{f:.6}");
    }
    t
}

impl EncodePlan {
    /// Offloads decoding when allowed; frames stay on the CPU unless the
    /// encoder needs otherwise.
    fn hwaccel_input_args(&self) -> Vec<String> {
        if !self.hw_decode {
            return Vec::new();
        }
        let backend = if self.hw_backend.is_empty() {
            self.settings.hardware_acceleration.as_str()
        } else {
            self.hw_backend.as_str()
        };
        match backend {
            // Encoding uploads through the named device (see
            // `hw_device_args`); decode-only opens the node directly.
            st::HW_VAAPI if self.hw_backend == st::HW_VAAPI => {
                vec![s("-hwaccel"), s("vaapi"), s("-hwaccel_device"), s("va")]
            }
            st::HW_VAAPI => {
                vec![s("-hwaccel"), s("vaapi"), s("-hwaccel_device"), super::policy::hardware_device(&self.settings)]
            }
            st::HW_NVENC => vec![s("-hwaccel"), s("cuda")],
            st::HW_QSV => vec![s("-hwaccel"), s("qsv")],
            st::HW_VIDEOTOOLBOX => vec![s("-hwaccel"), s("videotoolbox")],
            st::HW_RKMPP => vec![s("-hwaccel"), s("rkmpp")],
            _ => Vec::new(),
        }
    }

    /// The complete argv for one plan; `output` is the progressive temp
    /// file or the HLS raw playlist.
    pub fn ffmpeg_args(&self, output: &Path, subtitle_file: &Path, progress: bool) -> Vec<String> {
        let mut a = vec![s("-hide_banner"), s("-loglevel"), s("error"), s("-nostdin")];
        if progress {
            a.extend([s("-progress"), s("pipe:1"), s("-nostats")]);
        }
        // A VA-API encode uploads frames in the filter graph (`hwupload`),
        // which needs a device of its own: `-hwaccel_device` alone gives
        // the filters none ("A hardware device reference is required"),
        // so Go's argv always fell back to software.
        if self.hw_backend == st::HW_VAAPI && !self.copy_video {
            a.extend([
                s("-init_hw_device"),
                format!("vaapi=va:{}", super::policy::hardware_device(&self.settings)),
                s("-filter_hw_device"),
                s("va"),
            ]);
        }
        a.extend(self.hwaccel_input_args());
        if self.spec.start_sec > 0.0 {
            a.extend([s("-ss"), format!("{:.3}", self.spec.start_sec)]);
        }
        a.extend([s("-i"), self.spec.path.clone()]);
        let sub = escape_filter_path(&subtitle_file.to_string_lossy());
        let fonts = escape_filter_path(&self.settings.fallback_font_path);
        let fill = |chain: &str| chain.replace(SUBTITLE_PLACEHOLDER, &sub).replace(FONTS_PLACEHOLDER, &fonts);
        if !self.complex_filter.is_empty() {
            a.extend([s("-filter_complex"), fill(&self.complex_filter), s("-map"), s("[vout]")]);
        } else {
            a.extend([s("-map"), format!("0:{}", self.video.index)]);
            if !self.filters.is_empty() {
                a.extend([s("-vf"), fill(&self.filters.join(","))]);
            }
        }
        match &self.audio {
            Some(au) => a.extend([s("-map"), format!("0:{}", au.index)]),
            None => a.push(s("-an")),
        }
        if self.subtitle_mode != st::SUB_BURN {
            a.push(s("-sn"));
        }
        if self.copy_video {
            a.extend([s("-c:v"), s("copy")]);
        } else {
            a.extend([s("-c:v"), self.encoder.clone()]);
            a.extend(self.video_quality_args());
            if !self.encoder.ends_with("_vaapi") && !self.encoder.ends_with("_videotoolbox") {
                a.extend([s("-pix_fmt"), s("yuv420p")]);
            }
        }
        if self.audio.is_some() {
            if self.copy_audio {
                a.extend([s("-c:a"), s("copy")]);
            } else {
                a.extend([s("-c:a"), self.audio_codec.clone()]);
                a.extend(self.audio_quality_args());
                if self.downmix {
                    a.extend([s("-ac"), s("2")]);
                }
                let af = self.audio_filters();
                if !af.is_empty() {
                    a.extend([s("-af"), af]);
                }
            }
        }
        if self.settings.thread_count > 0 {
            a.extend([s("-threads"), self.settings.thread_count.to_string()]);
        }
        if self.settings.muxing_queue_size > 0 {
            a.extend([s("-max_muxing_queue_size"), self.settings.muxing_queue_size.to_string()]);
        }
        if self.is_hls() {
            let dir = output.parent().unwrap_or(Path::new("."));
            a.extend(self.hls_args(dir, output));
        } else {
            a.extend([
                s("-movflags"),
                s("+faststart"),
                s("-f"),
                s("mp4"),
                s("-y"),
                output.to_string_lossy().into_owned(),
            ]);
        }
        a
    }

    /// The HLS muxer arguments. Encodes pin the GOP to the segment length
    /// so segments cut where the playlist says; stream copy cannot move
    /// keyframes, and the playlist then reports the real target duration.
    fn hls_args(&self, dir: &Path, output: &Path) -> Vec<String> {
        let secs = self.settings.hls_segment_seconds;
        let mut a = vec![
            s("-f"),
            s("hls"),
            s("-hls_time"),
            secs.to_string(),
            s("-hls_playlist_type"),
            s("event"),
            s("-hls_list_size"),
            s("0"),
        ];
        if self.settings.hls_segment_container == st::HLS_TS {
            a.extend([
                s("-hls_segment_type"),
                s("mpegts"),
                s("-hls_segment_filename"),
                dir.join("seg%05d.ts").to_string_lossy().into_owned(),
            ]);
        } else {
            a.extend([
                s("-hls_segment_type"),
                s("fmp4"),
                s("-hls_fmp4_init_filename"),
                s("init.mp4"),
                s("-hls_segment_filename"),
                dir.join("seg%05d.m4s").to_string_lossy().into_owned(),
            ]);
        }
        a.extend([s("-hls_flags"), s("independent_segments+temp_file")]);
        if !self.copy_video {
            a.extend([s("-force_key_frames"), format!("expr:gte(t,n_forced*{secs})")]);
            let fps = self.video.frame_rate();
            if fps > 0.0 {
                let gop = ((fps * secs as f64 + 0.5) as i64).max(1);
                a.extend([s("-g"), gop.to_string(), s("-keyint_min"), gop.to_string()]);
                if self.encoder == "libx264" || self.encoder == "libx265" {
                    a.extend([s("-sc_threshold"), s("0")]);
                }
            }
        }
        a.push(output.to_string_lossy().into_owned());
        a
    }

    fn video_quality_args(&self) -> Vec<String> {
        let crf = self.crf.to_string();
        let max = self.max_bitrate_kbps;
        let cap = || -> Vec<String> {
            if max <= 0 {
                return Vec::new();
            }
            vec![s("-maxrate"), format!("{max}k"), s("-bufsize"), format!("{}k", max * 2)]
        };
        let e = self.encoder.as_str();
        let mut a = if e.ends_with("_vaapi") {
            vec![s("-qp"), crf]
        } else if e.ends_with("_nvenc") {
            vec![s("-preset"), s(nvenc_preset(&self.preset)), s("-cq"), crf, s("-rc"), s("vbr")]
        } else if e.ends_with("_qsv") {
            let mut v = vec![s("-global_quality"), crf];
            if self.settings.hardware_low_power {
                v.extend([s("-low_power"), s("1")]);
            }
            v
        } else if e.ends_with("_amf") {
            vec![s("-quality"), s("speed"), s("-qp_i"), crf.clone(), s("-qp_p"), crf]
        } else if e.ends_with("_v4l2m2m") {
            let rate = if max > 0 { format!("{max}k") } else { s("4000k") };
            return vec![s("-b:v"), rate];
        } else if e.ends_with("_videotoolbox") {
            vec![s("-q:v"), (100 - self.crf * 2).max(0).to_string()]
        } else if e == "libsvtav1" {
            vec![s("-preset"), s(svt_av1_preset(&self.preset)), s("-crf"), crf]
        } else if e == "libaom-av1" {
            let cpu = s(libaom_cpu_used(&self.preset));
            if max > 0 {
                let rate = format!("{max}k");
                return vec![
                    s("-cpu-used"),
                    cpu,
                    s("-crf"),
                    crf,
                    s("-b:v"),
                    rate.clone(),
                    s("-maxrate"),
                    rate,
                    s("-bufsize"),
                    format!("{}k", max * 2),
                ];
            }
            return vec![s("-cpu-used"), cpu, s("-crf"), crf, s("-b:v"), s("0")];
        } else {
            vec![s("-preset"), self.preset.clone(), s("-crf"), crf]
        };
        a.extend(cap());
        a
    }

    fn audio_quality_args(&self) -> Vec<String> {
        if self.settings.audio_vbr && self.audio_codec == st::AUDIO_AAC {
            return vec![s("-q:a"), fmt_float(st::audio_vbr_quality(self.audio_bitrate_kbps))];
        }
        vec![s("-b:a"), format!("{}k", self.audio_bitrate_kbps)]
    }

    /// The downmix gain and, when chosen and the source is 5.1, lain's
    /// nightmode matrix.
    fn audio_filters(&self) -> String {
        let mut chain = Vec::new();
        if self.downmix
            && self.settings.downmix_stereo_algorithm == st::DOWNMIX_NIGHTMODE
            && self.audio.as_ref().is_some_and(|a| a.channels == 6)
        {
            chain.push(NIGHTMODE_PAN.to_string());
        }
        let boost = self.settings.downmix_audio_boost;
        if self.downmix && boost > 0.0 && boost != 1.0 {
            chain.push(format!("volume={boost:.2}"));
        }
        chain.join(",")
    }

    /// The same session in software: encoder and hwaccel inputs change,
    /// the hardware upload tokens leave the filters.
    pub fn software_fallback(&self, has_svt: bool) -> EncodePlan {
        let mut out = self.clone();
        out.encoder = software_encoder_for(&self.video_codec, has_svt).to_string();
        out.hw_backend.clear();
        out.hw_decode = false;
        out.filters.retain(|f| f != "hwupload" && f != "format=nv12");
        out.complex_filter =
            out.complex_filter.replace(",format=nv12,hwupload", "").replace("format=nv12,hwupload", "");
        out.fallback = join_notes(&out.fallback, "hardware encode failed; used software");
        out
    }
}

/// The v1 (`mp4-compat-v1`) conversion: first video, every audio;
/// copied when the source already is H.264/AAC.
pub fn legacy_args(path: &str, report: Option<&RawReport>, tmp: &Path, progress: bool) -> (Vec<String>, String) {
    let (copy_v, copy_a) = report.map(legacy_copy).unwrap_or((false, false));
    let mut a = Vec::new();
    if progress {
        a.extend([s("-progress"), s("pipe:1"), s("-nostats")]);
    }
    a.extend([s("-hide_banner"), s("-loglevel"), s("error"), s("-nostdin"), s("-i"), path.to_string()]);
    a.extend([s("-map"), s("0:v:0"), s("-map"), s("0:a?")]);
    if copy_v {
        a.extend([s("-c:v"), s("copy")]);
    } else {
        a.extend([
            s("-c:v"),
            s("libx264"),
            s("-preset"),
            s("veryfast"),
            s("-crf"),
            s("23"),
            s("-pix_fmt"),
            s("yuv420p"),
        ]);
    }
    if copy_a {
        a.extend([s("-c:a"), s("copy")]);
    } else {
        a.extend([s("-c:a"), s("aac"), s("-b:a"), s("160k")]);
    }
    a.extend([s("-movflags"), s("+faststart"), s("-f"), s("mp4"), s("-y"), tmp.to_string_lossy().into_owned()]);
    let method = if copy_v && copy_a { "remux" } else { "transcode" };
    (a, method.to_string())
}

fn legacy_copy(r: &RawReport) -> (bool, bool) {
    let (mut video, mut copy_audio) = (0, true);
    for st in &r.streams {
        match st.codec_type.as_str() {
            "video" => {
                if video == 0 && st.codec_name != "h264" {
                    return (false, false);
                }
                video += 1;
            }
            "audio" if st.codec_name != "aac" => copy_audio = false,
            _ => {}
        }
    }
    if video == 0 {
        return (false, false);
    }
    (true, copy_audio)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcode::SourceSpec;
    use lain_probe::RawStream;

    fn plan() -> EncodePlan {
        EncodePlan {
            spec: SourceSpec { path: "/m/a.mkv".into(), ..Default::default() },
            delivery: "progressive".into(),
            settings: st::defaults(),
            video: RawStream {
                index: 0,
                codec_type: "video".into(),
                codec_name: "hevc".into(),
                r_frame_rate: "24/1".into(),
                ..Default::default()
            },
            audio: Some(RawStream {
                index: 1,
                codec_type: "audio".into(),
                codec_name: "ac3".into(),
                channels: 6,
                ..Default::default()
            }),
            subtitle_mode: "off".into(),
            video_codec: "h264".into(),
            audio_codec: "aac".into(),
            encoder: "libx264".into(),
            crf: 23,
            preset: "veryfast".into(),
            audio_bitrate_kbps: 160,
            ..Default::default()
        }
    }

    #[test]
    fn progressive_argv() {
        let mut p = plan();
        p.downmix = true;
        p.settings.thread_count = 4;
        let a = p.ffmpeg_args(Path::new("/c/out.mp4.tmp"), Path::new("/c/s.ass"), true).join(" ");
        assert!(
            a.starts_with(
                "-hide_banner -loglevel error -nostdin -progress pipe:1 -nostats -i /m/a.mkv -map 0:0 -map 0:1 -sn"
            ),
            "{a}"
        );
        assert!(a.contains("-c:v libx264 -preset veryfast -crf 23 -pix_fmt yuv420p"), "{a}");
        assert!(a.contains("-c:a aac -b:a 160k -ac 2 -af volume=2.00 -threads 4 -max_muxing_queue_size 2048"), "{a}");
        assert!(a.ends_with("-movflags +faststart -f mp4 -y /c/out.mp4.tmp"), "{a}");
    }

    #[test]
    fn hls_argv_pins_the_gop() {
        let mut p = plan();
        p.delivery = "hls".into();
        let a = p.ffmpeg_args(Path::new("/c/s/raw.m3u8"), Path::new(""), false).join(" ");
        assert!(
            a.contains("-f hls -hls_time 6 -hls_playlist_type event -hls_list_size 0 -hls_segment_type fmp4"),
            "{a}"
        );
        assert!(
            a.contains("-force_key_frames expr:gte(t,n_forced*6) -g 144 -keyint_min 144 -sc_threshold 0 /c/s/raw.m3u8"),
            "{a}"
        );
        p.settings.hls_segment_container = "mpegts".into();
        p.copy_video = true;
        let a = p.ffmpeg_args(Path::new("/c/s/raw.m3u8"), Path::new(""), false).join(" ");
        assert!(
            a.contains("-hls_segment_type mpegts") && a.contains("seg%05d.ts") && !a.contains("-force_key_frames"),
            "{a}"
        );
    }

    #[test]
    fn encoder_quality_args() {
        let mut p = plan();
        p.max_bitrate_kbps = 3000;
        for (enc, want) in [
            ("libx264", "-preset veryfast -crf 23 -maxrate 3000k -bufsize 6000k"),
            ("h264_nvenc", "-preset p1 -cq 23 -rc vbr -maxrate 3000k"),
            ("h264_vaapi", "-qp 23 -maxrate 3000k"),
            ("h264_v4l2m2m", "-b:v 3000k"),
            ("h264_videotoolbox", "-q:v 54"),
            ("libsvtav1", "-preset 10 -crf 23"),
            ("libaom-av1", "-cpu-used 7 -crf 23 -b:v 3000k -maxrate 3000k -bufsize 6000k"),
        ] {
            p.encoder = enc.into();
            let a = p.video_quality_args().join(" ");
            assert!(a.starts_with(want), "{enc}: {a}");
        }
        p.encoder = "h264_qsv".into();
        p.settings.hardware_low_power = true;
        assert!(p.video_quality_args().join(" ").contains("-low_power 1"));
    }

    #[test]
    fn audio_vbr_and_nightmode() {
        let mut p = plan();
        p.settings.audio_vbr = true;
        assert_eq!(p.audio_quality_args(), vec!["-q:a".to_string(), "1.5".to_string()]);
        p.downmix = true;
        p.settings.downmix_stereo_algorithm = "nightmode".into();
        assert!(p.audio_filters().starts_with("pan=stereo|"));
    }

    #[test]
    fn vaapi_encode_names_its_device() {
        let mut p = plan();
        p.encoder = "h264_vaapi".into();
        p.hw_backend = "vaapi".into();
        p.hw_decode = true;
        let a = p.ffmpeg_args(Path::new("/c/o.mp4"), Path::new(""), false).join(" ");
        assert!(
            a.contains(
                "-init_hw_device vaapi=va:/dev/dri/renderD128 -filter_hw_device va -hwaccel vaapi -hwaccel_device va -i"
            ),
            "{a}"
        );
        p.hw_backend.clear();
        p.encoder = "libx264".into();
        p.settings.hardware_acceleration = "vaapi".into();
        let a = p.ffmpeg_args(Path::new("/c/o.mp4"), Path::new(""), false).join(" ");
        assert!(
            a.contains("-hwaccel vaapi -hwaccel_device /dev/dri/renderD128 -i") && !a.contains("init_hw_device"),
            "{a}"
        );
    }

    #[test]
    fn hardware_fallback_strips_upload() {
        let mut p = plan();
        p.encoder = "h264_vaapi".into();
        p.hw_backend = "vaapi".into();
        p.hw_decode = true;
        p.filters = vec!["yadif".into(), "format=nv12".into(), "hwupload".into()];
        p.complex_filter = "[0:v:0]yadif,format=nv12,hwupload[base][0:3]overlay:shortest=0[vout]".into();
        let f = p.software_fallback(true);
        assert_eq!((f.encoder.as_str(), f.hw_backend.as_str(), f.hw_decode), ("libx264", "", false));
        assert_eq!(f.filters, vec!["yadif".to_string()]);
        assert!(!f.complex_filter.contains("hwupload"));
        assert!(f.fallback.contains("hardware encode failed"));
    }

    #[test]
    fn filter_path_escape() {
        assert_eq!(escape_filter_path(r"C:\a'b[c],d;e"), r"C\:\\a\'b\[c\]\,d\;e");
    }

    #[test]
    fn legacy_copy_rules() {
        let v = |c: &str| RawStream { codec_type: "video".into(), codec_name: c.into(), ..Default::default() };
        let a = |c: &str| RawStream { codec_type: "audio".into(), codec_name: c.into(), ..Default::default() };
        let r = |s: Vec<RawStream>| RawReport { streams: s, ..Default::default() };
        assert_eq!(legacy_copy(&r(vec![v("h264"), a("aac")])), (true, true));
        assert_eq!(legacy_copy(&r(vec![v("h264"), a("ac3")])), (true, false));
        assert_eq!(legacy_copy(&r(vec![v("hevc"), a("aac")])), (false, false));
        assert_eq!(legacy_args("/a", None, Path::new("/t"), false).1, "transcode");
    }
}
