package transcode


import (
	"errors"
	"path/filepath"
	"reflect"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

// planFixture builds a transcoder with injected probe/capability seams:
// policy decisions must be testable without ffmpeg or real media.
func planFixture(t *testing.T, report mediaReport, caps capabilities) *Transcoder {
	t.Helper()
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	tr.probeFn = func(string, string) (mediaReport, error) { return report, nil }
	tr.capsFn = func(contracts.TranscodeSettings) capabilities { return caps }
	return tr
}

func testSource() sourceSpec {
	return sourceSpec{Path: "/media/[Fansub-A] Show.mkv", Profile: "web-mp4-v3-test"}
}

func testRequest(settings contracts.TranscodeSettings) contracts.TranscodeV3Request {
	return contracts.TranscodeV3Request{
		Action:   contracts.TranscodeStartAction,
		FilePath: testSource().Path,
		Settings: settings,
	}
}

func testCaps() capabilities {
	return capabilities{
		Encoders: map[string]bool{
			"libx264": true, "libx265": true, "libsvtav1": true, "libaom-av1": true, "h264_vaapi": true,
		},
		Filters:  map[string]bool{"overlay": true, "subtitles": true, "ass": true, "zscale": true, "tonemap": true},
		Hwaccels: map[string]bool{"vaapi": true},
		Hardware: map[string]bool{},
		ToneMap:  true,
	}
}

func planFor(t *testing.T, report mediaReport, caps capabilities, mutate func(*contracts.TranscodeV3Request)) (encodePlan, error) {
	t.Helper()
	tr := planFixture(t, report, caps)
	in := testRequest(contracts.DefaultTranscodeSettings())
	if mutate != nil {
		mutate(&in)
	}
	return tr.planV3(testSource(), in, tr.capabilitiesFor(in.Settings))
}

func requireErrorCode(t *testing.T, err error, want string) {
	t.Helper()
	if err == nil {
		t.Fatalf("want error code %q, got nil", want)
	}
	var ce *core.Error
	if !errors.As(err, &ce) {
		t.Fatalf("error %v is not a core.Error", err)
	}
	if ce.Code != want {
		t.Fatalf("error code %q (%s), want %q", ce.Code, ce.Msg, want)
	}
}

func webSafeReport() mediaReport {
	return mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "h264", PixelFormat: "yuv420p", Width: 1920, Height: 1080},
		{Index: 1, CodecType: "audio", CodecName: "aac", Channels: 2, Default: 1},
	}}
}

func hdrReport() mediaReport {
	return mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "hevc", PixelFormat: "yuv420p10le",
			ColorTransfer: "smpte2084", Width: 3840, Height: 2160},
	}}
}

func subtitleReport(codec string) mediaReport {
	return mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "h264", PixelFormat: "yuv420p", Width: 1920, Height: 1080},
		{Index: 1, CodecType: "audio", CodecName: "aac", Channels: 2},
		{Index: 2, CodecType: "subtitle", CodecName: codec},
	}}
}

// TestPlanV3RemuxesWebSafeSource pins the fast path: a web-safe stream
// with nothing to change must never be re-encoded.
func TestPlanV3RemuxesWebSafeSource(t *testing.T) {
	plan, err := planFor(t, webSafeReport(), testCaps(), nil)
	if err != nil {
		t.Fatalf("plan: %v", err)
	}
	if !plan.copyVideo || !plan.copyAudio {
		t.Fatalf("copyVideo=%v copyAudio=%v, want both copy", plan.copyVideo, plan.copyAudio)
	}
	if plan.method != "remux" || plan.encoder != "" {
		t.Fatalf("method=%q encoder=%q, want remux with no encoder", plan.method, plan.encoder)
	}
	if want := []string{"container (mkv)"}; !reflect.DeepEqual(plan.reasons, want) {
		t.Fatalf("reasons=%v, want %v", plan.reasons, want)
	}
}

