package search


import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

func item(id, title, kind string, season, episode, year int, updated int64) contracts.CatalogItem {
	return contracts.CatalogItem{
		ID: id, Title: title, Kind: kind,
		Season: season, Episode: episode, Year: year, UpdatedAt: updated,
	}
}

func TestQueryFiltersCaseInsensitiveSubstring(t *testing.T) {
	all := []contracts.CatalogItem{
		item("a", "Frieren", "anime", 1, 1, 2023, 10),
		item("b", "FRIEREN movie", "movie", 0, 0, 2023, 20), // same name, other kind
		item("c", "One Piece", "anime", 1, 1, 1999, 30),
	}
	page := Query(all, QueryInput{Q: "frieren", Limit: 50})
	if page.Total != 2 || len(page.Items) != 2 {
		t.Fatalf("q=frieren: total=%d items=%d, want 2/2", page.Total, len(page.Items))
	}
	page = Query(all, QueryInput{Q: "FRIEREN", Kind: "MOVIE", Limit: 50})
	if page.Total != 1 || page.Items[0].ID != "b" {
		t.Fatalf("kind filter: %+v", page.Items)
	}
	page = Query(all, QueryInput{Q: "frieren", Kind: "anime", Limit: 50})
	if page.Total != 1 || page.Items[0].ID != "a" {
		t.Fatalf("q+kind: %+v", page.Items)
	}
	// Empty q + empty kind passes everything.
	page = Query(all, QueryInput{Limit: 50})
	if page.Total != 3 {
		t.Fatalf("empty query: total=%d, want 3", page.Total)
	}
}

func TestQueryPagesWithExactTotal(t *testing.T) {
	var all []contracts.CatalogItem
	for _, id := range []string{"c", "a", "b"} {
		all = append(all, item(id, id, "anime", 0, 0, 0, 0))
	}
	page := Query(all, QueryInput{Limit: 2, Offset: 1})
	if page.Total != 3 || len(page.Items) != 2 {
		t.Fatalf("page: %+v", page)
	}
	if page.Items[0].ID != "b" || page.Items[1].ID != "c" {
		t.Fatalf("title sort page order: %+v", page.Items)
	}
	// Offset past the end: empty items, total still exact.
	page = Query(all, QueryInput{Limit: 2, Offset: 9})
	if page.Total != 3 || len(page.Items) != 0 {
		t.Fatalf("past-end page: %+v", page)
	}
	// Offset exactly at the end is still an empty page.
	page = Query(all, QueryInput{Limit: 2, Offset: 3})
	if page.Total != 3 || len(page.Items) != 0 {
		t.Fatalf("at-end page: %+v", page)
	}
	if page.Items == nil {
		t.Fatal("empty page must serialize as [], not null")
	}
}

func TestQuerySortOrders(t *testing.T) {
	all := []contracts.CatalogItem{
		item("b", "same", "anime", 1, 2, 0, 10),
		item("a", "same", "anime", 1, 1, 0, 10), // same title, earlier episode
		item("c", "zeta", "anime", 0, 0, 0, 30),
		item("d", "alpha", "anime", 0, 0, 0, 20),
	}
	page := Query(all, QueryInput{Limit: 50})
	got := []string{page.Items[0].ID, page.Items[1].ID, page.Items[2].ID, page.Items[3].ID}
	if got[0] != "d" || got[1] != "a" || got[2] != "b" || got[3] != "c" {
		t.Fatalf("title sort: %v", got)
	}
	page = Query(all, QueryInput{Limit: 50, Sort: "recent"})
	got = []string{page.Items[0].ID, page.Items[1].ID, page.Items[2].ID, page.Items[3].ID}
	// recent: UpdatedAt desc; the tie between a and b breaks on ID asc.
	if got[0] != "c" || got[1] != "d" || got[2] != "a" || got[3] != "b" {
		t.Fatalf("recent sort: %v", got)
	}
}

