package transcode

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-22

import (
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
)

// idleJob builds a running HLS v3 session with a real on-disk playlist
// directory, so the ticker's rewrite step succeeds and the idle check is
// actually reached instead of being skipped by an early continue.
func idleJob(t *testing.T, tr *Transcoder, session string, idleSec int, lastTouch int64) *job {
	t.Helper()
	settings := contracts.DefaultTranscodeSettings()
	settings.IdleTimeoutSec = idleSec
	dir := tr.pathsFor(session, settings).hlsDir
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	raw := "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:6\n" +
		"#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:6.000,\nseg00000.m4s\n#EXT-X-ENDLIST\n"
	if err := os.WriteFile(filepath.Join(dir, hlsRawPlaylist), []byte(raw), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "seg00000.m4s"), []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	return &job{
		spec:      sourceSpec{Path: "/media/[Fansub-A] Show.mkv", Session: session, Settings: settings},
		state:     contracts.TranscodeRunning,
		v3:        true,
		delivery:  contracts.TranscodeDeliveryHLS,
		settings:  settings,
		lastTouch: lastTouch,
	}
}

// TestIdleTimeoutStopsAbandonedSession pins Jellyfin's idle-timeout
// behavior at runtime: a session nobody has fetched from is stopped with
// a stable reason, while a recently touched one keeps running. The clock
// is injected, so the assertion is deterministic.
func TestIdleTimeoutStopsAbandonedSession(t *testing.T) {
	now := time.Unix(1_700_000_000, 0)
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, func() time.Time { return now })
	t.Cleanup(func() { _ = tr.Close() })

	abandoned := idleJob(t, tr, "audit-idle-abandoned", 60, now.Unix()-3600)
	tr.mu.Lock()
	tr.jobs[abandoned.spec.Session] = abandoned
	tr.mu.Unlock()
	tr.tick()
	if abandoned.state != contracts.TranscodeFailed || abandoned.errCode != "idle-timeout" {
		t.Fatalf("abandoned session state=%q code=%q, want failed/idle-timeout", abandoned.state, abandoned.errCode)
	}

	fresh := idleJob(t, tr, "audit-idle-fresh", 60, now.Unix())
	tr.mu.Lock()
	tr.jobs[fresh.spec.Session] = fresh
	tr.mu.Unlock()
	tr.tick()
	if fresh.state != contracts.TranscodeRunning {
		t.Fatalf("recently touched session state=%q, want running", fresh.state)
	}
}

// TestIdleTimeoutDisabledKeepsSession proves the operator can turn the
// timeout off: a zero idle window never stops a running session.
func TestIdleTimeoutDisabledKeepsSession(t *testing.T) {
	now := time.Unix(1_700_000_000, 0)
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, func() time.Time { return now })
	t.Cleanup(func() { _ = tr.Close() })

	j := idleJob(t, tr, "audit-idle-off", 0, now.Unix()-86400)
	tr.mu.Lock()
	tr.jobs[j.spec.Session] = j
	tr.mu.Unlock()
	tr.tick()
	if j.state != contracts.TranscodeRunning {
		t.Fatalf("session state=%q with idle timeout off, want running", j.state)
	}
}
