package transcode


import (
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
)

// advCaps is the probed surface the advanced policy tests run against:
// software encoders, a ready VAAPI backend and the burn-in filters, so
// no real ffmpeg is needed.
func advCaps() capabilities {
	return capabilities{
		Encoders: map[string]bool{
			"libx264": true, "libx265": true, "libsvtav1": true, "libaom-av1": true,
			"h264_vaapi": true, "hevc_vaapi": true, "av1_vaapi": true,
		},
		Filters:  map[string]bool{"overlay": true, "subtitles": true, "ass": true, "zscale": true, "tonemap": true},
		Hwaccels: map[string]bool{"vaapi": true},
		Hardware: map[string]bool{contracts.HWVAAPI: true},
		ToneMap:  true,
	}
}

// advSource is the fictional media path for the advanced policy tests;
// the probe is injected, so the file never needs to exist.
func advSource() sourceSpec {
	return sourceSpec{Path: "/media/[Fansub-A] Advanced Show.mkv", Profile: "web-mp4-v3-adv"}
}

// advPlan builds a transcoder with injected probe/capability seams and
// plans one progressive request: every advanced knob must be testable
// without ffmpeg or real media.
func advPlan(t *testing.T, report mediaReport, caps capabilities, mutate func(*contracts.TranscodeV3Request)) (encodePlan, error) {
	t.Helper()
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	tr.probeFn = func(string, string) (mediaReport, error) { return report, nil }
	tr.capsFn = func(contracts.TranscodeSettings) capabilities { return caps }
	in := contracts.TranscodeV3Request{
		Action:   contracts.TranscodeStartAction,
		FilePath: advSource().Path,
		Delivery: contracts.TranscodeDeliveryProgressive,
		Settings: contracts.DefaultTranscodeSettings(),
	}
	if mutate != nil {
		mutate(&in)
	}
	return tr.planV3(advSource(), in, tr.capabilitiesFor(in.Settings))
}

// advRequirePlan fails the test when planning refused.
func advRequirePlan(t *testing.T, report mediaReport, caps capabilities, mutate func(*contracts.TranscodeV3Request)) encodePlan {
	t.Helper()
	plan, err := advPlan(t, report, caps, mutate)
	if err != nil {
		t.Fatalf("plan: %v", err)
	}
	return plan
}

// advVideoReport is a single-video SDR source with explicit pixel
// format and bits_per_raw_sample, which is what the 10-bit gate reads.
func advVideoReport(codec, pixel, bits string) mediaReport {
	return mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: codec, PixelFormat: pixel, BitsPerRaw: bits,
			Width: 1920, Height: 1080, FrameRate: "25/1"},
	}}
}

// advAVReport is one web-safe video plus one audio stream with a chosen
// codec and channel count, so only the audio policy varies.
func advAVReport(audioCodec string, channels int) mediaReport {
	return mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "h264", PixelFormat: "yuv420p",
			Width: 1920, Height: 1080, FrameRate: "25/1"},
		{Index: 1, CodecType: "audio", CodecName: audioCodec, Channels: channels, Default: 1},
	}}
}

// advSubtitleReport is a video + audio + text-subtitle source used for
// the burn-in font fallback.
func advSubtitleReport(codec string) mediaReport {
	return mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "h264", PixelFormat: "yuv420p",
			Width: 1920, Height: 1080, FrameRate: "25/1"},
		{Index: 1, CodecType: "audio", CodecName: "aac", Channels: 2, Default: 1},
		{Index: 2, CodecType: "subtitle", CodecName: codec},
	}}
}

// advArgs renders the encoder argv of one plan with a progressive output
// path; the path is never written to.
func advArgs(t *testing.T, plan encodePlan) []string {
	t.Helper()
	return plan.ffmpegArgs(filepath.Join(t.TempDir(), "out.mp4"), "", false)
}

// advFlagValue returns the argument after flag, which is how ffmpeg
// flags carrying a value are asserted.
func advFlagValue(args []string, flag string) (string, bool) {
	for i := 0; i+1 < len(args); i++ {
		if args[i] == flag {
			return args[i+1], true
		}
	}
	return "", false
}

