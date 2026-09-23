package contracts

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-22

import (
	"reflect"
	"testing"
)

// TestAdvancedNormalizeDefaults pins the upgrade path for the advanced
// knobs: a partial settings document must gain the shipped defaults for
// every new field, while "let ffmpeg decide" zeros and opt-in switches
// stay at their zero value.
func TestAdvancedNormalizeDefaults(t *testing.T) {
	got := (TranscodeSettings{}).Normalize()

	checks := []struct {
		name string
		got  any
		want any
	}{
		{"ThreadCount", got.ThreadCount, 0},
		{"MuxingQueueSize", got.MuxingQueueSize, 2048},
		{"DownmixAudioBoost", got.DownmixAudioBoost, 2.0},
		{"DownmixStereoAlgorithm", got.DownmixStereoAlgorithm, DownmixNone},
		{"H264CRF", got.H264CRF, 23},
		{"H265CRF", got.H265CRF, 28},
		{"AV1CRF", got.AV1CRF, 32},
		{"H264Preset", got.H264Preset, ""},
		{"H265Preset", got.H265Preset, ""},
		{"AV1Preset", got.AV1Preset, ""},
		{"TranscodeTempPath", got.TranscodeTempPath, ""},
		{"RemoteBitrateLimitKbps", got.RemoteBitrateLimitKbps, 0},
		{"AudioVBR", got.AudioVBR, false},
		{"DeinterlaceDoubleRate", got.DeinterlaceDoubleRate, false},
		{"HardwareDecode10BitHEVC", got.HardwareDecode10BitHEVC, false},
		{"HardwareDecode10BitVP9", got.HardwareDecode10BitVP9, false},
	}
	for _, c := range checks {
		if !reflect.DeepEqual(c.got, c.want) {
			t.Errorf("normalized zero settings: %s = %v, want %v", c.name, c.got, c.want)
		}
	}
	if err := got.Validate(); err != nil {
		t.Fatalf("normalized zero settings must validate: %v", err)
	}
}

// TestAdvancedNormalizeKeepsExplicitValues proves Normalize only fills
// zero values: an operator's explicit advanced choices must survive the
// round trip that every request performs.
func TestAdvancedNormalizeKeepsExplicitValues(t *testing.T) {
	in := TranscodeSettings{
		ThreadCount:             4,
		MuxingQueueSize:         4096,
		TranscodeTempPath:       "/tmp/x",
		RemoteBitrateLimitKbps:  5000,
		DownmixAudioBoost:       0.5,
		DownmixStereoAlgorithm:  DownmixNightmode,
		H264CRF:                 30,
		H265CRF:                 26,
		AV1CRF:                  35,
		H264Preset:              "slow",
		H265Preset:              "medium",
		AV1Preset:               "slower",
		AudioVBR:                true,
		DeinterlaceDoubleRate:   true,
		HardwareDecode10BitHEVC: true,
		HardwareDecode10BitVP9:  true,
	}
	got := in.Normalize()

	checks := []struct {
		name string
		got  any
		want any
	}{
		{"ThreadCount", got.ThreadCount, 4},
		{"MuxingQueueSize", got.MuxingQueueSize, 4096},
		{"TranscodeTempPath", got.TranscodeTempPath, "/tmp/x"},
		{"RemoteBitrateLimitKbps", got.RemoteBitrateLimitKbps, 5000},
		{"DownmixAudioBoost", got.DownmixAudioBoost, 0.5},
		{"DownmixStereoAlgorithm", got.DownmixStereoAlgorithm, DownmixNightmode},
		{"H264CRF", got.H264CRF, 30},
		{"H265CRF", got.H265CRF, 26},
		{"AV1CRF", got.AV1CRF, 35},
		{"H264Preset", got.H264Preset, "slow"},
		{"H265Preset", got.H265Preset, "medium"},
		{"AV1Preset", got.AV1Preset, "slower"},
		{"AudioVBR", got.AudioVBR, true},
		{"DeinterlaceDoubleRate", got.DeinterlaceDoubleRate, true},
		{"HardwareDecode10BitHEVC", got.HardwareDecode10BitHEVC, true},
		{"HardwareDecode10BitVP9", got.HardwareDecode10BitVP9, true},
	}
	for _, c := range checks {
		if !reflect.DeepEqual(c.got, c.want) {
			t.Errorf("explicit %s = %v, want %v", c.name, c.got, c.want)
		}
	}
}

