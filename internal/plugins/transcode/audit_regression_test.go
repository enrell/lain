package transcode

// Regression tests for the defects an adversarial audit found in the
// D-042 transcode slice. Each test pins the fixed behavior so it cannot
// silently regress.

import (
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

// TestAuditHLSRunContextHasNoDeadline pins fix #2: a throttled HLS
// session runs for the viewer's whole watch time, so it must not inherit
// the bounded preparation deadline that would kill it at one hour.
func TestAuditHLSRunContextHasNoDeadline(t *testing.T) {
	tr, _ := v3Fixture(t, Config{})

	hlsCtx, hlsCancel := tr.runContext(contracts.TranscodeDeliveryHLS)
	defer hlsCancel()
	if _, ok := hlsCtx.Deadline(); ok {
		t.Fatal("an HLS session must not carry a total-time deadline")
	}

	progCtx, progCancel := tr.runContext(contracts.TranscodeDeliveryProgressive)
	defer progCancel()
	if _, ok := progCtx.Deadline(); !ok {
		t.Fatal("a progressive preparation must be bounded by a deadline")
	}
}

// TestAuditQueueSizeSettingIsEnforced pins fix #3: the persisted
// queue_size setting must bound admission, not only the channel built
// from the CLI flag.
func TestAuditQueueSizeSettingIsEnforced(t *testing.T) {
	tr, _ := v3Fixture(t, Config{QueueSize: 8, MaxConcurrent: 1})

	release := make(chan struct{})
	releaseOnce(t, release)
	started := make(chan struct{})
	var once sync.Once
	tr.v3run = func(j *job) error {
		once.Do(func() { close(started) })
		<-release
		return writeV3Artifact(tr, j.spec.Session)
	}

	dir := t.TempDir()
	mk := func(name string) string {
		p := filepath.Join(dir, name)
		if err := os.WriteFile(p, []byte("source"), 0o600); err != nil {
			t.Fatal(err)
		}
		return p
	}
	req := func(src string) contracts.TranscodeV3Request {
		in := v3Request(contracts.TranscodeStartAction, src)
		in.Settings.QueueSize = 1
		// One slot so the second job genuinely waits for admission
		// instead of running alongside the first.
		in.Settings.MaxConcurrent = 1
		return in
	}

	// The first job takes the only slot and blocks; the second is
	// admitted and waits for a slot (pending == 1).
	v3Invoke(t, tr, req(mk("a.mkv")))
	select {
	case <-started:
	case <-time.After(2 * time.Second):
		t.Fatal("the first job never started")
	}
	v3Invoke(t, tr, req(mk("b.mkv")))

	// A third start must be refused by the adopted queue_size of 1, not
	// the channel capacity (8/64).
	out, err := tr.Invoke(contracts.CapPlaybackTranscodeV3, req(mk("c.mkv")))
	if err == nil {
		t.Fatalf("third start was admitted despite queue_size=1: %+v", out)
	}
	var ce *core.Error
	if !errors.As(err, &ce) || ce.Code != "queue-full" {
		t.Fatalf("third start error=%v, want queue-full", err)
	}
}

// TestAuditBitrateCapReachesAllQualityEncoders pins fix #5: a requested
// bitrate cap must appear in the argv for every quality-mode encoder,
// including the ones that previously dropped it.
func TestAuditBitrateCapReachesAllQualityEncoders(t *testing.T) {
	encoders := []string{
		"libx264", "libx265", "libsvtav1", "libaom-av1",
		"h264_amf", "hevc_amf",
		"h264_videotoolbox", "hevc_videotoolbox",
		"h264_vaapi", "hevc_vaapi", "h264_qsv", "h264_nvenc",
	}
	for _, enc := range encoders {
		args := encodePlan{encoder: enc, preset: "veryfast", crf: 23, maxBitrateKbps: 4000}.videoQualityArgs()
		if !hasPair(args, "-maxrate", "4000k") {
			t.Errorf("%s: args %v lack -maxrate 4000k", enc, args)
		}
		if !hasPair(args, "-bufsize", "8000k") {
			t.Errorf("%s: args %v lack -bufsize 8000k", enc, args)
		}
	}
	// With no cap requested, no maxrate is emitted.
	if hasPair((encodePlan{encoder: "libx264", preset: "veryfast", crf: 23}).videoQualityArgs(), "-maxrate", "") {
		t.Error("an uncapped encode must not emit -maxrate")
	}
}

func hasPair(args []string, key, value string) bool {
	for i := 0; i+1 < len(args); i++ {
		if args[i] == key && (value == "" || args[i+1] == value) {
			return true
		}
	}
	return false
}

// TestAuditMalformedSessionIsRejected pins fix #7: a client-supplied
// session id must never be joined into a filesystem path.
func TestAuditMalformedSessionIsRejected(t *testing.T) {
	for _, bad := range []string{"", "deadbeef", "../../etc/passwd", strings.Repeat("g", 64), strings.Repeat("a", 63)} {
		if validSessionID(bad) {
			t.Errorf("validSessionID(%q) = true, want false", bad)
		}
	}
	if !validSessionID(strings.Repeat("ab12", 16)) {
		t.Fatal("validSessionID rejected a well-formed 64-hex id")
	}

	tr, src := v3Fixture(t, Config{})
	out, err := tr.Invoke(contracts.CapPlaybackTranscodeV3, contracts.TranscodeV3Request{
		Action: contracts.TranscodeCancelAction, Session: "../../etc/passwd",
	})
	if err != nil {
		t.Fatalf("cancel of a malformed session returned %v", err)
	}
	if status, ok := out.(contracts.TranscodeV3Status); !ok || status.State != contracts.TranscodeIdle {
		t.Fatalf("cancel of a malformed session = %+v, want idle", out)
	}
	if _, err := tr.Invoke(contracts.CapPlaybackTranscodeV3, contracts.TranscodeV3Request{
		Action: contracts.TranscodeStatusAction, FilePath: src, Session: "../../etc/passwd",
	}); err == nil {
		t.Fatal("status of a malformed session was not rejected")
	}
}

// TestAuditUnknownQualityRejected pins fix #9: an unknown ladder name
// fails loudly instead of dropping the cap and forking a cache entry.
func TestAuditUnknownQualityRejected(t *testing.T) {
	if _, err := advPlan(t, v3Report(), testCaps(), func(in *contracts.TranscodeV3Request) {
		in.Quality = "not-a-ladder-entry"
	}); err == nil {
		t.Fatal("an unknown quality name was accepted")
	}

	known := contracts.DefaultTranscodeSettings().Qualities[0].Name
	if _, err := advPlan(t, v3Report(), testCaps(), func(in *contracts.TranscodeV3Request) {
		in.Quality = known
	}); err != nil {
		t.Fatalf("known quality %q was rejected: %v", known, err)
	}
}

// TestAuditCancelBindsSessionToSource pins the ownership half of fix #7:
// a cancel whose source path does not match the session's source is a
// no-op, so a client cannot delete another item's derivatives.
func TestAuditCancelBindsSessionToSource(t *testing.T) {
	tr, src := v3Fixture(t, Config{})
	tr.v3run = func(j *job) error { return writeV3Artifact(tr, j.spec.Session) }

	started := v3Invoke(t, tr, v3Request(contracts.TranscodeStartAction, src))
	ready := waitV3(t, tr, src, started.Session, contracts.TranscodeReady)
	media := tr.pathsFor(ready.Session, contracts.TranscodeSettings{}).media
	if _, err := os.Stat(media); err != nil {
		t.Fatalf("artifact missing before cancel: %v", err)
	}

	v3Invoke(t, tr, contracts.TranscodeV3Request{
		Action: contracts.TranscodeCancelAction, Session: ready.Session, FilePath: "/media/other.mkv",
	})
	if _, err := os.Stat(media); err != nil {
		t.Fatalf("a mismatched-source cancel removed the artifact: %v", err)
	}

	v3Invoke(t, tr, contracts.TranscodeV3Request{
		Action: contracts.TranscodeCancelAction, Session: ready.Session, FilePath: src,
	})
	if _, err := os.Stat(media); !os.IsNotExist(err) {
		t.Fatalf("a matching-source cancel left the artifact: %v", err)
	}
}

// TestAuditCachedSessionHonorsPolicy pins the policy check on the cached
// path: a user denied remux/transcode must not be served a derivative
// another account produced (the cache is keyed by source, not account).
func TestAuditCachedSessionHonorsPolicy(t *testing.T) {
	tr, src := v3Fixture(t, Config{})
	tr.v3run = func(j *job) error { return writeV3Artifact(tr, j.spec.Session) }

	started := v3Invoke(t, tr, v3Request(contracts.TranscodeStartAction, src))
	ready := waitV3(t, tr, src, started.Session, contracts.TranscodeReady)
	if ready.Method != "remux" {
		t.Skipf("fixture planned %q, not remux", ready.Method)
	}

	deny := false
	out, err := tr.Invoke(contracts.CapPlaybackTranscodeV3, contracts.TranscodeV3Request{
		Action:   contracts.TranscodeStartAction,
		FilePath: src,
		Delivery: contracts.TranscodeDeliveryProgressive,
		Settings: contracts.DefaultTranscodeSettings(),
		Policy:   &contracts.TranscodePolicy{AllowVideoTranscode: true, AllowAudioTranscode: true, AllowRemux: deny},
	})
	if err == nil {
		t.Fatalf("a denied remux was served from cache: %+v", out)
	}
	var ce *core.Error
	if !errors.As(err, &ce) || ce.Code != "forbidden" {
		t.Fatalf("cached policy error=%v, want forbidden", err)
	}
}

// TestAuditProfileKeyTracksLadderValues pins fix #4: editing a ladder
// entry's output-affecting values must change the cache/session key.
func TestAuditProfileKeyTracksLadderValues(t *testing.T) {
	base := contracts.DefaultTranscodeSettings()
	name := base.Qualities[1].Name
	req := contracts.TranscodeV3Request{Quality: name}

	edited := base
	edited.Qualities = append([]contracts.TranscodeQuality(nil), base.Qualities...)
	edited.Qualities[1].MaxHeight = 720
	edited.Qualities[1].BitrateKbps = 2000

	if contracts.TranscodeProfileKey(base, req) == contracts.TranscodeProfileKey(edited, req) {
		t.Fatal("editing a ladder entry did not change the profile key")
	}
}

// TestAuditLibAOMCapUsesBitrate pins the libaom-specific half of fix #5:
// libaom refuses any rate-control parameter without a bitrate, so a
// requested ceiling must be carried by -b:v, not the CRF-mode -b:v 0.
func TestAuditLibAOMCapUsesBitrate(t *testing.T) {
	capped := encodePlan{encoder: "libaom-av1", preset: "veryfast", crf: 23, maxBitrateKbps: 4000}.videoQualityArgs()
	if !hasPair(capped, "-b:v", "4000k") {
		t.Fatalf("capped libaom args %v must carry the cap in -b:v", capped)
	}
	if hasPair(capped, "-b:v", "0") {
		t.Fatalf("capped libaom args %v must not pin -b:v 0", capped)
	}
	uncapped := encodePlan{encoder: "libaom-av1", preset: "veryfast", crf: 23}.videoQualityArgs()
	if !hasPair(uncapped, "-b:v", "0") {
		t.Fatalf("uncapped libaom args %v must stay in CRF mode", uncapped)
	}
}

// TestAuditEncoderArgsAcceptedByRealFFmpeg is a real test: it runs the
// argv videoQualityArgs produces through the actual ffmpeg binary for
// every software encoder the build ships, so an argument combination
// ffmpeg rejects (libaom's "rate control without a bitrate") fails here
// instead of at a viewer's first play.
func TestAuditEncoderArgsAcceptedByRealFFmpeg(t *testing.T) {
	ffmpeg, err := exec.LookPath("ffmpeg")
	if err != nil {
		t.Skip("ffmpeg not available")
	}
	for _, enc := range []string{"libx264", "libx265", "libsvtav1", "libaom-av1"} {
		enc := enc
		t.Run(enc, func(t *testing.T) {
			if !ffmpegHasEncoder(ffmpeg, enc) {
				t.Skipf("%s not built into this ffmpeg", enc)
			}
			args := encodePlan{encoder: enc, preset: "veryfast", crf: 23, maxBitrateKbps: 4000}.videoQualityArgs()
			cmd := append([]string{"-hide_banner", "-loglevel", "error",
				"-f", "lavfi", "-i", "testsrc2=size=320x180:rate=12", "-t", "1",
				"-c:v", enc}, args...)
			cmd = append(cmd, "-f", "null", "-")
			if out, err := exec.Command(ffmpeg, cmd...).CombinedOutput(); err != nil {
				t.Fatalf("ffmpeg rejected the argv for %s: %v\nargs: %v\n%s", enc, err, args, out)
			}
		})
	}
}

func ffmpegHasEncoder(ffmpeg, name string) bool {
	out, err := exec.Command(ffmpeg, "-hide_banner", "-encoders").Output()
	if err != nil {
		return false
	}
	return strings.Contains(string(out), name+" ")
}

// TestAuditAV1PresetReachesArgv pins the AV1-preset no-op fix: the
// configured preset must change the encoder argv instead of a constant.
func TestAuditAV1PresetReachesArgv(t *testing.T) {
	fastSVT := encodePlan{encoder: "libsvtav1", preset: "ultrafast", crf: 32}.videoQualityArgs()
	slowSVT := encodePlan{encoder: "libsvtav1", preset: "veryslow", crf: 32}.videoQualityArgs()
	if !hasPair(fastSVT, "-preset", "13") || !hasPair(slowSVT, "-preset", "0") {
		t.Fatalf("svt preset did not track the ladder: fast=%v slow=%v", fastSVT, slowSVT)
	}
	fastAOM := encodePlan{encoder: "libaom-av1", preset: "ultrafast", crf: 32}.videoQualityArgs()
	slowAOM := encodePlan{encoder: "libaom-av1", preset: "veryslow", crf: 32}.videoQualityArgs()
	if !hasPair(fastAOM, "-cpu-used", "8") || !hasPair(slowAOM, "-cpu-used", "2") {
		t.Fatalf("libaom cpu-used did not track the ladder: fast=%v slow=%v", fastAOM, slowAOM)
	}
}

// TestAuditHLSContainerArgs pins the encoder argv for both HLS
// containers: fMP4 carries an init segment, mpegts does not.
func TestAuditHLSContainerArgs(t *testing.T) {
	ts := contracts.DefaultTranscodeSettings()
	ts.HLSSegmentContainer = contracts.HLSSegmentTS
	tsArgs := encodePlan{settings: ts, copyVideo: true}.hlsArgs("/tmp/dir", "out.m3u8")
	if !hasPair(tsArgs, "-hls_segment_type", "mpegts") {
		t.Fatalf("mpegts args lack -hls_segment_type mpegts: %v", tsArgs)
	}
	if hasPair(tsArgs, "-hls_fmp4_init_filename", "") {
		t.Fatalf("mpegts args must not set an fmp4 init filename: %v", tsArgs)
	}
	foundTS := false
	for _, a := range tsArgs {
		if strings.HasSuffix(a, "seg%05d.ts") {
			foundTS = true
		}
	}
	if !foundTS {
		t.Fatalf("mpegts args lack a .ts segment filename: %v", tsArgs)
	}

	fmp4Args := encodePlan{settings: contracts.DefaultTranscodeSettings(), copyVideo: true}.hlsArgs("/tmp/dir", "out.m3u8")
	if !hasPair(fmp4Args, "-hls_segment_type", "fmp4") {
		t.Fatalf("fmp4 args lack -hls_segment_type fmp4: %v", fmp4Args)
	}
	if !hasPair(fmp4Args, "-hls_fmp4_init_filename", "init.mp4") {
		t.Fatalf("fmp4 args lack an init filename: %v", fmp4Args)
	}
}

// TestAuditQSVLowPowerFlag pins the low-power setting: it affects only
// QSV encoders and only when enabled.
func TestAuditQSVLowPowerFlag(t *testing.T) {
	on := encodePlan{encoder: "h264_qsv", preset: "veryfast", crf: 23}
	on.settings.HardwareLowPower = true
	if !hasPair(on.videoQualityArgs(), "-low_power", "1") {
		t.Fatalf("low-power QSV args lack -low_power 1: %v", on.videoQualityArgs())
	}
	off := encodePlan{encoder: "h264_qsv", preset: "veryfast", crf: 23}
	if hasPair(off.videoQualityArgs(), "-low_power", "1") {
		t.Fatalf("QSV args carried -low_power with the setting off: %v", off.videoQualityArgs())
	}
	other := encodePlan{encoder: "h264_nvenc", preset: "veryfast", crf: 23}
	other.settings.HardwareLowPower = true
	if hasPair(other.videoQualityArgs(), "-low_power", "1") {
		t.Fatalf("a non-QSV encoder carried -low_power: %v", other.videoQualityArgs())
	}
}

// TestAuditCancelQueuedJobDoesNotRun pins the queued-cancel fix: a job
// cancelled before it starts must not run its encode (runV3 only runs a
// job still marked queued).
func TestAuditCancelQueuedJobDoesNotRun(t *testing.T) {
	tr, _ := v3Fixture(t, Config{MaxConcurrent: 1})

	release := make(chan struct{})
	var releaseOnceSync sync.Once
	releaseNow := func() { releaseOnceSync.Do(func() { close(release) }) }
	t.Cleanup(releaseNow)

	started := make(chan struct{})
	var once sync.Once
	var mu sync.Mutex
	ran := map[string]int{}
	tr.v3run = func(j *job) error {
		once.Do(func() { close(started) })
		mu.Lock()
		ran[j.spec.Session]++
		mu.Unlock()
		<-release
		return writeV3Artifact(tr, j.spec.Session)
	}

	dir := t.TempDir()
	mk := func(name string) string {
		p := filepath.Join(dir, name)
		if err := os.WriteFile(p, []byte("source"), 0o600); err != nil {
			t.Fatal(err)
		}
		return p
	}
	req := func(src string) contracts.TranscodeV3Request {
		in := v3Request(contracts.TranscodeStartAction, src)
		in.Settings.MaxConcurrent = 1
		return in
	}

	aSrc := mk("a.mkv")
	a := v3Invoke(t, tr, req(aSrc))
	select {
	case <-started:
	case <-time.After(2 * time.Second):
		t.Fatal("first job never started")
	}
	bSrc := mk("b.mkv")
	b := v3Invoke(t, tr, req(bSrc))

	// Cancel B while it is still waiting for the only slot.
	v3Invoke(t, tr, contracts.TranscodeV3Request{
		Action: contracts.TranscodeCancelAction, Session: b.Session, FilePath: bSrc,
	})

	// Let A finish so the worker would pick B up if it were not cancelled.
	releaseNow()
	waitV3(t, tr, aSrc, a.Session, contracts.TranscodeReady)
	time.Sleep(150 * time.Millisecond)

	mu.Lock()
	n := ran[b.Session]
	mu.Unlock()
	if n != 0 {
		t.Fatalf("a cancelled queued job still ran its encode %d time(s)", n)
	}
}

// TestAuditSweepRespectsTempRootOwnership pins the shared-temp-root fix:
// an instance that does not own <temp>/lain-transcode must not sweep it.
func TestAuditSweepRespectsTempRootOwnership(t *testing.T) {
	tr, _ := v3Fixture(t, Config{})
	settings := contracts.DefaultTranscodeSettings()
	settings.TranscodeTempPath = filepath.Join(t.TempDir(), "temp")
	if err := ensureRelocatedRoot(settings); err != nil {
		t.Fatal(err)
	}
	root := relocateRoot(settings)

	live := filepath.Join(root, "live-session")
	if err := os.MkdirAll(live, 0o700); err != nil {
		t.Fatal(err)
	}

	// A live foreign owner: leave the root alone.
	foreign := filepath.Join(t.TempDir(), "other", "transcodes")
	if err := os.MkdirAll(foreign, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, ".owner"), []byte(foreign), 0o600); err != nil {
		t.Fatal(err)
	}
	tr.sweepRelocatedOrphans(root, map[string]bool{})
	if _, err := os.Stat(live); err != nil {
		t.Fatalf("a foreign-owned temp root was swept: %v", err)
	}

	// A stale owner (its data dir is gone) is taken over.
	if err := os.WriteFile(filepath.Join(root, ".owner"), []byte(filepath.Join(t.TempDir(), "gone", "transcodes")), 0o600); err != nil {
		t.Fatal(err)
	}
	tr.sweepRelocatedOrphans(root, map[string]bool{})
	if _, err := os.Stat(live); !os.IsNotExist(err) {
		t.Fatalf("a stale owner was not taken over: %v", err)
	}
}

