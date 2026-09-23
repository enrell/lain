package transcode


// Mutation-killer tests: narrow cases aimed at branches the broader
// suites exercise but never assert on.

import (
	"context"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
)

// --- encode.go ---------------------------------------------------------

func TestKillSanitizeFontName(t *testing.T) {
	cases := []struct{ in, want string }{
		{"DejaVu Sans", "DejaVu Sans"},
		{"A'rial=Bold", "ArialBold"},   // quotes and = are filter syntax
		{"Font:Name;[x]", "FontNamex"}, // : ; [ ] dropped
		{"`{@/:/9", "9"},               // ` { @ / : sit at class edges
		{"azAZ09 -_+", "azAZ09 -_+"},   // every kept class
		{"  pad  ", "pad"},             // TrimSpace
		{"''';;;", ""},                 // nothing survives
		{"Noto Étude", "Noto tude"},    // non-ASCII dropped
	}
	for _, c := range cases {
		if got := sanitizeFontName(c.in); got != c.want {
			t.Errorf("sanitizeFontName(%q)=%q, want %q", c.in, got, c.want)
		}
	}
}

func TestKillSubtitleFontOptions(t *testing.T) {
	settings := contracts.DefaultTranscodeSettings()
	if got := subtitleFontOptions(settings); got != "" {
		t.Fatalf("options=%q, want empty with no font configured", got)
	}
	off := false
	settings.FallbackFontEnabled = &off
	settings.FallbackFontPath = "/fonts dir"
	settings.FallbackFontName = "Name"
	if got := subtitleFontOptions(settings); got != "" {
		t.Fatalf("options=%q, want empty when fallback fonts are off", got)
	}
	settings.FallbackFontEnabled = nil
	settings.FallbackFontName = "Evil';Name"
	got := subtitleFontOptions(settings)
	if !strings.Contains(got, ":fontsdir="+fontsPlaceholder) {
		t.Fatalf("options=%q, want fontsdir placeholder", got)
	}
	if !strings.Contains(got, "Fontname=EvilName") {
		t.Fatalf("options=%q, want sanitized family", got)
	}
	if strings.Contains(got, "';") {
		t.Fatalf("options=%q, injection survived", got)
	}
}

// encodePlanFor builds a minimal encodable plan for argv assertions.
func encodePlanFor(mutate func(*encodePlan)) encodePlan {
	p := encodePlan{
		spec:     sourceSpec{Path: "/media/in.mkv"},
		settings: contracts.DefaultTranscodeSettings(),
		video:    &stream{Index: 0, CodecName: "mpeg4", Width: 1920, Height: 1080, FrameRate: "30000/1001"},
		encoder:  "libx264", preset: "veryfast", crf: 23,
		videoCodec: contracts.VideoCodecH264, audioCodec: contracts.AudioCodecAAC,
		audioBitrateKbps: 160,
	}
	if mutate != nil {
		mutate(&p)
	}
	return p
}

func argv(args []string) string { return "\x00" + strings.Join(args, "\x00") + "\x00" }

func hasArg(args []string, pair ...string) bool {
	joined := argv(args)
	return strings.Contains(joined, "\x00"+strings.Join(pair, "\x00")+"\x00")
}

func TestKillFFmpegArgsShape(t *testing.T) {
	p := encodePlanFor(func(p *encodePlan) {
		p.spec.StartSec = 12.5
		p.filters = []string{"yadif", "scale=-2:720"}
		p.audio = &stream{Index: 1}
		p.settings.ThreadCount = 4
		p.settings.MuxingQueueSize = 512
	})
	args := p.ffmpegArgs("/tmp/out.mp4", "/tmp/burn.ass", true)
	for _, pair := range [][]string{
		{"-progress", "pipe:1"}, {"-ss", "12.500"}, {"-i", "/media/in.mkv"},
		{"-map", "0:0"}, {"-vf", "yadif,scale=-2:720"}, {"-map", "0:1"},
		{"-sn"}, {"-c:v", "libx264"}, {"-pix_fmt", "yuv420p"},
		{"-c:a", "aac"}, {"-threads", "4"}, {"-max_muxing_queue_size", "512"},
		{"-f", "mp4"},
	} {
		if !hasArg(args, pair...) {
			t.Errorf("args %v lack %v", args, pair)
		}
	}
	if hasArg(args, "-an") {
		t.Error("audio stream selected but -an emitted")
	}

	noAudio := encodePlanFor(nil)
	args = noAudio.ffmpegArgs("/tmp/out.mp4", "", false)
	if !hasArg(args, "-an") || hasArg(args, "-progress", "pipe:1") {
		t.Fatalf("args=%v, want -an and no -progress", args)
	}

	burn := encodePlanFor(func(p *encodePlan) {
		p.complexFilter = "[0:v:0]scale=iw:ih[base];[base][0:2]overlay[vout]"
		p.subtitleMode = contracts.SubtitleModeBurn
	})
	args = burn.ffmpegArgs("/tmp/out.mp4", "/tmp/burn file.ass", false)
	if !hasArg(args, "-map", "[vout]") || hasArg(args, "-sn") {
		t.Fatalf("burn args=%v", args)
	}
	if !hasArg(args, "-filter_complex") {
		t.Fatalf("burn args=%v, want -filter_complex", args)
	}

	copyPlan := encodePlanFor(func(p *encodePlan) { p.copyVideo = true })
	args = copyPlan.ffmpegArgs("/tmp/out.mp4", "", false)
	if !hasArg(args, "-c:v", "copy") || hasArg(args, "-pix_fmt") {
		t.Fatalf("copy args=%v", args)
	}

	vaapi := encodePlanFor(func(p *encodePlan) { p.encoder = "h264_vaapi" })
	if hasArg(vaapi.ffmpegArgs("/tmp/o", "", false), "-pix_fmt") {
		t.Fatal("vaapi must not get yuv420p pix_fmt")
	}
}

func TestKillHwaccelInputArgs(t *testing.T) {
	settings := contracts.DefaultTranscodeSettings()
	settings.HardwareAcceleration = contracts.HWNVENC
	p := encodePlan{settings: settings}
	if got := p.hwaccelInputArgs(); got != nil {
		t.Fatalf("hwDecode off args=%v, want nil", got)
	}
	p.hwDecode = true // backend falls back to the configured one
	if got := p.hwaccelInputArgs(); !hasArg(got, "-hwaccel", "cuda") {
		t.Fatalf("decode-only args=%v, want cuda", got)
	}
	p.hwBackend = contracts.HWVAAPI // plan backend wins
	settings.HardwareDevice = "/dev/dri/renderD128"
	if got := p.hwaccelInputArgs(); !hasArg(got, "-hwaccel", "vaapi", "-hwaccel_device", "/dev/dri/renderD128") {
		t.Fatalf("vaapi args=%v", got)
	}
	for backend, want := range map[string]string{
		contracts.HWQSV: "qsv", contracts.HWVideoToolbox: "videotoolbox", contracts.HWRKMPP: "rkmpp",
	} {
		p.hwBackend = backend
		if got := p.hwaccelInputArgs(); !hasArg(got, "-hwaccel", want) {
			t.Errorf("backend %s args=%v", backend, got)
		}
	}
	p.hwBackend = "bogus"
	if got := p.hwaccelInputArgs(); got != nil {
		t.Fatalf("unknown backend args=%v, want nil", got)
	}
}

func TestKillHardwareLabel(t *testing.T) {
	settings := contracts.DefaultTranscodeSettings()
	settings.HardwareAcceleration = contracts.HWVAAPI
	p := encodePlan{settings: settings}
	if p.hardwareLabel() != "" {
		t.Fatal("CPU plan must not claim hardware")
	}
	p.hwDecode = true
	if p.hardwareLabel() != contracts.HWVAAPI {
		t.Fatalf("label=%q, want vaapi for decode-only", p.hardwareLabel())
	}
	p.hwBackend = contracts.HWNVENC
	if p.hardwareLabel() != contracts.HWNVENC {
		t.Fatalf("label=%q, want the encode backend", p.hardwareLabel())
	}
}

func TestKillHLSArgs(t *testing.T) {
	base := encodePlanFor(func(p *encodePlan) {
		p.delivery = contracts.TranscodeDeliveryHLS
		p.settings.HLSSegmentSeconds = 6
	})
	args := base.hlsArgs("/h/dir", "/h/dir/raw.m3u8")
	for _, pair := range [][]string{
		{"-f", "hls"}, {"-hls_time", "6"}, {"-hls_playlist_type", "event"},
		{"-hls_list_size", "0"}, {"-hls_segment_type", "fmp4"},
		{"-hls_fmp4_init_filename", "init.mp4"},
	} {
		if !hasArg(args, pair...) {
			t.Errorf("hls args %v lack %v", args, pair)
		}
	}
	// 29.97fps * 6s = 179.82 -> gop 180.
	if !hasArg(args, "-g", "180") || !hasArg(args, "-keyint_min", "180") || !hasArg(args, "-sc_threshold", "0") {
		t.Fatalf("GOP args missing: %v", args)
	}
	if !hasArg(args, filepath.Join("/h/dir", "seg%05d.m4s")) {
		t.Fatalf("fmp4 segment filename missing: %v", args)
	}

	ts := encodePlanFor(func(p *encodePlan) {
		p.delivery = contracts.TranscodeDeliveryHLS
		p.settings.HLSSegmentSeconds = 4
		p.settings.HLSSegmentContainer = contracts.HLSSegmentTS
	})
	args = ts.hlsArgs("/h/dir", "/h/dir/raw.m3u8")
	if !hasArg(args, "-hls_segment_type", "mpegts") || !hasArg(args, filepath.Join("/h/dir", "seg%05d.ts")) {
		t.Fatalf("ts args=%v", args)
	}
	if hasArg(args, "-hls_fmp4_init_filename") {
		t.Fatalf("ts args carry init segment: %v", args)
	}

	// Stream copy cannot move keyframes: no GOP pinning.
	copyHLS := encodePlanFor(func(p *encodePlan) {
		p.delivery = contracts.TranscodeDeliveryHLS
		p.settings.HLSSegmentSeconds = 6
		p.copyVideo = true
	})
	if args := copyHLS.hlsArgs("/h/dir", "/h/dir/raw.m3u8"); hasArg(args, "-force_key_frames") || hasArg(args, "-g") {
		t.Fatalf("copy args=%v must not pin GOP", args)
	}

	// libx265 gets -sc_threshold 0; a hardware encoder does not.
	hevc := encodePlanFor(func(p *encodePlan) {
		p.delivery = contracts.TranscodeDeliveryHLS
		p.settings.HLSSegmentSeconds = 6
		p.encoder = "libx265"
	})
	if args := hevc.hlsArgs("/h/d", "/h/d/raw.m3u8"); !hasArg(args, "-sc_threshold", "0") {
		t.Fatal("libx265 wants -sc_threshold 0")
	}
	hw := encodePlanFor(func(p *encodePlan) {
		p.delivery = contracts.TranscodeDeliveryHLS
		p.settings.HLSSegmentSeconds = 6
		p.encoder = "h264_vaapi"
	})
	if args := hw.hlsArgs("/h/d", "/h/d/raw.m3u8"); hasArg(args, "-sc_threshold") {
		t.Fatal("hardware encoder must not get -sc_threshold")
	}

	// Unknown frame rate: no GOP is invented.
	slow := encodePlanFor(func(p *encodePlan) {
		p.delivery = contracts.TranscodeDeliveryHLS
		p.settings.HLSSegmentSeconds = 6
		p.video.FrameRate = "0/0"
	})
	if args := slow.hlsArgs("/h/d", "/h/d/raw.m3u8"); hasArg(args, "-g") {
		t.Fatal("unknown fps must not invent a GOP")
	}

	// Sub-frame GOP clamps to 1.
	tiny := encodePlanFor(func(p *encodePlan) {
		p.delivery = contracts.TranscodeDeliveryHLS
		p.settings.HLSSegmentSeconds = 0
		p.video.FrameRate = "1/1"
	})
	if args := tiny.hlsArgs("/h/d", "/h/d/raw.m3u8"); !hasArg(args, "-g", "1") {
		t.Fatalf("args=%v, want -g 1 floor", args)
	}
}

func TestKillVideoQualityArgs(t *testing.T) {
	at := func(mutate func(*encodePlan)) []string {
		return encodePlanFor(mutate).videoQualityArgs()
	}
	if got := at(func(p *encodePlan) { p.encoder = "h264_vaapi" }); !hasArg(got, "-qp", "23") {
		t.Fatalf("vaapi=%v", got)
	}
	if got := at(func(p *encodePlan) { p.encoder = "hevc_nvenc"; p.preset = "slow" }); !hasArg(got, "-preset", "p5", "-cq", "23", "-rc", "vbr") {
		t.Fatalf("nvenc=%v", got)
	}
	if got := at(func(p *encodePlan) { p.encoder = "h264_qsv" }); !hasArg(got, "-global_quality", "23") || hasArg(got, "-low_power") {
		t.Fatalf("qsv=%v", got)
	}
	if got := at(func(p *encodePlan) { p.encoder = "h264_qsv"; p.settings.HardwareLowPower = true }); !hasArg(got, "-low_power", "1") {
		t.Fatalf("qsv low_power=%v", got)
	}
	if got := at(func(p *encodePlan) { p.encoder = "h264_amf" }); !hasArg(got, "-quality", "speed", "-qp_i", "23", "-qp_p", "23") {
		t.Fatalf("amf=%v", got)
	}
	if got := at(func(p *encodePlan) { p.encoder = "h264_v4l2m2m" }); !hasArg(got, "-b:v", "4000k") {
		t.Fatalf("v4l2 default=%v", got)
	}
	if got := at(func(p *encodePlan) { p.encoder = "h264_v4l2m2m"; p.maxBitrateKbps = 2500 }); !hasArg(got, "-b:v", "2500k") {
		t.Fatalf("v4l2 capped=%v", got)
	}
	// VideoToolbox inverts CRF onto 0..100, clamped at 0.
	if got := at(func(p *encodePlan) { p.encoder = "h264_videotoolbox"; p.crf = 23 }); !hasArg(got, "-q:v", "54") {
		t.Fatalf("videotoolbox=%v", got)
	}
	if got := at(func(p *encodePlan) { p.encoder = "h264_videotoolbox"; p.crf = 60 }); !hasArg(got, "-q:v", "0") {
		t.Fatalf("videotoolbox clamp=%v", got)
	}
	if got := at(func(p *encodePlan) { p.encoder = "libsvtav1"; p.preset = "slow" }); !hasArg(got, "-preset", "4", "-crf", "23") {
		t.Fatalf("svtav1=%v", got)
	}
	if got := at(func(p *encodePlan) { p.encoder = "libaom-av1"; p.preset = "slow" }); !hasArg(got, "-cpu-used", "4", "-crf", "23", "-b:v", "0") {
		t.Fatalf("libaom=%v", got)
	}
	if got := at(func(p *encodePlan) {
		p.encoder = "libaom-av1"
		p.maxBitrateKbps = 3000
	}); !hasArg(got, "-b:v", "3000k", "-maxrate", "3000k", "-bufsize", "6000k") {
		t.Fatalf("libaom capped=%v", got)
	}
	// maxrate/bufsize apply on top of any encoder.
	if got := at(func(p *encodePlan) { p.maxBitrateKbps = 1000 }); !hasArg(got, "-maxrate", "1000k", "-bufsize", "2000k") {
		t.Fatalf("cap=%v", got)
	}
	if got := at(nil); !hasArg(got, "-preset", "veryfast", "-crf", "23") {
		t.Fatalf("x264=%v", got)
	}
}

