package transcode

import (
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
)

// throttleJob is a running HLS session with a live child process and a
// produced-ahead budget, so applyThrottle has something real to pause.
func throttleJob(t *testing.T, tr *Transcoder, session string, aheadSec int, cmd *exec.Cmd) *job {
	t.Helper()
	settings := contracts.DefaultTranscodeSettings()
	settings.Throttle = true
	settings.ThrottleAheadSec = aheadSec
	dir := tr.pathsFor(session, settings).hlsDir
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	raw := "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:6\n#EXTINF:6.000,\nseg00000.m4s\n#EXT-X-ENDLIST\n"
	if err := os.WriteFile(filepath.Join(dir, hlsRawPlaylist), []byte(raw), 0o600); err != nil {
		t.Fatal(err)
	}
	return &job{
		spec:  sourceSpec{Path: "/media/[Fansub-A] Show.mkv", Session: session, Settings: settings},
		state: contracts.TranscodeRunning, v3: true, delivery: contracts.TranscodeDeliveryHLS,
		settings: settings, cmd: cmd, clientSegment: -1,
	}
}

func (j *job) isPaused(t *testing.T, tr *Transcoder) bool {
	t.Helper()
	tr.mu.Lock()
	defer tr.mu.Unlock()
	return j.paused
}

// TestThrottlePausesAndResumesAheadProcess pins Jellyfin's "throttle
// transcodes" at runtime: ffmpeg is stopped once it runs further ahead
// than the budget and resumed after the client catches up. A real child
// process is used, so the SIGSTOP/SIGCONT path runs instead of a mock.
func TestThrottlePausesAndResumesAheadProcess(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("throttling uses SIGSTOP/SIGCONT")
	}
	if _, err := exec.LookPath("sleep"); err != nil {
		t.Skip("sleep not available")
	}
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })

	cmd := exec.Command("sleep", "30")
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = cmd.Process.Kill() })

	// 6s produced, client at 0s, 5s budget: further ahead than allowed.
	j := throttleJob(t, tr, "audit-throttle", 5, cmd)
	tr.applyThrottle(j)
	if !j.isPaused(t, tr) {
		t.Fatalf("process not paused with 6s produced ahead of a 5s budget")
	}

	// The client fetches the only segment, so ahead drops below budget/2.
	tr.mu.Lock()
	j.clientSegment = 0
	tr.mu.Unlock()
	tr.applyThrottle(j)
	if j.isPaused(t, tr) {
		t.Fatalf("process stayed paused after the client caught up")
	}
}

// TestThrottleOffNeverPauses proves the operator switch is honored: with
// throttling disabled the process is never stopped, however far ahead it
// runs.
func TestThrottleOffNeverPauses(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("throttling uses SIGSTOP/SIGCONT")
	}
	if _, err := exec.LookPath("sleep"); err != nil {
		t.Skip("sleep not available")
	}
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })

	cmd := exec.Command("sleep", "30")
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = cmd.Process.Kill() })

	j := throttleJob(t, tr, "audit-throttle-off", 5, cmd)
	tr.mu.Lock()
	j.settings.Throttle = false
	tr.mu.Unlock()
	tr.applyThrottle(j)
	if j.isPaused(t, tr) {
		t.Fatalf("process paused while throttling is disabled")
	}
}
