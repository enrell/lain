package gateway

import (
	"os/exec"
	"testing"
)

// requireRealFFmpeg gates every test that runs the host ffmpeg: they
// synthesize and encode real media, which costs minutes across the
// package, so they run only with -tags e2e (CI, `just test-e2e`).
func requireRealFFmpeg(t *testing.T) {
	t.Helper()
	if !e2eEnabled {
		t.Skip("real-ffmpeg test: run with -tags e2e")
	}
	if _, err := exec.LookPath("ffmpeg"); err != nil {
		t.Skip("ffmpeg not installed")
	}
}