// TestAuditV2SessionValidation pins the residual v2 hardening: a client
// session id never reaches metaPath unvalidated on the v2 path either.
func TestAuditV2SessionValidation(t *testing.T) {
	tr, src := v3Fixture(t, Config{})
	if _, err := tr.bySession(src, "../../etc/passwd", false); err == nil {
		t.Fatal("bySession accepted a malformed session id")
	}
}

// TestAuditCancelRespectsSessionOwnership pins the ownership check: a
// session may only be cancelled by the account that started it (or by an
// admin cancel, which sends no user id).
func TestAuditCancelRespectsSessionOwnership(t *testing.T) {
	tr, src := v3Fixture(t, Config{})
	release := make(chan struct{})
	releaseOnce(t, release)
	tr.v3run = func(j *job) error {
		<-release
		return writeV3Artifact(tr, j.spec.Session)
	}

	start := v3Request(contracts.TranscodeStartAction, src)
	start.UserID = "user-a"
	started := v3Invoke(t, tr, start)

	// A different user must not cancel it.
	v3Invoke(t, tr, contracts.TranscodeV3Request{
		Action: contracts.TranscodeCancelAction, Session: started.Session, FilePath: src, UserID: "user-b",
	})
	tr.mu.Lock()
	_, still := tr.jobs[started.Session]
	tr.mu.Unlock()
	if !still {
		t.Fatal("a different user cancelled the session")
	}

	// The owner can.
	v3Invoke(t, tr, contracts.TranscodeV3Request{
		Action: contracts.TranscodeCancelAction, Session: started.Session, FilePath: src, UserID: "user-a",
	})
	tr.mu.Lock()
	_, still = tr.jobs[started.Session]
	tr.mu.Unlock()
	if still {
		t.Fatal("the owner could not cancel the session")
	}
}