func TestKillPresetMaps(t *testing.T) {
	nv := map[string]string{
		"ultrafast": "p1", "superfast": "p1", "veryfast": "p1", "faster": "p2", "fast": "p2",
		"medium": "p4", "slow": "p5", "slower": "p6", "veryslow": "p7", "bogus": "p4",
	}
	for in, want := range nv {
		if got := nvencPreset(in); got != want {
			t.Errorf("nvencPreset(%q)=%q, want %q", in, got, want)
		}
	}
	svt := map[string]string{
		"ultrafast": "13", "superfast": "12", "veryfast": "10", "faster": "9", "fast": "8",
		"medium": "6", "slow": "4", "slower": "2", "veryslow": "0", "bogus": "8",
	}
	for in, want := range svt {
		if got := svtAV1Preset(in); got != want {
			t.Errorf("svtAV1Preset(%q)=%q, want %q", in, got, want)
		}
	}
	aom := map[string]string{
		"ultrafast": "8", "superfast": "8", "veryfast": "7", "faster": "6", "fast": "6",
		"medium": "5", "slow": "4", "slower": "3", "veryslow": "2", "bogus": "8",
	}
	for in, want := range aom {
		if got := libaomCPUUsed(in); got != want {
			t.Errorf("libaomCPUUsed(%q)=%q, want %q", in, got, want)
		}
	}
}

func TestKillAudioQualityArgs(t *testing.T) {
	p := encodePlanFor(nil)
	if got := p.audioQualityArgs(); !hasArg(got, "-b:a", "160k") {
		t.Fatalf("cbr=%v", got)
	}
	p.settings.AudioVBR = true
	if got := p.audioQualityArgs(); !hasArg(got, "-q:a") || hasArg(got, "-b:a") {
		t.Fatalf("vbr=%v", got)
	}
	// VBR only applies to AAC.
	p.audioCodec = contracts.AudioCodecAC3
	if got := p.audioQualityArgs(); !hasArg(got, "-b:a", "160k") {
		t.Fatalf("ac3=%v, want CBR", got)
	}
}

func TestKillAudioFilters(t *testing.T) {
	surround := &stream{Index: 1, Channels: 6}
	p := encodePlanFor(func(p *encodePlan) {
		p.audio = surround
		p.downmix = true
		p.settings.DownmixStereoAlgorithm = contracts.DownmixNightmode
	})
	if got := p.audioFilters(); !strings.Contains(got, "pan=stereo") {
		t.Fatalf("nightmode=%q", got)
	}
	// Nightmode only applies to a 5.1 source layout.
	p.audio = &stream{Index: 1, Channels: 8}
	if got := p.audioFilters(); strings.Contains(got, "pan=") {
		t.Fatalf("7.1 got nightmode: %q", got)
	}
	// Boost of exactly 1 is a no-op; other values emit a volume filter.
	p.audio = surround
	p.settings.DownmixAudioBoost = 1
	if got := p.audioFilters(); strings.Contains(got, "volume=") {
		t.Fatalf("boost=1 emitted volume: %q", got)
	}
	p.settings.DownmixAudioBoost = 1.5
	if got := p.audioFilters(); !strings.Contains(got, "volume=1.50") {
		t.Fatalf("boost=%q", got)
	}
	p.settings.DownmixStereoAlgorithm = contracts.DownmixNone
	got := p.audioFilters()
	if strings.Contains(got, "pan=") || !strings.Contains(got, "volume=1.50") {
		t.Fatalf("none+boost=%q", got)
	}
	p.downmix = false
	if got := p.audioFilters(); got != "" {
		t.Fatalf("no downmix=%q, want empty", got)
	}
}

func TestKillSoftwareFallbackPlan(t *testing.T) {
	p := encodePlanFor(func(p *encodePlan) {
		p.encoder = "h264_vaapi"
		p.hwBackend = contracts.HWVAAPI
		p.hwDecode = true
		p.videoCodec = contracts.VideoCodecHEVC
		p.filters = []string{"yadif", "format=nv12", "hwupload"}
		p.complexFilter = "[0:v]scale=iw:ih,format=nv12,hwupload[base];[base][0:2]overlay[vout]"
		p.fallback = "first note"
	})
	caps := capabilities{Encoders: map[string]bool{"libx265": true}}
	fb := p.softwareFallback(caps)
	if fb.encoder != "libx265" || fb.hwBackend != "" || fb.hwDecode {
		t.Fatalf("fallback=%+v", fb)
	}
	if strings.Contains(strings.Join(fb.filters, ","), "hwupload") || strings.Contains(strings.Join(fb.filters, ","), "format=nv12") {
		t.Fatalf("filters still carry hwupload: %v", fb.filters)
	}
	if strings.Contains(fb.complexFilter, "hwupload") {
		t.Fatalf("complexFilter still carries hwupload: %q", fb.complexFilter)
	}
	if fb.fallback != "first note; hardware encode failed; used software" {
		t.Fatalf("fallback note=%q", fb.fallback)
	}
	if got := stripHardwareUpload(""); got != "" {
		t.Fatalf("empty chain=%q", got)
	}
	if got := stripHardwareUpload("format=nv12,hwupload"); got != "" {
		t.Fatalf("bare chain=%q", got)
	}
	if got := softwareEncoderFor(contracts.VideoCodecAV1, capabilities{Encoders: map[string]bool{}}); got != "libaom-av1" {
		t.Fatalf("av1 without svt=%q", got)
	}
	if got := softwareEncoderFor(contracts.VideoCodecH264, capabilities{}); got != "libx264" {
		t.Fatalf("h264=%q", got)
	}
}

// stubFFmpeg writes a fake ffmpeg whose behavior depends on argv content.
func stubFFmpeg(t *testing.T, script string) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "ffmpeg")
	if err := os.WriteFile(path, []byte("#!/bin/sh\n"+script), 0o755); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestKillExecPlan(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })

	// A failing ffmpeg reports its stderr.
	p := encodePlanFor(nil)
	p.settings.FFmpegPath = stubFFmpeg(t, "echo 'encode broke' >&2; exit 3")
	err := tr.execPlan(context.Background(), p, filepath.Join(t.TempDir(), "o.mp4"), "", nil, nil)
	if err == nil || !strings.Contains(err.Error(), "encode broke") {
		t.Fatalf("err=%v, want stderr in the message", err)
	}

	// Success without an output file is a failure too.
	p.settings.FFmpegPath = stubFFmpeg(t, "exit 0")
	err = tr.execPlan(context.Background(), p, filepath.Join(t.TempDir(), "o.mp4"), "", nil, nil)
	if err == nil || !strings.Contains(err.Error(), "no output produced") {
		t.Fatalf("err=%v, want 'no output produced'", err)
	}

	// A working stub: writes the output (last argv), onCmd sees the child,
	// and -progress only appears when a sampler is installed.
	p.settings.FFmpegPath = stubFFmpeg(t,
		`for last; do :; done
case "$*" in *-progress*) echo 'progress=end' ;; esac
printf 'x' > "$last"`)
	out := filepath.Join(t.TempDir(), "ok.mp4")
	var gotCmd *exec.Cmd
	var samples int
	err = tr.execPlan(context.Background(), p, out, "", func(progressSample) { samples++ }, func(c *exec.Cmd) { gotCmd = c })
	if err != nil {
		t.Fatalf("execPlan: %v", err)
	}
	if gotCmd == nil || gotCmd.Process == nil {
		t.Fatal("onCmd never received the child")
	}
	if samples == 0 {
		t.Fatal("progress sampler never ran")
	}
	if fi, statErr := os.Stat(out); statErr != nil || fi.Size() != 1 {
		t.Fatalf("output missing: %v", statErr)
	}

	// HLS output skips the post-run size check (segments land in a dir).
	hls := encodePlanFor(func(p *encodePlan) { p.delivery = contracts.TranscodeDeliveryHLS })
	hls.settings.FFmpegPath = p.settings.FFmpegPath
	if err := tr.execPlan(context.Background(), hls, filepath.Join(t.TempDir(), "raw.m3u8"), "", nil, nil); err != nil {
		t.Fatalf("hls execPlan: %v", err)
	}
}

// --- ffmpeg.go ---------------------------------------------------------

func TestKillStreamHelpers(t *testing.T) {
	cases := []struct {
		s   stream
		ten bool
		fps float64
	}{
		{stream{BitsPerRaw: "8"}, false, 0},
		{stream{BitsPerRaw: "9"}, true, 0},
		{stream{BitsPerRaw: "10"}, true, 0},
		{stream{BitsPerRaw: "abc", PixelFormat: "yuv420p"}, false, 0},
		{stream{PixelFormat: "yuv420p10le"}, true, 0},
		{stream{PixelFormat: "p010le"}, true, 0},
		{stream{PixelFormat: "yuv420p"}, false, 0},
		{stream{FrameRate: "30000/1001"}, false, 29.97},
		{stream{FrameRate: "25/1"}, false, 25},
		{stream{FrameRate: "0/0"}, false, 0},
		{stream{FrameRate: "25/0"}, false, 0},
		{stream{FrameRate: "-25/1"}, false, 0},
		{stream{FrameRate: "abc"}, false, 0},
	}
	for _, c := range cases {
		if got := c.s.is10Bit(); got != c.ten {
			t.Errorf("%+v is10Bit=%v, want %v", c.s, got, c.ten)
		}
		if c.fps == 0 {
			if got := c.s.frameRate(); got != 0 {
				t.Errorf("%+v frameRate=%v, want 0", c.s, got)
			}
		} else if got := c.s.frameRate(); got < c.fps-0.01 || got > c.fps+0.01 {
			t.Errorf("%+v frameRate=%v, want ~%v", c.s, got, c.fps)
		}
	}
	for _, d := range []struct {
		in   string
		want float64
	}{
		{"60.5", 60.5}, {"0", 0}, {"-3", 0}, {"", 0}, {"junk", 0},
	} {
		var r mediaReport
		r.Format.Duration = d.in
		if got := r.durationSeconds(); got != d.want {
			t.Errorf("durationSeconds(%q)=%v, want %v", d.in, got, d.want)
		}
	}
	for _, b := range []struct {
		in   string
		want int
	}{
		{"5000000", 5000}, {"0", 0}, {"-4", 0}, {"junk", 0}, {"  1234567 ", 1234},
	} {
		if got := sourceKbps(b.in); got != b.want {
			t.Errorf("sourceKbps(%q)=%d, want %d", b.in, got, b.want)
		}
	}
	for _, b := range []struct {
		in      string
		want    int
		wantErr bool
	}{
		{"1234.5kbits/s", 1235, false}, {"100kbit/s", 100, false},
		{"N/A", 0, true}, {"", 0, true}, {"abc", 0, true},
	} {
		got, err := parseBitrateKbps(b.in)
		if (err != nil) != b.wantErr || got != b.want {
			t.Errorf("parseBitrateKbps(%q)=%d,%v", b.in, got, err)
		}
	}
}

func TestKillLegacyCopy(t *testing.T) {
	mk := func(streams ...stream) mediaReport { return mediaReport{Streams: streams} }
	cases := []struct {
		name         string
		report       mediaReport
		video, audio bool
	}{
		{"h264+aac", mk(stream{CodecType: "video", CodecName: "h264"}, stream{CodecType: "audio", CodecName: "aac"}), true, true},
		{"h264+ac3", mk(stream{CodecType: "video", CodecName: "h264"}, stream{CodecType: "audio", CodecName: "ac3"}), true, false},
		{"hevc first", mk(stream{CodecType: "video", CodecName: "hevc"}, stream{CodecType: "audio", CodecName: "aac"}), false, false},
		{"second video ignored", mk(stream{CodecType: "video", CodecName: "h264"}, stream{CodecType: "video", CodecName: "hevc"}, stream{CodecType: "audio", CodecName: "aac"}), true, true},
		{"no audio", mk(stream{CodecType: "video", CodecName: "h264"}), true, true},
		{"no video", mk(stream{CodecType: "audio", CodecName: "aac"}), false, false},
	}
	for _, c := range cases {
		v, a := legacyCopy(c.report)
		if v != c.video || a != c.audio {
			t.Errorf("%s: legacyCopy=%v,%v want %v,%v", c.name, v, a, c.video, c.audio)
		}
	}
	if got := methodName(true, true); got != "remux" {
		t.Fatalf("methodName=%q", got)
	}
	if got := methodName(true, false); got != "transcode" {
		t.Fatalf("methodName=%q", got)
	}
}

