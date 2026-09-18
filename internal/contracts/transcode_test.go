package contracts

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
