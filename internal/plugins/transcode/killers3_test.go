package transcode


// Round 3 mutation killers: config boundaries, lifecycle edges, cache
// eviction order, argv detail branches, playlist parsing and the v3
// admission/status fields the broader suites exercise but never assert.

import (
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

// --- coordinator.go: config + lifecycle edges ---------------------------

func TestKillConfigBoundaries(t *testing.T) {
	for _, cfg := range []Config{{MaxConcurrent: 0}, {MaxConcurrent: -3}} {
		tr := newWithDeps(filepath.Join(t.TempDir(), "c"), cfg, nil, time.Now)
		if tr.maxConcurrent != defaultMaxConcurrent {
			t.Fatalf("MaxConcurrent=%d -> %d, want default %d", cfg.MaxConcurrent, tr.maxConcurrent, defaultMaxConcurrent)
		}
		_ = tr.Close()
	}
	tr := newWithDeps(filepath.Join(t.TempDir(), "c"), Config{MaxConcurrent: 5}, nil, time.Now)
	if tr.maxConcurrent != 5 {
		t.Fatalf("maxConcurrent=%d, want 5", tr.maxConcurrent)
	}
	_ = tr.Close()
}

func TestKillReleaseSlotFloor(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "c"), Config{MaxConcurrent: 1}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	tr.releaseSlot() // active is already 0: must not go negative
	if tr.active != 0 {
		t.Fatalf("active=%d, want 0", tr.active)
	}
	if !tr.acquireSlot() || tr.active != 1 {
		t.Fatalf("acquireSlot failed, active=%d", tr.active)
	}
	tr.releaseSlot()
	tr.releaseSlot()
	if tr.active != 0 {
		t.Fatalf("double release: active=%d, want 0", tr.active)
	}
}

func TestKillAcquireSlotClosed(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "c"), Config{}, nil, time.Now)
	if err := tr.Close(); err != nil {
		t.Fatal(err)
	}
	done := make(chan bool, 1)
	go func() { done <- tr.acquireSlot() }()
	select {
	case ok := <-done:
		if ok {
			t.Fatal("acquireSlot on a closed transcoder must return false")
		}
	case <-time.After(2 * time.Second):
		t.Fatal("acquireSlot must not block after Close")
	}
}

func jobKey(at int64) string { return "s" + strconv.FormatInt(at, 36) }

func TestKillStopJobGuards(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "c"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	// nil cmd must not panic and must still clear the pause flag.
	j := &job{spec: sourceSpec{Session: "nc"}, state: contracts.TranscodeRunning, paused: true, done: make(chan struct{})}
	tr.stopJob(j, "x", "y")
	if j.paused {
		t.Fatal("stopJob must clear paused")
	}
	// A real process is actually killed.
	cmd := exec.Command("sleep", "60")
	if err := cmd.Start(); err != nil {
		t.Skipf("sleep unavailable: %v", err)
	}
	j2 := &job{spec: sourceSpec{Session: "pc"}, state: contracts.TranscodeRunning, cmd: cmd, done: make(chan struct{})}
	tr.jobs["pc"] = j2
	tr.stopJob(j2, "x", "y")
	if err := cmd.Wait(); err == nil {
		t.Fatal("killed process must exit non-zero")
	}
}

func TestKillRunStoppedMidflight(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "c"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	j := &job{
		spec: sourceSpec{Session: "st", Path: "/media/x.mkv"}, state: contracts.TranscodeQueued,
		v3: true, delivery: contracts.TranscodeDeliveryProgressive,
		settings: contracts.DefaultTranscodeSettings(), plan: &encodePlan{}, done: make(chan struct{}),
	}
	tr.v3run = func(j *job) error {
		// The artifact must exist so finalizeV3/recordV3 succeeds and the
		// post-run `err == nil && j.stopped` check is what fails the job.
		paths := tr.pathsFor(j.spec.Session, j.settings)
		if err := os.WriteFile(paths.media, []byte("mp4"), 0o600); err != nil {
			return err
		}
		tr.mu.Lock()
		j.stopped = true
		tr.mu.Unlock()
		return nil
	}
	tr.runV3(j)
	if j.state != contracts.TranscodeFailed {
		t.Fatalf("stopped mid-run must end failed, got %q", j.state)
	}
}

func TestKillCleanupRunsOnlyOnSuccess(t *testing.T) {
	dir := t.TempDir()
	// An orphan artifact with no sidecar: cleanup removes it — but only
	// when the job that triggered cleanup succeeded.
	for _, succeed := range []bool{true, false} {
		tr := newWithDeps(filepath.Join(dir, "c"), Config{}, nil, time.Now)
		orphan := filepath.Join(tr.dir, "orphan.mp4")
		if err := os.WriteFile(orphan, []byte("x"), 0o600); err != nil {
			t.Fatal(err)
		}
		j := &job{
			spec: sourceSpec{Session: "gate", Path: "/media/x.mkv"}, state: contracts.TranscodeQueued,
			v3: true, delivery: contracts.TranscodeDeliveryProgressive,
			settings: contracts.DefaultTranscodeSettings(), plan: &encodePlan{}, done: make(chan struct{}),
		}
		tr.v3run = func(j *job) error {
			if !succeed {
				return errors.New("boom")
			}
			return writeV3Artifact(tr, "gate")
		}
		tr.runV3(j)
		_, statErr := os.Stat(orphan)
		if succeed && statErr == nil {
			t.Fatal("successful run must clean orphan artifacts")
		}
		if !succeed && statErr != nil {
			t.Fatal("failed run must not run the cleanup sweep")
		}
		_ = tr.Close()
	}
}

