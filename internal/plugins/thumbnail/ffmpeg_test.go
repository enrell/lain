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

func TestGenerateClampsAndCacheIdentity(t *testing.T) {
	root := t.TempDir()
	src := makeClip(t, root)
	g := New(filepath.Join(root, "cache"))

	// Width <= 0 defaults to 480; TimeSec < 0 clamps to 0 and the clamped
	// value is what the cache key sees: -1 and 0 share one entry.
	out, err := g.Invoke(contracts.CapTransformThumb, contracts.ThumbnailRequest{FilePath: src, TimeSec: -1, Width: 0})
	if err != nil {
		t.Fatalf("clamped request: %v", err)
	}
	th := out.(contracts.Thumbnail)
	if th.Width != 480 {
		t.Fatalf("width=%d, want default 480", th.Width)
	}
	again, err := g.Invoke(contracts.CapTransformThumb, contracts.ThumbnailRequest{FilePath: src, TimeSec: 0, Width: 480})
	if err != nil {
		t.Fatal(err)
	}
	if c := again.(contracts.Thumbnail); !c.Cached || c.Path != th.Path {
		t.Fatalf("TimeSec -1 must share the TimeSec 0 cache entry: %+v", c)
	}

	// A zero-byte cache entry must be regenerated, not served.
	if err := os.Truncate(th.Path, 0); err != nil {
		t.Fatal(err)
	}
	regen, err := g.Invoke(contracts.CapTransformThumb, contracts.ThumbnailRequest{FilePath: src, TimeSec: 0, Width: 480})
	if err != nil {
		t.Fatal(err)
	}
	if regen.(contracts.Thumbnail).Cached {
		t.Fatal("empty cache file must regenerate, not serve")
	}
}

func TestGenerateExtractFailureSurfaces(t *testing.T) {
	root := t.TempDir()
	src := filepath.Join(root, "not-a-video.mp4")
	if err := os.WriteFile(src, []byte("definitely not a video"), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, err := exec.LookPath("ffmpeg"); err != nil {
		t.Skip("ffmpeg not installed")
	}
	g := New(filepath.Join(root, "cache"))
	// TimeSec 0: no retry, straight error.
	if _, err := g.Invoke(contracts.CapTransformThumb, contracts.ThumbnailRequest{FilePath: src, Width: 96}); err == nil {
		t.Fatal("unreadable media must fail")
	}
	// TimeSec > 0: retry at 0 also fails -> still an error.
	if _, err := g.Invoke(contracts.CapTransformThumb, contracts.ThumbnailRequest{FilePath: src, TimeSec: 5, Width: 96}); err == nil {
		t.Fatal("failed retry must fail")
	}
}

func TestHealthTracksCacheDir(t *testing.T) {
	// initErr: cache dir cannot be created under a regular file.
	f, err := os.CreateTemp(t.TempDir(), "file")
	if err != nil {
		t.Fatal(err)
	}
	f.Close()
	g := New(filepath.Join(f.Name(), "sub"))
	if g.Health() == nil {
		t.Fatal("unwritable cache dir must fail Health")
	}
	// Cache dir replaced by a file after a clean init.
	root := t.TempDir()
	g = New(filepath.Join(root, "cache"))
	dir := g.dir
	if err := os.RemoveAll(dir); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(dir, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	if g.Health() == nil {
		t.Fatal("file-instead-of-dir cache must fail Health")
	}
}