// TestPlanV3EncodesIncompatibleCodecs covers the conservative default:
// non-H.264/non-AAC sources become H.264/AAC through libx264.
func TestPlanV3EncodesIncompatibleCodecs(t *testing.T) {
	report := mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "mpeg4", PixelFormat: "yuv420p", Width: 640, Height: 480},
		{Index: 1, CodecType: "audio", CodecName: "mp3", Channels: 2},
	}}
	plan, err := planFor(t, report, testCaps(), nil)
	if err != nil {
		t.Fatalf("plan: %v", err)
	}
	if plan.copyVideo || plan.copyAudio {
		t.Fatalf("copyVideo=%v copyAudio=%v, want a full encode", plan.copyVideo, plan.copyAudio)
	}
	if plan.encoder != "libx264" || plan.videoCodec != contracts.VideoCodecH264 || plan.method != "transcode" {
		t.Fatalf("encoder=%q codec=%q method=%q", plan.encoder, plan.videoCodec, plan.method)
	}
	for _, want := range []string{"container (mkv)", "video codec (mpeg4)", "audio codec (mp3)"} {
		if !slices.Contains(plan.reasons, want) {
			t.Errorf("reasons=%v, want %q", plan.reasons, want)
		}
	}
}

// TestPlanV3HDRRequiresToneMapping pins the honest HDR path: refusal
// when the operator or the probe cannot tone map, never a washed-out
// silent encode.
func TestPlanV3HDRRequiresToneMapping(t *testing.T) {
	t.Run("disabled in settings", func(t *testing.T) {
		_, err := planFor(t, hdrReport(), testCaps(), func(in *contracts.TranscodeV3Request) {
			in.Settings.ToneMapping = false
		})
		requireErrorCode(t, err, "unsupported-media")
	})
	t.Run("probe failed", func(t *testing.T) {
		caps := testCaps()
		caps.ToneMap = false
		_, err := planFor(t, hdrReport(), caps, nil)
		requireErrorCode(t, err, "unsupported-media")
	})
	t.Run("planned with chain", func(t *testing.T) {
		caps := testCaps()
		caps.ToneMap2390 = true
		plan, err := planFor(t, hdrReport(), caps, nil)
		if err != nil {
			t.Fatalf("plan: %v", err)
		}
		if !plan.needTone {
			t.Fatal("HDR with tone mapping enabled must set needTone")
		}
		joined := strings.Join(plan.filters, ",")
		if !strings.Contains(joined, "libplacebo=tonemapping=bt.2390") {
			t.Fatalf("filters=%v, want the BT.2390 chain", plan.filters)
		}
	})
}

// TestPlanV3ToneMapFallbackIsVisible pins D-031/D-042: an unavailable
// BT.2390 degrades with a note the UI can show, it does not fail.
func TestPlanV3ToneMapFallbackIsVisible(t *testing.T) {
	caps := testCaps()
	caps.ToneMap2390 = false
	plan, err := planFor(t, hdrReport(), caps, nil)
	if err != nil {
		t.Fatalf("plan: %v", err)
	}
	if !strings.Contains(plan.fallback, "bt2390") {
		t.Fatalf("fallback=%q, want a visible bt2390 note", plan.fallback)
	}
	joined := strings.Join(plan.filters, ",")
	if !strings.Contains(joined, "tonemap=tonemap=hable") {
		t.Fatalf("filters=%v, want the hable fallback curve", plan.filters)
	}
}

// TestPlanV3DeinterlaceFollowsSettings proves deinterlacing is a policy
// choice, not an implicit filter.
func TestPlanV3DeinterlaceFollowsSettings(t *testing.T) {
	report := webSafeReport()
	report.Streams[0].FieldOrder = "tt"

	auto, err := planFor(t, report, testCaps(), nil)
	if err != nil {
		t.Fatalf("plan: %v", err)
	}
	if auto.copyVideo {
		t.Fatal("interlaced video must not be copied")
	}
	if !slices.Contains(auto.filters, "yadif") {
		t.Fatalf("filters=%v, want yadif with deinterlace auto", auto.filters)
	}

	off, err := planFor(t, report, testCaps(), func(in *contracts.TranscodeV3Request) {
		in.Settings.Deinterlace = contracts.DeinterlaceOff
	})
	if err != nil {
		t.Fatalf("plan: %v", err)
	}
	if slices.Contains(off.filters, "yadif") {
		t.Fatalf("filters=%v, want no yadif with deinterlace off", off.filters)
	}
	if slices.Contains(off.reasons, "interlaced video") {
		t.Fatalf("reasons=%v, want no interlaced reason when the filter is off", off.reasons)
	}
}

