package catalog

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-22

import (
	"strings"
	"testing"

	"github.com/enrell/lain/internal/contracts"
)

func TestInvokeAllBranches(t *testing.T) {
	s := testService(t)
	it := mkItem("l", "/x/a.mkv", "Show")

	// Write path: good input, then bad input.
	out, err := s.Invoke(contracts.CapCatalogWrite, UpsertInput{
		LibraryID: "l",
		Proposal:  contracts.Proposal{Kind: "episode", Title: "Show", Episode: 1, PluginID: "p"},
		Candidate: contracts.Candidate{Path: "/x/a.mkv", Size: 10, LibraryID: "l"},
	})
	if err != nil || out.(contracts.CatalogItem).ID != it.ID {
		t.Fatalf("write invoke: %v %+v", err, out)
	}
	if _, err := s.Invoke(contracts.CapCatalogWrite, "nope"); err == nil {
		t.Fatal("bad write input must fail")
	}

	// Read path: nil lists, GetInput fetches, unknown id and wrong
	// types fail.
	list, err := s.Invoke(contracts.CapCatalogRead, nil)
	if err != nil || len(list.([]contracts.CatalogItem)) != 1 {
		t.Fatalf("list invoke: %v", err)
	}
	got, err := s.Invoke(contracts.CapCatalogRead, GetInput{ID: it.ID})
	if err != nil || got.(contracts.CatalogItem).FilePath != "/x/a.mkv" {
		t.Fatalf("get invoke: %v", err)
	}
	if _, err := s.Invoke(contracts.CapCatalogRead, GetInput{ID: "unknown"}); err == nil {
		t.Fatal("unknown id must fail")
	}
	if _, err := s.Invoke(contracts.CapCatalogRead, 42); err == nil {
		t.Fatal("wrong read input type must fail")
	}
	if _, err := s.Invoke("lain.other@1", nil); err == nil {
		t.Fatal("unsupported cap must fail")
	}
}

func TestFingerprintDistinguishesEveryField(t *testing.T) {
	base := Fingerprint("l", "episode", "My  Show", 1, 2, 2020)
	if Fingerprint("l", "episode", "my show", 1, 2, 2020) != base {
		t.Fatal("title case/whitespace must normalize")
	}
	for _, fp := range []string{
		Fingerprint("x", "episode", "My  Show", 1, 2, 2020),
		Fingerprint("l", "movie", "My  Show", 1, 2, 2020),
		Fingerprint("l", "episode", "My  Show", 2, 2, 2020),
		Fingerprint("l", "episode", "My  Show", 1, 3, 2020),
		Fingerprint("l", "episode", "My  Show", 1, 2, 2021),
		Fingerprint("l", "episode", "Other", 1, 2, 2020),
	} {
		if fp == base {
			t.Fatal("fingerprint must differ per field")
		}
	}
}

// reconcileItems is a focused helper: it upserts the stored items then
// reconciles one scanned item, returning the rewritten slice.
func reconcileItems(t *testing.T, s *Service, stored []contracts.CatalogItem, scanned contracts.CatalogItem) ([]contracts.CatalogItem, int) {
	t.Helper()
	if err := s.UpsertBatch(stored); err != nil {
		t.Fatal(err)
	}
	return s.ReconcileMoves("l", []contracts.CatalogItem{scanned}, map[string]bool{scanned.ID: true})
}

func TestReconcileYearMatching(t *testing.T) {
	s := testService(t)
	old := mkItem("l", "/x/old.mkv", "Show")
	old.Year = 2020
	scanned := mkItem("l", "/x/new.mkv", "Show")
	scanned.Year = 0 // unknown year still matches (identifiers fill it later)
	items, n := reconcileItems(t, s, []contracts.CatalogItem{old}, scanned)
	if n != 1 || items[0].ID != old.ID {
		t.Fatalf("year-0 scanned must adopt old id: n=%d %+v", n, items[0])
	}

	s2 := testService(t)
	old.Year = 0
	scanned.Year = 2020
	items, n = reconcileItems(t, s2, []contracts.CatalogItem{old}, scanned)
	if n != 1 || items[0].ID != old.ID {
		t.Fatalf("year-0 stored must adopt too: n=%d", n)
	}

	s3 := testService(t)
	old.Year = 2020
	scanned.Year = 2021
	if _, n = reconcileItems(t, s3, []contracts.CatalogItem{old}, scanned); n != 0 {
		t.Fatal("different nonzero years must not merge")
	}
}

