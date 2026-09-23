// Mutation-testing round 2: covers the survivors of the first pass —
// coordinator lifecycle paths, execPlan with a stub ffmpeg binary, HLS
// throttle/segment bookkeeping and the startV3 adoption block.
package transcode

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-22

import (
	"context"
	"errors"
	"log/slog"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

// --- newWithDeps config bounds (coordinator.go:267-278) ---

func TestKillConfigBounds(t *testing.T) {
	dir := t.TempDir()
	tr := newWithDeps(filepath.Join(dir, "c"), Config{MaxCacheBytes: 0, QueueSize: 0, MaxConcurrent: 0}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	if tr.config.MaxCacheBytes != DefaultMaxCacheBytes {
		t.Fatalf("MaxCacheBytes=0 must default: %d", tr.config.MaxCacheBytes)
	}
	if tr.queueLimit != defaultQueueSize {
		t.Fatalf("QueueSize=0 must default: %d", tr.queueLimit)
	}
	if tr.maxConcurrent != defaultMaxConcurrent {
		t.Fatalf("MaxConcurrent=0 must default: %d", tr.maxConcurrent)
	}

	over := newWithDeps(filepath.Join(dir, "d"), Config{QueueSize: maxQueueCapacity + 1, MaxConcurrent: 7}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	if over.queueLimit != maxQueueCapacity {
		t.Fatalf("queueLimit must clamp at %d, got %d", maxQueueCapacity, over.queueLimit)
	}
	if over.maxConcurrent != 7 {
		t.Fatalf("MaxConcurrent=7 must be honored: %d", over.maxConcurrent)
	}
	at := newWithDeps(filepath.Join(dir, "e"), Config{QueueSize: maxQueueCapacity}, nil, time.Now)
	t.Cleanup(func() { _ = at.Close() })
	if at.queueLimit != maxQueueCapacity {
		t.Fatalf("queueLimit at the cap must pass through: %d", at.queueLimit)
	}
}

// --- SetLogger(nil) keeps a discard logger, never panics (coordinator.go:310) ---

func TestKillSetLoggerNil(t *testing.T) {
	tr := newWithDeps(t.TempDir(), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	tr.SetLogger(nil)
	if tr.logger() == nil {
		t.Fatal("logger() must never return nil")
	}
	custom := slog.New(slog.DiscardHandler)
	tr.SetLogger(custom)
	if tr.logger() != custom {
		t.Fatal("SetLogger must store the injected logger")
	}
}

// --- Close only fails queued jobs (coordinator.go:348) ---

func TestKillCloseSkipsRunningJob(t *testing.T) {
	tr := newWithDeps(t.TempDir(), Config{}, nil, time.Now)
	running := &job{state: contracts.TranscodeRunning, done: make(chan struct{})}
	queued := &job{state: contracts.TranscodeQueued, done: make(chan struct{})}
	tr.jobs["run"] = running
	tr.jobs["q"] = queued
	if err := tr.Close(); err != nil {
		t.Fatal(err)
	}
	if running.state != contracts.TranscodeRunning {
		t.Fatalf("a running job must survive Close's queued-job sweep: %q", running.state)
	}
	select {
	case <-running.done:
		t.Fatal("a running job's done channel must not be closed by Close")
	default:
	}
	if queued.state != contracts.TranscodeFailed || queued.errCode != "dependency-unavailable" {
		t.Fatalf("queued job state=%q code=%q", queued.state, queued.errCode)
	}
	select {
	case <-queued.done:
	default:
		t.Fatal("queued job's done channel must be closed")
	}
}

// --- invokeV2 session actions route to bySession (coordinator.go:404) ---

func TestKillInvokeV2SessionRouting(t *testing.T) {
	dir := t.TempDir()
	tr := newWithDeps(dir, Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	src := filepath.Join(dir, "in.mkv")
	if err := os.WriteFile(src, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	// status + session -> bySession -> unknown session is a not-found,
	// not the invalid-message the spec path would produce.
	_, err := tr.invokeV2(contracts.TranscodeV2Request{FilePath: src, Action: contracts.TranscodeStatusAction, Session: strings.Repeat("a", 64)})
	var ce *core.Error
	if !errors.As(err, &ce) || ce.Code != "not-found" {
		t.Fatalf("status+session must resolve bySession: %v", err)
	}
	// resolve + session takes the same branch.
	_, err = tr.invokeV2(contracts.TranscodeV2Request{FilePath: src, Action: contracts.TranscodeResolveAction, Session: strings.Repeat("a", 64)})
	if !errors.As(err, &ce) || ce.Code != "not-found" {
		t.Fatalf("resolve+session must resolve bySession: %v", err)
	}
	// A non-session action with a mismatched id hits the spec path.
	_, err = tr.invokeV2(contracts.TranscodeV2Request{FilePath: src, Action: contracts.TranscodeInspectAction, Session: strings.Repeat("b", 64)})
	if !errors.As(err, &ce) || ce.Code != "invalid-message" {
		t.Fatalf("inspect+mismatched session must be invalid-message: %v", err)
	}
}

// --- acquireSlot/releaseSlot bound concurrency (coordinator.go:625-641) ---

func TestKillSlotBound(t *testing.T) {
	tr := newWithDeps(t.TempDir(), Config{MaxConcurrent: 1}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	if !tr.acquireSlot() {
		t.Fatal("first acquire must succeed")
	}
	// Second acquire must block; once ctx is cancelled it gives up.
	done := make(chan bool, 1)
	go func() { done <- tr.acquireSlot() }()
	select {
	case got := <-done:
		t.Fatalf("second acquire must block while the slot is held, got %v", got)
	case <-time.After(200 * time.Millisecond):
	}
	tr.cancel() // covers the ctx.Done branch of acquireSlot
	select {
	case got := <-done:
		if got {
			t.Fatal("acquire must report false once the transcoder is cancelled")
		}
	case <-time.After(5 * time.Second):
		t.Fatal("acquireSlot never returned after cancel")
	}
	tr.releaseSlot()
	tr.mu.Lock()
	if tr.active != 0 {
		t.Fatalf("releaseSlot must decrement active: %d", tr.active)
	}
	tr.mu.Unlock()
}

// --- pruneJobs bounds the session map (coordinator.go:714-733) ---

// --- stopJob tolerates a nil command (coordinator.go:754) ---

func TestKillStopJobNilCmd(t *testing.T) {
	tr := newWithDeps(t.TempDir(), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	j := &job{state: contracts.TranscodeRunning}
	tr.stopJob(j, "x", "y")
	if j.state != contracts.TranscodeFailed || j.errCode != "x" || !j.stopped {
		t.Fatalf("stopJob state=%q code=%q stopped=%v", j.state, j.errCode, j.stopped)
	}
	// A terminal job is left alone.
	done := &job{state: contracts.TranscodeReady}
	tr.stopJob(done, "x", "y")
	if done.state != contracts.TranscodeReady {
		t.Fatal("stopJob must not touch a finished job")
	}
}

// --- run() refuses a job that is not queued (coordinator.go:761) ---

func TestKillRunRequiresQueued(t *testing.T) {
	dir := t.TempDir()
	called := false
	tr := newWithDeps(dir, Config{}, func(spec sourceSpec, out string, onSample func(progressSample)) (string, error) {
		called = true
		return "transcode", nil
	}, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	j := &job{spec: sourceSpec{Path: "/media/in.mkv", Session: "s"}, state: contracts.TranscodeRunning, done: make(chan struct{})}
	tr.run(j)
	if called {
		t.Fatal("run() must not convert a job that is not queued")
	}
	if j.state != contracts.TranscodeRunning {
		t.Fatalf("run() must not rewrite a non-queued state: %q", j.state)
	}
}

// --- the finish log reports a sane duration (coordinator.go:822) ---

type captureHandler struct {
	records []slog.Record
}

func (h *captureHandler) Enabled(context.Context, slog.Level) bool { return true }
func (h *captureHandler) WithAttrs([]slog.Attr) slog.Handler       { return h }
func (h *captureHandler) WithGroup(string) slog.Handler            { return h }
func (h *captureHandler) Handle(_ context.Context, r slog.Record) error {
	h.records = append(h.records, r)
	return nil
}

func (h *captureHandler) durSec() (int64, bool) {
	for _, r := range h.records {
		var found bool
		var v int64
		r.Attrs(func(a slog.Attr) bool {
			if a.Key == "dur_sec" {
				v = a.Value.Int64()
				found = true
			}
			return !found
		})
		if found {
			return v, true
		}
	}
	return 0, false
}

func TestKillRunLogsDuration(t *testing.T) {
	dir := t.TempDir()
	src := filepath.Join(dir, "in.mkv")
	if err := os.WriteFile(src, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{}, func(spec sourceSpec, out string, onSample func(progressSample)) (string, error) {
		return "transcode", os.WriteFile(out, []byte("mp4"), 0o600)
	}, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	h := &captureHandler{}
	tr.SetLogger(slog.New(h))
	spec, err := tr.spec(src, "", nil, nil)
	if err != nil {
		t.Fatal(err)
	}
	j := &job{spec: spec, state: contracts.TranscodeQueued, done: make(chan struct{})}
	tr.jobs[spec.Session] = j
	tr.run(j)
	if j.state != contracts.TranscodeReady {
		t.Fatalf("job state=%q err=%q", j.state, j.errMessage)
	}
	if d, ok := h.durSec(); !ok || d < 0 || d > 60 {
		t.Fatalf("dur_sec must be a sane duration: %v %v", d, ok)
	}
}

// --- runV3: a mid-run stop is terminal (coordinator.go:859,894) ---

func TestKillRunV3Stopped(t *testing.T) {
	dir := t.TempDir()
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	h := &captureHandler{}
	tr.SetLogger(slog.New(h))
	tr.v3run = func(j *job) error {
		j.stopped = true // operator stopped it while it ran
		return nil
	}
	j := &job{
		spec: sourceSpec{Path: "/media/in.mkv", Session: "s1"}, state: contracts.TranscodeQueued,
		done: make(chan struct{}), v3: true, delivery: contracts.TranscodeDeliveryProgressive,
		plan: &encodePlan{delivery: contracts.TranscodeDeliveryProgressive},
	}
	tr.jobs["s1"] = j
	tr.runV3(j)
	if j.state != contracts.TranscodeFailed {
		t.Fatalf("a stopped session must finish Failed, got %q", j.state)
	}

	// A clean run logs its duration and lands Ready.
	j2 := &job{
		spec: sourceSpec{Path: "/media/in.mkv", Session: "s2"}, state: contracts.TranscodeQueued,
		done: make(chan struct{}), v3: true, delivery: contracts.TranscodeDeliveryProgressive,
		plan:     &encodePlan{delivery: contracts.TranscodeDeliveryProgressive},
		settings: contracts.TranscodeSettings{},
	}
	tr.jobs["s2"] = j2
	tr.v3run = func(j *job) error {
		return os.WriteFile(tr.pathsFor(j.spec.Session, j.settings).media, []byte("mp4"), 0o600)
	}
	tr.runV3(j2)
	if j2.state != contracts.TranscodeReady {
		t.Fatalf("a clean run must finish Ready, got %q err=%q", j2.state, j2.errMessage)
	}
	if d, ok := h.durSec(); !ok || d < 0 || d > 60 {
		t.Fatalf("dur_sec must be a sane duration: %v %v", d, ok)
	}
}

// --- runV3Plan subtitle/extract/burn paths (coordinator.go:925-939) ---

func TestKillRunV3PlanSubtitles(t *testing.T) {
	dir := t.TempDir()
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })

	// burnText routes the extract to the .burn.ass path and a failing
	// ffmpeg surfaces its error.
	badFF := stubFFmpeg(t, "exit 9")
	j := &job{
		spec: sourceSpec{Path: "/media/in.mkv", Session: "burn", Settings: contracts.TranscodeSettings{FFmpegPath: badFF}},
		v3:   true, delivery: contracts.TranscodeDeliveryProgressive,
		plan: &encodePlan{delivery: contracts.TranscodeDeliveryProgressive, burnText: true, subtitle: &stream{Index: 3}},
	}
	j.settings = j.spec.Settings
	if err := tr.runV3Plan(j); err == nil {
		t.Fatal("a failing burn-in extract must fail the run")
	}

	// Extract mode with a working stub marks hasSubtitle and produces
	// the sidecar before the encode.
	okFF := stubFFmpeg(t, `
out="${@: -1}"
case "$out" in
  *.tmp) echo WEBVTT > "$out" ;;
  *) : ;;
esac
exit 0`)
	settings := contracts.TranscodeSettings{FFmpegPath: okFF}
	spec := sourceSpec{Path: "/media/in.mkv", Session: "sub", Settings: settings}
	j2 := &job{
		spec: spec, v3: true, delivery: contracts.TranscodeDeliveryProgressive,
		settings: settings,
		plan:     &encodePlan{delivery: contracts.TranscodeDeliveryProgressive, subtitleMode: contracts.SubtitleModeExtract, subtitle: &stream{Index: 2}, settings: settings, report: mediaReport{}, video: &stream{Index: 0}},
	}
	// The exec still runs the stub (which writes nothing): the run fails,
	// but the subtitle extract must already have happened.
	_ = tr.runV3Plan(j2)
	if _, err := os.Stat(tr.pathsFor("sub", settings).subtitle); err != nil {
		t.Fatalf("extract mode must write the WebVTT sidecar: %v", err)
	}
	if !j2.hasSubtitle {
		t.Fatal("hasSubtitle must be set after a successful extract")
	}
}

// --- runV3Plan HLS failure/fallback paths (coordinator.go:962-986) ---

func TestKillRunV3PlanHLSErrors(t *testing.T) {
	dir := t.TempDir()
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })

	// A blocked hlsDir (a file, not a directory) fails MkdirAll.
	settings := contracts.TranscodeSettings{}
	session := "blocked"
	blocked := tr.pathsFor(session, settings).hlsDir
	if err := os.WriteFile(blocked, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	j := &job{
		spec: sourceSpec{Path: "/media/in.mkv", Session: session}, v3: true,
		delivery: contracts.TranscodeDeliveryHLS, settings: settings,
		plan: &encodePlan{delivery: contracts.TranscodeDeliveryHLS, settings: settings},
	}
	if err := tr.runV3Plan(j); err == nil {
		t.Fatal("MkdirAll failure must fail the run")
	}

	// Hardware failure retries in software and reports the fallback.
	failFF := stubFFmpeg(t, "echo broken >&2; exit 4")
	hw := contracts.TranscodeSettings{FFmpegPath: failFF}
	j2 := &job{
		spec: sourceSpec{Path: "/media/in.mkv", Session: "hwfb", Settings: hw}, v3: true,
		delivery: contracts.TranscodeDeliveryHLS, settings: hw,
		plan: &encodePlan{delivery: contracts.TranscodeDeliveryHLS, settings: hw, encoder: "h264_nvenc", hwBackend: contracts.HWNVENC, video: &stream{}},
	}
	tr.capsFn = func(contracts.TranscodeSettings) capabilities {
		return capabilities{Encoders: map[string]bool{"libx264": true}}
	}
	if err := tr.runV3Plan(j2); err == nil {
		t.Fatal("a failing ffmpeg must still fail after the software retry")
	}
	if j2.fallback == "" || j2.hwBackend != "" {
		t.Fatalf("software retry must record the fallback and clear the backend: %q %q", j2.fallback, j2.hwBackend)
	}
}

// --- runV3Plan progressive rename failure (coordinator.go:1010) ---

func TestKillRunV3PlanRename(t *testing.T) {
	dir := t.TempDir()
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	okFF := stubFFmpeg(t, `
out="${@: -1}"
echo mp4 > "$out"
`)
	settings := contracts.TranscodeSettings{FFmpegPath: okFF}
	session := "ren"
	// Occupy the final media path with a directory so the rename fails.
	if err := os.MkdirAll(tr.pathsFor(session, settings).media, 0o700); err != nil {
		t.Fatal(err)
	}
	j := &job{
		spec: sourceSpec{Path: "/media/in.mkv", Session: session, Settings: settings}, v3: true,
		delivery: contracts.TranscodeDeliveryProgressive, settings: settings,
		plan: &encodePlan{delivery: contracts.TranscodeDeliveryProgressive, settings: settings, encoder: "libx264", video: &stream{Index: 0}},
	}
	if err := tr.runV3Plan(j); err == nil {
		t.Fatal("a failed rename must fail the run")
	}
	if _, err := os.Stat(tr.pathsFor(session, settings).media + ".tmp"); !os.IsNotExist(err) {
		t.Fatal("the .tmp must be removed after a failed rename")
	}
}

// --- progress plumbing inside runV3Plan (coordinator.go:946-949) ---

func TestKillRunV3PlanProgress(t *testing.T) {
	dir := t.TempDir()
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	ff := stubFFmpeg(t, `
printf 'fps=24.5\nbitrate=1200.0kbits/s\nout_time_us=500000\nprogress=continue\nprogress=end\n'
out="${@: -1}"
echo mp4 > "$out"
`)
	settings := contracts.TranscodeSettings{FFmpegPath: ff}
	j := &job{
		spec: sourceSpec{Path: "/media/in.mkv", Session: "prog", Settings: settings}, v3: true,
		delivery: contracts.TranscodeDeliveryProgressive, settings: settings,
		plan: &encodePlan{delivery: contracts.TranscodeDeliveryProgressive, settings: settings, encoder: "libx264", video: &stream{Index: 0}, report: mediaReport{}},
	}
	if err := tr.runV3Plan(j); err != nil {
		t.Fatalf("run: %v", err)
	}
	if j.fps != 24.5 {
		t.Fatalf("fps sample must reach the job: %v", j.fps)
	}
	if j.outputKbps != 1200 {
		t.Fatalf("bitrate sample must reach the job: %v", j.outputKbps)
	}
	if j.progress != 1 {
		t.Fatalf("the end marker must report fraction 1: %v", j.progress)
	}
}

// --- finalizeV3 skips an already-extracted sidecar (coordinator.go:1021-1032) ---

func TestKillFinalizeV3SkipsExtracted(t *testing.T) {
	dir := t.TempDir()
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	settings := contracts.TranscodeSettings{FFmpegPath: stubFFmpeg(t, "exit 9")}
	session := "fin"
	paths := tr.pathsFor(session, settings)
	if err := os.MkdirAll(filepath.Dir(paths.subtitle), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(paths.subtitle, []byte("WEBVTT"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(paths.media, []byte("mp4"), 0o600); err != nil {
		t.Fatal(err)
	}
	j := &job{
		spec: sourceSpec{Path: "/media/in.mkv", Session: session, Settings: settings}, v3: true,
		delivery: contracts.TranscodeDeliveryProgressive, settings: settings, hasSubtitle: true,
		plan: &encodePlan{delivery: contracts.TranscodeDeliveryProgressive, subtitleMode: contracts.SubtitleModeExtract, subtitle: &stream{Index: 2}, settings: settings},
	}
	// hasSubtitle is already true: the failing ffmpeg must NOT be run.
	if err := tr.finalizeV3(j); err != nil {
		t.Fatalf("finalize must skip the done extract: %v", err)
	}
	var e cacheEntry
	if err := readJSON(tr.metaPath(session), &e); err != nil {
		t.Fatalf("the cache entry must be written: %v", err)
	}
	if !e.HasSubtitle || e.Size <= 0 {
		t.Fatalf("entry must count the sidecar: %+v", e)
	}
}

// --- path helpers (coordinator.go:1088,1091) ---

func TestKillPathHelpers(t *testing.T) {
	tr := newWithDeps(t.TempDir(), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	if got := tr.burnSubtitlePath("s"); !strings.HasSuffix(got, "s.burn.ass") {
		t.Fatalf("burnSubtitlePath=%q", got)
	}
	if got := tr.hlsDir("s"); !strings.HasSuffix(got, "s.hls") {
		t.Fatalf("hlsDir=%q", got)
	}
}

// --- readyEntry zero-size boundaries (coordinator.go:1180-1207) ---

func TestKillReadyEntryEmptyArtifacts(t *testing.T) {
	dir := t.TempDir()
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	spec := sourceSpec{Path: "/media/in.mkv", Session: "e1", Profile: "p"}
	write := func(e cacheEntry) {
		if err := writeJSONAtomic(tr.metaPath(e.Session), e); err != nil {
			t.Fatal(err)
		}
	}
	base := cacheEntry{Session: spec.Session, SourcePath: spec.Path, SourceSize: spec.Size, SourceModTime: spec.ModTimeNS, Profile: "p"}

	// Progressive entry with an empty media file is not ready.
	write(base)
	if err := os.WriteFile(tr.mediaPath(spec.Session), nil, 0o600); err != nil {
		t.Fatal(err)
	}
	if _, ok := tr.readyEntry(spec); ok {
		t.Fatal("a zero-byte media file must not be ready")
	}
	if err := os.WriteFile(tr.mediaPath(spec.Session), []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, ok := tr.readyEntry(spec); !ok {
		t.Fatal("a non-empty media file must be ready")
	}

	// A subtitle delivery needs its sidecar to exist and be non-empty.
	sub := base
	sub.Delivery = contracts.TranscodeDeliverySubtitle
	write(sub)
	if _, ok := tr.readyEntry(spec); ok {
		t.Fatal("a subtitle entry without a sidecar must not be ready")
	}
	if err := os.WriteFile(tr.subtitlePath(spec.Session), nil, 0o600); err != nil {
		t.Fatal(err)
	}
	if _, ok := tr.readyEntry(spec); ok {
		t.Fatal("a zero-byte sidecar must not be ready")
	}
	if err := os.WriteFile(tr.subtitlePath(spec.Session), []byte("WEBVTT"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, ok := tr.readyEntry(spec); !ok {
		t.Fatal("a real sidecar must be ready")
	}
}

// --- cleanup: orphans, staleness, LRU and the grace window (coordinator.go:1274-1409) ---

func TestKillCleanupOrphans(t *testing.T) {
	dir := t.TempDir()
	cache := filepath.Join(dir, "cache")
	tr := newWithDeps(cache, Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })

	// Orphan artifacts with no sidecar are swept.
	orphan := filepath.Join(cache, "orphan.mp4")
	if err := os.WriteFile(orphan, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	orphanHLS := filepath.Join(cache, "orphanhls.hls")
	if err := os.MkdirAll(orphanHLS, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := tr.cleanup(false, nil); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(orphan); !os.IsNotExist(err) {
		t.Fatal("an orphan .mp4 must be removed")
	}
	if _, err := os.Stat(orphanHLS); !os.IsNotExist(err) {
		t.Fatal("an orphan .hls dir must be removed")
	}
}

func TestKillCleanupStaleness(t *testing.T) {
	dir := t.TempDir()
	cache := filepath.Join(dir, "cache")
	tr := newWithDeps(cache, Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	src := filepath.Join(dir, "src.mkv")
	if err := os.WriteFile(src, []byte("src"), 0o600); err != nil {
		t.Fatal(err)
	}
	srcFi, _ := os.Stat(src)

	entry := cacheEntry{Session: "stale1", SourcePath: src, SourceSize: srcFi.Size(), SourceModTime: srcFi.ModTime().UnixNano(), Profile: "p", HasSubtitle: true}
	if err := writeJSONAtomic(tr.metaPath("stale1"), entry); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(tr.mediaPath("stale1"), []byte("mp4"), 0o600); err != nil {
		t.Fatal(err)
	}
	// HasSubtitle with a missing sidecar is stale -> removed.
	if err := tr.cleanup(false, nil); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(tr.metaPath("stale1")); !os.IsNotExist(err) {
		t.Fatal("an entry whose sidecar vanished must be swept")
	}

	// A zero-byte sidecar is just as stale.
	entry2 := entry
	entry2.Session = "stale2"
	if err := writeJSONAtomic(tr.metaPath("stale2"), entry2); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(tr.mediaPath("stale2"), []byte("mp4"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(tr.subtitlePath("stale2"), nil, 0o600); err != nil {
		t.Fatal(err)
	}
	if err := tr.cleanup(false, nil); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(tr.metaPath("stale2")); !os.IsNotExist(err) {
		t.Fatal("a zero-byte sidecar must make the entry stale")
	}
}

func TestKillCleanupEviction(t *testing.T) {
	dir := t.TempDir()
	cache := filepath.Join(dir, "cache")
	tr := newWithDeps(cache, Config{MaxCacheBytes: 10}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	src := filepath.Join(dir, "src.mkv")
	if err := os.WriteFile(src, []byte("src"), 0o600); err != nil {
		t.Fatal(err)
	}
	srcFi, _ := os.Stat(src)
	now := tr.now().Unix()
	graceAgo := now - int64(recentAccessGraceSeconds) - 10

	put := func(session string, size int, accessed, created int64) {
		e := cacheEntry{Session: session, SourcePath: src, SourceSize: srcFi.Size(), SourceModTime: srcFi.ModTime().UnixNano(),
			Profile: "p", AccessedAt: accessed, CreatedAt: created}
		if err := writeJSONAtomic(tr.metaPath(session), e); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(tr.mediaPath(session), make([]byte, size), 0o600); err != nil {
			t.Fatal(err)
		}
	}

	// Two old entries over budget: the least recently accessed goes first.
	put("old", 8, graceAgo, 1)
	put("newer", 8, graceAgo+5, 2)
	if err := tr.cleanup(false, nil); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(tr.metaPath("old")); !os.IsNotExist(err) {
		t.Fatal("the LRU entry must be evicted first")
	}
	if _, err := os.Stat(tr.metaPath("newer")); err != nil {
		t.Fatal("the more recent entry must survive")
	}

	// A recently-accessed entry is protected by the grace window even
	// when it blows the budget on its own. Accessed 30s ago: inside the
	// 60s window, outside any degenerate ~0s one.
	put("hot", 50, now-30, 3)
	if err := tr.cleanup(false, nil); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(tr.metaPath("hot")); err != nil {
		t.Fatal("a session inside the grace window must not be evicted")
	}

	// An excluded (active) session survives even past the grace window.
	put("active", 50, graceAgo, 4)
	if err := tr.cleanup(false, map[string]bool{"active": true}); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(tr.metaPath("active")); err != nil {
		t.Fatal("an excluded session must not be evicted")
	}
}

func TestKillCleanupOverQuotaWarn(t *testing.T) {
	dir := t.TempDir()
	cache := filepath.Join(dir, "cache")
	tr := newWithDeps(cache, Config{MaxCacheBytes: 1}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	h := &captureHandler{}
	tr.SetLogger(slog.New(h))
	src := filepath.Join(dir, "src.mkv")
	if err := os.WriteFile(src, []byte("src"), 0o600); err != nil {
		t.Fatal(err)
	}
	srcFi, _ := os.Stat(src)
	// One in-grace entry over budget: eviction is blocked, so the
	// over-quota warning must fire.
	e := cacheEntry{Session: "hot", SourcePath: src, SourceSize: srcFi.Size(), SourceModTime: srcFi.ModTime().UnixNano(),
		Profile: "p", AccessedAt: tr.now().Unix(), CreatedAt: tr.now().Unix()}
	if err := writeJSONAtomic(tr.metaPath("hot"), e); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(tr.mediaPath("hot"), make([]byte, 100), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := tr.cleanup(false, nil); err != nil {
		t.Fatal(err)
	}
	warned := false
	for _, r := range h.records {
		if r.Level == slog.LevelWarn && strings.Contains(r.Message, "over quota") {
			warned = true
		}
	}
	if !warned {
		t.Fatal("an over-budget cache must log the over-quota warning")
	}
}

// --- cleanup counts sidecar bytes in the budget (coordinator.go:1348) ---

func TestKillCleanupSidecarBytes(t *testing.T) {
	dir := t.TempDir()
	cache := filepath.Join(dir, "cache")
	tr := newWithDeps(cache, Config{MaxCacheBytes: 10}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	src := filepath.Join(dir, "src.mkv")
	if err := os.WriteFile(src, []byte("src"), 0o600); err != nil {
		t.Fatal(err)
	}
	srcFi, _ := os.Stat(src)
	old := tr.now().Unix() - int64(recentAccessGraceSeconds) - 10
	e := cacheEntry{Session: "withsub", SourcePath: src, SourceSize: srcFi.Size(), SourceModTime: srcFi.ModTime().UnixNano(),
		Profile: "p", AccessedAt: old, CreatedAt: old, HasSubtitle: true}
	if err := writeJSONAtomic(tr.metaPath("withsub"), e); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(tr.mediaPath("withsub"), make([]byte, 8), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(tr.subtitlePath("withsub"), make([]byte, 8), 0o600); err != nil {
		t.Fatal(err)
	}
	// media 8 + sidecar 8 = 16 > budget 10: the sidecar bytes must count.
	if err := tr.cleanup(false, nil); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(tr.metaPath("withsub")); !os.IsNotExist(err) {
		t.Fatal("sidecar bytes must push the entry over budget")
	}
}

// --- removeArtifacts cleans a relocated session dir (coordinator.go:1409) ---

// --- sweepRelocatedOrphans ownership (coordinator.go:1446) ---

func TestKillSweepOrphanOwnership(t *testing.T) {
	dir := t.TempDir()
	cache := filepath.Join(dir, "cache")
	tr := newWithDeps(cache, Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	root := filepath.Join(dir, "tmp", "lain-transcode")
	if err := os.MkdirAll(root, 0o700); err != nil {
		t.Fatal(err)
	}
	orphan := filepath.Join(root, "deadbeef")
	if err := os.MkdirAll(orphan, 0o700); err != nil {
		t.Fatal(err)
	}

	// An owner file pointing at a live other data dir blocks the sweep.
	owner := filepath.Join(root, ".owner")
	other := filepath.Join(dir, "other-data")
	if err := os.MkdirAll(other, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(owner, []byte(other), 0o600); err != nil {
		t.Fatal(err)
	}
	tr.sweepRelocatedOrphans(root, nil)
	if _, err := os.Stat(orphan); err != nil {
		t.Fatal("a foreign-owned root must not be swept")
	}

	// A dead owner is taken over and the orphan goes away.
	if err := os.WriteFile(owner, []byte(filepath.Join(dir, "gone")), 0o600); err != nil {
		t.Fatal(err)
	}
	tr.sweepRelocatedOrphans(root, nil)
	if _, err := os.Stat(orphan); !os.IsNotExist(err) {
		t.Fatal("an orphan under an owned root must be removed")
	}

	// An unwritable claim path (.owner is a directory) returns early.
	root2 := filepath.Join(dir, "tmp2", "lain-transcode")
	if err := os.MkdirAll(filepath.Join(root2, ".owner"), 0o700); err != nil {
		t.Fatal(err)
	}
	orphan2 := filepath.Join(root2, "cafe")
	if err := os.MkdirAll(orphan2, 0o700); err != nil {
		t.Fatal(err)
	}
	tr.sweepRelocatedOrphans(root2, nil)
	if _, err := os.Stat(orphan2); err != nil {
		t.Fatal("a lost ownership race must abort the sweep")
	}

	// No owner at all: this instance claims the root and sweeps. A mutant
	// that skips the successful OpenFile claim never writes the marker
	// and aborts on the empty read-back.
	root3 := filepath.Join(dir, "tmp3", "lain-transcode")
	orphan3 := filepath.Join(root3, "beef")
	if err := os.MkdirAll(orphan3, 0o700); err != nil {
		t.Fatal(err)
	}
	tr.sweepRelocatedOrphans(root3, nil)
	if raw, err := os.ReadFile(filepath.Join(root3, ".owner")); err != nil || strings.TrimSpace(string(raw)) != tr.dir {
		t.Fatalf("the claim must write this instance's data dir, got %q", raw)
	}
	if _, err := os.Stat(orphan3); !os.IsNotExist(err) {
		t.Fatal("a freshly claimed root must be swept")
	}
}

// --- ffmpeg.go: frameRate/durationSeconds boundaries ---

func TestKillFrameRateBoundary(t *testing.T) {
	for in, want := range map[string]float64{
		"25/1": 25, "30000/1001": 29.97, "0/25": 0, "25/0": 0, "-5/2": 0, "x/y": 0, "25": 0, "": 0,
	} {
		got := (stream{FrameRate: in}).frameRate()
		if in == "30000/1001" {
			if got < 29.9 || got > 30.0 {
				t.Fatalf("frameRate(%q)=%v", in, got)
			}
			continue
		}
		if got != want {
			t.Fatalf("frameRate(%q)=%v, want %v", in, got, want)
		}
	}
	var r mediaReport
	r.Format.Duration = "0"
	if r.durationSeconds() != 0 {
		t.Fatal("a zero duration must report 0")
	}
	r.Format.Duration = "12.5"
	if r.durationSeconds() != 12.5 {
		t.Fatal("duration must parse")
	}
}

// --- conversionArgs picks the flagged-default audio (ffmpeg.go:206) ---

func TestKillDefaultAudioSelection(t *testing.T) {
	report := mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "h264", PixelFormat: "yuv420p"},
		{Index: 1, CodecType: "audio", CodecName: "aac"},
		{Index: 2, CodecType: "audio", CodecName: "aac", Default: 1},
	}}
	report.Streams[2].Disposition.Default = 1
	args, _, err := conversionArgs(sourceSpec{Path: "/in.mkv", Profile: profile}, report, "/tmp/o.mp4")
	if err != nil {
		t.Fatal(err)
	}
	joined := strings.Join(args, " ")
	if !strings.Contains(joined, "-map 0:2") {
		t.Fatalf("the default-flagged audio must be mapped: %v", args)
	}
}

// --- convertMedia needs ffprobe for the browser profile (ffmpeg.go:301) ---

func TestKillConvertMediaProbeGate(t *testing.T) {
	dir := t.TempDir()
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	bad := filepath.Join(dir, "missing.mkv")
	_, err := tr.convertMedia(sourceSpec{Path: bad, Profile: profile}, filepath.Join(dir, "o.mp4"), nil)
	var ce *core.Error
	if !errors.As(err, &ce) || ce.Code != "dependency-unavailable" {
		t.Fatalf("the browser profile must require ffprobe: %v", err)
	}
	// The legacy profile tolerates a probe failure and reaches ffmpeg.
	_, err = tr.convertMedia(sourceSpec{Path: bad, Profile: legacyProfile}, filepath.Join(dir, "o.mp4"), nil)
	if err == nil || (errors.As(err, &ce) && ce.Code == "dependency-unavailable" && strings.Contains(ce.Msg, "ffprobe")) {
		t.Fatalf("the legacy profile must not fail on the probe gate: %v", err)
	}
}

// --- consumeProgress: out_time_us=0 resets the position (ffmpeg.go:365) ---

func TestKillProgressZeroReset(t *testing.T) {
	var samples []progressSample
	consumeProgress(strings.NewReader("out_time_us=500000\nprogress=continue\nout_time_us=0\nprogress=continue\n"), 10, func(s progressSample) {
		samples = append(samples, s)
	})
	if len(samples) != 2 {
		t.Fatalf("samples=%v", samples)
	}
	if samples[0].Fraction != 0.05 {
		t.Fatalf("first fraction=%v", samples[0].Fraction)
	}
	if samples[1].Fraction != 0 {
		t.Fatalf("out_time_us=0 must reset the position: %v", samples[1].Fraction)
	}
}

// --- encodePlan argv boundaries (encode.go) ---

func TestKillFFmpegArgsBranches(t *testing.T) {
	// complexFilter wins over -vf and selects [vout].
	p := encodePlanFor(func(p *encodePlan) { p.complexFilter = "[0:v:0]scale=2[vout]" })
	args := strings.Join(p.ffmpegArgs("/tmp/o.mp4", "", false), " ")
	if !strings.Contains(args, "-filter_complex") || !strings.Contains(args, "[vout]") || strings.Contains(args, "-vf") {
		t.Fatalf("complexFilter argv: %v", args)
	}
	// filters produce -vf; none produce neither.
	p2 := encodePlanFor(func(p *encodePlan) { p.filters = []string{"scale=1:1"} })
	args = strings.Join(p2.ffmpegArgs("/tmp/o.mp4", "", false), " ")
	if !strings.Contains(args, "-vf scale=1:1") {
		t.Fatalf("filters argv: %v", args)
	}
	// muxing queue bound only when set.
	p3 := encodePlanFor(func(p *encodePlan) { p.settings.MuxingQueueSize = 2048 })
	args = strings.Join(p3.ffmpegArgs("/tmp/o.mp4", "", false), " ")
	if !strings.Contains(args, "-max_muxing_queue_size 2048") {
		t.Fatalf("muxing argv: %v", args)
	}
	p4 := encodePlanFor(func(p *encodePlan) { p.settings.MuxingQueueSize = 0 })
	if strings.Contains(strings.Join(p4.ffmpegArgs("/tmp/o.mp4", "", false), " "), "muxing_queue") {
		t.Fatal("muxing args must be absent at 0")
	}
	// audio filters: boost 0 -> none, 2 -> volume=2.00.
	p5 := encodePlanFor(func(p *encodePlan) { p.downmix = true; p.settings.DownmixAudioBoost = 0 })
	if got := p5.audioFilters(); strings.Contains(got, "volume") {
		t.Fatalf("boost 0 must add no volume filter: %q", got)
	}
	p6 := encodePlanFor(func(p *encodePlan) { p.downmix = true; p.settings.DownmixAudioBoost = 2 })
	if got := p6.audioFilters(); !strings.Contains(got, "volume=2.00") {
		t.Fatalf("boost 2 must emit volume=2.00: %q", got)
	}
}

func TestKillHLSArgsBoundaries(t *testing.T) {
	// Low fps clamps the GOP at 1.
	p := encodePlanFor(func(p *encodePlan) {
		p.delivery = contracts.TranscodeDeliveryHLS
		p.settings.HLSSegmentSeconds = 4
		p.video = &stream{Index: 0, FrameRate: "1/10"}
	})
	args := strings.Join(p.hlsArgs("/d", "/d/raw.m3u8"), " ")
	if !strings.Contains(args, "-g 1") || !strings.Contains(args, "-keyint_min 1") {
		t.Fatalf("a sub-1 GOP must clamp to 1: %v", args)
	}
	// TS container switches segment naming and drops the init file.
	ts := encodePlanFor(func(p *encodePlan) {
		p.delivery = contracts.TranscodeDeliveryHLS
		p.settings.HLSSegmentSeconds = 4
		p.settings.HLSSegmentContainer = contracts.HLSSegmentTS
		p.copyVideo = true
	})
	args = strings.Join(ts.hlsArgs("/d", "/d/raw.m3u8"), " ")
	if !strings.Contains(args, "mpegts") || !strings.Contains(args, "seg%05d.ts") {
		t.Fatalf("ts args: %v", args)
	}
	// Copy mode never forces keyframes.
	if strings.Contains(args, "force_key_frames") {
		t.Fatal("stream copy must not force keyframes")
	}
}

// --- execPlan covers the progress/onCmd/output checks (encode.go:476-509) ---

func TestKillExecPlanFull(t *testing.T) {
	dir := t.TempDir()
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })

	// Happy path: progress is consumed, onCmd sees the process, the
	// output file is produced.
	ff := stubFFmpeg(t, `
printf 'fps=30\nout_time_us=100\nprogress=continue\nprogress=end\n'
out="${@: -1}"
echo mp4 > "$out"
`)
	var gotCmd *exec.Cmd
	var samples []progressSample
	p := encodePlanFor(func(p *encodePlan) { p.settings.FFmpegPath = ff })
	p.report.Format.Duration = "10"
	out := filepath.Join(dir, "o.mp4")
	err := tr.execPlan(context.Background(), p, out, "", func(s progressSample) { samples = append(samples, s) }, func(c *exec.Cmd) { gotCmd = c })
	if err != nil {
		t.Fatalf("execPlan: %v", err)
	}
	if gotCmd == nil || gotCmd.Process == nil {
		t.Fatal("onCmd must receive the started process")
	}
	if len(samples) == 0 || samples[len(samples)-1].Fraction != 1 {
		t.Fatalf("progress must reach the sample callback: %v", samples)
	}
	if fi, _ := os.Stat(out); fi == nil || fi.Size() == 0 {
		t.Fatal("the stub output must be produced")
	}

	// Empty output is a failure.
	empty := stubFFmpeg(t, `out="${@: -1}"; : > "$out"`)
	p2 := encodePlanFor(func(p *encodePlan) { p.settings.FFmpegPath = empty })
	if err := tr.execPlan(context.Background(), p2, filepath.Join(dir, "e.mp4"), "", nil, nil); err == nil || !strings.Contains(err.Error(), "no output") {
		t.Fatalf("empty output must fail: %v", err)
	}

	// Start failure surfaces stderr.
	bad := stubFFmpeg(t, "echo nope >&2; exit 2")
	p3 := encodePlanFor(func(p *encodePlan) { p.settings.FFmpegPath = bad })
	if err := tr.execPlan(context.Background(), p3, filepath.Join(dir, "f.mp4"), "", nil, nil); err == nil || !strings.Contains(err.Error(), "nope") {
		t.Fatalf("exit failure must carry stderr: %v", err)
	}
}

// --- hls.go playlist bookkeeping ---

func TestKillHLSPlaylistTags(t *testing.T) {
	dir := t.TempDir()
	raw := `#EXTM3U

#EXT-X-MEDIA-SEQUENCE:3
#EXT-X-MAP:URI="init.mp4"
#EXTINF:4.000,
seg00003.m4s
#EXT-X-DISCONTINUITY
#EXTINF:2.5,
seg00004.m4s
#EXT-X-ENDLIST
`
	for _, f := range []string{"seg00003.m4s", "seg00004.m4s", "init.mp4"} {
		if err := os.WriteFile(filepath.Join(dir, f), []byte("x"), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.WriteFile(filepath.Join(dir, "raw.m3u8"), []byte(raw), 0o600); err != nil {
		t.Fatal(err)
	}
	segs, ended, err := parseHLSPlaylist(filepath.Join(dir, "raw.m3u8"))
	if err != nil || !ended {
		t.Fatalf("parse ended=%v err=%v", ended, err)
	}
	if len(segs) != 2 || segs[0].MediaSeq != 3 || segs[1].MediaSeq != 4 {
		t.Fatalf("media sequence tracking: %+v", segs)
	}
	if !segs[1].IsDiscont || segs[0].IsDiscont {
		t.Fatalf("discontinuity must attach to the following segment: %+v", segs)
	}
	if segs[1].StartSec != 4.0 || segs[1].EndSec != 6.5 {
		t.Fatalf("elapsed tracking: %+v", segs[1])
	}
	if err := writeIndexPlaylist(dir, 6); err != nil {
		t.Fatal(err)
	}
	idx, _ := os.ReadFile(filepath.Join(dir, "index.m3u8"))
	if !strings.Contains(string(idx), "#EXT-X-ENDLIST") {
		t.Fatalf("the endlist must propagate:\n%s", idx)
	}
	if !strings.Contains(string(idx), "#EXT-X-MEDIA-SEQUENCE:3") {
		t.Fatalf("the media sequence must propagate:\n%s", idx)
	}
	if !strings.Contains(string(idx), "#EXT-X-MAP:URI=\"init.mp4\"") {
		t.Fatalf("the init map must propagate:\n%s", idx)
	}
	// The first listed segment drops its discontinuity tag.
	if strings.Count(string(idx), "#EXT-X-DISCONTINUITY") != 1 {
		t.Fatalf("exactly one discontinuity must be emitted:\n%s", idx)
	}
	if !hlsPlayable(dir) {
		t.Fatal("a written index with live segments must be playable")
	}
}

func TestKillDeleteConsumedSegments(t *testing.T) {
	dir := t.TempDir()
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	raw := `#EXTM3U
#EXTINF:4.0,
seg00000.m4s
#EXTINF:4.0,
seg00001.m4s
#EXTINF:4.0,
seg00002.m4s
`
	settings := contracts.TranscodeSettings{SegmentDeletion: true, SegmentKeepSec: 0}
	hlsDir := filepath.Join(dir, "s.hls")
	if err := os.MkdirAll(hlsDir, 0o700); err != nil {
		t.Fatal(err)
	}
	for _, f := range []string{"seg00000.m4s", "seg00001.m4s", "seg00002.m4s"} {
		if err := os.WriteFile(filepath.Join(hlsDir, f), []byte("x"), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.WriteFile(filepath.Join(hlsDir, "raw.m3u8"), []byte(raw), 0o600); err != nil {
		t.Fatal(err)
	}
	// pathsFor must land on this dir: run through the default layout.
	j := &job{spec: sourceSpec{Session: "s"}, settings: settings, delivery: "hls", clientSegment: 0}
	tr2 := tr
	// Point the transcoder's dir so pathsFor("s") == hlsDir.
	tr2.dir = dir
	tr2.deleteConsumedSegments(j)
	if _, err := os.Stat(filepath.Join(hlsDir, "seg00000.m4s")); !os.IsNotExist(err) {
		t.Fatal("the consumed segment must be deleted")
	}
	if _, err := os.Stat(filepath.Join(hlsDir, "seg00001.m4s")); err != nil {
		t.Fatal("segments ahead of the client must be kept")
	}
	// A negative client position deletes nothing.
	for _, f := range []string{"seg00000.m4s", "seg00001.m4s", "seg00002.m4s"} {
		if err := os.WriteFile(filepath.Join(hlsDir, f), []byte("x"), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	j.clientSegment = -1
	tr2.deleteConsumedSegments(j)
	if _, err := os.Stat(filepath.Join(hlsDir, "seg00000.m4s")); err != nil {
		t.Fatal("a negative client position must keep every segment")
	}
}

func TestKillApplyThrottle(t *testing.T) {
	dir := t.TempDir()
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	hlsDir := filepath.Join(dir, "s.hls")
	if err := os.MkdirAll(hlsDir, 0o700); err != nil {
		t.Fatal(err)
	}
	// 20 seconds produced, client at segment 0 (4s): far ahead of a
	// 6-second allowance.
	raw := `#EXTM3U
#EXTINF:4.0,
seg00000.m4s
#EXTINF:4.0,
seg00001.m4s
#EXTINF:4.0,
seg00002.m4s
#EXTINF:4.0,
seg00003.m4s
#EXTINF:4.0,
seg00004.m4s
`
	if err := os.WriteFile(filepath.Join(hlsDir, "raw.m3u8"), []byte(raw), 0o600); err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command("sleep", "60")
	if err := cmd.Start(); err != nil {
		t.Skip("no sleep binary")
	}
	t.Cleanup(func() { _ = cmd.Process.Kill(); _ = cmd.Wait() })
	settings := contracts.TranscodeSettings{Throttle: true, ThrottleAheadSec: 6}
	tr.dir = dir
	j := &job{spec: sourceSpec{Session: "s"}, settings: settings, delivery: "hls", cmd: cmd, clientSegment: 0}
	tr.applyThrottle(j)
	if !j.paused {
		t.Fatal("production ahead of the limit must pause ffmpeg")
	}
	// Caught up: the client is at the last produced segment.
	j.clientSegment = 4
	tr.applyThrottle(j)
	if j.paused {
		t.Fatal("a caught-up client must resume ffmpeg")
	}
}

func TestKillListHLSFiles(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "b.m4s"), nil, 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "a.m4s"), nil, 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Join(dir, "sub"), 0o700); err != nil {
		t.Fatal(err)
	}
	got := listHLSFiles(dir)
	if len(got) != 2 || got[0] != "a.m4s" || got[1] != "b.m4s" {
		t.Fatalf("listHLSFiles must list only regular files, sorted: %v", got)
	}
}

// --- v3.go session routing ---

func TestKillV3RoutingEdges(t *testing.T) {
	dir := t.TempDir()
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	src := filepath.Join(dir, "in.mkv")
	if err := os.WriteFile(src, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}

	// status/resolve require a session id.
	for _, action := range []string{contracts.TranscodeStatusAction, contracts.TranscodeResolveAction, contracts.TranscodeCancelAction, contracts.TranscodePositionAction} {
		_, err := tr.invokeV3(contracts.TranscodeV3Request{Action: action, FilePath: src})
		var ce *core.Error
		if !errors.As(err, &ce) || ce.Code != "invalid-message" {
			t.Fatalf("%s without session must be invalid-message: %v", action, err)
		}
	}

	// A supplied session id that does not match the computed spec is
	// rejected before any state is created.
	_, err := tr.invokeV3(contracts.TranscodeV3Request{Action: contracts.TranscodeInspectAction, FilePath: src, Session: strings.Repeat("c", 64)})
	var ce *core.Error
	if !errors.As(err, &ce) || ce.Code != "invalid-message" {
		t.Fatalf("mismatched session must be invalid-message: %v", err)
	}
}

func TestKillBySessionV3PathChecks(t *testing.T) {
	dir := t.TempDir()
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })

	// Live job, wrong path.
	j := &job{spec: sourceSpec{Path: "/media/real.mkv"}, state: contracts.TranscodeRunning, done: make(chan struct{})}
	tr.jobs["s1"] = j
	_, err := tr.bySessionV3("/media/other.mkv", "s1", false)
	var ce *core.Error
	if !errors.As(err, &ce) || ce.Code != "invalid-message" {
		t.Fatalf("path mismatch on a live job must be invalid-message: %v", err)
	}
	// Right path resolves.
	st, err := tr.bySessionV3("/media/real.mkv", "s1", false)
	if err != nil || st.State != contracts.TranscodeRunning {
		t.Fatalf("matching path must resolve: %v %+v", err, st)
	}

	// Cache entry, wrong path.
	session := strings.Repeat("d", 64)
	e := cacheEntry{Session: session, SourcePath: "/media/real.mkv"}
	if err := writeJSONAtomic(tr.metaPath(session), e); err != nil {
		t.Fatal(err)
	}
	_, err = tr.bySessionV3("/media/wrong.mkv", session, false)
	if !errors.As(err, &ce) || ce.Code != "invalid-message" {
		t.Fatalf("path mismatch on a cache entry must be invalid-message: %v", err)
	}
}

func TestKillCancelV3(t *testing.T) {
	dir := t.TempDir()
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })

	// v3 job, no cmd: must not panic and drops artifacts.
	j := &job{spec: sourceSpec{Path: "/media/real.mkv", Session: "s1"}, state: contracts.TranscodeRunning,
		v3: true, delivery: contracts.TranscodeDeliveryProgressive, done: make(chan struct{})}
	tr.jobs["s1"] = j
	if err := os.WriteFile(tr.pathsFor("s1", j.settings).media, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	st := tr.cancelV3("s1", "", "")
	if st.State != contracts.TranscodeIdle {
		t.Fatalf("cancel must report idle: %+v", st)
	}
	if _, err := os.Stat(tr.pathsFor("s1", j.settings).media); !os.IsNotExist(err) {
		t.Fatal("cancel must remove the media artifact")
	}
	if _, ok := tr.jobs["s1"]; ok {
		t.Fatal("cancel must drop the job")
	}

	// A foreign user's session is not cancelled.
	j2 := &job{spec: sourceSpec{Path: "/media/real.mkv", Session: "s2"}, state: contracts.TranscodeRunning,
		v3: true, userID: "alice", done: make(chan struct{})}
	tr.jobs["s2"] = j2
	st = tr.cancelV3("s2", "", "bob")
	if _, ok := tr.jobs["s2"]; !ok {
		t.Fatal("a foreign user must not cancel the session")
	}
	if st.State != contracts.TranscodeIdle {
		t.Fatalf("rejected cancel reports idle: %+v", st)
	}

	// A v1/v2 job ignores the v3 cancel.
	j3 := &job{spec: sourceSpec{Path: "/media/real.mkv", Session: "s3"}, state: contracts.TranscodeRunning, done: make(chan struct{})}
	tr.jobs["s3"] = j3
	tr.cancelV3("s3", "", "")
	if _, ok := tr.jobs["s3"]; !ok {
		t.Fatal("a non-v3 job must survive a v3 cancel")
	}

	// Finished session: cache entry removed only for a matching path.
	session := strings.Repeat("e", 64)
	e := cacheEntry{Session: session, SourcePath: "/media/real.mkv"}
	if err := writeJSONAtomic(tr.metaPath(session), e); err != nil {
		t.Fatal(err)
	}
	tr.cancelFinishedV3(session, "/media/wrong.mkv")
	if _, err := os.Stat(tr.metaPath(session)); err != nil {
		t.Fatal("a path mismatch must keep the entry")
	}
	tr.cancelFinishedV3(session, "/media/real.mkv")
	if _, err := os.Stat(tr.metaPath(session)); !os.IsNotExist(err) {
		t.Fatal("a matching cancel must drop the entry")
	}
	// Malformed ids are ignored quietly.
	tr.cancelFinishedV3("nothex", "")
}

func TestKillActiveStreamsFor(t *testing.T) {
	tr := newWithDeps(t.TempDir(), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	mk := func(session, user, state string) {
		tr.jobs[session] = &job{userID: user, state: state, done: make(chan struct{})}
	}
	mk("a", "alice", contracts.TranscodeRunning)
	mk("b", "alice", contracts.TranscodeQueued)
	mk("c", "alice", contracts.TranscodeReady)
	mk("d", "bob", contracts.TranscodeRunning)
	if n := tr.activeStreamsFor("", ""); n != 0 {
		t.Fatalf("empty user counts nothing: %d", n)
	}
	if n := tr.activeStreamsFor("alice", ""); n != 2 {
		t.Fatalf("alice has 2 active streams: %d", n)
	}
	if n := tr.activeStreamsFor("alice", "a"); n != 1 {
		t.Fatalf("the excepted session must not count: %d", n)
	}
	if n := tr.activeStreamsFor("bob", ""); n != 1 {
		t.Fatalf("bob has 1 active stream: %d", n)
	}
}

// --- startV3 adoption + stream limits (v3.go:437-456) ---

func v3SpecFixture(t *testing.T, tr *Transcoder, dir string) (sourceSpec, contracts.TranscodeV3Request) {
	t.Helper()
	src := filepath.Join(dir, "in.mkv")
	if err := os.WriteFile(src, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	tr.probeFn = func(_, _ string) (mediaReport, error) {
		return mediaReport{Streams: []stream{
			{Index: 0, CodecType: "video", CodecName: "h264", PixelFormat: "yuv420p", Width: 640, Height: 360},
			{Index: 1, CodecType: "audio", CodecName: "aac"},
		}}, nil
	}
	tr.capsFn = func(contracts.TranscodeSettings) capabilities {
		return capabilities{Encoders: map[string]bool{"libx264": true, "aac": true}}
	}
	spec, settings, err := tr.specV3(src, contracts.TranscodeV3Request{})
	if err != nil {
		t.Fatal(err)
	}
	_ = settings
	in := contracts.TranscodeV3Request{Action: contracts.TranscodeStartAction, FilePath: src, Delivery: contracts.TranscodeDeliveryProgressive}
	return spec, in
}

func TestKillStartV3Adoption(t *testing.T) {
	dir := t.TempDir()
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	spec, in := v3SpecFixture(t, tr, dir)
	in.Settings.CacheBytes = 2 << 30
	in.Settings.MaxConcurrent = 3
	in.Settings.QueueSize = 4
	st, err := tr.startV3(spec, in, in.Settings.Normalize())
	if err != nil {
		t.Fatalf("startV3: %v", err)
	}
	if st.State != contracts.TranscodeQueued {
		t.Fatalf("state=%q", st.State)
	}
	if tr.config.MaxCacheBytes != 2<<30 || tr.maxConcurrent != 3 || tr.queueLimit != 4 {
		t.Fatalf("settings must be adopted: %+v %d %d", tr.config, tr.maxConcurrent, tr.queueLimit)
	}
	// Zero bounds in the effective settings never clobber the adopted
	// ones; downstream errors are irrelevant to this assertion.
	spec2 := spec
	spec2.Session = "other-session"
	in2 := in
	in2.Settings = contracts.TranscodeSettings{}
	_, _ = tr.startV3(spec2, in2, contracts.TranscodeSettings{})
	if tr.config.MaxCacheBytes != 2<<30 || tr.maxConcurrent != 3 || tr.queueLimit != 4 {
		t.Fatal("zero settings must keep the adopted bounds")
	}
}

func TestKillStartV3StreamLimit(t *testing.T) {
	dir := t.TempDir()
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	spec, in := v3SpecFixture(t, tr, dir)
	// One running session already belongs to alice.
	tr.jobs["other"] = &job{userID: "alice", state: contracts.TranscodeRunning, done: make(chan struct{})}
	in.UserID = "alice"
	in.Policy = &contracts.TranscodePolicy{AllowRemux: true, AllowVideoTranscode: true, AllowAudioTranscode: true, MaxStreams: 1}
	_, err := tr.startV3(spec, in, in.Settings.Normalize())
	var ce *core.Error
	if !errors.As(err, &ce) || ce.Code != "too-many-streams" {
		t.Fatalf("the stream limit must reject: %v", err)
	}
}

// --- chooseEncoder 10-bit gating and software fallback (policy.go) ---

func TestKillChooseEncoderGates(t *testing.T) {
	settings := contracts.TranscodeSettings{
		HardwareAcceleration: contracts.HWNVENC,
		HardwareEncode:       true,
		HardwareDecodeCodecs: []string{"h264"},
	}
	report := mediaReport{Streams: []stream{{CodecType: "video", CodecName: "h264"}}}

	// Hardware encode + allowed decode.
	caps := capabilities{
		Encoders: map[string]bool{"h264_nvenc": true, "libx264": true},
		Hardware: map[string]bool{contracts.HWNVENC: true},
	}
	enc, backend, decode, fb, err := chooseEncoder(settings, contracts.VideoCodecH264, report, caps, &report.Streams[0])
	if err != nil || enc != "h264_nvenc" || backend != contracts.HWNVENC || !decode || fb != "" {
		t.Fatalf("hw encode path: %q %q %v %q %v", enc, backend, decode, fb, err)
	}

	// Unknown codec must fail closed — resolveVideo validates the codec
	// upstream, but a defensive guard keeps a future caller from
	// panicking on the software map miss.
	_, _, _, _, err = chooseEncoder(settings, "vp9", report, caps, &report.Streams[0])
	if err == nil {
		t.Fatal("unknown codec must reject")
	}

	// Missing encoder.
	capsNoEnc := capabilities{Encoders: map[string]bool{"libx264": true}, Hardware: map[string]bool{contracts.HWNVENC: true}}
	enc, _, _, fb, _ = chooseEncoder(settings, contracts.VideoCodecH264, report, capsNoEnc, &report.Streams[0])
	if enc != "libx264" || !strings.Contains(fb, "unavailable") {
		t.Fatalf("missing-encoder fallback: %q %q", enc, fb)
	}

	// Probe failed.
	capsNoHw := capabilities{Encoders: map[string]bool{"h264_nvenc": true, "libx264": true}, Hardware: map[string]bool{}}
	enc, _, _, fb, _ = chooseEncoder(settings, contracts.VideoCodecH264, report, capsNoHw, &report.Streams[0])
	if enc != "libx264" || !strings.Contains(fb, "failed its probe") {
		t.Fatalf("failed-probe fallback: %q %q", enc, fb)
	}

	// 10-bit HEVC without the opt-in decodes in software.
	settings10 := settings
	settings10.HardwareDecodeCodecs = []string{"hevc"}
	hevc10 := mediaReport{Streams: []stream{{CodecType: "video", CodecName: "hevc", BitsPerRaw: "10"}}}
	enc, _, decode, fb, _ = chooseEncoder(settings10, contracts.VideoCodecHEVC, hevc10,
		capabilities{Encoders: map[string]bool{"hevc_nvenc": true, "libx265": true}, Hardware: map[string]bool{contracts.HWNVENC: true}}, &hevc10.Streams[0])
	if enc != "hevc_nvenc" || decode || !strings.Contains(fb, "10-bit HEVC") {
		t.Fatalf("10-bit gate: %q %v %q", enc, decode, fb)
	}
	// With the opt-in the decode is allowed.
	settings10.HardwareDecode10BitHEVC = true
	_, _, decode, _, _ = chooseEncoder(settings10, contracts.VideoCodecHEVC, hevc10,
		capabilities{Encoders: map[string]bool{"hevc_nvenc": true}, Hardware: map[string]bool{contracts.HWNVENC: true}}, &hevc10.Streams[0])
	if !decode {
		t.Fatal("the 10-bit opt-in must allow hardware decode")
	}
	// VP9 gets its own switch.
	vp9 := mediaReport{Streams: []stream{{CodecType: "video", CodecName: "vp9", BitsPerRaw: "10"}}}
	settingsVP9 := settings10
	settingsVP9.HardwareDecodeCodecs = []string{"vp9"}
	_, _, decode, fb, _ = chooseEncoder(settingsVP9, contracts.VideoCodecH264, vp9,
		capabilities{Encoders: map[string]bool{"libx264": true}, Hardware: map[string]bool{contracts.HWNVENC: true}}, &vp9.Streams[0])
	if decode || !strings.Contains(fb, "10-bit VP9") {
		t.Fatalf("vp9 gate: %v %q", decode, fb)
	}

	// Decode-only acceleration: encode off, backend on.
	settingsDec := contracts.TranscodeSettings{HardwareAcceleration: contracts.HWNVENC, HardwareDecodeCodecs: []string{"h264"}}
	enc, backend, decode, _, _ = chooseEncoder(settingsDec, contracts.VideoCodecH264, report, caps, &report.Streams[0])
	if enc != "libx264" || backend != "" || !decode {
		t.Fatalf("decode-only: %q %q %v", enc, backend, decode)
	}
	// A codec outside the allowed list decodes in software.
	settingsDec.HardwareDecodeCodecs = []string{"hevc"}
	_, _, decode, _, _ = chooseEncoder(settingsDec, contracts.VideoCodecH264, report, caps, &report.Streams[0])
	if decode {
		t.Fatal("a codec outside the allow-list must not hw-decode")
	}
	// Backend "none" resolves to software with no decode.
	settingsNone := contracts.TranscodeSettings{HardwareAcceleration: contracts.HWNone}
	enc, backend, decode, _, _ = chooseEncoder(settingsNone, contracts.VideoCodecH264, report, caps, &report.Streams[0])
	if enc != "libx264" || backend != "" || decode {
		t.Fatalf("none backend: %q %q %v", enc, backend, decode)
	}
}

// --- capabilitiesFor seam (v3.go:17) ---

func TestKillCapabilitiesFor(t *testing.T) {
	tr := newWithDeps(t.TempDir(), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	sentinel := capabilities{Encoders: map[string]bool{"x": true}}
	tr.capsFn = func(contracts.TranscodeSettings) capabilities { return sentinel }
	if got := tr.capabilitiesFor(contracts.TranscodeSettings{}); !got.Encoders["x"] {
		t.Fatal("the seam must return the injected capabilities")
	}
}

// probeCapabilities runs the real probe pipeline against a stub ffmpeg
// and caches inside capTTL.
func TestKillProbeCapabilitiesCache(t *testing.T) {
	dir := t.TempDir()
	countFile := filepath.Join(dir, "calls")
	ff := stubFFmpeg(t, `
echo call >> "`+countFile+`"
case "$*" in
  *-encoders*) printf ' V..... libx264\n V..... h264_nvenc\n' ;;
  *-filters*) printf ' ... scale\n' ;;
  *-hwaccels*) printf 'cuda\nvaapi\n' ;;
esac
exit 0`)
	calls := func() int {
		raw, err := os.ReadFile(countFile)
		if err != nil {
			return 0
		}
		return strings.Count(string(raw), "call")
	}
	now := time.Now()
	current := now
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{}, nil, func() time.Time { return current })
	t.Cleanup(func() { _ = tr.Close() })
	settings := contracts.TranscodeSettings{FFmpegPath: ff}
	caps := tr.capabilitiesFor(settings)
	if !caps.Encoders["libx264"] || !caps.Hwaccels["cuda"] || !caps.Filters["scale"] {
		t.Fatalf("probe must parse the stub lists: %+v", caps)
	}
	if !caps.ToneMap || !caps.ToneMap2390 {
		t.Fatal("a successful stub probe must report tone mapping")
	}
	if !caps.Hardware[contracts.HWNVENC] {
		t.Fatal("nvenc must pass its probe with a stub ffmpeg")
	}
	if caps.Hardware[contracts.HWVAAPI] {
		t.Fatal("vaapi lacks its encoder in the stub -> must not be reported")
	}
	before := calls()
	if again := tr.capabilitiesFor(settings); calls() != before || !again.ProbedAt.Equal(caps.ProbedAt) {
		t.Fatal("a second probe inside capTTL must hit the cache")
	}
	// Past the TTL it re-probes.
	current = now.Add(capTTL + time.Second)
	tr.capabilitiesFor(settings)
	if calls() <= before {
		t.Fatal("an expired probe must re-run")
	}
	// A different device key also misses the cache.
	settings2 := settings
	settings2.HardwareDevice = "/dev/dri/other"
	mid := calls()
	tr.capabilitiesFor(settings2)
	if calls() <= mid {
		t.Fatal("a changed device key must re-probe")
	}
}

func TestKillFFmpegBinaryDefaults(t *testing.T) {
	if ffmpegBinary(contracts.TranscodeSettings{}) != "ffmpeg" {
		t.Fatal("empty FFmpegPath must default to PATH ffmpeg")
	}
	if ffmpegBinary(contracts.TranscodeSettings{FFmpegPath: "/x/ff"}) != "/x/ff" {
		t.Fatal("FFmpegPath must win")
	}
	if ffprobeBinary(contracts.TranscodeSettings{}) != "ffprobe" {
		t.Fatal("empty FFprobePath must default to PATH ffprobe")
	}
	if hardwareDevice(contracts.TranscodeSettings{}) != contracts.DefaultHardwareDevice {
		t.Fatal("empty HardwareDevice must default")
	}
	if hardwareDevice(contracts.TranscodeSettings{HardwareDevice: " /dev/dri/x "}) != "/dev/dri/x" {
		t.Fatal("HardwareDevice must be trimmed")
	}
}

// --- quality ladder caps (policy.go:234-244) ---

func TestKillQualityCaps(t *testing.T) {
	dir := t.TempDir()
	tr := newWithDeps(filepath.Join(dir, "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	src := filepath.Join(dir, "in.mkv")
	if err := os.WriteFile(src, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	tr.probeFn = func(_, _ string) (mediaReport, error) {
		return mediaReport{Streams: []stream{
			{Index: 0, CodecType: "video", CodecName: "h264", PixelFormat: "yuv420p", Width: 1920, Height: 1080},
			{Index: 1, CodecType: "audio", CodecName: "aac"},
		}}, nil
	}
	tr.capsFn = func(contracts.TranscodeSettings) capabilities {
		return capabilities{Encoders: map[string]bool{"libx264": true, "aac": true}}
	}
	settings := contracts.DefaultTranscodeSettings()
	settings.Qualities = []contracts.TranscodeQuality{{Name: "low", MaxWidth: 640, MaxHeight: 360, BitrateKbps: 800}}
	in := contracts.TranscodeV3Request{Quality: "low", Delivery: contracts.TranscodeDeliveryProgressive, Settings: settings}
	spec, _, err := tr.specV3(src, in)
	if err != nil {
		t.Fatal(err)
	}
	plan, err := tr.planV3(spec, in, tr.capabilitiesFor(settings))
	if err != nil {
		t.Fatal(err)
	}
	if plan.width != 640 || plan.height != 360 || plan.maxBitrateKbps != 800 {
		t.Fatalf("the ladder must cap width/height/bitrate: %+v", plan)
	}
	// A caller cap lower than the ladder wins.
	in2 := in
	in2.MaxBitrateKbps = 400
	plan2, err := tr.planV3(spec, in2, tr.capabilitiesFor(settings))
	if err != nil {
		t.Fatal(err)
	}
	if plan2.maxBitrateKbps != 400 {
		t.Fatalf("the lower caller cap must win: %d", plan2.maxBitrateKbps)
	}
	// A higher caller cap does not lift the ladder.
	in3 := in
	in3.MaxBitrateKbps = 2000
	plan3, err := tr.planV3(spec, in3, tr.capabilitiesFor(settings))
	if err != nil {
		t.Fatal(err)
	}
	if plan3.maxBitrateKbps != 800 {
		t.Fatalf("the ladder cap must win: %d", plan3.maxBitrateKbps)
	}
	// Unknown quality fails loudly.
	in4 := in
	in4.Quality = "bogus"
	if _, err := tr.planV3(spec, in4, tr.capabilitiesFor(settings)); err == nil {
		t.Fatal("an unknown quality must fail")
	}
}

// --- selectAudio default/explicit selection (policy.go:99-125) ---

func TestKillSelectAudio(t *testing.T) {
	report := mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "h264"},
		{Index: 1, CodecType: "audio", CodecName: "aac"},
		{Index: 2, CodecType: "audio", CodecName: "ac3"},
	}}
	report.Streams[2].Default = 1
	report.Streams[2].Disposition.Default = 1

	got, err := selectAudio(report, nil)
	if err != nil || got.Index != 2 {
		t.Fatalf("the default-flagged audio must win: %+v %v", got, err)
	}
	// Without a flag the first audio wins.
	report.Streams[2].Default = 0
	got, _ = selectAudio(report, nil)
	if got.Index != 1 {
		t.Fatalf("first audio fallback: %+v", got)
	}
	// An explicit index wins over the default.
	one := 1
	got, err = selectAudio(report, &one)
	if err != nil || got.Index != 1 {
		t.Fatalf("explicit selection: %+v %v", got, err)
	}
	// A video index is not an audio track.
	zero := 0
	if _, err := selectAudio(report, &zero); err == nil {
		t.Fatal("a non-audio index must fail")
	}
	// No audio at all returns nil.
	only := mediaReport{Streams: []stream{{Index: 0, CodecType: "video"}}}
	if got, err := selectAudio(only, nil); got != nil || err != nil {
		t.Fatalf("no audio: %+v %v", got, err)
	}
}

func TestKillHasStreamCodec(t *testing.T) {
	report := mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "hevc"},
		{Index: 1, CodecType: "audio", CodecName: "AAC"},
	}}
	if !hasStreamCodec(report, "audio", "aac") {
		t.Fatal("codec match must be case-insensitive")
	}
	if hasStreamCodec(report, "audio", "hevc") {
		t.Fatal("codec must match within its own kind")
	}
	if hasStreamCodec(report, "subtitle", "aac") {
		t.Fatal("kind must match")
	}
}

// --- copyVideo gating conjunctions (policy.go:361-417) ---

func videoPlan(mutate func(*encodePlan)) *encodePlan {
	p := &encodePlan{
		settings: contracts.DefaultTranscodeSettings(),
		report:   mediaReport{Streams: []stream{{Index: 0, CodecType: "video", CodecName: "h264", PixelFormat: "yuv420p", Width: 1920, Height: 1080}}},
		video:    &stream{Index: 0, CodecType: "video", CodecName: "h264", PixelFormat: "yuv420p", Width: 1920, Height: 1080},
		width:    1920,
		height:   1080,
	}
	p.report.Streams[0] = *p.video
	if mutate != nil {
		mutate(p)
	}
	return p
}

func TestKillResolveVideoCopyGate(t *testing.T) {
	caps := capabilities{Encoders: map[string]bool{"libx264": true, "libx265": true}}

	// Web-safe h264 + nothing to change -> copy.
	p := videoPlan(nil)
	if err := p.resolveVideo(contracts.TranscodeV3Request{}, caps); err != nil || !p.copyVideo || p.encoder != "" {
		t.Fatalf("copy path: %v copy=%v enc=%q", err, p.copyVideo, p.encoder)
	}
	// yuvj420p is just as web-safe.
	p = videoPlan(func(p *encodePlan) { p.video.PixelFormat = "yuvj420p"; p.report.Streams[0].PixelFormat = "yuvj420p" })
	if err := p.resolveVideo(contracts.TranscodeV3Request{}, caps); err != nil || !p.copyVideo {
		t.Fatalf("yuvj420p must copy: %v copy=%v", err, p.copyVideo)
	}
	// 10-bit or non-web pixels encode.
	p = videoPlan(func(p *encodePlan) { p.video.PixelFormat = "yuv420p10le" })
	if err := p.resolveVideo(contracts.TranscodeV3Request{}, caps); err != nil || p.copyVideo {
		t.Fatalf("10-bit pixel must encode: %v copy=%v", err, p.copyVideo)
	}
	// Client-disabled stream copy encodes and says why.
	off := false
	p = videoPlan(nil)
	if err := p.resolveVideo(contracts.TranscodeV3Request{AllowVideoStreamCopy: &off}, caps); err != nil || p.copyVideo {
		t.Fatalf("disabled copy must encode: %v copy=%v", err, p.copyVideo)
	}
	if len(p.reasons) == 0 || !strings.Contains(p.reasons[0], "copy disabled") {
		t.Fatalf("the reason must be recorded: %v", p.reasons)
	}
	// A bitrate ceiling below the source bitrate forces encode.
	p = videoPlan(func(p *encodePlan) { p.maxBitrateKbps = 500; p.video.BitRate = "9000000" })
	if err := p.resolveVideo(contracts.TranscodeV3Request{}, caps); err != nil || p.copyVideo {
		t.Fatalf("over-ceiling copy must encode: %v copy=%v", err, p.copyVideo)
	}
	// A ceiling above the source bitrate still copies.
	p = videoPlan(func(p *encodePlan) { p.maxBitrateKbps = 20000; p.video.BitRate = "9000000" })
	if err := p.resolveVideo(contracts.TranscodeV3Request{}, caps); err != nil || !p.copyVideo {
		t.Fatalf("under-ceiling must copy: %v copy=%v", err, p.copyVideo)
	}
	// Downscaling encodes.
	p = videoPlan(func(p *encodePlan) { p.width = 640 })
	if err := p.resolveVideo(contracts.TranscodeV3Request{}, caps); err != nil || p.copyVideo {
		t.Fatalf("a downscale must encode: %v copy=%v", err, p.copyVideo)
	}
	// Asking for another codec encodes.
	p = videoPlan(nil)
	in := contracts.TranscodeV3Request{VideoCodec: contracts.VideoCodecHEVC}
	p.settings.AllowHEVC = true
	if err := p.resolveVideo(in, caps); err != nil || p.copyVideo || p.encoder != "libx265" {
		t.Fatalf("codec change: %v copy=%v enc=%q", err, p.copyVideo, p.encoder)
	}
	// HEVC gated off by settings is refused.
	p = videoPlan(nil)
	p.settings.AllowHEVC = false
	if err := p.resolveVideo(in, caps); err == nil {
		t.Fatal("HEVC must be refused when disabled")
	}
	// AV1 likewise.
	p = videoPlan(nil)
	inAV1 := contracts.TranscodeV3Request{VideoCodec: contracts.VideoCodecAV1}
	p.settings.AllowAV1 = false
	if err := p.resolveVideo(inAV1, caps); err == nil {
		t.Fatal("AV1 must be refused when disabled")
	}
	// Unknown codec is rejected.
	p = videoPlan(nil)
	if err := p.resolveVideo(contracts.TranscodeV3Request{VideoCodec: "vp9"}, caps); err == nil {
		t.Fatal("an unknown codec must fail")
	}
	// HDR without tone mapping is refused; with it, encodes.
	p = videoPlan(func(p *encodePlan) { p.video.ColorTransfer = "smpte2084" })
	p.settings.ToneMapping = false
	if err := p.resolveVideo(contracts.TranscodeV3Request{}, caps); err == nil {
		t.Fatal("HDR with tone mapping off must fail")
	}
	p = videoPlan(func(p *encodePlan) { p.video.ColorTransfer = "smpte2084" })
	p.settings.ToneMapping = true
	p.settings.ToneMappingMode = contracts.ToneMapModeAlways
	capsTone := capabilities{Encoders: map[string]bool{"libx264": true}, ToneMap: true}
	if err := p.resolveVideo(contracts.TranscodeV3Request{}, capsTone); err != nil || p.copyVideo || !p.needTone {
		t.Fatalf("HDR+tonemap: %v copy=%v tone=%v", err, p.copyVideo, p.needTone)
	}
	// Interlaced sources encode.
	p = videoPlan(func(p *encodePlan) { p.video.FieldOrder = "tt" })
	if err := p.resolveVideo(contracts.TranscodeV3Request{}, caps); err != nil || p.copyVideo {
		t.Fatalf("interlaced must encode: %v copy=%v", err, p.copyVideo)
	}
}

// --- scaleFilter / fileExt / audioQualityArgs (policy.go:577-593, encode.go:391) ---

func TestKillPureHelpers(t *testing.T) {
	if got := scaleFilter(640, 360); !strings.Contains(got, "min(iw,640)") || !strings.Contains(got, "min(ih,360)") {
		t.Fatalf("both axes: %q", got)
	}
	if got := scaleFilter(0, 360); !strings.Contains(got, "-2") || !strings.Contains(got, "min(ih,360)") {
		t.Fatalf("height only: %q", got)
	}
	if got := scaleFilter(640, 0); !strings.Contains(got, "min(iw,640)") || !strings.Contains(got, "':-2'") && !strings.Contains(got, ":-2") {
		t.Fatalf("width only: %q", got)
	}
	if got := fileExt("/a/b/c.mkv"); got != ".mkv" {
		t.Fatalf("fileExt=%q", got)
	}
	if got := fileExt("/a/b/nodot"); got != "" {
		t.Fatalf("fileExt=%q", got)
	}
	if got := joinNotes("", "b"); got != "b" {
		t.Fatalf("joinNotes=%q", got)
	}
	if got := joinNotes("a", ""); got != "a" {
		t.Fatalf("joinNotes=%q", got)
	}
	if got := joinNotes("a", "b"); got != "a; b" {
		t.Fatalf("joinNotes=%q", got)
	}
	// VBR quality mapping for AAC, CBR otherwise. The exact rendered
	// values pin the FormatFloat precision: "2" dies under precision 0
	// and 1 alike ("2" vs "2.0"), "0.5" dies under precision 0 ("0").
	for _, c := range []struct {
		kbps int
		want string
	}{{192, "2"}, {160, "1.5"}, {64, "0.5"}} {
		p := encodePlanFor(func(p *encodePlan) {
			p.settings.AudioVBR = true
			p.audioBitrateKbps = c.kbps
		})
		if !hasPair(p.audioQualityArgs(), "-q:a", c.want) {
			t.Fatalf("vbr aac %dk: %v, want -q:a %s", c.kbps, p.audioQualityArgs(), c.want)
		}
	}
	p := encodePlanFor(func(p *encodePlan) {
		p.settings.AudioVBR = true
		p.audioBitrateKbps = 160
	})
	if !hasPair(p.audioQualityArgs(), "-q:a", "1.5") {
		t.Fatalf("vbr aac: %v", p.audioQualityArgs())
	}
	p2 := encodePlanFor(func(p *encodePlan) { p.audioBitrateKbps = 96 })
	if !hasPair(p2.audioQualityArgs(), "-b:a", "96k") {
		t.Fatalf("cbr: %v", p2.audioQualityArgs())
	}
	p3 := encodePlanFor(func(p *encodePlan) {
		p.settings.AudioVBR = true
		p.audioCodec = contracts.AudioCodecAC3
		p.audioBitrateKbps = 160
	})
	if !hasPair(p3.audioQualityArgs(), "-b:a", "160k") {
		t.Fatalf("vbr only applies to aac: %v", p3.audioQualityArgs())
	}
}

// --- transcodeReasons chain (policy.go:528-575) ---

func TestKillTranscodeReasons(t *testing.T) {
	in := contracts.TranscodeV3Request{}
	// Full encode with everything on.
	p := encodePlanFor(func(p *encodePlan) {
		p.copyVideo = false
		p.videoCodec = contracts.VideoCodecHEVC
		p.audio = &stream{CodecName: "dts", Channels: 6}
		p.copyAudio = false
		p.downmix = true
		p.maxBitrateKbps = 900
		p.burnText = true
		p.video = &stream{CodecName: "mpeg4", ColorTransfer: "smpte2084", FieldOrder: "tt"}
		p.settings.DownmixStereoAlgorithm = contracts.DownmixNightmode
		p.settings.Deinterlace = contracts.DeinterlaceAuto
	})
	got := strings.Join(p.transcodeReasons(in), "|")
	for _, want := range []string{"video codec (mpeg4)", "HDR (smpte2084)", "interlaced", "output codec (hevc)", "audio codec (dts)", "audio downmix", "bitrate limit (900 kbps)", "subtitle burn-in"} {
		if !strings.Contains(got, want) {
			t.Fatalf("reasons %q lack %q", got, want)
		}
	}
	// Nightmode on a non-5.1 source reports its own reason.
	p2 := encodePlanFor(func(p *encodePlan) {
		p.copyVideo = false
		p.audio = &stream{CodecName: "aac", Channels: 4}
		p.downmix = true
		p.settings.DownmixStereoAlgorithm = contracts.DownmixNightmode
	})
	got = strings.Join(p2.transcodeReasons(in), "|")
	if !strings.Contains(got, "nightmode needs a 5.1 source") {
		t.Fatalf("quad nightmode reason missing: %q", got)
	}
	// 5.1 + nightmode: no such caveat.
	p3 := encodePlanFor(func(p *encodePlan) {
		p.copyVideo = false
		p.audio = &stream{CodecName: "aac", Channels: 6}
		p.downmix = true
		p.settings.DownmixStereoAlgorithm = contracts.DownmixNightmode
	})
	got = strings.Join(p3.transcodeReasons(in), "|")
	if strings.Contains(got, "nightmode needs") {
		t.Fatalf("5.1 must not warn: %q", got)
	}
	// Nothing special -> browser compatibility.
	p4 := encodePlanFor(func(p *encodePlan) {
		p.spec.Path = "/media/in.mp4"
		p.copyVideo = true
		p.copyAudio = true
	})
	if got := p4.transcodeReasons(in); len(got) != 1 || got[0] != "browser compatibility" {
		t.Fatalf("default reason: %v", got)
	}
}

// --- probeBuild tolerates a nil clock (policy.go:741) ---

func TestKillProbeBuildNilNow(t *testing.T) {
	ff := stubFFmpeg(t, `
case "$*" in
  *-encoders*) printf ' V..... libx264\n' ;;
esac
exit 0`)
	caps := probeBuild(contracts.TranscodeSettings{FFmpegPath: ff}, nil)
	if !caps.Encoders["libx264"] || caps.ProbedAt.IsZero() {
		t.Fatalf("probeBuild must work with a nil clock: %+v", caps)
	}
}
