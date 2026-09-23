package contracts

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-22

import "testing"

// A proposal is only accept-worthy with both a title and positive
// confidence — each half of the predicate is load-bearing.
func TestProposalAccepted(t *testing.T) {
	cases := []struct {
		p    Proposal
		want bool
	}{
		{Proposal{Title: "Show", Confidence: 0.9}, true},
		{Proposal{Title: "Show", Confidence: 1}, true},
		{Proposal{Title: "", Confidence: 0.9}, false},
		{Proposal{Title: "Show", Confidence: 0}, false},
		{Proposal{Title: "Show", Confidence: -0.5}, false},
		{Proposal{}, false},
	}
	for i, tc := range cases {
		if got := tc.p.Accepted(); got != tc.want {
			t.Errorf("case %d: Accepted()=%v, want %v", i, got, tc.want)
		}
	}
}

// NormalizePage clamps paging input to the documented bounds:
// limit 1..500 (default 50), offset >= 0, sort is title unless "recent".
func TestNormalizePageClamps(t *testing.T) {
	cases := []struct {
		limit, offset int
		sort          string
		want          PageParams
	}{
		{0, 0, "", PageParams{Limit: 50, Offset: 0, Sort: "title"}},
		{-5, -3, "bogus", PageParams{Limit: 50, Offset: 0, Sort: "title"}},
		{1, 1, "recent", PageParams{Limit: 1, Offset: 1, Sort: "recent"}},
		{500, 99, "recent", PageParams{Limit: 500, Offset: 99, Sort: "recent"}},
		{501, 0, "title", PageParams{Limit: 500, Offset: 0, Sort: "title"}},
		{100, 0, "title", PageParams{Limit: 100, Offset: 0, Sort: "title"}},
	}
	for i, tc := range cases {
		if got := NormalizePage(tc.limit, tc.offset, tc.sort); got != tc.want {
			t.Errorf("case %d: NormalizePage(%d,%d,%q)=%+v, want %+v",
				i, tc.limit, tc.offset, tc.sort, got, tc.want)
		}
	}
}