func TestReconcilePicksOldestAndMergesAliases(t *testing.T) {
	s := testService(t)
	newest := mkItem("l", "/x/n.mkv", "Show")
	newest.UpdatedAt = 200
	newest.Aliases = []string{"/x/history-1.mkv"}
	oldest := mkItem("l", "/x/o.mkv", "Show")
	oldest.UpdatedAt = 100
	oldest.Aliases = []string{"/x/history-2.mkv"}
	scanned := mkItem("l", "/x/new.mkv", "Show")
	// Pre-seed 8 aliases: one dup of the adopted path must not repeat,
	// and the merged history pushes the list past the cap so the trim
	// fires after the merge.
	scanned.Aliases = []string{"/x/a1", "/x/a2", "/x/a3", "/x/a4", "/x/a5", "/x/a6", "/x/a7", "/x/o.mkv"}
	items, n := reconcileItems(t, s, []contracts.CatalogItem{newest, oldest}, scanned)
	if n != 1 || items[0].ID != oldest.ID {
		t.Fatalf("oldest UpdatedAt must win: id=%s want %s", items[0].ID, oldest.ID)
	}
	if len(items[0].Aliases) != 8 {
		t.Fatalf("aliases must cap at 8, got %d", len(items[0].Aliases))
	}
	joined := strings.Join(items[0].Aliases, "|")
	if strings.Count(joined, "/x/o.mkv") != 1 || !strings.Contains(joined, "/x/history-2.mkv") {
		t.Fatalf("aliases must dedupe and merge adopted history: %v", items[0].Aliases)
	}
}

func TestQueryFilterCombinations(t *testing.T) {
	s := testService(t)
	mk := func(lib, path, title, kind string) contracts.CatalogItem {
		it := mkItem(lib, path, title)
		it.Kind = kind
		return it
	}
	if err := s.UpsertBatch([]contracts.CatalogItem{
		mk("l1", "/a/one.mkv", "Alpha Show", "episode"),
		mk("l1", "/a/two.mkv", "Beta", "movie"),
		mk("l2", "/b/three.mkv", "Alpha Two", "episode"),
	}); err != nil {
		t.Fatal(err)
	}
	got, total := s.Query("", "", contracts.PageParams{Limit: -1, Sort: "title", LibraryID: "l1"})
	if total != 2 || len(got) != 2 {
		t.Fatalf("library filter: total=%d", total)
	}
	got, total = s.Query("alpha", "EPISODE", contracts.PageParams{Limit: -1, Sort: "title"})
	if total != 2 { // kind filter is case-insensitive
		t.Fatalf("q+kind: total=%d", total)
	}
	if _, total = s.Query("zzz", "", contracts.PageParams{Limit: -1}); total != 0 {
		t.Fatalf("no-match query: total=%d", total)
	}
	got, total = s.Query("", "", contracts.PageParams{Limit: 0, Offset: 0, Sort: "title"})
	if total != 3 || len(got) != 0 {
		t.Fatalf("limit 0: total=%d len=%d", total, len(got))
	}
	// Search is Query unbounded.
	if res := s.Search("alpha", ""); len(res) != 2 {
		t.Fatalf("search: %d", len(res))
	}
}

func TestSortTieBreakers(t *testing.T) {
	s := testService(t)
	mk := func(path, title string, season, ep, year int, updated int64) contracts.CatalogItem {
		it := mkItem("l", path, title)
		it.Season, it.Episode, it.Year, it.UpdatedAt = season, ep, year, updated
		return it
	}
	if err := s.UpsertBatch([]contracts.CatalogItem{
		mk("/x/s1e2.mkv", "Show", 1, 2, 0, 10),
		mk("/x/s1e1b.mkv", "Show", 1, 1, 2021, 10),
		mk("/x/s1e1a.mkv", "Show", 1, 1, 2020, 10),
		mk("/x/s2.mkv", "Show", 2, 1, 0, 10),
		mk("/x/other.mkv", "Another", 9, 9, 0, 10),
	}); err != nil {
		t.Fatal(err)
	}
	got, _ := s.Query("", "", contracts.PageParams{Limit: -1, Sort: "title"})
	want := []string{"/x/other.mkv", "/x/s1e1a.mkv", "/x/s1e1b.mkv", "/x/s1e2.mkv", "/x/s2.mkv"}
	for i, w := range want {
		if got[i].FilePath != w {
			t.Fatalf("title order[%d]=%s want %s", i, got[i].FilePath, w)
		}
	}
	// Recent sort: newer first, ID breaks the tie deterministically.
	got, _ = s.Query("", "", contracts.PageParams{Limit: -1, Sort: "recent"})
	for i := 1; i < len(got); i++ {
		if got[i].UpdatedAt > got[i-1].UpdatedAt {
			t.Fatal("recent sort must be descending")
		}
	}
	// Equal timestamps must fall back to ID order.
	for i := 1; i < len(got); i++ {
		if got[i].UpdatedAt == got[i-1].UpdatedAt && got[i].ID < got[i-1].ID {
			t.Fatal("equal UpdatedAt must tie-break on ID")
		}
	}
	_ = s.List() // unbounded convenience path
}

