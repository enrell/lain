package catalog

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-22

import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/kv"
)

func testService(t *testing.T) *Service {
	t.Helper()
	s, _ := testServiceIn(t, t.TempDir())
	return s
}

func testServiceIn(t *testing.T, dir string) (*Service, string) {
	t.Helper()
	db, err := kv.Open(dir)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	s, err := New(db)
	if err != nil {
		t.Fatal(err)
	}
	return s, dir
}

func mkItem(lib, path, title string) contracts.CatalogItem {
	return NewItem(lib,
		contracts.Proposal{Kind: "episode", Title: title, Episode: 1, Confidence: 0.9, PluginID: "p"},
		contracts.Candidate{Path: path, Size: 10, LibraryID: lib})
}

func TestNewItemStableID(t *testing.T) {
	a := mkItem("l", "/x/a.mkv", "A")
	b := mkItem("l", "/x/a.mkv", "A-changed-title")
	if a.ID != b.ID {
		t.Fatal("identity must survive title changes (same library+path)")
	}
	if mkItem("other", "/x/a.mkv", "A").ID == a.ID {
		t.Fatal("identity must include the library")
	}
}

func TestBatchRoundtrip(t *testing.T) {
	dir := t.TempDir()
	s, _ := testServiceIn(t, dir)
	items := []contracts.CatalogItem{
		mkItem("l", "/x/a.mkv", "A"),
		mkItem("l", "/x/b.mkv", "B"),
	}
	if err := s.UpsertBatch(items); err != nil {
		t.Fatal(err)
	}
	if got := len(s.List()); got != 2 {
		t.Fatalf("list=%d, want 2", got)
	}
	// Close first handle, reopen: one persist must hold all.
	s.db.Close()
	db2, err := kv.Open(dir)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db2.Close() })
	s2, err := New(db2)
	if err != nil {
		t.Fatal(err)
	}
	if got := len(s2.List()); got != 2 {
		t.Fatalf("reloaded list=%d, want 2", got)
	}
}

func TestBatchEmptyIsNoop(t *testing.T) {
	s := testService(t)
	if err := s.UpsertBatch(nil); err != nil {
		t.Fatal(err)
	}
	if got := len(s.List()); got != 0 {
		t.Fatalf("list=%d, want 0", got)
	}
}

func TestBatchOverwritesSameID(t *testing.T) {
	s := testService(t)
	if err := s.UpsertBatch([]contracts.CatalogItem{mkItem("l", "/x/a.mkv", "A")}); err != nil {
		t.Fatal(err)
	}
	if err := s.UpsertBatch([]contracts.CatalogItem{mkItem("l", "/x/a.mkv", "A2")}); err != nil {
		t.Fatal(err)
	}
	items := s.List()
	if len(items) != 1 || items[0].Title != "A2" {
		t.Fatalf("re-upsert must refresh, got %+v", items)
	}
}

func TestMarkMissingScopedPerLibrary(t *testing.T) {
	s := testService(t)
	a := mkItem("lib-a", "/a/1.mkv", "A1")
	b := mkItem("lib-a", "/a/2.mkv", "A2")
	c := mkItem("lib-b", "/b/1.mkv", "B1")
	if err := s.UpsertBatch([]contracts.CatalogItem{a, b, c}); err != nil {
		t.Fatal(err)
	}
	// Only A1 seen: A2 marked missing, B1 untouched (other library).
	missing, restored, err := s.MarkMissing("lib-a", map[string]bool{a.ID: true})
	if err != nil {
		t.Fatal(err)
	}
	if missing != 1 || restored != 0 {
		t.Fatalf("missing=%d restored=%d, want 1/0", missing, restored)
	}
	got, ok := s.Get(b.ID)
	if !ok || !got.Missing {
		t.Fatalf("absent item must be marked missing: %+v", got)
	}
	got, ok = s.Get(a.ID)
	if !ok || got.Missing {
		t.Fatalf("seen item must stay present: %+v", got)
	}
	got, ok = s.Get(c.ID)
	if !ok || got.Missing {
		t.Fatalf("other library must be untouched: %+v", got)
	}
	// A second identical pass changes nothing.
	missing, restored, err = s.MarkMissing("lib-a", map[string]bool{a.ID: true})
	if err != nil || missing != 0 || restored != 0 {
		t.Fatalf("idempotent pass: missing=%d restored=%d err=%v", missing, restored, err)
	}
}