// TestAuditCancelIgnoresLegacyJobs pins the guard that keeps a v3 cancel
// from stranding a synchronous v1/v2 waiter: a legacy job is left alone.
func TestAuditCancelIgnoresLegacyJobs(t *testing.T) {
	tr, _ := v3Fixture(t, Config{})
	session := strings.Repeat("ab", 32)
	legacy := &job{spec: sourceSpec{Session: session}, state: contracts.TranscodeQueued, done: make(chan struct{})}
	tr.mu.Lock()
	tr.jobs[session] = legacy
	tr.mu.Unlock()

	st := tr.cancelV3(session, "", "")
	if st.State != contracts.TranscodeIdle {
		t.Fatalf("cancel of a legacy job returned state %q, want idle", st.State)
	}
	tr.mu.Lock()
	_, still := tr.jobs[session]
	state := legacy.state
	tr.mu.Unlock()
	if !still {
		t.Fatal("a v3 cancel removed a legacy job")
	}
	if state != contracts.TranscodeQueued {
		t.Fatalf("a v3 cancel changed a legacy job state to %q", state)
	}
}

// TestAuditAudioCodecCopyAndValidation pins the audio fixes: a source that
// already matches the requested MP4 codec is copied (no needless
// re-encode), and an unknown audio_codec is rejected before the argv.
func TestAuditAudioCodecCopyAndValidation(t *testing.T) {
	report := mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "h264", PixelFormat: "yuv420p", Width: 1280, Height: 720},
		{Index: 1, CodecType: "audio", CodecName: "ac3", Channels: 2, Default: 1},
	}}
	copied := advRequirePlan(t, report, testCaps(), func(in *contracts.TranscodeV3Request) {
		in.AudioCodec = contracts.AudioCodecAC3
	})
	if !copied.copyAudio {
		t.Fatalf("an ac3 source asked to stay ac3 should copy: %+v", copied)
	}

	if _, err := advPlan(t, report, testCaps(), func(in *contracts.TranscodeV3Request) {
		in.AudioCodec = "not-a-codec"
	}); err == nil {
		t.Fatal("an unknown audio_codec was accepted")
	}
}

