// ffmpeg argv construction for v3 sessions, plus execution with the
// visible software fallback D-031 requires: a hardware attempt that
// fails at runtime retries in software and reports why.
package transcode

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"

	"github.com/enrell/lain/internal/contracts"
)

// subtitlePlaceholder is replaced with the pre-extracted subtitle file
// (escaped for the filter graph) once the session directory is known.
const subtitlePlaceholder = "__SUBTITLE_FILE__"

// fontsPlaceholder is replaced with the escaped fallback font directory.
const fontsPlaceholder = "__FONTS_DIR__"

// subtitleFontOptions renders the burn-in font fallback: fontsdir is
// searched before the system font configuration, and a forced family
// covers styled tracks whose own font is missing. The family name is
// sanitized because it travels inside the filter graph.
func subtitleFontOptions(settings contracts.TranscodeSettings) string {
	if !settings.AllowsFallbackFonts() {
		return ""
	}
	var options strings.Builder
	if strings.TrimSpace(settings.FallbackFontPath) != "" {
		options.WriteString(":fontsdir=" + fontsPlaceholder)
	}
	if name := sanitizeFontName(settings.FallbackFontName); name != "" {
		options.WriteString(":force_style='Fontname=" + name + "'")
	}
	return options.String()
}

// sanitizeFontName keeps only characters a font family can contain, so a
// settings value can never inject filter-graph syntax.
func sanitizeFontName(name string) string {
	var b strings.Builder
	for _, r := range name {
		switch {
		case r >= 'a' && r <= 'z', r >= 'A' && r <= 'Z', r >= '0' && r <= '9',
			r == ' ', r == '-', r == '_', r == '+':
			b.WriteRune(r)
		}
	}
	return strings.TrimSpace(b.String())
}

func softwareEncoderFor(codec string, caps capabilities) string {
	switch codec {
	case contracts.VideoCodecHEVC:
		return "libx265"
	case contracts.VideoCodecAV1:
		if caps.hasEncoder("libsvtav1") {
			return "libsvtav1"
		}
		return "libaom-av1"
	default:
		return "libx264"
	}
}

func isVAAPIEncoder(name string) bool        { return strings.HasSuffix(name, "_vaapi") }
func isNVENCEncoder(name string) bool        { return strings.HasSuffix(name, "_nvenc") }
func isQSVEncoder(name string) bool          { return strings.HasSuffix(name, "_qsv") }
func isAMFEncoder(name string) bool          { return strings.HasSuffix(name, "_amf") }
func isV4L2Encoder(name string) bool         { return strings.HasSuffix(name, "_v4l2m2m") }
func isVideoToolboxEncoder(name string) bool { return strings.HasSuffix(name, "_videotoolbox") }

func nvencPreset(preset string) string {
	switch preset {
	case "ultrafast", "superfast", "veryfast":
		return "p1"
	case "faster", "fast":
		return "p2"
	case "medium":
		return "p4"
	case "slow":
		return "p5"
	case "slower":
		return "p6"
	case "veryslow":
		return "p7"
	default:
		return "p4"
	}
}

// hwaccelInputArgs offloads decoding when the operator enabled it and
// the source codec is allowed. Frames stay on the CPU unless the
// encoder needs otherwise, so filters and burn-in keep working.
func (p encodePlan) hwaccelInputArgs() []string {
	if !p.hwDecode {
		return nil
	}
	// Hardware encode carries its backend on the plan; decode-only
	// acceleration has none (chooseEncoder reports the encode backend), so
	// fall back to the operator's configured backend. Without this the
	// decode half stayed software even though the plan claimed hardware
	// decode — enabling acceleration without hardware encode did nothing.
	backend := p.hwBackend
	if backend == "" {
		backend = p.settings.HardwareAcceleration
	}
	switch backend {
	case contracts.HWVAAPI:
		return []string{"-hwaccel", "vaapi", "-hwaccel_device", hardwareDevice(p.settings)}
	case contracts.HWNVENC:
		return []string{"-hwaccel", "cuda"}
	case contracts.HWQSV:
		return []string{"-hwaccel", "qsv"}
	case contracts.HWVideoToolbox:
		return []string{"-hwaccel", "videotoolbox"}
	case contracts.HWRKMPP:
		return []string{"-hwaccel", "rkmpp"}
	default:
		return nil
	}
}

// hardwareLabel names the backend actually in use, for the session status:
// the encode backend when hardware encoding is on, otherwise the configured
// backend when only decoding is offloaded, and "" when everything is CPU.
func (p encodePlan) hardwareLabel() string {
	if p.hwBackend != "" {
		return p.hwBackend
	}
	if p.hwDecode {
		return p.settings.HardwareAcceleration
	}
	return ""
}

