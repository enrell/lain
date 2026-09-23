package identify

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-22

import (
	"strings"
	"testing"

	"github.com/enrell/lain/internal/contracts"
)

func ident(t *testing.T, file string) (contracts.Proposal, bool) {
	t.Helper()
	return IdentifyAnime(contracts.Candidate{Path: "/lib/" + file, LibraryID: "lib-1"})
}

func TestInvokeGates(t *testing.T) {
	if _, err := (Anime{}).Invoke("lain.other@1", nil); err == nil ||
		!strings.Contains(err.Error(), "unsupported cap") {
		t.Fatalf("anime: wrong cap must fail with unsupported cap: %v", err)
	}
	if _, err := (Anime{}).Invoke(contracts.CapMediaIdentify, "nope"); err == nil {
		t.Fatal("anime: wrong input type must fail")
	}
	if _, err := (Generic{}).Invoke("lain.other@1", nil); err == nil ||
		!strings.Contains(err.Error(), "unsupported cap") {
		t.Fatalf("generic: wrong cap must fail with unsupported cap: %v", err)
	}
	if _, err := (Generic{}).Invoke(contracts.CapMediaIdentify, 7); err == nil {
		t.Fatal("generic: wrong input type must fail")
	}
	out, err := (Generic{}).Invoke(contracts.CapMediaIdentify, contracts.Candidate{Path: "/x/a.mkv"})
	if err != nil || out.(contracts.Proposal).Kind != "video" {
		t.Fatalf("generic invoke: %v %+v", err, out)
	}
}

func TestStemDoubleExtensions(t *testing.T) {
	for _, tc := range []struct{ file, stem string }{
		{"a.mkv.mp4", "a"},
		{"a.txt.mkv", "a.txt"},
		{"a.mkv", "a"},
		{"noext", "noext"},
	} {
		if got := stemOf(tc.file); got != tc.stem {
			t.Errorf("stemOf(%q)=%q want %q", tc.file, got, tc.stem)
		}
	}
}

func TestTechNumberedVariants(t *testing.T) {
	for _, tc := range []struct {
		token string
		want  bool
	}{
		{"hdr10", true},  // numbered variant of hdr
		{"multi4", true}, // numbered variant of multi
		{"1080p", true},  // direct entry
		{"24", false},    // pure numbers never tech
		{"2", false},     // episode-sized number
		{"x264", true},
		{"vision", true}, // dolby vision tail token
		{"show", false},
		{"", false},
	} {
		if got := isTech(tc.token); got != tc.want {
			t.Errorf("isTech(%q)=%v want %v", tc.token, got, tc.want)
		}
	}
}

func TestParseSxxExxBoundaries(t *testing.T) {
	for _, tc := range []struct {
		tok  string
		s, e int
		ok   bool
	}{
		{"s01e05", 1, 5, true},
		{"s1e2", 1, 2, true},
		{"s12e345", 12, 345, true},
		{"s123e04", 0, 0, false},  // season too many digits
		{"s01e1234", 0, 0, false}, // episode too many digits
		{"s1e", 0, 0, false},
		{"se05", 0, 0, false},
		{"x01e05", 0, 0, false},
		{"sabe05", 0, 0, false},
		{"s1", 0, 0, false},
	} {
		s, e, ok := parseSxxExx(tc.tok)
		if ok != tc.ok || s != tc.s || e != tc.e {
			t.Errorf("parseSxxExx(%q)=(%d,%d,%v) want (%d,%d,%v)", tc.tok, s, e, ok, tc.s, tc.e, tc.ok)
		}
	}
}

func TestParseNxMBoundaries(t *testing.T) {
	for _, tc := range []struct {
		tok  string
		a, b int
		ok   bool
	}{
		{"2x13", 2, 13, true},
		{"12x345", 12, 345, true},
		{"123x45", 0, 0, false}, // season position too wide
		{"1x1234", 0, 0, false}, // episode too many digits
		{"x12", 0, 0, false},
		{"12x", 0, 0, false},
		{"1xy2", 0, 0, false},
		{"axb", 0, 0, false},
	} {
		a, b, ok := parseNxM(tc.tok)
		if ok != tc.ok || a != tc.a || b != tc.b {
			t.Errorf("parseNxM(%q)=(%d,%d,%v) want (%d,%d,%v)", tc.tok, a, b, ok, tc.a, tc.b, tc.ok)
		}
	}
}

func TestStripVersionSuffix(t *testing.T) {
	for _, tc := range []struct{ in, want string }{
		{"24v2", "24"},
		{"1090v3", "1090"},
		{"24v", "24v"},         // no digits after v: not a version
		{"v2", "v2"},           // v at position 0 is not a suffix
		{"24x2", "24x2"},       // only v counts
		{"12345v2", "12345v2"}, // too long for atoi
		{"2", "2"},
	} {
		if got := stripVersionSuffix(tc.in); got != tc.want {
			t.Errorf("stripVersionSuffix(%q)=%q want %q", tc.in, got, tc.want)
		}
	}
}