// TestAuditBitrateCapForcesEncodeOverCopy pins the cap fix: a requested
// cap below the source bitrate must force a re-encode, not a full-rate
// remux that reports the cap but ignores it.
func TestAuditBitrateCapForcesEncodeOverCopy(t *testing.T) {
	report := mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "h264", PixelFormat: "yuv420p", Width: 1920, Height: 1080, BitRate: "12000000"},
		{Index: 1, CodecType: "audio", CodecName: "aac", Channels: 2, Default: 1, BitRate: "128000"},
	}}
	uncapped := advRequirePlan(t, report, testCaps(), nil)
	if !uncapped.copyVideo {
		t.Fatalf("a web-safe source should copy without a cap: %+v", uncapped)
	}

	capped := advRequirePlan(t, report, testCaps(), func(in *contracts.TranscodeV3Request) {
		in.MaxBitrateKbps = 2000
	})
	if capped.copyVideo {
		t.Fatal("a cap below the source bitrate must force a re-encode, not a copy")
	}
	if capped.method != "transcode" {
		t.Fatalf("method=%q, want transcode", capped.method)
	}
}

// TestAuditNightmodeDownmixNeedsSurround pins the layout guard: lain's
// nightmode matrix names 5.1 positions, so it is only applied to a real
// 5.1 source, and the status explains the fallback.
func TestAuditNightmodeDownmixNeedsSurround(t *testing.T) {
	report := func(channels int) mediaReport {
		return mediaReport{Streams: []stream{
			{Index: 0, CodecType: "video", CodecName: "h264", PixelFormat: "yuv420p", Width: 1280, Height: 720},
			{Index: 1, CodecType: "audio", CodecName: "aac", Channels: channels, Default: 1},
		}}
	}
	nightmode := func(in *contracts.TranscodeV3Request) {
		in.Settings.DownmixStereoAlgorithm = contracts.DownmixNightmode
	}

	surround := advRequirePlan(t, report(6), testCaps(), nightmode)
	if !strings.Contains(surround.audioFilters(), "pan=stereo") {
		t.Fatalf("a 5.1 source should use the nightmode matrix: %q", surround.audioFilters())
	}

	quad := advRequirePlan(t, report(4), testCaps(), nightmode)
	if strings.Contains(quad.audioFilters(), "pan=stereo") {
		t.Fatalf("a quad source must not use the 5.1 matrix: %q", quad.audioFilters())
	}
	noted := false
	for _, r := range quad.reasons {
		if strings.Contains(r, "nightmode needs a 5.1 source") {
			noted = true
		}
	}
	if !noted {
		t.Fatalf("reasons=%v, want the nightmode fallback note", quad.reasons)
	}
}