// --- coordinator.go: finalizeV3 + readyEntry ----------------------------

func TestKillFinalizeV3Extract(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "c"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	spec := sourceSpec{Session: "fx", Path: "/nonexistent/in.mkv"}
	settings := contracts.DefaultTranscodeSettings()
	plan := &encodePlan{
		spec: spec, settings: settings, delivery: contracts.TranscodeDeliveryProgressive,
		subtitleMode: contracts.SubtitleModeExtract, subtitle: &stream{Index: 2, CodecName: "subrip"},
	}
	// hasSubtitle already set: the extract step is skipped entirely, so
	// even an unreadable source must not fail finalize. recordV3 still
	// needs real artifacts on disk.
	paths := tr.pathsFor(spec.Session, settings)
	if err := os.WriteFile(paths.media, []byte("media"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(paths.subtitle, []byte("WEBVTT"), 0o600); err != nil {
		t.Fatal(err)
	}
	jDone := &job{spec: spec, settings: settings, plan: plan, hasSubtitle: true, method: "transcode", done: make(chan struct{})}
	if err := tr.finalizeV3(jDone); err != nil {
		t.Fatalf("hasSubtitle must skip extraction: %v", err)
	}
	// hasSubtitle clear: extraction is attempted and fails on the
	// nonexistent source.
	jExtract := &job{spec: spec, settings: settings, plan: plan, method: "transcode", done: make(chan struct{})}
	if err := tr.finalizeV3(jExtract); err == nil {
		t.Fatal("extract of a missing source must fail")
	}
}

func TestKillReadyEntryArtifacts(t *testing.T) {
	dir := t.TempDir()
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	src := filepath.Join(dir, "in.mkv")
	if err := os.WriteFile(src, []byte("src"), 0o600); err != nil {
		t.Fatal(err)
	}
	fi, _ := os.Stat(src)
	mk := func(session string, mutate func(*cacheEntry)) sourceSpec {
		e := cacheEntry{
			Session: session, SourcePath: src, SourceSize: fi.Size(),
			SourceModTime: fi.ModTime().UnixNano(), Profile: "p",
			Delivery: contracts.TranscodeDeliveryProgressive, Method: "transcode",
		}
		if mutate != nil {
			mutate(&e)
		}
		if err := writeJSONAtomic(tr.metaPath(session), &e); err != nil {
			t.Fatal(err)
		}
		return sourceSpec{Session: session, Path: src, Size: fi.Size(), ModTimeNS: fi.ModTime().UnixNano(), Profile: "p"}
	}
	// Zero-byte media: not ready.
	spec := mk("z1", nil)
	if err := os.WriteFile(tr.mediaPath("z1"), nil, 0o600); err != nil {
		t.Fatal(err)
	}
	if _, ok := tr.readyEntry(spec); ok {
		t.Fatal("zero-byte artifact must not be ready")
	}
	// Real media + declared subtitle with a zero-byte sidecar: not ready.
	spec = mk("z2", func(e *cacheEntry) { e.HasSubtitle = true })
	if err := os.WriteFile(tr.mediaPath("z2"), []byte("media"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(tr.subtitlePath("z2"), nil, 0o600); err != nil {
		t.Fatal(err)
	}
	if _, ok := tr.readyEntry(spec); ok {
		t.Fatal("zero-byte subtitle sidecar must not be ready")
	}
	// Both artifacts non-empty: ready.
	spec = mk("ok", func(e *cacheEntry) { e.HasSubtitle = true })
	_ = os.WriteFile(tr.mediaPath("ok"), []byte("media"), 0o600)
	_ = os.WriteFile(tr.subtitlePath("ok"), []byte("WEBVTT"), 0o600)
	if e, ok := tr.readyEntry(spec); !ok || e.Size != int64(len("media")+len("WEBVTT")) {
		t.Fatalf("ready entry: ok=%v size=%d", ok, e.Size)
	}
	// HLS entry with a declared but empty subtitle sidecar: not ready.
	spec = mk("hs", func(e *cacheEntry) {
		e.Delivery = contracts.TranscodeDeliveryHLS
		e.HasSubtitle = true
	})
	paths := tr.pathsForEntry(cacheEntry{Session: "hs"})
	if err := os.MkdirAll(paths.hlsDir, 0o700); err != nil {
		t.Fatal(err)
	}
	_ = os.WriteFile(paths.playlist, []byte("#EXTM3U\n#EXTINF:1.0,\nseg0.ts\n"), 0o600)
	_ = os.WriteFile(filepath.Join(paths.hlsDir, "seg0.ts"), []byte("ts"), 0o600)
	_ = os.WriteFile(paths.subtitle, nil, 0o600)
	if _, ok := tr.readyEntry(spec); ok {
		t.Fatal("an HLS entry with a zero-byte subtitle must not be ready")
	}
}

// --- coordinator.go: cleanup ordering ------------------------------------

func TestKillCleanupStaleAndOrphans(t *testing.T) {
	dir := t.TempDir()
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	src := filepath.Join(dir, "in.mkv")
	if err := os.WriteFile(src, []byte("src"), 0o600); err != nil {
		t.Fatal(err)
	}
	fi, _ := os.Stat(src)
	put := func(session string, mutate func(*cacheEntry)) cacheEntry {
		e := cacheEntry{
			Session: session, SourcePath: src, SourceSize: fi.Size(),
			SourceModTime: fi.ModTime().UnixNano(), Profile: "p",
			Delivery: contracts.TranscodeDeliveryProgressive, Method: "transcode",
		}
		if mutate != nil {
			mutate(&e)
		}
		if err := writeJSONAtomic(tr.metaPath(session), &e); err != nil {
			t.Fatal(err)
		}
		return e
	}
	// Stale: zero-byte media.
	put("stale", nil)
	_ = os.WriteFile(tr.mediaPath("stale"), nil, 0o600)
	// Stale: subtitle declared but sidecar missing.
	put("nosub", func(e *cacheEntry) { e.HasSubtitle = true })
	_ = os.WriteFile(tr.mediaPath("nosub"), []byte("m"), 0o600)
	// Kept: subtitle-delivery entry with a real sidecar.
	put("subonly", func(e *cacheEntry) { e.Delivery = contracts.TranscodeDeliverySubtitle })
	_ = os.WriteFile(tr.subtitlePath("subonly"), []byte("WEBVTT"), 0o600)
	// Orphans: artifacts no sidecar claims.
	_ = os.WriteFile(filepath.Join(tr.dir, "orphan.mp4"), []byte("x"), 0o600)
	orphanHLS := filepath.Join(tr.dir, "orphan.hls")
	_ = os.MkdirAll(orphanHLS, 0o700)
	_ = os.WriteFile(filepath.Join(orphanHLS, "seg0.ts"), []byte("x"), 0o600)

	if err := tr.cleanup(false, nil); err != nil {
		t.Fatal(err)
	}
	for _, p := range []string{tr.metaPath("stale"), tr.mediaPath("stale"), tr.metaPath("nosub"),
		filepath.Join(tr.dir, "orphan.mp4"), orphanHLS} {
		if _, err := os.Stat(p); err == nil {
			t.Errorf("%s must be removed by cleanup", filepath.Base(p))
		}
	}
	if _, err := os.Stat(tr.metaPath("subonly")); err != nil {
		t.Error("valid subtitle-only entry must survive cleanup")
	}
}

func TestKillCleanupEvictionOrder(t *testing.T) {
	dir := t.TempDir()
	src := filepath.Join(dir, "in.mkv")
	_ = os.WriteFile(src, []byte("src"), 0o600)
	fi, _ := os.Stat(src)
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{MaxCacheBytes: 200}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	old := tr.now().Add(-2 * time.Hour).Unix()
	put := func(session string, accessed, created int64) {
		e := cacheEntry{
			Session: session, SourcePath: src, SourceSize: fi.Size(),
			SourceModTime: fi.ModTime().UnixNano(), Profile: "p",
			Delivery: contracts.TranscodeDeliveryProgressive, Method: "transcode",
			AccessedAt: accessed, CreatedAt: created,
		}
		if err := writeJSONAtomic(tr.metaPath(session), &e); err != nil {
			t.Fatal(err)
		}
		_ = os.WriteFile(tr.mediaPath(session), make([]byte, 100), 0o600)
	}
	// Same AccessedAt: the CreatedAt tie-break evicts the oldest entry
	// first. Names are chosen so ReadDir's alphabetical order is the
	// reverse of CreatedAt order: a comparator that degrades to "keep
	// input order" (the `==` mutated to `!=`) evicts the wrong session.
	put("z-old", old, 10)
	put("m-mid", old, 20)
	put("a-new", old, 30)
	if err := tr.cleanup(false, nil); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(tr.metaPath("a-new")); err != nil {
		t.Error("newest entry must survive eviction")
	}
	if _, err := os.Stat(tr.metaPath("m-mid")); err != nil {
		t.Error("middle entry must survive: only one eviction fits the budget")
	}
	if _, err := os.Stat(tr.metaPath("z-old")); err == nil {
		t.Error("oldest entry must be evicted first")
	}
}

func TestKillCleanupBudgetExact(t *testing.T) {
	dir := t.TempDir()
	src := filepath.Join(dir, "in.mkv")
	_ = os.WriteFile(src, []byte("src"), 0o600)
	fi, _ := os.Stat(src)
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{MaxCacheBytes: 200}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	old := tr.now().Add(-2 * time.Hour).Unix()
	for _, s := range []string{"a", "b"} {
		e := cacheEntry{
			Session: s, SourcePath: src, SourceSize: fi.Size(), SourceModTime: fi.ModTime().UnixNano(),
			Profile: "p", Delivery: contracts.TranscodeDeliveryProgressive, Method: "transcode",
			AccessedAt: old, CreatedAt: old,
		}
		if err := writeJSONAtomic(tr.metaPath(s), &e); err != nil {
			t.Fatal(err)
		}
		_ = os.WriteFile(tr.mediaPath(s), make([]byte, 100), 0o600)
	}
	// total == budget exactly: nothing may be evicted.
	if err := tr.cleanup(false, nil); err != nil {
		t.Fatal(err)
	}
	for _, s := range []string{"a", "b"} {
		if _, err := os.Stat(tr.metaPath(s)); err != nil {
			t.Errorf("entry %s evicted although total == budget", s)
		}
	}
}

func TestKillCleanupAccessGrace(t *testing.T) {
	dir := t.TempDir()
	src := filepath.Join(dir, "in.mkv")
	_ = os.WriteFile(src, []byte("src"), 0o600)
	fi, _ := os.Stat(src)
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{MaxCacheBytes: 150}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	put := func(session string, accessed int64) {
		e := cacheEntry{
			Session: session, SourcePath: src, SourceSize: fi.Size(), SourceModTime: fi.ModTime().UnixNano(),
			Profile: "p", Delivery: contracts.TranscodeDeliveryProgressive, Method: "transcode",
			AccessedAt: accessed, CreatedAt: accessed,
		}
		if err := writeJSONAtomic(tr.metaPath(session), &e); err != nil {
			t.Fatal(err)
		}
		_ = os.WriteFile(tr.mediaPath(session), make([]byte, 100), 0o600)
	}
	put("recent", tr.now().Unix())                                                 // inside the grace window
	put("old", tr.now().Add((-2 * recentAccessGraceSeconds * time.Second)).Unix()) // evictable
	if err := tr.cleanup(false, nil); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(tr.metaPath("recent")); err != nil {
		t.Error("recently accessed entry must survive eviction despite the budget")
	}
	if _, err := os.Stat(tr.metaPath("old")); err == nil {
		t.Error("stale-access entry must be evicted")
	}
}

func TestKillRemoveArtifactsRoot(t *testing.T) {
	dir := t.TempDir()
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	root := filepath.Join(dir, "relocated")
	sessionDir := artifactDirFor(root, "s1")
	if err := os.MkdirAll(sessionDir, 0o700); err != nil {
		t.Fatal(err)
	}
	e := cacheEntry{Session: "s1", ArtifactRoot: root}
	tr.removeArtifacts(e, tr.pathsForEntry(e))
	if _, err := os.Stat(sessionDir); err == nil {
		t.Error("relocated session directory must be removed")
	}
}

// --- encode.go: argv detail branches --------------------------------------

func TestKillFFmpegArgsEdges(t *testing.T) {
	// StartSec == 0: no -ss at all.
	p := encodePlanFor(func(p *encodePlan) { p.spec.StartSec = 0 })
	if hasArg(p.ffmpegArgs("/tmp/o.mp4", "", false), "-ss") {
		t.Fatal("StartSec=0 must not emit -ss")
	}
	// Empty complexFilter and empty filters: plain -map, no -vf.
	p = encodePlanFor(func(p *encodePlan) { p.filters = nil; p.complexFilter = "" })
	args := p.ffmpegArgs("/tmp/o.mp4", "", false)
	if !hasArg(args, "-map", "0:0") || hasArg(args, "-vf") || hasArg(args, "-filter_complex") {
		t.Fatalf("plain plan args=%v", args)
	}
	// Very low frame rate: the GOP clamps to 1 rather than emitting 0.
	p = encodePlanFor(func(p *encodePlan) {
		p.delivery = contracts.TranscodeDeliveryHLS
		p.video.FrameRate = "1/10"
		p.settings.HLSSegmentSeconds = 4
	})
	args = p.ffmpegArgs("/tmp/o.m3u8", "", false)
	if !hasArg(args, "-g", "1") || !hasArg(args, "-keyint_min", "1") {
		t.Fatalf("low-fps gop args=%v", args)
	}
}

// --- ffmpeg.go ------------------------------------------------------------

func TestKillFrameRateParse(t *testing.T) {
	for _, c := range []struct {
		rate string
		want float64
	}{
		{"0/25", 0}, {"25/0", 0}, {"", 0}, {"abc", 0}, {"-3/1", 0}, {"25", 0},
		{"30000/1001", 30000.0 / 1001.0}, {"50/1", 50},
	} {
		if got := (stream{FrameRate: c.rate}).frameRate(); got != c.want {
			t.Errorf("frameRate(%q)=%v, want %v", c.rate, got, c.want)
		}
	}
}

func TestKillDurationSeconds(t *testing.T) {
	for _, c := range []struct {
		d    string
		want float64
	}{
		{"0", 0}, {"-1.5", 0}, {"abc", 0}, {"", 0}, {"12.5", 12.5},
	} {
		r := mediaReport{}
		r.Format.Duration = c.d
		if got := r.durationSeconds(); got != c.want {
			t.Errorf("durationSeconds(%q)=%v, want %v", c.d, got, c.want)
		}
	}
}

func TestKillConvertMediaProgressFlag(t *testing.T) {
	if _, err := exec.LookPath("ffprobe"); err != nil {
		t.Skip("ffprobe required")
	}
	dir := t.TempDir()
	argsFile := filepath.Join(dir, "args.txt")
	stub := filepath.Join(dir, "ffmpeg")
	script := "#!/bin/sh\nprintf '%s\n' \"$@\" > " + argsFile + "\nfor last; do :; done\nprintf x > \"$last\"\n"
	if err := os.WriteFile(stub, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", dir+string(os.PathListSeparator)+os.Getenv("PATH"))
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	spec := sourceSpec{Path: "/nonexistent/in.mkv", Profile: legacyProfile}
	out := filepath.Join(dir, "out.mp4")

	if _, err := tr.convertMedia(spec, out, func(progressSample) {}); err != nil {
		t.Fatalf("convertMedia: %v", err)
	}
	raw, _ := os.ReadFile(argsFile)
	if !strings.Contains(string(raw), "-progress") {
		t.Fatalf("onSample set must request -progress, argv=%q", raw)
	}
	if _, err := tr.convertMedia(spec, out, nil); err != nil {
		t.Fatalf("convertMedia: %v", err)
	}
	raw, _ = os.ReadFile(argsFile)
	if strings.Contains(string(raw), "-progress") {
		t.Fatalf("nil onSample must not request -progress, argv=%q", raw)
	}
}

func TestKillConsumeProgressGuards(t *testing.T) {
	var got []progressSample
	feed := "out_time_us=-5\nout_time_us=25000000\nfps=0\nfps=25.5\nspeed=0\nspeed=1.5x\nbitrate=1200kbits/s\nprogress=continue\n"
	consumeProgress(strings.NewReader(feed), 100, func(s progressSample) { got = append(got, s) })
	if len(got) != 1 {
		t.Fatalf("samples=%v", got)
	}
	s := got[0]
	if !s.HasFraction || s.Fraction != 0.25 || s.FPS != 25.5 || s.Speed != 1.5 || s.BitrateKbps != 1200 {
		t.Fatalf("sample=%+v", s)
	}
	// A block of only invalid values produces no sample at all (the
	// emit guard requires at least one real metric).
	got = nil
	consumeProgress(strings.NewReader("fps=0\nspeed=abc\nspeed=-2x\nprogress=continue\n"), 0, func(s progressSample) { got = append(got, s) })
	if len(got) != 0 {
		t.Fatalf("invalid values must be ignored: %+v", got)
	}
	// progress=end pins the fraction to 1 even with no position data.
	consumeProgress(strings.NewReader("progress=end\n"), 100, func(s progressSample) { got = append(got, s) })
	if len(got) != 1 || got[0].Fraction != 1 || !got[0].HasFraction {
		t.Fatalf("end marker: %+v", got)
	}
}

// --- hls.go ---------------------------------------------------------------

func TestKillParseHLSPlaylist(t *testing.T) {
	dir := t.TempDir()
	pl := strings.Join([]string{
		"#EXTM3U",
		"#EXT-X-MEDIA-SEQUENCE:5",
		"#EXTINF:6.000,",
		"seg5.ts",
		"#EXT-X-DISCONTINUITY",
		"#EXTINF:3.5", // EXTINF without a comma still parses
		"seg6.ts",
		"#EXT-X-UNKNOWN-TAG:x",
		"#EXTINF:4.0,",
		"seg7.ts",
		"#EXT-X-ENDLIST",
	}, "\n")
	path := filepath.Join(dir, "raw.m3u8")
	if err := os.WriteFile(path, []byte(pl), 0o600); err != nil {
		t.Fatal(err)
	}
	segs, ended, err := parseHLSPlaylist(path)
	if err != nil || !ended {
		t.Fatalf("parse: ended=%v err=%v", ended, err)
	}
	if len(segs) != 3 {
		t.Fatalf("segments=%v", segs)
	}
	if segs[0].MediaSeq != 5 || segs[0].Duration != 6 || segs[0].URI != "seg5.ts" {
		t.Fatalf("seg0=%+v", segs[0])
	}
	if !segs[1].IsDiscont || segs[1].MediaSeq != 6 || segs[1].Duration != 3.5 {
		t.Fatalf("seg1=%+v", segs[1])
	}
	if segs[2].Duration != 4 || segs[2].StartSec <= segs[1].StartSec {
		t.Fatalf("seg2=%+v", segs[2])
	}
	// No ENDLIST: ended is false.
	open := strings.Join([]string{"#EXTM3U", "#EXTINF:6.0,", "s0.ts"}, "\n")
	_ = os.WriteFile(path, []byte(open), 0o600)
	if _, ended, err := parseHLSPlaylist(path); err != nil || ended {
		t.Fatalf("open playlist: ended=%v err=%v", ended, err)
	}
}

func TestKillDeleteConsumedSegmentsBounds(t *testing.T) {
	dir := t.TempDir()
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	settings := contracts.DefaultTranscodeSettings()
	settings.SegmentDeletion = true
	settings.SegmentKeepSec = 0
	hlsDir := tr.pathsFor("del", settings).hlsDir
	if err := os.MkdirAll(hlsDir, 0o700); err != nil {
		t.Fatal(err)
	}
	var lines []string
	for _, n := range []string{"seg0.ts", "seg1.ts", "seg2.ts"} {
		lines = append(lines, "#EXTINF:6.0,", n)
		_ = os.WriteFile(filepath.Join(hlsDir, n), []byte("x"), 0o600)
	}
	pl := "#EXTM3U\n" + strings.Join(lines, "\n") + "\n"
	if err := os.WriteFile(filepath.Join(hlsDir, hlsRawPlaylist), []byte(pl), 0o600); err != nil {
		t.Fatal(err)
	}
	j := &job{spec: sourceSpec{Session: "del"}, delivery: contracts.TranscodeDeliveryHLS,
		settings: settings, clientSegment: 3, done: make(chan struct{})} // == len(segments): out of range
	tr.deleteConsumedSegments(j)
	for _, n := range []string{"seg0.ts", "seg1.ts", "seg2.ts"} {
		if _, err := os.Stat(filepath.Join(hlsDir, n)); err != nil {
			t.Fatalf("out-of-range clientSegment deleted %s", n)
		}
	}
	j.clientSegment = 1 // EndSec 12; cutoff 12: seg0 and seg1 fully behind
	tr.deleteConsumedSegments(j)
	for _, n := range []string{"seg0.ts", "seg1.ts"} {
		if _, err := os.Stat(filepath.Join(hlsDir, n)); err == nil {
			t.Errorf("consumed %s must be deleted", n)
		}
	}
	if _, err := os.Stat(filepath.Join(hlsDir, "seg2.ts")); err != nil {
		t.Error("segment ahead of the client must survive")
	}
}

// --- policy.go ------------------------------------------------------------

func TestKillPlanQualityCaps(t *testing.T) {
	settings := contracts.DefaultTranscodeSettings()
	settings.Qualities = []contracts.TranscodeQuality{
		{Name: "low", MaxWidth: 640, MaxHeight: 360, BitrateKbps: 500},
		{Name: "nocap"},
		{Name: "wide", MaxWidth: 640},
		{Name: "exact-h", MaxHeight: 1080},
		{Name: "half-h", MaxHeight: 720},
	}
	plan, err := planFor(t, v3Report(), testCaps(), func(in *contracts.TranscodeV3Request) {
		in.Settings = settings
		in.Quality = "low"
		in.MaxBitrateKbps = 400 // tighter than the ladder: wins
	})
	if err != nil {
		t.Fatal(err)
	}
	if plan.width != 640 || plan.height != 360 || plan.maxBitrateKbps != 400 {
		t.Fatalf("quality caps: w=%d h=%d br=%d", plan.width, plan.height, plan.maxBitrateKbps)
	}
	if plan.copyVideo {
		t.Fatal("a downscale plan must not copy the video stream")
	}
	// Ladder bitrate tighter than the user cap: the ladder wins.
	plan, err = planFor(t, v3Report(), testCaps(), func(in *contracts.TranscodeV3Request) {
		in.Settings = settings
		in.Quality = "low"
		in.MaxBitrateKbps = 900
	})
	if err != nil {
		t.Fatal(err)
	}
	if plan.maxBitrateKbps != 500 {
		t.Fatalf("ladder bitrate must apply: %d", plan.maxBitrateKbps)
	}
	// Zero-valued caps do not clamp — neither dimension nor bitrate. The
	// height assertion kills `q.MaxHeight > 0` -> `>=`: a zero cap would
	// then overwrite plan.height with 0 via min().
	plan, err = planFor(t, v3Report(), testCaps(), func(in *contracts.TranscodeV3Request) {
		in.Settings = settings
		in.Quality = "nocap"
	})
	if err != nil {
		t.Fatal(err)
	}
	if plan.width != 1920 || plan.height != 1080 || plan.maxBitrateKbps != 0 {
		t.Fatalf("nocap quality: w=%d h=%d br=%d", plan.width, plan.height, plan.maxBitrateKbps)
	}
	if hasScaleFilter(plan) {
		t.Fatal("an uncapped plan must not gain a scale filter")
	}
	if !plan.copyVideo {
		t.Fatal("an uncapped plan on a copyable source must keep copyVideo")
	}
	// A width-only cap below the source must scale (the width clause of
	// the scale condition alone decides it).
	plan, err = planFor(t, v3Report(), testCaps(), func(in *contracts.TranscodeV3Request) {
		in.Settings = settings
		in.Quality = "wide"
	})
	if err != nil {
		t.Fatal(err)
	}
	if !hasScaleFilter(plan) {
		t.Fatal("a width cap below the source must add a scale filter")
	}
	if plan.copyVideo {
		t.Fatal("scaling forbids stream copy")
	}
	// A height cap equal to the source height adds no filter and keeps
	// the copy eligible (>=, not >).
	plan, err = planFor(t, v3Report(), testCaps(), func(in *contracts.TranscodeV3Request) {
		in.Settings = settings
		in.Quality = "exact-h"
	})
	if err != nil {
		t.Fatal(err)
	}
	if hasScaleFilter(plan) {
		t.Fatal("a height cap equal to the source must not scale")
	}
	if !plan.copyVideo {
		t.Fatal("a cap that does not shrink the frame must keep copyVideo")
	}
	// A height-only cap below the source must force a re-encode: the
	// width clause stays satisfied, so only the height guard decides.
	plan, err = planFor(t, v3Report(), testCaps(), func(in *contracts.TranscodeV3Request) {
		in.Settings = settings
		in.Quality = "half-h"
	})
	if err != nil {
		t.Fatal(err)
	}
	if plan.copyVideo {
		t.Fatal("a height downscale must not copy the video stream")
	}
	if !hasScaleFilter(plan) {
		t.Fatal("a height downscale must add a scale filter")
	}
}

func hasScaleFilter(p encodePlan) bool {
	for _, f := range p.filters {
		if strings.HasPrefix(f, "scale") {
			return true
		}
	}
	return false
}

func TestKillPlanBitrateCeilingForcesEncode(t *testing.T) {
	report := mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "h264", PixelFormat: "yuv420p",
			Width: 1920, Height: 1080, BitRate: "4000000"},
		{Index: 1, CodecType: "audio", CodecName: "aac", Channels: 2, Default: 1},
	}}
	spec := testSource()
	spec.Path = "/media/in.mp4"
	// Source bitrate equal to the ceiling: copy still allowed (> not >=).
	tr := planFixture(t, report, testCaps())
	in := testRequest(contracts.DefaultTranscodeSettings())
	in.MaxBitrateKbps = 4000
	plan, err := tr.planV3(spec, in, testCaps())
	if err != nil {
		t.Fatal(err)
	}
	if !plan.copyVideo {
		t.Fatal("source bitrate equal to the cap must still copy")
	}
	// One kbps over: forces a re-encode.
	in.MaxBitrateKbps = 3999
	plan, err = tr.planV3(spec, in, testCaps())
	if err != nil {
		t.Fatal(err)
	}
	if plan.copyVideo {
		t.Fatal("source bitrate over the cap must force a re-encode")
	}
}

func TestKillReasonBitrateLimit(t *testing.T) {
	p := encodePlanFor(func(p *encodePlan) { p.copyVideo = true; p.copyAudio = true; p.maxBitrateKbps = 500 })
	p.spec.Path = "/media/in.mp4"
	for _, r := range p.transcodeReasons(contracts.TranscodeV3Request{}) {
		if strings.Contains(r, "bitrate limit") {
			t.Fatalf("copy plan must not claim a bitrate-limit reason: %v", r)
		}
	}
	p = encodePlanFor(func(p *encodePlan) { p.maxBitrateKbps = 500 })
	found := false
	for _, r := range p.transcodeReasons(contracts.TranscodeV3Request{}) {
		found = found || strings.Contains(r, "bitrate limit (500 kbps)")
	}
	if !found {
		t.Fatal("transcoding under a cap must report the bitrate limit")
	}
	// No cap on a real transcode: no bitrate-limit reason may appear.
	p = encodePlanFor(func(p *encodePlan) {})
	for _, r := range p.transcodeReasons(contracts.TranscodeV3Request{}) {
		if strings.Contains(r, "bitrate limit") {
			t.Fatalf("an uncapped transcode must not claim a bitrate limit: %v", r)
		}
	}
}

func TestKillFileExt(t *testing.T) {
	for _, c := range []struct{ in, want string }{
		{".mkv", ".mkv"}, {"a/b/c.mp4", ".mp4"}, {"noext", ""}, {"a.b/x", ".b/x"}, {"x.", "."},
	} {
		if got := fileExt(c.in); got != c.want {
			t.Errorf("fileExt(%q)=%q, want %q", c.in, got, c.want)
		}
	}
}

func TestKillScaleFilter(t *testing.T) {
	if got := scaleFilter(640, 360); !strings.Contains(got, "min(iw,640)") || !strings.Contains(got, "min(ih,360)") {
		t.Errorf("both dims: %q", got)
	}
	if got := scaleFilter(0, 360); got != "scale=-2:'min(ih,360)'" {
		t.Errorf("height only: %q", got)
	}
	if got := scaleFilter(640, 0); got != "scale='min(iw,640)':-2" {
		t.Errorf("width only: %q", got)
	}
}

func TestKillChooseEncoderNoHWName(t *testing.T) {
	// VideoToolbox has no AV1 encoder name in hwNames: the lookup must
	// fall back to software with an explanatory note.
	settings := contracts.DefaultTranscodeSettings()
	settings.HardwareAcceleration = contracts.HWVideoToolbox
	settings.HardwareEncode = true
	settings.HardwareDecodeCodecs = contracts.HardwareDecodeCodecs
	report := mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "h264", PixelFormat: "yuv420p", Width: 640, Height: 360},
	}}
	video := &report.Streams[0]
	caps := testCaps()
	caps.Hardware = map[string]bool{contracts.HWVideoToolbox: true}
	encoder, _, _, fallback, err := chooseEncoder(settings, contracts.VideoCodecAV1, report, caps, video)
	if err != nil {
		t.Fatal(err)
	}
	if encoder == "" || strings.Contains(encoder, "videotoolbox") {
		t.Fatalf("encoder=%q, want software", encoder)
	}
	if !strings.Contains(fallback, "cannot encode") {
		t.Fatalf("fallback=%q, want the cannot-encode note", fallback)
	}
}