func TestEpisodesEveryOrderBranch(t *testing.T) {
	s := testService(t)
	mk := func(path string, season, ep, year int) contracts.CatalogItem {
		it := mkItem("l", path, "My Show")
		it.Season, it.Episode, it.Year = season, ep, year
		return it
	}
	items := []contracts.CatalogItem{
		mk("/x/s1e2.mkv", 1, 2, 0),
		mk("/x/s1e1y.mkv", 1, 1, 2021),
		mk("/x/s1e1x.mkv", 1, 1, 2020),
		mk("/x/s2e1.mkv", 2, 1, 0),
	}
	if err := s.UpsertBatch(items); err != nil {
		t.Fatal(err)
	}
	if _, ok := s.Episodes("unknown"); ok {
		t.Fatal("unknown id must report ok=false")
	}
	eps, ok := s.Episodes(items[0].ID)
	if !ok || len(eps) != 4 {
		t.Fatalf("episodes: ok=%v len=%d", ok, len(eps))
	}
	want := []string{"/x/s1e1x.mkv", "/x/s1e1y.mkv", "/x/s1e2.mkv", "/x/s2e1.mkv"}
	for i, w := range want {
		if eps[i].FilePath != w {
			t.Fatalf("watch order[%d]=%s want %s", i, eps[i].FilePath, w)
		}
	}
	// A second library file with the same title joins the same page.
	other := mkItem("l2", "/y/s1e1.mkv", "my  show")
	other.Season, other.Episode = 0, 0
	if err := s.UpsertBatch([]contracts.CatalogItem{other}); err != nil {
		t.Fatal(err)
	}
	eps, _ = s.Episodes(items[0].ID)
	if len(eps) != 5 || eps[0].FilePath != "/y/s1e1.mkv" {
		t.Fatalf("cross-library title must group first (season 0): %v", eps)
	}
}

func TestListByLibraryOrderAndCount(t *testing.T) {
	s := testService(t)
	mk := func(lib, path, title string, ep int) contracts.CatalogItem {
		it := mkItem(lib, path, title)
		it.Episode = ep
		return it
	}
	if err := s.UpsertBatch([]contracts.CatalogItem{
		mk("l", "/x/b2.mkv", "B", 2),
		mk("l", "/x/b1.mkv", "B", 1),
		mk("l", "/x/a.mkv", "A", 1),
		mk("l2", "/x/c.mkv", "C", 1),
	}); err != nil {
		t.Fatal(err)
	}
	got := s.ListByLibrary("l")
	if len(got) != 3 || got[0].Title != "A" || got[1].Episode != 1 || got[2].Episode != 2 {
		t.Fatalf("library listing: %v", got)
	}
	if s.Count() != 4 {
		t.Fatalf("count=%d", s.Count())
	}
	if got := s.ListByLibrary("none"); len(got) != 0 {
		t.Fatalf("unknown library must be empty, got %d", len(got))
	}
}

