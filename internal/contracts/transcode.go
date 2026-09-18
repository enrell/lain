// Transcode v3 contract: session options and runtime settings for the
// full browser pipeline (D-042..D-045).
//
// V3 is additive: `lain.playback.transcode@1` (synchronous MP4) and
// `@2` (asynchronous inspect/start/status/resolve) keep their frozen
// payloads. V3 carries the operator settings inside the request — the
// plugin never reads a bucket or holds HTTP state (D-006/D-045) — and
// reports pipeline facts (encoder, hardware, fallback, reasons) that
// the UI can show honestly.
package contracts

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"strings"
)

// Delivery modes for one session.
const (
	// TranscodeDeliveryProgressive is a complete MP4 served with Range
	// (the retained @1/@2 behavior).
	TranscodeDeliveryProgressive = "progressive"
	// TranscodeDeliveryHLS is fMP4 HLS: playable while ffmpeg still runs.
	TranscodeDeliveryHLS = "hls"
	// TranscodeDeliverySubtitle is not a transcode at all: it is a
	// subtitle sidecar extracted on demand for direct play (Jellyfin's
	// "allow subtitle extraction on the fly").
	TranscodeDeliverySubtitle = "subtitle"
)

// HLS segment containers (Jellyfin's HLS segment container).
const (
	// HLSSegmentFMP4 is CMAF fragmented MP4 with an init segment.
	HLSSegmentFMP4 = "fmp4"
	// HLSSegmentTS is classic MPEG-TS segments with no init segment.
	HLSSegmentTS = "mpegts"
)

// V3 actions beyond the shared inspect/start/status/resolve set.
const (
	// TranscodeCancelAction stops a running session and drops its
	// artifacts (admin stop, or the player leaving for good).
	TranscodeCancelAction = "cancel"
	// TranscodePositionAction reports which HLS segment the client last
	// fetched, which drives throttling and idle cleanup.
	TranscodePositionAction = "position"
	// TranscodeListAction returns the active sessions (admin view).
	TranscodeListAction = "list"
	// TranscodeSubtitleAction extracts one subtitle stream to a cached
	// WebVTT sidecar without starting a transcode.
	TranscodeSubtitleAction = "subtitles"
)

// Subtitle handling modes.
const (
	// SubtitleModeAuto extracts convertible text subtitles as WebVTT
	// and burns in image-based tracks that cannot become text.
	SubtitleModeAuto = "auto"
	// SubtitleModeExtract only ever extracts; an image track fails.
	SubtitleModeExtract = "extract"
	// SubtitleModeBurn renders the selected track into the video.
	SubtitleModeBurn = "burn"
	// SubtitleModeOff selects no subtitle.
	SubtitleModeOff = "off"
)

// Video output codecs. HEVC/AV1 need explicit operator permission.
const (
	VideoCodecH264 = "h264"
	VideoCodecHEVC = "hevc"
	VideoCodecAV1  = "av1"
)

// Audio output codecs that survive in MP4.
const (
	AudioCodecAAC  = "aac"
	AudioCodecAC3  = "ac3"
	AudioCodecEAC3 = "eac3"
)

// Hardware acceleration backends, in probe order.
const (
	HWNone         = "none"
	HWVAAPI        = "vaapi"
	HWNVENC        = "nvenc"
	HWQSV          = "qsv"
	HWAMF          = "amf"
	HWV4L2M2M      = "v4l2m2m"
	HWVideoToolbox = "videotoolbox"
	// Rockchip's media processing pipeline (Jellyfin's RKMPP method).
	HWRKMPP = "rkmpp"
)

// DefaultHardwareDevice is the render node VA-API uses when the operator
// does not name one (Jellyfin's VA-API device setting).
const DefaultHardwareDevice = "/dev/dri/renderD128"

// HardwareDecodeCodecs is the fixed set of codecs with hardware
// decoding support (Jellyfin parity); settings select a subset.
var HardwareDecodeCodecs = []string{"h264", "hevc", "mpeg2", "vc1", "vp8", "vp9", "av1"}

// Tone-mapping algorithms. BT.2390 needs libplacebo/Vulkan; the rest
// run through the zscale+tonemap chain. Availability is probed, and an
// unavailable choice degrades to a visible fallback, never silently.
const (
	ToneMapBT2390   = "bt2390"
	ToneMapHable    = "hable"
	ToneMapReinhard = "reinhard"
	ToneMapMobius   = "mobius"
	ToneMapClip     = "clip"
	ToneMapLinear   = "linear"
)

