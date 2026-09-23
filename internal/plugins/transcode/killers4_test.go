package transcode


import (
	"log/slog"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
)

// --- v3.go:370 (activeStreamsForLocked state counting) ---

func TestKillActiveStreamsStates(t *testing.T) {
	tr, _ := v3Fixture(t, Config{})
	add := func(session, user string, state string) {
		tr.mu.Lock()
		tr.jobs[session] = &job{spec: sourceSpec{Session: session}, state: state, userID: user, done: make(chan struct{})}
		tr.mu.Unlock()
	}
	add("sess-run", "u1", contracts.TranscodeRunning)
	add("sess-que", "u1", contracts.TranscodeQueued)
	add("sess-done", "u1", contracts.TranscodeReady)
	add("sess-fail", "u1", contracts.TranscodeFailed)
	add("sess-other", "u2", contracts.TranscodeRunning)

	tr.mu.Lock()
	got := tr.activeStreamsForLocked("u1", "")
	except := tr.activeStreamsForLocked("u1", "sess-run")
	tr.mu.Unlock()
	if got != 2 {
		t.Fatalf("active streams = %d, want 2 (running+queued)", got)
	}
	if except != 1 {
		t.Fatalf("joining own session must not count it: %d", except)
	}
}

// --- v3.go:455 (MaxStreams guard) ---

func TestKillMaxStreamsAdmission(t *testing.T) {
	tr, src := v3Fixture(t, Config{})
	in := v3Request(contracts.TranscodeStartAction, src)
	in.UserID = "u1"
	in.Policy = &contracts.TranscodePolicy{AllowVideoTranscode: true, AllowAudioTranscode: true, AllowRemux: true, MaxStreams: 1}

	// A running session of the same user fills the limit; a *different*
	// source must be refused.
	tr.mu.Lock()
	tr.jobs["sess-a"] = &job{spec: sourceSpec{Session: "sess-a"}, state: contracts.TranscodeRunning, userID: "u1", done: make(chan struct{})}
	tr.mu.Unlock()
	src2 := filepath.Join(t.TempDir(), "other.mkv")
	if err := os.WriteFile(src2, []byte("media2"), 0o600); err != nil {
		t.Fatal(err)
	}
	in2 := v3Request(contracts.TranscodeStartAction, src2)
	in2.UserID = "u1"
	in2.Policy = in.Policy
	if _, err := tr.startV3(mustSpecV3(t, tr, in2), in2, in2.Settings); err == nil {
		t.Fatal("a second simultaneous stream must be refused at MaxStreams=1")
	}

	// MaxStreams==0 means unlimited: the same policy without a limit
	// must admit even with an active session.
	in2.Policy = &contracts.TranscodePolicy{AllowVideoTranscode: true, AllowAudioTranscode: true, AllowRemux: true}
	if _, err := tr.startV3(mustSpecV3(t, tr, in2), in2, in2.Settings); err != nil {
		t.Fatalf("unlimited policy rejected: %v", err)
	}
}

// --- v3.go:487/488 (VideoDirect/AudioDirect flags) ---

func TestKillStatusV3DirectFlagPlanes(t *testing.T) {
	j := &job{spec: sourceSpec{Session: "s"}, state: contracts.TranscodeReady, done: make(chan struct{})}
	tr, _ := v3Fixture(t, Config{})
	st := tr.statusV3FromJob(j)
	if st.VideoDirect || st.AudioDirect {
		t.Fatal("nil plan must report both direct flags false")
	}
	j.plan = &encodePlan{copyVideo: true, copyAudio: false}
	st = tr.statusV3FromJob(j)
	if !st.VideoDirect || st.AudioDirect {
		t.Fatalf("flags = video:%v audio:%v, want true/false", st.VideoDirect, st.AudioDirect)
	}
	j.plan = &encodePlan{copyVideo: false, copyAudio: true}
	st = tr.statusV3FromJob(j)
	if st.VideoDirect || !st.AudioDirect {
		t.Fatalf("flags = video:%v audio:%v, want false/true", st.VideoDirect, st.AudioDirect)
	}
}

// --- policy.go:900 (custom ffmpeg path) ---

func TestKillFFmpegBinaryPath(t *testing.T) {
	settings := contracts.DefaultTranscodeSettings()
	if got := ffmpegBinary(settings); got != "ffmpeg" {
		t.Fatalf("default binary = %q", got)
	}
	settings.FFmpegPath = "/opt/ffmpeg-custom"
	if got := ffmpegBinary(settings); got != "/opt/ffmpeg-custom" {
		t.Fatalf("custom path ignored: %q", got)
	}
}