// TestAdvancedValidateRejectsOutOfRange pins every advanced bound: an
// admin PUT outside the documented range must be refused, not clamped.
func TestAdvancedValidateRejectsOutOfRange(t *testing.T) {
	cases := []struct {
		name   string
		mutate func(*TranscodeSettings)
	}{
		{"thread count 65", func(s *TranscodeSettings) { s.ThreadCount = 65 }},
		{"thread count -1", func(s *TranscodeSettings) { s.ThreadCount = -1 }},
		{"muxing queue 127", func(s *TranscodeSettings) { s.MuxingQueueSize = 127 }},
		{"muxing queue 65537", func(s *TranscodeSettings) { s.MuxingQueueSize = 65537 }},
		{"downmix boost 0.4", func(s *TranscodeSettings) { s.DownmixAudioBoost = 0.4 }},
		{"downmix boost 9", func(s *TranscodeSettings) { s.DownmixAudioBoost = 9 }},
		{"downmix algorithm dpl2", func(s *TranscodeSettings) { s.DownmixStereoAlgorithm = "dpl2" }},
		{"h264 crf 52", func(s *TranscodeSettings) { s.H264CRF = 52 }},
		{"h265 crf 52", func(s *TranscodeSettings) { s.H265CRF = 52 }},
		{"av1 crf 52", func(s *TranscodeSettings) { s.AV1CRF = 52 }},
		{"h264 preset insane", func(s *TranscodeSettings) { s.H264Preset = "insane" }},
		{"h265 preset insane", func(s *TranscodeSettings) { s.H265Preset = "insane" }},
		{"av1 preset insane", func(s *TranscodeSettings) { s.AV1Preset = "insane" }},
		{"remote bitrate -1", func(s *TranscodeSettings) { s.RemoteBitrateLimitKbps = -1 }},
		{"remote bitrate 1000001", func(s *TranscodeSettings) { s.RemoteBitrateLimitKbps = 1000001 }},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			s := DefaultTranscodeSettings()
			tc.mutate(&s)
			if err := s.Validate(); err == nil {
				t.Fatalf("Validate accepted %+v", s)
			}
		})
	}
}

// TestAdvancedValidateAcceptsBoundaries pins the inclusive ends of each
// advanced range, so the documented limits cannot drift into off-by-one
// refusals.
func TestAdvancedValidateAcceptsBoundaries(t *testing.T) {
	s := DefaultTranscodeSettings()
	s.ThreadCount = 64
	s.MuxingQueueSize = 128
	s.DownmixAudioBoost = 8
	s.DownmixStereoAlgorithm = DownmixNightmode
	s.RemoteBitrateLimitKbps = 1_000_000
	s.H264CRF = 51
	s.H264Preset = "ultrafast"
	s.H265Preset = "medium"
	s.AV1Preset = "veryslow"
	if err := s.Validate(); err != nil {
		t.Fatalf("Validate refused boundary values: %v", err)
	}
}

