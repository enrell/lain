// Package transcode prepares browser-playable MP4s from catalog files
// with ffmpeg. It returns paths, never bytes: the gateway streams the
// prepared file from disk exactly like media (docs/ARCHITECTURE.md).
//
// V1 (D-023) retains its synchronous first-video/all-audio behavior.
// V2 (D-028/D-030) selects one audio stream and refuses HDR rather
// than producing incorrect colors. Both write complete +faststart MP4s
// for Range delivery; text subtitles are prepared as sidecars elsewhere.
package transcode

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/exec"
	"strconv"
	"strings"
	"time"

	"github.com/enrell/lain/internal/core"
)

const (
	// ID is the built-in ffmpeg transcode provider id.
	ID = "lain-transcode-ffmpeg"

	legacyProfile = "mp4-compat-v1"
	// profile pins the current output contract. Any encode policy change
	// must rename it so old cache entries stop matching.
	profile = "web-mp4-sdr-v2"
	// subtitleProfile keys an on-demand WebVTT sidecar, which is not a
	// transcode and never matches a session entry.
	subtitleProfile = "subs-vtt-v1"

	// timeout bounds one preparation. A detached context (not the
	// HTTP request's) lets an abandoned first attempt still warm the
	// cache for the next visit.
	timeout = time.Hour
)

// stream describes one ffprobe stream entry; only the fields the
// copy-vs-encode decision needs.
type stream struct {
	Index          int    `json:"index"`
	CodecType      string `json:"codec_type"`
	CodecName      string `json:"codec_name"`
	PixelFormat    string `json:"pix_fmt"`
	ColorTransfer  string `json:"color_transfer"`
	ColorPrimaries string `json:"color_primaries"`
	ColorSpace     string `json:"color_space"`
	FieldOrder     string `json:"field_order"`
	Width          int    `json:"width"`
	Height         int    `json:"height"`
	Channels       int    `json:"channels"`
	BitRate        string `json:"bit_rate"`
	FrameRate      string `json:"r_frame_rate"`
	BitsPerRaw     string `json:"bits_per_raw_sample"`
	Default        int
	Disposition    struct {
		Default int `json:"default"`
	} `json:"disposition"`
}

type mediaReport struct {
	Streams []stream `json:"streams"`
	Format  struct {
		Duration   string `json:"duration"`
		BitRate    string `json:"bit_rate"`
		FormatName string `json:"format_name"`
	} `json:"format"`
}

// is10Bit reports whether the stream carries more than 8 bits per
// sample, which decides whether hardware decoding is allowed (Jellyfin's
// per-codec 10-bit toggles).
func (s stream) is10Bit() bool {
	if bits, err := strconv.Atoi(strings.TrimSpace(s.BitsPerRaw)); err == nil && bits > 8 {
		return true
	}
	pixel := strings.ToLower(s.PixelFormat)
	return strings.Contains(pixel, "10le") || strings.Contains(pixel, "10be") ||
		strings.HasPrefix(pixel, "p010") || strings.HasPrefix(pixel, "p410")
}

// frameRate parses ffprobe's "num/den" rate; 0 means unknown, and the
// caller then avoids inventing a GOP size.
func (s stream) frameRate() float64 {
	num, den, ok := strings.Cut(s.FrameRate, "/")
	if !ok {
		return 0
	}
	n, err1 := strconv.ParseFloat(num, 64)
	d, err2 := strconv.ParseFloat(den, 64)
	if err1 != nil || err2 != nil || d <= 0 || n <= 0 {
		return 0
	}
	return n / d
}

// progressSample is one reading of ffmpeg's -progress stream: the
// preparation fraction plus the live pipeline metrics the session list
// reports (Jellyfin's transcoding info).
type progressSample struct {
	Fraction    float64
	HasFraction bool
	FPS         float64
	Speed       float64
	BitrateKbps int
}

// durationSeconds is 0 when ffprobe could not report a usable duration;
// progress is then reported as indeterminate instead of invented.
func (r mediaReport) durationSeconds() float64 {
	sec, err := strconv.ParseFloat(r.Format.Duration, 64)
	if err != nil || sec <= 0 {
		return 0
	}
	return sec
}

