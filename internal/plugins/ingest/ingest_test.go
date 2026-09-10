package ingest

import (
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/kv"
	"github.com/enrell/lain/internal/plugins/catalog"
	"github.com/enrell/lain/internal/plugins/identify"
	"github.com/enrell/lain/internal/plugins/source"
)

func TestScanEndToEnd(t *testing.T) {
	root := t.TempDir()
	files := []string{
		"[Fansub-A] Frieren - 12 [1080p][HEVC x265 10bit].mkv",
		"[Fansub-B] Solo Leveling S02E05 (1080p).mkv",
		"random-movie.mp4",
		"notes.txt", // ignored: unknown extension
	}
	for _, f := range files {
		if err := os.WriteFile(filepath.Join(root, f), []byte("fake"), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	db, err := kv.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	cat, err := catalog.New(db)
	if err != nil {
		t.Fatal(err)
	}
	reg := core.NewRegistry(core.DefaultComposition())
	reg.Register(source.Provider{})
	reg.Register(identify.Anime{})
	reg.Register(identify.Generic{})
	reg.Register(cat)
	r := &Runner{Reg: reg, Cat: cat}
	stats, err := r.Run(ScanInput{Libraries: []contracts.Library{{ID: "lib-anime", Name: "Anime", Type: "anime", Path: root}}})
	if err != nil {
		t.Fatal(err)
	}
	if stats.Candidates != 3 || stats.Identified != 3 {
		t.Fatalf("stats %+v, want 3 candidates identified", stats)
	}
	items := cat.List()
	if len(items) != 3 {
		t.Fatalf("catalog has %d items, want 3", len(items))
	}
	byTitle := map[string]contracts.CatalogItem{}
	for _, it := range items {
		byTitle[it.Title] = it
	}
	frieren, ok := byTitle["Frieren"]
	if !ok {
		t.Fatalf("frieren missing: %+v", byTitle)
	}
	if frieren.Episode != 12 || frieren.Origin != identify.AnimeID {
		t.Fatalf("frieren wrong: %+v", frieren)
	}
	if _, ok := byTitle["random movie"]; !ok {
		t.Fatalf("generic fallback missing: %+v", byTitle)
	}
}

func TestScanKeepsServingWhenAnimeWithdrawn(t *testing.T) {
	db, err := kv.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	cat, err := catalog.New(db)
	if err != nil {
		t.Fatal(err)
	}
	reg := core.NewRegistry(core.DefaultComposition())
	reg.Register(source.Provider{})
	reg.Register(identify.Anime{})
	reg.Register(identify.Generic{})
	reg.Register(cat)
	if err := reg.Withdraw(identify.AnimeID); err != nil {
		t.Fatal(err)
	}
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "[Fansub-A] Frieren - 12 [1080p].mkv"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	r := &Runner{Reg: reg, Cat: cat}
	stats, err := r.Run(ScanInput{Libraries: []contracts.Library{{ID: "l", Type: "anime", Path: root}}})
	if err != nil {
		t.Fatal(err)
	}
	if stats.Identified != 1 {
		t.Fatalf("generic did not cover withdrawn anime: %+v", stats)
	}
}

func testRunner(t *testing.T) (*Runner, *catalog.Service) {
	t.Helper()
	db, err := kv.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	cat, err := catalog.New(db)
	if err != nil {
		t.Fatal(err)
	}
	reg := core.NewRegistry(core.DefaultComposition())
	reg.Register(source.Provider{})
	reg.Register(identify.Anime{})
	reg.Register(identify.Generic{})
	reg.Register(cat)
	return &Runner{Reg: reg, Cat: cat}, cat
}

func writeFile(t *testing.T, path string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte("fake-media"), 0o644); err != nil {
		t.Fatal(err)
	}
}

// Deleted files are pruned; the scan reports the count.
func TestRescanPrunesDeletedFiles(t *testing.T) {
	root := t.TempDir()
	writeFile(t, filepath.Join(root, "[Fansub-A] Frieren - 12 [1080p].mkv"))
	writeFile(t, filepath.Join(root, "gone.mp4"))
	r, cat := testRunner(t)
	libs := []contracts.Library{{ID: "l", Type: "anime", Path: root}}
	if _, err := r.Run(ScanInput{Libraries: libs}); err != nil {
		t.Fatal(err)
	}
	if got := len(cat.List()); got != 2 {
		t.Fatalf("list=%d, want 2", got)
	}
	if err := os.Remove(filepath.Join(root, "gone.mp4")); err != nil {
		t.Fatal(err)
	}
	stats, err := r.Run(ScanInput{Libraries: libs})
	if err != nil {
		t.Fatal(err)
	}
	if stats.Pruned != 1 {
		t.Fatalf("pruned=%d, want 1", stats.Pruned)
	}
	if got := len(cat.List()); got != 1 {
		t.Fatalf("list=%d, want 1 after prune", got)
	}
}

// An unreadable root (unmounted drive, deleted path) never reads as
// deletions: the catalog stays exactly as it was.
func TestInaccessibleRootSkipsPrune(t *testing.T) {
	root := t.TempDir()
	writeFile(t, filepath.Join(root, "[Fansub-A] Frieren - 12 [1080p].mkv"))
	r, cat := testRunner(t)
	libs := []contracts.Library{{ID: "l", Type: "anime", Path: root}}
	if _, err := r.Run(ScanInput{Libraries: libs}); err != nil {
		t.Fatal(err)
	}
	before := len(cat.List())
	if err := os.Chmod(root, 0o000); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.Chmod(root, 0o755) })
	stats, err := r.Run(ScanInput{Libraries: libs})
	if err != nil {
		t.Fatal(err)
	}
	if stats.Pruned != 0 {
		t.Fatalf("pruned=%d on dead root, want 0", stats.Pruned)
	}
	if got := len(cat.List()); got != before {
		t.Fatalf("catalog changed on dead root: %d -> %d", before, got)
	}
}

