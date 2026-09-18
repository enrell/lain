package transcode

import (
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
)

// subsRequireFFmpeg skips unless both ffmpeg and ffprobe are installed:
// on-demand extraction probes the source and then converts the track.
func subsRequireFFmpeg(t *testing.T) {
	t.Helper()
	if _, err := exec.LookPath("ffmpeg"); err != nil {
		t.Skip("ffmpeg not installed")
	}
	if _, err := exec.LookPath("ffprobe"); err != nil {
		t.Skip("ffprobe not installed")
	}
}

// subsFixture builds a transcoder over a real (tiny) source file with
// the shipped defaults; the probe/capability seams stay for the caller.
func subsFixture(t *testing.T) (*Transcoder, string) {
	t.Helper()
	root := t.TempDir()
	src := filepath.Join(root, "[Fansub-A] Subbed Show.mkv")
	if err := os.WriteFile(src, []byte("source"), 0o600); err != nil {
		t.Fatal(err)
	}
	tr := newWithDeps(filepath.Join(root, "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	return tr, src
}

// subsWithProbe injects a fixed probe report and capability surface so
// the refusal paths are testable without ffmpeg or real media.
func subsWithProbe(t *testing.T, report mediaReport) (*Transcoder, string) {
	t.Helper()
	tr, src := subsFixture(t)
	tr.probeFn = func(string, string) (mediaReport, error) { return report, nil }
	tr.capsFn = func(contracts.TranscodeSettings) capabilities { return testCaps() }
	return tr, src
}

// subsRequest builds one subtitle-extraction request with the shipped
// settings.
func subsRequest(src string, index int) contracts.TranscodeV3Request {
	return contracts.TranscodeV3Request{
		Action:         contracts.TranscodeSubtitleAction,
		FilePath:       src,
		SubtitleStream: &index,
		Settings:       contracts.DefaultTranscodeSettings(),
	}
}

// subsInvoke calls the subtitles action and returns the status and the
// error without failing: the error code is what some tests assert.
func subsInvoke(t *testing.T, tr *Transcoder, in contracts.TranscodeV3Request) (contracts.TranscodeV3Status, error) {
	t.Helper()
	out, err := tr.Invoke(contracts.CapPlaybackTranscodeV3, in)
	if err != nil {
		return contracts.TranscodeV3Status{}, err
	}
	status, ok := out.(contracts.TranscodeV3Status)
	if !ok {
		t.Fatalf("subtitles returned %T", out)
	}
	return status, nil
}

// subsMKV synthesizes a tiny H.264 Matroska carrying one SRT track, the
// shape the on-demand extraction path exists for.
func subsMKV(t *testing.T, dir, name string) string {
	t.Helper()
	subsRequireFFmpeg(t)
	srt := filepath.Join(dir, "subs.srt")
	if err := os.WriteFile(srt, []byte("1\n00:00:00,000 --> 00:00:01,000\nHello from [Fansub-A]\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	out := filepath.Join(dir, name)
	args := []string{"-hide_banner", "-loglevel", "error",
		"-f", "lavfi", "-i", "testsrc=size=160x120:rate=10:duration=1",
		"-i", srt,
		"-map", "0:v", "-map", "1:s",
		"-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:s", "srt",
		"-y", out}
	if combo, err := exec.Command("ffmpeg", args...).CombinedOutput(); err != nil {
		t.Skipf("cannot synthesize subbed mkv: %v: %s", err, combo)
	}
	return out
}

// TestSubtitlesActionForbiddenWhenDisabled pins the operator kill
// switch: an explicit allow_subtitle_extraction=false refuses with
// "forbidden" before any probe or ffmpeg work happens.
func TestSubtitlesActionForbiddenWhenDisabled(t *testing.T) {
	tr, src := subsFixture(t)
	probes := 0
	tr.probeFn = func(string, string) (mediaReport, error) {
		probes++
		return v3Report(), nil
	}
	tr.capsFn = func(contracts.TranscodeSettings) capabilities { return testCaps() }

	settings := contracts.DefaultTranscodeSettings()
	off := false
	settings.AllowSubtitleExtraction = &off
	in := subsRequest(src, 2)
	in.Settings = settings

	_, err := subsInvoke(t, tr, in)
	requireErrorCode(t, err, "forbidden")
	if probes != 0 {
		t.Fatalf("probe ran %d times, want none when extraction is disabled", probes)
	}
}

// TestSubtitlesActionValidatesInput pins the three malformed shapes:
// no source, no selected stream, and an index that is not a subtitle.
func TestSubtitlesActionValidatesInput(t *testing.T) {
	t.Run("missing file_path", func(t *testing.T) {
		tr, _ := subsWithProbe(t, v3Report())
		_, err := subsInvoke(t, tr, subsRequest("", 2))
		requireErrorCode(t, err, "invalid-message")
	})
	t.Run("missing subtitle_stream", func(t *testing.T) {
		tr, src := subsWithProbe(t, v3Report())
		in := subsRequest(src, 2)
		in.SubtitleStream = nil
		_, err := subsInvoke(t, tr, in)
		requireErrorCode(t, err, "invalid-message")
	})
	t.Run("index is a video track", func(t *testing.T) {
		tr, src := subsWithProbe(t, v3Report())
		_, err := subsInvoke(t, tr, subsRequest(src, 0))
		requireErrorCode(t, err, "invalid-message")
	})
	t.Run("index is an audio track", func(t *testing.T) {
		tr, src := subsWithProbe(t, v3Report())
		_, err := subsInvoke(t, tr, subsRequest(src, 1))
		requireErrorCode(t, err, "invalid-message")
	})
}

// TestSubtitlesActionRejectsImageSubtitle pins the honest refusal: a
// bitmap track cannot become WebVTT.
func TestSubtitlesActionRejectsImageSubtitle(t *testing.T) {
	report := mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "h264", PixelFormat: "yuv420p", Width: 1920, Height: 1080},
		{Index: 1, CodecType: "subtitle", CodecName: "hdmv_pgs_subtitle"},
	}}
	tr, src := subsWithProbe(t, report)
	_, err := subsInvoke(t, tr, subsRequest(src, 1))
	requireErrorCode(t, err, "unsupported-media")
}

// TestSubtitlesActionExtractsAndCachesWebVTT drives the real ffmpeg
// path: one SRT track becomes a cached WebVTT sidecar, and the second
// request is served from the cache without rewriting the artifact.
func TestSubtitlesActionExtractsAndCachesWebVTT(t *testing.T) {
	subsRequireFFmpeg(t)
	root := t.TempDir()
	src := subsMKV(t, root, "[Fansub-A] Subbed Show.mkv")
	tr := newWithDeps(filepath.Join(root, "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })

	settings := contracts.DefaultTranscodeSettings()
	report, err := tr.probeReport(settings, src)
	if err != nil {
		t.Fatalf("probe: %v", err)
	}
	index := -1
	for _, s := range report.Streams {
		if s.CodecType == "subtitle" {
			index = s.Index
		}
	}
	if index < 0 {
		t.Fatalf("synthesized mkv has no subtitle track: %+v", report.Streams)
	}

	first, err := subsInvoke(t, tr, subsRequest(src, index))
	if err != nil {
		t.Fatalf("subtitles: %v", err)
	}
	if first.Delivery != contracts.TranscodeDeliverySubtitle || first.Playable || first.Method != "extract" {
		t.Fatalf("status=%+v, want a non-playable subtitle extraction", first)
	}
	if first.SubtitlePath == "" || first.Path != "" || first.PlaylistPath != "" {
		t.Fatalf("paths=%+v, want only the sidecar", first)
	}
	raw, err := os.ReadFile(first.SubtitlePath)
	if err != nil || !strings.HasPrefix(string(raw), "WEBVTT") {
		t.Fatalf("sidecar=%q err=%v, want WEBVTT", raw, err)
	}
	before, err := os.Stat(first.SubtitlePath)
	if err != nil {
		t.Fatal(err)
	}

	second, err := subsInvoke(t, tr, subsRequest(src, index))
	if err != nil {
		t.Fatalf("cached subtitles: %v", err)
	}
	if second.Session != first.Session || !second.Cached {
		t.Fatalf("second=%+v, want the cached session %q", second, first.Session)
	}
	after, err := os.Stat(second.SubtitlePath)
	if err != nil {
		t.Fatal(err)
	}
	if !after.ModTime().Equal(before.ModTime()) {
		t.Fatalf("cache hit rewrote the sidecar: %v -> %v", before.ModTime(), after.ModTime())
	}
}

// TestSubtitleSidecarSurvivesCleanupAndCancel pins the sidecar-only
// lifecycle: the .vtt is the artifact cleanup must keep, and cancelling
// the session drops the sidecar and its JSON together.
func TestSubtitleSidecarSurvivesCleanupAndCancel(t *testing.T) {
	tr, src := subsFixture(t)
	fi, err := os.Stat(src)
	if err != nil {
		t.Fatal(err)
	}
	index := 2
	spec := sourceSpec{
		Path: src, Size: fi.Size(), ModTimeNS: fi.ModTime().UnixNano(),
		Profile: subtitleProfile, SubtitleStream: &index,
	}
	spec.Session = sessionKey(spec)
	paths := tr.pathsFor(spec.Session, contracts.TranscodeSettings{})

	const vtt = "WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nHello from [Fansub-A]\n"
	writeTestFile(t, paths.subtitle, vtt)
	now := tr.now().Unix()
	entry := cacheEntry{
		Session: spec.Session, SourcePath: src, SourceSize: fi.Size(), SourceModTime: fi.ModTime().UnixNano(),
		Profile: subtitleProfile, Delivery: contracts.TranscodeDeliverySubtitle, SubtitleStream: &index,
		HasSubtitle: true, Method: "extract", Size: int64(len(vtt)), CreatedAt: now, AccessedAt: now,
	}
	if err := writeJSONAtomic(tr.metaPath(spec.Session), entry); err != nil {
		t.Fatal(err)
	}

	// A sidecar-only entry has exactly one artifact; cleanup must not
	// treat the missing media file as staleness.
	if err := tr.cleanup(false, nil); err != nil {
		t.Fatalf("cleanup: %v", err)
	}
	if _, err := os.Stat(paths.subtitle); err != nil {
		t.Fatalf("cleanup dropped the sidecar: %v", err)
	}
	if _, err := os.Stat(tr.metaPath(spec.Session)); err != nil {
		t.Fatalf("cleanup dropped the sidecar entry: %v", err)
	}
	if _, ok := tr.readyEntry(spec); !ok {
		t.Fatal("sidecar-only entry is not resolvable after cleanup")
	}
	status := tr.statusV3FromEntry(entry)
	if status.Playable || status.Method != "extract" || status.Path != "" || status.PlaylistPath != "" || status.SubtitlePath == "" {
		t.Fatalf("projection=%+v, want a non-playable extract with only the sidecar", status)
	}

	tr.cancelV3(spec.Session, "", "")
	if _, err := os.Stat(paths.subtitle); !os.IsNotExist(err) {
		t.Fatalf("cancel kept the sidecar: %v", err)
	}
	if _, err := os.Stat(tr.metaPath(spec.Session)); !os.IsNotExist(err) {
		t.Fatalf("cancel kept the sidecar entry: %v", err)
	}
}

// TestSubtitleProfileKeyIsDistinctFromSessions pins D-030/D-045: the
// sidecar profile must never collide with a transcode session, and the
// selected subtitle stream index must change the identity.
func TestSubtitleProfileKeyIsDistinctFromSessions(t *testing.T) {
	settings := contracts.DefaultTranscodeSettings()
	first, second := 2, 3
	req := contracts.TranscodeV3Request{
		Action: contracts.TranscodeSubtitleAction, FilePath: "/media/[Fansub-A] Show.mkv",
		SubtitleStream: &first, Settings: settings,
	}
	key := contracts.TranscodeProfileKey(settings, req)
	if key == subtitleProfile {
		t.Fatalf("transcode profile %q collides with the sidecar profile %q", key, subtitleProfile)
	}

	// The sidecar session and the transcode session of the same source
	// must be different cache entries.
	spec := sourceSpec{Path: req.FilePath, Size: 1, ModTimeNS: 2, Profile: subtitleProfile, SubtitleStream: &first}
	sidecarSession := sessionKey(spec)
	spec.Profile = key
	if sessionKey(spec) == sidecarSession {
		t.Fatal("sidecar and transcode sessions collide")
	}

	// Changing the selected stream changes both identities.
	other := req
	other.SubtitleStream = &second
	if contracts.TranscodeProfileKey(settings, other) == key {
		t.Fatal("profile key ignored the subtitle stream index")
	}
	spec.SubtitleStream = &second
	if sessionKey(spec) == sidecarSession {
		t.Fatal("sidecar session ignored the subtitle stream index")
	}
}

// TestAllowStreamCopyDefaults pins the nil-means-allowed contract.
func TestAllowStreamCopyDefaults(t *testing.T) {
	if !contracts.AllowStreamCopy(nil) {
		t.Fatal("nil must allow stream copy")
	}
	yes, no := true, false
	if !contracts.AllowStreamCopy(&yes) {
		t.Fatal("an explicit true must allow stream copy")
	}
	if contracts.AllowStreamCopy(&no) {
		t.Fatal("an explicit false must forbid stream copy")
	}
}

// TestPlanStreamCopyFlagsForceEncode pins the client-side permissions:
// an explicit false re-encodes that stream and leaves the other alone;
// nil or true keep the remux fast path.
func TestPlanStreamCopyFlagsForceEncode(t *testing.T) {
	plan, err := planFor(t, webSafeReport(), testCaps(), nil)
	if err != nil {
		t.Fatalf("plan: %v", err)
	}
	if plan.method != "remux" || !plan.copyVideo || !plan.copyAudio {
		t.Fatalf("default plan=%+v, want a full remux", plan)
	}

	no, yes := false, true
	video, err := planFor(t, webSafeReport(), testCaps(), func(in *contracts.TranscodeV3Request) {
		in.AllowVideoStreamCopy = &no
	})
	if err != nil {
		t.Fatalf("video plan: %v", err)
	}
	if video.copyVideo || !video.copyAudio || video.method != "transcode" {
		t.Fatalf("video plan copyVideo=%v copyAudio=%v method=%q, want a video re-encode", video.copyVideo, video.copyAudio, video.method)
	}

	audio, err := planFor(t, webSafeReport(), testCaps(), func(in *contracts.TranscodeV3Request) {
		in.AllowAudioStreamCopy = &no
	})
	if err != nil {
		t.Fatalf("audio plan: %v", err)
	}
	if audio.copyAudio || !audio.copyVideo || audio.method != "transcode" {
		t.Fatalf("audio plan copyVideo=%v copyAudio=%v method=%q, want an audio re-encode", audio.copyVideo, audio.copyAudio, audio.method)
	}

	both, err := planFor(t, webSafeReport(), testCaps(), func(in *contracts.TranscodeV3Request) {
		in.AllowVideoStreamCopy = &yes
		in.AllowAudioStreamCopy = &yes
	})
	if err != nil {
		t.Fatalf("both plan: %v", err)
	}
	if both.method != "remux" || !both.copyVideo || !both.copyAudio {
		t.Fatalf("both plan=%+v, want the remux path kept", both)
	}
}

// TestStreamCopyRefusalAddsReason pins the note resolveVideo and
// resolveAudio attach when the client forbids a copy. planV3 currently
// ends with `plan.reasons = plan.transcodeReasons(in)`, which replaces
// that slice, so the note never reaches the status reasons (reported as
// a source bug); this keeps the unit behavior itself pinned.
func TestStreamCopyRefusalAddsReason(t *testing.T) {
	settings := contracts.DefaultTranscodeSettings()
	no := false

	report := webSafeReport()
	video := encodePlan{spec: testSource(), settings: settings, report: report, video: &report.Streams[0]}
	if err := video.resolveVideo(contracts.TranscodeV3Request{AllowVideoStreamCopy: &no}, testCaps()); err != nil {
		t.Fatalf("resolveVideo: %v", err)
	}
	if video.copyVideo {
		t.Fatal("resolveVideo copied despite the client flag")
	}
	if !slices.Contains(video.reasons, "video stream copy disabled by the client") {
		t.Fatalf("reasons=%v, want the stream-copy note", video.reasons)
	}

	audioReport := webSafeReport()
	audio := encodePlan{spec: testSource(), settings: settings, report: audioReport, audio: &audioReport.Streams[1]}
	audio.resolveAudio(contracts.TranscodeV3Request{AllowAudioStreamCopy: &no}, settings)
	if audio.copyAudio {
		t.Fatal("resolveAudio copied despite the client flag")
	}
	if !slices.Contains(audio.reasons, "audio stream copy disabled by the client") {
		t.Fatalf("reasons=%v, want the stream-copy note", audio.reasons)
	}
}

// TestPlanDeinterlaceMethodSelection pins the method choice and the
// visible bwdif->yadif fallback when the build lacks bwdif.
func TestPlanDeinterlaceMethodSelection(t *testing.T) {
	interlaced := webSafeReport()
	interlaced.Streams[0].FieldOrder = "tt"

	def, err := planFor(t, interlaced, testCaps(), nil)
	if err != nil {
		t.Fatalf("default plan: %v", err)
	}
	if !slices.Contains(def.filters, contracts.DeinterlaceYadif) {
		t.Fatalf("filters=%v, want yadif as the shipped default", def.filters)
	}

	withBwdif := testCaps()
	withBwdif.Filters[contracts.DeinterlaceBwdif] = true
	chosen, err := planFor(t, interlaced, withBwdif, func(in *contracts.TranscodeV3Request) {
		in.Settings.DeinterlaceMethod = contracts.DeinterlaceBwdif
	})
	if err != nil {
		t.Fatalf("bwdif plan: %v", err)
	}
	if !slices.Contains(chosen.filters, contracts.DeinterlaceBwdif) {
		t.Fatalf("filters=%v, want bwdif when the build has it", chosen.filters)
	}
	if strings.Contains(chosen.fallback, "bwdif") {
		t.Fatalf("fallback=%q, want no fallback when bwdif is available", chosen.fallback)
	}

	fallback, err := planFor(t, interlaced, testCaps(), func(in *contracts.TranscodeV3Request) {
		in.Settings.DeinterlaceMethod = contracts.DeinterlaceBwdif
	})
	if err != nil {
		t.Fatalf("fallback plan: %v", err)
	}
	if slices.Contains(fallback.filters, contracts.DeinterlaceBwdif) || !slices.Contains(fallback.filters, contracts.DeinterlaceYadif) {
		t.Fatalf("filters=%v, want yadif when bwdif is missing", fallback.filters)
	}
	if !strings.Contains(fallback.fallback, "bwdif") {
		t.Fatalf("fallback=%q, want the missing-bwdif note", fallback.fallback)
	}
}

// TestStatusV3DirectFlagsFromPlan pins the per-stream direct flags the
// player sees: they mirror the plan's copy decisions.
func TestStatusV3DirectFlagsFromPlan(t *testing.T) {
	tr, _ := subsFixture(t)
	settings := contracts.DefaultTranscodeSettings()

	direct, err := planFor(t, webSafeReport(), testCaps(), nil)
	if err != nil {
		t.Fatalf("direct plan: %v", err)
	}
	j := &job{
		spec:     sourceSpec{Session: "sess", Profile: direct.profile},
		state:    contracts.TranscodeReady,
		delivery: contracts.TranscodeDeliveryProgressive,
		method:   direct.method, plan: &direct, settings: settings,
	}
	got := tr.statusV3FromJob(j)
	if !got.VideoDirect || !got.AudioDirect {
		t.Fatalf("direct status video=%v audio=%v, want both direct", got.VideoDirect, got.AudioDirect)
	}

	encoded, err := planFor(t, mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "mpeg4", PixelFormat: "yuv420p", Width: 640, Height: 480},
		{Index: 1, CodecType: "audio", CodecName: "mp3", Channels: 2},
	}}, testCaps(), nil)
	if err != nil {
		t.Fatalf("encoded plan: %v", err)
	}
	j.plan = &encoded
	got = tr.statusV3FromJob(j)
	if got.VideoDirect || got.AudioDirect {
		t.Fatalf("encoded status video=%v audio=%v, want both re-encoded", got.VideoDirect, got.AudioDirect)
	}
}