// Tone-map apply modes.
const (
	ToneMapModeAuto   = "auto"   // only when the source is HDR
	ToneMapModeAlways = "always" // even for SDR sources
	ToneMapModeNever  = "never"
)

// Deinterlace modes.
const (
	DeinterlaceAuto = "auto"
	DeinterlaceOff  = "off"
)

// Deinterlace filters, in probe order. bwdif is the better filter where
// the build has it; yadif is the fallback every build ships.
const (
	DeinterlaceYadif = "yadif"
	DeinterlaceBwdif = "bwdif"
)

// Downmix stereo algorithms. `none` is ffmpeg's own downmix; `nightmode`
// is lain's own documented matrix (center-dialogue lift, reduced
// surround and LFE) and is not Jellyfin's Dave750/NightmodeDialogue
// coefficients.
const (
	DownmixNone      = "none"
	DownmixNightmode = "nightmode"
)

// AudioVBRQuality maps a target audio bitrate to ffmpeg's native AAC VBR
// quality step (-q:a, 0.1..2). The mapping is documented so an operator
// can predict what `audio_vbr` does instead of guessing.
func AudioVBRQuality(bitrateKbps int) float64 {
	switch {
	case bitrateKbps >= 192:
		return 2
	case bitrateKbps >= 128:
		return 1.5
	case bitrateKbps >= 96:
		return 1
	case bitrateKbps >= 64:
		return 0.5
	default:
		return 0.1
	}
}

// EncoderPresets is the accepted x264/x265 preset ladder.
var EncoderPresets = []string{
	"ultrafast", "superfast", "veryfast", "faster", "fast",
	"medium", "slow", "slower", "veryslow",
}

// TranscodeQuality is one ladder entry the player can request.
type TranscodeQuality struct {
	Name        string `json:"name"`
	MaxWidth    int    `json:"max_width,omitempty"`
	MaxHeight   int    `json:"max_height,omitempty"`
	BitrateKbps int    `json:"bitrate_kbps,omitempty"`
}

