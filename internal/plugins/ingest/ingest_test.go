package ingest

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/plugins/catalog"
	"github.com/enrell/lain/internal/plugins/identify"
	"github.com/enrell/lain/internal/plugins/source"
	"github.com/enrell/lain/internal/store"
)

func TestScanEndToEnd(t *testing.T) {
	root := t.TempDir()
	files := []string{
		"[Erai-raws] Frieren - 12 [1080p][HEVC x265 10bit].mkv",
		"[SubsPlease] Solo Leveling S02E05 (1080p).mkv",
		"random-movie.mp4",
		"notes.txt", // ignored: unknown extension
	}
	for _, f := range files {
		if err := os.WriteFile(filepath.Join(root, f), []byte("fake"), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	st, err := store.New(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	cat, err := catalog.New(st)
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
	st, err := store.New(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	cat, err := catalog.New(st)
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
	if err := os.WriteFile(filepath.Join(root, "[Erai-raws] Frieren - 12 [1080p].mkv"), []byte("x"), 0o644); err != nil {
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
