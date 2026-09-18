package transcode

import (
	"os"
	"path/filepath"
	"reflect"
	"slices"
	"sync/atomic"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
)

func v3Report() mediaReport {
	return mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "h264", PixelFormat: "yuv420p", Width: 1920, Height: 1080},
		{Index: 1, CodecType: "audio", CodecName: "aac", Channels: 2, Default: 1},
	}}
}

// v3Fixture builds a transcoder over a real (tiny) source file with
// injected probe/capability seams: session mechanics must not need
// ffmpeg.
func v3Fixture(t *testing.T, cfg Config) (*Transcoder, string) {
	t.Helper()
	root := t.TempDir()
	src := filepath.Join(root, "[Fansub-A] Show.mkv")
	if err := os.WriteFile(src, []byte("source"), 0o600); err != nil {
		t.Fatal(err)
	}
	tr := newWithDeps(filepath.Join(root, "cache"), cfg, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	tr.probeFn = func(string, string) (mediaReport, error) { return v3Report(), nil }
	tr.capsFn = func(contracts.TranscodeSettings) capabilities { return testCaps() }
	return tr, src
}

func v3Request(action, src string) contracts.TranscodeV3Request {
	return contracts.TranscodeV3Request{
		Action:   action,
		FilePath: src,
		Delivery: contracts.TranscodeDeliveryProgressive,
		Settings: contracts.DefaultTranscodeSettings(),
	}
}

func v3Invoke(t *testing.T, tr *Transcoder, in contracts.TranscodeV3Request) contracts.TranscodeV3Status {
	t.Helper()
	out, err := tr.Invoke(contracts.CapPlaybackTranscodeV3, in)
	if err != nil {
		t.Fatalf("%s: %v", in.Action, err)
	}
	status, ok := out.(contracts.TranscodeV3Status)
	if !ok {
		t.Fatalf("%s returned %T", in.Action, out)
	}
	return status
}

func waitV3(t *testing.T, tr *Transcoder, src, session, want string) contracts.TranscodeV3Status {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		status := v3Invoke(t, tr, contracts.TranscodeV3Request{
			Action: contracts.TranscodeStatusAction, FilePath: src, Session: session,
		})
		if status.State == want {
			return status
		}
		time.Sleep(5 * time.Millisecond)
	}
	t.Fatalf("session %s did not reach %s", session, want)
	return contracts.TranscodeV3Status{}
}

// writeV3Artifact plays the role of the ffmpeg step for a progressive
// session; the injected run hook has to produce the file recordV3 checks.
func writeV3Artifact(tr *Transcoder, session string) error {
	return os.WriteFile(tr.pathsFor(session, contracts.TranscodeSettings{}).media, []byte("\x00\x00\x00\x18ftypmp42"), 0o600)
}

func releaseOnce(t *testing.T, release chan struct{}) {
	t.Helper()
	t.Cleanup(func() {
		select {
		case <-release:
		default:
			close(release)
		}
	})
}

// TestV3InspectStartStatusLifecycle is the basic session arc: inspect is
// idle, start keeps the same session id and the job reaches ready with
// a playable artifact.
func TestV3InspectStartStatusLifecycle(t *testing.T) {
	tr, src := v3Fixture(t, Config{})
	var calls atomic.Int32
	tr.v3run = func(j *job) error {
		calls.Add(1)
		return writeV3Artifact(tr, j.spec.Session)
	}

	idle := v3Invoke(t, tr, v3Request(contracts.TranscodeInspectAction, src))
	if idle.State != contracts.TranscodeIdle || idle.Session == "" || idle.Profile == "" {
		t.Fatalf("inspect=%+v, want idle with session and profile", idle)
	}
	if got := calls.Load(); got != 0 {
		t.Fatalf("inspect ran v3run %d times, want none", got)
	}

	started := v3Invoke(t, tr, v3Request(contracts.TranscodeStartAction, src))
	if started.Session != idle.Session || started.Profile != idle.Profile {
		t.Fatalf("start=%+v, want the inspected session %q", started, idle.Session)
	}
	if started.State != contracts.TranscodeQueued && started.State != contracts.TranscodeRunning {
		t.Fatalf("start state=%q", started.State)
	}
	if started.Delivery != contracts.TranscodeDeliveryProgressive {
		t.Fatalf("delivery=%q", started.Delivery)
	}

	ready := waitV3(t, tr, src, idle.Session, contracts.TranscodeReady)
	if !ready.Playable || ready.Path == "" || ready.Method != "remux" {
		t.Fatalf("ready=%+v, want a playable progressive remux", ready)
	}
	if _, err := os.Stat(ready.Path); err != nil {
		t.Fatalf("ready artifact missing: %v", err)
	}
	if got := calls.Load(); got != 1 {
		t.Fatalf("v3run calls=%d, want 1", got)
	}
}