func TestImportExportRound(t *testing.T) {
	s := testService(t)
	if err := s.UpsertBatch([]contracts.CatalogItem{mkItem("l", "/x/a.mkv", "A"), mkItem("l", "/x/b.mkv", "B")}); err != nil {
		t.Fatal(err)
	}
	doc := s.Export()
	if doc.Format != "lain.catalog-export" || doc.Version != 2 || len(doc.Items) != 2 {
		t.Fatalf("export doc: %+v", doc)
	}

	s2 := testService(t)
	// Import replaces: seed a stale item that must disappear.
	if err := s2.UpsertBatch([]contracts.CatalogItem{mkItem("l", "/x/stale.mkv", "Stale")}); err != nil {
		t.Fatal(err)
	}
	if err := s2.Import(doc); err != nil {
		t.Fatal(err)
	}
	if s2.Count() != 2 || len(s2.ListByLibrary("l")) != 2 {
		t.Fatalf("import must replace contents: count=%d", s2.Count())
	}
	for _, bad := range []ExportDoc{
		{Format: "other", Version: 2},
		{Format: "lain.catalog-export", Version: 3},
		{Format: "lain.catalog-export", Version: 0},
	} {
		if err := s2.Import(bad); err == nil {
			t.Fatalf("bad doc %+v must fail", bad)
		}
	}
	// nil Items imports as an empty catalog.
	if err := s2.Import(ExportDoc{Format: "lain.catalog-export", Version: 1}); err != nil {
		t.Fatal(err)
	}
	if s2.Count() != 0 {
		t.Fatalf("empty import must clear, count=%d", s2.Count())
	}
}

// crafted items let tests control the ID order (bucket order) so
// comparator boundary mutations cannot hide behind input ordering.
func craftItem(id, lib, title string, season, ep, year int, updated int64) contracts.CatalogItem {
	return contracts.CatalogItem{
		ID: id, LibraryID: lib, Kind: "episode", Title: title,
		Season: season, Episode: ep, Year: year, FilePath: "/x/" + id + ".mkv",
		UpdatedAt: updated, Confidence: 0.5,
	}
}

func TestTitleOrderEveryBranch(t *testing.T) {
	s := testService(t)
	// Bucket (ID) order deliberately opposes the correct title order:
	// zz-sorted-first items must still sort last by field.
	if err := s.UpsertBatch([]contracts.CatalogItem{
		craftItem("zz", "l", "Show", 2, 1, 0, 10),    // season 2 last
		craftItem("yz", "l", "Show", 1, 2, 0, 10),    // ep 2
		craftItem("yy", "l", "Show", 1, 1, 2021, 10), // year 2021
		craftItem("yx", "l", "Show", 1, 1, 2020, 10), // year 2020
		craftItem("ab", "l", "Alpha", 9, 9, 0, 10),   // earlier title
	}); err != nil {
		t.Fatal(err)
	}
	got, _ := s.Query("", "", contracts.PageParams{Limit: -1, Sort: "title"})
	want := []string{"ab", "yx", "yy", "yz", "zz"}
	for i, w := range want {
		if got[i].ID != w {
			t.Fatalf("title order[%d]=%s want %s", i, got[i].ID, w)
		}
	}
	// Recent: inverted IDs vs UpdatedAt.
	s2 := testService(t)
	if err := s2.UpsertBatch([]contracts.CatalogItem{
		craftItem("aa", "l", "A", 0, 0, 0, 1), // oldest, lowest ID
		craftItem("zz", "l", "B", 0, 0, 0, 9), // newest, highest ID
		craftItem("mm", "l", "C", 0, 0, 0, 5),
	}); err != nil {
		t.Fatal(err)
	}
	got, _ = s2.Query("", "", contracts.PageParams{Limit: -1, Sort: "recent"})
	if got[0].ID != "zz" || got[1].ID != "mm" || got[2].ID != "aa" {
		t.Fatalf("recent order: %v", got)
	}
}

func TestEpisodesIDTiebreak(t *testing.T) {
	s := testService(t)
	// Same season/episode/year: ID decides, and bucket order opposes.
	// ID order inverts the watch order so comparator boundaries are
	// observable on every field.
	if err := s.UpsertBatch([]contracts.CatalogItem{
		craftItem("aa", "l", "Show", 2, 1, 0, 10), // sorts last
		craftItem("bb", "l", "Show", 1, 2, 0, 10),
		craftItem("cc", "l", "Show", 1, 1, 2021, 10),
		craftItem("dd", "l", "Show", 1, 1, 2020, 10), // sorts first
		craftItem("de", "l", "Show", 1, 1, 2020, 10), // ID tiebreak
	}); err != nil {
		t.Fatal(err)
	}
	eps, ok := s.Episodes("aa")
	if !ok || len(eps) != 5 {
		t.Fatalf("episodes: %v %d", ok, len(eps))
	}
	want := []string{"dd", "de", "cc", "bb", "aa"}
	for i, w := range want {
		if eps[i].ID != w {
			t.Fatalf("watch order[%d]=%s want %s", i, eps[i].ID, w)
		}
	}
}

