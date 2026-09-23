package transcode


import (
	"math"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// FuzzParseHLSPlaylist feeds arbitrary playlist text through the raw
// playlist parser. ffmpeg writes the input, but segment names carry
// operator-controlled source paths, and a truncated file mid-write is
// routine — the parser must stay sane on all of it.
//
// Campaign: go test -fuzz=FuzzParseHLSPlaylist -fuzztime=60s ./internal/plugins/transcode/
func FuzzParseHLSPlaylist(f *testing.F) {
	for _, s := range []string{
		"#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:0\n" +
			"#EXTINF:4.000,\nseg00000.m4s\n#EXTINF:4.000,\nseg00001.m4s\n#EXT-X-ENDLIST\n",
		"#EXTM3U\n#EXT-X-DISCONTINUITY\n#EXTINF:2.0,\nseg00009.ts\n",
		"#EXT-X-MEDIA-SEQUENCE:-5\n#EXTINF:1.0,\nseg00000.m4s\n",
		"#EXTINF:abc,\nseg00000.m4s\n",
		"#EXTINF:NaN,\nseg00000.m4s\n",
		"#EXTINF:-2.5,\nseg00000.m4s\n",
		"#EXTINF:1e999,\nseg00000.m4s\n",
		"#EXT-X-MEDIA-SEQUENCE:99999999999999999999999\n#EXTINF:1,\nx\n",
		"#EXTM3U",
		"",
		"seg00000.m4s\nseg00001.m4s\n",
		"#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:1,\nseg00000.m4s\n",
		"\x00binary\x00\n",
		"#EXTINF:1,\n" + strings.Repeat("a", 70000) + "\n",
	} {
		f.Add(s)
	}
	f.Fuzz(func(t *testing.T, content string) {
		path := filepath.Join(t.TempDir(), "index.raw.m3u8")
		if err := os.WriteFile(path, []byte(content), 0o600); err != nil {
			t.Fatal(err)
		}
		segs, _, err := parseHLSPlaylist(path)
		if err != nil {
			return
		}
		for i, seg := range segs {
			if seg.URI == "" {
				t.Fatalf("segment %d has empty URI (input %q)", i, content)
			}
			if math.IsNaN(seg.Duration) || math.IsInf(seg.Duration, 0) {
				t.Fatalf("segment %d duration %v is not finite", i, seg.Duration)
			}
			if math.IsNaN(seg.StartSec) || math.IsNaN(seg.EndSec) {
				t.Fatalf("segment %d timeline NaN (duration %v)", i, seg.Duration)
			}
		}
	})
}

// FuzzParseSegmentIndex covers the segment filename parser.
func FuzzParseSegmentIndex(f *testing.F) {
	for _, s := range []string{
		"seg00000.m4s", "seg00042.m4s", "seg999999.m4s", "seg00000.ts",
		"seg-1.m4s", "seg.m4s", "seg00000", "seg 01.m4s", "xseg00001.m4s",
		"seg00001.m4sx", "seg00001.ts.m4s", "", "seg１２３.m4s",
	} {
		f.Add(s)
	}
	f.Fuzz(func(t *testing.T, name string) {
		n, ok := parseSegmentIndex(name)
		if !ok {
			return
		}
		if n < 0 {
			t.Fatalf("negative index %d from %q", n, name)
		}
		if !strings.HasPrefix(name, hlsSegmentPrefix) {
			t.Fatalf("parsed %q without the %q prefix", name, hlsSegmentPrefix)
		}
	})
}
