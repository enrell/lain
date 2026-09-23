package catalog


import (
	"strings"
	"testing"
	"testing/quick"
)

// nulFree reports whether a string can never confuse the \x00-separated
// composite keys. User ids and item ids are generated (never contain
// NUL); titles come from filenames — NTFS/ext4 forbid NUL too, so the
// property only has to hold outside it.
func nulFree(s string) bool { return !strings.ContainsRune(s, '\x00') }

// Property: libKey is injective over NUL-free inputs — a composite key
// collision would silently merge two items' rows.
func TestPropertyLibKeyInjective(t *testing.T) {
	err := quick.Check(func(a, b, c, d string) bool {
		if !nulFree(a + b + c + d) {
			return true
		}
		if string(libKey(a, b)) != string(libKey(c, d)) {
			return true
		}
		return a == c && b == d
	}, &quick.Config{MaxCount: 500})
	if err != nil {
		t.Error(err)
	}
}

// Property: TitleKey is idempotent, case-insensitive and
// whitespace-insensitive — the fingerprint built on it must be stable
// across filename noise.
func TestPropertyTitleKey(t *testing.T) {
	err := quick.Check(func(s string) bool {
		k := TitleKey(s)
		if TitleKey(k) != k || strings.Contains(k, "  ") || k != strings.TrimSpace(k) {
			return false
		}
		// Case-insensitivity is an ASCII contract: unicode case maps
		// are not symmetric (e.g. ß, İ) and folding them is unicode's
		// business, not TitleKey's.
		if strings.IndexFunc(s, func(r rune) bool { return r > 127 }) >= 0 {
			return true
		}
		return TitleKey(strings.ToUpper(s)) == k && TitleKey(strings.ToLower(s)) == k
	}, &quick.Config{MaxCount: 500})
	if err != nil {
		t.Error(err)
	}
}

// Property: Fingerprint is deterministic and embeds each field — same
// logical item must collide, differing season/episode must not.
func TestPropertyFingerprintStable(t *testing.T) {
	err := quick.Check(func(lib, kind, title string, season, episode, year uint8) bool {
		if !nulFree(lib + kind + title) {
			return true
		}
		a := Fingerprint(lib, kind, title, int(season), int(episode), int(year))
		b := Fingerprint(lib, kind, title, int(season), int(episode), int(year))
		if a != b {
			return false
		}
		c := Fingerprint(lib, kind, title, int(season), int(episode)+1, int(year))
		return a != c
	}, &quick.Config{MaxCount: 500})
	if err != nil {
		t.Error(err)
	}
}
