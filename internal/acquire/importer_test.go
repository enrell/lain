package acquire

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/plugins/identify"
	"github.com/enrell/lain/internal/plugins/release"
)

func parse(name, kind string) contracts.Release { return release.Tokenize(name, kind) }

func touch(t *testing.T, p string, n int) File {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(p, make([]byte, n), 0o644); err != nil {
		t.Fatal(err)
	}
	return File{Path: p, Rel: filepath.Base(p), Length: int64(n)}
}

func TestPlanImportNaming(t *testing.T) {
	src := t.TempDir()
	cases := []struct {
		kind   string
		files  []string
		grab   string
		titles []string
		want   []string
	}{
		{"anime", []string{"[Fansub-A] Sousou no Frieren - 05 [1080p].mkv"}, "[Fansub-A] Sousou no Frieren - 05 [1080p]", nil,
			[]string{"Sousou no Frieren/[Fansub-A] Sousou no Frieren - 05.mkv"}},
		{"anime", []string{"[Fansub-A] One Piece - 1100 [720p].mkv"}, "[Fansub-A] One Piece - 1100", []string{"ONE PIECE"},
			[]string{"ONE PIECE/[Fansub-A] ONE PIECE - 1100.mkv"}},
		{"series", []string{"Show.Name.S01E02.1080p.WEB-DL.x264-GRP.mkv", "Show.Name.S01E03.1080p.WEB-DL.x264-GRP.mkv", "sample.mkv"},
			"Show.Name.S01.1080p.WEB-DL.x264-GRP", []string{"Show: Name"},
			[]string{"Show Name/Season 01/Show Name - S01E02.mkv", "Show Name/Season 01/Show Name - S01E03.mkv"}},
		{"movie", []string{"Movie.Title.2019.2160p.BluRay.x265-GRP.mkv", "Featurette.mkv"}, "Movie.Title.2019.2160p.BluRay.x265-GRP", nil,
			[]string{"Movie Title (2019)/Movie Title (2019).mkv"}},
		{"manga", []string{"Frieren v01 c001.cbz", "notes.txt"}, "Frieren v01", nil,
			[]string{"Frieren/Frieren v01 c001.cbz"}},
		{"comic", []string{"Saga 054.cbr"}, "Saga #054 (2018)", nil,
			[]string{"Saga/Saga c054.cbr"}},
		{"anime", []string{"Show - 07.mkv"}, "Show - 07 [1080p]", nil,
			[]string{"Show/Show - S00E07.mkv"}},
		{"series", []string{"Show.S02E05E06.720p.mkv"}, "Show.S02E05E06.720p", nil,
			[]string{"Show/Season 02/Show - S02E05-E06.mkv"}},
	}
	for i, c := range cases {
		lib := contracts.Library{ID: "lib", Type: c.kind, Path: filepath.Join(t.TempDir(), "lib")}
		var files []File
		for j, f := range c.files {
			size := 1000 - j // the first file is the largest (movie pick)
			files = append(files, touch(t, filepath.Join(src, c.kind, f), size))
		}
		plan, err := PlanImport(lib, files, parse(c.grab, c.kind), parse, c.titles)
		if err != nil {
			t.Fatalf("case %d: %v", i, err)
		}
		if len(plan) != len(c.want) {
			t.Fatalf("case %d: plan %+v", i, plan)
		}
		for j, it := range plan {
			rel, _ := filepath.Rel(lib.Path, it.Dst)
			if rel != c.want[j] {
				t.Errorf("case %d file %d: %q, want %q", i, j, rel, c.want[j])
			}
		}
	}
}

func TestPlanImportRejectsWrongMedia(t *testing.T) {
	src := t.TempDir()
	lib := contracts.Library{ID: "lib", Type: "manga", Path: t.TempDir()}
	f := touch(t, filepath.Join(src, "Show - 01.mkv"), 10)
	if _, err := PlanImport(lib, []File{f}, contracts.Release{Title: "Show"}, parse, nil); CodeOf(err) != CodeImport {
		t.Fatalf("video into a manga library: %v", err)
	}
}

func TestSafeComponent(t *testing.T) {
	for in, want := range map[string]string{
		"../../etc":      "etc",
		"a/b\\c:d*e?":    "a b c d e",
		"  ..  ":         "Unknown",
		"Show\x00Name":   "ShowName",
		"Re:Zero Season": "Re Zero Season",
	} {
		if got := safeComponent(in); got != want {
			t.Errorf("%q -> %q, want %q", in, got, want)
		}
	}
}

