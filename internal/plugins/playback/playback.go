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
}

// Plan decides direct vs transcode. The mpv desktop always direct-plays;
// browsers facing mkv/ass/hevc report transcode-required (the transcode
// plugin is a planned provider; v0.1 reports honestly instead of
// faking a stream it cannot produce).
func Plan(in PlanInput) contracts.Plan {
	asset := "asset:" + in.Request.ItemID
	ext := strings.ToLower(strings.TrimPrefix(filepath.Ext(in.FilePath), "."))
	client := strings.ToLower(in.Request.Client)
	if strings.Contains(client, "mpv") || strings.Contains(client, "desktop") {
		return contracts.Plan{Mode: "direct", Asset: asset, Available: true}
	}
	switch ext {
	case "mp4", "m4v", "webm", "mp3", "ogg":
		return contracts.Plan{Mode: "direct", Asset: asset, Available: true}
	default:
		return contracts.Plan{
			Mode: "transcode-required", Asset: asset, Available: false,
			Reason: "container " + ext + " needs transcode for browser clients (no transcode provider installed)",
		}
	}
}