// advSubtitleFilter extracts the burn-in filter from a plan.
func advSubtitleFilter(t *testing.T, plan encodePlan) string {
	t.Helper()
	for _, f := range plan.filters {
		if strings.HasPrefix(f, "subtitles=") {
			return f
		}
	}
	t.Fatalf("no subtitles filter in %v", plan.filters)
	return ""
}

// TestAdvancedTenBitDecodeGating pins the per-codec 10-bit opt-in: a
// 10-bit source keeps hardware decoding only when the operator enabled
// that codec, and the software fallback stays visible otherwise.
func TestAdvancedTenBitDecodeGating(t *testing.T) {
	base := contracts.DefaultTranscodeSettings()
	base.HardwareAcceleration = contracts.HWVAAPI
	caps := advCaps()

	cases := []struct {
		name       string
		report     mediaReport
		mutate     func(*contracts.TranscodeSettings)
		wantDecode bool
	}{
		{"hevc 10le without opt-in", advVideoReport("hevc", "yuv420p10le", ""),
			func(s *contracts.TranscodeSettings) { s.HardwareDecode10BitHEVC = false }, false},
		{"hevc 10le with opt-in", advVideoReport("hevc", "yuv420p10le", ""),
			func(s *contracts.TranscodeSettings) { s.HardwareDecode10BitHEVC = true }, true},
		{"vp9 10le without opt-in", advVideoReport("vp9", "yuv420p10le", ""),
			func(s *contracts.TranscodeSettings) { s.HardwareDecode10BitVP9 = false }, false},
		{"vp9 10le with opt-in", advVideoReport("vp9", "yuv420p10le", ""),
			func(s *contracts.TranscodeSettings) { s.HardwareDecode10BitVP9 = true }, true},
		{"bits_per_raw_sample 10 without opt-in", advVideoReport("hevc", "yuv420p", "10"),
			func(s *contracts.TranscodeSettings) { s.HardwareDecode10BitHEVC = false }, false},
		{"bits_per_raw_sample 10 with opt-in", advVideoReport("hevc", "yuv420p", "10"),
			func(s *contracts.TranscodeSettings) { s.HardwareDecode10BitHEVC = true }, true},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			plan := advRequirePlan(t, tc.report, caps, func(in *contracts.TranscodeV3Request) {
				in.Settings = base
				tc.mutate(&in.Settings)
			})
			if plan.hwDecode != tc.wantDecode {
				t.Fatalf("hwDecode=%v, want %v (fallback=%q)", plan.hwDecode, tc.wantDecode, plan.fallback)
			}
			if tc.wantDecode && plan.fallback != "" {
				t.Fatalf("fallback=%q, want none when 10-bit decode is enabled", plan.fallback)
			}
			if !tc.wantDecode && !strings.Contains(plan.fallback, "10-bit") {
				t.Fatalf("fallback=%q, want a visible 10-bit note", plan.fallback)
			}
		})
	}
}

// TestAdvancedDeinterlaceDoubleRate pins the double-rate knob: it changes
// the yadif mode only while deinterlacing is enabled.
func TestAdvancedDeinterlaceDoubleRate(t *testing.T) {
	report := advAVReport("aac", 2)
	report.Streams[0].FieldOrder = "tt"

	double := advRequirePlan(t, report, advCaps(), func(in *contracts.TranscodeV3Request) {
		in.Settings.Deinterlace = contracts.DeinterlaceAuto
		in.Settings.DeinterlaceDoubleRate = true
	})
	if !slices.Contains(double.filters, "yadif=mode=send_field") {
		t.Fatalf("filters=%v, want yadif=mode=send_field for double rate", double.filters)
	}

	single := advRequirePlan(t, report, advCaps(), func(in *contracts.TranscodeV3Request) {
		in.Settings.Deinterlace = contracts.DeinterlaceAuto
		in.Settings.DeinterlaceDoubleRate = false
	})
	if !slices.Contains(single.filters, "yadif") || slices.Contains(single.filters, "yadif=mode=send_field") {
		t.Fatalf("filters=%v, want plain yadif without double rate", single.filters)
	}

	off := advRequirePlan(t, report, advCaps(), func(in *contracts.TranscodeV3Request) {
		in.Settings.Deinterlace = contracts.DeinterlaceOff
		in.Settings.DeinterlaceDoubleRate = true
	})
	for _, f := range off.filters {
		if strings.HasPrefix(f, "yadif") {
			t.Fatalf("filters=%v, want no yadif when deinterlace is off", off.filters)
		}
	}
}