// --- encode.go:163 (empty filter chain must not emit -vf) ---

func TestKillNoFiltersNoVF(t *testing.T) {
	p := encodePlanFor(nil)
	args := p.ffmpegArgs("/tmp/out.mp4", "", false)
	if hasArg(args, "-vf") {
		t.Fatalf("args %v carry -vf with no filters", args)
	}
}

// --- encode.go:263 (zero frame rate must not emit GOP keys) ---

func TestKillGOPSkippedOnZeroFPS(t *testing.T) {
	p := encodePlanFor(func(p *encodePlan) {
		p.video.FrameRate = "0/0"
		p.delivery = contracts.TranscodeDeliveryHLS
	})
	args := p.ffmpegArgs("/tmp/out.m3u8", "", false)
	if hasArg(args, "-g") || hasArg(args, "-keyint_min") {
		t.Fatalf("zero-fps plan emitted GOP keys: %v", args)
	}
	// A real frame rate emits them, pinned to the segment length.
	p.video.FrameRate = "25/1"
	p.settings.HLSSegmentSeconds = 4
	args = p.ffmpegArgs("/tmp/out.m3u8", "", false)
	if !hasArg(args, "-g", "100") || !hasArg(args, "-keyint_min", "100") {
		t.Fatalf("25fps/4s plan must emit -g 100: %v", args)
	}
}

// --- hls.go:255 (throttle boundary: ahead == limit must not pause) ---

func TestKillThrottleExactBoundary(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("SIGSTOP/SIGCONT")
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
	// Produced exactly 6s ahead with a 6s budget: at the boundary the
	// process must keep running (the pause needs strictly more).
	j := throttleJob(t, tr, "throttle-boundary", 6, cmd)
	tr.applyThrottle(j)
	if j.isPaused(t, tr) {
		t.Fatal("process paused at exactly the throttle limit")
	}
}

// --- coordinator.go:1390 (cleanup stops exactly at budget) ---