func TestIsYearBoundaries(t *testing.T) {
	for _, tc := range []struct {
		tok  string
		want bool
	}{
		{"1899", false},
		{"1900", true},
		{"2099", true},
		{"2100", false},
		{"999", false},
		{"10000", false},
		{"20a0", false},
	} {
		if got := isYear(tc.tok); got != tc.want {
			t.Errorf("isYear(%q)=%v want %v", tc.tok, got, tc.want)
		}
	}
}

func TestParseOrdinalSuffixes(t *testing.T) {
	for _, tc := range []struct {
		tok string
		n   int
		ok  bool
	}{
		{"1st", 1, true},
		{"2nd", 2, true},
		{"3rd", 3, true},
		{"4th", 4, true},
		{"21st", 21, true},
		{"th", 0, false},
		{"xth", 0, false},
		{"4t", 0, false},
		{"44", 0, false},
	} {
		n, ok := parseOrdinal(tc.tok)
		if ok != tc.ok || n != tc.n {
			t.Errorf("parseOrdinal(%q)=(%d,%v) want (%d,%v)", tc.tok, n, ok, tc.n, tc.ok)
		}
	}
}

func TestAnimeSemanticCases(t *testing.T) {
	cases := []struct {
		file    string
		accept  bool
		title   string
		season  int
		episode int
		year    int
		kind    string
		evid    string // required evidence marker ("" = don't check)
	}{
		// Group tag excluded from title, tokens after ']' unbracketed.
		{"[Fansub-A] Show S01E01.mkv", true, "Show", 1, 1, 0, "episode", "release-group"},
		// Episode hidden inside brackets: parity fallback.
		{"Show Name [S01E05] 1080p.mkv", true, "Show Name", 1, 5, 0, "episode", "sxxexx"},
		{"Show Name [2x13].mkv", true, "Show Name", 2, 13, 0, "episode", "nxm"},
		// Ordinal + word season; both tokens leave the title. Season
		// words alone carry no episode certainty, so a group tag (or a
		// strong marker) must still be present to accept.
		{"[Fansub-A] Show 4th Season 03 [1080p].mkv", true, "Show", 4, 3, 0, "episode", "season-word"},
		{"[Fansub-A] Show Season 2 - 05.mkv", true, "Show", 2, 2, 0, "episode", "season-word"},
		// Bracketed "Season" cannot satisfy the lookahead.
		{"[Fansub-A] Show 4th [Season] 03.mkv", true, "Show", 0, 3, 0, "episode", ""},
		// Ordinal cap: 31st loses to the "Season N" form (Season 03),
		// while 30th still parses.
		{"[Fansub-A] Show 31st Season 03.mkv", true, "Show", 3, 3, 0, "episode", "season-word"},
		{"[Fansub-A] Show 30th Season 03.mkv", true, "Show", 30, 3, 0, "episode", "season-word"},
		// "Season 31" is over the cap: no season skip, so the 31 itself
		// becomes the weak episode marker and cuts the title.
		{"[Fansub-A] Show Season 31 05.mkv", true, "Show Season", 0, 31, 0, "episode", "episode-number"},
		// First episode marker wins; later numbers stay out.
		{"Show S01E01 S02E02.mkv", true, "Show", 1, 1, 0, "episode", "sxxexx"},
		{"Show S01E01 24.mkv", true, "Show", 1, 1, 0, "episode", ""},
		// Bare number episode; version suffix strips.
		{"[Fansub-A] Show - 24v2 [1080p].mkv", true, "Show", 0, 24, 0, "episode", "episode-number"},
		{"[Fansub-A] Show E05 [1080p].mkv", true, "Show", 0, 5, 0, "episode", "episode-number"},
		{"[Fansub-A] Show EP12 [1080p].mkv", true, "Show", 0, 12, 0, "episode", "episode-number"},
		// Year alone is not episode 2020; tech corroborates a movie.
		{"Movie 2020 1080p.mkv", true, "Movie", 0, 0, 2020, "video", "year"},
		{"[Fansub-A] Movie 2020.mkv", true, "Movie", 0, 0, 2020, "video", ""},
		// First year wins; second ignored.
		{"Show 2020 2021 S01E01.mkv", true, "Show", 1, 1, 2020, "episode", ""},
		// A strong marker overrides a stray number already read as the
		// episode; the title cut follows the strong marker position.
		{"Show 2.0 S01E01.mkv", true, "Show 2 0", 1, 1, 0, "episode", "sxxexx"},
		// Declines: no group, no strong marker, no corroborated year.
		{"Movie 2020.mkv", false, "", 0, 0, 0, "", ""},
		{"random file name.mkv", false, "", 0, 0, 0, "", ""},
		{"[] Show S01E01.mkv", true, "Show", 1, 1, 0, "episode", "sxxexx"}, // empty brackets are not a group
		// Second episode marker must not override the first.
		{"[Fansub-A] Show S01E01 2x03.mkv", true, "Show", 1, 1, 0, "episode", "sxxexx"},
		// Unbracketed tech before the marker counts toward the cut index.
		{"Show 1080p S01E01.mkv", true, "Show", 1, 1, 0, "episode", "sxxexx"},
		// Tech without a group, marker or year still declines.
		{"Movie 1080p.mkv", false, "", 0, 0, 0, "", ""},
		// Bracketed strong marker must not displace an earlier weak one.
		{"[Fansub-A] Show 12 [S01E05].mkv", true, "Show", 0, 12, 0, "episode", "episode-number"},
		// Trailing season-word/ordinal tokens exercise lookahead bounds.
		{"[Fansub-A] Show S01E01 Season.mkv", true, "Show", 1, 1, 0, "episode", ""},
		{"[Fansub-A] Show 12 4th.mkv", true, "Show", 0, 12, 0, "episode", "episode-number"},
		// "s123" has no e-marker: the s-digit scan must terminate, not read past.
		{"[Fansub-A] Show s123 12.mkv", true, "Show s123", 0, 12, 0, "episode", "episode-number"},
		// A stray ']' before '[' must not un-bracket the group tag.
		{"Show ]x [Fansub-A] S01E01.mkv", true, "Show x", 1, 1, 0, "episode", "sxxexx"},
		// Season word at end of name (lookahead must not read past).
		{"[Fansub-A] Show 12 Season.mkv", true, "Show", 0, 12, 0, "episode", "episode-number"},
		// "Season 30" is inside the cap; 31 was rejected above.
		{"[Fansub-A] Show 12 Season 30.mkv", true, "Show", 30, 12, 0, "episode", "season-word"},
	}
	for _, tc := range cases {
		p, ok := ident(t, tc.file)
		if ok != tc.accept {
			t.Errorf("%s: accept=%v want %v", tc.file, ok, tc.accept)
			continue
		}
		if !ok {
			continue
		}
		if p.Title != tc.title || p.Season != tc.season || p.Episode != tc.episode || p.Year != tc.year || p.Kind != tc.kind {
			t.Errorf("%s: %+v want title=%q s=%d e=%d y=%d kind=%s",
				tc.file, p, tc.title, tc.season, tc.episode, tc.year, tc.kind)
		}
		if tc.evid != "" {
			found := false
			for _, e := range p.Evidence {
				if e == tc.evid {
					found = true
				}
			}
			if !found {
				t.Errorf("%s: evidence %q missing from %v", tc.file, tc.evid, p.Evidence)
			}
		}
		if strings.HasPrefix(tc.file, "[]") {
			for _, e := range p.Evidence {
				if e == "release-group" {
					t.Errorf("%s: empty brackets must not count as a group", tc.file)
				}
			}
		}
	}
}

