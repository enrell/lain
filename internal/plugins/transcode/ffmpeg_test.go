package transcode

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-22

import (
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

func requireFFmpeg(t *testing.T) {
	t.Helper()
	if _, err := exec.LookPath("ffmpeg"); err != nil {
		t.Skip("ffmpeg not installed")
	}
}

// makeMKV synthesizes a tiny Matroska file with the given video and
// (optional) audio codecs. Empty audio means no audio track, like the
// common fansub release shape the remux fast path targets.
func makeMKV(t *testing.T, dir, name, vcodec, acodec string) string {
	t.Helper()
	requireFFmpeg(t)
	out := filepath.Join(dir, name)
	// All inputs come first, then every output option: -pix_fmt placed
	// before a later -i is parsed as an input option and ffmpeg exits with
	// "Option pixel_format not found", silently skipping this test.
	args := []string{"-hide_banner", "-loglevel", "error",
		"-f", "lavfi", "-i", "testsrc=size=160x120:rate=10:duration=2"}
	if acodec != "" {
		args = append(args,
			"-f", "lavfi", "-i", "sine=frequency=440:sample_rate=22050:duration=2")
	}
	args = append(args, "-c:v", vcodec, "-pix_fmt", "yuv420p")
	if acodec != "" {
		args = append(args, "-c:a", acodec)
	}
	args = append(args, "-y", out)
	if combo, err := exec.Command("ffmpeg", args...).CombinedOutput(); err != nil {
		t.Skipf("cannot synthesize mkv: %v: %s", err, combo)
	}
	return out
}

func prepare(t *testing.T, tr *Transcoder, src string) contracts.Transcode {
	t.Helper()
	out, err := tr.Invoke(contracts.CapPlaybackTranscode, contracts.TranscodeRequest{FilePath: src})
	if err != nil {
		t.Fatalf("prepare: %v", err)
	}
	return out.(contracts.Transcode)
}

func waitState(t *testing.T, tr *Transcoder, src, session, want string) contracts.TranscodeStatus {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		out, err := tr.Invoke(contracts.CapPlaybackTranscodeV2, contracts.TranscodeV2Request{
			Action: contracts.TranscodeStatusAction, FilePath: src, Session: session,
		})
		if err != nil {
			t.Fatalf("status: %v", err)
		}
		got := out.(contracts.TranscodeStatus)
		if got.State == want {
			return got
		}
		time.Sleep(5 * time.Millisecond)
	}
	t.Fatalf("session %s did not reach %s", session, want)
	return contracts.TranscodeStatus{}
}

func assertMP4(t *testing.T, path string) {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if len(data) < 8 || string(data[4:8]) != "ftyp" {
		t.Fatalf("not an mp4: % x", data[:min(8, len(data))])
	}
}

func TestPrepareRemuxesCompatibleMKV(t *testing.T) {
	root := t.TempDir()
	src := makeMKV(t, root, "show.mkv", "libx264", "")
	tr := New(filepath.Join(root, "cache"))
	if err := tr.Health(); err != nil {
		t.Fatalf("health: %v", err)
	}

	got := prepare(t, tr, src)
	if got.Method != "remux" {
		t.Fatalf("method=%q, want remux for h264-only mkv", got.Method)
	}
	if got.Cached {
		t.Fatal("first call must prepare, not serve cache")
	}
	assertMP4(t, got.Path)

	again := prepare(t, tr, src)
	if cached := again; !cached.Cached || cached.Path != got.Path {
		t.Fatalf("expected cache hit on the same path, got %+v", cached)
	}
}

func TestPrepareTranscodesIncompatibleStreams(t *testing.T) {
	root := t.TempDir()
	// mpeg4 video + mp3 audio: neither survives into a browser MP4.
	src := makeMKV(t, root, "old.mkv", "mpeg4", "libmp3lame")
	tr := New(filepath.Join(root, "cache"))

	got := prepare(t, tr, src)
	if got.Method != "transcode" {
		t.Fatalf("method=%q, want transcode for mpeg4/mp3 mkv", got.Method)
	}
	assertMP4(t, got.Path)
}