// A mid-walk I/O error blocks pruning for that library only: a partial
// walk must not delete what it failed to see. Other roots still prune.
func TestWalkErrorBlocksPruneForThatLibrary(t *testing.T) {
	good := t.TempDir()
	bad := t.TempDir()
	writeFile(t, filepath.Join(good, "keep.mkv"))
	writeFile(t, filepath.Join(good, "drop.mkv"))
	writeFile(t, filepath.Join(bad, "b1.mkv"))
	writeFile(t, filepath.Join(bad, "sub", "b2.mkv"))
	r, cat := testRunner(t)
	libs := []contracts.Library{
		{ID: "good", Type: "anime", Path: good},
		{ID: "bad", Type: "anime", Path: bad},
	}
	if _, err := r.Run(ScanInput{Libraries: libs}); err != nil {
		t.Fatal(err)
	}
	if err := os.Remove(filepath.Join(good, "drop.mkv")); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(filepath.Join(bad, "sub"), 0o000); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.Chmod(filepath.Join(bad, "sub"), 0o755) })
	stats, err := r.Run(ScanInput{Libraries: libs})
	if err != nil {
		t.Fatal(err)
	}
	if stats.WalkErrors == 0 {
		t.Fatal("expected counted walk errors")
	}
	// good lib pruned normally...
	if stats.Pruned != 1 {
		t.Fatalf("pruned=%d, want 1 (good lib only)", stats.Pruned)
	}
	// ...bad lib kept everything it ever saw.
	n := 0
	for _, it := range cat.List() {
		if it.LibraryID == "bad" {
			n++
		}
	}
	if n != 2 {
		t.Fatalf("bad lib items=%d, want 2 (no prune on dirty walk)", n)
	}
}

