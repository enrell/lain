package gateway

import (
	"encoding/json"
	"net/http"
	"os/exec"
	"path/filepath"
	"testing"

	"github.com/enrell/lain/internal/contracts"
)

// catalogCapabilityMKV synthesizes one H.264/AAC Matroska: a file the
// conservative browser rules always transcode and a client that reports
// the right capabilities can play as it is.
func catalogCapabilityMKV(t *testing.T, srv *Server, admin, dir, name string) string {
	t.Helper()
	if _, err := exec.LookPath("ffmpeg"); err != nil {
		t.Skip("ffmpeg not installed")
	}
	if _, err := exec.LookPath("ffprobe"); err != nil {
		t.Skip("ffprobe not installed")
	}
	out := filepath.Join(dir, name)
	cmd := exec.Command("ffmpeg", "-hide_banner", "-loglevel", "error",
		"-f", "lavfi", "-i", "testsrc=size=320x240:rate=15:duration=2",
		"-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:duration=2",
		"-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac",
		"-y", out)
	if combo, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("cannot synthesize mkv: %v: %s", err, combo)
	}
	return catalogDir(t, srv, admin, dir)
}

// TestPlaybackPlanHonoursReportedCapabilities pins the D-058 wire
// contract at the endpoint: `caps` reaches the planner, is validated
// against the server's vocabulary, and its absence keeps the old rules.
func TestPlaybackPlanHonoursReportedCapabilities(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	id := catalogCapabilityMKV(t, srv, admin, t.TempDir(), "[Fansub-A] Capability.mkv")

	cases := []struct {
		name  string
		query string
		want  string
	}{
		{"no capability list keeps the conservative rules", "", "transcode"},
		{"reported container and tracks direct-play", "&caps=mkv,mkv/h264,mkv/aac", "direct"},
		{"an uncovered audio family transcodes", "&caps=mkv,mkv/h264", "transcode"},
		{"an empty claim transcodes", "&caps=", "transcode"},
		{"unknown tokens are dropped, not honoured", "&caps=mkv/h264,mkv/aac,mkv/nonesuch,avi", "transcode"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			rec := do(t, srv, "GET", "/api/items/"+id+"/playback?client=web"+tc.query, nil, admin)
			if rec.Code != http.StatusOK {
				t.Fatalf("plan: %d %s", rec.Code, rec.Body.String())
			}
			var plan contracts.Plan
			if err := json.Unmarshal(rec.Body.Bytes(), &plan); err != nil {
				t.Fatalf("plan body: %s", rec.Body.String())
			}
			if plan.Mode != tc.want {
				t.Fatalf("mode=%q, want %q (%s)", plan.Mode, tc.want, rec.Body.String())
			}
		})
	}
}