func TestPrepareRejectsBadInput(t *testing.T) {
	tr := New(filepath.Join(t.TempDir(), "cache"))
	if _, err := tr.Invoke(contracts.CapPlaybackTranscode, contracts.TranscodeRequest{}); err == nil {
		t.Fatal("empty file_path must fail")
	}
	if _, err := tr.Invoke("lain.playback.other@1", contracts.TranscodeRequest{FilePath: "x"}); err == nil {
		t.Fatal("wrong capability must fail")
	}
	if _, err := tr.Invoke(contracts.CapPlaybackTranscode, "not-a-request"); err == nil {
		t.Fatal("wrong input type must fail")
	}
	if _, err := tr.Invoke(contracts.CapPlaybackTranscode,
		contracts.TranscodeRequest{FilePath: filepath.Join(t.TempDir(), "missing.mkv")}); err == nil {
		t.Fatal("missing source must fail")
	}
}

func TestHealthTracksFFmpeg(t *testing.T) {
	tr := New(filepath.Join(t.TempDir(), "cache"))
	_, lookErr := exec.LookPath("ffmpeg")
	if got := tr.Health(); (lookErr == nil) != (got == nil) {
		t.Fatalf("health=%v, ffmpeg lookup=%v", got, lookErr)
	}
}

func TestAsyncLifecycleDeduplicatesWork(t *testing.T) {
	root := t.TempDir()
	src := filepath.Join(root, "show.mkv")
	if err := os.WriteFile(src, []byte("source"), 0o600); err != nil {
		t.Fatal(err)
	}
	started := make(chan struct{})
	release := make(chan struct{})
	var calls atomic.Int32
	tr := newWithDeps(filepath.Join(root, "cache"), Config{MaxCacheBytes: 1024, QueueSize: 2}, func(_ sourceSpec, out string, _ func(progressSample)) (string, error) {
		if calls.Add(1) == 1 {
			close(started)
		}
		<-release
		return "transcode", os.WriteFile(out, []byte("mp4"), 0o600)
	}, time.Now)
	t.Cleanup(func() { _ = tr.Close() })

	inspect, err := tr.Invoke(contracts.CapPlaybackTranscodeV2, contracts.TranscodeV2Request{
		Action: contracts.TranscodeInspectAction, FilePath: src,
	})
	if err != nil {
		t.Fatal(err)
	}
	idle := inspect.(contracts.TranscodeStatus)
	if idle.State != contracts.TranscodeIdle || idle.Session == "" {
		t.Fatalf("inspect=%+v, want idle session", idle)
	}

	for range 2 {
		out, err := tr.Invoke(contracts.CapPlaybackTranscodeV2, contracts.TranscodeV2Request{
			Action: contracts.TranscodeStartAction, FilePath: src, Session: idle.Session,
		})
		if err != nil {
			t.Fatal(err)
		}
		state := out.(contracts.TranscodeStatus).State
		if state != contracts.TranscodeQueued && state != contracts.TranscodeRunning {
			t.Fatalf("start state=%s", state)
		}
	}
	<-started
	close(release)
	ready := waitState(t, tr, src, idle.Session, contracts.TranscodeReady)
	if ready.Path == "" || ready.Method != "transcode" {
		t.Fatalf("ready=%+v", ready)
	}
	if got := calls.Load(); got != 1 {
		t.Fatalf("convert calls=%d, want 1", got)
	}
}