// TestPlanV3QualityCapsResolutionAndBitrate pins the ladder arithmetic:
// preset caps apply, and the stricter per-user limit wins.
func TestPlanV3QualityCapsResolutionAndBitrate(t *testing.T) {
	plan, err := planFor(t, webSafeReport(), testCaps(), func(in *contracts.TranscodeV3Request) {
		in.Quality = "720p"
	})
	if err != nil {
		t.Fatalf("plan: %v", err)
	}
	if plan.width != 1280 || plan.height != 720 {
		t.Fatalf("size=%dx%d, want 1280x720", plan.width, plan.height)
	}
	if plan.maxBitrateKbps != 4000 {
		t.Fatalf("maxBitrateKbps=%d, want the 720p preset 4000", plan.maxBitrateKbps)
	}
	if plan.copyVideo {
		t.Fatal("downscaling must disable the copy fast path")
	}

	userCap, err := planFor(t, webSafeReport(), testCaps(), func(in *contracts.TranscodeV3Request) {
		in.Quality = "720p"
		in.MaxBitrateKbps = 2000
	})
	if err != nil {
		t.Fatalf("plan: %v", err)
	}
	if userCap.maxBitrateKbps != 2000 {
		t.Fatalf("maxBitrateKbps=%d, want the lower per-user cap 2000", userCap.maxBitrateKbps)
	}

	higher, err := planFor(t, webSafeReport(), testCaps(), func(in *contracts.TranscodeV3Request) {
		in.Quality = "720p"
		in.MaxBitrateKbps = 8000
	})
	if err != nil {
		t.Fatalf("plan: %v", err)
	}
	if higher.maxBitrateKbps != 4000 {
		t.Fatalf("maxBitrateKbps=%d, want the stricter preset cap 4000", higher.maxBitrateKbps)
	}
}

// TestPlanV3EnforcesUserPolicy pins D-042 per-user limits: each kind of
// work is refused with the stable "forbidden" code.
func TestPlanV3EnforcesUserPolicy(t *testing.T) {
	t.Run("video transcode forbidden", func(t *testing.T) {
		report := mediaReport{Streams: []stream{
			{Index: 0, CodecType: "video", CodecName: "mpeg4", PixelFormat: "yuv420p", Width: 640, Height: 480},
			{Index: 1, CodecType: "audio", CodecName: "aac", Channels: 2},
		}}
		_, err := planFor(t, report, testCaps(), func(in *contracts.TranscodeV3Request) {
			in.Policy = &contracts.TranscodePolicy{AllowVideoTranscode: false, AllowAudioTranscode: true, AllowRemux: true}
		})
		requireErrorCode(t, err, "forbidden")
	})
	t.Run("remux forbidden", func(t *testing.T) {
		_, err := planFor(t, webSafeReport(), testCaps(), func(in *contracts.TranscodeV3Request) {
			in.Policy = &contracts.TranscodePolicy{AllowVideoTranscode: true, AllowAudioTranscode: true, AllowRemux: false}
		})
		requireErrorCode(t, err, "forbidden")
	})
	t.Run("audio transcode forbidden", func(t *testing.T) {
		report := mediaReport{Streams: []stream{
			{Index: 0, CodecType: "video", CodecName: "h264", PixelFormat: "yuv420p", Width: 1920, Height: 1080},
			{Index: 1, CodecType: "audio", CodecName: "mp3", Channels: 2},
		}}
		_, err := planFor(t, report, testCaps(), func(in *contracts.TranscodeV3Request) {
			in.Policy = &contracts.TranscodePolicy{AllowVideoTranscode: true, AllowAudioTranscode: false, AllowRemux: true}
		})
		requireErrorCode(t, err, "forbidden")
	})
}

