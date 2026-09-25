package gateway

import (
	"encoding/json"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/enrell/lain/internal/contracts"
)

// TestThumbnailAuthAndKnownItem proves the query-token path works for
// <img> consumers and unknown items fail without invoking ffmpeg.
func TestThumbnailAuthAndKnownItem(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)

	if rec := do(t, srv, "GET", "/api/items/whatever/thumbnail", nil, ""); rec.Code != 401 {
		t.Fatalf("unauthenticated: %d, want 401", rec.Code)
	}
	// Headerless clients (QML Image) pass ?token= instead.
	if rec := do(t, srv, "GET", "/api/items/nope/thumbnail?token="+admin, nil, ""); rec.Code != 404 {
		t.Fatalf("unknown item via query token: %d, want 404", rec.Code)
	}
}

// TestThumbnailServesJPEG is the end-to-end slice: synthesize a clip,
// catalogue it through a scan, request a still and read JPEG magic.
func TestThumbnailServesJPEG(t *testing.T) {
	if _, err := exec.LookPath("ffmpeg"); err != nil {
		t.Skip("ffmpeg not installed")
	}
	srv := testServer(t)
	admin := setupAdmin(t, srv)

	root := t.TempDir()
	clip := filepath.Join(root, "Clip.mp4")
	cmd := exec.Command("ffmpeg", "-hide_banner", "-loglevel", "error",
		"-f", "lavfi", "-i", "testsrc=size=160x120:rate=10:duration=2",
		"-c:v", "libx264", "-pix_fmt", "yuv420p", "-y", clip)
	if combo, err := cmd.CombinedOutput(); err != nil {
		t.Skipf("cannot synthesize clip: %v: %s", err, combo)
	}

	if rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "Clips", "type": "movie", "path": root}, admin); rec.Code != 201 {
		t.Fatalf("library: %d %s", rec.Code, rec.Body.String())
	}
	if rec := do(t, srv, "POST", "/api/library/scan", nil, admin); rec.Code != 202 {
		t.Fatalf("scan: %d", rec.Code)
	}
	waitScan(t, srv, admin)

	var page contracts.CatalogPage
	rec := do(t, srv, "GET", "/api/catalog", nil, admin)
	if err := json.Unmarshal(rec.Body.Bytes(), &page); err != nil || len(page.Items) != 1 {
		t.Fatalf("catalog: %d %s", rec.Code, rec.Body.String())
	}
	id := page.Items[0].ID

	rec = do(t, srv, "GET", "/api/items/"+id+"/thumbnail?t=1&w=96", nil, admin)
	if rec.Code != 200 {
		t.Fatalf("thumbnail: %d %s", rec.Code, rec.Body.String())
	}
	if ct := rec.Header().Get("Content-Type"); ct != "image/jpeg" {
		t.Fatalf("content-type %q, want image/jpeg", ct)
	}
	if cc := rec.Header().Get("Cache-Control"); !strings.Contains(cc, "max-age") {
		t.Fatalf("cache-control %q, want max-age", cc)
	}
	body := rec.Body.Bytes()
	if len(body) < 3 || body[0] != 0xFF || body[1] != 0xD8 {
		t.Fatalf("not a jpeg: % x", body[:min(3, len(body))])
	}

	// Second read comes from the provider cache and still serves bytes.
	rec = do(t, srv, "GET", "/api/items/"+id+"/thumbnail?t=1&w=96", nil, admin)
	if rec.Code != 200 || rec.Body.Len() == 0 {
		t.Fatalf("cached thumbnail: %d %d bytes", rec.Code, rec.Body.Len())
	}
}