func TestAsyncQueueIsBounded(t *testing.T) {
	root := t.TempDir()
	block := make(chan struct{})
	tr := newWithDeps(filepath.Join(root, "cache"), Config{MaxCacheBytes: 1024, QueueSize: 1, MaxConcurrent: 1}, func(_ sourceSpec, out string, _ func(progressSample)) (string, error) {
		<-block
		return "transcode", os.WriteFile(out, []byte("mp4"), 0o600)
	}, time.Now)
	t.Cleanup(func() {
		select {
		case <-block:
		default:
			close(block)
		}
		_ = tr.Close()
	})

	start := func(name string) (contracts.TranscodeStatus, error) {
		src := filepath.Join(root, name)
		if err := os.WriteFile(src, []byte(name), 0o600); err != nil {
			t.Fatal(err)
		}
		out, err := tr.Invoke(contracts.CapPlaybackTranscodeV2, contracts.TranscodeV2Request{
			Action: contracts.TranscodeStartAction, FilePath: src,
		})
		if err != nil {
			return contracts.TranscodeStatus{}, err
		}
		return out.(contracts.TranscodeStatus), nil
	}
	first, err := start("one.mkv")
	if err != nil {
		t.Fatal(err)
	}
	waitState(t, tr, filepath.Join(root, "one.mkv"), first.Session, contracts.TranscodeRunning)
	if _, err := start("two.mkv"); err != nil {
		t.Fatalf("one queued job should fit: %v", err)
	}
	if _, err := start("three.mkv"); err == nil {
		t.Fatal("third job must fail when one is running and one is queued")
	} else {
		var ce *core.Error
		if !errors.As(err, &ce) || ce.Code != "queue-full" {
			t.Fatalf("error=%v, want queue-full", err)
		}
	}
	close(block)
}

func TestCacheEvictsLeastRecentlyUsed(t *testing.T) {
	root := t.TempDir()
	now := time.Unix(100, 0)
	tr := newWithDeps(filepath.Join(root, "cache"), Config{MaxCacheBytes: 5, QueueSize: 2}, func(_ sourceSpec, out string, _ func(progressSample)) (string, error) {
		return "transcode", os.WriteFile(out, []byte("123"), 0o600)
	}, func() time.Time { return now })
	t.Cleanup(func() { _ = tr.Close() })

	makeOne := func(name string) contracts.Transcode {
		src := filepath.Join(root, name)
		if err := os.WriteFile(src, []byte(name), 0o600); err != nil {
			t.Fatal(err)
		}
		return prepare(t, tr, src)
	}
	first := makeOne("one.mkv")
	// Move past the recent-access grace so the older session is evictable.
	now = now.Add(2 * time.Minute)
	second := makeOne("two.mkv")
	if _, err := os.Stat(first.Path); !os.IsNotExist(err) {
		t.Fatalf("oldest artifact still present: %v", err)
	}
	if _, err := os.Stat(second.Path); err != nil {
		t.Fatalf("newest artifact removed: %v", err)
	}
}

// TestCacheKeepsRecentlyWatchedSession pins the eviction guard: a session a
// viewer touched recently is not deleted under them even when the cache is
// over budget, and becomes evictable again once it goes idle past the grace.
func TestCacheKeepsRecentlyWatchedSession(t *testing.T) {
	root := t.TempDir()
	now := time.Unix(1000, 0)
	tr := newWithDeps(filepath.Join(root, "cache"), Config{MaxCacheBytes: 1, QueueSize: 2}, func(_ sourceSpec, out string, _ func(progressSample)) (string, error) {
		return "transcode", os.WriteFile(out, []byte("12345"), 0o600)
	}, func() time.Time { return now })
	t.Cleanup(func() { _ = tr.Close() })

	src := filepath.Join(root, "one.mkv")
	if err := os.WriteFile(src, []byte("one"), 0o600); err != nil {
		t.Fatal(err)
	}
	one := prepare(t, tr, src)
	if err := tr.cleanup(false, nil); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(one.Path); err != nil {
		t.Fatalf("a recently watched session was evicted: %v", err)
	}
	now = now.Add(2 * time.Minute)
	if err := tr.cleanup(false, nil); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(one.Path); !os.IsNotExist(err) {
		t.Fatalf("an idle over-budget session was kept: %v", err)
	}
}

