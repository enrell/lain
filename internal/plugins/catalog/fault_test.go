package catalog

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-23

// Fault-injection tests: every public surface must degrade to an error
// or an empty result on a closed database — never panic, never hang.
// A bolt handle dies with the process anyway in production, but these
// paths also cover "the store went away" failures the operator sees as
// typed errors, not crashes.

import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/kv"
)

func closedCatalog(t *testing.T) *Service {
	t.Helper()
	db, err := kv.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	s, err := New(db)
	if err != nil {
		t.Fatal(err)
	}
	db.Close()
	return s
}

func TestClosedDBReadsDegrade(t *testing.T) {
	s := closedCatalog(t)
	if got := s.List(); len(got) != 0 {
		t.Fatalf("List on closed db = %v", got)
	}
	if _, ok := s.Get("nope"); ok {
		t.Fatal("Get on closed db returned hit")
	}
	if got := s.ListByLibrary("l"); len(got) != 0 {
		t.Fatalf("ListByLibrary on closed db = %v", got)
	}
	if got := s.Search("q", "anime"); len(got) != 0 {
		t.Fatalf("Search on closed db = %v", got)
	}
	if _, ok := s.Episodes("nope"); ok {
		t.Fatal("Episodes on closed db returned hit")
	}
	if got := s.Count(); got != 0 {
		t.Fatalf("Count on closed db = %d", got)
	}
	// A read may legitimately degrade to an empty page or an error —
	// what it must never do is panic or fabricate rows.
	if page, err := s.Page(contracts.PageParams{Limit: 10}); err == nil && len(page.Items) != 0 {
		t.Fatalf("Page on closed db returned %d items", len(page.Items))
	}
}

func TestClosedDBWritesPropagate(t *testing.T) {
	s := closedCatalog(t)
	in := UpsertInput{
		LibraryID: "l",
		Proposal:  contracts.Proposal{Kind: "anime", Title: "t", Confidence: 0.9},
		Candidate: contracts.Candidate{Path: "/f.mkv"},
	}
	if _, err := s.Upsert(in); err == nil {
		t.Fatal("Upsert on closed db returned nil error")
	}
	it := contracts.CatalogItem{ID: "x", LibraryID: "l", Title: "t", FilePath: "/f.mkv"}
	if err := s.UpsertBatch([]contracts.CatalogItem{it}); err == nil {
		t.Fatal("UpsertBatch on closed db returned nil error")
	}
	if _, _, err := s.MarkMissing("l", map[string]bool{}); err == nil {
		t.Fatal("MarkMissing on closed db returned nil error")
	}
	if _, err := s.DeleteLibrary("l"); err == nil {
		t.Fatal("DeleteLibrary on closed db returned nil error")
	}
	if err := s.Import(ExportDoc{}); err == nil {
		t.Fatal("Import on closed db returned nil error")
	}
}

// ReconcileMoves and Export run read-only passes; on a closed db they
// must still return a usable (empty) result instead of crashing.
func TestClosedDBReconcileAndExport(t *testing.T) {
	s := closedCatalog(t)
	items := []contracts.CatalogItem{{ID: "x", LibraryID: "l", Title: "t", FilePath: "/f.mkv"}}
	got, n := s.ReconcileMoves("l", items, map[string]bool{"x": true})
	if n != 0 || len(got) != 1 {
		t.Fatalf("ReconcileMoves on closed db = %d migrated, %d items", n, len(got))
	}
	doc := s.Export()
	if len(doc.Items) != 0 {
		t.Fatalf("Export on closed db = %d items", len(doc.Items))
	}
}