// TranscodeSettings is the operator policy for the transcode pipeline.
// It travels inside the v3 request (D-045): the gateway resolves it
// from the meta bucket and the plugin stays free of storage.
type TranscodeSettings struct {
	// Delivery and session behavior.
	DefaultDelivery   string `json:"default_delivery"`    // progressive|hls
	HLSSegmentSeconds int    `json:"hls_segment_seconds"` // 1..30
	// HLSSegmentContainer is the HLS segment format: fmp4 (CMAF, with an
	// init segment) or mpegts (classic MPEG-TS segments).
	HLSSegmentContainer string `json:"hls_segment_container"` // fmp4|mpegts
	Throttle            bool   `json:"throttle"`              // pause ffmpeg when the client is far behind
	ThrottleAheadSec    int    `json:"throttle_ahead_sec"`    // produced-ahead budget
	SegmentDeletion     bool   `json:"segment_deletion"`      // drop consumed HLS segments
	SegmentKeepSec      int    `json:"segment_keep_sec"`      // retained window behind the client
	IdleTimeoutSec      int    `json:"idle_timeout_sec"`      // stop abandoned sessions

	// Resources.
	CacheBytes    int64 `json:"cache_bytes"`
	QueueSize     int   `json:"queue_size"`
	MaxConcurrent int   `json:"max_concurrent"` // simultaneous running jobs
	// ThreadCount pins ffmpeg's encoder threads (Jellyfin's transcoding
	// thread count); 0 leaves the choice to ffmpeg.
	ThreadCount int `json:"thread_count"`
	// MuxingQueueSize bounds ffmpeg's output packet queue (Jellyfin's max
	// muxing queue size) so a slow client cannot abort the muxer.
	MuxingQueueSize int `json:"max_muxing_queue_size"`
	// TranscodeTempPath moves artifacts to another volume; empty keeps
	// them under the data dir. Sidecars always stay with the data dir.
	TranscodeTempPath string `json:"transcode_temp_path"`
	// RemoteBitrateLimitKbps is the server-wide cap; the effective cap is
	// the tighter of this and the per-user limit.
	RemoteBitrateLimitKbps int `json:"remote_bitrate_limit_kbps"`

	// Encoder policy. The per-codec values win; zero/empty inherits the
	// general EncoderPreset/CRF, which keeps the just-shipped settings
	// valid while matching Jellyfin's per-codec defaults.
	EncoderPreset string             `json:"encoder_preset"`
	CRF           int                `json:"crf"`
	H264Preset    string             `json:"h264_preset"`
	H265Preset    string             `json:"h265_preset"`
	AV1Preset     string             `json:"av1_preset"`
	H264CRF       int                `json:"h264_crf"`
	H265CRF       int                `json:"h265_crf"`
	AV1CRF        int                `json:"av1_crf"`
	AllowHEVC     bool               `json:"allow_hevc"`
	AllowAV1      bool               `json:"allow_av1"`
	Qualities     []TranscodeQuality `json:"qualities"`

	// Audio policy.
	AudioBitrateKbps       int     `json:"audio_bitrate_kbps"`
	AudioVBR               bool    `json:"audio_vbr"`
	DownmixAudio           bool    `json:"downmix_audio"`
	DownmixAudioBoost      float64 `json:"downmix_audio_boost"` // linear gain applied when downmixing
	DownmixStereoAlgorithm string  `json:"downmix_stereo_algorithm"`

	// Subtitle policy.
	SubtitleMode string `json:"subtitle_mode"` // auto|extract|burn|off
	// AllowSubtitleExtraction serves WebVTT sidecars on demand for direct
	// play, not only inside a transcode session (Jellyfin's "allow
	// subtitle extraction on the fly"). Nil means the shipped default
	// (on): a plain bool could not tell "unset" from "explicitly off".
	AllowSubtitleExtraction *bool `json:"allow_subtitle_extraction,omitempty"`
	// Burn-in font fallback: fontsdir is searched before the system font
	// config, and FallbackFontName forces a family for styled tracks.
	FallbackFontPath string `json:"fallback_font_path"`
	FallbackFontName string `json:"fallback_font_name"`
	// FallbackFontEnabled gates the two above (Jellyfin's "Enable fallback
	// fonts"): a saved path/name stays configured but is ignored when off.
	// Nil means the shipped default (on), like AllowSubtitleExtraction.
	FallbackFontEnabled *bool `json:"fallback_font_enabled,omitempty"`

	// HDR policy.
	ToneMapping          bool   `json:"tone_mapping"`
	ToneMappingAlgorithm string `json:"tone_mapping_algorithm"`
	ToneMappingMode      string `json:"tone_mapping_mode"`
	ToneMappingPeakNits  int    `json:"tone_mapping_peak_nits"`

	// Deinterlace policy.
	Deinterlace           string `json:"deinterlace"` // auto|off
	DeinterlaceMethod     string `json:"deinterlace_method"`
	DeinterlaceDoubleRate bool   `json:"deinterlace_double_rate"`

	// Hardware policy (D-031: explicit opt-in + probe + visible fallback).
	HardwareAcceleration string   `json:"hardware_acceleration"` // none|vaapi|nvenc|qsv|amf|v4l2m2m|videotoolbox
	HardwareDecodeCodecs []string `json:"hardware_decode_codecs"`
	HardwareEncode       bool     `json:"hardware_encode"`
	// HardwareLowPower selects QSV's low-power encoder (Jellyfin's
	// "Intel Low-Power H.264/HEVC hardware encoder"); only QSV honours
	// it, other backends ignore the flag.
	HardwareLowPower bool `json:"hardware_low_power"`
	// 10-bit decoding is opt-in per codec: some backends produce wrong
	// output or fail outright on 10-bit sources.
	HardwareDecode10BitHEVC bool `json:"hardware_decode_10bit_hevc"`
	HardwareDecode10BitVP9  bool `json:"hardware_decode_10bit_vp9"`
	// HardwareDevice is the VA-API render node; empty means the default.
	// Other backends select their own device (or none).
	HardwareDevice string `json:"hardware_device"`

	// Binary paths; empty means PATH lookup.
	FFmpegPath  string `json:"ffmpeg_path"`
	FFprobePath string `json:"ffprobe_path"`
}