func TestV2ConversionSelectsOneAudioStream(t *testing.T) {
	report := mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "h264", PixelFormat: "yuv420p"},
		{Index: 1, CodecType: "audio", CodecName: "ac3", Default: 1},
		{Index: 2, CodecType: "audio", CodecName: "aac"},
	}}
	spec := sourceSpec{Path: "/media/show.mkv", Profile: profile}
	args, method, err := conversionArgs(spec, report, "/cache/out.mp4.tmp")
	if err != nil {
		t.Fatal(err)
	}
	joined := strings.Join(args, " ")
	if !strings.Contains(joined, "-map 0:0 -map 0:1") || strings.Contains(joined, "0:2") {
		t.Fatalf("default audio selection argv=%s", joined)
	}
	if !strings.Contains(joined, "-c:v copy") || !strings.Contains(joined, "-c:a aac") || method != "transcode" {
		t.Fatalf("partial transcode argv=%s method=%s", joined, method)
	}

	selected := 2
	spec.AudioStream = &selected
	args, method, err = conversionArgs(spec, report, "/cache/out.mp4.tmp")
	if err != nil {
		t.Fatal(err)
	}
	joined = strings.Join(args, " ")
	if !strings.Contains(joined, "-map 0:2") || method != "remux" || !strings.Contains(joined, "-c:a copy") {
		t.Fatalf("selected AAC argv=%s method=%s", joined, method)
	}
}

func TestV2ConversionRejectsHDR(t *testing.T) {
	_, _, err := conversionArgs(sourceSpec{Path: "/media/hdr.mkv", Profile: profile}, mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "hevc", PixelFormat: "yuv420p10le", ColorTransfer: "smpte2084"},
	}}, "/cache/out.mp4.tmp")
	var ce *core.Error
	if !errors.As(err, &ce) || ce.Code != "unsupported-media" {
		t.Fatalf("error=%v, want unsupported-media", err)
	}
}

