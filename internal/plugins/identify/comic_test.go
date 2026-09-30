package identify

import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
)

func TestIdentifyComic(t *testing.T) {
	cases := []struct {
		path, lib     string
		kind, title   string
		season, ep, y int
	}{
		{"/c/Space Saga - Ep 01.cbz", "comic", "comic", "Space Saga", 0, 1, 0},
		{"/c/Space Saga/Space Saga 012 (2019) (Digital) [Fansub-A].cbz", "comic", "comic", "Space Saga", 0, 12, 2019},
		{"/c/Space Saga #7.cbz", "comic", "comic", "Space Saga", 0, 7, 0},
		{"/m/Tiny Blade v03 (Digital).cbz", "manga", "manga", "Tiny Blade", 3, 0, 0},
		{"/m/Tiny Blade - Volume 12.cbr", "manga", "manga", "Tiny Blade", 12, 0, 0},
		{"/m/Tiny Blade Vol. 02 Ch. 015.cbz", "manga", "manga", "Tiny Blade", 2, 15, 0},
		{"/m/Tiny Blade - c045.cbz", "manga", "manga", "Tiny Blade", 0, 45, 0},
		{"/m/Tiny Blade/Vol 04.cb7", "manga", "manga", "Tiny Blade", 4, 0, 0},
		{"/m/Tiny_Blade_Chapter_9.cbz", "manga", "manga", "Tiny Blade", 0, 9, 0},
		{"/m/One Shot.cbz", "manga", "manga", "One Shot", 0, 0, 0},
		{"/c/Series 2 - Ep 04.cbz", "comic", "comic", "Series 2", 0, 4, 0},
	}
	for _, c := range cases {
		p := IdentifyComic(contracts.Candidate{Path: c.path, LibraryType: c.lib})
		if p.Kind != c.kind || p.Title != c.title || p.Season != c.season || p.Episode != c.ep || p.Year != c.y {
			t.Errorf("%s: got %s %q S%d E%d y%d, want %s %q S%d E%d y%d", c.path,
				p.Kind, p.Title, p.Season, p.Episode, p.Year, c.kind, c.title, c.season, c.ep, c.y)
		}
		if !p.Accepted() {
			t.Errorf("%s: not accepted", c.path)
		}
	}
}

func TestIdentifyComicDeclinesOtherFiles(t *testing.T) {
	for _, p := range []string{"/v/Show - 01.mkv", "/b/book.pdf", "/c/x.txt"} {
		if IdentifyComic(contracts.Candidate{Path: p}).Accepted() {
			t.Errorf("%s must be declined", p)
		}
	}
}