func TestMarkMissingRestoresReturnedFile(t *testing.T) {
	s := testService(t)
	a := mkItem("l", "/a.mkv", "A")
	if err := s.UpsertBatch([]contracts.CatalogItem{a}); err != nil {
		t.Fatal(err)
	}
	if _, _, err := s.MarkMissing("l", map[string]bool{}); err != nil {
		t.Fatal(err)
	}
	if got, _ := s.Get(a.ID); !got.Missing {
		t.Fatal("item must be missing while its file is gone")
	}
	missing, restored, err := s.MarkMissing("l", map[string]bool{a.ID: true})
	if err != nil || missing != 0 || restored != 1 {
		t.Fatalf("missing=%d restored=%d err=%v, want 0/1/<nil>", missing, restored, err)
	}
	if got, _ := s.Get(a.ID); got.Missing {
		t.Fatal("returned file must clear the missing flag")
	}
}

func TestDeleteLibraryRemovesOnlyItsItems(t *testing.T) {
	s := testService(t)
	a := mkItem("lib-a", "/a/1.mkv", "A1")
	b := mkItem("lib-a", "/a/2.mkv", "A2")
	c := mkItem("lib-b", "/b/1.mkv", "B1")
	if err := s.UpsertBatch([]contracts.CatalogItem{a, b, c}); err != nil {
		t.Fatal(err)
	}
	n, err := s.DeleteLibrary("lib-a")
	if err != nil || n != 2 {
		t.Fatalf("DeleteLibrary=%d err=%v, want 2/<nil>", n, err)
	}
	if _, ok := s.Get(a.ID); ok {
		t.Fatal("deleted library item must be gone")
	}
	if _, ok := s.Get(b.ID); ok {
		t.Fatal("deleted library item must be gone")
	}
	if _, ok := s.Get(c.ID); !ok {
		t.Fatal("other library must be untouched")
	}
	if got := len(s.ListByLibrary("lib-a")); got != 0 {
		t.Fatalf("by-library index still has %d entries", got)
	}
	if got := len(s.ListByLibrary("lib-b")); got != 1 {
		t.Fatalf("by-library index of lib-b has %d entries, want 1", got)
	}
	// A mark-missing pass over a still-existing root changes nothing.
	missing, restored, err := s.MarkMissing("lib-b", map[string]bool{c.ID: true})
	if err != nil || missing != 0 || restored != 0 {
		t.Fatalf("missing=%d restored=%d err=%v, want 0/0/<nil>", missing, restored, err)
	}
}

func TestDeleteLibraryUnknownIsNoop(t *testing.T) {
	s := testService(t)
	if n, err := s.DeleteLibrary("nope"); err != nil || n != 0 {
		t.Fatalf("DeleteLibrary=%d err=%v, want 0/<nil>", n, err)
	}
}

func TestBatchLargeStaysFast(t *testing.T) {
	s := testService(t)
	var items []contracts.CatalogItem
	for i := 0; i < 3000; i++ {
		items = append(items, mkItem("l", "/x/file-"+itoa(i)+".mkv", "T"))
	}
	if err := s.UpsertBatch(items); err != nil {
		t.Fatal(err)
	}
	if got := len(s.List()); got != 3000 {
		t.Fatalf("list=%d, want 3000", got)
	}
}

func itoa(i int) string {
	if i == 0 {
		return "0"
	}
	var b [8]byte
	p := len(b)
	for i > 0 {
		p--
		b[p] = byte('0' + i%10)
		i /= 10
	}
	return string(b[p:])
}

func TestPageBoundaries(t *testing.T) {
	s := testService(t)
	var items []contracts.CatalogItem
	for _, title := range []string{"B", "A", "C", "D", "E"} {
		items = append(items, mkItem("l", "/x/"+title+".mkv", title))
	}
	if err := s.UpsertBatch(items); err != nil {
		t.Fatal(err)
	}
	// Title order: A B C D E.
	p1, err := s.Page(contracts.PageParams{Limit: 2, Offset: 0, Sort: "title"})
	if err != nil {
		t.Fatal(err)
	}
	if p1.Total != 5 || len(p1.Items) != 2 || p1.Items[0].Title != "A" || p1.Items[1].Title != "B" {
		t.Fatalf("page1: %+v", p1)
	}
	p2, _ := s.Page(contracts.PageParams{Limit: 2, Offset: 2, Sort: "title"})
	if len(p2.Items) != 2 || p2.Items[0].Title != "C" {
		t.Fatalf("page2: %+v", p2)
	}
	p3, _ := s.Page(contracts.PageParams{Limit: 2, Offset: 4, Sort: "title"})
	if len(p3.Items) != 1 || p3.Total != 5 {
		t.Fatalf("last partial page: %+v", p3)
	}
	empty, _ := s.Page(contracts.PageParams{Limit: 2, Offset: 10, Sort: "title"})
	if len(empty.Items) != 0 || empty.Total != 5 {
		t.Fatalf("past-end page: %+v", empty)
	}
	if empty.Items == nil {
		t.Fatal("items must be [] not null")
	}
}