// DefaultTranscodeSettings returns the shipped policy. It preserves the
// D-030 conservative encode as the default while opening the new
// knobs; HLS is the default delivery because it starts playback
// before preparation finishes.
func DefaultTranscodeSettings() TranscodeSettings {
	return TranscodeSettings{
		DefaultDelivery:     TranscodeDeliveryHLS,
		HLSSegmentSeconds:   6,
		HLSSegmentContainer: HLSSegmentFMP4,
		Throttle:            true,
		ThrottleAheadSec:    30,
		SegmentDeletion:     false,
		SegmentKeepSec:      60,
		IdleTimeoutSec:      120,
		CacheBytes:          20 << 30,
		QueueSize:           8,
		MaxConcurrent:       2,
		EncoderPreset:       "veryfast",
		CRF:                 23,
		AllowHEVC:           false,
		AllowAV1:            false,
		Qualities: []TranscodeQuality{
			{Name: "2160p", MaxWidth: 3840, MaxHeight: 2160, BitrateKbps: 40000},
			{Name: "1080p", MaxWidth: 1920, MaxHeight: 1080, BitrateKbps: 8000},
			{Name: "720p", MaxWidth: 1280, MaxHeight: 720, BitrateKbps: 4000},
			{Name: "480p", MaxWidth: 854, MaxHeight: 480, BitrateKbps: 1500},
			{Name: "360p", MaxWidth: 640, MaxHeight: 360, BitrateKbps: 700},
		},
		AudioBitrateKbps:        160,
		AudioVBR:                false,
		DownmixAudio:            true,
		DownmixAudioBoost:       2,
		DownmixStereoAlgorithm:  DownmixNone,
		ThreadCount:             0,
		MuxingQueueSize:         2048,
		TranscodeTempPath:       "",
		RemoteBitrateLimitKbps:  0,
		H264Preset:              "",
		H265Preset:              "",
		AV1Preset:               "",
		H264CRF:                 23,
		H265CRF:                 28,
		AV1CRF:                  32,
		DeinterlaceDoubleRate:   false,
		DeinterlaceMethod:       DeinterlaceYadif,
		FallbackFontPath:        "",
		FallbackFontName:        "",
		HardwareDecode10BitHEVC: false,
		HardwareDecode10BitVP9:  false,
		HardwareDevice:          DefaultHardwareDevice,
		SubtitleMode:            SubtitleModeAuto,
		ToneMapping:             true,
		ToneMappingAlgorithm:    ToneMapBT2390,
		ToneMappingMode:         ToneMapModeAuto,
		ToneMappingPeakNits:     100,
		Deinterlace:             DeinterlaceAuto,
		HardwareAcceleration:    HWNone,
		HardwareDecodeCodecs:    []string{"h264", "hevc", "mpeg2", "vc1", "vp8", "vp9", "av1"},
		HardwareEncode:          false,
		HardwareLowPower:        false,
	}
}

// Normalize fills zero values with defaults so a partial settings
// document (older server, hand-edited JSON) still yields a full policy.
func (s TranscodeSettings) Normalize() TranscodeSettings {
	d := DefaultTranscodeSettings()
	if s.DefaultDelivery == "" {
		s.DefaultDelivery = d.DefaultDelivery
	}
	if s.HLSSegmentSeconds <= 0 {
		s.HLSSegmentSeconds = d.HLSSegmentSeconds
	}
	if s.HLSSegmentContainer == "" {
		s.HLSSegmentContainer = d.HLSSegmentContainer
	}
	if s.ThrottleAheadSec <= 0 {
		s.ThrottleAheadSec = d.ThrottleAheadSec
	}
	if s.SegmentKeepSec <= 0 {
		s.SegmentKeepSec = d.SegmentKeepSec
	}
	if s.IdleTimeoutSec <= 0 {
		s.IdleTimeoutSec = d.IdleTimeoutSec
	}
	if s.CacheBytes <= 0 {
		s.CacheBytes = d.CacheBytes
	}
	if s.QueueSize <= 0 {
		s.QueueSize = d.QueueSize
	}
	if s.MaxConcurrent <= 0 {
		s.MaxConcurrent = d.MaxConcurrent
	}
	if s.EncoderPreset == "" {
		s.EncoderPreset = d.EncoderPreset
	}
	if s.CRF <= 0 {
		s.CRF = d.CRF
	}
	if len(s.Qualities) == 0 {
		s.Qualities = append([]TranscodeQuality(nil), d.Qualities...)
	}
	if s.AudioBitrateKbps <= 0 {
		s.AudioBitrateKbps = d.AudioBitrateKbps
	}
	if s.DownmixAudioBoost <= 0 {
		s.DownmixAudioBoost = d.DownmixAudioBoost
	}
	if s.DownmixStereoAlgorithm == "" {
		s.DownmixStereoAlgorithm = d.DownmixStereoAlgorithm
	}
	if s.MuxingQueueSize <= 0 {
		s.MuxingQueueSize = d.MuxingQueueSize
	}
	if s.H264CRF <= 0 {
		s.H264CRF = d.H264CRF
	}
	if s.H265CRF <= 0 {
		s.H265CRF = d.H265CRF
	}
	if s.AV1CRF <= 0 {
		s.AV1CRF = d.AV1CRF
	}
	if s.SubtitleMode == "" {
		s.SubtitleMode = d.SubtitleMode
	}
	if s.ToneMappingAlgorithm == "" {
		s.ToneMappingAlgorithm = d.ToneMappingAlgorithm
	}
	if s.ToneMappingMode == "" {
		s.ToneMappingMode = d.ToneMappingMode
	}
	if s.ToneMappingPeakNits <= 0 {
		s.ToneMappingPeakNits = d.ToneMappingPeakNits
	}
	if s.Deinterlace == "" {
		s.Deinterlace = d.Deinterlace
	}
	if s.DeinterlaceMethod == "" {
		s.DeinterlaceMethod = d.DeinterlaceMethod
	}
	if s.HardwareAcceleration == "" {
		s.HardwareAcceleration = d.HardwareAcceleration
	}
	if s.HardwareDevice == "" {
		s.HardwareDevice = d.HardwareDevice
	}
	return s
}