// ffmpegArgs builds the complete argv for one plan. output is the
// progressive MP4 temp path, or the HLS raw playlist path.
func (p encodePlan) ffmpegArgs(output, subtitleFile string, onProgress bool) []string {
	args := []string{"-hide_banner", "-loglevel", "error", "-nostdin"}
	if onProgress {
		args = append(args, "-progress", "pipe:1", "-nostats")
	}
	args = append(args, p.hwaccelInputArgs()...)
	if p.spec.StartSec > 0 {
		// Input seeking: the session starts at the requested position and
		// ffmpeg shifts the produced timestamps to zero.
		args = append(args, "-ss", strconv.FormatFloat(p.spec.StartSec, 'f', 3, 64))
	}
	args = append(args, "-i", p.spec.Path)

	if p.complexFilter != "" {
		chain := strings.ReplaceAll(p.complexFilter, subtitlePlaceholder, escapeFilterPath(subtitleFile))
		chain = strings.ReplaceAll(chain, fontsPlaceholder, escapeFilterPath(p.settings.FallbackFontPath))
		args = append(args, "-filter_complex", chain, "-map", "[vout]")
	} else {
		args = append(args, "-map", fmt.Sprintf("0:%d", p.video.Index))
		if len(p.filters) > 0 {
			chain := strings.Join(p.filters, ",")
			chain = strings.ReplaceAll(chain, subtitlePlaceholder, escapeFilterPath(subtitleFile))
			chain = strings.ReplaceAll(chain, fontsPlaceholder, escapeFilterPath(p.settings.FallbackFontPath))
			args = append(args, "-vf", chain)
		}
	}
	if p.audio != nil {
		args = append(args, "-map", fmt.Sprintf("0:%d", p.audio.Index))
	} else {
		args = append(args, "-an")
	}
	if p.subtitleMode != contracts.SubtitleModeBurn {
		args = append(args, "-sn")
	}

	// Video codec and quality.
	if p.copyVideo {
		args = append(args, "-c:v", "copy")
	} else {
		args = append(args, "-c:v", p.encoder)
		args = append(args, p.videoQualityArgs()...)
		if !isVAAPIEncoder(p.encoder) && !isVideoToolboxEncoder(p.encoder) {
			args = append(args, "-pix_fmt", "yuv420p")
		}
	}

	// Audio codec and quality.
	if p.audio == nil {
		// nothing
	} else if p.copyAudio {
		args = append(args, "-c:a", "copy")
	} else {
		args = append(args, "-c:a", p.audioCodec)
		args = append(args, p.audioQualityArgs()...)
		if p.downmix {
			args = append(args, "-ac", "2")
		}
		if af := p.audioFilters(); af != "" {
			args = append(args, "-af", af)
		}
	}

	// Output-side muxer/encoder bounds. They change speed and failure
	// behavior, never the produced bytes.
	if p.settings.ThreadCount > 0 {
		args = append(args, "-threads", strconv.Itoa(p.settings.ThreadCount))
	}
	if p.settings.MuxingQueueSize > 0 {
		args = append(args, "-max_muxing_queue_size", strconv.Itoa(p.settings.MuxingQueueSize))
	}

	if p.isHLS() {
		dir := filepath.Dir(output)
		args = append(args, p.hlsArgs(dir, output)...)
	} else {
		args = append(args, "-movflags", "+faststart", "-f", "mp4", "-y", output)
	}
	return args
}

// hlsArgs builds the HLS muxer arguments. For encodes the segment
// length is only meaningful with keyframes at the boundaries, so the
// GOP is pinned to the segment length (and scene-cut keyframes
// disabled) — otherwise ffmpeg can only cut wherever the encoder
// happened to place a keyframe, which is what makes seeking in HLS
// unpredictable. Stream copy cannot move keyframes; there the source
// GOP decides the segment length, which the playlist then reports
// honestly through #EXT-X-TARGETDURATION.
func (p encodePlan) hlsArgs(dir, output string) []string {
	seg := strconv.Itoa(p.settings.HLSSegmentSeconds)
	container := p.settings.HLSSegmentContainer
	if container == "" {
		container = contracts.HLSSegmentFMP4
	}
	args := []string{
		"-f", "hls",
		"-hls_time", seg,
		"-hls_playlist_type", "event",
		"-hls_list_size", "0",
	}
	if container == contracts.HLSSegmentTS {
		// Classic MPEG-TS segments: no init segment, .ts files.
		args = append(args,
			"-hls_segment_type", "mpegts",
			"-hls_segment_filename", filepath.Join(dir, "seg%05d.ts"),
		)
	} else {
		args = append(args,
			"-hls_segment_type", "fmp4",
			"-hls_fmp4_init_filename", "init.mp4",
			"-hls_segment_filename", filepath.Join(dir, "seg%05d.m4s"),
		)
	}
	args = append(args, "-hls_flags", "independent_segments+temp_file")
	if !p.copyVideo {
		seconds := p.settings.HLSSegmentSeconds
		args = append(args, "-force_key_frames", fmt.Sprintf("expr:gte(t,n_forced*%d)", seconds))
		if fps := p.video.frameRate(); fps > 0 {
			gop := int(fps*float64(seconds) + 0.5)
			if gop < 1 {
				gop = 1
			}
			args = append(args, "-g", strconv.Itoa(gop), "-keyint_min", strconv.Itoa(gop))
			if p.encoder == "libx264" || p.encoder == "libx265" {
				args = append(args, "-sc_threshold", "0")
			}
		}
	}
	return append(args, output)
}