func TestListByLibraryCraftedOrder(t *testing.T) {
	s := testService(t)
	if err := s.UpsertBatch([]contracts.CatalogItem{
		craftItem("aa", "l", "Show", 1, 2, 0, 10), // ep 2, lowest ID
		craftItem("bb", "l", "Show", 1, 1, 2021, 10),
		craftItem("cc", "l", "Show", 1, 1, 2020, 10), // sorts first
		craftItem("dd", "l", "Beta", 1, 1, 0, 10),    // earlier title
	}); err != nil {
		t.Fatal(err)
	}
	got := s.ListByLibrary("l")
	want := []string{"dd", "cc", "bb", "aa"}
	for i, w := range want {
		if got[i].ID != w {
			t.Fatalf("library order[%d]=%s want %s", i, got[i].ID, w)
		}
	}
}

func TestReconcileOldestWinsOnTie(t *testing.T) {
	s := testService(t)
	a := craftItem("aa", "l", "Show", 1, 1, 0, 100)
	b := craftItem("bb", "l", "Show", 1, 1, 0, 100) // same UpdatedAt
	scanned := mkItem("l", "/x/new.mkv", "Show")
	scanned.Season, scanned.Episode = 1, 1
	items, n := reconcileItems(t, s, []contracts.CatalogItem{a, b}, scanned)
	if n != 1 {
		t.Fatal("must migrate")
	}
	// Strictly-less comparison keeps the first-seen oldest; <= would
	// pick the later one.
	if items[0].ID != "aa" {
		t.Fatalf("must adopt first-seen oldest id, got %s", items[0].ID)
	}
}

func TestImportClearsStaleIndex(t *testing.T) {
	s := testService(t)
	// Stale item lives in l2; the imported doc reuses its ID in l1.
	stale := craftItem("deadbeef", "l2", "Stale", 0, 0, 0, 1)
	if err := s.UpsertBatch([]contracts.CatalogItem{stale}); err != nil {
		t.Fatal(err)
	}
	fresh := craftItem("deadbeef", "l1", "Fresh", 0, 0, 0, 2)
	err := s.Import(ExportDoc{Format: "lain.catalog-export", Version: 2,
		Items: map[string]contracts.CatalogItem{"deadbeef": fresh}})
	if err != nil {
		t.Fatal(err)
	}
	if got := s.ListByLibrary("l2"); len(got) != 0 {
		t.Fatalf("stale l2 index entry must be gone: %v", got)
	}
	if got := s.ListByLibrary("l1"); len(got) != 1 || got[0].Title != "Fresh" {
		t.Fatalf("imported item must index under l1: %v", got)
	}
}

// Two-item sets force sort.Slice to compare that exact pair — no
// comparator mutation can hide behind a skipped comparison.
func TestTitleOrderPairwise(t *testing.T) {
	cases := []struct {
		name string
		a, b contracts.CatalogItem
	}{
		{"title", craftItem("zz", "l", "Alpha", 0, 0, 0, 0), craftItem("aa", "l", "Beta", 0, 0, 0, 0)},
		{"season", craftItem("zz", "l", "S", 1, 0, 0, 0), craftItem("aa", "l", "S", 2, 0, 0, 0)},
		{"episode", craftItem("zz", "l", "S", 1, 1, 0, 0), craftItem("aa", "l", "S", 1, 2, 0, 0)},
		{"year", craftItem("zz", "l", "S", 1, 1, 2020, 0), craftItem("aa", "l", "S", 1, 1, 2021, 0)},
		{"id", craftItem("aa", "l", "S", 1, 1, 2020, 0), craftItem("zz", "l", "S", 1, 1, 2020, 0)},
	}
	for _, tc := range cases {
		// IDs always oppose the field order, so a guard negation that
		// falls through to the ID tie-break flips the result.
		if !titleOrder(tc.a, tc.b) {
			t.Errorf("%s: titleOrder(a,b) must be true", tc.name)
		}
		if titleOrder(tc.b, tc.a) {
			t.Errorf("%s: titleOrder(b,a) must be false", tc.name)
		}
	}
}