func TestKillCleanupExactBudgetStop(t *testing.T) {
	dir := t.TempDir()
	cache := filepath.Join(dir, "cache")
	var warnBuf strings.Builder
	tr := newWithDeps(cache, Config{MaxCacheBytes: 8}, nil, time.Now)
	tr.SetLogger(slog.New(slog.NewTextHandler(&warnBuf, &slog.HandlerOptions{Level: slog.LevelDebug})))
	t.Cleanup(func() { _ = tr.Close() })
	src := filepath.Join(dir, "src.mkv")
	if err := os.WriteFile(src, []byte("src"), 0o600); err != nil {
		t.Fatal(err)
	}
	srcFi, _ := os.Stat(src)
	old := tr.now().Unix() - int64(recentAccessGraceSeconds) - 10

	put := func(session string, size int, accessed int64) {
		e := cacheEntry{Session: session, SourcePath: src, SourceSize: srcFi.Size(), SourceModTime: srcFi.ModTime().UnixNano(),
			Profile: "p", AccessedAt: accessed, CreatedAt: accessed}
		if err := writeJSONAtomic(tr.metaPath(session), e); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(tr.mediaPath(session), make([]byte, size), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	// 8+8=16 over a 16 budget minus one eviction: after dropping "victim"
	// the remainder lands exactly on the budget — nothing else may go.
	put("victim", 8, old)
	put("survivor", 8, old+5)
	if err := tr.cleanup(false, nil); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(tr.metaPath("victim")); !os.IsNotExist(err) {
		t.Fatal("LRU entry not evicted")
	}
	if _, err := os.Stat(tr.metaPath("survivor")); err != nil {
		t.Fatal("eviction continued past the exact budget boundary")
	}
	if strings.Contains(warnBuf.String(), "over quota") {
		t.Fatal("at exactly the budget the over-quota warning must not fire")
	}
}

// --- coordinator.go:1397 (grace boundary: AccessedAt == grace is protected) ---

func TestKillCleanupGraceBoundary(t *testing.T) {
	dir := t.TempDir()
	cache := filepath.Join(dir, "cache")
	tr := newWithDeps(cache, Config{MaxCacheBytes: 4}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	src := filepath.Join(dir, "src.mkv")
	if err := os.WriteFile(src, []byte("src"), 0o600); err != nil {
		t.Fatal(err)
	}
	srcFi, _ := os.Stat(src)
	// Accessed exactly at the grace edge: still protected.
	edge := tr.now().Unix() - int64(recentAccessGraceSeconds)
	e := cacheEntry{Session: "edge", SourcePath: src, SourceSize: srcFi.Size(), SourceModTime: srcFi.ModTime().UnixNano(),
		Profile: "p", AccessedAt: edge, CreatedAt: edge}
	if err := writeJSONAtomic(tr.metaPath("edge"), e); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(tr.mediaPath("edge"), make([]byte, 8), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := tr.cleanup(false, nil); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(tr.metaPath("edge")); err != nil {
		t.Fatal("entry at the grace boundary must be protected")
	}
}

// --- coordinator.go:1412 (relocated artifact dir removed on evict) ---

func TestKillCleanupRemovesArtifactDir(t *testing.T) {
	dir := t.TempDir()
	cache := filepath.Join(dir, "cache")
	tr := newWithDeps(cache, Config{MaxCacheBytes: 4}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	src := filepath.Join(dir, "src.mkv")
	if err := os.WriteFile(src, []byte("src"), 0o600); err != nil {
		t.Fatal(err)
	}
	srcFi, _ := os.Stat(src)
	root := filepath.Join(dir, "temp")
	sessionDir := artifactDirFor(root, "relocated")
	if err := os.MkdirAll(sessionDir, 0o700); err != nil {
		t.Fatal(err)
	}
	old := tr.now().Unix() - int64(recentAccessGraceSeconds) - 10
	e := cacheEntry{Session: "relocated", SourcePath: src, SourceSize: srcFi.Size(), SourceModTime: srcFi.ModTime().UnixNano(),
		Profile: "p", AccessedAt: old, CreatedAt: old, ArtifactRoot: root}
	if err := writeJSONAtomic(tr.metaPath("relocated"), e); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(tr.mediaPath("relocated"), make([]byte, 8), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := tr.cleanup(false, nil); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(sessionDir); !os.IsNotExist(err) {
		t.Fatal("relocated session dir must be removed with its artifacts")
	}
}

// --- coordinator.go:1449 (owner marker matching us allows the sweep) ---

func TestKillSweepOwnerMatch(t *testing.T) {
	dir := t.TempDir()
	cache := filepath.Join(dir, "cache")
	tr := newWithDeps(cache, Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	root := filepath.Join(dir, "temp")
	if err := os.MkdirAll(root, 0o700); err != nil {
		t.Fatal(err)
	}
	// The marker already names this data dir: the sweep is ours to run.
	if err := os.WriteFile(filepath.Join(root, ".owner"), []byte(tr.dir), 0o600); err != nil {
		t.Fatal(err)
	}
	orphan := filepath.Join(root, "orphan-session")
	if err := os.MkdirAll(orphan, 0o700); err != nil {
		t.Fatal(err)
	}
	tr.sweepRelocatedOrphans(root, nil)
	if _, err := os.Stat(orphan); !os.IsNotExist(err) {
		t.Fatal("orphan under an owned root must be swept")
	}
}

// --- ffmpeg.go:162 (video stream count) ---
// `video++` -> `--` is equivalent: video is only read by `video == 0`,
// and a first video stream still leaves it non-zero in both directions.
// Documented in state.json.

// --- hls.go:255 (resume boundary: ahead == limit/2 must stay paused) ---

func TestKillThrottleResumeBoundary(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("SIGSTOP/SIGCONT")
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

	settings := contracts.DefaultTranscodeSettings()
	settings.Throttle = true
	settings.ThrottleAheadSec = 6
	dir := tr.pathsFor("resume-boundary", settings).hlsDir
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	// Two 3s segments: client at segment 0 -> 3s consumed, 6s produced,
	// ahead = 3 == limit/2. Resume needs strictly less than half.
	raw := "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:3\n" +
		"#EXTINF:3.000,\nseg00000.m4s\n#EXTINF:3.000,\nseg00001.m4s\n#EXT-X-ENDLIST\n"
	if err := os.WriteFile(filepath.Join(dir, hlsRawPlaylist), []byte(raw), 0o600); err != nil {
		t.Fatal(err)
	}
	j := &job{
		spec:  sourceSpec{Path: "/media/Show.mkv", Session: "resume-boundary", Settings: settings},
		state: contracts.TranscodeRunning, v3: true, delivery: contracts.TranscodeDeliveryHLS,
		settings: settings, cmd: cmd, clientSegment: 0, paused: true,
	}
	tr.applyThrottle(j)
	if !j.isPaused(t, tr) {
		t.Fatal("process resumed at exactly limit/2 ahead")
	}
	// Strictly below half resumes.
	tr.mu.Lock()
	j.clientSegment = 1
	tr.mu.Unlock()
	tr.applyThrottle(j)
	if j.isPaused(t, tr) {
		t.Fatal("process still paused below half the limit")
	}
}