func probeMedia(path string) (mediaReport, error) {
	return probeMediaWith("ffprobe", path)
}

func probeMediaWith(ffprobePath, path string) (mediaReport, error) {
	if ffprobePath == "" {
		ffprobePath = "ffprobe"
	}
	cmd := exec.Command(ffprobePath,
		"-v", "error",
		"-show_entries", "stream=index,codec_type,codec_name,pix_fmt,color_transfer,color_primaries,color_space,field_order,width,height,channels,bit_rate,r_frame_rate,bits_per_raw_sample:stream_disposition=default:format=duration,bit_rate,format_name",
		"-of", "json",
		path)
	raw, err := cmd.Output()
	if err != nil {
		return mediaReport{}, err
	}
	var report mediaReport
	if err := json.Unmarshal(raw, &report); err != nil {
		return mediaReport{}, err
	}
	for i := range report.Streams {
		report.Streams[i].Default = report.Streams[i].Disposition.Default
	}
	return report, nil
}

func legacyCopy(report mediaReport) (copyVideo, copyAudio bool) {
	var video, audio int
	copyAudio = true
	for _, s := range report.Streams {
		switch s.CodecType {
		case "video":
			if video == 0 && s.CodecName != "h264" {
				copyVideo = false
				copyAudio = false
				return
			}
			video++
		case "audio":
			if s.CodecName != "aac" {
				copyAudio = false
			}
			audio++
		}
	}
	if video == 0 {
		return false, false
	}
	if audio == 0 {
		copyAudio = true
	}
	return true, copyAudio
}

func methodName(copyVideo, copyAudio bool) string {
	if copyVideo && copyAudio {
		return "remux"
	}
	return "transcode"
}

func conversionArgs(spec sourceSpec, report mediaReport, tmp string) ([]string, string, error) {
	args := []string{"-hide_banner", "-loglevel", "error", "-nostdin", "-i", spec.Path}
	copyVideo, copyAudio := false, false
	if spec.Profile == legacyProfile {
		copyVideo, copyAudio = legacyCopy(report)
		args = append(args, "-map", "0:v:0", "-map", "0:a?")
	} else {
		var video *stream
		var firstAudio, defaultAudio *stream
		for i := range report.Streams {
			s := &report.Streams[i]
			switch s.CodecType {
			case "video":
				if video == nil {
					video = s
				}
			case "audio":
				if firstAudio == nil {
					firstAudio = s
				}
				if defaultAudio == nil && s.Default != 0 {
					defaultAudio = s
				}
			}
		}
		if video == nil {
			return nil, "", &core.Error{Code: "unsupported-media", Msg: "source has no video stream"}
		}
		if hdrVideo(*video) {
			return nil, "", &core.Error{Code: "unsupported-media", Msg: "HDR requires a tone-map profile"}
		}
		pixel := strings.ToLower(video.PixelFormat)
		copyVideo = strings.EqualFold(video.CodecName, "h264") && (pixel == "yuv420p" || pixel == "yuvj420p")
		selectedAudio := defaultAudio
		if selectedAudio == nil {
			selectedAudio = firstAudio
		}
		if spec.AudioStream != nil {
			selectedAudio = nil
			for i := range report.Streams {
				s := &report.Streams[i]
				if s.Index == *spec.AudioStream && s.CodecType == "audio" {
					selectedAudio = s
					break
				}
			}
			if selectedAudio == nil {
				return nil, "", &core.Error{Code: "invalid-message", Msg: "audio_stream is not an audio track"}
			}
		}
		args = append(args, "-map", fmt.Sprintf("0:%d", video.Index))
		if selectedAudio != nil {
			args = append(args, "-map", fmt.Sprintf("0:%d", selectedAudio.Index))
			copyAudio = strings.EqualFold(selectedAudio.CodecName, "aac")
		} else {
			args = append(args, "-an")
			copyAudio = true
		}
		if spec.SubtitleStream != nil {
			var selectedSubtitle *stream
			for i := range report.Streams {
				s := &report.Streams[i]
				if s.Index == *spec.SubtitleStream && s.CodecType == "subtitle" {
					selectedSubtitle = s
					break
				}
			}
			if selectedSubtitle == nil {
				return nil, "", &core.Error{Code: "invalid-message", Msg: "subtitle_stream is not a subtitle track"}
			}
			if !textSubtitleCodec(selectedSubtitle.CodecName) {
				return nil, "", &core.Error{Code: "unsupported-media", Msg: "selected subtitle cannot be converted to WebVTT"}
			}
		}
	}
	method := methodName(copyVideo, copyAudio)
	if copyVideo {
		args = append(args, "-c:v", "copy")
	} else {
		args = append(args, "-c:v", "libx264", "-preset", "veryfast", "-crf", "23",
			"-pix_fmt", "yuv420p")
	}
	if copyAudio {
		args = append(args, "-c:a", "copy")
	} else {
		args = append(args, "-c:a", "aac", "-b:a", "160k")
	}
	args = append(args, "-movflags", "+faststart", "-f", "mp4", "-y", tmp)
	return args, method, nil
}