// TestV3DuplicateStartJoinsRunningJob pins convergence: a second start
// with the same options must adopt the live session, not enqueue work.
func TestV3DuplicateStartJoinsRunningJob(t *testing.T) {
	tr, src := v3Fixture(t, Config{})
	release := make(chan struct{})
	releaseOnce(t, release)
	var calls atomic.Int32
	tr.v3run = func(j *job) error {
		calls.Add(1)
		<-release
		return writeV3Artifact(tr, j.spec.Session)
	}

	first := v3Invoke(t, tr, v3Request(contracts.TranscodeStartAction, src))
	waitV3(t, tr, src, first.Session, contracts.TranscodeRunning)

	second := v3Invoke(t, tr, v3Request(contracts.TranscodeStartAction, src))
	if second.Session != first.Session || second.State != contracts.TranscodeRunning {
		t.Fatalf("second start=%+v, want the running session %q", second, first.Session)
	}
	if got := calls.Load(); got != 1 {
		t.Fatalf("v3run calls=%d, want 1 (a duplicate start must not enqueue)", got)
	}

	close(release)
	waitV3(t, tr, src, first.Session, contracts.TranscodeReady)
	if got := calls.Load(); got != 1 {
		t.Fatalf("v3run calls=%d after ready, want 1", got)
	}
}

// TestV3SelectionChangesSession proves selection is part of the session
// identity (D-030): different output must not share a session.
func TestV3SelectionChangesSession(t *testing.T) {
	tr, src := v3Fixture(t, Config{})
	release := make(chan struct{})
	releaseOnce(t, release)
	tr.v3run = func(j *job) error {
		<-release
		return writeV3Artifact(tr, j.spec.Session)
	}

	first := v3Invoke(t, tr, v3Request(contracts.TranscodeStartAction, src))

	quality := v3Request(contracts.TranscodeStartAction, src)
	quality.Quality = "720p"
	second := v3Invoke(t, tr, quality)

	audio := v3Request(contracts.TranscodeStartAction, src)
	selected := 1
	audio.AudioStream = &selected
	third := v3Invoke(t, tr, audio)

	if first.Session == second.Session {
		t.Fatal("a quality change reused the session")
	}
	if first.Session == third.Session {
		t.Fatal("an audio selection change reused the session")
	}
	if second.Session == third.Session {
		t.Fatal("quality and audio sessions collided")
	}

	close(release)
	for _, session := range []string{first.Session, second.Session, third.Session} {
		waitV3(t, tr, src, session, contracts.TranscodeReady)
	}
}

// TestV3InspectIsSideEffectFree pins D-028/D-044: inspection never
// starts work, even for a different selection.
func TestV3InspectIsSideEffectFree(t *testing.T) {
	tr, src := v3Fixture(t, Config{})
	var calls atomic.Int32
	tr.v3run = func(*job) error {
		calls.Add(1)
		return nil
	}

	idle := v3Invoke(t, tr, v3Request(contracts.TranscodeInspectAction, src))
	if idle.State != contracts.TranscodeIdle || idle.Session == "" {
		t.Fatalf("inspect=%+v, want idle with a session", idle)
	}
	if idle.Delivery != contracts.DefaultTranscodeSettings().DefaultDelivery {
		t.Fatalf("delivery=%q, want the settings default", idle.Delivery)
	}
	if got := calls.Load(); got != 0 {
		t.Fatalf("v3run calls=%d, want none", got)
	}

	other := v3Request(contracts.TranscodeInspectAction, src)
	other.Quality = "480p"
	again := v3Invoke(t, tr, other)
	if again.State != contracts.TranscodeIdle || again.Session == idle.Session {
		t.Fatalf("second inspect=%+v", again)
	}
	if got := calls.Load(); got != 0 {
		t.Fatalf("v3run calls=%d after two inspects, want none", got)
	}
}

