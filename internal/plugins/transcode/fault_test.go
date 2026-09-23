package transcode


// Fault-injection for the derivative cache: an unwritable or vanished
// artifact root must surface as a typed error or a failed job — never
// a panic, never a stuck queued job that outlives the process.

import (
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
)

// A cache directory that cannot be written makes admission fail with
// the unavailable code — the queue stays empty rather than absorbing
// work it can never finish.
func TestUnwritableCacheRejectsAdmission(t *testing.T) {
	if os.Geteuid() == 0 {
		t.Skip("running as root: chmod 000 does not block writes")
	}
	root := t.TempDir()
	cache := filepath.Join(root, "cache")
	tr := newWithDeps(cache, Config{MaxCacheBytes: 1 << 20, QueueSize: 2}, func(_ sourceSpec, out string, _ func(progressSample)) (string, error) {
		return "transcode", os.WriteFile(out, []byte("mp4"), 0o600)
	}, time.Now)
	t.Cleanup(func() { _ = tr.Close() })

	src := filepath.Join(root, "src.mkv")
	if err := os.WriteFile(src, []byte("src"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(cache, 0o000); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.Chmod(cache, 0o700) })

	_, err := tr.Invoke(contracts.CapPlaybackTranscodeV2, contracts.TranscodeV2Request{
		Action: contracts.TranscodeStartAction, FilePath: src,
	})
	if err == nil {
		t.Fatal("start on unwritable cache returned nil error")
	}
	tr.mu.Lock()
	pending, active := tr.pending, tr.active
	tr.mu.Unlock()
	if pending != 0 || active != 0 {
		t.Fatalf("rejected admission left counters pending=%d active=%d", pending, active)
	}
}

// If the artifact root vanishes between admission and convert, the job
// must end failed — not hang queued forever, not panic the worker.
func TestCacheDirVanishesMidJob(t *testing.T) {
	root := t.TempDir()
	cache := filepath.Join(root, "cache")
	started := make(chan struct{})
	release := make(chan struct{})
	tr := newWithDeps(cache, Config{MaxCacheBytes: 1 << 20, QueueSize: 2}, func(_ sourceSpec, out string, _ func(progressSample)) (string, error) {
		close(started)
		<-release // hold until the cache tree is gone
		return "transcode", os.WriteFile(out, []byte("mp4"), 0o600)
	}, time.Now)
	t.Cleanup(func() { _ = tr.Close() })

	src := filepath.Join(root, "src.mkv")
	if err := os.WriteFile(src, []byte("src"), 0o600); err != nil {
		t.Fatal(err)
	}
	out, err := tr.Invoke(contracts.CapPlaybackTranscodeV2, contracts.TranscodeV2Request{
		Action: contracts.TranscodeStartAction, FilePath: src,
	})
	if err != nil {
		t.Fatal(err)
	}
	session := out.(contracts.TranscodeStatus).Session
	<-started
	if err := os.RemoveAll(cache); err != nil {
		t.Fatal(err)
	}
	close(release) // the artifact write now fails under the worker
	st := waitState(t, tr, src, session, contracts.TranscodeFailed)
	if st.Error == "" && st.State != contracts.TranscodeFailed {
		t.Fatalf("vanished cache produced %+v, want failed", st)
	}
}
