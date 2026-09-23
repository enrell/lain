package probe

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-23

import (
	"math"
	"testing"
)

// FuzzParse feeds arbitrary bytes to the ffprobe JSON parser. ffprobe
// output is external-tool data — malformed, truncated or hostile JSON
// must produce an error or a sane MediaInfo, never a panic.
//
// Campaign: go test -fuzz=FuzzParse -fuzztime=60s ./internal/plugins/probe/
func FuzzParse(f *testing.F) {
	for _, s := range []string{
		`{"format":{"format_name":"matroska,webm","duration":"12.5","bit_rate":"8000000"},` +
			`"streams":[{"codec_type":"video","codec_name":"hevc","width":1920,"height":1080,` +
			`"pix_fmt":"yuv420p10le"},{"codec_type":"audio","codec_name":"aac","channels":2}]}`,
		`{}`,
		`{"format":{},"streams":[]}`,
		`{"format":{"duration":"abc"}}`,
		`{"format":{"duration":"NaN"}}`,
		`{"format":{"duration":"-42"}}`,
		`{"format":{"duration":"1e999"}}`,
		`{"streams":[{"codec_type":"data","codec_name":"bin"},{"codec_type":"video"}]}`,
		`{"streams":[{"codec_type":"video","width":-1,"height":999999999}]}`,
		`null`,
		`[1,2,3]`,
		`"string"`,
		`{"streams":` + string(make([]byte, 0)) + `}`,
	} {
		f.Add([]byte(s))
	}
	f.Fuzz(func(t *testing.T, data []byte) {
		info, err := parse(data)
		if err != nil {
			return
		}
		if math.IsNaN(info.Duration) || math.IsInf(info.Duration, 0) || info.Duration < 0 {
			t.Fatalf("insane duration %v from %q", info.Duration, data)
		}
		for _, s := range info.Streams {
			switch s.Type {
			case "video", "audio", "subtitle":
			default:
				t.Fatalf("unfiltered stream type %q survived parse", s.Type)
			}
			if s.Width < 0 || s.Height < 0 || s.Channels < 0 {
				t.Fatalf("negative geometry in %+v", s)
			}
		}
	})
}
