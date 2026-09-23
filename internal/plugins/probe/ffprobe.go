// Package probe inspects technical media streams through the external
// ffprobe binary. It returns bounded metadata, never media bytes.
package probe

import (
	"context"
	"encoding/json"
	"fmt"
	"math"
	"os/exec"
	"strconv"
	"strings"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

const (
	ID      = "lain-probe-ffprobe"
	timeout = 20 * time.Second
)

type Provider struct{}

func (Provider) ID() string             { return ID }
func (Provider) Capabilities() []string { return []string{contracts.CapMediaProbe} }

func (Provider) Health() error {
	if _, err := exec.LookPath("ffprobe"); err != nil {
		return fmt.Errorf("ffprobe not found: %w", err)
	}
	return nil
}

func (Provider) Invoke(cap string, input any) (any, error) {
	if cap != contracts.CapMediaProbe {
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
	in, ok := input.(contracts.MediaProbeRequest)
	if !ok {
		return nil, &core.Error{Code: "invalid-message", Msg: "MediaProbeRequest required"}
	}
	if in.FilePath == "" {
		return nil, &core.Error{Code: "invalid-message", Msg: "file_path required"}
	}
	ctx, cancel := context.WithTimeout(context.Background(), timeout)
	defer cancel()
	cmd := exec.CommandContext(ctx, "ffprobe", "-v", "error", "-show_format", "-show_streams", "-of", "json", in.FilePath)
	raw, err := cmd.Output()
	if err != nil {
		return nil, &core.Error{Code: "dependency-unavailable", Msg: "media probe failed"}
	}
	info, err := parse(raw)
	if err != nil {
		return nil, &core.Error{Code: "dependency-unavailable", Msg: "invalid ffprobe output"}
	}
	return info, nil
}

type report struct {
	Format struct {
		FormatName string `json:"format_name"`
		Duration   string `json:"duration"`
	} `json:"format"`
	Streams []struct {
		Index          int    `json:"index"`
		CodecType      string `json:"codec_type"`
		CodecName      string `json:"codec_name"`
		Profile        string `json:"profile"`
		PixelFormat    string `json:"pix_fmt"`
		Width          int    `json:"width"`
		Height         int    `json:"height"`
		Channels       int    `json:"channels"`
		ColorTransfer  string `json:"color_transfer"`
		ColorPrimaries string `json:"color_primaries"`
		BitRate        string `json:"bit_rate"`
		Tags           struct {
			Language string `json:"language"`
			Title    string `json:"title"`
		} `json:"tags"`
		Disposition struct {
			Default int `json:"default"`
			Forced  int `json:"forced"`
		} `json:"disposition"`
	} `json:"streams"`
}

func parse(raw []byte) (contracts.MediaInfo, error) {
	var r report
	if err := json.Unmarshal(raw, &r); err != nil {
		return contracts.MediaInfo{}, err
	}
	info := contracts.MediaInfo{Format: r.Format.FormatName, Streams: []contracts.MediaStream{}}
	// d >= 0 rejects NaN outright (NaN comparisons are false), so a
	// malformed or infinite duration degrades to 0 — unknown — instead
	// of poisoning duration math downstream.
	if d, err := strconv.ParseFloat(r.Format.Duration, 64); err == nil && d >= 0 && !math.IsInf(d, 0) {
		info.Duration = d
	}
	for _, s := range r.Streams {
		if s.CodecType != "video" && s.CodecType != "audio" && s.CodecType != "subtitle" {
			continue
		}
		info.Streams = append(info.Streams, contracts.MediaStream{
			Index: s.Index, Type: s.CodecType, Codec: strings.ToLower(s.CodecName),
			Profile: s.Profile, PixelFormat: strings.ToLower(s.PixelFormat),
			// Geometry feeds scaling and plan math; negative values are
			// malformed input, not real dimensions — clamp to unknown.
			Width: max(s.Width, 0), Height: max(s.Height, 0), Channels: max(s.Channels, 0),
			Language: strings.ToLower(s.Tags.Language), Title: s.Tags.Title,
			Default: s.Disposition.Default != 0, Forced: s.Disposition.Forced != 0,
			ColorTransfer: strings.ToLower(s.ColorTransfer), ColorPrimaries: strings.ToLower(s.ColorPrimaries),
			BitRate:     streamBitRate(s.BitRate),
			Convertible: s.CodecType == "subtitle" && textSubtitle(s.CodecName),
		})
	}
	if len(info.Streams) == 0 {
		return contracts.MediaInfo{}, fmt.Errorf("no media streams")
	}
	return info, nil
}

func textSubtitle(codec string) bool {
	switch strings.ToLower(codec) {
	case "subrip", "srt", "ass", "ssa", "webvtt", "mov_text":
		return true
	default:
		return false
	}
}

// streamBitRate parses ffprobe's bit_rate (bits per second, sometimes
// absent or "N/A") into an int, defaulting to 0 when unknown.
func streamBitRate(raw string) int {
	n, err := strconv.Atoi(strings.TrimSpace(raw))
	if err != nil || n < 0 {
		return 0
	}
	return n
}