// The title sort cascades title→season→episode→year→id; every level
// must participate, so each row differs from its predecessor in
// exactly one key.
func TestQuerySortCascade(t *testing.T) {
	all := []contracts.CatalogItem{
		item("x", "same", "anime", 2, 1, 2000, 0), // loses on season
		item("y", "same", "anime", 1, 2, 2000, 0), // loses on episode
		item("z", "same", "anime", 1, 1, 2001, 0), // loses on year
		item("w", "same", "anime", 1, 1, 2000, 0), // loses on id
		item("v", "same", "anime", 1, 1, 2000, 0), // wins on id
	}
	page := Query(all, QueryInput{Limit: 50})
	var got []string
	for _, it := range page.Items {
		got = append(got, it.ID)
	}
	want := []string{"v", "w", "z", "y", "x"}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("cascade order: %v, want %v", got, want)
		}
	}
}

func TestProviderInvokeContract(t *testing.T) {
	p := Provider{Items: func() []contracts.CatalogItem {
		return []contracts.CatalogItem{item("a", "Frieren", "anime", 1, 1, 2023, 1)}
	}}
	if p.ID() != ID {
		t.Fatalf("id=%s", p.ID())
	}
	caps := p.Capabilities()
	if len(caps) != 1 || caps[0] != contracts.CapSearchQuery {
		t.Fatalf("caps=%v", caps)
	}
	if err := p.Health(); err != nil {
		t.Fatalf("health: %v", err)
	}
	if _, err := p.Invoke("lain.other@1", QueryInput{}); err == nil {
		t.Fatal("unsupported cap must fail")
	} else if e, ok := err.(*core.Error); !ok || e.Code != "invalid-message" {
		t.Fatalf("want invalid-message, got %v", err)
	}
	if _, err := p.Invoke(contracts.CapSearchQuery, "wrong type"); err == nil {
		t.Fatal("wrong input type must fail")
	}
	out, err := p.Invoke(contracts.CapSearchQuery, QueryInput{Q: "frieren", Limit: 50})
	if err != nil {
		t.Fatal(err)
	}
	if page := out.(contracts.CatalogPage); page.Total != 1 {
		t.Fatalf("invoke query: %+v", page)
	}
	// A nil Items func yields an empty page, not a panic.
	empty := Provider{}
	out, err = empty.Invoke(contracts.CapSearchQuery, QueryInput{Limit: 50})
	if err != nil {
		t.Fatal(err)
	}
	if page := out.(contracts.CatalogPage); page.Total != 0 {
		t.Fatalf("nil items: %+v", page)
	}
}

// Substring search must be a real substring match — a query longer than
// the title never matches, and the fold stays cheap for non-ASCII.
func TestQuerySubstringBoundaries(t *testing.T) {
	all := []contracts.CatalogItem{item("a", "Go", "movie", 0, 0, 0, 0)}
	if page := Query(all, QueryInput{Q: "golang", Limit: 50}); page.Total != 0 {
		t.Fatalf("longer query must not match: %+v", page)
	}
	if page := Query(all, QueryInput{Q: "o", Limit: 50}); page.Total != 1 {
		t.Fatalf("suffix substring must match: %+v", page)
	}
	// Equal length must match — the scan window includes the last
	// position and a same-length query is not "longer than title".
	if page := Query(all, QueryInput{Q: "go", Limit: 50}); page.Total != 1 {
		t.Fatalf("exact-length query must match: %+v", page)
	}
}

// lower() folds exactly 'A'..'Z': the boundary bytes must fold and the
// bytes just outside ('@', '[') must not.
func TestLowerFoldsOnlyASCIIUpper(t *testing.T) {
	for _, tc := range []struct{ title, q string }{
		{"Apple", "apple"},    // 'A' edge folds
		{"Zebra", "zebra"},    // 'Z' edge folds
		{"A[Z] show", "a[z]"}, // '[' must not fold; 'A'/'Z' must
	} {
		all := []contracts.CatalogItem{item("a", tc.title, "movie", 0, 0, 0, 0)}
		if page := Query(all, QueryInput{Q: tc.q, Limit: 50}); page.Total != 1 {
			t.Fatalf("title %q vs q %q: %+v", tc.title, tc.q, page)
		}
	}
	// A lowercase query never matches a literal '{' (0x7B) just because
	// the title holds '[' (0x5B) — only the exact byte folds.
	all := []contracts.CatalogItem{item("a", "a[z", "movie", 0, 0, 0, 0)}
	if page := Query(all, QueryInput{Q: "a{z", Limit: 50}); page.Total != 0 {
		t.Fatalf("'[' must not fold to '{': %+v", page)
	}
}
