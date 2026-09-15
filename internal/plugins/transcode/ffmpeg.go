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
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
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
	Default        int
	Disposition    struct {
		Default int `json:"default"`
	} `json:"disposition"`
}

type mediaReport struct {
	Streams []stream `json:"streams"`
}

func probeMedia(path string) (mediaReport, error) {
	cmd := exec.Command("ffprobe",
		"-v", "error",
		"-show_entries", "stream=index,codec_type,codec_name,pix_fmt,color_transfer,color_primaries:stream_disposition=default",
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
// writing atomically: readers only ever see a complete MP4.
func (t *Transcoder) convertMedia(spec sourceSpec, out string) (string, error) {
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

	ctx, cancel := context.WithTimeout(t.ctx, timeout)
	defer cancel()
	cmd := exec.CommandContext(ctx, "ffmpeg", args...)
	if output, err := cmd.CombinedOutput(); err != nil {
		_ = os.Remove(tmp)
		return "", fmt.Errorf("%v: %s", err, strings.TrimSpace(string(output)))
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

func (t *Transcoder) extractSubtitle(spec sourceSpec, out string) error {
	if spec.SubtitleStream == nil {
		return nil
	}
	tmp := out + ".tmp"
	_ = os.Remove(tmp)
	ctx, cancel := context.WithTimeout(t.ctx, 5*time.Minute)
	defer cancel()
	cmd := exec.CommandContext(ctx, "ffmpeg",
		"-hide_banner", "-loglevel", "error", "-nostdin", "-i", spec.Path,
		"-map", fmt.Sprintf("0:%d", *spec.SubtitleStream),
		"-f", "webvtt", "-y", tmp)
	if output, err := cmd.CombinedOutput(); err != nil {
		_ = os.Remove(tmp)
		return fmt.Errorf("%v: %s", err, strings.TrimSpace(string(output)))
	}
	fi, err := os.Stat(tmp)
	if err != nil || fi.Size() == 0 {
		_ = os.Remove(tmp)
		return fmt.Errorf("no WebVTT produced")
	}
	return os.Rename(tmp, out)
}