// TestAuditReasonsSurvivePlanBuild pins the merge fix: a cause recorded
// while resolving streams (a client-side copy flag) still reaches the
// status reason list instead of being replaced by the later pass.
func TestAuditReasonsSurvivePlanBuild(t *testing.T) {
	no := false
	plan := advRequirePlan(t, webSafeReport(), testCaps(), func(in *contracts.TranscodeV3Request) {
		in.AllowVideoStreamCopy = &no
	})
	found := false
	for _, r := range plan.reasons {
		if strings.Contains(r, "video stream copy disabled by the client") {
			found = true
		}
	}
	if !found {
		t.Fatalf("reasons=%v, want the client copy note", plan.reasons)
	}
}

// TestAuditFallbackFontsToggle pins Jellyfin's "Enable fallback fonts":
// an explicit false drops the burn-in font options even with a path set,
// and the profile key tracks the effective permission.
func TestAuditFallbackFontsToggle(t *testing.T) {
	on := contracts.DefaultTranscodeSettings()
	on.FallbackFontPath = "/tmp/fonts"
	on.FallbackFontName = "DejaVu Sans"
	if got := subtitleFontOptions(on); !strings.Contains(got, "fontsdir=") {
		t.Fatalf("fallback fonts on: options=%q, want a fontsdir", got)
	}

	off := on
	disabled := false
	off.FallbackFontEnabled = &disabled
	if got := subtitleFontOptions(off); got != "" {
		t.Fatalf("fallback fonts off: options=%q, want none", got)
	}

	req := contracts.TranscodeV3Request{}
	if contracts.TranscodeProfileKey(on, req) == contracts.TranscodeProfileKey(off, req) {
		t.Fatal("the profile key ignored the fallback-font permission")
	}
}