// TestAdvancedSubtitleFontFallback pins the burn-in font fallback:
// fontsdir plus a sanitized forced family, and nothing at all when the
// operator configured neither.
func TestAdvancedSubtitleFontFallback(t *testing.T) {
	selected := 2
	burn := func(in *contracts.TranscodeV3Request) {
		in.SubtitleStream = &selected
		in.SubtitleMode = contracts.SubtitleModeBurn
	}

	withFonts := advRequirePlan(t, advSubtitleReport("ass"), advCaps(), func(in *contracts.TranscodeV3Request) {
		burn(in)
		in.Settings.FallbackFontPath = "/tmp/fonts"
		in.Settings.FallbackFontName = "DejaVu Sans"
	})
	filter := advSubtitleFilter(t, withFonts)
	if !strings.Contains(filter, "fontsdir=") {
		t.Fatalf("filter=%q, want fontsdir", filter)
	}
	if !strings.Contains(filter, "force_style='Fontname=DejaVu Sans'") {
		t.Fatalf("filter=%q, want the forced family", filter)
	}

	// A hostile settings value must never inject filter-graph syntax.
	hostile := advRequirePlan(t, advSubtitleReport("ass"), advCaps(), func(in *contracts.TranscodeV3Request) {
		burn(in)
		in.Settings.FallbackFontName = "Bad'; rm -rf /"
	})
	filter = advSubtitleFilter(t, hostile)
	if strings.Contains(filter, "rm -rf /") {
		t.Fatalf("filter=%q kept the hostile suffix", filter)
	}
	if strings.Count(filter, "'") != 2 {
		t.Fatalf("filter=%q must keep exactly the two force_style quotes", filter)
	}
	idx := strings.Index(filter, "Fontname=")
	if idx < 0 {
		t.Fatalf("filter=%q lost the forced family", filter)
	}
	name := strings.TrimSuffix(filter[idx+len("Fontname="):], "'")
	if name == "" {
		t.Fatalf("filter=%q sanitized the family to nothing", filter)
	}
	for _, r := range name {
		safe := r == ' ' || r == '-' || r == '_' || r == '+' ||
			(r >= 'a' && r <= 'z') || (r >= 'A' && r <= 'Z') || (r >= '0' && r <= '9')
		if !safe {
			t.Fatalf("filter=%q kept unsafe rune %q in the family", filter, r)
		}
	}

	plain := advRequirePlan(t, advSubtitleReport("ass"), advCaps(), burn)
	filter = advSubtitleFilter(t, plain)
	if strings.Contains(filter, "fontsdir=") || strings.Contains(filter, "force_style") {
		t.Fatalf("filter=%q, want no font options when none are configured", filter)
	}
}

// TestAdvancedAudioVBRAndDownmix pins the audio argv: VBR replaces the
// bitrate flag for AAC, the downmix gain becomes a volume filter, and
// the nightmode matrix only applies to a real downmix.
func TestAdvancedAudioVBRAndDownmix(t *testing.T) {
	vbr := advRequirePlan(t, advAVReport("mp3", 2), advCaps(), func(in *contracts.TranscodeV3Request) {
		in.Settings.AudioVBR = true
	})
	args := advArgs(t, vbr)
	if v, ok := advFlagValue(args, "-q:a"); !ok || v != "1.5" {
		t.Fatalf("args=%v, want -q:a 1.5 for 160 kbps VBR", args)
	}
	if _, ok := advFlagValue(args, "-b:a"); ok {
		t.Fatalf("args=%v, VBR must not also pin a bitrate", args)
	}

	cbr := advRequirePlan(t, advAVReport("mp3", 2), advCaps(), func(in *contracts.TranscodeV3Request) {
		in.Settings.AudioVBR = false
	})
	args = advArgs(t, cbr)
	if v, ok := advFlagValue(args, "-b:a"); !ok || v != "160k" {
		t.Fatalf("args=%v, want -b:a 160k without VBR", args)
	}
	if _, ok := advFlagValue(args, "-q:a"); ok {
		t.Fatalf("args=%v, CBR must not also set the VBR quality", args)
	}

	downmix := advRequirePlan(t, advAVReport("ac3", 6), advCaps(), func(in *contracts.TranscodeV3Request) {
		in.Settings.DownmixAudioBoost = 2
		in.Settings.DownmixStereoAlgorithm = contracts.DownmixNone
	})
	args = advArgs(t, downmix)
	if v, ok := advFlagValue(args, "-af"); !ok || v != "volume=2.00" {
		t.Fatalf("args=%v, want the downmix gain as -af volume=2.00", args)
	}

	night := advRequirePlan(t, advAVReport("ac3", 6), advCaps(), func(in *contracts.TranscodeV3Request) {
		in.Settings.DownmixStereoAlgorithm = contracts.DownmixNightmode
	})
	args = advArgs(t, night)
	if v, ok := advFlagValue(args, "-af"); !ok || !strings.HasPrefix(v, "pan=stereo|") {
		t.Fatalf("args=%v, want the nightmode pan chain first in -af", args)
	}

	// A stereo source is not downmixed, so the matrix must never apply.
	stereo := advRequirePlan(t, advAVReport("ac3", 2), advCaps(), func(in *contracts.TranscodeV3Request) {
		in.Settings.DownmixStereoAlgorithm = contracts.DownmixNightmode
	})
	args = advArgs(t, stereo)
	if _, ok := advFlagValue(args, "-af"); ok {
		t.Fatalf("args=%v, a stereo source must not get a downmix filter", args)
	}
}