// TestPlanV3SubtitleModes covers auto/extract/off against text and
// image tracks, including the missing-overlay refusal.
func TestPlanV3SubtitleModes(t *testing.T) {
	selected := 2

	t.Run("text extracts in auto", func(t *testing.T) {
		plan, err := planFor(t, subtitleReport("subrip"), testCaps(), func(in *contracts.TranscodeV3Request) {
			in.SubtitleStream = &selected
		})
		if err != nil {
			t.Fatalf("plan: %v", err)
		}
		if plan.subtitleMode != contracts.SubtitleModeExtract || plan.burnText || plan.burnImage {
			t.Fatalf("subtitleMode=%q burnText=%v burnImage=%v", plan.subtitleMode, plan.burnText, plan.burnImage)
		}
	})
	t.Run("image burns in auto with overlay", func(t *testing.T) {
		plan, err := planFor(t, subtitleReport("hdmv_pgs_subtitle"), testCaps(), func(in *contracts.TranscodeV3Request) {
			in.SubtitleStream = &selected
		})
		if err != nil {
			t.Fatalf("plan: %v", err)
		}
		if plan.subtitleMode != contracts.SubtitleModeBurn || !plan.burnImage {
			t.Fatalf("subtitleMode=%q burnImage=%v, want burn", plan.subtitleMode, plan.burnImage)
		}
		if !strings.Contains(plan.complexFilter, "overlay") || plan.filters != nil {
			t.Fatalf("complexFilter=%q filters=%v", plan.complexFilter, plan.filters)
		}
	})
	t.Run("image fails without overlay", func(t *testing.T) {
		caps := testCaps()
		delete(caps.Filters, "overlay")
		_, err := planFor(t, subtitleReport("hdmv_pgs_subtitle"), caps, func(in *contracts.TranscodeV3Request) {
			in.SubtitleStream = &selected
		})
		requireErrorCode(t, err, "unsupported-media")
	})
	t.Run("image fails in extract mode", func(t *testing.T) {
		_, err := planFor(t, subtitleReport("hdmv_pgs_subtitle"), testCaps(), func(in *contracts.TranscodeV3Request) {
			in.SubtitleStream = &selected
			in.SubtitleMode = contracts.SubtitleModeExtract
		})
		requireErrorCode(t, err, "unsupported-media")
	})
	t.Run("off selects nothing", func(t *testing.T) {
		plan, err := planFor(t, subtitleReport("subrip"), testCaps(), func(in *contracts.TranscodeV3Request) {
			in.SubtitleStream = &selected
			in.SubtitleMode = contracts.SubtitleModeOff
		})
		if err != nil {
			t.Fatalf("plan: %v", err)
		}
		if plan.subtitle != nil || plan.subtitleMode != contracts.SubtitleModeOff {
			t.Fatalf("subtitle=%+v mode=%q, want none", plan.subtitle, plan.subtitleMode)
		}
	})
}

// TestChooseEncoderHonorsHardwarePolicy pins D-031: hardware is opt-in,
// decode-only offload is still allowed, and an unusable backend is a
// visible software fallback instead of a failed session.
func TestChooseEncoderHonorsHardwarePolicy(t *testing.T) {
	report := mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "h264", PixelFormat: "yuv420p", Width: 1920, Height: 1080},
	}}
	base := contracts.DefaultTranscodeSettings()
	base.HardwareAcceleration = contracts.HWVAAPI
	video := &report.Streams[0]

	t.Run("decode-only offload keeps software encode", func(t *testing.T) {
		settings := base
		settings.HardwareEncode = false
		caps := testCaps()
		caps.Hardware = map[string]bool{contracts.HWVAAPI: true}
		encoder, backend, hwDecode, fallback, err := chooseEncoder(settings, contracts.VideoCodecH264, report, caps, video)
		if err != nil {
			t.Fatal(err)
		}
		if encoder != "libx264" || backend != "" || !hwDecode || fallback != "" {
			t.Fatalf("encoder=%q backend=%q hwDecode=%v fallback=%q", encoder, backend, hwDecode, fallback)
		}
	})
	t.Run("missing hardware encoder falls back visibly", func(t *testing.T) {
		settings := base
		settings.HardwareEncode = true
		caps := testCaps()
		caps.Encoders = map[string]bool{"libx264": true} // no h264_vaapi
		caps.Hardware = map[string]bool{contracts.HWVAAPI: true}
		encoder, backend, _, fallback, err := chooseEncoder(settings, contracts.VideoCodecH264, report, caps, video)
		if err != nil {
			t.Fatal(err)
		}
		if encoder != "libx264" || backend != "" {
			t.Fatalf("encoder=%q backend=%q, want software", encoder, backend)
		}
		if !strings.Contains(fallback, "h264_vaapi") {
			t.Fatalf("fallback=%q, want the missing encoder named", fallback)
		}
	})
	t.Run("ready backend returns the hardware encoder", func(t *testing.T) {
		settings := base
		settings.HardwareEncode = true
		caps := testCaps()
		caps.Hardware = map[string]bool{contracts.HWVAAPI: true}
		encoder, backend, hwDecode, fallback, err := chooseEncoder(settings, contracts.VideoCodecH264, report, caps, video)
		if err != nil {
			t.Fatal(err)
		}
		if encoder != "h264_vaapi" || backend != contracts.HWVAAPI || !hwDecode || fallback != "" {
			t.Fatalf("encoder=%q backend=%q hwDecode=%v fallback=%q", encoder, backend, hwDecode, fallback)
		}
	})
}