// TestEncoderTuningPerCodec pins the per-codec resolution: the codec's
// own CRF/preset wins when set, otherwise the general values keep the
// pre-existing behavior, and an unknown codec is treated as H.264.
func TestEncoderTuningPerCodec(t *testing.T) {
	const (
		generalCRF    = 23
		generalPreset = "veryfast"
	)
	cases := []struct {
		name       string
		codec      string
		mutate     func(*TranscodeSettings)
		wantCRF    int
		wantPreset string
	}{
		{"h264 explicit", VideoCodecH264, func(s *TranscodeSettings) { s.H264CRF = 30; s.H264Preset = "slow" }, 30, "slow"},
		{"h264 falls back when zero and empty", VideoCodecH264, func(s *TranscodeSettings) { s.H264CRF = 0; s.H264Preset = "" }, generalCRF, generalPreset},
		{"h264 mixed fallback keeps the explicit CRF", VideoCodecH264, func(s *TranscodeSettings) { s.H264CRF = 30; s.H264Preset = "" }, 30, generalPreset},
		{"hevc explicit", VideoCodecHEVC, func(s *TranscodeSettings) { s.H265CRF = 26; s.H265Preset = "medium" }, 26, "medium"},
		{"hevc falls back when zero and empty", VideoCodecHEVC, func(s *TranscodeSettings) { s.H265CRF = 0; s.H265Preset = "" }, generalCRF, generalPreset},
		{"hevc ignores the h264 knobs", VideoCodecHEVC, func(s *TranscodeSettings) { s.H265CRF = 0; s.H265Preset = ""; s.H264CRF = 30; s.H264Preset = "slow" }, generalCRF, generalPreset},
		{"av1 explicit", VideoCodecAV1, func(s *TranscodeSettings) { s.AV1CRF = 35; s.AV1Preset = "slower" }, 35, "slower"},
		{"av1 falls back when zero and empty", VideoCodecAV1, func(s *TranscodeSettings) { s.AV1CRF = 0; s.AV1Preset = "" }, generalCRF, generalPreset},
		{"unknown codec behaves like h264", "mpeg4", func(s *TranscodeSettings) { s.H264CRF = 30; s.H264Preset = "slow" }, 30, "slow"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			s := DefaultTranscodeSettings()
			s.CRF, s.EncoderPreset = generalCRF, generalPreset
			tc.mutate(&s)
			crf, preset := s.EncoderTuning(tc.codec)
			if crf != tc.wantCRF || preset != tc.wantPreset {
				t.Fatalf("EncoderTuning(%q) = (%d, %q), want (%d, %q)", tc.codec, crf, preset, tc.wantCRF, tc.wantPreset)
			}
		})
	}
}

// TestAudioVBRQualityBoundaries pins the documented bitrate-to-quality
// mapping: an operator must be able to predict what audio_vbr produces.
func TestAudioVBRQualityBoundaries(t *testing.T) {
	cases := []struct {
		kbps int
		want float64
	}{
		{32, 0.1},
		{63, 0.1},
		{64, 0.5},
		{95, 0.5},
		{96, 1},
		{127, 1},
		{128, 1.5},
		{191, 1.5},
		{192, 2},
		{640, 2},
	}
	for _, c := range cases {
		if got := AudioVBRQuality(c.kbps); got != c.want {
			t.Errorf("AudioVBRQuality(%d) = %v, want %v", c.kbps, got, c.want)
		}
	}
}

