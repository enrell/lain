package subtitle

import (
	"encoding/binary"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

func TestLanguageFolding(t *testing.T) {
	for in, want := range map[string]string{
		"en": "eng", "EN": "eng", "eng": "eng", "pt-BR": "por", "pt_br": "por", "por": "por",
		"fre": "fra", "fr": "fra", "ger": "deu", "de": "deu", "ja": "jpn", "zh-TW": "zho", "chi": "zho",
		"xx": "", "": "", "english": "", "e1": "",
	} {
		if got := Language(in); got != want {
			t.Errorf("%q -> %q, want %q", in, got, want)
		}
	}
	if Tag("eng", "") != "en" || Tag("por", "pt-BR") != "pt-BR" || Tag("fil", "") != "fil" {
		t.Fatalf("tags: %q %q %q", Tag("eng", ""), Tag("por", "pt-BR"), Tag("fil", ""))
	}
}

func TestDiscoverSidecars(t *testing.T) {
	dir := t.TempDir()
	media := filepath.Join(dir, "Show - S01E01.mkv")
	for _, n := range []string{
		"Show - S01E01.mkv",
		"Show - S01E01.srt",
		"Show - S01E01.en.srt",
		"Show - S01E01.pt-BR.forced.ass",
		"Show - S01E01.eng.sdh.vtt",
		"Show - S01E01.ja.hi.ssa",
		"Show - S01E01.notalanguage.srt",
		"Show - S01E01.en.txt",  // not a subtitle extension
		"Show - S01E010.en.srt", // another episode's prefix
		"Show - S01E02.en.srt",  // another episode
		"Other.en.srt",
	} {
		if err := os.WriteFile(filepath.Join(dir, n), []byte("x"), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	got := Discover(media)
	type row struct {
		Name, Lang  string
		Forced, HI  bool
		Format, Tag string
	}
	var rows []row
	for _, s := range got {
		rows = append(rows, row{filepath.Base(s.Path), s.Language, s.Forced, s.HI, s.Format, s.Tag})
	}
	want := []row{
		{"Show - S01E01.en.srt", "eng", false, false, "srt", "en"},
		{"Show - S01E01.eng.sdh.vtt", "eng", false, true, "vtt", "eng"},
		{"Show - S01E01.ja.hi.ssa", "jpn", false, true, "ssa", "ja"},
		{"Show - S01E01.notalanguage.srt", "und", false, false, "srt", "notalanguage"},
		{"Show - S01E01.pt-BR.forced.ass", "por", true, false, "ass", "pt-BR"},
		{"Show - S01E01.srt", "und", false, false, "srt", ""},
	}
	if !reflect.DeepEqual(rows, want) {
		t.Fatalf("got  %+v\nwant %+v", rows, want)
	}
}

func TestSidecarName(t *testing.T) {
	cases := []struct {
		lang, region string
		forced, hi   bool
		format, want string
	}{
		{"eng", "", false, false, "srt", "/l/Show - S01E01.en.srt"},
		{"por", "pt-BR", true, false, "ass", "/l/Show - S01E01.pt-BR.forced.ass"},
		{"jpn", "", false, true, "srt", "/l/Show - S01E01.ja.sdh.srt"},
	}
	for _, c := range cases {
		if got := SidecarPath("/l/Show - S01E01.mkv", c.lang, c.region, c.forced, c.hi, c.format); got != c.want {
			t.Errorf("%+v -> %q", c, got)
		}
	}
	// The name Lain writes is the name Discover reads back.
	dir := t.TempDir()
	media := filepath.Join(dir, "Film (2001).mkv")
	p := SidecarPath(media, "por", "pt-BR", true, false, "srt")
	_ = os.WriteFile(p, []byte("x"), 0o644)
	if s := Discover(media); len(s) != 1 || s[0].Language != "por" || !s[0].Forced || s[0].Tag != "pt-BR" {
		t.Fatalf("round trip: %+v", s)
	}
}

// refHash is the published OpenSubtitles algorithm written the slow,
// obvious way, to check MovieHash against.
func refHash(b []byte) string {
	h := uint64(len(b))
	sum := func(chunk []byte) {
		for i := 0; i+8 <= len(chunk); i += 8 {
			h += binary.LittleEndian.Uint64(chunk[i:])
		}
	}
	n := min(65536, len(b))
	sum(b[:n])
	sum(b[len(b)-n:])
	return fmt.Sprintf("%016x", h)
}

func TestMovieHash(t *testing.T) {
	dir := t.TempDir()
	data := make([]byte, 300_000)
	for i := range data {
		data[i] = byte(i*131 + 7)
	}
	p := filepath.Join(dir, "a.mkv")
	if err := os.WriteFile(p, data, 0o644); err != nil {
		t.Fatal(err)
	}
	got, err := MovieHash(p)
	if err != nil || got != refHash(data) || len(got) != 16 {
		t.Fatalf("hash %q (want %q) %v", got, refHash(data), err)
	}
	// The middle does not count; the head does.
	data[150_000] ^= 0xff
	_ = os.WriteFile(p, data, 0o644)
	if again, _ := MovieHash(p); again != got {
		t.Fatal("a change in the middle must not change the hash")
	}
	data[10] ^= 0xff
	_ = os.WriteFile(p, data, 0o644)
	if again, _ := MovieHash(p); again == got {
		t.Fatal("a change in the head must change the hash")
	}
	small := filepath.Join(dir, "small.mkv")
	_ = os.WriteFile(small, []byte("tiny"), 0o644)
	if _, err := MovieHash(small); err == nil {
		t.Fatal("files under 64 KiB have no hash")
	}
}