// TestV3CancelStopsRunningJob pins cancel semantics: the caller sees
// idle and the session is gone for later polls.
func TestV3CancelStopsRunningJob(t *testing.T) {
	tr, src := v3Fixture(t, Config{})
	release := make(chan struct{})
	releaseOnce(t, release)
	tr.v3run = func(j *job) error {
		<-release
		return writeV3Artifact(tr, j.spec.Session)
	}

	started := v3Invoke(t, tr, v3Request(contracts.TranscodeStartAction, src))
	waitV3(t, tr, src, started.Session, contracts.TranscodeRunning)

	cancelled := v3Invoke(t, tr, contracts.TranscodeV3Request{
		Action: contracts.TranscodeCancelAction, Session: started.Session,
	})
	if cancelled.State != contracts.TranscodeIdle || cancelled.Session != started.Session {
		t.Fatalf("cancel=%+v, want idle for %q", cancelled, started.Session)
	}

	_, err := tr.Invoke(contracts.CapPlaybackTranscodeV3, contracts.TranscodeV3Request{
		Action: contracts.TranscodeStatusAction, FilePath: src, Session: started.Session,
	})
	requireErrorCode(t, err, "not-found")
}

// TestV3PositionRecordsClientSegment pins the throttle/deletion input:
// the highest fetched segment is remembered, and a rewind is ignored.
func TestV3PositionRecordsClientSegment(t *testing.T) {
	tr, src := v3Fixture(t, Config{})
	release := make(chan struct{})
	releaseOnce(t, release)
	tr.v3run = func(j *job) error {
		<-release
		return writeV3Artifact(tr, j.spec.Session)
	}

	started := v3Invoke(t, tr, v3Request(contracts.TranscodeStartAction, src))
	waitV3(t, tr, src, started.Session, contracts.TranscodeRunning)

	v3Invoke(t, tr, contracts.TranscodeV3Request{
		Action: contracts.TranscodePositionAction, FilePath: src, Session: started.Session, SegmentIndex: 5,
	})
	tr.mu.Lock()
	got := tr.jobs[started.Session].clientSegment
	tr.mu.Unlock()
	if got != 5 {
		t.Fatalf("clientSegment=%d, want 5", got)
	}

	v3Invoke(t, tr, contracts.TranscodeV3Request{
		Action: contracts.TranscodePositionAction, FilePath: src, Session: started.Session, SegmentIndex: 2,
	})
	tr.mu.Lock()
	got = tr.jobs[started.Session].clientSegment
	tr.mu.Unlock()
	if got != 5 {
		t.Fatalf("clientSegment=%d after a lower report, want 5", got)
	}

	close(release)
	waitV3(t, tr, src, started.Session, contracts.TranscodeReady)
}

