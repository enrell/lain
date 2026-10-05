package torrent

import (
	"bytes"
	"crypto/sha1"
	"errors"
	"strings"
	"testing"

	"github.com/enrell/lain/internal/torrent/bencode"
)

func fixture(n int, seed byte) []byte {
	b := make([]byte, n)
	x := uint32(seed) + 1
	for i := range b {
		x = x*1664525 + 1013904223
		b[i] = byte(x >> 24)
	}
	return b
}

func TestBuildAndParseMultiFile(t *testing.T) {
	files := []SourceFile{
		{Path: []string{"[Fansub-A] Show - 01.mkv"}, Data: fixture(40000, 1)},
		{Path: []string{"extras", "notes.txt"}, Data: fixture(123, 2)},
	}
	raw, mi, err := Build("[Fansub-A] Show", files, 16384, []string{"http://tracker-exemplo.invalid/announce", "udp://tracker-exemplo.invalid:6969"})
	if err != nil {
		t.Fatal(err)
	}
	if !mi.Info.Multi || len(mi.Info.Files) != 2 || mi.Info.Length != 40123 || mi.Info.NumPieces() != 3 {
		t.Fatalf("info = %+v", mi.Info)
	}
	if mi.Info.Files[1].Offset != 40000 || mi.Info.Files[1].RelPath() != "[Fansub-A] Show/extras/notes.txt" {
		t.Fatalf("file 1 = %+v", mi.Info.Files[1])
	}
	if mi.Info.PieceSize(2) != 40123-2*16384 {
		t.Fatalf("last piece = %d", mi.Info.PieceSize(2))
	}
	if len(mi.Trackers) != 2 {
		t.Fatalf("trackers = %v", mi.Trackers)
	}
	_, rawd, _ := bencode.DecodeDict(raw)
	if mi.InfoHash != sha1.Sum(rawd["info"]) || !bytes.Equal(mi.Info.Raw, rawd["info"]) {
		t.Fatal("info hash must be over the original info bytes")
	}
}

func TestSingleFile(t *testing.T) {
	_, mi, err := Build("Movie (2001).mkv", []SourceFile{{Data: fixture(1000, 3)}}, 16384, nil)
	if err != nil || mi.Info.Multi || mi.Info.Files[0].RelPath() != "Movie (2001).mkv" || mi.Info.NumPieces() != 1 {
		t.Fatalf("%+v %v", mi, err)
	}
}

func TestRejectsUnsafeMetainfo(t *testing.T) {
	enc := func(info map[string]any) []byte {
		raw, _ := bencode.Encode(map[string]any{"info": info})
		return raw
	}
	pieces := string(make([]byte, 20))
	cases := map[string]map[string]any{
		"dotdot name":   {"name": "..", "piece length": int64(16), "pieces": pieces, "length": int64(1)},
		"slash name":    {"name": "a/b", "piece length": int64(16), "pieces": pieces, "length": int64(1)},
		"traversal":     {"name": "x", "piece length": int64(16), "pieces": pieces, "files": []any{map[string]any{"length": int64(1), "path": []any{"..", "etc"}}}},
		"zero plen":     {"name": "x", "piece length": int64(0), "pieces": pieces, "length": int64(1)},
		"pieces count":  {"name": "x", "piece length": int64(16), "pieces": pieces + pieces, "length": int64(1)},
		"ragged pieces": {"name": "x", "piece length": int64(16), "pieces": "abc", "length": int64(1)},
		"duplicate":     {"name": "x", "piece length": int64(16), "pieces": pieces, "files": []any{map[string]any{"length": int64(1), "path": []any{"a"}}, map[string]any{"length": int64(0), "path": []any{"A"}}}},
	}
	for name, info := range cases {
		if _, err := ParseMetaInfo(enc(info)); !errors.Is(err, ErrMeta) {
			t.Errorf("%s: %v", name, err)
		}
	}
}

func TestMagnet(t *testing.T) {
	m, err := ParseMagnet("magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Show&tr=http%3A%2F%2Ftracker-exemplo.invalid%2Fa&tr=ftp%3A%2F%2Fx")
	if err != nil || m.Name != "Show" || len(m.Trackers) != 1 || m.InfoHash.Hex() != "0123456789abcdef0123456789abcdef01234567" {
		t.Fatalf("%+v %v", m, err)
	}
	b32, err := ParseMagnet("magnet:?xt=urn:btih:AERUKZ4JVPG66AJDIVTYTK6N54ASGRLH")
	if err != nil || b32.InfoHash.Hex() != "0123456789abcdef0123456789abcdef01234567" {
		t.Fatalf("base32: %+v %v", b32, err)
	}
	again, _ := ParseMagnet(m.String())
	if again.InfoHash != m.InfoHash || again.Name != m.Name {
		t.Fatal("magnet round trip")
	}
	for _, bad := range []string{"http://x", "magnet:?dn=x", "magnet:?xt=urn:btih:zz"} {
		if _, err := ParseMagnet(bad); err == nil {
			t.Errorf("%s accepted", bad)
		}
	}
}

func FuzzParseMetaInfo(f *testing.F) {
	raw, _, _ := Build("x", []SourceFile{{Path: []string{"a"}, Data: []byte("hello")}, {Path: []string{"b", "c"}, Data: []byte("world")}}, 4, []string{"udp://tracker-exemplo.invalid:1"})
	f.Add(raw)
	f.Add([]byte("d4:infod4:name1:x12:piece lengthi1e6:pieces20:aaaaaaaaaaaaaaaaaaaa6:lengthi1eee"))
	f.Fuzz(func(t *testing.T, data []byte) {
		mi, err := ParseMetaInfo(data)
		if err != nil {
			return
		}
		var total int64
		for _, fl := range mi.Info.Files {
			for _, p := range fl.Path {
				if !safeComponent(p) || strings.Contains(p, "..") && p == ".." {
					t.Fatalf("unsafe component %q", p)
				}
			}
			if fl.Offset != total {
				t.Fatal("offsets must be contiguous")
			}
			total += fl.Length
		}
		if total != mi.Info.Length {
			t.Fatal("length mismatch")
		}
	})
}