// Validate rejects out-of-range policy values with a stable message.
func (s TranscodeSettings) Validate() error {
	switch s.DefaultDelivery {
	case TranscodeDeliveryProgressive, TranscodeDeliveryHLS:
	default:
		return fmt.Errorf("default_delivery must be %s or %s", TranscodeDeliveryProgressive, TranscodeDeliveryHLS)
	}
	if s.HLSSegmentSeconds < 1 || s.HLSSegmentSeconds > 30 {
		return fmt.Errorf("hls_segment_seconds must be 1..30")
	}
	switch s.HLSSegmentContainer {
	case HLSSegmentFMP4, HLSSegmentTS:
	default:
		return fmt.Errorf("hls_segment_container must be %s or %s", HLSSegmentFMP4, HLSSegmentTS)
	}
	if s.ThrottleAheadSec < 5 || s.ThrottleAheadSec > 600 {
		return fmt.Errorf("throttle_ahead_sec must be 5..600")
	}
	if s.SegmentKeepSec < 0 || s.SegmentKeepSec > 3600 {
		return fmt.Errorf("segment_keep_sec must be 0..3600")
	}
	if s.IdleTimeoutSec < 15 || s.IdleTimeoutSec > 3600 {
		return fmt.Errorf("idle_timeout_sec must be 15..3600")
	}
	if s.CacheBytes < 1<<30 {
		return fmt.Errorf("cache_bytes must be at least 1 GiB")
	}
	if s.QueueSize < 1 || s.QueueSize > 64 {
		return fmt.Errorf("queue_size must be 1..64")
	}
	if s.MaxConcurrent < 1 || s.MaxConcurrent > 16 {
		return fmt.Errorf("max_concurrent must be 1..16")
	}
	if !containsString(EncoderPresets, s.EncoderPreset) {
		return fmt.Errorf("encoder_preset must be one of %s", strings.Join(EncoderPresets, ", "))
	}
	if s.CRF < 0 || s.CRF > 51 {
		return fmt.Errorf("crf must be 0..51")
	}
	for _, q := range s.Qualities {
		if strings.TrimSpace(q.Name) == "" {
			return fmt.Errorf("quality entries need a name")
		}
		if q.MaxWidth < 0 || q.MaxHeight < 0 || q.BitrateKbps < 0 {
			return fmt.Errorf("quality %s has negative bounds", q.Name)
		}
	}
	if s.AudioBitrateKbps < 32 || s.AudioBitrateKbps > 640 {
		return fmt.Errorf("audio_bitrate_kbps must be 32..640")
	}
	if s.DownmixAudioBoost < 0.5 || s.DownmixAudioBoost > 8 {
		return fmt.Errorf("downmix_audio_boost must be 0.5..8")
	}
	switch s.DownmixStereoAlgorithm {
	case DownmixNone, DownmixNightmode:
	default:
		return fmt.Errorf("downmix_stereo_algorithm must be none or nightmode")
	}
	if s.ThreadCount < 0 || s.ThreadCount > 64 {
		return fmt.Errorf("thread_count must be 0..64")
	}
	if s.MuxingQueueSize < 128 || s.MuxingQueueSize > 65536 {
		return fmt.Errorf("max_muxing_queue_size must be 128..65536")
	}
	if s.RemoteBitrateLimitKbps < 0 || s.RemoteBitrateLimitKbps > 1_000_000 {
		return fmt.Errorf("remote_bitrate_limit_kbps must be 0..1000000")
	}
	for name, crf := range map[string]int{"h264_crf": s.H264CRF, "h265_crf": s.H265CRF, "av1_crf": s.AV1CRF} {
		if crf < 0 || crf > 51 {
			return fmt.Errorf("%s must be 0..51", name)
		}
	}
	for name, preset := range map[string]string{"h264_preset": s.H264Preset, "h265_preset": s.H265Preset, "av1_preset": s.AV1Preset} {
		if preset != "" && !containsString(EncoderPresets, preset) {
			return fmt.Errorf("%s must be one of %s", name, strings.Join(EncoderPresets, ", "))
		}
	}
	switch s.SubtitleMode {
	case SubtitleModeAuto, SubtitleModeExtract, SubtitleModeBurn, SubtitleModeOff:
	default:
		return fmt.Errorf("subtitle_mode must be auto, extract, burn or off")
	}
	switch s.ToneMappingAlgorithm {
	case ToneMapBT2390, ToneMapHable, ToneMapReinhard, ToneMapMobius, ToneMapClip, ToneMapLinear:
	default:
		return fmt.Errorf("tone_mapping_algorithm is not recognized")
	}
	switch s.ToneMappingMode {
	case ToneMapModeAuto, ToneMapModeAlways, ToneMapModeNever:
	default:
		return fmt.Errorf("tone_mapping_mode must be auto, always or never")
	}
	if s.ToneMappingPeakNits < 50 || s.ToneMappingPeakNits > 10000 {
		return fmt.Errorf("tone_mapping_peak_nits must be 50..10000")
	}
	switch s.Deinterlace {
	case DeinterlaceAuto, DeinterlaceOff:
	default:
		return fmt.Errorf("deinterlace must be auto or off")
	}
	switch s.DeinterlaceMethod {
	case DeinterlaceYadif, DeinterlaceBwdif:
	default:
		return fmt.Errorf("deinterlace_method must be yadif or bwdif")
	}
	switch s.HardwareAcceleration {
	case HWNone, HWVAAPI, HWNVENC, HWQSV, HWAMF, HWV4L2M2M, HWVideoToolbox, HWRKMPP:
	default:
		return fmt.Errorf("hardware_acceleration is not recognized")
	}
	for _, codec := range s.HardwareDecodeCodecs {
		if !containsString(HardwareDecodeCodecs, codec) {
			return fmt.Errorf("hardware_decode_codecs contains unknown codec %s", codec)
		}
	}
	// The device is handed to ffmpeg as one argv element, so only reject
	// what would break the argv or name no device at all.
	if device := strings.TrimSpace(s.HardwareDevice); device != "" {
		if len(device) > 512 || strings.ContainsAny(device, "\x00\n\r") {
			return fmt.Errorf("hardware_device is not a usable device path")
		}
	}
	return nil
}