func TestKillConversionArgs(t *testing.T) {
	tmp := "/tmp/out.tmp"
	legacy := sourceSpec{Path: "/media/in.mkv", Profile: legacyProfile}
	report := mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "h264", PixelFormat: "yuv420p"},
		{Index: 1, CodecType: "audio", CodecName: "aac"},
	}}
	args, method, err := conversionArgs(legacy, report, tmp)
	if err != nil || method != "remux" {
		t.Fatalf("legacy: method=%q err=%v", method, err)
	}
	if !hasArg(args, "-map", "0:v:0") || !hasArg(args, "-map", "0:a?") || !hasArg(args, "-c:v", "copy") {
		t.Fatalf("legacy args=%v", args)
	}

	modern := sourceSpec{Path: "/media/in.mkv", Profile: profile}
	// No video at all.
	_, _, err = conversionArgs(modern, mediaReport{Streams: []stream{{CodecType: "audio", CodecName: "aac"}}}, tmp)
	if err == nil || !strings.Contains(err.Error(), "no video") {
		t.Fatalf("no-video err=%v", err)
	}
	// HDR is refused on the v2 path.
	hdr := mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "hevc", ColorTransfer: "smpte2084"},
	}}
	if _, _, err = conversionArgs(modern, hdr, tmp); err == nil || !strings.Contains(err.Error(), "tone-map") {
		t.Fatalf("hdr err=%v", err)
	}
	// yuvj420p is also a legal copy pixel format.
	args, method, err = conversionArgs(modern, mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "h264", PixelFormat: "yuvj420p"},
		{Index: 2, CodecType: "audio", CodecName: "aac"},
	}}, tmp)
	if err != nil || method != "remux" || !hasArg(args, "-map", "0:0", "-map", "0:2") {
		t.Fatalf("yuvj420p: method=%q err=%v args=%v", method, err, args)
	}
	// Explicit audio selection, and its rejection.
	two := 7
	sel := sourceSpec{Path: "/m", Profile: profile, AudioStream: &two}
	args, _, err = conversionArgs(sel, mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "h264", PixelFormat: "yuv420p"},
		{Index: 1, CodecType: "audio", CodecName: "aac", Default: 1},
		{Index: 7, CodecType: "audio", CodecName: "ac3"},
	}}, tmp)
	if err != nil || !hasArg(args, "-map", "0:7") || !hasArg(args, "-c:a", "aac") {
		t.Fatalf("selected audio: err=%v args=%v", err, args)
	}
	nine := 9
	sel.AudioStream = &nine
	if _, _, err = conversionArgs(sel, report, tmp); err == nil || !strings.Contains(err.Error(), "audio track") {
		t.Fatalf("bad audio index err=%v", err)
	}
	// A video index passed as audio_stream must not match.
	zero := 0
	sel.AudioStream = &zero
	if _, _, err = conversionArgs(sel, report, tmp); err == nil {
		t.Fatal("video index accepted as audio_stream")
	}
	// No audio at all maps -an.
	args, _, err = conversionArgs(modern, mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "h264", PixelFormat: "yuv420p"},
	}}, tmp)
	if err != nil || !hasArg(args, "-an") || !hasArg(args, "-c:a", "copy") {
		t.Fatalf("an: err=%v args=%v", err, args)
	}
	// Subtitle selection must be a subtitle track and text-convertible.
	sub := 3
	selSub := sourceSpec{Path: "/m", Profile: profile, SubtitleStream: &sub}
	if _, _, err = conversionArgs(selSub, report, tmp); err == nil || !strings.Contains(err.Error(), "subtitle track") {
		t.Fatalf("bad sub err=%v", err)
	}
	imgReport := mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "h264", PixelFormat: "yuv420p"},
		{Index: 3, CodecType: "subtitle", CodecName: "hdmv_pgs_subtitle"},
	}}
	if _, _, err = conversionArgs(selSub, imgReport, tmp); err == nil || !strings.Contains(err.Error(), "WebVTT") {
		t.Fatalf("image sub err=%v", err)
	}
	txtReport := mediaReport{Streams: []stream{
		{Index: 0, CodecType: "video", CodecName: "h264", PixelFormat: "yuv420p"},
		{Index: 3, CodecType: "subtitle", CodecName: "subrip"},
	}}
	if _, _, err = conversionArgs(selSub, txtReport, tmp); err != nil {
		t.Fatalf("text sub err=%v", err)
	}
}