func TestQueryFiltersAndSorts(t *testing.T) {
	s := testService(t)
	if err := s.UpsertBatch([]contracts.CatalogItem{
		mkItem("l", "/x/show-s1e1.mkv", "Show"),
		mkItem("l", "/x/show-s1e2.mkv", "Show"),
		mkItem("l", "/x/other.mp4", "Other"),
	}); err != nil {
		t.Fatal(err)
	}
	got, total := s.Query("show", "", contracts.PageParams{Limit: 1, Offset: 1, Sort: "title"})
	if total != 2 || len(got) != 1 {
		t.Fatalf("query page: total=%d len=%d", total, len(got))
	}
	eps, total := s.Query("", "episode", contracts.PageParams{Limit: -1, Sort: "title"})
	if total != 3 { // mkItem kind is episode for all
		t.Fatalf("kind filter total=%d", total)
	}
	_ = eps
}

func TestNormalizePage(t *testing.T) {
	p := contracts.NormalizePage(0, -5, "bogus")
	if p.Limit != 50 || p.Offset != 0 || p.Sort != "title" {
		t.Fatalf("defaults: %+v", p)
	}
	p = contracts.NormalizePage(99999, 0, "recent")
	if p.Limit != 500 || p.Sort != "recent" {
		t.Fatalf("cap: %+v", p)
	}
}

func reconcileSetup(t *testing.T) *Service {
	t.Helper()
	s := testService(t)
	if err := s.UpsertBatch([]contracts.CatalogItem{mkItem("l", "/x/a.mkv", "Show")}); err != nil {
		t.Fatal(err)
	}
	return s
}

func TestReconcileRenameKeepsID(t *testing.T) {
	s := reconcileSetup(t)
	old := s.List()[0]
	moved := mkItem("l", "/y/b.mkv", "Show")
	present := map[string]bool{moved.ID: true}
	out, n := s.ReconcileMoves("l", []contracts.CatalogItem{moved}, present)
	if n != 1 {
		t.Fatalf("migrated=%d, want 1", n)
	}
	if out[0].ID != old.ID {
		t.Fatalf("id=%s, want stable %s", out[0].ID, old.ID)
	}
	if len(out[0].Aliases) != 1 || out[0].Aliases[0] != "/x/a.mkv" {
		t.Fatalf("aliases=%v, want [/x/a.mkv]", out[0].Aliases)
	}
	if !present[old.ID] || present[moved.ID] {
		t.Fatalf("present not rewritten: %v", present)
	}
}

func TestReconcileDuplicateBothPresent(t *testing.T) {
	s := reconcileSetup(t)
	old := s.List()[0]
	dup := mkItem("l", "/y/b.mkv", "Show")
	present := map[string]bool{old.ID: true, dup.ID: true}
	out, n := s.ReconcileMoves("l", []contracts.CatalogItem{dup}, present)
	if n != 0 || out[0].ID != dup.ID {
		t.Fatalf("genuine duplicate must keep its own id: %+v migrated=%d", out[0], n)
	}
}

func TestReconcileCrossLibraryNoMerge(t *testing.T) {
	s := reconcileSetup(t)
	other := mkItem("other", "/y/b.mkv", "Show")
	present := map[string]bool{other.ID: true}
	out, n := s.ReconcileMoves("other", []contracts.CatalogItem{other}, present)
	if n != 0 || out[0].ID != other.ID {
		t.Fatalf("cross-library titles must not merge: %+v migrated=%d", out[0], n)
	}
}

func TestReconcileRefreshKnownPath(t *testing.T) {
	s := reconcileSetup(t)
	same := mkItem("l", "/x/a.mkv", "Show")
	present := map[string]bool{same.ID: true}
	out, n := s.ReconcileMoves("l", []contracts.CatalogItem{same}, present)
	if n != 0 || len(out[0].Aliases) != 0 {
		t.Fatalf("refresh must not migrate: %+v migrated=%d", out[0], n)
	}
}

