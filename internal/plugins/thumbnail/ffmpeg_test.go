package thumbnail

import (
	"os"
	"os/exec"
	"path/filepath"
	"testing"

	"github.com/enrell/lain/internal/contracts"
)

func makeClip(t *testing.T, dir string) string {
	t.Helper()
	if _, err := exec.LookPath("ffmpeg"); err != nil {
		t.Skip("ffmpeg not installed")
	}
	out := filepath.Join(dir, "clip.mp4")
	cmd := exec.Command("ffmpeg", "-hide_banner", "-loglevel", "error",
		"-f", "lavfi", "-i", "testsrc=size=160x120:rate=10:duration=2",
		"-c:v", "libx264", "-pix_fmt", "yuv420p", "-y", out)
	if combo, err := cmd.CombinedOutput(); err != nil {
		t.Skipf("cannot synthesize clip: %v: %s", err, combo)
	}
	return out
}

func TestGenerateExtractsAndCaches(t *testing.T) {
	root := t.TempDir()
	src := makeClip(t, root)
	g := New(filepath.Join(root, "cache"))
	if err := g.Health(); err != nil {
		t.Fatalf("health: %v", err)
	}

	out, err := g.Invoke(contracts.CapTransformThumb, contracts.ThumbnailRequest{FilePath: src, TimeSec: 1, Width: 96})
	if err != nil {
		t.Fatalf("generate: %v", err)
	}
	thumb := out.(contracts.Thumbnail)
	if thumb.Cached {
		t.Fatal("first call must generate, not serve cache")
	}
	data, err := os.ReadFile(thumb.Path)
	if err != nil {
		t.Fatal(err)
	}
	if len(data) < 3 || data[0] != 0xFF || data[1] != 0xD8 {
		t.Fatalf("not a jpeg: % x", data[:min(3, len(data))])
	}

	again, err := g.Invoke(contracts.CapTransformThumb, contracts.ThumbnailRequest{FilePath: src, TimeSec: 1, Width: 96})
	if err != nil {
		t.Fatalf("cached call: %v", err)
	}
	if cached := again.(contracts.Thumbnail); !cached.Cached || cached.Path != thumb.Path {
		t.Fatalf("expected cache hit on the same path, got %+v", cached)
	}

	// A seek past the end still yields the first frame.
	past, err := g.Invoke(contracts.CapTransformThumb, contracts.ThumbnailRequest{FilePath: src, TimeSec: 999, Width: 96})
	if err != nil {
		t.Fatalf("past-the-end fallback: %v", err)
	}
	if fi, err := os.Stat(past.(contracts.Thumbnail).Path); err != nil || fi.Size() == 0 {
		t.Fatalf("fallback thumbnail missing: %v", err)
	}
}

func TestGenerateRejectsBadInput(t *testing.T) {
	g := New(filepath.Join(t.TempDir(), "cache"))
	if _, err := g.Invoke(contracts.CapTransformThumb, contracts.ThumbnailRequest{}); err == nil {
		t.Fatal("empty file_path must fail")
	}
	if _, err := g.Invoke("lain.transform.other@1", contracts.ThumbnailRequest{FilePath: "x"}); err == nil {
		t.Fatal("wrong capability must fail")
	}
	if _, err := g.Invoke(contracts.CapTransformThumb, contracts.ThumbnailRequest{FilePath: filepath.Join(t.TempDir(), "missing.mp4")}); err == nil {
		t.Fatal("missing source must fail")
	}
}

func TestHealthTracksFFmpeg(t *testing.T) {
	g := New(filepath.Join(t.TempDir(), "cache"))
	_, lookErr := exec.LookPath("ffmpeg")
	if got := g.Health(); (lookErr == nil) != (got == nil) {
		t.Fatalf("health=%v, ffmpeg lookup=%v", got, lookErr)
	}
}