// TestAuditStatusPollDoesNotKeepSessionAlive pins the idle-timeout fix: a
// status poll must not refresh lastTouch, while a media fetch (resolve)
// must.
func TestAuditStatusPollDoesNotKeepSessionAlive(t *testing.T) {
	root := t.TempDir()
	src := filepath.Join(root, "[Fansub-A] Show.mkv")
	if err := os.WriteFile(src, []byte("source"), 0o600); err != nil {
		t.Fatal(err)
	}
	var clock atomic.Int64
	clock.Store(time.Unix(1000, 0).UnixNano())
	tr := newWithDeps(filepath.Join(root, "cache"), Config{}, nil, func() time.Time { return time.Unix(0, clock.Load()) })
	t.Cleanup(func() { _ = tr.Close() })
	tr.probeFn = func(string, string) (mediaReport, error) { return v3Report(), nil }
	tr.capsFn = func(contracts.TranscodeSettings) capabilities { return testCaps() }

	release := make(chan struct{})
	releaseOnce(t, release)
	tr.v3run = func(j *job) error {
		<-release
		return writeV3Artifact(tr, j.spec.Session)
	}

	started := v3Invoke(t, tr, v3Request(contracts.TranscodeStartAction, src))
	tr.mu.Lock()
	j := tr.jobs[started.Session]
	tr.mu.Unlock()
	if j == nil {
		t.Fatal("job not tracked")
	}
	tr.mu.Lock()
	before := j.lastTouch
	tr.mu.Unlock()

	clock.Add(int64(10 * time.Second))
	v3Invoke(t, tr, contracts.TranscodeV3Request{Action: contracts.TranscodeStatusAction, FilePath: src, Session: started.Session})
	tr.mu.Lock()
	afterPoll := j.lastTouch
	tr.mu.Unlock()
	if afterPoll != before {
		t.Fatalf("a status poll refreshed lastTouch: %d -> %d", before, afterPoll)
	}

	clock.Add(int64(10 * time.Second))
	if _, err := tr.Invoke(contracts.CapPlaybackTranscodeV3, contracts.TranscodeV3Request{
		Action: contracts.TranscodeResolveAction, FilePath: src, Session: started.Session,
	}); err != nil {
		t.Fatalf("resolve: %v", err)
	}
	tr.mu.Lock()
	afterResolve := j.lastTouch
	tr.mu.Unlock()
	if afterResolve == before {
		t.Fatal("a resolve did not refresh lastTouch")
	}
}