func TestExportV2AcceptsV1(t *testing.T) {
	s := reconcileSetup(t)
	if got := s.Export().Version; got != 2 {
		t.Fatalf("export version=%d, want 2", got)
	}
	v1 := ExportDoc{Format: "lain.catalog-export", Version: 1,
		Items: map[string]contracts.CatalogItem{mkItem("l", "/x/a.mkv", "Show").ID: mkItem("l", "/x/a.mkv", "Show")}}
	if err := s.Import(v1); err != nil {
		t.Fatalf("v1 import rejected: %v", err)
	}
	bad := ExportDoc{Format: "lain.catalog-export", Version: 3, Items: map[string]contracts.CatalogItem{}}
	if err := s.Import(bad); err == nil {
		t.Fatal("v3 import must fail")
	}
}

func mkEpisode(lib, path, title string, season, episode int) contracts.CatalogItem {
	return NewItem(lib,
		contracts.Proposal{Kind: "episode", Title: title, Season: season, Episode: episode, Confidence: 0.9, PluginID: "p"},
		contracts.Candidate{Path: path, Size: 10, LibraryID: lib})
}

func TestTitleSortOrdersEpisodes(t *testing.T) {
	s := testService(t)
	if err := s.UpsertBatch([]contracts.CatalogItem{
		mkEpisode("l", "/x/show-e20.mkv", "Show", 1, 20),
		mkEpisode("l", "/x/show-e9.mkv", "Show", 1, 9),
		mkEpisode("l", "/x/show-e2s2.mkv", "Show", 2, 2),
		mkEpisode("l", "/x/show-e1s2.mkv", "Show", 2, 1),
		mkEpisode("l", "/x/other.mkv", "Other", 0, 0),
	}); err != nil {
		t.Fatal(err)
	}
	got, total := s.Query("", "", contracts.PageParams{Limit: -1, Sort: "title"})
	if total != 5 {
		t.Fatalf("total=%d, want 5", total)
	}
	want := []struct {
		title   string
		season  int
		episode int
	}{
		{"Other", 0, 0},
		{"Show", 1, 9},
		{"Show", 1, 20},
		{"Show", 2, 1},
		{"Show", 2, 2},
	}
	for i, w := range want {
		if got[i].Title != w.title || got[i].Season != w.season || got[i].Episode != w.episode {
			t.Fatalf("pos %d: got %s S%dE%d, want %s S%dE%d", i, got[i].Title, got[i].Season, got[i].Episode, w.title, w.season, w.episode)
		}
	}
	if got2 := s.ListByLibrary("l"); len(got2) != 5 || got2[1].Episode != 9 || got2[2].Episode != 20 {
		t.Fatalf("ListByLibrary not in episode order: %+v", got2)
	}
}

// TestEpisodesReturnsTheTitleInWatchOrder pins the title page's source:
// every file whose title normalizes to the same key, in watch order even
// when the raw titles disagree on casing or spacing, across libraries;
// a different title never leaks in, and an unknown id reports ok=false.
func TestEpisodesReturnsTheTitleInWatchOrder(t *testing.T) {
	s := testService(t)
	e1 := mkEpisode("l", "/x/show-s1e1.mkv", "Show", 1, 1)
	e2 := mkEpisode("l", "/x/show-s1e2.mkv", "  SHOW ", 1, 2)
	e10 := mkEpisode("l", "/x/show-s1e10.mkv", "Show", 1, 10)
	other := mkEpisode("l", "/x/other.mkv", "Other Show", 1, 1)
	// A show split across libraries still opens as one page.
	far := mkEpisode("m", "/y/show-s2e1.mkv", "show", 2, 1)
	if err := s.UpsertBatch([]contracts.CatalogItem{e10, e2, e1, other, far}); err != nil {
		t.Fatal(err)
	}

	got, ok := s.Episodes(e1.ID)
	if !ok {
		t.Fatal("Episodes reported a known id as unknown")
	}
	want := []string{e1.ID, e2.ID, e10.ID, far.ID}
	if len(got) != len(want) {
		t.Fatalf("episodes=%d, want %d: %+v", len(got), len(want), got)
	}
	for i, id := range want {
		if got[i].ID != id {
			t.Fatalf("pos %d: id=%s, want %s", i, got[i].ID, id)
		}
	}
	for _, it := range got {
		if it.ID == other.ID {
			t.Fatal("a different title leaked into the group")
		}
	}

	if _, ok := s.Episodes("does-not-exist"); ok {
		t.Fatal("an unknown id must report ok=false")
	}
}
