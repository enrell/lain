package acquire

import (
	"reflect"
	"testing"

	"github.com/enrell/lain/internal/contracts"
)

func item(title string, season, ep int, path string) contracts.CatalogItem {
	return contracts.CatalogItem{ID: path, LibraryID: "lib", Title: title, Season: season, Episode: ep, FilePath: path}
}

func TestSeasonMapConvertsBothWays(t *testing.T) {
	m := SeasonMap{{Season: 1, First: 1}, {Season: 2, First: 13}, {Season: 3, First: 25}}
	if abs, ok := m.ToAbsolute(2, 1); !ok || abs != 13 {
		t.Fatalf("S02E01 -> %d %v", abs, ok)
	}
	if s, e, ok := m.ToSeasonal(26); !ok || s != 3 || e != 2 {
		t.Fatalf("26 -> S%dE%d %v", s, e, ok)
	}
	if n, ok := m.SeasonLength(2); !ok || n != 12 {
		t.Fatalf("season 2 length %d %v", n, ok)
	}
	if _, ok := m.SeasonLength(3); ok {
		t.Fatal("the last season's length is unknown")
	}
	if _, ok := m.ToAbsolute(9, 1); ok {
		t.Fatal("unmapped season must not convert")
	}
	if err := (SeasonMap{{Season: 1, First: 1}, {Season: 2, First: 1}}).Validate(); err == nil {
		t.Fatal("overlapping seasons must be refused")
	}
}

func TestMonitoredValidate(t *testing.T) {
	ok := Monitored{LibraryID: "lib", Kind: "anime", Title: "Show", Numbering: NumberAbsolute, From: 1}
	if _, err := ok.Validate(); err != nil {
		t.Fatal(err)
	}
	for _, bad := range []Monitored{
		{LibraryID: "lib", Kind: "anime", Title: "", Numbering: NumberAbsolute},
		{LibraryID: "lib", Kind: "anime", Title: "Show", Numbering: "lunar"},
		{LibraryID: "lib", Kind: "movie", Title: "Film", Numbering: NumberAbsolute},
		{LibraryID: "lib", Kind: "manga", Title: "Frieren", Numbering: NumberSeasonal},
		{LibraryID: "lib", Kind: "anime", Title: "Show", Numbering: NumberAbsolute, From: 10, To: 5},
		{LibraryID: "lib", Kind: "series", Title: "Show", Numbering: NumberSeasonal, Seasons: []SeasonWant{{Season: 0, From: 1}}},
	} {
		if _, err := bad.Validate(); CodeOf(err) != CodeInvalid {
			t.Errorf("accepted %+v", bad)
		}
	}
	// Defaults by kind.
	for kind, want := range map[string]string{"movie": NumberMovie, "series": NumberSeasonal, "anime": NumberAbsolute, "manga": NumberChapter, "comic": NumberChapter} {
		m, err := Monitored{LibraryID: "lib", Kind: kind, Title: "X"}.Validate()
		if err != nil || m.Numbering != want {
			t.Errorf("%s default numbering %q %v", kind, m.Numbering, err)
		}
	}
}

func TestWantedAbsolute(t *testing.T) {
	m := Monitored{Kind: "anime", Title: "Show", Aliases: []string{"Show Alt"}, Numbering: NumberAbsolute, From: 1, To: 6}
	items := []contracts.CatalogItem{
		item("Show", 0, 1, "/l/Show/[Fansub-A] Show - 01 [720p].mkv"),
		item("show", 0, 2, "/l/Show/[Fansub-A] Show - 02 [1080p].mkv"),
		item("Show Alt", 0, 4, "/l/Show/Show Alt - 04.mkv"),
		item("Other", 0, 3, "/l/Other/Other - 03.mkv"),
		{Title: "Show", Episode: 5, FilePath: "/l/Show/x05.mkv", Missing: true}, // gone from disk: still wanted
	}
	w := m.Wanted(items, DefaultProfile(), parse)
	if !reflect.DeepEqual(w.Missing, []Unit{{0, 3}, {0, 5}, {0, 6}}) || w.OpenFrom != nil {
		t.Fatalf("missing = %+v open = %+v", w.Missing, w.OpenFrom)
	}
	// Episode 1 is 720p, below the 1080p cutoff: upgradable. Episode 4 has
	// no stated quality: upgradable too.
	if len(w.Upgrades) != 2 || w.Upgrades[0].Unit != (Unit{0, 1}) || w.Upgrades[0].Quality != "720p" || w.Upgrades[1].Unit != (Unit{0, 4}) {
		t.Fatalf("upgrades = %+v", w.Upgrades)
	}
	// Open-ended: every number after the highest present is wanted.
	m.To = 0
	w = m.Wanted(items, DefaultProfile(), parse)
	if !reflect.DeepEqual(w.Missing, []Unit{{0, 3}}) || w.OpenFrom == nil || *w.OpenFrom != (Unit{0, 5}) {
		t.Fatalf("open: missing %+v open %+v", w.Missing, w.OpenFrom)
	}
}