// TestAuditClientPositionPastEnd pins the throttle-stall fix: a segment
// index past the produced segments means the client is caught up, not at
// the start (returning 0 would strand ffmpeg paused forever).
func TestAuditClientPositionPastEnd(t *testing.T) {
	dir := t.TempDir()
	writeTestFile(t, filepath.Join(dir, hlsRawPlaylist), "#EXTM3U\n#EXTINF:4.000,\nseg00000.m4s\n#EXTINF:4.000,\nseg00001.m4s\n#EXT-X-ENDLIST\n")
	if got := clientPositionSec(dir, 0); !approx(got, 4) {
		t.Fatalf("clientPositionSec(0)=%v, want 4", got)
	}
	if got := clientPositionSec(dir, 99); !approx(got, 8) {
		t.Fatalf("clientPositionSec(99)=%v, want 8 (caught up), not 0", got)
	}
	if got := clientPositionSec(dir, -1); got != 0 {
		t.Fatalf("clientPositionSec(-1)=%v, want 0", got)
	}
}

// TestAuditReadyJobStatusHasPath pins the v1/v2 status projection: a
// finished in-memory job must report its artifact path, not an empty one.
func TestAuditReadyJobStatusHasPath(t *testing.T) {
	tr, _ := v3Fixture(t, Config{})
	session := strings.Repeat("ab", 32)
	j := &job{spec: sourceSpec{Session: session}, state: contracts.TranscodeReady}
	if status := tr.statusFromJob(j); status.Path == "" {
		t.Fatal("a ready in-memory job reported no path")
	}
}

