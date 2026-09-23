package contracts

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-22

import (
	"reflect"
	"strings"
	"testing"
)

// TestDefaultSettingsValidate pins the shipped policy as valid: a bad
// default would make every fresh server reject its own settings.
func TestDefaultSettingsValidate(t *testing.T) {
	if err := DefaultTranscodeSettings().Validate(); err != nil {
		t.Fatalf("default settings must validate: %v", err)
	}
}

// TestNormalizeFillsZeroValues covers the upgrade path: a partial
// document (older server, hand-edited JSON) must become a full policy.
// Boolean switches and the codec list are deliberately not normalized:
// their zero value is a meaningful "off"/"none" choice.
func TestNormalizeFillsZeroValues(t *testing.T) {
	d := DefaultTranscodeSettings()
	got := (TranscodeSettings{}).Normalize()

	checks := []struct {
		name string
		got  any
		want any
	}{
		{"DefaultDelivery", got.DefaultDelivery, d.DefaultDelivery},
		{"HLSSegmentSeconds", got.HLSSegmentSeconds, d.HLSSegmentSeconds},
		{"ThrottleAheadSec", got.ThrottleAheadSec, d.ThrottleAheadSec},
		{"SegmentKeepSec", got.SegmentKeepSec, d.SegmentKeepSec},
		{"IdleTimeoutSec", got.IdleTimeoutSec, d.IdleTimeoutSec},
		{"CacheBytes", got.CacheBytes, d.CacheBytes},
		{"QueueSize", got.QueueSize, d.QueueSize},
		{"MaxConcurrent", got.MaxConcurrent, d.MaxConcurrent},
		{"EncoderPreset", got.EncoderPreset, d.EncoderPreset},
		{"CRF", got.CRF, d.CRF},
		{"AudioBitrateKbps", got.AudioBitrateKbps, d.AudioBitrateKbps},
		{"SubtitleMode", got.SubtitleMode, d.SubtitleMode},
		{"ToneMappingAlgorithm", got.ToneMappingAlgorithm, d.ToneMappingAlgorithm},
		{"ToneMappingMode", got.ToneMappingMode, d.ToneMappingMode},
		{"ToneMappingPeakNits", got.ToneMappingPeakNits, d.ToneMappingPeakNits},
		{"Deinterlace", got.Deinterlace, d.Deinterlace},
		{"HardwareAcceleration", got.HardwareAcceleration, d.HardwareAcceleration},
		{"Qualities", got.Qualities, d.Qualities},
	}
	for _, c := range checks {
		if !reflect.DeepEqual(c.got, c.want) {
			t.Errorf("%s = %v, want %v", c.name, c.got, c.want)
		}
	}
	if err := got.Validate(); err != nil {
		t.Fatalf("normalized zero settings must validate: %v", err)
	}
}