func containsString(list []string, want string) bool {
	for _, v := range list {
		if v == want {
			return true
		}
	}
	return false
}

// AllowStreamCopy reports a stream-copy permission: only an explicit
// false forbids copying (Jellyfin's PlaybackInfo flags).
func AllowStreamCopy(flag *bool) bool {
	return flag == nil || *flag
}

// AllowsSubtitleExtraction reports the effective permission: only an
// explicit false turns on-the-fly extraction off.
func (s TranscodeSettings) AllowsSubtitleExtraction() bool {
	return s.AllowSubtitleExtraction == nil || *s.AllowSubtitleExtraction
}

// AllowsFallbackFonts reports the effective fallback-font permission: only
// an explicit false turns the burn-in font fallback off.
func (s TranscodeSettings) AllowsFallbackFonts() bool {
	return s.FallbackFontEnabled == nil || *s.FallbackFontEnabled
}

// EncoderTuning resolves the per-codec CRF and preset, falling back to
// the general values so the settings shipped before the per-codec knobs
// keep behaving exactly as they did.
func (s TranscodeSettings) EncoderTuning(codec string) (crf int, preset string) {
	crf, preset = s.CRF, s.EncoderPreset
	switch codec {
	case VideoCodecHEVC:
		if s.H265CRF > 0 {
			crf = s.H265CRF
		}
		if s.H265Preset != "" {
			preset = s.H265Preset
		}
	case VideoCodecAV1:
		if s.AV1CRF > 0 {
			crf = s.AV1CRF
		}
		if s.AV1Preset != "" {
			preset = s.AV1Preset
		}
	default:
		if s.H264CRF > 0 {
			crf = s.H264CRF
		}
		if s.H264Preset != "" {
			preset = s.H264Preset
		}
	}
	return crf, preset
}

// Quality resolves a requested preset name; an empty or unknown name
// falls back to the unconstrained entry (nil), which the caller treats
// as "no cap beyond the user bitrate limit".
func (s TranscodeSettings) Quality(name string) (TranscodeQuality, bool) {
	if name == "" {
		return TranscodeQuality{}, false
	}
	for _, q := range s.Qualities {
		if q.Name == name {
			return q, true
		}
	}
	return TranscodeQuality{}, false
}