func TestV2ExtractsSubtitleSidecar(t *testing.T) {
	requireFFmpeg(t)
	root := t.TempDir()
	srt := filepath.Join(root, "subs.srt")
	if err := os.WriteFile(srt, []byte("1\n00:00:00,000 --> 00:00:01,000\nHello\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	src := filepath.Join(root, "subbed.mkv")
	mux := []string{"-hide_banner", "-loglevel", "error",
		"-f", "lavfi", "-i", "testsrc=size=160x120:rate=10:duration=2",
		"-i", srt, "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:s", "srt", "-y", src}
	if combo, err := exec.Command("ffmpeg", mux...).CombinedOutput(); err != nil {
		t.Skipf("cannot synthesize subbed mkv: %v: %s", err, combo)
	}
	tr := New(filepath.Join(root, "cache"))
	t.Cleanup(func() { _ = tr.Close() })

	subtitle := 1
	start, err := tr.Invoke(contracts.CapPlaybackTranscodeV2, contracts.TranscodeV2Request{
		Action: contracts.TranscodeStartAction, FilePath: src, SubtitleStream: &subtitle,
	})
	if err != nil {
		t.Fatalf("start: %v", err)
	}
	ready := waitState(t, tr, src, start.(contracts.TranscodeStatus).Session, contracts.TranscodeReady)
	if ready.SubtitlePath == "" {
		t.Fatalf("ready=%+v, want subtitle path", ready)
	}
	raw, err := os.ReadFile(ready.SubtitlePath)
	if err != nil || len(raw) < 6 || string(raw[:6]) != "WEBVTT" {
		t.Fatalf("subtitle=%q err=%v, want WEBVTT", raw, err)
	}
}

// TestConsumeProgressReportsFractions pins the block semantics: ffmpeg
// writes one block per progress interval and ends it with `progress=`,
// so each position is reported once (out_time_us and out_time_ms carry
// the same value) and progress=end reports completion.
func TestConsumeProgressReportsFractions(t *testing.T) {
	var got []float64
	consumeProgress(strings.NewReader(strings.Join([]string{
		"out_time_us=5000000",
		"out_time_ms=5000000",
		"progress=continue",
		"out_time=N/A",
		"out_time_us=60000000",
		"progress=end",
		"",
	}, "\n")), 120, func(sample progressSample) { got = append(got, sample.Fraction) })

	// One report per block: the duplicate key collapses, the unparsable
	// out_time is skipped, and the terminal block reports completion.
	want := []float64{5.0 / 120, 1}
	if len(got) != len(want) {
		t.Fatalf("reports=%v, want %v", got, want)
	}
	for i, w := range want {
		if diff := got[i] - w; diff > 1e-9 || diff < -1e-9 {
			t.Fatalf("report %d = %v, want %v", i, got[i], w)
		}
	}
}

// TestConsumeProgressCarriesLiveMetrics covers the session-list metrics:
// fps, encoding speed and output bitrate arrive in the same block as the
// position and must reach the caller without inventing a fraction.
func TestConsumeProgressCarriesLiveMetrics(t *testing.T) {
	var got []progressSample
	consumeProgress(strings.NewReader(strings.Join([]string{
		"bitrate=3000.5kbits/s",
		"fps=42.5",
		"speed=1.35x",
		"out_time_us=6000000",
		"progress=continue",
		"",
	}, "\n")), 0, func(sample progressSample) { got = append(got, sample) })

	if len(got) != 1 {
		t.Fatalf("reports=%v, want one block", got)
	}
	if got[0].HasFraction {
		t.Fatalf("fraction=%v, want none without a probed duration", got[0].Fraction)
	}
	if got[0].FPS != 42.5 || got[0].BitrateKbps != 3001 || got[0].Speed != 1.35 {
		// 3000.5 kbit/s rounds to a whole kbit/s for reporting.
		t.Fatalf("sample=%+v, want the live metrics", got[0])
	}
}

func TestConsumeProgressWithoutDurationStaysIndeterminate(t *testing.T) {
	reports := 0
	consumeProgress(strings.NewReader("out_time_us=5000000\nprogress=continue\n"), 0, func(progressSample) { reports++ })
	if reports != 0 {
		t.Fatalf("reports=%d, want none without a probed duration", reports)
	}
}

func TestAsyncStatusReportsProgress(t *testing.T) {
	root := t.TempDir()
	src := filepath.Join(root, "show.mkv")
	if err := os.WriteFile(src, []byte("source"), 0o600); err != nil {
		t.Fatal(err)
	}
	release := make(chan struct{})
	var released atomic.Bool
	tr := newWithDeps(filepath.Join(root, "cache"), Config{MaxCacheBytes: 1024, QueueSize: 2}, func(_ sourceSpec, out string, onProgress func(progressSample)) (string, error) {
		onProgress(progressSample{Fraction: 0.25, HasFraction: true, FPS: 42.5, BitrateKbps: 3000})
		<-release
		return "transcode", os.WriteFile(out, []byte("mp4"), 0o600)
	}, time.Now)
	t.Cleanup(func() {
		if released.CompareAndSwap(false, true) {
			close(release)
		}
		_ = tr.Close()
	})

	out, err := tr.Invoke(contracts.CapPlaybackTranscodeV2, contracts.TranscodeV2Request{
		Action: contracts.TranscodeInspectAction, FilePath: src,
	})
	if err != nil {
		t.Fatal(err)
	}
	session := out.(contracts.TranscodeStatus).Session
	if _, err := tr.Invoke(contracts.CapPlaybackTranscodeV2, contracts.TranscodeV2Request{
		Action: contracts.TranscodeStartAction, FilePath: src, Session: session,
	}); err != nil {
		t.Fatal(err)
	}

	deadline := time.Now().Add(2 * time.Second)
	for {
		out, err := tr.Invoke(contracts.CapPlaybackTranscodeV2, contracts.TranscodeV2Request{
			Action: contracts.TranscodeStatusAction, FilePath: src, Session: session,
		})
		if err != nil {
			t.Fatal(err)
		}
		status := out.(contracts.TranscodeStatus)
		if status.Progress > 0 {
			if status.Progress != 0.25 {
				t.Fatalf("progress=%v, want 0.25 while running", status.Progress)
			}
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("progress never surfaced: %+v", status)
		}
		time.Sleep(5 * time.Millisecond)
	}

	released.Store(true)
	close(release)
	ready := waitState(t, tr, src, session, contracts.TranscodeReady)
	if ready.Progress != 0 {
		t.Fatalf("terminal status carries progress=%v, want none", ready.Progress)
	}
}
