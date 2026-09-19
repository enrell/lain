// V3 session planning: resolve operator settings, probed streams and
// the client selection into one encode policy. This file owns the
// decisions (copy vs encode, codecs, filters, hardware, subtitles,
// reasons); encode.go owns the ffmpeg argv, hls.go owns packaging.
package transcode

import (
	"context"
	"fmt"
	"os/exec"
	"strconv"
	"strings"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

// capabilities is the probed ffmpeg surface (D-031: capability
// probing, never silent opportunistic switching).
type capabilities struct {
	Encoders    map[string]bool
	Filters     map[string]bool
	Hwaccels    map[string]bool
	ToneMap     bool // zscale+tonemap chain executes
	ToneMap2390 bool // libplacebo BT.2390 path executes
	Hardware    map[string]bool
	ProbedAt    time.Time
	FFmpeg      string
}

func (c capabilities) hasEncoder(name string) bool { return c.Encoders[name] }
func (c capabilities) hasFilter(name string) bool  { return c.Filters[name] }

// encodePlan is one fully-resolved session policy.
type encodePlan struct {
	spec     sourceSpec
	profile  string
	delivery string
	settings contracts.TranscodeSettings

	report mediaReport

	video    *stream
	audio    *stream
	subtitle *stream

	subtitleMode string // off|extract|burn
	burnText     bool
	burnImage    bool

	copyVideo  bool
	copyAudio  bool
	videoCodec string
	audioCodec string
	encoder    string
	hwBackend  string
	hwDecode   bool
	needTone   bool
	fallback   string

	// crf and preset are the per-codec values resolved once at plan
	// time, so the encoder argv and the cache key cannot drift apart.
	crf    int
	preset string

	filters       []string
	complexFilter string

	width, height    int
	maxBitrateKbps   int
	audioBitrateKbps int
	downmix          bool

	reasons []string
	method  string
}

func (p encodePlan) isHLS() bool { return p.delivery == contracts.TranscodeDeliveryHLS }

func hasStreamCodec(report mediaReport, kind, codec string) bool {
	for _, s := range report.Streams {
		if s.CodecType == kind && strings.EqualFold(s.CodecName, codec) {
			return true
		}
	}
	return false
}

func firstVideo(report mediaReport) *stream {
	for i := range report.Streams {
		if report.Streams[i].CodecType == "video" {
			return &report.Streams[i]
		}
	}
	return nil
}

func selectAudio(report mediaReport, want *int) (*stream, error) {
	var first, def *stream
	for i := range report.Streams {
		s := &report.Streams[i]
		if s.CodecType != "audio" {
			continue
		}
		if first == nil {
			first = s
		}
		if def == nil && s.Default != 0 {
			def = s
		}
	}
	if want != nil {
		for i := range report.Streams {
			s := &report.Streams[i]
			if s.Index == *want && s.CodecType == "audio" {
				return s, nil
			}
		}
		return nil, invalid("audio_stream is not an audio track")
	}
	if def != nil {
		return def, nil
	}
	return first, nil
}

func selectSubtitle(report mediaReport, want *int) (*stream, error) {
	if want == nil {
		return nil, nil
	}
	for i := range report.Streams {
		s := &report.Streams[i]
		if s.Index == *want && s.CodecType == "subtitle" {
			return s, nil
		}
	}
	return nil, invalid("subtitle_stream is not a subtitle track")
}

func imageSubtitleCodec(codec string) bool {
	switch strings.ToLower(codec) {
	case "hdmv_pgs_subtitle", "dvd_subtitle", "dvb_subtitle", "xsub":
		return true
	default:
		return false
	}
}

func hdrStream(s *stream) bool {
	if s == nil {
		return false
	}
	switch strings.ToLower(s.ColorTransfer) {
	case "smpte2084", "arib-std-b67":
		return true
	default:
		return false
	}
}

func interlacedStream(s *stream) bool {
	if s == nil {
		return false
	}
	switch strings.ToLower(s.FieldOrder) {
	case "tt", "bb", "tb", "bt":
		return true
	default:
		return false
	}
}

// planV3 resolves one v3 session. It is pure with respect to the
// filesystem: only probing reads, nothing starts work.
func (t *Transcoder) planV3(spec sourceSpec, in contracts.TranscodeV3Request, caps capabilities) (encodePlan, error) {
	settings := in.Settings.Normalize()
	if err := settings.Validate(); err != nil {
		return encodePlan{}, invalid(err.Error())
	}
	report, err := t.probeReport(settings, spec.Path)
	if err != nil {
		return encodePlan{}, &core.Error{Code: "dependency-unavailable", Msg: "ffprobe could not read the source"}
	}
	plan := encodePlan{spec: spec, settings: settings, report: report}
	plan.delivery = in.Delivery
	if plan.delivery == "" {
		plan.delivery = settings.DefaultDelivery
	}
	switch plan.delivery {
	case contracts.TranscodeDeliveryProgressive, contracts.TranscodeDeliveryHLS:
	default:
		return encodePlan{}, invalid("delivery must be progressive or hls")
	}

	video := firstVideo(report)
	if video == nil {
		return encodePlan{}, &core.Error{Code: "unsupported-media", Msg: "source has no video stream"}
	}
	plan.video = video
	plan.width, plan.height = video.Width, video.Height

	audio, err := selectAudio(report, in.AudioStream)
	if err != nil {
		return encodePlan{}, err
	}
	plan.audio = audio

	subtitle, err := selectSubtitle(report, in.SubtitleStream)
	if err != nil {
		return encodePlan{}, err
	}
	plan.subtitle = subtitle

	mode := in.SubtitleMode
	if mode == "" {
		mode = settings.SubtitleMode
	}
	if err := plan.resolveSubtitle(mode, caps); err != nil {
		return encodePlan{}, err
	}

	// Quality target: preset caps, then the per-user bitrate limit.
	if in.Quality != "" {
		if _, ok := settings.Quality(in.Quality); !ok {
			// An unknown ladder name must fail loudly: silently
			// ignoring it would drop the requested cap and mint a
			// distinct cache entry per arbitrary string.
			return encodePlan{}, invalid(fmt.Sprintf("unknown quality %q", in.Quality))
		}
	}
	maxBitrate := in.MaxBitrateKbps
	if q, ok := settings.Quality(in.Quality); ok {
		if q.MaxWidth > 0 {
			plan.width = min(plan.width, q.MaxWidth)
		}
		if q.MaxHeight > 0 {
			plan.height = min(plan.height, q.MaxHeight)
		}
		if q.BitrateKbps > 0 && (maxBitrate == 0 || q.BitrateKbps < maxBitrate) {
			maxBitrate = q.BitrateKbps
		}
	}
	if maxBitrate > 0 {
		plan.maxBitrateKbps = maxBitrate
	}

	if err := plan.resolveVideo(in, caps); err != nil {
		return encodePlan{}, err
	}
	if err := plan.resolveAudio(in, settings); err != nil {
		return encodePlan{}, err
	}
	plan.resolveFilters(caps)
	plan.method = "transcode"
	if plan.copyVideo && plan.copyAudio {
		plan.method = "remux"
	}
	if in.Policy != nil {
		if plan.method == "remux" && !in.Policy.AllowRemux {
			return encodePlan{}, &core.Error{Code: "forbidden", Msg: "remuxing is disabled for this user"}
		}
		if !plan.copyVideo && !in.Policy.AllowVideoTranscode {
			return encodePlan{}, &core.Error{Code: "forbidden", Msg: "video transcoding is disabled for this user"}
		}
		if !plan.copyAudio && !in.Policy.AllowAudioTranscode {
			return encodePlan{}, &core.Error{Code: "forbidden", Msg: "audio transcoding is disabled for this user"}
		}
	}
	plan.profile = contracts.TranscodeProfileKey(settings, in)
	// Merge rather than replace: resolveVideo/resolveAudio already recorded
	// why a copy was refused (a client-side flag), and the reason list is a
	// user-facing explanation, so dropping those entries would hide a cause.
	for _, r := range plan.transcodeReasons(in) {
		plan.reasons = appendReason(plan.reasons, r)
	}
	return plan, nil
}

// resolveSubtitle decides extract vs burn for the selected track.
func (p *encodePlan) resolveSubtitle(mode string, caps capabilities) error {
	switch mode {
	case contracts.SubtitleModeOff:
		p.subtitleMode = contracts.SubtitleModeOff
		p.subtitle = nil
		return nil
	}
	if p.subtitle == nil {
		p.subtitleMode = contracts.SubtitleModeOff
		return nil
	}
	image := imageSubtitleCodec(p.subtitle.CodecName)
	text := textSubtitleCodec(p.subtitle.CodecName)
	switch mode {
	case contracts.SubtitleModeExtract:
		if !text {
			return &core.Error{Code: "unsupported-media", Msg: "selected subtitle cannot be converted to WebVTT"}
		}
		p.subtitleMode = contracts.SubtitleModeExtract
	case contracts.SubtitleModeBurn:
		return p.enableBurn(image, text, caps)
	case contracts.SubtitleModeAuto:
		if text {
			p.subtitleMode = contracts.SubtitleModeExtract
			return nil
		}
		if image {
			return p.enableBurn(image, text, caps)
		}
		return &core.Error{Code: "unsupported-media", Msg: "selected subtitle cannot be converted to WebVTT"}
	default:
		return invalid("subtitle_mode must be auto, extract, burn or off")
	}
	return nil
}

func (p *encodePlan) enableBurn(image, text bool, caps capabilities) error {
	if image {
		if !caps.hasFilter("overlay") {
			return &core.Error{Code: "unsupported-media", Msg: "this ffmpeg cannot burn image subtitles (overlay filter missing)"}
		}
		p.subtitleMode = contracts.SubtitleModeBurn
		p.burnImage = true
		return nil
	}
	if text {
		if !caps.hasFilter("subtitles") && !caps.hasFilter("ass") {
			return &core.Error{Code: "unsupported-media", Msg: "this ffmpeg cannot burn text subtitles (libass missing)"}
		}
		p.subtitleMode = contracts.SubtitleModeBurn
		p.burnText = true
		return nil
	}
	return &core.Error{Code: "unsupported-media", Msg: "selected subtitle cannot be burned"}
}

// resolveVideo decides copy vs encode and the encoder.
func (p *encodePlan) resolveVideo(in contracts.TranscodeV3Request, caps capabilities) error {
	codec := in.VideoCodec
	if codec == "" {
		codec = contracts.VideoCodecH264
	}
	switch codec {
	case contracts.VideoCodecH264:
	case contracts.VideoCodecHEVC:
		if !p.settings.AllowHEVC {
			return invalid("HEVC encoding is not allowed by server settings")
		}
	case contracts.VideoCodecAV1:
		if !p.settings.AllowAV1 {
			return invalid("AV1 encoding is not allowed by server settings")
		}
	default:
		return invalid("video_codec must be h264, hevc or av1")
	}
	p.videoCodec = codec

	// Copying is only safe for a web-safe H.264 stream with nothing to
	// change: no filters (tone map/scale/deinterlace), no burn-in, and
	// an encode request that did not ask for another codec.
	pixel := strings.ToLower(p.video.PixelFormat)
	webVideo := strings.EqualFold(p.video.CodecName, "h264") && (pixel == "yuv420p" || pixel == "yuvj420p")
	wantsDifferentCodec := codec != contracts.VideoCodecH264
	// A client may forbid stream copying (Jellyfin's PlaybackInfo flags):
	// then the video is re-encoded even though a copy would be legal.
	if !contracts.AllowStreamCopy(in.AllowVideoStreamCopy) {
		webVideo = false
		p.reasons = appendReason(p.reasons, "video stream copy disabled by the client")
	}
	// A bitrate ceiling cannot be honoured by a copy: when the source video
	// exceeds it, force a re-encode (Jellyfin's ContainerBitrateExceedsLimit).
	if p.maxBitrateKbps > 0 && sourceKbps(p.video.BitRate) > p.maxBitrateKbps {
		webVideo = false
	}
	p.copyVideo = webVideo && !wantsDifferentCodec && !hdrStream(p.video) &&
		!interlacedStream(p.video) && !p.burnImage && !p.burnText &&
		(p.width == 0 || p.width >= p.video.Width) && (p.height == 0 || p.height >= p.video.Height)
	if p.copyVideo {
		p.encoder = ""
		return nil
	}

	// Tone mapping needs to be possible before promising an encode.
	hdr := hdrStream(p.video)
	toneMode := p.settings.ToneMappingMode
	needTone := hdr && p.settings.ToneMapping && toneMode != contracts.ToneMapModeNever
	alwaysTone := p.settings.ToneMapping && toneMode == contracts.ToneMapModeAlways
	if hdr && !p.settings.ToneMapping {
		return &core.Error{Code: "unsupported-media", Msg: "HDR source and tone mapping is disabled in server settings"}
	}
	if hdr && toneMode == contracts.ToneMapModeNever {
		return &core.Error{Code: "unsupported-media", Msg: "HDR source and tone mapping mode is never"}
	}
	if needTone || alwaysTone {
		if !caps.ToneMap {
			return &core.Error{Code: "unsupported-media", Msg: "HDR tone mapping is unavailable on this server (probe failed)"}
		}
		if p.settings.ToneMappingAlgorithm == contracts.ToneMapBT2390 && !caps.ToneMap2390 {
			// Visible fallback: keep the session working with the best
			// available curve instead of failing the play.
			p.fallback = "bt2390 tone mapping unavailable; used hable"
		}
	}
	p.needTone = needTone || alwaysTone

	encoder, backend, hwDecode, fallback, err := chooseEncoder(p.settings, codec, p.report, caps, p.video)
	if err != nil {
		return err
	}
	p.encoder = encoder
	p.hwBackend = backend
	p.hwDecode = hwDecode
	if fallback != "" {
		p.fallback = joinNotes(p.fallback, fallback)
	}
	p.crf, p.preset = p.settings.EncoderTuning(codec)
	return nil
}

func joinNotes(a, b string) string {
	if a == "" {
		return b
	}
	if b == "" {
		return a
	}
	return a + "; " + b
}

func (p *encodePlan) resolveAudio(in contracts.TranscodeV3Request, settings contracts.TranscodeSettings) error {
	codec := in.AudioCodec
	if codec == "" {
		codec = contracts.AudioCodecAAC
	}
	switch codec {
	case contracts.AudioCodecAAC, contracts.AudioCodecAC3, contracts.AudioCodecEAC3:
	default:
		// Never let an unvalidated codec reach the ffmpeg argv.
		return invalid("audio_codec must be aac, ac3 or eac3")
	}
	p.audioCodec = codec
	p.audioBitrateKbps = settings.AudioBitrateKbps
	if p.audio == nil {
		p.copyAudio = true
		return nil
	}
	// Copy when the source already is the requested MP4-compatible codec
	// (aac/ac3/eac3): asking for the source's own codec must not force a
	// needless re-encode.
	p.copyAudio = strings.EqualFold(p.audio.CodecName, codec)
	if !contracts.AllowStreamCopy(in.AllowAudioStreamCopy) {
		p.copyAudio = false
		p.reasons = appendReason(p.reasons, "audio stream copy disabled by the client")
	}
	if p.audio.Channels > 2 && settings.DownmixAudio {
		p.downmix = true
		// Downmixing changes the bytes even when the codec matches.
		p.copyAudio = false
	}
	return nil
}

// needTone is set by resolveVideo; kept on the plan for filter building.
func (p *encodePlan) resolveFilters(caps capabilities) {
	var filters []string
	if p.burnText {
		// The subtitle file is pre-extracted next to the artifact; the
		// path is appended by encode.go once the temp name is known.
		filters = append(filters, "subtitles="+subtitlePlaceholder+subtitleFontOptions(p.settings))
	}
	if p.settings.Deinterlace == contracts.DeinterlaceAuto && interlacedStream(p.video) {
		// bwdif is the better filter; a build without it degrades to yadif
		// and the plan says so (D-031's visible-fallback rule).
		method := p.settings.DeinterlaceMethod
		if method == contracts.DeinterlaceBwdif && !caps.hasFilter("bwdif") {
			method = contracts.DeinterlaceYadif
			p.fallback = joinNotes(p.fallback, "bwdif is unavailable; used yadif")
		}
		if method == "" {
			method = contracts.DeinterlaceYadif
		}
		if p.settings.DeinterlaceDoubleRate {
			// Double rate keeps both fields as frames (Jellyfin's
			// "deinterlace double rate"): smoother motion, twice the
			// output frame count.
			filters = append(filters, method+"=mode=send_field")
		} else {
			filters = append(filters, method)
		}
	}
	if p.needTone {
		filters = append(filters, toneMapChain(p.settings, caps))
	}
	if p.width > 0 && p.video.Width > 0 && (p.width < p.video.Width || (p.height > 0 && p.height < p.video.Height)) {
		filters = append(filters, scaleFilter(p.width, p.height))
	}
	if p.hwBackend == contracts.HWVAAPI && !p.copyVideo {
		filters = append(filters, "format=nv12", "hwupload")
	}
	if p.burnImage {
		// Image subtitles overlay the (already filtered) video stream.
		base := ""
		if len(filters) > 0 {
			base = strings.Join(filters, ",")
		}
		label := "[0:v:0]"
		if base != "" {
			label = fmt.Sprintf("[0:v:0]%s[base]", base)
		}
		p.complexFilter = fmt.Sprintf("%s[0:%d]overlay:shortest=0[vout]", label, p.subtitle.Index)
		p.filters = nil
		return
	}
	p.filters = filters
}

func appendReason(list []string, reason string) []string {
	for _, r := range list {
		if r == reason {
			return list
		}
	}
	return append(list, reason)
}

// transcodeReasons explains, per Jellyfin's TranscodeReasons idea, why
// this session cannot be a direct play.
func (p encodePlan) transcodeReasons(in contracts.TranscodeV3Request) []string {
	var reasons []string
	ext := strings.ToLower(strings.TrimPrefix(fileExt(p.spec.Path), "."))
	if ext != "" && ext != "mp4" && ext != "m4v" {
		reasons = appendReason(reasons, "container ("+ext+")")
	}
	if !p.copyVideo {
		reasons = appendReason(reasons, "video codec ("+p.video.CodecName+")")
		if hdrStream(p.video) {
			reasons = appendReason(reasons, "HDR ("+strings.ToLower(p.video.ColorTransfer)+")")
		}
		if interlacedStream(p.video) && p.settings.Deinterlace == contracts.DeinterlaceAuto {
			reasons = appendReason(reasons, "interlaced video")
		}
		if p.videoCodec != contracts.VideoCodecH264 {
			reasons = appendReason(reasons, "output codec ("+p.videoCodec+")")
		}
	}
	if !p.copyAudio && p.audio != nil {
		reasons = appendReason(reasons, "audio codec ("+p.audio.CodecName+")")
	}
	if p.downmix {
		reasons = appendReason(reasons, "audio downmix")
		// lain's nightmode matrix names 5.1 channel positions, so it is only
		// applied to a real 5.1 source; other layouts fall back to ffmpeg's
		// own layout-aware coefficients, and the status says why.
		if p.settings.DownmixStereoAlgorithm == contracts.DownmixNightmode && (p.audio == nil || p.audio.Channels != 6) {
			reasons = appendReason(reasons, "nightmode needs a 5.1 source")
		}
	}
	if p.maxBitrateKbps > 0 && !p.copyVideo {
		reasons = appendReason(reasons, "bitrate limit ("+strconv.Itoa(p.maxBitrateKbps)+" kbps)")
	}
	if q, ok := p.settings.Quality(in.Quality); ok {
		reasons = appendReason(reasons, "quality ("+q.Name+")")
	}
	if p.burnImage {
		reasons = appendReason(reasons, "image subtitle ("+p.subtitle.CodecName+")")
	}
	if p.burnText {
		reasons = appendReason(reasons, "subtitle burn-in")
	}
	if len(reasons) == 0 {
		reasons = append(reasons, "browser compatibility")
	}
	return reasons
}

func fileExt(path string) string {
	if i := strings.LastIndexByte(path, '.'); i >= 0 {
		return path[i:]
	}
	return ""
}

// scaleFilter caps both dimensions while preserving aspect ratio and
// even dimensions (encoders require them).
func scaleFilter(w, h int) string {
	switch {
	case w > 0 && h > 0:
		return fmt.Sprintf("scale=w='min(iw,%d)':h='min(ih,%d)':force_original_aspect_ratio=decrease:force_divisible_by=2", w, h)
	case h > 0:
		return fmt.Sprintf("scale=-2:'min(ih,%d)'", h)
	default:
		return fmt.Sprintf("scale='min(iw,%d)':-2", w)
	}
}

// toneMapChain builds the HDR->SDR conversion. The float-first chain is
// what this ffmpeg's zimg integration needs; explicit input properties
// keep it independent of missing container metadata.
func toneMapChain(settings contracts.TranscodeSettings, caps capabilities) string {
	alg := settings.ToneMappingAlgorithm
	if alg == contracts.ToneMapBT2390 && caps.ToneMap2390 {
		return "libplacebo=tonemapping=bt.2390:colorspace=bt709:color_primaries=bt709:color_trc=bt709:range=tv:format=yuv420p"
	}
	if alg == contracts.ToneMapBT2390 {
		alg = contracts.ToneMapHable
	}
	peak := settings.ToneMappingPeakNits
	return fmt.Sprintf(
		"format=gbrpf32le,zscale=transferin=smpte2084:primariesin=bt2020:matrixin=bt2020nc:transfer=linear:primaries=bt2020:matrix=bt2020nc:npl=%d,tonemap=tonemap=%s:desat=0,zscale=transferin=linear:primariesin=bt2020:matrixin=bt2020nc:transfer=bt709:primaries=bt709:matrix=bt709:range=limited,format=yuv420p",
		peak, alg,
	)
}

// chooseEncoder picks the video encoder honoring the hardware policy.
// A configured-but-unusable backend never fails the session: it falls
// back to software with a visible note (D-031).
func chooseEncoder(settings contracts.TranscodeSettings, codec string, report mediaReport, caps capabilities, video *stream) (encoder, backend string, hwDecode bool, fallback string, err error) {
	software := map[string][]string{
		contracts.VideoCodecH264: {"libx264"},
		contracts.VideoCodecHEVC: {"libx265"},
		contracts.VideoCodecAV1:  {"libsvtav1", "libaom-av1"},
	}
	hwNames := map[string]map[string]string{
		contracts.HWVAAPI: {
			contracts.VideoCodecH264: "h264_vaapi", contracts.VideoCodecHEVC: "hevc_vaapi", contracts.VideoCodecAV1: "av1_vaapi",
		},
		contracts.HWNVENC: {
			contracts.VideoCodecH264: "h264_nvenc", contracts.VideoCodecHEVC: "hevc_nvenc", contracts.VideoCodecAV1: "av1_nvenc",
		},
		contracts.HWQSV: {
			contracts.VideoCodecH264: "h264_qsv", contracts.VideoCodecHEVC: "hevc_qsv", contracts.VideoCodecAV1: "av1_qsv",
		},
		contracts.HWAMF: {
			contracts.VideoCodecH264: "h264_amf", contracts.VideoCodecHEVC: "hevc_amf", contracts.VideoCodecAV1: "av1_amf",
		},
		contracts.HWV4L2M2M: {
			contracts.VideoCodecH264: "h264_v4l2m2m", contracts.VideoCodecHEVC: "hevc_v4l2m2m",
		},
		contracts.HWVideoToolbox: {
			contracts.VideoCodecH264: "h264_videotoolbox", contracts.VideoCodecHEVC: "hevc_videotoolbox",
		},
		// Rockchip's media processing pipeline (Jellyfin's RKMPP method).
		contracts.HWRKMPP: {
			contracts.VideoCodecH264: "h264_rkmpp", contracts.VideoCodecHEVC: "hevc_rkmpp",
		},
	}

	backend = settings.HardwareAcceleration
	if backend == contracts.HWNone {
		backend = ""
	}
	decodeAllowed := containsCodec(settings.HardwareDecodeCodecs, strings.ToLower(streamCodecName(report)))
	// 10-bit sources need their own opt-in per codec: a backend that
	// cannot handle them would either fail or produce wrong colors.
	if decodeAllowed && video != nil && video.is10Bit() {
		switch strings.ToLower(video.CodecName) {
		case "hevc", "h265":
			if !settings.HardwareDecode10BitHEVC {
				decodeAllowed = false
				fallback = joinNotes(fallback, "10-bit HEVC decoding is disabled; decoded in software")
			}
		case "vp9":
			if !settings.HardwareDecode10BitVP9 {
				decodeAllowed = false
				fallback = joinNotes(fallback, "10-bit VP9 decoding is disabled; decoded in software")
			}
		}
	}
	if backend != "" && settings.HardwareEncode {
		want := hwNames[backend][codec]
		switch {
		case want == "":
			fallback = "hardware " + backend + " cannot encode " + codec + "; used software"
		case !caps.hasEncoder(want):
			fallback = "hardware encoder " + want + " unavailable; used software"
		case !caps.Hardware[backend]:
			fallback = "hardware backend " + backend + " failed its probe; used software"
		default:
			return want, backend, decodeAllowed, "", nil
		}
		backend = ""
	}
	for _, candidate := range software[codec] {
		if caps.hasEncoder(candidate) {
			// Decode-only hardware still offloads the decode half when
			// the operator enabled acceleration but not hardware encode.
			hwDecode = backend != "" && decodeAllowed && caps.Hardware[backend]
			return candidate, "", hwDecode, fallback, nil
		}
	}
	// Without a working probe the conservative default still exists on
	// every real ffmpeg build.
	return software[codec][0], "", false, fallback, nil
}

func containsCodec(list []string, codec string) bool {
	for _, c := range list {
		if c == codec {
			return true
		}
	}
	return false
}

func streamCodecName(report mediaReport) string {
	if v := firstVideo(report); v != nil {
		return v.CodecName
	}
	return ""
}

// probeCapabilities inspects the ffmpeg build: encoder/filter/hwaccel
// lists plus real execution probes for tone mapping and the configured
// hardware backend. Results are cached briefly because probing spawns
// ffmpeg.
func (t *Transcoder) probeCapabilities(settings contracts.TranscodeSettings) capabilities {
	key := settings.FFmpegPath + "|" + settings.HardwareAcceleration
	t.capMu.Lock()
	if t.caps.ProbedAt.After(t.now().Add(-capTTL)) && t.capKey == key {
		caps := t.caps
		t.capMu.Unlock()
		return caps
	}
	t.capMu.Unlock()

	caps := probeBuild(settings, t.now)

	t.capMu.Lock()
	t.caps, t.capKey = caps, key
	t.capMu.Unlock()
	return caps
}

// probeBuild runs the probe with no cache; doctor and the settings API
// both need a stateless reading of the local ffmpeg.
func probeBuild(settings contracts.TranscodeSettings, now func() time.Time) capabilities {
	if now == nil {
		now = time.Now
	}
	caps := capabilities{
		Encoders: map[string]bool{}, Filters: map[string]bool{},
		Hwaccels: map[string]bool{}, Hardware: map[string]bool{},
		ProbedAt: now(), FFmpeg: ffmpegBinary(settings),
	}
	ffmpeg := ffmpegBinary(settings)
	if out, err := exec.Command(ffmpeg, "-hide_banner", "-encoders").Output(); err == nil {
		parseFFmpegList(out, caps.Encoders)
	}
	if out, err := exec.Command(ffmpeg, "-hide_banner", "-filters").Output(); err == nil {
		parseFFmpegList(out, caps.Filters)
	}
	if out, err := exec.Command(ffmpeg, "-hide_banner", "-hwaccels").Output(); err == nil {
		parseFFmpegList(out, caps.Hwaccels)
	}
	caps.ToneMap = probeToneMap(ffmpeg, false)
	caps.ToneMap2390 = probeToneMap(ffmpeg, true)
	if backend := settings.HardwareAcceleration; backend != contracts.HWNone {
		caps.Hardware[backend] = probeHardware(ffmpeg, backend, caps, hardwareDevice(settings))
	}
	return caps
}

// Probe runs the capability probe without a server, for `lain doctor`.
func Probe(settings contracts.TranscodeSettings) CapabilitiesReport {
	return reportFromCaps(probeBuild(settings.Normalize(), time.Now))
}

const capTTL = 30 * time.Second

// parseFFmpegList extracts the tool name from each listed line. The
// -encoders and -filters listings put flags first and the name second;
// -hwaccels lists bare names.
func parseFFmpegList(out []byte, set map[string]bool) {
	for _, line := range strings.Split(string(out), "\n") {
		fields := strings.Fields(strings.TrimSpace(line))
		switch {
		case len(fields) == 1:
			set[fields[0]] = true
		case len(fields) >= 2:
			name := fields[1]
			if name == "=" || strings.Contains(name, ":") {
				continue
			}
			set[name] = true
		}
	}
}

func probeToneMap(ffmpeg string, bt2390 bool) bool {
	chain := "format=gbrpf32le,zscale=transferin=smpte2084:primariesin=bt2020:matrixin=bt2020nc:transfer=linear:primaries=bt2020:matrix=bt2020nc,tonemap=tonemap=hable:desat=0,zscale=transferin=linear:primariesin=bt2020:matrixin=bt2020nc:transfer=bt709:primaries=bt709:matrix=bt709:range=limited,format=yuv420p"
	args := []string{"-hide_banner", "-loglevel", "error"}
	if bt2390 {
		args = append(args, "-init_hw_device", "vulkan=vk")
		chain = "libplacebo=tonemapping=bt.2390:colorspace=bt709:color_primaries=bt709:color_trc=bt709:range=tv:format=yuv420p"
	}
	args = append(args,
		"-f", "lavfi", "-i", "testsrc2=size=64x64:rate=5:duration=0.2",
		"-vf", chain, "-frames:v", "1", "-f", "null", "-")
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	return exec.CommandContext(ctx, ffmpeg, args...).Run() == nil
}

func probeHardware(ffmpeg, backend string, caps capabilities, device string) bool {
	var args []string
	switch backend {
	case contracts.HWVAAPI:
		args = []string{"-init_hw_device", "vaapi=va:" + device,
			"-f", "lavfi", "-i", "testsrc2=size=64x64:rate=5:duration=0.2",
			"-vf", "format=nv12,hwupload", "-c:v", "h264_vaapi", "-frames:v", "1", "-f", "null", "-"}
	case contracts.HWNVENC:
		args = []string{"-init_hw_device", "cuda",
			"-f", "lavfi", "-i", "testsrc2=size=64x64:rate=5:duration=0.2",
			"-c:v", "h264_nvenc", "-frames:v", "1", "-f", "null", "-"}
	case contracts.HWQSV:
		args = []string{"-init_hw_device", "qsv=hw",
			"-f", "lavfi", "-i", "testsrc2=size=64x64:rate=5:duration=0.2",
			"-c:v", "h264_qsv", "-frames:v", "1", "-f", "null", "-"}
	case contracts.HWAMF:
		args = []string{"-f", "lavfi", "-i", "testsrc2=size=64x64:rate=5:duration=0.2",
			"-c:v", "h264_amf", "-frames:v", "1", "-f", "null", "-"}
	case contracts.HWV4L2M2M:
		args = []string{"-f", "lavfi", "-i", "testsrc2=size=64x64:rate=5:duration=0.2",
			"-c:v", "h264_v4l2m2m", "-frames:v", "1", "-f", "null", "-"}
	case contracts.HWVideoToolbox:
		args = []string{"-f", "lavfi", "-i", "testsrc2=size=64x64:rate=5:duration=0.2",
			"-c:v", "h264_videotoolbox", "-frames:v", "1", "-f", "null", "-"}
	case contracts.HWRKMPP:
		args = []string{"-init_hw_device", "rkmpp=hw",
			"-f", "lavfi", "-i", "testsrc2=size=64x64:rate=5:duration=0.2",
			"-vf", "format=nv12,hwupload", "-c:v", "h264_rkmpp", "-frames:v", "1", "-f", "null", "-"}
	default:
		return false
	}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	return exec.CommandContext(ctx, ffmpeg, append([]string{"-hide_banner", "-loglevel", "error"}, args...)...).Run() == nil
}

func ffmpegBinary(settings contracts.TranscodeSettings) string {
	if settings.FFmpegPath != "" {
		return settings.FFmpegPath
	}
	return "ffmpeg"
}

// hardwareDevice is the VA-API render node: the operator's choice, or the
// shipped default when unset.
func hardwareDevice(settings contracts.TranscodeSettings) string {
	if device := strings.TrimSpace(settings.HardwareDevice); device != "" {
		return device
	}
	return contracts.DefaultHardwareDevice
}

func ffprobeBinary(settings contracts.TranscodeSettings) string {
	if settings.FFprobePath != "" {
		return settings.FFprobePath
	}
	return "ffprobe"
}