func textSubtitleCodec(codec string) bool {
	switch strings.ToLower(codec) {
	case "subrip", "srt", "ass", "ssa", "webvtt", "mov_text":
		return true
	default:
		return false
	}
}

func hdrVideo(s stream) bool {
	switch strings.ToLower(s.ColorTransfer) {
	case "smpte2084", "arib-std-b67":
		return true
	default:
		return false
	}
}

// convertMedia builds the ffmpeg argv from probed streams and runs it,
// writing atomically: readers only ever see a complete MP4. When
// onSample is non-nil, ffmpeg's -progress stream is drained and the
// preparation fraction plus live metrics are reported as they advance.
func (t *Transcoder) convertMedia(spec sourceSpec, out string, onSample func(progressSample)) (string, error) {
	report, probeErr := probeMedia(spec.Path)
	if probeErr != nil && spec.Profile != legacyProfile {
		return "", &core.Error{Code: "dependency-unavailable", Msg: "ffprobe is required for the browser profile"}
	}
	tmp := out + ".tmp"
	_ = os.Remove(tmp)
	args, method, err := conversionArgs(spec, report, tmp)
	if err != nil {
		return "", err
	}
	if onSample != nil {
		args = append([]string{"-progress", "pipe:1", "-nostats"}, args...)
	}

	ctx, cancel := context.WithTimeout(t.ctx, timeout)
	defer cancel()
	cmd := exec.CommandContext(ctx, "ffmpeg", args...)
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return "", err
	}
	var stderr strings.Builder
	cmd.Stderr = &stderr
	if err := cmd.Start(); err != nil {
		_ = os.Remove(tmp)
		return "", fmt.Errorf("%v: %s", err, strings.TrimSpace(stderr.String()))
	}
	// Drain stdout while ffmpeg runs: the progress stream is written
	// there, and a full pipe would block the child. stderr keeps the
	// error text the failure path reports.
	consumeProgress(stdout, report.durationSeconds(), onSample)
	if err := cmd.Wait(); err != nil {
		_ = os.Remove(tmp)
		return "", fmt.Errorf("%v: %s", err, strings.TrimSpace(stderr.String()))
	}
	fi, err := os.Stat(tmp)
	if err != nil || fi.Size() == 0 {
		_ = os.Remove(tmp)
		return "", fmt.Errorf("no mp4 produced")
	}
	if err := os.Rename(tmp, out); err != nil {
		return "", err
	}
	return method, nil
}