func TestKillProbeMediaWith(t *testing.T) {
	if _, err := probeMediaWith("", "/nonexistent-file"); err == nil {
		t.Fatal("want error for missing file")
	}
	ffprobe := filepath.Join(t.TempDir(), "ffprobe")
	script := `#!/bin/sh
printf '%s' '{"streams":[{"index":0,"codec_type":"video","codec_name":"h264","disposition":{"default":1}}],"format":{"duration":"12.5"}}'
`
	if err := os.WriteFile(ffprobe, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	report, err := probeMediaWith(ffprobe, "/anything")
	if err != nil {
		t.Fatal(err)
	}
	if len(report.Streams) != 1 || report.Streams[0].Default != 1 {
		t.Fatalf("report=%+v", report.Streams)
	}
	if report.durationSeconds() != 12.5 {
		t.Fatalf("duration=%v", report.durationSeconds())
	}
	// Garbage output is an error.
	bad := filepath.Join(t.TempDir(), "ffprobe")
	if err := os.WriteFile(bad, []byte("#!/bin/sh\necho notjson\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	if _, err := probeMediaWith(bad, "/x"); err == nil {
		t.Fatal("want JSON error")
	}
	// Convert without ffprobe only survives for the legacy profile.
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	if _, err := tr.convertMedia(sourceSpec{Path: "/nope", Profile: profile}, "/tmp/x.mp4", nil); err == nil {
		t.Fatal("want dependency-unavailable for browser profile without ffprobe")
	}
}

func TestKillConsumeProgressEdges(t *testing.T) {
	var samples []progressSample
	feed := func(input string, total float64) {
		samples = nil
		consumeProgress(strings.NewReader(input), total, func(s progressSample) { samples = append(samples, s) })
	}
	// A block carrying nothing true is never reported.
	feed("out_time_us=-5\nfps=0\nspeed=0x\nbitrate=N/A\nprogress=continue\n", 0)
	if len(samples) != 0 {
		t.Fatalf("non-informative block reported: %+v", samples)
	}
	// With a known duration even fraction 0 is a real reading.
	feed("out_time_us=-5\nprogress=continue\n", 10)
	if len(samples) != 1 || samples[0].Fraction != 0 || !samples[0].HasFraction {
		t.Fatalf("fraction-0 samples=%+v", samples)
	}
	// out_time_ms is the same field on newer ffmpeg.
	feed("out_time_ms=5000000\nprogress=continue\n", 10)
	if len(samples) != 1 || samples[0].Fraction != 0.5 || !samples[0].HasFraction {
		t.Fatalf("samples=%+v", samples)
	}
	// progress=end always reports fraction 1, even without duration.
	feed("progress=end\n", 0)
	if len(samples) != 1 || samples[0].Fraction != 1 {
		t.Fatalf("end samples=%+v", samples)
	}
	// Fraction clamps at 1.
	feed("out_time_us=20000000\nprogress=continue\n", 10)
	if len(samples) != 1 || samples[0].Fraction != 1 {
		t.Fatalf("clamp samples=%+v", samples)
	}
	// Metrics alone (unknown duration) still report.
	feed("fps=59.9\nspeed=2.5x\nbitrate=512.2kbits/s\nprogress=continue\n", 0)
	if len(samples) != 1 || samples[0].FPS != 59.9 || samples[0].Speed != 2.5 || samples[0].BitrateKbps != 512 || samples[0].HasFraction {
		t.Fatalf("metrics samples=%+v", samples)
	}
	// Malformed lines are skipped.
	feed("no-equals-here\n\nprogress=continue\n", 10)
	if len(samples) != 1 {
		t.Fatalf("samples=%+v", samples)
	}
}

func TestKillExtractSubtitleStream(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	spec := sourceSpec{Path: "/media/in.mkv"}
	out := filepath.Join(t.TempDir(), "sub.vtt")

	fail := stubFFmpeg(t, "echo nope >&2; exit 1")
	if err := tr.extractSubtitleStream(spec, 3, out, "webvtt", fail); err == nil || !strings.Contains(err.Error(), "nope") {
		t.Fatalf("err=%v", err)
	}
	if _, statErr := os.Stat(out); statErr == nil {
		t.Fatal("failed extraction left an output file")
	}

	empty := stubFFmpeg(t, "exit 0")
	if err := tr.extractSubtitleStream(spec, 3, out, "webvtt", empty); err == nil || !strings.Contains(err.Error(), "no subtitle") {
		t.Fatalf("err=%v", err)
	}

	ok := stubFFmpeg(t, `for last; do :; done
printf 'WEBVTT\n' > "$last"`)
	if err := tr.extractSubtitleStream(spec, 3, out, "webvtt", ok); err != nil {
		t.Fatalf("extract: %v", err)
	}
	raw, err := os.ReadFile(out)
	if err != nil || string(raw) != "WEBVTT\n" {
		t.Fatalf("out=%q err=%v", raw, err)
	}
}

func TestKillTextSubtitleAndHDRHelpers(t *testing.T) {
	for codec, want := range map[string]bool{
		"subrip": true, "srt": true, "ass": true, "ssa": true, "webvtt": true, "mov_text": true,
		"hdmv_pgs_subtitle": false, "dvd_subtitle": false, "": false,
	} {
		if got := textSubtitleCodec(codec); got != want {
			t.Errorf("textSubtitleCodec(%q)=%v", codec, got)
		}
		if got := imageSubtitleCodec(codec); got != (codec == "hdmv_pgs_subtitle" || codec == "dvd_subtitle" || codec == "dvb_subtitle" || codec == "xsub") {
			t.Errorf("imageSubtitleCodec(%q)=%v", codec, got)
		}
	}
	if !hdrVideo(stream{ColorTransfer: "ARIB-STD-B67"}) {
		t.Error("hlg transfer must be HDR")
	}
	if hdrVideo(stream{ColorTransfer: "bt709"}) {
		t.Error("bt709 is not HDR")
	}
	if !interlacedStream(&stream{FieldOrder: "tb"}) || interlacedStream(&stream{FieldOrder: "progressive"}) || interlacedStream(nil) {
		t.Error("interlaced detection wrong")
	}
	if hdrStream(nil) {
		t.Error("nil stream is not HDR")
	}
}

// --- hls.go ------------------------------------------------------------

func writeRawPlaylist(t *testing.T, dir, body string) {
	t.Helper()
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, hlsRawPlaylist), []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
}

func TestKillParseHLSPlaylistEdges(t *testing.T) {
	dir := t.TempDir()
	writeRawPlaylist(t, dir, strings.Join([]string{
		"#EXTM3U",
		"#EXT-X-VERSION:7",
		"", // blank lines are legal padding
		"#EXT-X-MEDIA-SEQUENCE:7",
		"#EXTINF:4.000", // no trailing comma
		"seg00007.m4s",
		"#EXT-X-DISCONTINUITY",
		"#EXT-X-UNKNOWN-TAG:foo",
		"#EXTINF:2.500,title",
		"seg00008.m4s",
		"#EXT-X-ENDLIST",
	}, "\n"))
	segs, ended, err := parseHLSPlaylist(filepath.Join(dir, hlsRawPlaylist))
	if err != nil || !ended {
		t.Fatalf("segs=%v ended=%v err=%v", segs, ended, err)
	}
	if len(segs) != 2 {
		t.Fatalf("segs=%v", segs)
	}
	if segs[0].MediaSeq != 7 || segs[0].Duration != 4 || segs[0].StartSec != 0 || segs[0].EndSec != 4 || segs[0].IsDiscont {
		t.Fatalf("seg0=%+v", segs[0])
	}
	if segs[1].MediaSeq != 8 || segs[1].Duration != 2.5 || segs[1].StartSec != 4 || segs[1].EndSec != 6.5 || !segs[1].IsDiscont {
		t.Fatalf("seg1=%+v", segs[1])
	}
	if _, _, err := parseHLSPlaylist(filepath.Join(dir, "missing.m3u8")); err == nil {
		t.Fatal("want error for missing playlist")
	}
}

func TestKillWriteIndexPlaylistDetails(t *testing.T) {
	dir := t.TempDir()
	writeRawPlaylist(t, dir, strings.Join([]string{
		"#EXTM3U",
		"#EXT-X-MEDIA-SEQUENCE:3",
		"#EXT-X-MAP:URI=\"init.mp4\"",
		"#EXT-X-DISCONTINUITY",
		"#EXTINF:6.6,",
		"seg00003.m4s",
		"#EXT-X-DISCONTINUITY",
		"#EXTINF:4.4,",
		"seg00004.m4s",
		"#EXTINF:2.0,",
		"gone.m4s",
		"#EXT-X-ENDLIST",
	}, "\n"))
	// Only two of three segments still exist.
	for _, name := range []string{"seg00003.m4s", "seg00004.m4s"} {
		if err := os.WriteFile(filepath.Join(dir, name), []byte("x"), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	if err := writeIndexPlaylist(dir, 6); err != nil {
		t.Fatal(err)
	}
	raw, err := os.ReadFile(filepath.Join(dir, hlsIndexPlaylist))
	if err != nil {
		t.Fatal(err)
	}
	index := string(raw)
	// Target duration is the rounded-up max segment: 6.6 -> 7.
	if !strings.Contains(index, "#EXT-X-TARGETDURATION:7\n") {
		t.Fatalf("index=%q", index)
	}
	if !strings.Contains(index, "#EXT-X-MEDIA-SEQUENCE:3\n") || !strings.Contains(index, "#EXT-X-MAP:URI=\"init.mp4\"\n") {
		t.Fatalf("index=%q", index)
	}
	// The first kept segment's discontinuity is dropped; the middle one stays.
	if strings.Count(index, "#EXT-X-DISCONTINUITY") != 1 {
		t.Fatalf("index=%q, want exactly one discontinuity", index)
	}
	if strings.Contains(index, "gone.m4s") || !strings.Contains(index, "#EXT-X-ENDLIST") {
		t.Fatalf("index=%q", index)
	}

	// No surviving segments: target falls back, no MEDIA-SEQUENCE.
	dir2 := t.TempDir()
	writeRawPlaylist(t, dir2, "#EXTM3U\n#EXTINF:1.0,\nmissing.m4s\n")
	if err := writeIndexPlaylist(dir2, 9); err != nil {
		t.Fatal(err)
	}
	raw, _ = os.ReadFile(filepath.Join(dir2, hlsIndexPlaylist))
	if !strings.Contains(string(raw), "#EXT-X-TARGETDURATION:9\n") || strings.Contains(string(raw), "MEDIA-SEQUENCE") {
		t.Fatalf("empty index=%q", raw)
	}
	if hlsPlayable(dir2) {
		t.Fatal("index with zero segments is not playable")
	}
	// Playable requires the index file, its init segment, and a segment.
	if hlsPlayable(dir) {
		t.Fatal("index references init.mp4 which is absent")
	}
	if err := os.WriteFile(filepath.Join(dir, "init.mp4"), []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	if !hlsPlayable(dir) {
		t.Fatal("complete index must be playable")
	}
}

func TestKillClientPositionSec(t *testing.T) {
	dir := t.TempDir()
	writeRawPlaylist(t, dir, "#EXTM3U\n#EXTINF:6.0,\nseg0.m4s\n#EXTINF:6.0,\nseg1.m4s\n")
	if got := clientPositionSec(dir, -1); got != 0 {
		t.Fatalf("negative=%v", got)
	}
	if got := clientPositionSec(dir, 0); got != 6 {
		t.Fatalf("seg0=%v", got)
	}
	if got := clientPositionSec(dir, 99); got != 12 {
		t.Fatalf("past-end=%v, want the produced tail", got)
	}
	if got := clientPositionSec(t.TempDir(), 0); got != 0 {
		t.Fatalf("missing playlist=%v", got)
	}
}

func TestKillThrottleBoundaries(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("SIGSTOP/SIGCONT")
	}
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	cmd := exec.Command("sleep", "30")
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = cmd.Process.Kill() })

	// Exactly at the budget: not paused.
	j := throttleJob(t, tr, "kill-throttle-eq", 6, cmd)
	tr.applyThrottle(j)
	if j.isPaused(t, tr) {
		t.Fatal("ahead == budget must not pause")
	}

	// Paused, still above half the budget: stays paused.
	tr.mu.Lock()
	j.paused = true
	j.settings.ThrottleAheadSec = 4
	j.clientSegment = -1
	tr.mu.Unlock()
	tr.applyThrottle(j) // produced 6s, ahead 6, limit/2 = 2
	if !j.isPaused(t, tr) {
		t.Fatal("still ahead of half the budget must stay paused")
	}

	// No process: nothing happens.
	j2 := throttleJob(t, tr, "kill-throttle-nocmd", 5, nil)
	tr.applyThrottle(j2)
	if j2.isPaused(t, tr) {
		t.Fatal("nil cmd must not mark paused")
	}
}

func TestKillDeleteConsumedSegmentsEdges(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	settings := contracts.DefaultTranscodeSettings()
	settings.SegmentDeletion = true
	settings.SegmentKeepSec = 4
	session := "kill-delete"
	dir := tr.pathsFor(session, settings).hlsDir
	writeRawPlaylist(t, dir, strings.Join([]string{
		"#EXTM3U",
		"#EXTINF:6.0,", "seg0.m4s",
		"#EXTINF:6.0,", "seg1.m4s",
		"#EXTINF:6.0,", "seg2.m4s",
	}, "\n"))
	for _, n := range []string{"seg0.m4s", "seg1.m4s", "seg2.m4s"} {
		if err := os.WriteFile(filepath.Join(dir, n), []byte("x"), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	j := &job{spec: sourceSpec{Session: session, Settings: settings}, settings: settings,
		delivery: contracts.TranscodeDeliveryHLS, clientSegment: 1} // client at 12s; cutoff 8s
	tr.deleteConsumedSegments(j)
	if _, err := os.Stat(filepath.Join(dir, "seg0.m4s")); err == nil {
		t.Fatal("seg0 (ends 6s <= cutoff 8s) must be removed")
	}
	for _, n := range []string{"seg1.m4s", "seg2.m4s"} {
		if _, err := os.Stat(filepath.Join(dir, n)); err != nil {
			t.Fatalf("%s must be kept: %v", n, err)
		}
	}
	// clientSegment out of range -> clientSec 0 -> cutoff negative -> nothing removed.
	j.clientSegment = 99
	tr.deleteConsumedSegments(j)
	if _, err := os.Stat(filepath.Join(dir, "seg1.m4s")); err != nil {
		t.Fatal("out-of-range position must not delete")
	}
	// Disabled and non-HLS jobs are no-ops.
	j.settings.SegmentDeletion = false
	tr.deleteConsumedSegments(j)
	j.settings.SegmentDeletion = true
	j.delivery = "progressive"
	tr.deleteConsumedSegments(j)
}

func TestKillWriteFileAtomicRenameError(t *testing.T) {
	dir := t.TempDir()
	target := filepath.Join(dir, "asdir")
	if err := os.MkdirAll(target, 0o700); err != nil {
		t.Fatal(err)
	}
	// Renaming a file onto an existing directory fails on POSIX.
	if err := writeFileAtomic(target, []byte("x")); err == nil {
		t.Fatal("want rename error")
	}
	if _, err := os.Stat(target + ".tmp"); err == nil {
		t.Fatal("tmp must be cleaned on rename failure")
	}
	if err := writeJSONAtomic(target, map[string]int{"a": 1}); err == nil {
		t.Fatal("writeJSONAtomic must surface the rename error")
	}
	// Marshal failure propagates.
	if err := writeJSONAtomic(filepath.Join(dir, "ok.json"), func() {}); err == nil {
		t.Fatal("want marshal error")
	}
	if err := readJSON(filepath.Join(dir, "absent.json"), &struct{}{}); err == nil {
		t.Fatal("want read error")
	}
	var v map[string]int
	if err := writeJSONAtomic(filepath.Join(dir, "ok.json"), map[string]int{"a": 1}); err != nil {
		t.Fatal(err)
	}
	if err := readJSON(filepath.Join(dir, "ok.json"), &v); err != nil || v["a"] != 1 {
		t.Fatalf("round trip: %v %v", v, err)
	}
}

// --- v3.go -------------------------------------------------------------

func TestKillInvokeV3Routing(t *testing.T) {
	tr, src := v3Fixture(t, Config{})
	for _, action := range []string{
		contracts.TranscodeCancelAction, contracts.TranscodePositionAction,
		contracts.TranscodeStatusAction, contracts.TranscodeResolveAction,
	} {
		if _, err := tr.invokeV3(v3Request(action, src)); err == nil {
			t.Errorf("%s without session must fail", action)
		}
	}
	// Session actions do not need file_path.
	if _, err := tr.invokeV3(v3Request(contracts.TranscodeCancelAction, "")); err == nil {
		t.Fatal("cancel without session must fail")
	}
	// A start with no file fails before spec building.
	if _, err := tr.invokeV3(v3Request(contracts.TranscodeStartAction, "")); err == nil {
		t.Fatal("start without file_path must fail")
	}
	// A client-supplied session must match the derived identity.
	tr.probeFn = func(string, string) (mediaReport, error) { return webSafeReport(), nil }
	tr.capsFn = func(contracts.TranscodeSettings) capabilities { return testCaps() }
	in := v3Request(contracts.TranscodeInspectAction, src)
	in.Session = strings.Repeat("a", 64)
	if _, err := tr.invokeV3(in); err == nil {
		t.Fatal("foreign session must be rejected")
	}
	// Unknown action on an existing file.
	in = v3Request("bogus", src)
	if _, err := tr.invokeV3(in); err == nil {
		t.Fatal("unknown action must fail")
	}
	// A missing source is unavailable.
	in = v3Request(contracts.TranscodeInspectAction, filepath.Join(t.TempDir(), "gone.mkv"))
	if _, err := tr.invokeV3(in); err == nil {
		t.Fatal("missing source must fail")
	}
	// Negative start_sec is invalid.
	in = v3Request(contracts.TranscodeInspectAction, src)
	in.StartSec = -1
	if _, err := tr.invokeV3(in); err == nil {
		t.Fatal("negative start_sec must fail")
	}
}

func TestKillBySessionV3(t *testing.T) {
	tr, _ := v3Fixture(t, Config{})
	session := strings.Repeat("ab", 32)
	j := &job{spec: sourceSpec{Path: "/media/real.mkv", Session: session},
		state: contracts.TranscodeRunning, v3: true, plan: &encodePlan{}}
	tr.mu.Lock()
	tr.jobs[session] = j
	tr.mu.Unlock()

	// A mismatched source path is refused.
	if _, err := tr.bySessionV3("/media/other.mkv", session, false); err == nil {
		t.Fatal("path mismatch must fail")
	}
	// touch=false does not refresh lastTouch.
	j.lastTouch = 7
	if _, err := tr.bySessionV3("/media/real.mkv", session, false); err != nil {
		t.Fatal(err)
	}
	if j.lastTouch != 7 {
		t.Fatal("status poll must not refresh lastTouch")
	}
	if _, err := tr.bySessionV3("/media/real.mkv", session, true); err != nil {
		t.Fatal(err)
	}
	if j.lastTouch == 7 {
		t.Fatal("resolve must refresh lastTouch")
	}

	// Unknown session: invalid id -> not-found; valid id without meta -> not-found.
	if _, err := tr.bySessionV3("", "not-hex", false); err == nil {
		t.Fatal("invalid id must fail")
	}
	if _, err := tr.bySessionV3("", strings.Repeat("be", 32), false); err == nil {
		t.Fatal("unknown session must fail")
	}

	// A cache entry resolves; a path mismatch on it is refused.
	entry := cacheEntry{Session: strings.Repeat("ca", 32), SourcePath: "/media/real.mkv", Method: "remux",
		Delivery: contracts.TranscodeDeliveryProgressive}
	if err := writeJSONAtomic(tr.metaPath(entry.Session), entry); err != nil {
		t.Fatal(err)
	}
	if _, err := tr.bySessionV3("/media/other.mkv", entry.Session, false); err == nil {
		t.Fatal("entry path mismatch must fail")
	}
	st, err := tr.bySessionV3("/media/real.mkv", entry.Session, false)
	if err != nil || st.Session != entry.Session || st.State != contracts.TranscodeReady {
		t.Fatalf("entry status=%+v err=%v", st, err)
	}
}

func TestKillPositionV3(t *testing.T) {
	tr, _ := v3Fixture(t, Config{})
	session := strings.Repeat("cd", 32)
	j := &job{spec: sourceSpec{Path: "/media/real.mkv", Session: session},
		state: contracts.TranscodeRunning, v3: true, delivery: contracts.TranscodeDeliveryHLS,
		settings: contracts.DefaultTranscodeSettings(), plan: &encodePlan{}, clientSegment: -1}
	tr.mu.Lock()
	tr.jobs[session] = j
	tr.mu.Unlock()

	if _, err := tr.positionV3("missing", "", 0); err == nil {
		t.Fatal("unknown session must fail")
	}
	if _, err := tr.positionV3(session, "/media/other.mkv", 0); err == nil {
		t.Fatal("path mismatch must fail")
	}
	// A negative index is not a position.
	if _, err := tr.positionV3(session, "", -3); err != nil {
		t.Fatal(err)
	}
	if j.clientSegment != -1 {
		t.Fatalf("clientSegment=%d, want unchanged", j.clientSegment)
	}
	if _, err := tr.positionV3(session, "", 4); err != nil {
		t.Fatal(err)
	}
	if j.clientSegment != 4 {
		t.Fatalf("clientSegment=%d", j.clientSegment)
	}
	// Boundary: index 0 is a real position (the >= guard must admit it).
	if _, err := tr.positionV3(session, "", 0); err != nil {
		t.Fatal(err)
	}
	if j.clientSegment != 0 {
		t.Fatalf("clientSegment=%d, want 0", j.clientSegment)
	}
}

func TestKillCancelV3Paths(t *testing.T) {
	tr, _ := v3Fixture(t, Config{})
	mk := func(session string) *job {
		j := &job{spec: sourceSpec{Path: "/media/real.mkv", Session: session, Profile: "p"},
			state: contracts.TranscodeQueued, v3: true, done: make(chan struct{}),
			settings: contracts.DefaultTranscodeSettings(), plan: &encodePlan{}}
		tr.mu.Lock()
		tr.jobs[session] = j
		tr.mu.Unlock()
		return j
	}
	s := strings.Repeat("ef", 32)
	mk(s)
	if st := tr.cancelV3(s, "/media/other.mkv", ""); st.State != contracts.TranscodeIdle {
		t.Fatalf("path-mismatch cancel=%+v, want idle without touching the job", st)
	}
	if _, ok := tr.jobs[s]; !ok {
		t.Fatal("mismatched cancel removed the job")
	}
	// A different user cannot cancel.
	j := mk(s)
	j.userID = "alice"
	if st := tr.cancelV3(s, "", "bob"); st.State != contracts.TranscodeIdle {
		t.Fatalf("foreign-user cancel=%+v", st)
	}
	if _, ok := tr.jobs[s]; !ok {
		t.Fatal("foreign-user cancel removed the job")
	}
	// Legacy jobs are not v3-cancellable.
	j.userID = ""
	j.v3 = false
	if st := tr.cancelV3(s, "", ""); st.State != contracts.TranscodeIdle {
		t.Fatalf("legacy cancel=%+v", st)
	}
	if _, ok := tr.jobs[s]; !ok {
		t.Fatal("legacy job was removed by a v3 cancel")
	}
	// A queued v3 job is dropped before it ever runs.
	j.v3 = true
	st := tr.cancelV3(s, "", "admin")
	if st.State != contracts.TranscodeIdle || st.Profile != "p" {
		t.Fatalf("cancel=%+v", st)
	}
	if _, ok := tr.jobs[s]; ok {
		t.Fatal("cancelled job still tracked")
	}

	// Finished-entry cancel: id must be valid, meta must exist, path must match.
	if st := tr.cancelFinishedV3("!!", ""); st.Session != "!!" || st.State != contracts.TranscodeIdle {
		t.Fatalf("invalid id cancel=%+v", st)
	}
	entry := cacheEntry{Session: strings.Repeat("01", 32), SourcePath: "/media/real.mkv", Profile: "p"}
	if err := writeJSONAtomic(tr.metaPath(entry.Session), entry); err != nil {
		t.Fatal(err)
	}
	if st := tr.cancelFinishedV3(entry.Session, "/media/other.mkv"); st.Profile != "" {
		t.Fatalf("mismatched entry cancel=%+v", st)
	}
	if _, err := os.Stat(tr.metaPath(entry.Session)); err != nil {
		t.Fatal("mismatched cancel removed the entry")
	}
	if st := tr.cancelFinishedV3(entry.Session, ""); st.Profile != "p" {
		t.Fatalf("entry cancel=%+v", st)
	}
	if _, err := os.Stat(tr.metaPath(entry.Session)); err == nil {
		t.Fatal("cancelled entry still on disk")
	}
}

func TestKillActiveStreams(t *testing.T) {
	tr, _ := v3Fixture(t, Config{})
	if n := tr.activeStreamsFor("", ""); n != 0 {
		t.Fatalf("anonymous count=%d", n)
	}
	if n := tr.activeStreamsForLocked("", ""); n != 0 {
		t.Fatalf("locked anonymous count=%d", n)
	}
	mk := func(session, user, state string) {
		tr.jobs[session] = &job{userID: user, state: state, spec: sourceSpec{Session: session}, done: make(chan struct{})}
	}
	mk("s1", "alice", contracts.TranscodeRunning)
	mk("s2", "alice", contracts.TranscodeQueued)
	mk("s3", "alice", contracts.TranscodeReady) // finished sessions do not count
	mk("s4", "bob", contracts.TranscodeRunning)
	if n := tr.activeStreamsFor("alice", ""); n != 2 {
		t.Fatalf("alice=%d, want 2", n)
	}
	if n := tr.activeStreamsFor("alice", "s1"); n != 1 {
		t.Fatalf("alice except s1=%d, want 1", n)
	}
	if n := tr.activeStreamsFor("bob", ""); n != 1 {
		t.Fatalf("bob=%d, want 1", n)
	}
}

func TestKillPolicyAllowsMethod(t *testing.T) {
	if err := policyAllowsMethod(nil, "transcode"); err != nil {
		t.Fatal("nil policy is unrestricted")
	}
	p := &contracts.TranscodePolicy{}
	if err := policyAllowsMethod(p, "extract"); err != nil {
		t.Fatal("a sidecar is never a video transcode")
	}
	if err := policyAllowsMethod(&contracts.TranscodePolicy{AllowRemux: false}, "remux"); err == nil {
		t.Fatal("remux gate")
	}
	if err := policyAllowsMethod(&contracts.TranscodePolicy{AllowRemux: true}, "remux"); err != nil {
		t.Fatal(err)
	}
	if err := policyAllowsMethod(&contracts.TranscodePolicy{AllowVideoTranscode: false}, "transcode"); err == nil {
		t.Fatal("transcode gate")
	}
}

func TestKillStatusV3FromJob(t *testing.T) {
	tr, _ := v3Fixture(t, Config{})
	settings := contracts.DefaultTranscodeSettings()
	session := strings.Repeat("99", 32)
	j := &job{spec: sourceSpec{Session: session, Profile: "p"}, v3: true,
		delivery: contracts.TranscodeDeliveryHLS, settings: settings,
		state: contracts.TranscodeRunning, plan: &encodePlan{}, playable: true}
	st := tr.statusV3FromJob(j)
	if st.PlaylistPath == "" || !st.Playable {
		t.Fatalf("running HLS status=%+v", st)
	}
	j.playable = false
	if st := tr.statusV3FromJob(j); st.Playable {
		t.Fatal("unplayable running session must report playable=false")
	}
	j.delivery = contracts.TranscodeDeliveryProgressive
	j.state = contracts.TranscodeReady
	st = tr.statusV3FromJob(j)
	if st.Path == "" || !st.Playable || st.PlaylistPath != "" {
		t.Fatalf("ready progressive status=%+v", st)
	}
	j.state = contracts.TranscodeQueued
	j.progress = 0.5
	if st := tr.statusV3FromJob(j); st.Progress != 0.5 {
		t.Fatalf("queued status=%+v", st)
	}
	j.state = contracts.TranscodeFailed
	if st := tr.statusV3FromJob(j); st.Progress != 0 || st.Path != "" {
		t.Fatalf("failed status=%+v", st)
	}
	// A subtitle sidecar path is only reported when one exists.
	j.state = contracts.TranscodeReady
	j.hasSubtitle = true
	if st := tr.statusV3FromJob(j); st.SubtitlePath == "" {
		t.Fatalf("subtitle status=%+v", st)
	}
}

func TestKillSubtitlesV3(t *testing.T) {
	tr, src := v3Fixture(t, Config{})
	// Validation order: file, stream, settings, permission.
	if _, err := tr.subtitlesV3(v3Request("", "")); err == nil {
		t.Fatal("missing file must fail")
	}
	in := v3Request("", src)
	if _, err := tr.subtitlesV3(in); err == nil {
		t.Fatal("missing subtitle_stream must fail")
	}
	sub := 2
	in.SubtitleStream = &sub
	in.Settings.SubtitleMode = "bogus"
	if _, err := tr.subtitlesV3(in); err == nil {
		t.Fatal("invalid settings must fail")
	}
	in.Settings = contracts.DefaultTranscodeSettings()
	no := false
	in.Settings.AllowSubtitleExtraction = &no
	if _, err := tr.subtitlesV3(in); err == nil {
		t.Fatal("disabled extraction must fail")
	}
	in.Settings.AllowSubtitleExtraction = nil
	in.Settings.TranscodeTempPath = filepath.Join(t.TempDir(), "temp")
	// The selected index must be a subtitle stream.
	tr.probeFn = func(string, string) (mediaReport, error) { return subtitleReport("subrip"), nil }
	if _, err := tr.subtitlesV3(in); err == nil {
		t.Fatal("non-subtitle index must fail")
	}
	// An image track cannot become WebVTT.
	tr.probeFn = func(string, string) (mediaReport, error) { return subtitleReport("hdmv_pgs_subtitle"), nil }
	if _, err := tr.subtitlesV3(in); err == nil {
		t.Fatal("image track must fail")
	}
	// Text track extracts through the configured ffmpeg.
	tr.probeFn = func(string, string) (mediaReport, error) { return subtitleReport("subrip"), nil }
	in.Settings.FFmpegPath = stubFFmpeg(t, `for last; do :; done
printf 'WEBVTT\n' > "$last"`)
	st, err := tr.subtitlesV3(in)
	if err != nil {
		t.Fatalf("extract: %v", err)
	}
	if st.State != contracts.TranscodeReady || st.SubtitlePath == "" {
		t.Fatalf("status=%+v", st)
	}
	// Second call hits the sidecar cache (probeFn now fails).
	tr.probeFn = func(string, string) (mediaReport, error) { return mediaReport{}, errors.New("probe gone") }
	st2, err := tr.subtitlesV3(in)
	if err != nil || st2.Session != st.Session || !st2.Cached {
		t.Fatalf("cached=%+v err=%v", st2, err)
	}
	// Extraction failure is a dependency error.
	in2 := v3Request("", src)
	sub2 := 5
	in2.SubtitleStream = &sub2
	in2.Settings = contracts.DefaultTranscodeSettings()
	in2.Settings.FFmpegPath = stubFFmpeg(t, "exit 1")
	tr.probeFn = func(string, string) (mediaReport, error) {
		return mediaReport{Streams: []stream{
			{Index: 0, CodecType: "video", CodecName: "h264"},
			{Index: 5, CodecType: "subtitle", CodecName: "subrip"},
		}}, nil
	}
	if _, err := tr.subtitlesV3(in2); err == nil {
		t.Fatal("failing ffmpeg must fail the request")
	}
}

func TestKillStartV3Admission(t *testing.T) {
	tr, src := v3Fixture(t, Config{})
	// Queue adoption: settings tighten the runtime bounds.
	in := v3Request(contracts.TranscodeStartAction, src)
	in.Settings.QueueSize = 1
	in.Settings.CacheBytes = 1 << 30
	in.Settings.MaxConcurrent = 1
	st, err := tr.startV3(mustSpecV3(t, tr, in), in, in.Settings)
	if err != nil {
		t.Fatalf("start: %v", err)
	}
	if st.State != contracts.TranscodeQueued && st.State != contracts.TranscodeRunning {
		t.Fatalf("state=%q", st.State)
	}
	if tr.config.MaxCacheBytes != 1<<30 || tr.maxConcurrent != 1 || tr.queueLimit != 1 {
		t.Fatalf("adopted bounds: cache=%d conc=%d queue=%d", tr.config.MaxCacheBytes, tr.maxConcurrent, tr.queueLimit)
	}
	// A full queue rejects deterministically (the worker may have already
	// drained the first job, so pending is forced rather than raced).
	src2 := filepath.Join(t.TempDir(), "other.mkv")
	if err := os.WriteFile(src2, []byte("media2"), 0o600); err != nil {
		t.Fatal(err)
	}
	in2 := v3Request(contracts.TranscodeStartAction, src2)
	in2.Settings.QueueSize = 1
	// pending is set well above the limit so a racing worker decrement
	// (job1 leaving the channel) can never drop it below the bound.
	tr.mu.Lock()
	tr.pending = tr.queueLimit + 100
	tr.mu.Unlock()
	if _, err := tr.startV3(mustSpecV3(t, tr, in2), in2, in2.Settings); err == nil {
		t.Fatal("full queue must reject")
	}
}

func mustSpecV3(t *testing.T, tr *Transcoder, in contracts.TranscodeV3Request) sourceSpec {
	t.Helper()
	spec, _, err := tr.specV3(in.FilePath, in)
	if err != nil {
		t.Fatalf("specV3: %v", err)
	}
	return spec
}

func TestKillStartV3PolicyAndJoin(t *testing.T) {
	tr, src := v3Fixture(t, Config{})
	// Seed a ready cache entry for a remux.
	spec, settings, err := tr.specV3(src, v3Request(contracts.TranscodeStartAction, src))
	if err != nil {
		t.Fatal(err)
	}
	media := tr.defaultPaths(spec.Session).media
	if err := os.WriteFile(media, []byte("mp4"), 0o600); err != nil {
		t.Fatal(err)
	}
	entry := cacheEntry{Session: spec.Session, SourcePath: spec.Path, SourceSize: spec.Size,
		SourceModTime: spec.ModTimeNS, Profile: spec.Profile, Method: "remux"}
	if err := writeJSONAtomic(tr.metaPath(spec.Session), entry); err != nil {
		t.Fatal(err)
	}
	in := v3Request(contracts.TranscodeStartAction, src)
	in.Policy = &contracts.TranscodePolicy{AllowRemux: false}
	if _, err := tr.startV3(spec, in, settings); err == nil {
		t.Fatal("cached remux must honor the caller's policy")
	}
	in.Policy = nil
	st, err := tr.startV3(spec, in, settings)
	if err != nil || !st.Cached {
		t.Fatalf("cached start=%+v err=%v", st, err)
	}
}

// --- coordinator.go ----------------------------------------------------

func TestKillNewWithDepsBounds(t *testing.T) {
	dir := t.TempDir()
	tr := newWithDeps(filepath.Join(dir, "a"), Config{QueueSize: maxQueueCapacity + 10, MaxConcurrent: -1, MaxCacheBytes: -1}, nil, nil)
	t.Cleanup(func() { _ = tr.Close() })
	if tr.config.QueueSize != maxQueueCapacity || tr.queueLimit != maxQueueCapacity {
		t.Fatalf("queue clamp: %d/%d", tr.config.QueueSize, tr.queueLimit)
	}
	if tr.config.MaxConcurrent != defaultMaxConcurrent || tr.config.MaxCacheBytes != DefaultMaxCacheBytes {
		t.Fatalf("defaults: %+v", tr.config)
	}
	tr2 := newWithDeps(filepath.Join(dir, "b"), Config{QueueSize: maxQueueCapacity}, nil, time.Now)
	t.Cleanup(func() { _ = tr2.Close() })
	if tr2.queueLimit != maxQueueCapacity {
		t.Fatalf("queue at capacity=%d", tr2.queueLimit)
	}
	// A dir that cannot be created leaves initErr.
	blocked := filepath.Join(dir, "file")
	if err := os.WriteFile(blocked, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	tr3 := newWithDeps(filepath.Join(blocked, "sub"), Config{}, nil, time.Now)
	if tr3.initErr == nil {
		t.Fatal("want initErr for unwritable dir")
	}
	if err := tr3.Health(); err == nil {
		t.Fatal("Health must surface initErr")
	}
	_ = tr3.Close()
}

func TestKillHealthAndLogger(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "cache")
	tr := newWithDeps(dir, Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	if err := tr.Health(); err != nil {
		t.Fatalf("healthy: %v", err)
	}
	// The injected converter still requires ffmpeg absent? No: convert!=nil
	// means the ffmpeg lookup is skipped entirely.
	tr2 := newWithDeps(filepath.Join(t.TempDir(), "c"), Config{}, func(sourceSpec, string, func(progressSample)) (string, error) {
		return "remux", nil
	}, time.Now)
	t.Cleanup(func() { _ = tr2.Close() })
	if err := tr2.Health(); err != nil {
		t.Fatalf("convert seam should not need ffmpeg: %v", err)
	}
	if err := os.RemoveAll(dir); err != nil {
		t.Fatal(err)
	}
	if err := tr.Health(); err == nil {
		t.Fatal("missing cache dir must fail health")
	}
	tr.SetLogger(nil)
	if tr.logger() == nil {
		t.Fatal("logger must never be nil")
	}
}

func TestKillCloseFailsQueuedJobs(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	j := &job{spec: sourceSpec{Session: "s"}, state: contracts.TranscodeQueued, done: make(chan struct{})}
	tr.mu.Lock()
	tr.jobs["s"] = j
	tr.mu.Unlock()
	if err := tr.Close(); err != nil {
		t.Fatal(err)
	}
	if j.state != contracts.TranscodeFailed || j.errCode != "dependency-unavailable" || j.errMessage == "" || j.finishedAt == 0 {
		t.Fatalf("queued job after Close: %+v", j)
	}
	select {
	case <-j.done:
	default:
		t.Fatal("done must be closed")
	}
	// Close is idempotent.
	if err := tr.Close(); err != nil {
		t.Fatal(err)
	}
}

func TestKillSpecAndSessionIdentity(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	src := filepath.Join(t.TempDir(), "in.mkv")
	if err := os.WriteFile(src, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := tr.spec("/nonexistent", "", nil, nil); err == nil {
		t.Fatal("missing file must fail")
	}
	if _, err := tr.spec(src, "bogus-profile", nil, nil); err == nil {
		t.Fatal("unknown profile must fail")
	}
	s1, err := tr.spec(src, "", nil, nil)
	if err != nil {
		t.Fatal(err)
	}
	if s1.Profile != profile || s1.Session == "" {
		t.Fatalf("spec=%+v", s1)
	}
	audio := 3
	s2, err := tr.spec(src, legacyProfile, &audio, nil)
	if err != nil || s2.Profile != legacyProfile || s2.Session == s1.Session {
		t.Fatalf("spec2=%+v err=%v", s2, err)
	}
	// sessionKey reacts to every identity field.
	base := sourceSpec{Path: "/m", Size: 1, ModTimeNS: 2, Profile: "p"}
	other := base
	other.StartSec = 5
	if sessionKey(base) == sessionKey(other) {
		t.Fatal("start_sec must change the session")
	}
	// validSessionID: exactly 64 lowercase hex.
	if !validSessionID(sessionKey(base)) {
		t.Fatal("sessionKey must produce a valid id")
	}
	for _, bad := range []string{"", strings.Repeat("a", 63), strings.Repeat("a", 65), strings.Repeat("A", 64), strings.Repeat("g", 64), strings.Repeat("0", 63) + "!"} {
		if validSessionID(bad) {
			t.Errorf("validSessionID(%q)=true", bad)
		}
	}
	if cloneInt(nil) != nil {
		t.Fatal("cloneInt(nil)")
	}
	if *cloneInt(&audio) != 3 {
		t.Fatal("cloneInt")
	}
}

func TestKillInvokeV2BySession(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	src := filepath.Join(t.TempDir(), "in.mkv")
	if err := os.WriteFile(src, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	in := contracts.TranscodeV2Request{Action: contracts.TranscodeStatusAction, FilePath: src, Session: "whatever"}
	// Missing source is unavailable even for status.
	if _, err := tr.invokeV2(contracts.TranscodeV2Request{Action: contracts.TranscodeStatusAction, FilePath: "/gone", Session: "x"}); err == nil {
		t.Fatal("missing source must fail")
	}
	// Live job with mismatched identity is refused.
	spec, err := tr.spec(src, "", nil, nil)
	if err != nil {
		t.Fatal(err)
	}
	j := &job{spec: spec, state: contracts.TranscodeRunning, done: make(chan struct{})}
	j.spec.Size++ // stale identity
	tr.mu.Lock()
	tr.jobs[spec.Session] = j
	tr.mu.Unlock()
	in.Session = spec.Session
	if _, err := tr.invokeV2(in); err == nil {
		t.Fatal("stale identity must fail")
	}
	// Matching job returns its status without falling through.
	j.spec.Size--
	st, err := tr.invokeV2(in)
	if err != nil || st.State != contracts.TranscodeRunning {
		t.Fatalf("job status=%+v err=%v", st, err)
	}
	// Ready job falls through to the artifact existence check.
	j.state = contracts.TranscodeReady
	if _, err := tr.invokeV2(in); err == nil {
		t.Fatal("ready job without artifact must fail")
	}
	// Valid id, meta exists, but media missing -> not-found.
	delete(tr.jobs, spec.Session)
	entry := cacheEntry{Session: spec.Session, SourcePath: spec.Path, SourceSize: spec.Size,
		SourceModTime: spec.ModTimeNS, Profile: spec.Profile, Method: "remux"}
	if err := writeJSONAtomic(tr.metaPath(spec.Session), entry); err != nil {
		t.Fatal(err)
	}
	if _, err := tr.invokeV2(in); err == nil {
		t.Fatal("missing media must fail")
	}
	// Zero-byte media is also not-found.
	if err := os.WriteFile(tr.mediaPath(spec.Session), nil, 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := tr.invokeV2(in); err == nil {
		t.Fatal("empty media must fail")
	}
	if err := os.WriteFile(tr.mediaPath(spec.Session), []byte("mp4"), 0o600); err != nil {
		t.Fatal(err)
	}
	st, err = tr.invokeV2(in)
	if err != nil || st.State != contracts.TranscodeReady || st.Path == "" {
		t.Fatalf("resolve=%+v err=%v", st, err)
	}
}

func TestKillSlotAndPending(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	// releaseSlot at zero must not go negative.
	tr.releaseSlot()
	if tr.active != 0 {
		t.Fatalf("active=%d", tr.active)
	}
	tr.mu.Lock()
	tr.active = 1
	tr.mu.Unlock()
	tr.releaseSlot()
	if tr.active != 0 {
		t.Fatalf("active=%d", tr.active)
	}
	// finishPending only decrements positive pending.
	j := &job{done: make(chan struct{})}
	tr.finishPending(j)
	if tr.pending != 0 {
		t.Fatalf("pending=%d", tr.pending)
	}
	tr.mu.Lock()
	tr.pending = 1
	tr.mu.Unlock()
	tr.finishPending(j)
	if tr.pending != 0 {
		t.Fatalf("pending=%d", tr.pending)
	}
	// acquireSlot on a closed transcoder fails fast.
	if err := tr.Close(); err != nil {
		t.Fatal(err)
	}
	if tr.acquireSlot() {
		t.Fatal("closed transcoder must not hand out slots")
	}
}

func TestKillPruneJobs(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	// Under the cap nothing happens.
	for i := 0; i < maxTrackedJobs; i++ {
		tr.jobs[string(rune('a'+i%26))+strings.Repeat("x", i)] = &job{state: contracts.TranscodeReady, finishedAt: int64(i), done: make(chan struct{})}
	}
	tr.pruneJobs()
	if len(tr.jobs) != maxTrackedJobs {
		t.Fatalf("pruned at cap: %d", len(tr.jobs))
	}
	// Over the cap: oldest finished first; running jobs never pruned.
	tr.jobs["running"] = &job{state: contracts.TranscodeRunning, done: make(chan struct{})}
	tr.jobs["oldest"] = &job{state: contracts.TranscodeReady, finishedAt: -1, done: make(chan struct{})}
	tr.pruneJobs()
	if _, ok := tr.jobs["running"]; !ok {
		t.Fatal("running job pruned")
	}
	if _, ok := tr.jobs["oldest"]; ok {
		t.Fatal("oldest finished job should be pruned first")
	}
	if len(tr.jobs) != maxTrackedJobs {
		t.Fatalf("after prune=%d", len(tr.jobs))
	}
}

func TestKillTickIdleBoundary(t *testing.T) {
	now := time.Unix(1000, 0)
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, func() time.Time { return now })
	t.Cleanup(func() { _ = tr.Close() })
	settings := contracts.DefaultTranscodeSettings()
	settings.IdleTimeoutSec = 60
	// The running-HLS path only reaches the idle check once the session
	// directory produces a playlist.
	for _, s := range []string{"s1", "s2"} {
		writeRawPlaylist(t, tr.pathsFor(s, settings).hlsDir, "#EXTM3U\n#EXTINF:6.0,\nseg0.m4s\n")
	}
	j := &job{spec: sourceSpec{Session: "s1", Settings: settings}, v3: true,
		delivery: contracts.TranscodeDeliveryHLS, settings: settings,
		state: contracts.TranscodeRunning, lastTouch: 1000 - 60, done: make(chan struct{}), plan: &encodePlan{}}
	tr.jobs["s1"] = j
	tr.tick() // exactly at the limit: not stopped
	if j.state != contracts.TranscodeRunning {
		t.Fatal("at-limit must not be stopped")
	}
	j.lastTouch = 1000 - 61
	tr.tick()
	if j.state != contracts.TranscodeFailed || j.errCode != "idle-timeout" {
		t.Fatalf("over-limit job=%+v", j)
	}
	// lastTouch 0 (never resolved) is never idle-stopped.
	j2 := &job{spec: sourceSpec{Session: "s2", Settings: settings}, v3: true,
		delivery: contracts.TranscodeDeliveryHLS, settings: settings,
		state: contracts.TranscodeRunning, lastTouch: 0, done: make(chan struct{}), plan: &encodePlan{}}
	tr.jobs["s2"] = j2
	tr.tick()
	if j2.state != contracts.TranscodeRunning {
		t.Fatal("lastTouch=0 must not idle-stop")
	}
	// Ready HLS sessions get their playlist regenerated without stopping.
	j3 := &job{spec: sourceSpec{Session: "s3", Settings: settings}, v3: true,
		delivery: contracts.TranscodeDeliveryHLS, settings: settings,
		state: contracts.TranscodeReady, done: make(chan struct{}), plan: &encodePlan{}}
	tr.jobs["s3"] = j3
	tr.tick()
	if j3.state != contracts.TranscodeReady {
		t.Fatal("ready session stopped by tick")
	}
}

func TestKillStopJob(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	// Ready/failed jobs are not stopped.
	j := &job{state: contracts.TranscodeReady, done: make(chan struct{}), spec: sourceSpec{Session: "x"}}
	tr.stopJob(j, "code", "msg")
	if j.state != contracts.TranscodeReady {
		t.Fatal("ready job stopped")
	}
	// Running with a real process: killed, marked, unpaused.
	if runtime.GOOS == "windows" {
		t.Skip("process kill")
	}
	cmd := exec.Command("sleep", "30")
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = cmd.Process.Kill() })
	j2 := &job{state: contracts.TranscodeRunning, cmd: cmd, paused: true, done: make(chan struct{}), spec: sourceSpec{Session: "y"}}
	tr.stopJob(j2, "idle-timeout", "stopped")
	if j2.state != contracts.TranscodeFailed || j2.errCode != "idle-timeout" || j2.paused || !j2.stopped {
		t.Fatalf("stopped job=%+v", j2)
	}
	// nil cmd is fine.
	j3 := &job{state: contracts.TranscodeQueued, done: make(chan struct{}), spec: sourceSpec{Session: "z"}}
	tr.stopJob(j3, "x", "y")
	if j3.state != contracts.TranscodeFailed {
		t.Fatal("queued job not stopped")
	}
}

func TestKillRunLifecycle(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })

	// A job that is not queued never runs.
	j := &job{spec: sourceSpec{Path: "/m", Session: "nr"}, state: contracts.TranscodeReady, done: make(chan struct{})}
	tr.run(j)
	if j.startedAt != 0 {
		t.Fatal("non-queued job ran")
	}

	// Success path: convert seam produces the artifact, record writes meta.
	tr.convert = func(spec sourceSpec, out string, onSample func(progressSample)) (string, error) {
		if onSample != nil {
			onSample(progressSample{Fraction: 0.5, HasFraction: true, FPS: 30, BitrateKbps: 400})
			onSample(progressSample{}) // empty sample leaves metrics alone
		}
		return "transcode", os.WriteFile(out, []byte("mp4"), 0o600)
	}
	j2 := &job{spec: sourceSpec{Path: "/m/in.mkv", Session: "ok", Profile: profile},
		state: contracts.TranscodeQueued, done: make(chan struct{})}
	tr.jobs["ok"] = j2
	tr.run(j2)
	if j2.state != contracts.TranscodeReady || j2.method != "transcode" || j2.progress != 0.5 || j2.fps != 30 || j2.outputKbps != 400 {
		t.Fatalf("job=%+v", j2)
	}
	if _, err := os.Stat(tr.metaPath("ok")); err != nil {
		t.Fatal("no cache entry recorded")
	}
	<-j2.done

	// Convert failure marks the job and removes partial artifacts.
	tr.convert = func(spec sourceSpec, out string, _ func(progressSample)) (string, error) {
		_ = os.WriteFile(out+".tmp", []byte("partial"), 0o600)
		_ = os.WriteFile(out, []byte("partial"), 0o600)
		return "", errors.New("ffmpeg died")
	}
	j3 := &job{spec: sourceSpec{Path: "/m/bad.mkv", Session: "bad", Profile: profile},
		state: contracts.TranscodeQueued, done: make(chan struct{})}
	tr.jobs["bad"] = j3
	tr.run(j3)
	if j3.state != contracts.TranscodeFailed || j3.errCode != "dependency-unavailable" || j3.errMessage == "" {
		t.Fatalf("failed job=%+v", j3)
	}
	paths := tr.defaultPaths("bad")
	if _, err := os.Stat(paths.media); err == nil {
		t.Fatal("partial output not removed")
	}
	if _, err := os.Stat(paths.media + ".tmp"); err == nil {
		t.Fatal("tmp output not removed")
	}

	// A core.Error keeps its stable code.
	tr.convert = func(sourceSpec, string, func(progressSample)) (string, error) {
		return "", invalid("input rejected")
	}
	j4 := &job{spec: sourceSpec{Path: "/m/x", Session: "coded", Profile: profile},
		state: contracts.TranscodeQueued, done: make(chan struct{})}
	tr.run(j4)
	if j4.errCode != "invalid-message" || j4.errMessage != "input rejected" {
		t.Fatalf("job=%+v", j4)
	}
}

func TestKillRunV3States(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })

	// Not queued: no run.
	j := &job{spec: sourceSpec{Session: "nq"}, state: contracts.TranscodeReady, done: make(chan struct{}), plan: &encodePlan{}}
	tr.runV3(j)
	if j.startedAt != 0 {
		t.Fatal("non-queued v3 ran")
	}

	// A stop landing mid-run is terminal even when the work succeeded.
	tr.v3run = func(j *job) error {
		j.stopped = true
		return nil
	}
	j2 := &job{spec: sourceSpec{Session: "st", Profile: "p"}, state: contracts.TranscodeQueued,
		done: make(chan struct{}), plan: &encodePlan{}, v3: true}
	tr.runV3(j2)
	if j2.state != contracts.TranscodeFailed || j2.playable {
		t.Fatalf("stopped job=%+v", j2)
	}

	// A cancelled job keeps its identity but never becomes a cache entry.
	tr.v3run = func(j *job) error {
		j.cancelled = true
		return nil
	}
	j3 := &job{spec: sourceSpec{Session: "cx", Profile: "p"}, state: contracts.TranscodeQueued,
		done: make(chan struct{}), plan: &encodePlan{}, v3: true}
	tr.runV3(j3)
	if j3.state != contracts.TranscodeFailed {
		t.Fatalf("cancelled job=%+v", j3)
	}
	if _, err := os.Stat(tr.metaPath("cx")); err == nil {
		t.Fatal("cancelled session wrote a cache entry")
	}

	// A pre-set error code survives; a bare error gets the default.
	tr.v3run = func(j *job) error {
		j.errCode = "quota"
		j.errMessage = "over quota"
		return errors.New("boom")
	}
	j4 := &job{spec: sourceSpec{Session: "ec", Profile: "p"}, state: contracts.TranscodeQueued,
		done: make(chan struct{}), plan: &encodePlan{}, v3: true}
	tr.runV3(j4)
	if j4.errCode != "quota" {
		t.Fatalf("errCode=%q", j4.errCode)
	}

	// Success: ready + playable.
	tr.v3run = func(j *job) error { return nil }
	tr.capsFn = func(contracts.TranscodeSettings) capabilities { return testCaps() }
	j5 := &job{spec: sourceSpec{Session: "win", Profile: "p", Path: "/m"},
		state: contracts.TranscodeQueued, done: make(chan struct{}), v3: true,
		plan: &encodePlan{delivery: contracts.TranscodeDeliveryProgressive}}
	// finalizeV3 -> recordV3 needs the artifact on disk.
	tr.v3run = func(j *job) error {
		return os.WriteFile(tr.defaultPaths(j.spec.Session).media, []byte("mp4"), 0o600)
	}
	tr.runV3(j5)
	if j5.state != contracts.TranscodeReady || !j5.playable {
		t.Fatalf("ready job=%+v", j5)
	}
}

