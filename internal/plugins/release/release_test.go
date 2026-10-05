package release

import (
	"encoding/json"
	"net"
	"net/http"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/testutil/contract"
)

func TestTokenize(t *testing.T) {
	cases := []struct {
		name, kind string
		want       contracts.Release
	}{
		{"[Fansub-A] Sousou no Frieren - 05 (1080p) [ABCD1234].mkv", "anime", contracts.Release{
			Title: "Sousou no Frieren", Episodes: []int{5}, Absolute: true, Group: "Fansub-A", Resolution: "1080p"}},
		{"[Fansub-A] Sousou no Frieren - 01-04 [1080p] (Batch)", "anime", contracts.Release{
			Title: "Sousou no Frieren", Episodes: []int{1, 2, 3, 4}, Absolute: true, Group: "Fansub-A", Resolution: "1080p"}},
		{"[Fansub-B] One Piece - 1100v2 [720p]", "anime", contracts.Release{
			Title: "One Piece", Episodes: []int{1100}, Absolute: true, Version: 2, Group: "Fansub-B", Resolution: "720p"}},
		{"Show.Name.S01E02.1080p.WEB-DL.x264-GROUP", "series", contracts.Release{
			Title: "Show Name", Season: 1, Episodes: []int{2}, Group: "GROUP", Resolution: "1080p", Source: "web", Codec: "h264"}},
		{"Show.Name.S01E05E06.PROPER.720p.HDTV.x265-GRP", "series", contracts.Release{
			Title: "Show Name", Season: 1, Episodes: []int{5, 6}, Proper: true, Group: "GRP", Resolution: "720p", Source: "hdtv", Codec: "h265"}},
		{"Show Name S01E01-E03 1080p", "series", contracts.Release{
			Title: "Show Name", Season: 1, Episodes: []int{1, 2, 3}, Resolution: "1080p"}},
		{"Show Name S02 1080p BluRay x265-GROUP", "series", contracts.Release{
			Title: "Show Name", Season: 2, SeasonPack: true, Group: "GROUP", Resolution: "1080p", Source: "bluray", Codec: "h265"}},
		{"Show Name Season 3 Complete 720p", "series", contracts.Release{
			Title: "Show Name", Season: 3, SeasonPack: true, Resolution: "720p"}},
		{"Movie Title 2019 2160p UHD BluRay x265-GROUP", "movie", contracts.Release{
			Title: "Movie Title", Year: 2019, Group: "GROUP", Resolution: "2160p", Source: "bluray", Codec: "h265"}},
		{"Movie Title (1999) [1080p] REPACK", "movie", contracts.Release{
			Title: "Movie Title", Year: 1999, Resolution: "1080p", Repack: true}},
		{"1917 (2019) 1080p", "movie", contracts.Release{Title: "1917", Year: 2019, Resolution: "1080p"}},
		{"Frieren v01 c001.cbz", "manga", contracts.Release{Title: "Frieren", Volume: 1, Chapter: 1}},
		{"[Fansub-A] Frieren Vol. 3 Ch. 25.cbz", "manga", contracts.Release{Title: "Frieren", Volume: 3, Chapter: 25, Group: "Fansub-A"}},
		{"Saga #054 (2018).cbr", "comic", contracts.Release{Title: "Saga", Chapter: 54, Year: 2018}},
		{"Saga 054 (2018).cbr", "comic", contracts.Release{Title: "Saga", Chapter: 54, Year: 2018}},
		{"20th Century Boys v01.cbz", "manga", contracts.Release{Title: "20th Century Boys", Volume: 1}},
		{"Show Name - The Pilot [WEB-1080p]", "series", contracts.Release{Title: "Show Name", Resolution: "1080p", Source: "web"}},
	}
	for _, c := range cases {
		got := Tokenize(c.name, c.kind)
		c.want.Parser = TokenizerID
		if !reflect.DeepEqual(got, c.want) {
			t.Errorf("%q\n got %+v\nwant %+v", c.name, got, c.want)
		}
	}
}

