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
// browsers play what they can decode themselves and get browser-safe
// output for the rest through the transcode provider (D-023). What a
// browser can decode is either guessed conservatively from the container
// (D-030) or reported by the client itself (D-058) — see directPlay.
// Only when no transcode provider is healthy does the gateway downgrade
// the plan to an honest transcode-required it cannot produce.
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
		if directPlay(ext, *in.MediaInfo, in.Request.Capabilities) {
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

// directPlay answers whether a browser can decode this file as it is.
//
// Without a reported capability set — an older client, curl, the desktop
// — the conservative per-container rules decide (D-030). With one, the
// client's own claim decides, but only inside this server's vocabulary
// and only for the tracks a player would actually use (D-058): a claim
// cannot license a container this server has no Content-Type for, or a
// profile no browser decodes whatever the client says.
func directPlay(ext string, info contracts.MediaInfo, caps *contracts.ClientCapabilities) bool {
	if caps == nil {
		return browserDirect(ext, info)
	}
	return reportedDirect(ext, info, caps)
}

// reportedDirect applies a client's claim to the probed streams. The
// container must open and every track the player would use must decode
// in that container — the pairing is what keeps "HEVC decodes in MP4"
// from licensing "HEVC in Matroska", which is a black screen.
//
// Tracks the player would not select are not consulted: a file whose
// default audio is AAC and whose second audio track is AC-3 still
// direct-plays, exactly as the browser ignores the track it cannot use.
func reportedDirect(ext string, info contracts.MediaInfo, caps *contracts.ClientCapabilities) bool {
	container, ok := contracts.DirectContainer(ext)
	if !ok || !caps.Has(container) {
		return false
	}
	video, audio := selectTracks(info)
	if video == nil && audio == nil {
		// Nothing was probed, so nothing can be claimed: a container
		// that opens is not evidence that a stream decodes.
		return false
	}
	if video != nil {
		family, ok := contracts.CapabilityCodec(video.Codec)
		if !ok || !caps.Has(container+"/"+family) || !videoDecodable(video) {
			return false
		}
	}
	if audio != nil {
		family, ok := contracts.CapabilityCodec(audio.Codec)
		if !ok || !caps.Has(container+"/"+family) {
			return false
		}
	}
	return true
}

// videoDecodable is the floor a client's claim cannot raise. H.264 High
// 10, 4:2:2 and 4:4:4 have no browser decoder at all, and a codec string
// cannot express that — the profile bits a client would have to encode
// are exactly the ones canPlayType does not parse, so it answers
// "probably" for them. The probed pixel format decides instead. VP9 and
// AV1 do decode 10-bit, so the check is H.264-only.
func videoDecodable(video *contracts.MediaStream) bool {
	if strings.ToLower(video.Codec) != "h264" {
		return true
	}
	switch strings.ToLower(video.PixelFormat) {
	case "yuv420p", "yuvj420p":
		return true
	default:
		return false
	}
}

// selectTracks picks the streams a player would use: the first video
// track and the default audio track (the first one when none is marked).
func selectTracks(info contracts.MediaInfo) (video, audio *contracts.MediaStream) {
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
	return video, audio
}

func browserDirect(ext string, info contracts.MediaInfo) bool {
	video, audio := selectTracks(info)
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
		return videoOK && (audio == nil || webSafeMP4Audio(audioCodec))
	case "webm":
		videoCodec := strings.ToLower(video.Codec)
		videoOK := videoCodec == "vp8" || videoCodec == "vp9" || videoCodec == "av1"
		return videoOK && (audio == nil || audioCodec == "opus" || audioCodec == "vorbis")
	default:
		return false
	}
}

// webSafeMP4Audio is the audio set a browser is trusted to decode inside
// an MP4 without re-encoding: AAC and MP3 are universal. AC-3/E-AC-3 are
// deliberately excluded even though Jellyfin's web profile lists them —
// Chromium on Linux cannot decode them, and D-030 only direct-plays
// verified web-safe streams.
func webSafeMP4Audio(codec string) bool {
	switch codec {
	case "aac", "mp3":
		return true
	default:
		return false
	}
}