// TestAdvancedProfileKeySensitivity pins D-030/D-045: every advanced
// setting that changes the produced bytes must move the derivative key,
// and the purely operational knobs must not (they change speed and
// placement, not the output).
func TestAdvancedProfileKeySensitivity(t *testing.T) {
	settings := DefaultTranscodeSettings()
	req := TranscodeV3Request{
		Action:   TranscodeStartAction,
		Delivery: TranscodeDeliveryHLS,
		Settings: settings,
	}
	base := TranscodeProfileKey(settings, req)

	moves := []struct {
		name   string
		mutate func(*TranscodeSettings)
	}{
		{"audio vbr", func(s *TranscodeSettings) { s.AudioVBR = true }},
		{"downmix audio boost", func(s *TranscodeSettings) { s.DownmixAudioBoost = 3 }},
		{"downmix stereo algorithm", func(s *TranscodeSettings) { s.DownmixStereoAlgorithm = DownmixNightmode }},
		{"h264 preset", func(s *TranscodeSettings) { s.H264Preset = "slow" }},
		{"h264 crf", func(s *TranscodeSettings) { s.H264CRF = 25 }},
		{"h265 crf", func(s *TranscodeSettings) { s.H265CRF = 30 }},
		{"av1 crf", func(s *TranscodeSettings) { s.AV1CRF = 35 }},
		{"fallback font path", func(s *TranscodeSettings) { s.FallbackFontPath = "/tmp/fonts" }},
		{"fallback font name", func(s *TranscodeSettings) { s.FallbackFontName = "DejaVu Sans" }},
		{"deinterlace double rate", func(s *TranscodeSettings) { s.DeinterlaceDoubleRate = true }},
		{"10-bit hevc decode", func(s *TranscodeSettings) { s.HardwareDecode10BitHEVC = true }},
		{"10-bit vp9 decode", func(s *TranscodeSettings) { s.HardwareDecode10BitVP9 = true }},
	}
	for _, m := range moves {
		t.Run(m.name, func(t *testing.T) {
			s := settings
			m.mutate(&s)
			if got := TranscodeProfileKey(s, req); got == base {
				t.Fatalf("key %q did not change for %s", got, m.name)
			}
		})
	}

	stays := []struct {
		name   string
		mutate func(*TranscodeSettings)
	}{
		{"thread count", func(s *TranscodeSettings) { s.ThreadCount = 4 }},
		{"muxing queue size", func(s *TranscodeSettings) { s.MuxingQueueSize = 4096 }},
		{"transcode temp path", func(s *TranscodeSettings) { s.TranscodeTempPath = "/tmp/x" }},
		{"cache bytes", func(s *TranscodeSettings) { s.CacheBytes = 10 << 30 }},
		{"queue size", func(s *TranscodeSettings) { s.QueueSize = 4 }},
		{"max concurrent", func(s *TranscodeSettings) { s.MaxConcurrent = 1 }},
		// The render node picks which GPU runs the work; it does not change
		// the produced bytes, so it stays out of the session identity.
		{"hardware device", func(s *TranscodeSettings) { s.HardwareDevice = "/dev/dri/renderD129" }},
	}
	for _, m := range stays {
		t.Run(m.name, func(t *testing.T) {
			s := settings
			m.mutate(&s)
			if got := TranscodeProfileKey(s, req); got != base {
				t.Fatalf("key changed for operational knob %s: %q vs %q", m.name, got, base)
			}
		})
	}
}

// TestAdvancedHardwareDevice pins the VA-API render node: shipped default,
// Normalize backfill, and validation that cannot break the ffmpeg argv.
func TestAdvancedHardwareDevice(t *testing.T) {
	if d := DefaultTranscodeSettings(); d.HardwareDevice != DefaultHardwareDevice {
		t.Fatalf("default hardware_device=%q, want %q", d.HardwareDevice, DefaultHardwareDevice)
	}
	if got := (TranscodeSettings{}).Normalize().HardwareDevice; got != DefaultHardwareDevice {
		t.Fatalf("Normalize hardware_device=%q, want %q", got, DefaultHardwareDevice)
	}
	ok := DefaultTranscodeSettings()
	ok.HardwareDevice = "/dev/dri/renderD129"
	if err := ok.Validate(); err != nil {
		t.Fatalf("a real device path was rejected: %v", err)
	}
	bad := DefaultTranscodeSettings()
	bad.HardwareDevice = "/dev/dri/renderD128\nrm -rf"
	if err := bad.Validate(); err == nil {
		t.Fatal("a newline in hardware_device must be rejected")
	}
}

// TestAdvancedRKMPPBackendAccepted pins Jellyfin's Rockchip method: the
// backend list must accept it so a Rockchip host can select its VPU.
func TestAdvancedRKMPPBackendAccepted(t *testing.T) {
	s := DefaultTranscodeSettings()
	s.HardwareAcceleration = HWRKMPP
	if err := s.Validate(); err != nil {
		t.Fatalf("rkmpp backend rejected: %v", err)
	}
}
