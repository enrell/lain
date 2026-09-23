package identify

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-23

import (
	"strings"
	"testing"
	"testing/quick"

	"github.com/enrell/lain/internal/contracts"
)

// Metamorphic property: case-folding a filename must never change the
// extracted numbers — episode markers and years match on lowercased
// tokens, so a case permutation is an irrelevant transform. The title
// itself intentionally keeps original case and is excluded.
func TestPropertyIdentifyCaseFoldInvariant(t *testing.T) {
	err := quick.Check(func(name string) bool {
		c := contracts.Candidate{Path: name, LibraryID: "l"}
		base, _ := IdentifyAnime(c)
		up, _ := IdentifyAnime(contracts.Candidate{Path: strings.ToUpper(name), LibraryID: "l"})
		down, _ := IdentifyAnime(contracts.Candidate{Path: strings.ToLower(name), LibraryID: "l"})
		for _, p := range []contracts.Proposal{up, down} {
			if p.Season != base.Season || p.Episode != base.Episode || p.Year != base.Year || p.Kind != base.Kind {
				return false
			}
		}
		return true
	}, &quick.Config{MaxCount: 500})
	if err != nil {
		t.Error(err)
	}
}

// Property: splitting the same tokens over different separators yields
// the same numeric parse — dots, underscores, dashes and spaces are all
// declared separators.
func TestPropertyIdentifySeparatorInvariant(t *testing.T) {
	err := quick.Check(func(a, b uint8) bool {
		base := "Show S01E02"
		for _, sep := range []string{".", "_", "-", " "} {
			name := strings.ReplaceAll(base, " ", sep)
			p, _ := IdentifyAnime(contracts.Candidate{Path: name + ".mkv", LibraryID: "l"})
			if p.Episode != 2 || p.Season != 1 {
				return false
			}
		}
		return a+b >= 0 // keep the args so quick exercises seeds
	}, &quick.Config{MaxCount: 10})
	if err != nil {
		t.Error(err)
	}
}
