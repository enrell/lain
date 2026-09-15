package metadata

import (
	"strings"
	"unicode"

	"github.com/enrell/lain/internal/contracts"
)

// normalizeTitle folds a title for comparison: lowercase, alphanumeric
// runes only. "Darling in the FranXX" and "darling-in-the-franxx"
// become the same key.
func normalizeTitle(s string) string {
	var b strings.Builder
	for _, r := range strings.ToLower(s) {
		if unicode.IsLetter(r) || unicode.IsDigit(r) {
			b.WriteRune(r)
		}
	}
	return b.String()
}

type scored struct {
	c     contracts.MetadataCandidate
	order int
	exact bool
}

// MergeCandidates dedups provider outputs by normalized title and
// scores them: exact title match first, then binding precedence order
// (ids arrive in binding order). Candidates that do not plausibly
// match the searched title are dropped, so a fuzzy upstream hit never
// replaces the real identity. Providers beyond limit per source are
// trimmed by the providers themselves; the merge caps the total.
func MergeCandidates(query string, outputs []any, ids []string, limit int) []contracts.MetadataCandidate {
	want := normalizeTitle(query)
	var all []scored
	seen := map[string]bool{}
	for i, out := range outputs {
		list, ok := out.([]contracts.MetadataCandidate)
		if !ok {
			continue
		}
		for _, c := range list {
			if c.Title == "" || !plausibleCandidate(want, c) {
				continue
			}
			key := c.Provider + "\x00" + normalizeTitle(c.Title)
			if seen[key] {
				continue
			}
			seen[key] = true
			all = append(all, scored{c: c, order: i, exact: normalizeTitle(c.Title) == want && want != ""})
		}
	}
	// Stable: exact matches first, then precedence, keeping provider
	// order within ties (insertion sort is stable).
	for i := 1; i < len(all); i++ {
		for j := i; j > 0 && less(all[j], all[j-1]); j-- {
			all[j], all[j-1] = all[j-1], all[j]
		}
	}
	if limit <= 0 || limit > 20 {
		limit = 10
	}
	var out []contracts.MetadataCandidate
	for i, s := range all {
		if i >= limit {
			break
		}
		s.c.Score = float64(len(all)-i) + boolScore(s.exact, 1000)
		out = append(out, s.c)
	}
	if out == nil {
		out = []contracts.MetadataCandidate{}
	}
	return out
}

// plausibleCandidate reports whether a hit really answers the search:
// an exact title, or a title/synonym that is a prefix of the other
// (season and edition variants) with enough length to matter. Local
// NFO hits are curated and already pre-filtered, so they always pass.
func plausibleCandidate(want string, c contracts.MetadataCandidate) bool {
	if want == "" || c.Provider == LocalProviderID {
		return true
	}
	for _, title := range append([]string{c.Title}, c.Synonyms...) {
		if plausibleMatch(want, normalizeTitle(title)) {
			return true
		}
	}
	return false
}

func plausibleMatch(want, have string) bool {
	if have == "" {
		return false
	}
	if have == want {
		return true
	}
	shorter, longer := want, have
	if len(have) < len(want) {
		shorter, longer = have, want
	}
	return len(shorter) >= 4 && strings.HasPrefix(longer, shorter)
}

func less(a, b scored) bool {
	if a.exact != b.exact {
		return a.exact
	}
	return a.order < b.order
}

func boolScore(v bool, n float64) float64 {
	if v {
		return n
	}
	return 0
}

// BestPick returns the top merged candidate (exact match preferred).
func BestPick(merged []contracts.MetadataCandidate) (contracts.MetadataCandidate, bool) {
	if len(merged) == 0 {
		return contracts.MetadataCandidate{}, false
	}
	return merged[0], true
}