// hlsStub writes a one-segment fMP4 playlist beside the output path.
const hlsStubScript = `for last; do :; done
dir=$(dirname "$last")
printf '#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-MAP:URI="init.mp4"\n#EXTINF:6.0,\nseg00000.m4s\n#EXT-X-ENDLIST\n' > "$last"
printf 'x' > "$dir/init.mp4"
printf 'x' > "$dir/seg00000.m4s"
`

func TestKillRunV3PlanProgressive(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	settings := contracts.DefaultTranscodeSettings()
	settings.FFmpegPath = stubFFmpeg(t, `for last; do :; done
printf 'mp4' > "$last"`)

	session := strings.Repeat("11", 32)
	spec := sourceSpec{Path: "/m/in.mkv", Session: session, Settings: settings}
	plan := &encodePlan{spec: spec, settings: settings, delivery: contracts.TranscodeDeliveryProgressive,
		video: &stream{Index: 0}, encoder: "libx264"}
	j := &job{spec: spec, settings: settings, plan: plan, delivery: plan.delivery, done: make(chan struct{})}
	if err := tr.runV3Plan(j); err != nil {
		t.Fatalf("runV3Plan: %v", err)
	}
	if _, err := os.Stat(tr.defaultPaths(session).media); err != nil {
		t.Fatal("media artifact missing")
	}
	if _, err := os.Stat(tr.defaultPaths(session).media + ".tmp"); err == nil {
		t.Fatal("tmp left behind")
	}

	// Encode failure propagates and removes the tmp.
	settings.FFmpegPath = stubFFmpeg(t, "exit 1")
	plan2 := &encodePlan{spec: spec, settings: settings, delivery: contracts.TranscodeDeliveryProgressive,
		video: &stream{Index: 0}, encoder: "libx264"}
	j2 := &job{spec: spec, settings: settings, plan: plan2, done: make(chan struct{})}
	if err := tr.runV3Plan(j2); err == nil {
		t.Fatal("want encode failure")
	}

	// Hardware failure retries in software and reports it.
	settings.FFmpegPath = stubFFmpeg(t, `for last; do :; done
case "$*" in
  *h264_vaapi*) exit 1 ;;
esac
printf 'mp4' > "$last"`)
	caps := testCaps()
	tr.capsFn = func(contracts.TranscodeSettings) capabilities { return caps }
	plan3 := &encodePlan{spec: spec, settings: settings, delivery: contracts.TranscodeDeliveryProgressive,
		video: &stream{Index: 0}, encoder: "h264_vaapi", hwBackend: contracts.HWVAAPI,
		videoCodec: contracts.VideoCodecH264}
	j3 := &job{spec: spec, settings: settings, plan: plan3, done: make(chan struct{})}
	if err := tr.runV3Plan(j3); err != nil {
		t.Fatalf("hw fallback: %v", err)
	}
	if j3.encoder != "libx264" || j3.hwBackend != "" || !strings.Contains(j3.fallback, "software") {
		t.Fatalf("fallback job=%+v", j3)
	}

	// A job already stopped never starts work.
	j4 := &job{spec: spec, settings: settings, plan: plan3, stopped: true, done: make(chan struct{})}
	if err := tr.runV3Plan(j4); err == nil {
		t.Fatal("stopped job must not run")
	}
}