func (p encodePlan) videoQualityArgs() []string {
	preset := p.preset
	crf := strconv.Itoa(p.crf)
	cap := func() []string {
		if p.maxBitrateKbps <= 0 {
			return nil
		}
		return []string{"-maxrate", fmt.Sprintf("%dk", p.maxBitrateKbps), "-bufsize", fmt.Sprintf("%dk", p.maxBitrateKbps*2)}
	}
	switch {
	case isVAAPIEncoder(p.encoder):
		return append([]string{"-qp", crf}, cap()...)
	case isNVENCEncoder(p.encoder):
		args := []string{"-preset", nvencPreset(preset), "-cq", crf, "-rc", "vbr"}
		return append(args, cap()...)
	case isQSVEncoder(p.encoder):
		args := []string{"-global_quality", crf}
		if p.settings.HardwareLowPower {
			// Jellyfin's "Intel Low-Power H.264/HEVC hardware encoder".
			args = append(args, "-low_power", "1")
		}
		return append(args, cap()...)
	case isAMFEncoder(p.encoder):
		args := []string{"-quality", "speed", "-qp_i", crf, "-qp_p", crf}
		return append(args, cap()...)
	case isV4L2Encoder(p.encoder):
		if p.maxBitrateKbps > 0 {
			return []string{"-b:v", fmt.Sprintf("%dk", p.maxBitrateKbps)}
		}
		return []string{"-b:v", "4000k"}
	case isVideoToolboxEncoder(p.encoder):
		// VideoToolbox takes a 0..100 quality scale; invert the CRF
		// so lower CRF keeps meaning higher quality.
		q := 100 - p.crf*2
		if q < 0 {
			q = 0
		}
		args := []string{"-q:v", strconv.Itoa(q)}
		return append(args, cap()...)
	case p.encoder == "libsvtav1":
		// The AV1 preset is the operator's choice, not a constant.
		args := []string{"-preset", svtAV1Preset(preset), "-crf", crf}
		return append(args, cap()...)
	case p.encoder == "libaom-av1":
		// CRF (constant quality) normally pins -b:v 0, but libaom
		// refuses any rate-control parameters without a bitrate, so a
		// requested ceiling switches to constrained quality carried by
		// -b:v instead of 0.
		cpu := libaomCPUUsed(preset)
		if p.maxBitrateKbps > 0 {
			rate := fmt.Sprintf("%dk", p.maxBitrateKbps)
			return []string{"-cpu-used", cpu, "-crf", crf, "-b:v", rate, "-maxrate", rate, "-bufsize", fmt.Sprintf("%dk", p.maxBitrateKbps*2)}
		}
		return []string{"-cpu-used", cpu, "-crf", crf, "-b:v", "0"}
	default: // libx264, libx265
		args := []string{"-preset", preset, "-crf", crf}
		return append(args, cap()...)
	}
}

// svtAV1Preset maps the shared x264-style preset ladder onto SVT-AV1's
// numeric presets (0 slowest .. 13 fastest). The scale runs the opposite
// way, so the mapping is explicit rather than a cast.
func svtAV1Preset(preset string) string {
	switch preset {
	case "ultrafast":
		return "13"
	case "superfast":
		return "12"
	case "veryfast":
		return "10"
	case "faster":
		return "9"
	case "fast":
		return "8"
	case "medium":
		return "6"
	case "slow":
		return "4"
	case "slower":
		return "2"
	case "veryslow":
		return "0"
	default:
		return "8"
	}
}

// libaomCPUUsed maps the preset ladder onto libaom's 0..8 cpu-used
// (0 slowest .. 8 fastest).
func libaomCPUUsed(preset string) string {
	switch preset {
	case "ultrafast", "superfast":
		return "8"
	case "veryfast":
		return "7"
	case "faster", "fast":
		return "6"
	case "medium":
		return "5"
	case "slow":
		return "4"
	case "slower":
		return "3"
	case "veryslow":
		return "2"
	default:
		return "8"
	}
}