// TranscodePolicy carries the per-user playback limits (D-042). A nil
// pointer on the request means unrestricted.
type TranscodePolicy struct {
	AllowVideoTranscode bool `json:"allow_video_transcode"`
	AllowAudioTranscode bool `json:"allow_audio_transcode"`
	AllowRemux          bool `json:"allow_remux"`
	// MaxStreams bounds simultaneous transcodes for this user; 0 means
	// unlimited. The plugin only compares opaque user ids, so it never
	// learns anything about accounts.
	MaxStreams int `json:"max_streams,omitempty"`
}

// TranscodeV3Request controls one v3 session. FilePath is always
// resolved by the gateway. Settings is the effective operator policy.
type TranscodeV3Request struct {
	Action   string `json:"action"`
	FilePath string `json:"file_path"`
	Session  string `json:"session,omitempty"`

	Delivery       string `json:"delivery,omitempty"`         // "" uses settings default
	Quality        string `json:"quality,omitempty"`          // ladder name or "" (auto)
	MaxBitrateKbps int    `json:"max_bitrate_kbps,omitempty"` // effective per-user cap, 0 = none
	VideoCodec     string `json:"video_codec,omitempty"`      // h264 (default), hevc, av1
	AudioCodec     string `json:"audio_codec,omitempty"`      // aac (default), ac3, eac3

	AudioStream    *int   `json:"audio_stream,omitempty"`
	SubtitleStream *int   `json:"subtitle_stream,omitempty"`
	SubtitleMode   string `json:"subtitle_mode,omitempty"` // "" uses settings default

	// SegmentIndex is set by the gateway on TranscodePositionAction:
	// the highest HLS segment the client has fetched.
	SegmentIndex int `json:"segment_index,omitempty"`

	// AllowVideoStreamCopy/AllowAudioStreamCopy mirror the client-side
	// flags of Jellyfin's PlaybackInfo request: false forces a re-encode
	// of that stream even when a copy would be possible.
	AllowVideoStreamCopy *bool `json:"allow_video_stream_copy,omitempty"`
	AllowAudioStreamCopy *bool `json:"allow_audio_stream_copy,omitempty"`

	// Policy is the requesting user's limits; nil allows everything.
	Policy *TranscodePolicy `json:"policy,omitempty"`

	// UserID is an opaque account reference used only to count a user's
	// own simultaneous sessions and to label the admin session list.
	UserID string `json:"user_id,omitempty"`

	Settings TranscodeSettings `json:"settings"`
}

// TranscodeV3Status is the v3 result. Path/PlaylistPath/SubtitlePath are
// trusted-plane values for the gateway; they are never serialized to a
// client directly.
type TranscodeV3Status struct {
	Session  string `json:"session"`
	State    string `json:"state"`
	Profile  string `json:"profile"`
	Delivery string `json:"delivery,omitempty"`

	Path         string `json:"path,omitempty"`
	PlaylistPath string `json:"playlist_path,omitempty"`
	SubtitlePath string `json:"subtitle_path,omitempty"`

	Method   string `json:"method,omitempty"` // remux|transcode
	Cached   bool   `json:"cached,omitempty"`
	Playable bool   `json:"playable,omitempty"` // HLS: playlist with at least one segment

	// VideoDirect/AudioDirect report which streams are stream-copied
	// rather than re-encoded (Jellyfin's TranscodingInfo IsVideoDirect).
	VideoDirect bool `json:"video_direct,omitempty"`
	AudioDirect bool `json:"audio_direct,omitempty"`

	Encoder  string `json:"encoder,omitempty"`
	Hardware string `json:"hardware,omitempty"`
	Fallback string `json:"fallback,omitempty"` // why software was used instead

	Reasons []string `json:"reasons,omitempty"`

	VideoCodec  string `json:"video_codec,omitempty"`
	AudioCodec  string `json:"audio_codec,omitempty"`
	Width       int    `json:"width,omitempty"`
	Height      int    `json:"height,omitempty"`
	BitrateKbps int    `json:"bitrate_kbps,omitempty"`

	// Live pipeline metrics read from ffmpeg's progress stream while the
	// job runs (the session list's transcoding info).
	FPS               float64 `json:"fps,omitempty"`
	OutputBitrateKbps int     `json:"output_bitrate_kbps,omitempty"`

	// UserID labels the owning account for the admin session list. The
	// public player projection drops it.
	UserID string `json:"user_id,omitempty"`

	ErrorCode string `json:"error_code,omitempty"`
	Error     string `json:"error,omitempty"`

	QueuedAt   int64 `json:"queued_at,omitempty"`
	StartedAt  int64 `json:"started_at,omitempty"`
	FinishedAt int64 `json:"finished_at,omitempty"`

	Progress float64 `json:"progress,omitempty"`
}