func TestTokenizeNeverPanicsOnOddInput(t *testing.T) {
	for _, s := range []string{"", " ", "-", "[", "[]", "((((", "S", "S01E", "s1e999999", "- - -", "x.y.z", strings.Repeat("[a]", 300)} {
		_ = Tokenize(s, "anime")
		_ = Tokenize(s, "manga")
	}
}

func FuzzTokenize(f *testing.F) {
	for _, s := range []string{"[Fansub-A] Show - 01-12 [1080p]", "Show.S01E02E03.x264-GRP", "Title v01 c001.cbz", "Movie (2001)"} {
		f.Add(s, "anime")
	}
	f.Fuzz(func(t *testing.T, name, kind string) {
		r := Tokenize(name, kind)
		if len(r.Episodes) > 2001 || len(r.Title) > 200 {
			t.Fatalf("unbounded parse: %d episodes, %d title bytes", len(r.Episodes), len(r.Title))
		}
	})
}

func TestTokenizerContract(t *testing.T) {
	contract.Run(t, Tokenizer{}, []contract.Cap{{Name: contracts.CapReleaseParse, Sample: contracts.ReleaseParseInput{Name: "Show S01E01"}}})
	contract.Run(t, NewModel(filepath.Join(t.TempDir(), "none.sock")), []contract.Cap{{Name: contracts.CapReleaseParse, Sample: contracts.ReleaseParseInput{Name: "Show S01E01"}}})
}

// fakeParser serves the lain-parser protocol on a unix socket.
func fakeParser(t *testing.T, record any) string {
	t.Helper()
	sock := filepath.Join(t.TempDir(), "parser.sock")
	ln, err := net.Listen("unix", sock)
	if err != nil {
		t.Fatal(err)
	}
	mux := http.NewServeMux()
	mux.HandleFunc("GET /health", func(w http.ResponseWriter, _ *http.Request) { _, _ = w.Write([]byte(`{"status":"ok"}`)) })
	mux.HandleFunc("POST /parse", func(w http.ResponseWriter, r *http.Request) {
		var in struct{ Filename string }
		_ = json.NewDecoder(r.Body).Decode(&in)
		if in.Filename == "" {
			w.WriteHeader(400)
			return
		}
		_ = json.NewEncoder(w).Encode(map[string]any{"record": record, "spans": []any{}})
	})
	srv := &http.Server{Handler: mux}
	go func() { _ = srv.Serve(ln) }()
	t.Cleanup(func() { _ = srv.Close() })
	return sock
}

func TestModelMergesWithTokenizerTags(t *testing.T) {
	sock := fakeParser(t, map[string]any{
		"title": "Sousou no Frieren", "season": 1, "episodes": []int{5}, "release_group": "Fansub-A", "resolution": "1080p",
	})
	m := NewModel(sock)
	if err := m.Health(); err != nil {
		t.Fatalf("health: %v", err)
	}
	out, err := m.Invoke(contracts.CapReleaseParse, contracts.ReleaseParseInput{Name: "[Fansub-A] Frieren - 05 [1080p][WEB][HEVC]"})
	if err != nil {
		t.Fatal(err)
	}
	r := out.(contracts.Release)
	if r.Parser != ModelID || r.Title != "Sousou no Frieren" || r.Season != 1 || !reflect.DeepEqual(r.Episodes, []int{5}) ||
		r.Absolute || r.Source != "web" || r.Codec != "h265" || r.Group != "Fansub-A" {
		t.Fatalf("merged = %+v", r)
	}
}

func TestModelAbstainsAndIsUnavailable(t *testing.T) {
	m := NewModel(fakeParser(t, nil))
	out, err := m.Invoke(contracts.CapReleaseParse, contracts.ReleaseParseInput{Name: "x"})
	if err != nil || out.(contracts.Release).Accepted() {
		t.Fatalf("abstention: %+v %v", out, err)
	}
	gone := NewModel(filepath.Join(t.TempDir(), "missing.sock"))
	_, err = gone.Invoke(contracts.CapReleaseParse, contracts.ReleaseParseInput{Name: "x"})
	if ce, ok := err.(*core.Error); !ok || ce.Code != "dependency-unavailable" {
		t.Fatalf("missing socket: %v", err)
	}
	if gone.Health() == nil {
		t.Fatal("health must report a missing socket")
	}
}
