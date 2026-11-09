package identify_test

import (
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/plugins/identify"
	"github.com/enrell/lain/internal/plugins/release"
)

// The parse parity fixture holds the Go parsers' answers for a corpus
// of names (every literal of the parser tests plus edge cases). The
// Rust port (crates/lain-identify) checks itself against the same file,
// so the two parsers cannot drift. LAIN_WRITE_PARITY=1 rewrites the
// answers after an intended Go change.

// parityProposal is an accepted proposal, or null for a decline.
type parityProposal = *contracts.Proposal

type parityIdentify struct {
	Path        string         `json:"path"`
	LibraryType string         `json:"library_type,omitempty"`
	Comic       parityProposal `json:"comic"`
	Anime       parityProposal `json:"anime"`
	Generic     parityProposal `json:"generic"`
}

type parityRelease struct {
	Name    string            `json:"name"`
	Kind    string            `json:"kind,omitempty"`
	Release contracts.Release `json:"release"`
}

type parityDoc struct {
	Format   string           `json:"format"`
	Corpus   []string         `json:"corpus"`
	Identify []parityIdentify `json:"identify"`
	Release  []parityRelease  `json:"release"`
}

const parityFile = "../../../conformance/fixtures/parse-parity.json"

func answer(p contracts.Proposal, ok bool) parityProposal {
	if !ok || !p.Accepted() {
		return nil
	}
	return &p
}

func build(corpus []string) parityDoc {
	doc := parityDoc{Format: "lain.parse-parity@1", Corpus: corpus}
	for _, name := range corpus {
		for _, lt := range []string{"", "manga"} {
			c := contracts.Candidate{Path: filepath.Join("/lib", name), LibraryID: "lib-1", LibraryType: lt}
			comic := identify.IdentifyComic(c)
			anime, ok := identify.IdentifyAnime(c)
			doc.Identify = append(doc.Identify, parityIdentify{
				Path: c.Path, LibraryType: lt,
				Comic:   answer(comic, true),
				Anime:   answer(anime, ok),
				Generic: answer(identify.IdentifyGeneric(c), true),
			})
		}
		for _, kind := range []string{"", "anime", "manga"} {
			r := release.Tokenize(name, kind)
			if r.Episodes == nil {
				r.Episodes = []int{}
			}
			doc.Release = append(doc.Release, parityRelease{Name: name, Kind: kind, Release: r})
		}
	}
	return doc
}

// encodeLines writes one record per line, so a parser change reads as
// a small diff.
func encodeLines(doc parityDoc) []byte {
	var b []byte
	line := func(v any) []byte { out, _ := json.Marshal(v); return out }
	b = append(b, `{"format":`...)
	b = append(b, line(doc.Format)...)
	b = append(b, ",\n\"corpus\":[\n"...)
	for i, c := range doc.Corpus {
		b = append(b, line(c)...)
		if i < len(doc.Corpus)-1 {
			b = append(b, ',')
		}
		b = append(b, '\n')
	}
	b = append(b, "],\n\"identify\":[\n"...)
	for i, c := range doc.Identify {
		b = append(b, line(c)...)
		if i < len(doc.Identify)-1 {
			b = append(b, ',')
		}
		b = append(b, '\n')
	}
	b = append(b, "],\n\"release\":[\n"...)
	for i, c := range doc.Release {
		b = append(b, line(c)...)
		if i < len(doc.Release)-1 {
			b = append(b, ',')
		}
		b = append(b, '\n')
	}
	return append(b, "]}\n"...)
}

func TestParseParityFixture(t *testing.T) {
	raw, err := os.ReadFile(parityFile)
	if err != nil {
		t.Fatal(err)
	}
	var doc parityDoc
	if err := json.Unmarshal(raw, &doc); err != nil {
		t.Fatal(err)
	}
	fresh := build(doc.Corpus)
	if os.Getenv("LAIN_WRITE_PARITY") == "1" {
		if err := os.WriteFile(parityFile, encodeLines(fresh), 0o644); err != nil {
			t.Fatal(err)
		}
		return
	}
	// Compare through JSON so nil and empty slices read the same.
	norm := func(v any) any {
		b, _ := json.Marshal(v)
		var out any
		_ = json.Unmarshal(b, &out)
		return out
	}
	if !reflect.DeepEqual(norm(fresh.Identify), norm(doc.Identify)) {
		t.Fatal("identify answers differ from the parity fixture (LAIN_WRITE_PARITY=1 rewrites it)")
	}
	if !reflect.DeepEqual(norm(fresh.Release), norm(doc.Release)) {
		t.Fatal("release answers differ from the parity fixture (LAIN_WRITE_PARITY=1 rewrites it)")
	}
}