// TranscodeProfileKey derives the cache/session profile identity from
// the settings and selections that change the produced bytes. Any
// policy change that alters output must change this key, so stale
// derivatives never match (D-030/D-045). Purely operational knobs
// (thread count, muxing queue, temp path, cache/queue bounds) stay out:
// they change speed and placement, not the bytes.
func TranscodeProfileKey(s TranscodeSettings, req TranscodeV3Request) string {
	delivery := req.Delivery
	if delivery == "" {
		delivery = s.DefaultDelivery
	}
	videoCodec := req.VideoCodec
	if videoCodec == "" {
		videoCodec = VideoCodecH264
	}
	audioCodec := req.AudioCodec
	if audioCodec == "" {
		audioCodec = AudioCodecAAC
	}
	subtitleMode := req.SubtitleMode
	if subtitleMode == "" {
		subtitleMode = s.SubtitleMode
	}
	quality := req.Quality
	// The resolved ladder entry changes the produced bytes (scale and
	// maxrate), so its values belong in the key, not only the name: an
	// operator editing a ladder entry must not keep serving the old
	// derivative.
	qualitySpec := ""
	if q, ok := s.Quality(quality); ok {
		qualitySpec = fmt.Sprintf("%d/%d/%d", q.MaxWidth, q.MaxHeight, q.BitrateKbps)
	}
	audio, subtitle := "default", "off"
	if req.AudioStream != nil {
		audio = fmt.Sprint(*req.AudioStream)
	}
	if req.SubtitleStream != nil {
		subtitle = fmt.Sprint(*req.SubtitleStream)
	}
	fields := []string{
		"v3", delivery, quality, qualitySpec, videoCodec, audioCodec, subtitleMode,
		fmt.Sprintf("ab:%d", s.AudioBitrateKbps),
		fmt.Sprintf("vbr:%t", s.AudioVBR),
		fmt.Sprintf("dm:%t", s.DownmixAudio),
		fmt.Sprintf("dmb:%.2f", s.DownmixAudioBoost),
		"dma:" + s.DownmixStereoAlgorithm,
		fmt.Sprintf("preset:%s", s.EncoderPreset),
		fmt.Sprintf("p:%s/%s/%s", s.H264Preset, s.H265Preset, s.AV1Preset),
		fmt.Sprintf("crf:%d", s.CRF),
		fmt.Sprintf("crf3:%d/%d/%d", s.H264CRF, s.H265CRF, s.AV1CRF),
		fmt.Sprintf("maxrate:%d", req.MaxBitrateKbps),
		fmt.Sprintf("hw:%s", s.HardwareAcceleration),
		fmt.Sprintf("hwe:%t", s.HardwareEncode),
		fmt.Sprintf("hwlp:%t", s.HardwareLowPower),
		fmt.Sprintf("hw10:%t/%t", s.HardwareDecode10BitHEVC, s.HardwareDecode10BitVP9),
		"hwdec:" + strings.Join(s.HardwareDecodeCodecs, ","),
		"ffmpeg:" + s.FFmpegPath,
		fmt.Sprintf("tm:%t:%s:%s:%d", s.ToneMapping, s.ToneMappingAlgorithm, s.ToneMappingMode, s.ToneMappingPeakNits),
		fmt.Sprintf("di:%s:%t", s.Deinterlace, s.DeinterlaceDoubleRate),
		"dim:" + s.DeinterlaceMethod,
		fmt.Sprintf("copyv:%t", AllowStreamCopy(req.AllowVideoStreamCopy)),
		fmt.Sprintf("copya:%t", AllowStreamCopy(req.AllowAudioStreamCopy)),
		fmt.Sprintf("seg:%d", s.HLSSegmentSeconds),
		"hlsct:" + s.HLSSegmentContainer,
		"font:" + s.FallbackFontPath + "/" + s.FallbackFontName + fmt.Sprintf("/%t", s.AllowsFallbackFonts()),
		"a:" + audio, "s:" + subtitle,
	}
	h := sha256.Sum256([]byte(strings.Join(fields, "|")))
	return "web-mp4-v3-" + hex.EncodeToString(h[:6])
}