// TestV3QueueFullRejectsSecondStart pins bounded admission: the queue
// bound is enforced before a job is created.
func TestV3QueueFullRejectsSecondStart(t *testing.T) {
	tr, src := v3Fixture(t, Config{QueueSize: 1, MaxConcurrent: 1})
	secondSrc := filepath.Join(filepath.Dir(src), "[Fansub-A] Show 2.mkv")
	if err := os.WriteFile(secondSrc, []byte("source"), 0o600); err != nil {
		t.Fatal(err)
	}
	settings := contracts.DefaultTranscodeSettings()
	settings.QueueSize = 1
	settings.MaxConcurrent = 1
	release := make(chan struct{})
	releaseOnce(t, release)
	tr.v3run = func(j *job) error {
		<-release
		return writeV3Artifact(tr, j.spec.Session)
	}

	// Occupy the single concurrency slot so the first admitted job stays
	// pending; otherwise the worker drains it immediately and the bound
	// would be racy to observe.
	tr.mu.Lock()
	tr.active = tr.maxConcurrent
	tr.mu.Unlock()

	first := v3Invoke(t, tr, contracts.TranscodeV3Request{
		Action: contracts.TranscodeStartAction, FilePath: src,
		Delivery: contracts.TranscodeDeliveryProgressive, Settings: settings,
	})
	if first.State != contracts.TranscodeQueued {
		t.Fatalf("first start=%q, want queued behind the occupied slot", first.State)
	}

	_, err := tr.Invoke(contracts.CapPlaybackTranscodeV3, contracts.TranscodeV3Request{
		Action: contracts.TranscodeStartAction, FilePath: secondSrc,
		Delivery: contracts.TranscodeDeliveryProgressive, Settings: settings,
	})
	requireErrorCode(t, err, "queue-full")
}

// TestV3ProbeReportsCapabilities pins the admin-visible probe summary:
// only known encoder names are reported, hardware states are passed
// through honestly.
func TestV3ProbeReportsCapabilities(t *testing.T) {
	tr, _ := v3Fixture(t, Config{})
	tr.capsFn = func(contracts.TranscodeSettings) capabilities {
		return capabilities{
			Encoders: map[string]bool{"libx264": true, "h264_vaapi": true, "libnotlisted": true},
			Hardware: map[string]bool{contracts.HWVAAPI: true, contracts.HWNVENC: false},
			ToneMap:  true, ToneMap2390: false, FFmpeg: "/opt/ffmpeg",
		}
	}

	report := tr.Probe(contracts.DefaultTranscodeSettings())
	if report.FFmpeg != "/opt/ffmpeg" {
		t.Fatalf("ffmpeg=%q", report.FFmpeg)
	}
	if !report.ToneMapping || report.ToneMappingBT2390 {
		t.Fatalf("tone mapping=%v/%v", report.ToneMapping, report.ToneMappingBT2390)
	}
	if !slices.Contains(report.Encoders, "libx264") || !slices.Contains(report.Encoders, "h264_vaapi") {
		t.Fatalf("encoders=%v, want libx264 and h264_vaapi", report.Encoders)
	}
	if slices.Contains(report.Encoders, "libnotlisted") {
		t.Fatalf("encoders=%v must stay inside the known list", report.Encoders)
	}
	if len(report.Encoders) != 2 {
		t.Fatalf("encoders=%v, want exactly the two known names", report.Encoders)
	}
	if report.Hardware[contracts.HWVAAPI] != true || report.Hardware[contracts.HWNVENC] != false || len(report.Hardware) != 2 {
		t.Fatalf("hardware=%v", report.Hardware)
	}
}