// Rescans are idempotent: stable ids, no duplicates, no phantom prune.
func TestRescanIsIdempotent(t *testing.T) {
	root := t.TempDir()
	writeFile(t, filepath.Join(root, "Show.S01E01.1080p.mkv"))
	writeFile(t, filepath.Join(root, "Show.S01E02.1080p.mkv"))
	r, cat := testRunner(t)
	libs := []contracts.Library{{ID: "l", Type: "show", Path: root}}
	first, err := r.Run(ScanInput{Libraries: libs})
	if err != nil {
		t.Fatal(err)
	}
	second, err := r.Run(ScanInput{Libraries: libs})
	if err != nil {
		t.Fatal(err)
	}
	if first.Identified != 2 || second.Identified != 2 || second.Pruned != 0 {
		t.Fatalf("not idempotent: %+v -> %+v", first, second)
	}
	if got := len(cat.List()); got != 2 {
		t.Fatalf("list=%d, want 2", got)
	}
}

// 5000 files must scan fast: batch persist keeps disk writes constant,
// so this guards an O(n^2) regression, not the clock.
func TestLargeLibraryStaysFast(t *testing.T) {
	if testing.Short() {
		t.Skip("large scan skipped in short mode")
	}
	root := t.TempDir()
	for d := 0; d < 50; d++ {
		dir := filepath.Join(root, "dir")
		writeFile(t, filepath.Join(dir, "show-s1e"+itoaFile(d)+".mkv"))
		for f := 0; f < 99; f++ {
			writeFile(t, filepath.Join(dir, "ep-"+itoaFile(d*100+f)+".mp4"))
		}
	}
	r, cat := testRunner(t)
	start := time.Now().UnixNano()
	stats, err := r.Run(ScanInput{Libraries: []contracts.Library{{ID: "l", Type: "show", Path: root}}})
	if err != nil {
		t.Fatal(err)
	}
	if took := time.Now().UnixNano() - start; took > 30_000_000_000 {
		t.Fatalf("5000-file scan took %dns, want < 30s", took)
	}
	if stats.Candidates != 5000 || stats.Identified != 5000 {
		t.Fatalf("stats %+v, want 5000/5000", stats)
	}
	if got := len(cat.List()); got != 5000 {
		t.Fatalf("list=%d, want 5000", got)
	}
}

func itoaFile(i int) string {
	s := ""
	if i == 0 {
		return "0000"
	}
	for i > 0 {
		s = string(rune('0'+i%10)) + s
		i /= 10
	}
	for len(s) < 4 {
		s = "0" + s
	}
	return s
}

func TestTraceCollectsPhases(t *testing.T) {
	root := t.TempDir()
	writeFile(t, filepath.Join(root, "[Fansub-A] Show - 01 [1080p].mkv"))
	writeFile(t, filepath.Join(root, "plain.mp4"))
	r, _ := testRunner(t)
	r.Trace = &ScanTrace{}
	stats, err := r.Run(ScanInput{Libraries: []contracts.Library{{ID: "l", Type: "anime", Path: root}}})
	if err != nil {
		t.Fatal(err)
	}
	if stats.Candidates != 2 {
		t.Fatalf("candidates=%d, want 2", stats.Candidates)
	}
	tr := r.Trace.Snapshot()
	if tr.Enumerate <= 0 {
		t.Error("enumerate phase untimed")
	}
	if tr.Identify <= 0 {
		t.Error("identify phase untimed")
	}
	if tr.Persist <= 0 {
		t.Error("persist phase untimed")
	}
	// Prune phase is timed even when there is nothing to prune.
}

func TestNilTraceCostsNothing(t *testing.T) {
	root := t.TempDir()
	writeFile(t, filepath.Join(root, "a.mkv"))
	r, _ := testRunner(t)
	if r.Trace != nil {
		t.Fatal("production runner must default to nil trace")
	}
	if _, err := r.Run(ScanInput{Libraries: []contracts.Library{{ID: "l", Type: "anime", Path: root}}}); err != nil {
		t.Fatal(err)
	}
}