// TestValidateRejectsOutOfRange pins every bounded knob: an admin PUT
// with a value outside the documented range must be refused, not
// clamped silently.
func TestValidateRejectsOutOfRange(t *testing.T) {
	cases := []struct {
		name   string
		mutate func(*TranscodeSettings)
	}{
		{"delivery unknown", func(s *TranscodeSettings) { s.DefaultDelivery = "dash" }},
		{"segment seconds low", func(s *TranscodeSettings) { s.HLSSegmentSeconds = 0 }},
		{"segment seconds high", func(s *TranscodeSettings) { s.HLSSegmentSeconds = 31 }},
		{"throttle ahead low", func(s *TranscodeSettings) { s.ThrottleAheadSec = 4 }},
		{"throttle ahead high", func(s *TranscodeSettings) { s.ThrottleAheadSec = 601 }},
		{"segment keep negative", func(s *TranscodeSettings) { s.SegmentKeepSec = -1 }},
		{"segment keep high", func(s *TranscodeSettings) { s.SegmentKeepSec = 3601 }},
		{"idle timeout low", func(s *TranscodeSettings) { s.IdleTimeoutSec = 14 }},
		{"idle timeout high", func(s *TranscodeSettings) { s.IdleTimeoutSec = 3601 }},
		{"cache bytes tiny", func(s *TranscodeSettings) { s.CacheBytes = 1<<30 - 1 }},
		{"queue size zero", func(s *TranscodeSettings) { s.QueueSize = 0 }},
		{"queue size high", func(s *TranscodeSettings) { s.QueueSize = 65 }},
		{"max concurrent zero", func(s *TranscodeSettings) { s.MaxConcurrent = 0 }},
		{"max concurrent high", func(s *TranscodeSettings) { s.MaxConcurrent = 17 }},
		{"preset unknown", func(s *TranscodeSettings) { s.EncoderPreset = "turbo" }},
		{"crf negative", func(s *TranscodeSettings) { s.CRF = -1 }},
		{"crf high", func(s *TranscodeSettings) { s.CRF = 52 }},
		{"quality without name", func(s *TranscodeSettings) { s.Qualities[0].Name = "  " }},
		{"quality negative bounds", func(s *TranscodeSettings) { s.Qualities[0].MaxWidth = -1 }},
		{"audio bitrate low", func(s *TranscodeSettings) { s.AudioBitrateKbps = 31 }},
		{"audio bitrate high", func(s *TranscodeSettings) { s.AudioBitrateKbps = 641 }},
		{"subtitle mode unknown", func(s *TranscodeSettings) { s.SubtitleMode = "sometimes" }},
		{"tone algorithm unknown", func(s *TranscodeSettings) { s.ToneMappingAlgorithm = "aces" }},
		{"tone mode unknown", func(s *TranscodeSettings) { s.ToneMappingMode = "sometimes" }},
		{"tone peak low", func(s *TranscodeSettings) { s.ToneMappingPeakNits = 49 }},
		{"tone peak high", func(s *TranscodeSettings) { s.ToneMappingPeakNits = 10001 }},
		{"deinterlace unknown", func(s *TranscodeSettings) { s.Deinterlace = "half" }},
		{"hardware backend unknown", func(s *TranscodeSettings) { s.HardwareAcceleration = "cuda" }},
		{"decode codec unknown", func(s *TranscodeSettings) { s.HardwareDecodeCodecs = append(s.HardwareDecodeCodecs, "mpeg1") }},
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

// TestValidateAcceptsBoundaryValues pins the inclusive edges of every
// bounded knob: the documented min and max must validate, so an
// off-by-one in the comparison is caught.
func TestValidateAcceptsBoundaryValues(t *testing.T) {
	edges := []struct {
		name   string
		mutate func(*TranscodeSettings)
	}{
		{"segment seconds 1", func(s *TranscodeSettings) { s.HLSSegmentSeconds = 1 }},
		{"segment seconds 30", func(s *TranscodeSettings) { s.HLSSegmentSeconds = 30 }},
		{"throttle ahead 5", func(s *TranscodeSettings) { s.ThrottleAheadSec = 5 }},
		{"throttle ahead 600", func(s *TranscodeSettings) { s.ThrottleAheadSec = 600 }},
		{"segment keep 0", func(s *TranscodeSettings) { s.SegmentKeepSec = 0 }},
		{"segment keep 3600", func(s *TranscodeSettings) { s.SegmentKeepSec = 3600 }},
		{"idle timeout 15", func(s *TranscodeSettings) { s.IdleTimeoutSec = 15 }},
		{"idle timeout 3600", func(s *TranscodeSettings) { s.IdleTimeoutSec = 3600 }},
		{"cache bytes 1GiB", func(s *TranscodeSettings) { s.CacheBytes = 1 << 30 }},
		{"queue size 1", func(s *TranscodeSettings) { s.QueueSize = 1 }},
		{"queue size 64", func(s *TranscodeSettings) { s.QueueSize = 64 }},
		{"max concurrent 1", func(s *TranscodeSettings) { s.MaxConcurrent = 1 }},
		{"max concurrent 16", func(s *TranscodeSettings) { s.MaxConcurrent = 16 }},
		{"crf 0", func(s *TranscodeSettings) { s.CRF = 0 }},
		{"crf 51", func(s *TranscodeSettings) { s.CRF = 51 }},
		{"quality zero bounds", func(s *TranscodeSettings) {
			s.Qualities[0].MaxWidth, s.Qualities[0].MaxHeight, s.Qualities[0].BitrateKbps = 0, 0, 0
		}},
		{"audio bitrate 32", func(s *TranscodeSettings) { s.AudioBitrateKbps = 32 }},
		{"audio bitrate 640", func(s *TranscodeSettings) { s.AudioBitrateKbps = 640 }},
		{"downmix boost 0.5", func(s *TranscodeSettings) { s.DownmixAudioBoost = 0.5 }},
		{"thread count 0", func(s *TranscodeSettings) { s.ThreadCount = 0 }},
		{"muxing queue 128", func(s *TranscodeSettings) { s.MuxingQueueSize = 128 }},
		{"muxing queue 65536", func(s *TranscodeSettings) { s.MuxingQueueSize = 65536 }},
		{"remote bitrate 0", func(s *TranscodeSettings) { s.RemoteBitrateLimitKbps = 0 }},
		{"device exactly 512", func(s *TranscodeSettings) { s.HardwareDevice = strings.Repeat("d", 512) }},
		{"device with space ok", func(s *TranscodeSettings) { s.HardwareDevice = " /dev/dri/renderD128 " }},
	}
	for _, tc := range edges {
		t.Run(tc.name, func(t *testing.T) {
			s := DefaultTranscodeSettings()
			tc.mutate(&s)
			if err := s.Validate(); err != nil {
				t.Fatalf("boundary value rejected: %v", err)
			}
		})
	}
	// Device edge: one byte over the cap or a newline inside must fail.
	s := DefaultTranscodeSettings()
	s.HardwareDevice = strings.Repeat("d", 513)
	if err := s.Validate(); err == nil {
		t.Fatal("device > 512 bytes must refuse")
	}
	s = DefaultTranscodeSettings()
	s.HardwareDevice = "/dev/x\ny"
	if err := s.Validate(); err == nil {
		t.Fatal("device with newline must refuse")
	}
}

// Per-codec CRF fields share the 0..51 bound; presets share the
// EncoderPresets vocabulary; peak nits are 50..10000 inclusive.
func TestPerCodecBounds(t *testing.T) {
	for _, crf := range []int{-1, 52} {
		for _, set := range []func(*TranscodeSettings, int){
			func(s *TranscodeSettings, v int) { s.H264CRF = v },
			func(s *TranscodeSettings, v int) { s.H265CRF = v },
			func(s *TranscodeSettings, v int) { s.AV1CRF = v },
		} {
			s := DefaultTranscodeSettings()
			set(&s, crf)
			if err := s.Validate(); err == nil {
				t.Fatalf("crf %d must refuse", crf)
			}
		}
	}
	for _, crf := range []int{0, 51} {
		s := DefaultTranscodeSettings()
		s.H264CRF, s.H265CRF, s.AV1CRF = crf, crf, crf
		if err := s.Validate(); err != nil {
			t.Fatalf("crf %d must accept: %v", crf, err)
		}
	}
	for _, preset := range []string{"turbo", "PLACEHOLDER"} {
		s := DefaultTranscodeSettings()
		s.H264Preset = preset
		if err := s.Validate(); err == nil {
			t.Fatalf("preset %q must refuse", preset)
		}
	}
	for _, preset := range EncoderPresets {
		s := DefaultTranscodeSettings()
		s.H264Preset, s.H265Preset, s.AV1Preset = preset, preset, preset
		if err := s.Validate(); err != nil {
			t.Fatalf("preset %q must accept: %v", preset, err)
		}
	}
	s := DefaultTranscodeSettings()
	s.H264Preset = ""
	if err := s.Validate(); err != nil {
		t.Fatalf("empty preset must accept (unset): %v", err)
	}
	for _, nits := range []int{50, 10000} {
		s := DefaultTranscodeSettings()
		s.ToneMappingPeakNits = nits
		if err := s.Validate(); err != nil {
			t.Fatalf("peak %d must accept: %v", nits, err)
		}
	}
}

// An empty request field inherits the settings default — and the
// resolved value is what lands in the profile key.
func TestProfileKeyDefaultsResolve(t *testing.T) {
	s := DefaultTranscodeSettings()
	req := TranscodeV3Request{Settings: s}
	def := TranscodeProfileKey(s, req)

	explicit := req
	explicit.Delivery = s.DefaultDelivery
	explicit.VideoCodec = VideoCodecH264
	explicit.AudioCodec = AudioCodecAAC
	explicit.SubtitleMode = s.SubtitleMode
	if TranscodeProfileKey(s, explicit) != def {
		t.Fatal("explicit-equal-to-default must produce the same key")
	}
	// A different default produces a different key even with the field
	// still empty on the request.
	s2 := s
	s2.DefaultDelivery = TranscodeDeliveryProgressive
	if s.DefaultDelivery == s2.DefaultDelivery {
		t.Fatal("test needs a changed default")
	}
	if TranscodeProfileKey(s2, req) == def {
		t.Fatal("changed default must move the key for empty request fields")
	}
	s2 = s
	s2.SubtitleMode = SubtitleModeOff
	if s.SubtitleMode == s2.SubtitleMode {
		t.Fatal("test needs a changed subtitle default")
	}
	if TranscodeProfileKey(s2, req) == def {
		t.Fatal("changed subtitle default must move the key")
	}
}

// AllowsSubtitleExtraction / AllowsFallbackFonts default to on and only
// an explicit false disables them.
func TestAdditivePermissionDefaults(t *testing.T) {
	s := DefaultTranscodeSettings()
	if !s.AllowsSubtitleExtraction() || !s.AllowsFallbackFonts() {
		t.Fatal("nil permission flags must default to allowed")
	}
	f := false
	s.AllowSubtitleExtraction = &f
	if s.AllowSubtitleExtraction == nil || *s.AllowSubtitleExtraction != false {
		t.Fatal("fixture")
	}
	if s.AllowsSubtitleExtraction() {
		t.Fatal("explicit false must deny extraction")
	}
	s.AllowSubtitleExtraction = nil
	s.FallbackFontEnabled = &f
	if s.AllowsFallbackFonts() {
		t.Fatal("explicit false must deny fallback fonts")
	}
	// AllowStreamCopy has the same shape on the request side.
	if !AllowStreamCopy(nil) {
		t.Fatal("nil stream-copy flag must default to allowed")
	}
	if AllowStreamCopy(&f) {
		t.Fatal("explicit false must deny stream copy")
	}
}

// TestQualityLookup covers the ladder lookup the planner uses for the
// player's quality menu.
func TestQualityLookup(t *testing.T) {
	s := DefaultTranscodeSettings()
	q, ok := s.Quality("720p")
	if !ok {
		t.Fatal("720p must resolve")
	}
	if q.MaxWidth != 1280 || q.MaxHeight != 720 || q.BitrateKbps != 4000 {
		t.Fatalf("720p = %+v", q)
	}
	if _, ok := s.Quality("144p"); ok {
		t.Fatal("unknown quality must report absent")
	}
	if _, ok := s.Quality(""); ok {
		t.Fatal("empty quality must report absent")
	}
}

// TestProfileKeyIsStableAndSensitive pins D-030/D-045: the profile key
// is the derivative identity, so any output-affecting change must move
// it or stale artifacts would be served.
func TestProfileKeyIsStableAndSensitive(t *testing.T) {
	settings := DefaultTranscodeSettings()
	req := TranscodeV3Request{
		Action:   TranscodeStartAction,
		Delivery: TranscodeDeliveryHLS,
		Quality:  "1080p",
		Settings: settings,
	}
	base := TranscodeProfileKey(settings, req)
	if again := TranscodeProfileKey(settings, req); again != base {
		t.Fatalf("identical inputs changed the key: %q vs %q", base, again)
	}
	if !strings.HasPrefix(base, "web-mp4-v3-") {
		t.Fatalf("key %q lacks the version prefix", base)
	}

	mutations := []struct {
		name     string
		settings func(*TranscodeSettings)
		request  func(*TranscodeV3Request)
	}{
		{"delivery", nil, func(r *TranscodeV3Request) { r.Delivery = TranscodeDeliveryProgressive }},
		{"quality", nil, func(r *TranscodeV3Request) { r.Quality = "720p" }},
		{"video codec", nil, func(r *TranscodeV3Request) { r.VideoCodec = VideoCodecHEVC }},
		{"audio codec", nil, func(r *TranscodeV3Request) { r.AudioCodec = AudioCodecAC3 }},
		{"subtitle mode", nil, func(r *TranscodeV3Request) { r.SubtitleMode = SubtitleModeOff }},
		{"max bitrate", nil, func(r *TranscodeV3Request) { r.MaxBitrateKbps = 2500 }},
		{"audio stream", nil, func(r *TranscodeV3Request) { one := 1; r.AudioStream = &one }},
		{"subtitle stream", nil, func(r *TranscodeV3Request) { two := 2; r.SubtitleStream = &two }},
		{"preset", func(s *TranscodeSettings) { s.EncoderPreset = "slow" }, nil},
		{"crf", func(s *TranscodeSettings) { s.CRF = 20 }, nil},
		{"audio bitrate", func(s *TranscodeSettings) { s.AudioBitrateKbps = 192 }, nil},
		{"downmix", func(s *TranscodeSettings) { s.DownmixAudio = false }, nil},
		{"hardware backend", func(s *TranscodeSettings) { s.HardwareAcceleration = HWVAAPI }, nil},
		{"hardware encode", func(s *TranscodeSettings) { s.HardwareEncode = true }, nil},
		{"tone mapping", func(s *TranscodeSettings) { s.ToneMapping = false }, nil},
		{"tone algorithm", func(s *TranscodeSettings) { s.ToneMappingAlgorithm = ToneMapHable }, nil},
		{"tone mode", func(s *TranscodeSettings) { s.ToneMappingMode = ToneMapModeNever }, nil},
		{"tone peak", func(s *TranscodeSettings) { s.ToneMappingPeakNits = 200 }, nil},
		{"deinterlace", func(s *TranscodeSettings) { s.Deinterlace = DeinterlaceOff }, nil},
		{"segment seconds", func(s *TranscodeSettings) { s.HLSSegmentSeconds = 4 }, nil},
	}
	for _, m := range mutations {
		t.Run(m.name, func(t *testing.T) {
			s2, r2 := settings, req
			if m.settings != nil {
				m.settings(&s2)
			}
			if m.request != nil {
				m.request(&r2)
			}
			if got := TranscodeProfileKey(s2, r2); got == base {
				t.Fatalf("key %q did not change for %s", got, m.name)
			}
		})
	}

	// Two different requests must not collide even when both are valid.
	other := req
	other.Quality = "360p"
	if TranscodeProfileKey(settings, other) == base {
		t.Fatal("different requests collided")
	}
}