func TestAnimeEvidenceDedupes(t *testing.T) {
	p, ok := ident(t, "[Fansub-A] Show S01E01 1080p 1080p.mkv")
	if !ok {
		t.Fatal("must accept")
	}
	n := 0
	for _, e := range p.Evidence {
		if e == "resolution" {
			n++
		}
	}
	if n != 1 {
		t.Fatalf("resolution evidence duplicated: %v", p.Evidence)
	}
}

func TestAnimeConfidenceExact(t *testing.T) {
	p, ok := ident(t, "[Fansub-A] Show S01E01 [1080p].mkv")
	if !ok {
		t.Fatal("must accept")
	}
	if p.Confidence < 0.87 || p.Confidence > 0.89 { // 0.7 + 0.1 group + 0.08 resolution
		t.Fatalf("conf=%f want ~0.88", p.Confidence)
	}
	p, _ = ident(t, "Show S01E01.mkv")
	if p.Confidence != 0.7 {
		t.Fatalf("bare strong marker conf=%f want 0.7", p.Confidence)
	}
	if p.Confidence > 0.96 {
		t.Fatal("confidence must stay capped at 0.96")
	}
}

func TestGenericKindsAndTitleFallback(t *testing.T) {
	for _, tc := range []struct{ file, kind, title string }{
		{"song.mp3", "audio", "song"},
		{"book.epub", "book", "book"},
		{"issue.cbz", "comic", "issue"},
		{"shot.webp", "photo", "shot"},
		{"clip.mkv", "video", "clip"},
		{"noext", "video", "noext"},
		{"[tag].mp4", "video", "[tag].mp4"}, // all-bracketed title falls back to basename
		{"my.movie-name_2020.mp4", "video", "my movie name 2020"},
	} {
		p := IdentifyGeneric(contracts.Candidate{Path: "/lib/" + tc.file})
		if p.Kind != tc.kind || p.Title != tc.title {
			t.Errorf("%s: kind=%q title=%q want %q/%q", tc.file, p.Kind, p.Title, tc.kind, tc.title)
		}
		if p.Confidence != 0.4 || p.PluginID != GenericID {
			t.Errorf("%s: conf=%f plugin=%q", tc.file, p.Confidence, p.PluginID)
		}
	}
}