// TestAdvancedEncoderArgvBounds pins the resource bounds and the
// per-codec tuning in the actual ffmpeg argv.
func TestAdvancedEncoderArgvBounds(t *testing.T) {
	report := mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "mpeg4", PixelFormat: "yuv420p",
			Width: 640, Height: 480, FrameRate: "25/1"},
		{Index: 1, CodecType: "audio", CodecName: "mp3", Channels: 2, Default: 1},
	}}

	bound := advRequirePlan(t, report, advCaps(), func(in *contracts.TranscodeV3Request) {
		in.Settings.ThreadCount = 4
		in.Settings.MuxingQueueSize = 4096
		in.Settings.H264CRF = 30
		in.Settings.H264Preset = "slow"
	})
	args := advArgs(t, bound)
	if v, ok := advFlagValue(args, "-threads"); !ok || v != "4" {
		t.Fatalf("args=%v, want -threads 4", args)
	}
	if v, ok := advFlagValue(args, "-max_muxing_queue_size"); !ok || v != "4096" {
		t.Fatalf("args=%v, want -max_muxing_queue_size 4096", args)
	}
	if v, ok := advFlagValue(args, "-crf"); !ok || v != "30" {
		t.Fatalf("args=%v, want the h264 crf 30", args)
	}
	if v, ok := advFlagValue(args, "-preset"); !ok || v != "slow" {
		t.Fatalf("args=%v, want the h264 preset slow", args)
	}

	defaults := advRequirePlan(t, report, advCaps(), nil)
	args = advArgs(t, defaults)
	if _, ok := advFlagValue(args, "-threads"); ok {
		t.Fatalf("args=%v, thread_count 0 must leave the choice to ffmpeg", args)
	}
	if v, ok := advFlagValue(args, "-max_muxing_queue_size"); !ok || v != "2048" {
		t.Fatalf("args=%v, want the shipped muxing queue 2048", args)
	}
	if v, ok := advFlagValue(args, "-preset"); !ok || v != contracts.DefaultTranscodeSettings().EncoderPreset {
		t.Fatalf("args=%v, want the general preset when no per-codec preset is set", args)
	}

	hevc := advRequirePlan(t, report, advCaps(), func(in *contracts.TranscodeV3Request) {
		in.VideoCodec = contracts.VideoCodecHEVC
		in.Settings.AllowHEVC = true
		in.Settings.H265CRF = 26
		in.Settings.H265Preset = "medium"
	})
	if hevc.encoder != "libx265" {
		t.Fatalf("encoder=%q, want libx265", hevc.encoder)
	}
	args = advArgs(t, hevc)
	if v, ok := advFlagValue(args, "-crf"); !ok || v != "26" {
		t.Fatalf("args=%v, want the hevc crf 26", args)
	}
	if v, ok := advFlagValue(args, "-preset"); !ok || v != "medium" {
		t.Fatalf("args=%v, want the hevc preset medium", args)
	}

	av1 := advRequirePlan(t, report, advCaps(), func(in *contracts.TranscodeV3Request) {
		in.VideoCodec = contracts.VideoCodecAV1
		in.Settings.AllowAV1 = true
		in.Settings.AV1CRF = 35
	})
	if av1.encoder != "libsvtav1" {
		t.Fatalf("encoder=%q, want libsvtav1", av1.encoder)
	}
	args = advArgs(t, av1)
	if v, ok := advFlagValue(args, "-crf"); !ok || v != "35" {
		t.Fatalf("args=%v, want the av1 crf 35", args)
	}
}