// TestV3StatusProjections pins the trusted status shapes: paths are
// populated only for the delivery and state that actually have them.
func TestV3StatusProjections(t *testing.T) {
	tr, _ := v3Fixture(t, Config{})
	j := &job{
		spec:       sourceSpec{Session: "sess", Profile: "web-mp4-v3-abcdef"},
		state:      contracts.TranscodeRunning,
		delivery:   contracts.TranscodeDeliveryHLS,
		method:     "transcode",
		encoder:    "libx264",
		hwBackend:  contracts.HWVAAPI,
		fallback:   "note",
		reasons:    []string{"container (mkv)"},
		videoCodec: "h264",
		audioCodec: "aac",
		width:      1920, height: 1080, bitrateKbps: 8000,
		progress: 0.5, queuedAt: 10, startedAt: 11,
		hasSubtitle: true,
	}

	got := tr.statusV3FromJob(j)
	if got.Session != "sess" || got.State != contracts.TranscodeRunning ||
		got.Profile != "web-mp4-v3-abcdef" || got.Delivery != contracts.TranscodeDeliveryHLS {
		t.Fatalf("identity fields: %+v", got)
	}
	if got.Method != "transcode" || got.Encoder != "libx264" ||
		got.Hardware != contracts.HWVAAPI || got.Fallback != "note" {
		t.Fatalf("pipeline fields: %+v", got)
	}
	if got.VideoCodec != "h264" || got.AudioCodec != "aac" ||
		got.Width != 1920 || got.Height != 1080 || got.BitrateKbps != 8000 {
		t.Fatalf("output fields: %+v", got)
	}
	if !reflect.DeepEqual(got.Reasons, j.reasons) {
		t.Fatalf("reasons=%v", got.Reasons)
	}
	if got.Progress != 0.5 || got.QueuedAt != 10 || got.StartedAt != 11 || got.FinishedAt != 0 {
		t.Fatalf("timing fields: %+v", got)
	}
	if got.PlaylistPath == "" || got.Path != "" {
		t.Fatalf("running HLS must expose only the playlist: %+v", got)
	}
	if got.Playable {
		t.Fatal("a running session without segments must not be playable")
	}
	if got.SubtitlePath == "" {
		t.Fatal("a session with a sidecar must expose the subtitle path")
	}

	// A queued HLS job has neither playlist nor path yet.
	j.state = contracts.TranscodeQueued
	queued := tr.statusV3FromJob(j)
	if queued.PlaylistPath != "" || queued.Path != "" || queued.Playable {
		t.Fatalf("queued HLS=%+v", queued)
	}
	if queued.Progress != 0.5 {
		t.Fatalf("queued progress=%v, want the live fraction", queued.Progress)
	}

	// Ready HLS exposes the playlist; progressive exposes the file.
	j.state = contracts.TranscodeReady
	j.playable = true
	ready := tr.statusV3FromJob(j)
	if ready.PlaylistPath == "" || ready.Path != "" || !ready.Playable || ready.Progress != 0 {
		t.Fatalf("ready HLS=%+v", ready)
	}

	j.delivery = contracts.TranscodeDeliveryProgressive
	j.state = contracts.TranscodeRunning
	if running := tr.statusV3FromJob(j); running.Path != "" || running.PlaylistPath != "" {
		t.Fatalf("running progressive=%+v", running)
	}
	j.state = contracts.TranscodeReady
	progressive := tr.statusV3FromJob(j)
	if progressive.Path == "" || progressive.PlaylistPath != "" || !progressive.Playable {
		t.Fatalf("ready progressive=%+v", progressive)
	}

	// Cache entries always project as ready, cached and playable.
	e := cacheEntry{
		Session: "sess", Profile: "web-mp4-v3-abcdef", Delivery: contracts.TranscodeDeliveryHLS,
		Method: "remux", Encoder: "libx264", Hardware: contracts.HWVAAPI, Fallback: "note",
		Reasons: []string{"container (mkv)"}, VideoCodec: "h264", AudioCodec: "aac",
		Width: 1280, Height: 720, BitrateKbps: 4000, CreatedAt: 42, HasSubtitle: true,
	}
	entry := tr.statusV3FromEntry(e)
	if entry.State != contracts.TranscodeReady || !entry.Cached || !entry.Playable {
		t.Fatalf("entry state: %+v", entry)
	}
	if entry.PlaylistPath == "" || entry.Path != "" || entry.FinishedAt != 42 || entry.SubtitlePath == "" {
		t.Fatalf("entry HLS fields: %+v", entry)
	}
	if entry.Method != "remux" || entry.Encoder != "libx264" || entry.Hardware != contracts.HWVAAPI ||
		entry.Width != 1280 || entry.Height != 720 || entry.BitrateKbps != 4000 {
		t.Fatalf("entry pipeline fields: %+v", entry)
	}

	e.Delivery = contracts.TranscodeDeliveryProgressive
	entry = tr.statusV3FromEntry(e)
	if entry.Path == "" || entry.PlaylistPath != "" {
		t.Fatalf("entry progressive fields: %+v", entry)
	}
}
