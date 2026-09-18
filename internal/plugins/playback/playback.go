// Package playback plans how a client should play an item.
// The planner returns references, never bytes: media streaming stays on
// the data gateway (HTTP Range), never inside plugin calls.
package playback

import (
	"path/filepath"
	"strings"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

// ID is the built-in playback planner provider id.
const ID = "lain-playback-default"

// Planner answers lain.playback.plan@1.
type Planner struct{}

func (Planner) ID() string             { return ID }
func (Planner) Capabilities() []string { return []string{contracts.CapPlaybackPlan} }
func (Planner) Health() error          { return nil }

func (Planner) Invoke(cap string, input any) (any, error) {
	if cap != contracts.CapPlaybackPlan {
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
	in, ok := input.(PlanInput)
	if !ok {
		return nil, &core.Error{Code: "invalid-message", Msg: "PlanInput required"}
	}
	return Plan(in), nil
}

// PlanInput resolves the opaque asset reference to a file.
type PlanInput struct {
	Request contracts.PlanRequest `json:"request"`
	// FilePath is supplied by the gateway after catalog lookup and
	// authorization; plugins never browse the filesystem for it.
	FilePath string `json:"file_path"`
	// MediaInfo is optional so old gateways/providers remain compatible.
	// When present it replaces extension guesses for browser policy.
	MediaInfo *contracts.MediaInfo `json:"media_info,omitempty"`
	// ToneMap reports that the transcode pipeline can convert HDR to
	// SDR right now (tone mapping enabled and its probe passed). Without
	// it an HDR browser play stays honestly unavailable (D-030/D-042).
	ToneMap bool `json:"tone_map,omitempty"`
}

// Plan decides direct vs transcode. The mpv desktop always direct-plays;
// browsers play mp4/webm directly and get browser-safe MP4s for the rest
// through the transcode provider (D-023); only when no transcode
// provider is healthy does the gateway downgrade the plan to an honest
// transcode-required it cannot produce.
func Plan(in PlanInput) contracts.Plan {
	asset := "asset:" + in.Request.ItemID
	ext := strings.ToLower(strings.TrimPrefix(filepath.Ext(in.FilePath), "."))
	client := strings.ToLower(in.Request.Client)
	if strings.Contains(client, "mpv") || strings.Contains(client, "desktop") {
		return contracts.Plan{Mode: "direct", Asset: asset, Available: true}
	}
	if in.MediaInfo != nil {
		if hasHDR(*in.MediaInfo) {
			if in.ToneMap {
				return contracts.Plan{Mode: "transcode", Asset: asset, Available: true, Streams: in.MediaInfo.Streams}
			}
			return contracts.Plan{Mode: "transcode-required", Asset: asset, Available: false, Reason: "HDR browser playback needs tone mapping, which is unavailable on this server"}
		}
		if browserDirect(ext, *in.MediaInfo) {
			return contracts.Plan{Mode: "direct", Asset: asset, Available: true, Streams: in.MediaInfo.Streams}
		}
		return contracts.Plan{Mode: "transcode", Asset: asset, Available: true, Streams: in.MediaInfo.Streams}
	}
	switch ext {
	case "mp4", "m4v", "webm", "mp3", "ogg":
		return contracts.Plan{Mode: "direct", Asset: asset, Available: true}
	default:
		return contracts.Plan{Mode: "transcode", Asset: asset, Available: true}
	}
}

func hasHDR(info contracts.MediaInfo) bool {
	for _, stream := range info.Streams {
		if stream.Type != "video" {
			continue
		}
		switch strings.ToLower(stream.ColorTransfer) {
		case "smpte2084", "arib-std-b67":
			return true
		}
	}
	return false
}

func browserDirect(ext string, info contracts.MediaInfo) bool {
	var video *contracts.MediaStream
	var audio *contracts.MediaStream
	for i := range info.Streams {
		s := &info.Streams[i]
		switch s.Type {
		case "video":
			if video == nil {
				video = s
			}
		case "audio":
			if audio == nil || (!audio.Default && s.Default) {
				audio = s
			}
		}
	}
	if video == nil {
		return ext == "mp3" || ext == "ogg"
	}
	audioCodec := ""
	if audio != nil {
		audioCodec = strings.ToLower(audio.Codec)
	}
	switch ext {
	case "mp4", "m4v":
		pixel := strings.ToLower(video.PixelFormat)
		videoOK := strings.ToLower(video.Codec) == "h264" && (pixel == "yuv420p" || pixel == "yuvj420p")
		return videoOK && (audio == nil || audioCodec == "aac")
	case "webm":
		videoCodec := strings.ToLower(video.Codec)
		videoOK := videoCodec == "vp8" || videoCodec == "vp9" || videoCodec == "av1"
		return videoOK && (audio == nil || audioCodec == "opus" || audioCodec == "vorbis")
	default:
		return false
	}
}