// consumeProgress drains ffmpeg's -progress stream one block at a time:
// each block ends with a `progress=` line, and a block is only reported
// when it carries something true — a fraction of a probed duration or a
// live metric (fps, output bitrate, encoding speed). An unknown duration
// never invents a fraction (D-039).
func consumeProgress(r io.Reader, totalSec float64, onSample func(progressSample)) {
	if onSample == nil {
		return
	}
	scanner := bufio.NewScanner(r)
	var block progressSample
	var positionUS int64
	for scanner.Scan() {
		key, value, ok := strings.Cut(strings.TrimSpace(scanner.Text()), "=")
		if !ok {
			continue
		}
		switch key {
		case "out_time_us", "out_time_ms":
			if us, err := strconv.ParseInt(value, 10, 64); err == nil && us >= 0 {
				positionUS = us
			}
		case "fps":
			if fps, err := strconv.ParseFloat(value, 64); err == nil && fps > 0 {
				block.FPS = fps
			}
		case "speed":
			if speed, err := strconv.ParseFloat(strings.TrimSuffix(value, "x"), 64); err == nil && speed > 0 {
				block.Speed = speed
			}
		case "bitrate":
			if kbps, err := parseBitrateKbps(value); err == nil {
				block.BitrateKbps = kbps
			}
		case "progress":
			sample := block
			if totalSec > 0 {
				sample.Fraction = min(1, max(0, float64(positionUS)/(totalSec*1e6)))
				sample.HasFraction = true
			}
			if value == "end" {
				sample.Fraction = 1
				sample.HasFraction = true
			}
			if sample.HasFraction || sample.FPS > 0 || sample.BitrateKbps > 0 || sample.Speed > 0 {
				onSample(sample)
			}
			block = progressSample{}
		}
	}
}

// parseBitrateKbps reads ffmpeg's "1234.5kbits/s" style bitrate value.
func parseBitrateKbps(value string) (int, error) {
	trimmed := strings.TrimSpace(strings.TrimSuffix(strings.TrimSpace(value), "kbits/s"))
	trimmed = strings.TrimSuffix(trimmed, "kbit/s")
	if trimmed == "N/A" || trimmed == "" {
		return 0, strconv.ErrSyntax
	}
	kbps, err := strconv.ParseFloat(trimmed, 64)
	if err != nil {
		return 0, err
	}
	return int(kbps + 0.5), nil
}

// sourceKbps converts ffprobe's stream bit_rate (bits per second) to kbps;
// 0 when it is absent, so a cap never forces an encode blindly.
func sourceKbps(raw string) int {
	n, err := strconv.Atoi(strings.TrimSpace(raw))
	if err != nil || n <= 0 {
		return 0
	}
	return n / 1000
}

func (t *Transcoder) extractSubtitle(spec sourceSpec, out string) error {
	if spec.SubtitleStream == nil {
		return nil
	}
	return t.extractSubtitleStream(spec, *spec.SubtitleStream, out, "webvtt", "ffmpeg")
}

// extractSubtitleAs converts one subtitle stream to a standalone file
// (webvtt sidecar, or ass for burn-in rendering) using the configured
// ffmpeg binary.
func (t *Transcoder) extractSubtitleAs(spec sourceSpec, s stream, out, format string) error {
	return t.extractSubtitleStream(spec, s.Index, out, format, ffmpegBinary(spec.Settings))
}

func (t *Transcoder) extractSubtitleStream(spec sourceSpec, index int, out, format, ffmpeg string) error {
	tmp := out + ".tmp"
	_ = os.Remove(tmp)
	ctx, cancel := context.WithTimeout(t.ctx, 5*time.Minute)
	defer cancel()
	cmd := exec.CommandContext(ctx, ffmpeg,
		"-hide_banner", "-loglevel", "error", "-nostdin", "-i", spec.Path,
		"-map", fmt.Sprintf("0:%d", index),
		"-f", format, "-y", tmp)
	if output, err := cmd.CombinedOutput(); err != nil {
		_ = os.Remove(tmp)
		return fmt.Errorf("%v: %s", err, strings.TrimSpace(string(output)))
	}
	fi, err := os.Stat(tmp)
	if err != nil || fi.Size() == 0 {
		_ = os.Remove(tmp)
		return fmt.Errorf("no subtitle file produced")
	}
	return os.Rename(tmp, out)
}