// A decode-only hardware policy must actually pass -hwaccel to ffmpeg: the
// plan reported hardware decode while the argv stayed software, so enabling
// acceleration without hardware encode did nothing.
func TestAdvancedDecodeOnlyHardwareEmitsHwaccel(t *testing.T) {
	caps := testCaps()
	caps.Hardware = map[string]bool{contracts.HWVAAPI: true}
	noCopy := false
	plan := advRequirePlan(t, advVideoReport("h264", "yuv420p", "8"), caps, func(in *contracts.TranscodeV3Request) {
		in.Settings.HardwareAcceleration = contracts.HWVAAPI
		in.Settings.HardwareEncode = false
		// Force a re-encode so chooseEncoder runs on a web-safe source.
		in.AllowVideoStreamCopy = &noCopy
	})
	if plan.hwBackend != "" {
		t.Fatalf("hwBackend=%q, want empty for a software encode", plan.hwBackend)
	}
	if !plan.hwDecode {
		t.Fatalf("hwDecode=false, want true (fallback=%q)", plan.fallback)
	}
	args := advArgs(t, plan)
	if v, ok := advFlagValue(args, "-hwaccel"); !ok || v != "vaapi" {
		t.Fatalf("decode-only plan did not pass -hwaccel vaapi: %v", args)
	}
	if got := plan.hardwareLabel(); got != contracts.HWVAAPI {
		t.Fatalf("hardwareLabel()=%q, want %q", got, contracts.HWVAAPI)
	}
}

// Hardware encoding names the encode backend, so the fully accelerated path
// keeps its label and still passes the decode hwaccel.
func TestAdvancedHardwareEncodeNamesBackend(t *testing.T) {
	caps := testCaps()
	caps.Hardware = map[string]bool{contracts.HWVAAPI: true}
	noCopy := false
	plan := advRequirePlan(t, advVideoReport("h264", "yuv420p", "8"), caps, func(in *contracts.TranscodeV3Request) {
		in.Settings.HardwareAcceleration = contracts.HWVAAPI
		in.Settings.HardwareEncode = true
		in.AllowVideoStreamCopy = &noCopy
	})
	if plan.hwBackend != contracts.HWVAAPI {
		t.Fatalf("hwBackend=%q, want %q", plan.hwBackend, contracts.HWVAAPI)
	}
	if got := plan.hardwareLabel(); got != contracts.HWVAAPI {
		t.Fatalf("hardwareLabel()=%q, want %q", got, contracts.HWVAAPI)
	}
	args := advArgs(t, plan)
	if v, ok := advFlagValue(args, "-hwaccel"); !ok || v != "vaapi" {
		t.Fatalf("hardware-encode plan did not pass -hwaccel vaapi: %v", args)
	}
}

// The operator-selected VA-API render node must reach the decode argv
// instead of the hardcoded default.
func TestAdvancedHardwareDeviceReachesHwaccel(t *testing.T) {
	caps := testCaps()
	caps.Hardware = map[string]bool{contracts.HWVAAPI: true}
	noCopy := false
	plan := advRequirePlan(t, advVideoReport("h264", "yuv420p", "8"), caps, func(in *contracts.TranscodeV3Request) {
		in.Settings.HardwareAcceleration = contracts.HWVAAPI
		in.Settings.HardwareDevice = "/dev/dri/renderD129"
		in.AllowVideoStreamCopy = &noCopy
	})
	args := advArgs(t, plan)
	if v, ok := advFlagValue(args, "-hwaccel_device"); !ok || v != "/dev/dri/renderD129" {
		t.Fatalf("args=%v, want -hwaccel_device /dev/dri/renderD129", args)
	}
}