func TestKillParseFFmpegList(t *testing.T) {
	set := map[string]bool{}
	out := "vaapi\n V....= libx264\n V....= =\nenc:foo bar\n  \n V..... hevc_nvenc\n"
	parseFFmpegList([]byte(out), set)
	if !set["vaapi"] || !set["libx264"] || !set["hevc_nvenc"] {
		t.Fatalf("set=%v", set)
	}
	if set["="] || set["enc:foo"] || set["foo"] {
		t.Fatalf("flag/=: set=%v", set)
	}
}

func TestKillFFmpegBinary(t *testing.T) {
	if got := ffmpegBinary(contracts.TranscodeSettings{}); got != "ffmpeg" {
		t.Fatalf("default: %q", got)
	}
	if got := ffmpegBinary(contracts.TranscodeSettings{FFmpegPath: "/opt/ff"}); got != "/opt/ff" {
		t.Fatalf("custom: %q", got)
	}
}

// --- v3.go ------------------------------------------------------------------

func TestKillCancelV3KillsProcess(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "c"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	// Queued v3 job with no process: cancel is clean.
	j := &job{spec: sourceSpec{Session: "q1", Path: "/media/a.mkv"}, state: contracts.TranscodeQueued,
		v3: true, settings: contracts.DefaultTranscodeSettings(), plan: &encodePlan{}, done: make(chan struct{})}
	tr.jobs["q1"] = j
	if st := tr.cancelV3("q1", "/media/a.mkv", ""); st.State != contracts.TranscodeIdle {
		t.Fatalf("cancel queued: %q", st.State)
	}
	// Running job with a live process: the kill branch runs.
	cmd := exec.Command("sleep", "60")
	if err := cmd.Start(); err != nil {
		t.Skip(err)
	}
	j2 := &job{spec: sourceSpec{Session: "r1", Path: "/media/b.mkv"}, state: contracts.TranscodeRunning,
		v3: true, settings: contracts.DefaultTranscodeSettings(), plan: &encodePlan{}, cmd: cmd, done: make(chan struct{})}
	tr.jobs["r1"] = j2
	if st := tr.cancelV3("r1", "/media/b.mkv", ""); st.State != contracts.TranscodeIdle {
		t.Fatalf("cancel running: %q", st.State)
	}
	if err := cmd.Wait(); err == nil {
		t.Fatal("cancelled session's process must be killed")
	}
}

