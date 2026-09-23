package bench

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-22

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestSummarizeStats(t *testing.T) {
	s := summarize([]float64{2, 4, 9})
	if s.Mean != 5 || s.Min != 2 || s.Max != 9 {
		t.Fatalf("summarize: %+v, want mean=5 min=2 max=9", s)
	}
	if z := summarize(nil); z != (stats{}) {
		t.Fatalf("empty summarize: %+v", z)
	}
	// Order-independent extremes.
	s = summarize([]float64{9, 2, 4})
	if s.Min != 2 || s.Max != 9 {
		t.Fatalf("extremes: %+v", s)
	}
}

func TestHumanBytesBoundaries(t *testing.T) {
	cases := []struct {
		b    uint64
		want string
	}{
		{0, "0 B"},
		{1023, "1023 B"},
		{1024, "1.0 KiB"},
		{1024*1024 - 1, "1024.0 KiB"}, // still KiB just under 1 MiB
		{1024 * 1024, "1.0 MiB"},
		{1536 * 1024 * 1024, "1.5 GiB"},
		{1<<64 - 1<<40, "17179868160.0 GiB"}, // huge values keep the GiB suffix
	}
	for _, c := range cases {
		if got := humanBytes(c.b); got != c.want {
			t.Errorf("humanBytes(%d) = %q, want %q", c.b, got, c.want)
		}
	}
}

// The RSS reader must return a plausible byte count on Linux — a real
// process always has well over 1 MiB resident.
func TestRSSPlausible(t *testing.T) {
	if got := rss(); got < 1<<20 {
		t.Fatalf("rss()=%d, want > 1 MiB for any real process", got)
	}
}

func TestSamplerMeasuresPeak(t *testing.T) {
	s := Start(5 * time.Millisecond)
	time.Sleep(30 * time.Millisecond)
	peak := s.Stop()
	if peak == 0 {
		t.Skip("RSS unsupported on this platform")
	}
	if peak < s.initial {
		t.Fatalf("peak %d < initial %d", peak, s.initial)
	}
}

// The SVG charts are deterministic geometry: pin the exact numbers so
// a flipped comparison or sign in the layout math cannot slip through.
func TestStackedChartGeometry(t *testing.T) {
	runs := []RunResult{
		{Run: 1, TotalMs: 10000, EnumerateMs: 40, IdentifyMs: 58, PersistMs: 6, PruneMs: 1},
		{Run: 2, TotalMs: 100, EnumerateMs: 35, IdentifyMs: 0, PersistMs: 6, PruneMs: 1},
	}
	svg := string(stackedChart(runs))
	// H = 2*34 + 46 = 114; barW = 760-90-90 = 580; maxTotal = 10000.
	for _, want := range []string{
		`height="114"`,
		`width="2.3"`, // run1 enumerate: 40/10000*580
		`width="0.5"`, // tiny positive phase clamps to the 0.5 minimum
		`width="0.0"`, // a zero phase draws a zero-width rect
		`x="676"`,     // total label column: W-84
		`x="106"`,     // first legend label: 90+16
		`run 2`,
		`x="90" y="90" width="12"`,  // legend rect row: H-24
		`x="106" y="100" font-size`, // legend text row: lx+16, H-14
		`x="200" y="90" width="12"`, // second legend entry: lx += 110
	} {
		if !strings.Contains(svg, want) {
			t.Errorf("stacked chart missing %q", want)
		}
	}
	// The label column and the total column share y arithmetic: each
	// y value must appear exactly twice, so a mutation on either
	// Fprintf site drops the count.
	for _, want := range []string{`y="23"`, `y="57"`} {
		if n := strings.Count(svg, want); n != 2 {
			t.Errorf("%q appears %d times, want 2 (label + total)", want, n)
		}
	}
}

func TestBarsGeometry(t *testing.T) {
	runs := []RunResult{
		{Run: 1, FilesPerSec: 4166},
		{Run: 2, FilesPerSec: 5000},
	}
	svg := string(bars("throughput", runs, func(r RunResult) float64 { return r.FilesPerSec }, "files/s"))
	// barW = 760-90-120 = 550; maxV = 5000.
	for _, want := range []string{
		`height="76"`,   // H = 2*30 + 16
		`width="458.3"`, // 4166/5000*550
		`width="550.0"`, // max run spans the full bar
		`x="650"`,       // value label column: W-110
	} {
		if !strings.Contains(svg, want) {
			t.Errorf("bars chart missing %q", want)
		}
	}
	// Run-label and value-label share y+14 arithmetic: each value must
	// appear exactly twice.
	for _, want := range []string{`y="34"`, `y="64"`} {
		if n := strings.Count(svg, want); n != 2 {
			t.Errorf("%q appears %d times, want 2", want, n)
		}
	}
}

func TestWriteReportSelfContained(t *testing.T) {
	dir := t.TempDir()
	rep := &Report{
		Tool: "lain bench scan", Target: "scan", Path: "/media",
		StartedAt: time.Date(2026, 9, 10, 5, 0, 0, 0, time.UTC),
		GoVersion: "go1.27.1", NumCPU: 8,
		Note: "test",
		Runs: []RunResult{
			{Run: 1, TotalMs: 120, EnumerateMs: 40, IdentifyMs: 70, PersistMs: 8, PruneMs: 2, Candidates: 500, Identified: 500, Dirs: 10, FilesPerSec: 4166, PeakRSS: 20 << 20, HeapAlloc: 8 << 20},
			{Run: 2, TotalMs: 100, EnumerateMs: 35, IdentifyMs: 58, PersistMs: 6, PruneMs: 1, Candidates: 500, Identified: 500, Dirs: 10, FilesPerSec: 5000, PeakRSS: 21 << 20, HeapAlloc: 9 << 20},
		},
	}
	htmlPath, err := WriteReport(dir, rep)
	if err != nil {
		t.Fatal(err)
	}
	html, err := os.ReadFile(htmlPath)
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{
		"lain bench: scan", "<svg", "enumerate", "identify", "persist", "prune",
		"files/s", "4166", "results.json",
	} {
		if !strings.Contains(string(html), want) {
			t.Errorf("report.html missing %q", want)
		}
	}
	// No external references: must open offline.
	for _, banned := range []string{"http://", "https://", "<script"} {
		if strings.Contains(string(html), banned) {
			t.Errorf("report.html must be self-contained, contains %q", banned)
		}
	}
	// Dark-mode override must beat the base label rule: equal
	// specificity means source order decides, so the media query has
	// to come after `.lbl{fill:#1c1e21}`.
	base := strings.Index(string(html), ".lbl{fill:#1c1e21}")
	dark := strings.Index(string(html), "prefers-color-scheme:dark")
	if base < 0 || dark < 0 || dark < base {
		t.Errorf("dark-mode .lbl override must follow the base rule (base=%d dark=%d)", base, dark)
	}
	raw, err := os.ReadFile(filepath.Join(dir, "results.json"))
	if err != nil {
		t.Fatal(err)
	}
	var back Report
	if err := json.Unmarshal(raw, &back); err != nil {
		t.Fatalf("results.json invalid: %v", err)
	}
	if len(back.Runs) != 2 || back.Runs[0].Candidates != 500 {
		t.Fatalf("results.json roundtrip wrong: %+v", back.Runs)
	}
}
