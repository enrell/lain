package acquire

import (
	"strings"
	"testing"

	"github.com/enrell/lain/internal/contracts"
)

func cand(title string, seeders int, size int64) contracts.SearchResult {
	return contracts.SearchResult{Title: title, Seeders: seeders, Size: size, Protocol: contracts.ProtocolTorrent}
}

func TestProfileValidate(t *testing.T) {
	if _, err := DefaultProfile().Validate(); err != nil {
		t.Fatal(err)
	}
	for _, bad := range []func(*Profile){
		func(p *Profile) { p.Name = "" },
		func(p *Profile) { p.Resolutions = []string{"1080p", "potato"} },
		func(p *Profile) { p.Resolutions = []string{"1080p", "1080p"} },
		func(p *Profile) { p.Cutoff = "2160p"; p.Resolutions = []string{"1080p", "720p"} },
		func(p *Profile) { p.Sources = []string{"vhs"} },
		func(p *Profile) { p.MinSizeMB, p.MaxSizeMB = 500, 100 },
		func(p *Profile) { p.MinSeeders = -1 },
	} {
		p := DefaultProfile()
		bad(&p)
		if _, err := p.Validate(); CodeOf(err) != CodeInvalid {
			t.Errorf("accepted %+v", p)
		}
	}
}

func TestEvaluateAcceptsAndRanks(t *testing.T) {
	p := DefaultProfile()
	p.PreferredGroups = []string{"Fansub-A"}
	parse := func(s string) contracts.Release { return parse(s, "anime") }
	a := Evaluate(p, cand("[Fansub-A] Show - 05 [1080p]", 10, 700<<20), parse("[Fansub-A] Show - 05 [1080p]"), 1)
	b := Evaluate(p, cand("[Fansub-B] Show - 05 [1080p]", 10, 700<<20), parse("[Fansub-B] Show - 05 [1080p]"), 1)
	c := Evaluate(p, cand("[Fansub-B] Show - 05 [720p]", 500, 300<<20), parse("[Fansub-B] Show - 05 [720p]"), 1)
	for _, d := range []Decision{a, b, c} {
		if !d.Accepted {
			t.Fatalf("rejected: %+v", d)
		}
	}
	if !(a.Score > b.Score && b.Score > c.Score) {
		t.Fatalf("preferred group, then resolution must lead seeders: %d %d %d", a.Score, b.Score, c.Score)
	}
	if a.Quality != "1080p" || c.Quality != "720p" {
		t.Fatalf("quality: %q %q", a.Quality, c.Quality)
	}
}

func TestEvaluateRejections(t *testing.T) {
	p := DefaultProfile()
	p.Sources = []string{"web", "bluray"}
	p.BlockedGroups = []string{"Fansub-X"}
	p.BlockedWords = []string{"cam"}
	p.MinSeeders = 3
	p.MinSizeMB, p.MaxSizeMB = 100, 2000
	parse := func(s string) contracts.Release { return parse(s, "series") }
	cases := map[string]struct {
		title   string
		seeders int
		size    int64
		units   int
	}{
		"resolution not allowed": {"Show.S01E01.480p.WEB.x264-GRP", 9, 500 << 20, 1},
		"unknown resolution":     {"Show.S01E01.WEB.x264-GRP", 9, 500 << 20, 1},
		"source not allowed":     {"Show.S01E01.1080p.HDTV.x264-GRP", 9, 500 << 20, 1},
		"blocked group":          {"Show.S01E01.1080p.WEB.x264-Fansub-X", 9, 500 << 20, 1},
		"blocked word":           {"Show.S01E01.CAM.1080p.WEB.x264-GRP", 9, 500 << 20, 1},
		"too few seeders":        {"Show.S01E01.1080p.WEB.x264-GRP", 1, 500 << 20, 1},
		"too small":              {"Show.S01E01.1080p.WEB.x264-GRP", 9, 10 << 20, 1},
		"too large":              {"Show.S01E01.1080p.WEB.x264-GRP", 9, 5000 << 20, 1},
	}
	for name, c := range cases {
		d := Evaluate(p, cand(c.title, c.seeders, c.size), parse(c.title), c.units)
		if d.Accepted || len(d.Rejections) == 0 {
			t.Errorf("%s: accepted %+v", name, d)
			continue
		}
		if !strings.Contains(strings.Join(d.Rejections, "; "), strings.Split(name, " ")[0]) {
			t.Errorf("%s: reasons %v", name, d.Rejections)
		}
	}
	// Size bounds are per unit: a 12-episode pack of 6 GB is 500 MB each.
	pack := "Show.S01.1080p.WEB.x264-GRP"
	rel := parse(pack)
	if d := Evaluate(p, cand(pack, 9, 6000<<20), rel, 12); !d.Accepted {
		t.Fatalf("pack within per-episode bounds rejected: %v", d.Rejections)
	}
	// Usenet stays deferred (D-123).
	u := cand("Show.S01E01.1080p.WEB.x264-GRP", 0, 500<<20)
	u.Protocol = contracts.ProtocolUsenet
	if d := Evaluate(p, u, parse(u.Title), 1); d.Accepted {
		t.Fatal("usenet must be rejected")
	}
}

func TestProperPreference(t *testing.T) {
	p := DefaultProfile()
	parse := func(s string) contracts.Release { return parse(s, "series") }
	plain := Evaluate(p, cand("Show.S01E01.1080p.WEB.x264-GRP", 5, 500<<20), parse("Show.S01E01.1080p.WEB.x264-GRP"), 1)
	proper := Evaluate(p, cand("Show.S01E01.PROPER.1080p.WEB.x264-GRP", 5, 500<<20), parse("Show.S01E01.PROPER.1080p.WEB.x264-GRP"), 1)
	if proper.Score <= plain.Score {
		t.Fatalf("proper must outrank: %d vs %d", proper.Score, plain.Score)
	}
}

func TestUpgradeRules(t *testing.T) {
	p := DefaultProfile() // 2160p > 1080p > 720p, cutoff 1080p
	cases := []struct {
		have, cand string
		want       bool
	}{
		{"720p", "1080p", true},   // below cutoff, better
		{"720p", "720p", false},   // not better
		{"1080p", "2160p", false}, // cutoff met: no more upgrades
		{"", "720p", true},        // unknown existing quality counts as worst
		{"720p", "480p", false},   // not allowed at all
	}
	for _, c := range cases {
		if got := IsUpgrade(p, c.have, c.cand); got != c.want {
			t.Errorf("have %q cand %q: %v", c.have, c.cand, got)
		}
	}
	if !MeetsCutoff(p, "2160p") || MeetsCutoff(p, "720p") {
		t.Fatal("cutoff")
	}
}