// audioQualityArgs picks CBR or VBR for the audio encoder. VBR uses
// ffmpeg's native AAC quality step, mapped from the configured bitrate
// by contracts.AudioVBRQuality so the result is predictable.
func (p encodePlan) audioQualityArgs() []string {
	if p.settings.AudioVBR && p.audioCodec == contracts.AudioCodecAAC {
		return []string{"-q:a", strconv.FormatFloat(contracts.AudioVBRQuality(p.audioBitrateKbps), 'f', -1, 64)}
	}
	return []string{"-b:a", fmt.Sprintf("%dk", p.audioBitrateKbps)}
}

// audioFilters applies the downmix gain and, when selected, lain's own
// nightmode matrix. `none` leaves ffmpeg's default downmix coefficients
// alone, so the setting is only ever additive.
func (p encodePlan) audioFilters() string {
	var chain []string
	if p.downmix && p.settings.DownmixStereoAlgorithm == contracts.DownmixNightmode {
		// lain's nightmode: lift the centre (dialogue) channel and keep
		// only a hint of surrounds and LFE. These coefficients are
		// lain's own, documented here and in docs/CONTRACTS.md; they are
		// not Jellyfin's Dave750/NightmodeDialogue matrices.
		chain = append(chain, "pan=stereo|c0=0.4*c0+0.4*c1+0.8*c2+0.2*c4|c1=0.4*c0+0.4*c1+0.8*c2+0.2*c5")
	}
	if p.downmix && p.settings.DownmixAudioBoost > 0 && p.settings.DownmixAudioBoost != 1 {
		chain = append(chain, fmt.Sprintf("volume=%.2f", p.settings.DownmixAudioBoost))
	}
	return strings.Join(chain, ",")
}

// escapeFilterPath escapes a filesystem path for ffmpeg filter syntax.
func escapeFilterPath(path string) string {
	r := strings.NewReplacer(
		`\`, `\\`,
		`:`, `\:`,
		`'`, `\'`,
		`[`, `\[`,
		`]`, `\]`,
		`,`, `\,`,
		`;`, `\;`,
	)
	return r.Replace(path)
}

// softwareFallback returns a plan that reruns the same session in
// software. Filters stay; only the encoder and hwaccel inputs change.
func (p encodePlan) softwareFallback(caps capabilities) encodePlan {
	out := p
	out.encoder = softwareEncoderFor(p.videoCodec, caps)
	out.hwBackend = ""
	out.hwDecode = false
	filters := make([]string, 0, len(out.filters))
	for _, f := range out.filters {
		if f == "hwupload" || f == "format=nv12" {
			continue
		}
		filters = append(filters, f)
	}
	out.filters = filters
	// The burn-in path bakes the same upload tokens into complexFilter
	// instead of filters; without this the software retry replays the
	// hardware graph and fails again.
	out.complexFilter = stripHardwareUpload(out.complexFilter)
	out.fallback = joinNotes(out.fallback, "hardware encode failed; used software")
	return out
}

// stripHardwareUpload removes the VAAPI upload tokens from a filter
// chain. resolveFilters always appends "format=nv12" and "hwupload"
// together, so both spellings (with and without a leading comma) cover
// the chain positions they can occupy.
func stripHardwareUpload(chain string) string {
	if chain == "" {
		return chain
	}
	chain = strings.ReplaceAll(chain, ",format=nv12,hwupload", "")
	chain = strings.ReplaceAll(chain, "format=nv12,hwupload", "")
	return chain
}

// execPlan runs one ffmpeg invocation, writing atomically when the
// output is a single file. onCmd hands the running process to the
// caller so sessions can be throttled or cancelled.
func (t *Transcoder) execPlan(ctx context.Context, p encodePlan, output string, subtitleFile string, onSample func(progressSample), onCmd func(*exec.Cmd)) error {
	ffmpeg := ffmpegBinary(p.settings)
	args := p.ffmpegArgs(output, subtitleFile, onSample != nil)
	if !p.isHLS() {
		_ = os.Remove(output)
	}
	cmd := exec.CommandContext(ctx, ffmpeg, args...)
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return err
	}
	var stderr strings.Builder
	cmd.Stderr = &stderr
	if onCmd != nil {
		onCmd(cmd)
	}
	if err := cmd.Start(); err != nil {
		return fmt.Errorf("%v: %s", err, strings.TrimSpace(stderr.String()))
	}
	if onSample != nil {
		consumeProgress(stdout, p.report.durationSeconds(), onSample)
	}
	if err := cmd.Wait(); err != nil {
		return fmt.Errorf("%v: %s", err, strings.TrimSpace(stderr.String()))
	}
	if p.isHLS() {
		return nil
	}
	fi, err := os.Stat(output)
	if err != nil || fi.Size() == 0 {
		return fmt.Errorf("no output produced")
	}
	return nil
}