func TestKillRunV3PlanHLS(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	settings := contracts.DefaultTranscodeSettings()
	settings.FFmpegPath = stubFFmpeg(t, hlsStubScript)
	session := strings.Repeat("22", 32)
	spec := sourceSpec{Path: "/m/in.mkv", Session: session, Settings: settings}
	plan := &encodePlan{spec: spec, settings: settings, delivery: contracts.TranscodeDeliveryHLS,
		video: &stream{Index: 0}, encoder: "libx264"}
	j := &job{spec: spec, settings: settings, plan: plan, delivery: plan.delivery, done: make(chan struct{})}
	if err := tr.runV3Plan(j); err != nil {
		t.Fatalf("hls plan: %v", err)
	}
	dir := tr.defaultPaths(session).hlsDir
	if _, err := os.Stat(filepath.Join(dir, hlsIndexPlaylist)); err != nil {
		t.Fatal("index playlist missing")
	}
	if !j.playable {
		t.Fatal("playable flag not set")
	}

	// Encode failure leaves no index.
	session2 := strings.Repeat("33", 32)
	settings2 := settings
	settings2.FFmpegPath = stubFFmpeg(t, "exit 1")
	spec2 := sourceSpec{Path: "/m/in.mkv", Session: session2, Settings: settings2}
	plan2 := &encodePlan{spec: spec2, settings: settings2, delivery: contracts.TranscodeDeliveryHLS,
		video: &stream{Index: 0}, encoder: "libx264"}
	j2 := &job{spec: spec2, settings: settings2, plan: plan2, done: make(chan struct{})}
	if err := tr.runV3Plan(j2); err == nil {
		t.Fatal("want hls encode failure")
	}
}