func TestWantedSeasonalAndMovie(t *testing.T) {
	m := Monitored{Kind: "series", Title: "Show", Numbering: NumberSeasonal, Seasons: []SeasonWant{{Season: 1, From: 1, To: 3}, {Season: 2, From: 1}}}
	items := []contracts.CatalogItem{
		item("Show", 1, 1, "/l/Show/Season 01/Show - S01E01.mkv"),
		item("Show", 1, 3, "/l/Show/Season 01/Show - S01E03.mkv"),
		item("Show", 2, 2, "/l/Show/Season 02/Show - S02E02.mkv"),
	}
	w := m.Wanted(items, DefaultProfile(), parse)
	if !reflect.DeepEqual(w.Missing, []Unit{{1, 2}, {2, 1}}) {
		t.Fatalf("missing = %+v", w.Missing)
	}
	if !reflect.DeepEqual(w.OpenSeasons, map[int]int{2: 3}) {
		t.Fatalf("open seasons = %+v", w.OpenSeasons)
	}
	film := Monitored{Kind: "movie", Title: "Film", Numbering: NumberMovie}
	if w := film.Wanted(nil, DefaultProfile(), parse); !reflect.DeepEqual(w.Missing, []Unit{{0, 1}}) {
		t.Fatalf("movie missing = %+v", w.Missing)
	}
	if w := film.Wanted([]contracts.CatalogItem{item("Film", 0, 0, "/l/Film (2001)/Film (2001).mkv")}, DefaultProfile(), parse); len(w.Missing) != 0 {
		t.Fatalf("present movie still wanted: %+v", w.Missing)
	}
}

func TestCovers(t *testing.T) {
	anime := Monitored{Kind: "anime", Title: "Show", Aliases: []string{"Shou"}, Numbering: NumberAbsolute,
		SeasonMap: SeasonMap{{Season: 1, First: 1}, {Season: 2, First: 13}, {Season: 3, First: 25}}}
	cases := []struct {
		m    Monitored
		name string
		want []Unit
	}{
		{anime, "[Fansub-A] Show - 14 [1080p]", []Unit{{0, 14}}},
		{anime, "[Fansub-A] Shou - 01-03 [1080p]", []Unit{{0, 1}, {0, 2}, {0, 3}}},
		{anime, "Show S02E03 1080p", []Unit{{0, 15}}},             // seasonal release, absolute title
		{anime, "Show S02 1080p BluRay", seq(13, 24)},             // season pack via the map
		{anime, "Show S03 1080p BluRay", nil},                     // last season: length unknown
		{anime, "Other Show - 14 [1080p]", nil},                   // another title
		{Monitored{Kind: "series", Title: "Show", Numbering: NumberSeasonal, SeasonMap: anime.SeasonMap},
			"[Fansub-A] Show - 14 [1080p]", []Unit{{2, 2}}},       // absolute release, seasonal title
		{Monitored{Kind: "series", Title: "Show", Numbering: NumberSeasonal}, "Show.S01E05E06.1080p", []Unit{{1, 5}, {1, 6}}},
		{Monitored{Kind: "series", Title: "Show", Numbering: NumberSeasonal}, "Show S04 Complete 1080p", []Unit{{4, 0}}}, // pack marker
		{Monitored{Kind: "manga", Title: "Frieren", Numbering: NumberChapter}, "Frieren v03 c025.cbz", []Unit{{0, 25}}},
		{Monitored{Kind: "manga", Title: "Frieren", Numbering: NumberVolume}, "Frieren v03.cbz", []Unit{{0, 3}}},
		{Monitored{Kind: "movie", Title: "Film", Year: 2001, Numbering: NumberMovie}, "Film 2001 1080p BluRay", []Unit{{0, 1}}},
		{Monitored{Kind: "movie", Title: "Film", Year: 2001, Numbering: NumberMovie}, "Film 1998 1080p BluRay", nil}, // other year
	}
	for _, c := range cases {
		got := c.m.Covers(parse(c.name, c.m.Kind))
		if !reflect.DeepEqual(got, c.want) {
			t.Errorf("%s / %q: %+v, want %+v", c.m.Numbering, c.name, got, c.want)
		}
	}
}

func seq(a, b int) []Unit {
	var out []Unit
	for i := a; i <= b; i++ {
		out = append(out, Unit{0, i})
	}
	return out
}