func TestKillStartV3QueueFullBoundary(t *testing.T) {
	tr, src := v3Fixture(t, Config{})
	in := v3Request(contracts.TranscodeStartAction, src)
	in.Settings.QueueSize = 1 // adoption sets queueLimit=1
	tr.mu.Lock()
	tr.pending = 1 // exactly at the limit: admission must fail
	tr.mu.Unlock()
	_, err := tr.Invoke(contracts.CapPlaybackTranscodeV3, in)
	var ce *core.Error
	if err == nil || !errors.As(err, &ce) || ce.Code != "queue-full" {
		t.Fatalf("pending==limit: err=%v, want queue-full", err)
	}
	tr.mu.Lock()
	tr.pending = 0
	tr.mu.Unlock()
	if _, err := tr.Invoke(contracts.CapPlaybackTranscodeV3, in); err != nil {
		t.Fatalf("pending<limit: err=%v", err)
	}
}

func TestKillStatusV3DirectFlags(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "c"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	j := &job{spec: sourceSpec{Session: "s1"}, state: contracts.TranscodeReady, done: make(chan struct{}),
		plan: &encodePlan{copyVideo: true, copyAudio: false}}
	st := tr.statusV3FromJob(j)
	if !st.VideoDirect || st.AudioDirect {
		t.Fatalf("direct flags: %+v", st)
	}
	j.plan = nil
	st = tr.statusV3FromJob(j)
	if st.VideoDirect || st.AudioDirect {
		t.Fatalf("nil plan must report no direct flags: %+v", st)
	}
}