func TestPlaceNeverOverwrites(t *testing.T) {
	dir := t.TempDir()
	src := filepath.Join(dir, "src.mkv")
	if err := os.WriteFile(src, []byte("payload"), 0o644); err != nil {
		t.Fatal(err)
	}
	dst := filepath.Join(dir, "lib", "Show", "Show - 01.mkv")
	mode, err := Place(src, dst, ImportHardlink)
	if err != nil || mode != ImportHardlink {
		t.Fatalf("hardlink: %s %v", mode, err)
	}
	a, _ := os.Stat(src)
	b, _ := os.Stat(dst)
	if !os.SameFile(a, b) {
		t.Fatal("hardlink must share the inode")
	}
	if _, err := Place(src, dst, ImportCopy); CodeOf(err) != CodeImport {
		t.Fatalf("existing destination must be refused: %v", err)
	}
	dst2 := filepath.Join(dir, "lib", "Show", "Show - 02.mkv")
	if _, err := Place(src, dst2, ImportCopy); err != nil {
		t.Fatal(err)
	}
	c, _ := os.Stat(dst2)
	if os.SameFile(a, c) {
		t.Fatal("copy must not share the inode")
	}
	if leftovers, _ := filepath.Glob(filepath.Join(dir, "lib", "Show", "*.lain-import")); len(leftovers) != 0 {
		t.Fatalf("temp files left: %v", leftovers)
	}
	dst3 := filepath.Join(dir, "lib", "Show", "Show - 03.mkv")
	if _, err := Place(src, dst3, ImportMove); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(src); !os.IsNotExist(err) {
		t.Fatal("move must remove the source")
	}
}

func TestSettingsValidate(t *testing.T) {
	s := DefaultSettings("/data")
	if _, err := s.Validate(); err != nil {
		t.Fatal(err)
	}
	for _, bad := range []func(*Settings){
		func(s *Settings) { s.Dir = "relative" },
		func(s *Settings) { s.ListenPort = 70000 },
		func(s *Settings) { s.MaxActive = 0 },
		func(s *Settings) { s.SeedRatio = -1 },
		func(s *Settings) { s.ImportMode = "symlink" },
		func(s *Settings) { s.UploadKBps = -5 },
	} {
		v := DefaultSettings("/data")
		bad(&v)
		if _, err := v.Validate(); CodeOf(err) != CodeInvalid {
			t.Errorf("accepted %+v", v)
		}
	}
}

// TestImportedNamesRoundTrip proves every name the importer writes is
// read back by Lain's own identifiers as the same title and numbers —
// otherwise an import would land in the catalog as an unknown video.
func TestImportedNamesRoundTrip(t *testing.T) {
	cases := []struct {
		kind string
		r    contracts.Release
	}{
		{"anime", contracts.Release{Title: "Sousou no Frieren", Episodes: []int{5}, Absolute: true, Group: "Fansub-A"}},
		{"anime", contracts.Release{Title: "One Piece", Episodes: []int{1100}, Absolute: true, Group: "Fansub-B"}},
		{"anime", contracts.Release{Title: "Show", Episodes: []int{7}, Absolute: true}},
		{"anime", contracts.Release{Title: "Show", Episodes: []int{1, 2}, Absolute: true, Group: "Fansub-A"}},
		{"anime", contracts.Release{Title: "Show", Season: 2, Episodes: []int{3}}},
		{"series", contracts.Release{Title: "Show Name", Season: 1, Episodes: []int{2}}},
		{"series", contracts.Release{Title: "Show Name", Season: 1, Episodes: []int{5, 6}}},
		{"manga", contracts.Release{Title: "Frieren", Volume: 1, Chapter: 1}},
		{"manga", contracts.Release{Title: "Frieren", Volume: 3}},
		{"comic", contracts.Release{Title: "Saga", Chapter: 54}},
	}
	for _, c := range cases {
		ext := ".mkv"
		if readingKind(c.kind) {
			ext = ".cbz"
		}
		rel := destination(c.kind, c.r.Title, c.r, "x"+ext)
		cand := contracts.Candidate{Path: filepath.Join("/lib", rel), LibraryType: c.kind}
		if readingKind(c.kind) {
			p := identify.IdentifyComic(cand)
			if p.Title != c.r.Title || p.Season != c.r.Volume || p.Episode != c.r.Chapter {
				t.Errorf("%s -> %+v", rel, p)
			}
			continue
		}
		p, ok := identify.IdentifyAnime(cand)
		season := c.r.Season
		if c.r.Absolute {
			season = 0
		}
		if !ok || p.Title != c.r.Title || p.Season != season || p.Episode != c.r.Episodes[0] {
			t.Errorf("%s -> ok=%v %+v", rel, ok, p)
		}
	}
	// Movies are identified by the generic fallback from their folder name.
	rel := destination("movie", "Movie Title", contracts.Release{Year: 2019}, "x.mkv")
	if p := identify.IdentifyGeneric(contracts.Candidate{Path: filepath.Join("/lib", rel)}); p.Title != "Movie Title" {
		t.Errorf("%s -> %+v", rel, p)
	}
}
