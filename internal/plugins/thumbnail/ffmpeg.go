// Package thumbnail extracts still frames from catalog files with
// ffmpeg. It returns paths, never bytes: the gateway streams thumbnails
// from disk exactly like media (docs/ARCHITECTURE.md).
package thumbnail

import (
	"context"
	"crypto/sha1"
	"encoding/hex"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

const (
	// ID is the built-in ffmpeg thumbnail provider id.
	ID = "lain-thumbnail-ffmpeg"

	// maxParallel bounds concurrent ffmpeg processes: thumbnails are
	// decoration and must never starve playback or a scan.
	maxParallel = 2

	// timeout bounds one extraction.
	timeout = 20 * time.Second
)

// Generator serves lain.transform.thumbnail@1 with an on-disk cache
// keyed by source file identity, timestamp and width.
type Generator struct {
	dir     string
	sem     chan struct{}
	initErr error
}

// New prepares the cache directory. Absence of ffmpeg degrades the
// capability through Health, never the boot.
func New(dir string) *Generator {
	g := &Generator{dir: dir, sem: make(chan struct{}, maxParallel)}
	if err := os.MkdirAll(dir, 0o700); err != nil {
		g.initErr = err
	}
	return g
}

func (g *Generator) ID() string { return ID }

func (g *Generator) Capabilities() []string { return []string{contracts.CapTransformThumb} }

// Health is cheap and side-effect free: ffmpeg must be installed and
// the cache directory usable.
func (g *Generator) Health() error {
	if g.initErr != nil {
		return g.initErr
	}
	if _, err := exec.LookPath("ffmpeg"); err != nil {
		return fmt.Errorf("ffmpeg not found: %w", err)
	}
	if fi, err := os.Stat(g.dir); err != nil || !fi.IsDir() {
		return fmt.Errorf("thumbnail cache %s unusable", g.dir)
	}
	return nil
}

func (g *Generator) Invoke(cap string, input any) (any, error) {
	if cap != contracts.CapTransformThumb {
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
	in, ok := input.(contracts.ThumbnailRequest)
	if !ok {
		return nil, &core.Error{Code: "invalid-message", Msg: "ThumbnailRequest required"}
	}
	if in.FilePath == "" {
		return nil, &core.Error{Code: "invalid-message", Msg: "file_path required"}
	}
	if in.Width <= 0 {
		in.Width = 480
	}
	if in.TimeSec < 0 {
		in.TimeSec = 0
	}
	return g.generate(in)
}

func (g *Generator) generate(in contracts.ThumbnailRequest) (contracts.Thumbnail, error) {
	fi, err := os.Stat(in.FilePath)
	if err != nil {
		return contracts.Thumbnail{}, &core.Error{Code: "dependency-unavailable", Msg: "source file unavailable"}
	}
	out := filepath.Join(g.dir, cacheKey(in, fi.ModTime().Unix(), fi.Size())+".jpg")
	if cached, err := os.Stat(out); err == nil && cached.Size() > 0 {
		return contracts.Thumbnail{Path: out, Width: in.Width, Cached: true}, nil
	}

	g.sem <- struct{}{}
	defer func() { <-g.sem }()
	// Another request may have produced it while we waited for a slot.
	if cached, err := os.Stat(out); err == nil && cached.Size() > 0 {
		return contracts.Thumbnail{Path: out, Width: in.Width, Cached: true}, nil
	}

	if err := g.extract(in.FilePath, in.TimeSec, in.Width, out); err != nil {
		// A seek past the end yields nothing; the start always exists.
		if in.TimeSec > 0 {
			if retryErr := g.extract(in.FilePath, 0, in.Width, out); retryErr == nil {
				return contracts.Thumbnail{Path: out, Width: in.Width}, nil
			}
		}
		return contracts.Thumbnail{}, &core.Error{Code: "dependency-unavailable", Msg: "ffmpeg: " + err.Error()}
	}
	return contracts.Thumbnail{Path: out, Width: in.Width}, nil
}

// extract seeks before -i (keyframe-fast), writes one JPEG atomically.
func (g *Generator) extract(path string, at float64, width int, out string) error {
	ctx, cancel := context.WithTimeout(context.Background(), timeout)
	defer cancel()
	tmp := out + ".tmp"
	_ = os.Remove(tmp)
	cmd := exec.CommandContext(ctx, "ffmpeg",
		"-hide_banner", "-loglevel", "error", "-nostdin",
		"-ss", fmt.Sprintf("%.3f", at),
		"-i", path,
		"-frames:v", "1",
		"-vf", fmt.Sprintf("scale=%d:-2", width),
		"-q:v", "4",
		"-f", "image2", "-y", tmp)
	if output, err := cmd.CombinedOutput(); err != nil {
		return fmt.Errorf("%v: %s", err, strings.TrimSpace(string(output)))
	}
	fi, err := os.Stat(tmp)
	if err != nil || fi.Size() == 0 {
		return fmt.Errorf("no frame at %.1fs", at)
	}
	return os.Rename(tmp, out)
}

func cacheKey(in contracts.ThumbnailRequest, mtime, size int64) string {
	h := sha1.Sum([]byte(fmt.Sprintf("%s|%d|%d|%.1f|%d", in.FilePath, mtime, size, in.TimeSec, in.Width)))
	return hex.EncodeToString(h[:])
}