func TestEpisodesPairwise(t *testing.T) {
	one := func(a, b contracts.CatalogItem, field string) {
		s := testService(t)
		if err := s.UpsertBatch([]contracts.CatalogItem{a, b}); err != nil {
			t.Fatal(err)
		}
		eps, ok := s.Episodes(a.ID)
		if !ok || len(eps) != 2 {
			t.Fatalf("episodes: ok=%v len=%d", ok, len(eps))
		}
		if eps[0].ID != a.ID {
			t.Fatalf("%s: order=%s,%s want %s first", field, eps[0].ID, eps[1].ID, a.ID)
		}
	}
	one(craftItem("zz", "l", "Show", 1, 1, 0, 0), craftItem("aa", "l", "Show", 2, 1, 0, 0), "season")
	one(craftItem("zz", "l", "Show", 1, 1, 0, 0), craftItem("aa", "l", "Show", 1, 2, 0, 0), "episode")
	one(craftItem("zz", "l", "Show", 1, 1, 2020, 0), craftItem("aa", "l", "Show", 1, 1, 2021, 0), "year")
	one(craftItem("aa", "l", "Show", 1, 1, 2020, 0), craftItem("zz", "l", "Show", 1, 1, 2020, 0), "id")
}

func TestListByLibraryPairwise(t *testing.T) {
	one := func(a, b contracts.CatalogItem, field string) {
		s := testService(t)
		if err := s.UpsertBatch([]contracts.CatalogItem{a, b}); err != nil {
			t.Fatal(err)
		}
		got := s.ListByLibrary("l")
		if len(got) != 2 || got[0].ID != a.ID {
			t.Fatalf("%s: order=%v want %s first", field, got, a.ID)
		}
	}
	one(craftItem("zz", "l", "Alpha", 0, 0, 0, 0), craftItem("aa", "l", "Beta", 0, 0, 0, 0), "title")
	one(craftItem("zz", "l", "S", 1, 0, 0, 0), craftItem("aa", "l", "S", 2, 0, 0, 0), "season")
	one(craftItem("zz", "l", "S", 1, 1, 0, 0), craftItem("aa", "l", "S", 1, 2, 0, 0), "episode")
	one(craftItem("zz", "l", "S", 1, 1, 2020, 0), craftItem("aa", "l", "S", 1, 1, 2021, 0), "year")
	one(craftItem("aa", "l", "S", 1, 1, 2020, 0), craftItem("zz", "l", "S", 1, 1, 2020, 0), "id")
}

func TestQueryTitleSortPairwise(t *testing.T) {
	one := func(a, b contracts.CatalogItem, field string) {
		s := testService(t)
		if err := s.UpsertBatch([]contracts.CatalogItem{a, b}); err != nil {
			t.Fatal(err)
		}
		got, _ := s.Query("", "", contracts.PageParams{Limit: -1, Sort: "title"})
		if len(got) != 2 || got[0].ID != a.ID {
			t.Fatalf("%s: order=%v want %s first", field, got, a.ID)
		}
	}
	one(craftItem("zz", "l", "Alpha", 0, 0, 0, 0), craftItem("aa", "l", "Beta", 0, 0, 0, 0), "title")
	one(craftItem("zz", "l", "S", 1, 0, 0, 0), craftItem("aa", "l", "S", 2, 0, 0, 0), "season")
	one(craftItem("zz", "l", "S", 1, 1, 0, 0), craftItem("aa", "l", "S", 1, 2, 0, 0), "episode")
	one(craftItem("zz", "l", "S", 1, 1, 2020, 0), craftItem("aa", "l", "S", 1, 1, 2021, 0), "year")
	one(craftItem("aa", "l", "S", 1, 1, 2020, 0), craftItem("zz", "l", "S", 1, 1, 2020, 0), "id")
}

func TestRecentSortPairwise(t *testing.T) {
	s := testService(t)
	if err := s.UpsertBatch([]contracts.CatalogItem{
		craftItem("zz", "l", "A", 0, 0, 0, 10), // newest, highest ID
		craftItem("aa", "l", "B", 0, 0, 0, 10), // same time, lowest ID
	}); err != nil {
		t.Fatal(err)
	}
	got, _ := s.Query("", "", contracts.PageParams{Limit: -1, Sort: "recent"})
	if len(got) != 2 || got[0].ID != "aa" {
		t.Fatalf("equal UpdatedAt must order by ID: %v", got)
	}
}