// --- coordinator.go: Health, invokeV2 touch, run/Close guards --------------

func TestKillHealthFFmpeg(t *testing.T) {
	// With no ffmpeg on PATH and no convert seam, Health must fail; the
	// `&&` mutant (`||`) would also fail when ffmpeg exists.
	t.Setenv("PATH", t.TempDir())
	tr := newWithDeps(filepath.Join(t.TempDir(), "c"), Config{}, nil, time.Now)
	if err := tr.Health(); err == nil {
		t.Fatal("no ffmpeg and no convert seam: Health must fail")
	}
	_ = tr.Close()
	stub := func(spec sourceSpec, out string, onSample func(progressSample)) (string, error) {
		return "", nil
	}
	tr = newWithDeps(filepath.Join(t.TempDir(), "c"), Config{}, stub, time.Now)
	if err := tr.Health(); err != nil {
		t.Fatalf("a convert seam substitutes for ffmpeg: %v", err)
	}
	_ = tr.Close()
}

func TestKillInvokeV2Touch(t *testing.T) {
	dir := t.TempDir()
	src := filepath.Join(dir, "in.mkv")
	if err := os.WriteFile(src, []byte("src"), 0o600); err != nil {
		t.Fatal(err)
	}
	fi, _ := os.Stat(src)
	cur := time.Unix(1_700_000_000, 0)
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{}, nil, func() time.Time { return cur })
	t.Cleanup(func() { _ = tr.Close() })
	session := strings.Repeat("a", 64)
	old := cur.Add(-time.Hour).Unix()
	e := cacheEntry{
		Session: session, SourcePath: src, SourceSize: fi.Size(),
		SourceModTime: fi.ModTime().UnixNano(), Profile: "p",
		Delivery: contracts.TranscodeDeliveryProgressive, Method: "transcode",
		AccessedAt: old, CreatedAt: old,
	}
	if err := writeJSONAtomic(tr.metaPath(session), &e); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(tr.mediaPath(session), []byte("mp4"), 0o600); err != nil {
		t.Fatal(err)
	}
	readAccess := func() int64 {
		var got cacheEntry
		if err := readJSON(tr.metaPath(session), &got); err != nil {
			t.Fatal(err)
		}
		return got.AccessedAt
	}
	// A status poll must not refresh the LRU timestamp; a resolve does.
	if _, err := tr.invokeV2(contracts.TranscodeV2Request{
		FilePath: src, Session: session, Action: contracts.TranscodeStatusAction,
	}); err != nil {
		t.Fatal(err)
	}
	if got := readAccess(); got != old {
		t.Fatalf("status touched AccessedAt: %d, want %d", got, old)
	}
	if _, err := tr.invokeV2(contracts.TranscodeV2Request{
		FilePath: src, Session: session, Action: contracts.TranscodeResolveAction,
	}); err != nil {
		t.Fatal(err)
	}
	if got := readAccess(); got != cur.Unix() {
		t.Fatalf("resolve must touch AccessedAt: %d, want %d", got, cur.Unix())
	}
}