func TestScaleFilter(t *testing.T) {
	cases := []struct {
		w, h int
		want string
	}{
		{1280, 720, "scale=w='min(iw,1280)':h='min(ih,720)':force_original_aspect_ratio=decrease:force_divisible_by=2"},
		{0, 720, "scale=-2:'min(ih,720)'"},
		{1280, 0, "scale='min(iw,1280)':-2"},
	}
	for _, c := range cases {
		if got := scaleFilter(c.w, c.h); got != c.want {
			t.Errorf("scaleFilter(%d,%d)=%q, want %q", c.w, c.h, got, c.want)
		}
	}
}

func TestToneMapChain(t *testing.T) {
	settings := contracts.DefaultTranscodeSettings() // bt2390, peak 100

	caps := capabilities{ToneMap2390: true}
	if got := toneMapChain(settings, caps); !strings.HasPrefix(got, "libplacebo=tonemapping=bt.2390") {
		t.Fatalf("bt2390 chain=%q", got)
	}

	caps.ToneMap2390 = false
	got := toneMapChain(settings, caps)
	for _, want := range []string{"npl=100", "tonemap=tonemap=hable", "transfer=bt709"} {
		if !strings.Contains(got, want) {
			t.Errorf("fallback chain %q lacks %q", got, want)
		}
	}

	settings.ToneMappingAlgorithm = contracts.ToneMapReinhard
	if got := toneMapChain(settings, caps); !strings.Contains(got, "tonemap=tonemap=reinhard") {
		t.Fatalf("reinhard chain=%q", got)
	}
}

func TestEscapeFilterPath(t *testing.T) {
	in := "/cache/s:1'2[3],4;5"
	want := `/cache/s\:1\'2\[3\]\,4\;5`
	if got := escapeFilterPath(in); got != want {
		t.Fatalf("escapeFilterPath(%q)=%q, want %q", in, got, want)
	}
}

// TestParseFFmpegList pins the -encoders/-filters/-hwaccels parsing:
// flags first, name second, legend lines skipped, bare names accepted.
func TestParseFFmpegList(t *testing.T) {
	set := map[string]bool{}
	out := strings.Join([]string{
		"Encoders:",
		" V..... = Video",
		" A..... = Audio",
		" ------",
		" V....D libx264              libx264 H.264 / AVC (codec h264)",
		" V....D libx265              libx265 H.265 / HEVC (codec hevc)",
		"  E  hls             Apple HTTP Live Streaming",
		"vaapi",
	}, "\n")
	parseFFmpegList([]byte(out), set)

	for _, want := range []string{"libx264", "libx265", "hls", "vaapi"} {
		if !set[want] {
			t.Errorf("parser missed %q: %v", want, set)
		}
	}
	for _, unwanted := range []string{"=", "Video", "V....D"} {
		if set[unwanted] {
			t.Errorf("parser kept legend token %q: %v", unwanted, set)
		}
	}
}
