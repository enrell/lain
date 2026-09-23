package identify


import (
	"reflect"
	"strings"
	"testing"

	"github.com/enrell/lain/internal/contracts"
)

// FuzzIdentifyAnime drives the full tokenizer+parser pipeline on
// arbitrary filenames. Filenames are untrusted input straight from the
// library directory, so the pipeline must never panic and must keep its
// numeric and string outputs inside sane bounds.
//
// Campaign: go test -fuzz=FuzzIdentifyAnime -fuzztime=60s ./internal/plugins/identify/
func FuzzIdentifyAnime(f *testing.F) {
	for _, s := range []string{
		"[Fansub-A] Show Name - S01E02 [1080p][HEVC][AAC].mkv",
		"Show.Name.S02E10.2160p.WEB-DL.x265-tracker-exemplo.mkv",
		"Show - 03v2 (BD 1080p).mkv",
		"Show 1x03.mkv",
		"Movie Name (2024).mkv",
		"進撃 - 05.mkv",
		"",
		"S",
		"[",
		"]]",
		"S999E9999",
		"E05",
		"1080p",
		"- - -",
		"Show - S01E02-extra - .mkv",
		"a\x00b.mkv",
		strings.Repeat("x", 4096),
	} {
		f.Add(s)
	}
	f.Fuzz(func(t *testing.T, name string) {
		c := contracts.Candidate{Path: name, LibraryID: "l"}
		p, ok := IdentifyAnime(c)
		assertProposalSane(t, p)
		if ok && !p.Accepted() {
			t.Fatalf("ok proposal rejected by Accepted(): %+v", p)
		}
		// Determinism: the same input must always produce the same
		// proposal — the catalog id depends on it.
		p2, ok2 := IdentifyAnime(c)
		if ok2 != ok || !reflect.DeepEqual(p2, p) {
			t.Fatalf("non-deterministic parse of %q: %+v/%v vs %+v/%v", name, p, ok, p2, ok2)
		}
	})
}

// FuzzIdentifyGeneric covers the fallback provider over the same input
// space.
func FuzzIdentifyGeneric(f *testing.F) {
	for _, s := range []string{
		"Documentary.2021.S01E03.720p.mkv",
		"Some Movie (1999).mp4",
		"",
		"###",
	} {
		f.Add(s)
	}
	f.Fuzz(func(t *testing.T, name string) {
		p := IdentifyGeneric(contracts.Candidate{Path: name, LibraryID: "l"})
		assertProposalSane(t, p)
	})
}

func assertProposalSane(t *testing.T, p contracts.Proposal) {
	t.Helper()
	if p.Confidence < 0 || p.Confidence > 1 {
		t.Fatalf("confidence out of [0,1]: %v", p.Confidence)
	}
	if p.Season < 0 || p.Episode < 0 || p.Year < 0 {
		t.Fatalf("negative field in %+v", p)
	}
	if strings.ContainsRune(p.Title, '\x00') {
		t.Fatalf("NUL byte leaked into title %q", p.Title)
	}
}