func TestKillFinalizeAndRecordV3(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	settings := contracts.DefaultTranscodeSettings()

	// Progressive with an extracted subtitle: the sidecar must exist.
	session := strings.Repeat("44", 32)
	spec := sourceSpec{Path: "/m", Session: session, Settings: settings, SubtitleStream: cloneInt(&[]int{3}[0])}
	sub := 3
	plan := &encodePlan{spec: spec, settings: settings, delivery: contracts.TranscodeDeliveryProgressive,
		subtitleMode: contracts.SubtitleModeExtract, subtitle: &stream{Index: sub}}
	j := &job{spec: spec, settings: settings, plan: plan, hasSubtitle: true, done: make(chan struct{})}
	paths := tr.defaultPaths(session)
	if err := os.WriteFile(paths.media, []byte("mp4"), 0o600); err != nil {
		t.Fatal(err)
	}
	// hasSubtitle without the file is an error.
	if err := tr.finalizeV3(j); err == nil {
		t.Fatal("want missing-WebVTT error")
	}
	if err := os.WriteFile(paths.subtitle, []byte("WEBVTT"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := tr.finalizeV3(j); err != nil {
		t.Fatalf("finalize: %v", err)
	}
	var e cacheEntry
	if err := readJSON(tr.metaPath(session), &e); err != nil {
		t.Fatal(err)
	}
	if e.Size != int64(len("mp4")+len("WEBVTT")) || !e.HasSubtitle {
		t.Fatalf("entry=%+v", e)
	}

	// Cancelled sessions never record.
	j2 := &job{spec: spec, settings: settings, plan: plan, cancelled: true, done: make(chan struct{})}
	if err := tr.finalizeV3(j2); err == nil {
		t.Fatal("cancelled session must not record")
	}

	// HLS without a playable playlist fails to record.
	session3 := strings.Repeat("55", 32)
	spec3 := sourceSpec{Path: "/m", Session: session3, Settings: settings}
	plan3 := &encodePlan{spec: spec3, settings: settings, delivery: contracts.TranscodeDeliveryHLS}
	j3 := &job{spec: spec3, settings: settings, plan: plan3, done: make(chan struct{})}
	if err := tr.recordV3(j3); err == nil {
		t.Fatal("want no-playable-HLS error")
	}
	writeRawPlaylist(t, tr.defaultPaths(session3).hlsDir, "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:6.0,\nseg0.m4s\n")
	hdir := tr.defaultPaths(session3).hlsDir
	for _, n := range []string{"init.mp4", "seg0.m4s"} {
		if err := os.WriteFile(filepath.Join(hdir, n), []byte("x"), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	if err := writeIndexPlaylist(hdir, 6); err != nil {
		t.Fatal(err)
	}
	if err := tr.recordV3(j3); err != nil {
		t.Fatalf("recordV3 hls: %v", err)
	}

	// Progressive without media fails.
	session4 := strings.Repeat("66", 32)
	j4 := &job{spec: sourceSpec{Path: "/m", Session: session4, Settings: settings}, settings: settings,
		plan: &encodePlan{delivery: contracts.TranscodeDeliveryProgressive}, done: make(chan struct{})}
	if err := tr.recordV3(j4); err == nil {
		t.Fatal("want no-mp4 error")
	}
}

func TestKillRemoveV3Artifacts(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	settings := contracts.DefaultTranscodeSettings()

	session := strings.Repeat("77", 32)
	paths := tr.defaultPaths(session)
	for _, p := range []string{paths.media, paths.media + ".tmp", paths.subtitle, paths.burn} {
		if err := os.WriteFile(p, []byte("x"), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	j := &job{spec: sourceSpec{Session: session, Settings: settings}, settings: settings,
		delivery: contracts.TranscodeDeliveryProgressive}
	tr.removeV3Artifacts(j)
	for _, p := range []string{paths.media, paths.media + ".tmp", paths.subtitle, paths.burn} {
		if _, err := os.Stat(p); err == nil {
			t.Fatalf("%s not removed", p)
		}
	}

	session2 := strings.Repeat("88", 32)
	hdir := tr.defaultPaths(session2).hlsDir
	if err := os.MkdirAll(hdir, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(hdir, "seg.m4s"), []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	j2 := &job{spec: sourceSpec{Session: session2, Settings: settings}, settings: settings,
		delivery: contracts.TranscodeDeliveryHLS}
	tr.removeV3Artifacts(j2)
	if _, err := os.Stat(hdir); err == nil {
		t.Fatal("hls dir not removed")
	}
}

func TestKillStatusFromJob(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	j := &job{spec: sourceSpec{Session: "s", Profile: "p"}, state: contracts.TranscodeRunning,
		progress: 0.4, done: make(chan struct{})}
	st := tr.statusFromJob(j)
	if st.Progress != 0.4 || st.Path != "" {
		t.Fatalf("running status=%+v", st)
	}
	j.state = contracts.TranscodeReady
	st = tr.statusFromJob(j)
	if st.Path == "" || st.Progress != 0 {
		t.Fatalf("ready status=%+v", st)
	}
	e := cacheEntry{Session: "s", Profile: "p", Method: "remux", HasSubtitle: true}
	st = statusFromEntry(e, "/d/s.mp4", true)
	if st.SubtitlePath == "" || !st.Cached || st.State != contracts.TranscodeReady {
		t.Fatalf("entry status=%+v", st)
	}
}

func TestKillRecord(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	spec := sourceSpec{Path: "/m", Session: "rec", Profile: "p"}
	if err := tr.record(spec, "remux"); err == nil {
		t.Fatal("want no-mp4 error")
	}
	if err := os.WriteFile(tr.mediaPath("rec"), []byte("mp4"), 0o600); err != nil {
		t.Fatal(err)
	}
	sub := 1
	spec.SubtitleStream = &sub
	if err := tr.record(spec, "remux"); err == nil {
		t.Fatal("want no-WebVTT error")
	}
	if err := os.WriteFile(tr.subtitlePath("rec"), []byte("WEBVTT"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := tr.record(spec, "remux"); err != nil {
		t.Fatal(err)
	}
	var e cacheEntry
	if err := readJSON(tr.metaPath("rec"), &e); err != nil {
		t.Fatal(err)
	}
	if e.Size != int64(len("mp4")+len("WEBVTT")) || !e.HasSubtitle {
		t.Fatalf("entry=%+v", e)
	}
}

func TestKillReadyEntry(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	spec := sourceSpec{Path: "/m/in.mkv", Session: strings.Repeat("aa", 32), Size: 5, ModTimeNS: 7, Profile: "p"}

	// No meta file.
	if _, ok := tr.readyEntry(spec); ok {
		t.Fatal("no meta must not be ready")
	}
	entry := cacheEntry{Session: spec.Session, SourcePath: spec.Path, SourceSize: spec.Size,
		SourceModTime: spec.ModTimeNS, Profile: spec.Profile, Method: "remux"}
	write := func(e cacheEntry) {
		if err := writeJSONAtomic(tr.metaPath(spec.Session), e); err != nil {
			t.Fatal(err)
		}
	}
	write(entry)
	// Meta exists but media missing.
	if _, ok := tr.readyEntry(spec); ok {
		t.Fatal("missing media must not be ready")
	}
	if err := os.WriteFile(tr.mediaPath(spec.Session), nil, 0o600); err != nil {
		t.Fatal(err)
	}
	if _, ok := tr.readyEntry(spec); ok {
		t.Fatal("empty media must not be ready")
	}
	if err := os.WriteFile(tr.mediaPath(spec.Session), []byte("mp4"), 0o600); err != nil {
		t.Fatal(err)
	}
	if e, ok := tr.readyEntry(spec); !ok || e.Size != 3 {
		t.Fatalf("ready=%v size=%d", ok, e.Size)
	}
	// Identity drift invalidates the entry.
	stale := entry
	stale.SourceSize = 9
	write(stale)
	if _, ok := tr.readyEntry(spec); ok {
		t.Fatal("stale source size must not be ready")
	}
	write(entry)
	// HasSubtitle requires the sidecar.
	subEntry := entry
	subEntry.HasSubtitle = true
	write(subEntry)
	if _, ok := tr.readyEntry(spec); ok {
		t.Fatal("missing subtitle must not be ready")
	}
	if err := os.WriteFile(tr.subtitlePath(spec.Session), []byte("WEBVTT"), 0o600); err != nil {
		t.Fatal(err)
	}
	if e, ok := tr.readyEntry(spec); !ok || e.Size != 3+6 {
		t.Fatalf("ready=%v size=%d", ok, e.Size)
	}

	// Sidecar-only entries resolve through the subtitle path.
	subSpec := spec
	subSpec.Session = strings.Repeat("bb", 32)
	side := cacheEntry{Session: subSpec.Session, SourcePath: spec.Path, SourceSize: 5,
		SourceModTime: 7, Profile: "p", Delivery: contracts.TranscodeDeliverySubtitle, HasSubtitle: true, Method: "extract"}
	if err := writeJSONAtomic(tr.metaPath(subSpec.Session), side); err != nil {
		t.Fatal(err)
	}
	if _, ok := tr.readyEntry(subSpec); ok {
		t.Fatal("missing sidecar must not be ready")
	}
	if err := os.WriteFile(tr.subtitlePath(subSpec.Session), []byte("WEBVTT"), 0o600); err != nil {
		t.Fatal(err)
	}
	if e, ok := tr.readyEntry(subSpec); !ok || e.Size != 6 {
		t.Fatalf("sidecar ready=%v size=%d", ok, e.Size)
	}

	// HLS entries need a playable playlist.
	hSpec := spec
	hSpec.Session = strings.Repeat("ee", 32)
	hEntry := cacheEntry{Session: hSpec.Session, SourcePath: spec.Path, SourceSize: 5,
		SourceModTime: 7, Profile: "p", Delivery: contracts.TranscodeDeliveryHLS, Method: "transcode"}
	if err := writeJSONAtomic(tr.metaPath(hSpec.Session), hEntry); err != nil {
		t.Fatal(err)
	}
	if _, ok := tr.readyEntry(hSpec); ok {
		t.Fatal("unplayable hls must not be ready")
	}
	hdir := tr.defaultPaths(hSpec.Session).hlsDir
	writeRawPlaylist(t, hdir, "#EXTM3U\n#EXTINF:6.0,\nseg0.m4s\n")
	if err := os.WriteFile(filepath.Join(hdir, "seg0.m4s"), []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := writeIndexPlaylist(hdir, 6); err != nil {
		t.Fatal(err)
	}
	e, ok := tr.readyEntry(hSpec)
	if !ok || e.Size <= 0 {
		t.Fatalf("hls ready=%v size=%d", ok, e.Size)
	}
	hlsSize := e.Size
	// HLS + subtitle needs the sidecar too.
	hEntry.HasSubtitle = true
	if err := writeJSONAtomic(tr.metaPath(hSpec.Session), hEntry); err != nil {
		t.Fatal(err)
	}
	if _, ok := tr.readyEntry(hSpec); ok {
		t.Fatal("missing hls sidecar must not be ready")
	}
	if err := os.WriteFile(tr.subtitlePath(hSpec.Session), []byte("WEBVTT"), 0o600); err != nil {
		t.Fatal(err)
	}
	if e, ok := tr.readyEntry(hSpec); !ok || e.Size != hlsSize+6 {
		t.Fatalf("hls+sub ready=%v size=%d, want %d", ok, e.Size, hlsSize+6)
	}
}

func TestKillActiveSessionsWith(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	out := tr.activeSessionsWith("extra")
	if !out["extra"] || len(out) != 1 {
		t.Fatalf("out=%v", out)
	}
	tr.jobs["run"] = &job{state: contracts.TranscodeRunning, done: make(chan struct{})}
	tr.jobs["done"] = &job{state: contracts.TranscodeReady, done: make(chan struct{})}
	out = tr.activeSessions()
	if !out["run"] || out["done"] {
		t.Fatalf("out=%v", out)
	}
}

func TestKillCleanup(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "cache")
	tr := newWithDeps(dir, Config{MaxCacheBytes: 10}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })

	// Startup removes stray .tmp files.
	if err := os.WriteFile(filepath.Join(dir, "orphan.tmp"), []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	// A corrupt meta file is removed.
	if err := os.WriteFile(filepath.Join(dir, "bad.json"), []byte("{"), 0o600); err != nil {
		t.Fatal(err)
	}
	// Orphaned artifacts (no meta) are removed; excluded ones survive.
	if err := os.WriteFile(filepath.Join(dir, "orphan.mp4"), []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "kept.mp4"), []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	orphanHLS := filepath.Join(dir, "orphan.hls")
	if err := os.MkdirAll(orphanHLS, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := tr.cleanup(true, map[string]bool{"kept": true}); err != nil {
		t.Fatal(err)
	}
	for _, gone := range []string{"orphan.tmp", "bad.json", "orphan.mp4", "orphan.hls"} {
		if _, err := os.Stat(filepath.Join(dir, gone)); err == nil {
			t.Fatalf("%s should be removed", gone)
		}
	}
	if _, err := os.Stat(filepath.Join(dir, "kept.mp4")); err != nil {
		t.Fatal("excluded artifact removed")
	}

	// Stale entries: media missing, source missing/changed, sidecar missing.
	mkEntry := func(session string, mutate func(*cacheEntry)) cacheEntry {
		e := cacheEntry{Session: session, SourcePath: "/m/in.mkv", SourceSize: 5,
			SourceModTime: 7, Profile: "p", Method: "remux"}
		if mutate != nil {
			mutate(&e)
		}
		if err := writeJSONAtomic(tr.metaPath(session), e); err != nil {
			t.Fatal(err)
		}
		return e
	}
	src := filepath.Join(t.TempDir(), "src.mkv")
	if err := os.WriteFile(src, []byte("12345"), 0o600); err != nil {
		t.Fatal(err)
	}
	fi, _ := os.Stat(src)
	mkEntry("s-no-media", func(e *cacheEntry) {
		e.SourcePath = src
		e.SourceSize = fi.Size()
		e.SourceModTime = fi.ModTime().UnixNano()
	})
	e2 := mkEntry("s-stale-src", func(e *cacheEntry) {
		e.SourcePath = src
		e.SourceSize = fi.Size() + 1 // source changed
		e.SourceModTime = fi.ModTime().UnixNano()
	})
	_ = e2
	sess3 := "s-no-sub"
	mkEntry(sess3, func(e *cacheEntry) {
		e.SourcePath = src
		e.SourceSize = fi.Size()
		e.SourceModTime = fi.ModTime().UnixNano()
		e.HasSubtitle = true
	})
	if err := os.WriteFile(tr.mediaPath(sess3), []byte("mp4"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := tr.cleanup(false, nil); err != nil {
		t.Fatal(err)
	}
	for _, s := range []string{"s-no-media", "s-stale-src", sess3} {
		if _, err := os.Stat(tr.metaPath(s)); err == nil {
			t.Fatalf("%s meta should be gone", s)
		}
	}
	if _, err := os.Stat(tr.mediaPath(sess3)); err == nil {
		t.Fatal("stale media should be gone")
	}

	// Fresh entry with missing sidecar excluded from removal survives.
	sess4 := "s-excluded"
	mkEntry(sess4, func(e *cacheEntry) {
		e.SourcePath = src
		e.SourceSize = fi.Size()
		e.SourceModTime = fi.ModTime().UnixNano()
	})
	// LRU eviction over budget: two ready entries of 4 bytes each, budget 5.
	now := time.Now().Unix()
	old := "s-old"
	recent := "s-new"
	for _, s := range []string{old, recent} {
		if err := os.WriteFile(tr.mediaPath(s), []byte("mp4x"), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	eOld := mkEntry(old, func(e *cacheEntry) {
		e.SourcePath, e.SourceSize, e.SourceModTime = src, fi.Size(), fi.ModTime().UnixNano()
		e.AccessedAt = now - 3600
		e.CreatedAt = now - 3600
	})
	_ = eOld
	mkEntry(recent, func(e *cacheEntry) {
		e.SourcePath, e.SourceSize, e.SourceModTime = src, fi.Size(), fi.ModTime().UnixNano()
		e.AccessedAt = now // inside the grace window
		e.CreatedAt = now
	})
	mkEntry(sess4, func(e *cacheEntry) { e.AccessedAt = now - 7200; e.CreatedAt = now - 7200 })
	if err := os.WriteFile(tr.mediaPath(sess4), []byte("mp4x"), 0o600); err != nil {
		t.Fatal(err)
	}
	tr.config.MaxCacheBytes = 5
	if err := tr.cleanup(false, nil); err != nil {
		t.Fatal(err)
	}
	// Oldest two evicted; the recently-accessed one survives the grace window.
	if _, err := os.Stat(tr.metaPath(old)); err == nil {
		t.Fatal("old entry should be evicted")
	}
	if _, err := os.Stat(tr.metaPath(sess4)); err == nil {
		t.Fatal("older entry should be evicted")
	}
	if _, err := os.Stat(tr.metaPath(recent)); err != nil {
		t.Fatal("recent entry evicted despite grace")
	}
}

func TestKillRemoveArtifactsRelocated(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	root := filepath.Join(t.TempDir(), "temp", "lain-transcode")
	session := strings.Repeat("f0", 32)
	e := cacheEntry{Session: session, ArtifactRoot: root}
	dir := artifactDirFor(root, session)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "media.mp4"), []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := writeJSONAtomic(tr.metaPath(session), e); err != nil {
		t.Fatal(err)
	}
	tr.removeArtifacts(e, tr.pathsForEntry(e))
	if _, err := os.Stat(dir); err == nil {
		t.Fatal("session dir not removed")
	}
	if _, err := os.Stat(tr.metaPath(session)); err == nil {
		t.Fatal("meta not removed")
	}
}

func TestKillSweepRelocatedOrphans(t *testing.T) {
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	// Empty root is a no-op.
	tr.sweepRelocatedOrphans("", nil)

	root := filepath.Join(t.TempDir(), "temp", "lain-transcode")
	orphan := filepath.Join(root, "orphan-session")
	claimed := filepath.Join(root, "claimed-session")
	excluded := filepath.Join(root, "excluded-session")
	for _, d := range []string{orphan, claimed, excluded} {
		if err := os.MkdirAll(d, 0o700); err != nil {
			t.Fatal(err)
		}
	}
	// A non-directory sibling is never touched.
	if err := os.WriteFile(filepath.Join(root, "notes.txt"), []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	// claimed-session has a sidecar; excluded-session is in the exclude set.
	if err := writeJSONAtomic(tr.metaPath("claimed-session"), cacheEntry{Session: "claimed-session"}); err != nil {
		t.Fatal(err)
	}
	tr.sweepRelocatedOrphans(root, map[string]bool{"excluded-session": true})
	if _, err := os.Stat(orphan); err == nil {
		t.Fatal("orphan not swept")
	}
	for _, d := range []string{claimed, excluded} {
		if _, err := os.Stat(d); err != nil {
			t.Fatalf("%s swept: %v", d, err)
		}
	}
	if _, err := os.Stat(filepath.Join(root, "notes.txt")); err != nil {
		t.Fatal("sibling file removed")
	}
	// The owner marker now claims this data dir.
	raw, err := os.ReadFile(filepath.Join(root, ".owner"))
	if err != nil || strings.TrimSpace(string(raw)) != tr.dir {
		t.Fatalf("owner=%q err=%v", raw, err)
	}

	// A foreign live owner blocks the sweep entirely.
	root2 := filepath.Join(t.TempDir(), "temp2", "lain-transcode")
	liveOwner := t.TempDir() // exists
	if err := os.MkdirAll(root2, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root2, ".owner"), []byte(liveOwner), 0o600); err != nil {
		t.Fatal(err)
	}
	victim := filepath.Join(root2, "victim")
	if err := os.MkdirAll(victim, 0o700); err != nil {
		t.Fatal(err)
	}
	tr.sweepRelocatedOrphans(root2, nil)
	if _, err := os.Stat(victim); err != nil {
		t.Fatal("foreign owner's dir was swept")
	}
}