func TestKillRunNonQueuedGuard(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "c"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	// A finished job re-run must be a no-op; a mutant that drops or
	// negates the queued-state guard executes it again.
	j := &job{spec: sourceSpec{Session: "r", Path: "/media/none.mkv"},
		state: contracts.TranscodeReady, done: make(chan struct{})}
	tr.run(j)
	if j.state != contracts.TranscodeReady {
		t.Fatalf("a non-queued job must not run, got state %q", j.state)
	}
	// A queued job must leave the queued state; an early return here
	// would leave the job queued forever while waiters block on done.
	j2 := &job{spec: sourceSpec{Session: "q", Path: "/media/none.mkv"},
		state: contracts.TranscodeQueued, done: make(chan struct{})}
	tr.run(j2)
	if j2.state == contracts.TranscodeQueued {
		t.Fatal("a queued job must be picked up")
	}
}

func TestKillCloseNonQueuedJob(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "c"), Config{}, nil, time.Now)
	// A Ready job has no done waiter; the `==` mutant would close(nil)
	// and panic inside Close.
	tr.jobs["r"] = &job{spec: sourceSpec{Session: "r"}, state: contracts.TranscodeReady}
	if err := tr.Close(); err != nil {
		t.Fatal(err)
	}
	if tr.jobs["r"].state != contracts.TranscodeReady {
		t.Fatal("Close must not touch a finished job")
	}
}
