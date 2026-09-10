package bench

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

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