// TestAuditStoppedJobIsTerminal pins the stop semantics: a session the
// idle timeout (or an operator) ended must not resurrect as Ready or be
// cached, even if the encoder finishes afterwards.
func TestAuditStoppedJobIsTerminal(t *testing.T) {
	tr, src := v3Fixture(t, Config{})
	release := make(chan struct{})
	var releaseOnceSync sync.Once
	releaseNow := func() { releaseOnceSync.Do(func() { close(release) }) }
	t.Cleanup(releaseNow)
	started := make(chan struct{})
	var once sync.Once
	tr.v3run = func(j *job) error {
		once.Do(func() { close(started) })
		<-release
		return writeV3Artifact(tr, j.spec.Session)
	}

	status := v3Invoke(t, tr, v3Request(contracts.TranscodeStartAction, src))
	<-started
	tr.mu.Lock()
	j := tr.jobs[status.Session]
	tr.mu.Unlock()
	if j == nil {
		t.Fatal("job not tracked")
	}

	// Stop the running job exactly as the idle timeout does.
	tr.stopJob(j, "idle-timeout", "session stopped after inactivity")
	releaseNow()

	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		tr.mu.Lock()
		state := j.state
		tr.mu.Unlock()
		if state == contracts.TranscodeFailed {
			break
		}
		time.Sleep(5 * time.Millisecond)
	}
	tr.mu.Lock()
	state, errCode := j.state, j.errCode
	tr.mu.Unlock()
	if state != contracts.TranscodeFailed {
		t.Fatalf("stopped job state=%q, want failed", state)
	}
	if errCode != "idle-timeout" {
		t.Fatalf("stopped job errCode=%q, want the stop reason preserved", errCode)
	}
	if _, err := os.Stat(tr.metaPath(status.Session)); err == nil {
		t.Fatal("a stopped job was cached")
	}
}
