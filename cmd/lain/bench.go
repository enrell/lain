// Command surface for `lain bench`: controlled workloads against real
// usage plus self-contained HTML reports. First workload: scan.
package main

import (
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"time"

	"github.com/enrell/lain/internal/bench"
	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/plugins/catalog"
	"github.com/enrell/lain/internal/plugins/identify"
	"github.com/enrell/lain/internal/plugins/ingest"
	"github.com/enrell/lain/internal/plugins/source"
	"github.com/enrell/lain/internal/store"
)

func cmdBench(args []string) error {
	if len(args) == 0 {
		return errUsage("usage: lain bench scan --path DIR [--runs N] [--out DIR] [--type T]")
	}
	switch args[0] {
	case "scan":
		return cmdBenchScan(args[1:])
	default:
		return errUsage("unknown bench target " + args[0])
	}
}

func errUsage(s string) error { return &usageError{s} }

type usageError struct{ s string }

func (e *usageError) Error() string { return e.s }

// benchInt parses --flag N with a default.
func benchInt(args []string, name string, def int) int {
	for i := 0; i+1 < len(args); i++ {
		if args[i] == "--"+name {
			var n int
			if _, err := fmt.Sscanf(args[i+1], "%d", &n); err == nil && n > 0 {
				return n
			}
		}
	}
	return def
}

func cmdBenchScan(args []string) error {
	path := flag(args, "path", "")
	if path == "" {
		return errUsage("lain bench scan --path DIR [--runs 3] [--out benchmarks] [--type anime]")
	}
	fi, err := os.Stat(path)
	if err != nil || !fi.IsDir() {
		return fmt.Errorf("path is not a readable directory: %s", path)
	}
	runs := benchInt(args, "runs", 3)
	out := flag(args, "out", "benchmarks")
	libType := flag(args, "type", "anime")

	rep := &bench.Report{
		Tool:      "lain bench scan",
		Target:    "scan",
		Path:      path,
		StartedAt: time.Now(),
		GoVersion: runtime.Version(),
		NumCPU:    runtime.NumCPU(),
		Note:      "Cold insert path: each run scans into a fresh data dir. The media tree is only read, never written.",
	}
	for i := 1; i <= runs; i++ {
		res, err := benchOneScan(path, libType, i)
		if err != nil {
			return fmt.Errorf("run %d: %w", i, err)
		}
		rep.Runs = append(rep.Runs, *res)
		fmt.Printf("run %d/%d: %d files in %.0f ms (%.0f files/s), peak RSS %s\n",
			i, runs, res.Candidates, res.TotalMs, res.FilesPerSec, humanBytesCLI(res.PeakRSS))
	}
	dir := filepath.Join(out, "scan-"+rep.StartedAt.Format("20060102-150405"))
	htmlPath, err := bench.WriteReport(dir, rep)
	if err != nil {
		return err
	}
	fmt.Println("report:", htmlPath)
	return nil
}

// benchOneScan wires the same composition the server uses and scans
// path into an isolated temp data dir (removed afterwards).
func benchOneScan(path, libType string, run int) (*bench.RunResult, error) {
	dataDir, err := os.MkdirTemp("", "lain-bench-*")
	if err != nil {
		return nil, err
	}
	defer os.RemoveAll(dataDir)

	st, err := store.New(dataDir)
	if err != nil {
		return nil, err
	}
	cat, err := catalog.New(st)
	if err != nil {
		return nil, err
	}
	reg := core.NewRegistry(core.DefaultComposition())
	reg.Register(source.Provider{})
	reg.Register(identify.Anime{})
	reg.Register(identify.Generic{})
	reg.Register(cat)
	trace := &ingest.ScanTrace{}
	runner := &ingest.Runner{Reg: reg, Cat: cat, Trace: trace}

	sampler := bench.Start(20 * time.Millisecond)
	t0 := time.Now()
	stats, err := runner.Run(ingest.ScanInput{
		Libraries: []contracts.Library{{ID: "bench", Name: "bench", Type: libType, Path: path}},
	})
	total := time.Since(t0)
	peak := sampler.Stop()
	if err != nil {
		return nil, err
	}
	var mem runtime.MemStats
	runtime.ReadMemStats(&mem)
	tr := trace.Snapshot()
	res := &bench.RunResult{
		Run:         run,
		TotalMs:     float64(total.Microseconds()) / 1000,
		EnumerateMs: float64(tr.Enumerate.Microseconds()) / 1000,
		IdentifyMs:  float64(tr.Identify.Microseconds()) / 1000,
		PersistMs:   float64(tr.Persist.Microseconds()) / 1000,
		PruneMs:     float64(tr.Prune.Microseconds()) / 1000,
		Candidates:  stats.Candidates,
		Identified:  stats.Identified,
		Dirs:        stats.Dirs,
		PeakRSS:     peak,
		HeapAlloc:   mem.HeapAlloc,
	}
	if total.Seconds() > 0 {
		res.FilesPerSec = float64(stats.Candidates) / total.Seconds()
	}
	return res, nil
}

func humanBytesCLI(b uint64) string {
	const u = 1024
	if b < u {
		return fmt.Sprintf("%d B", b)
	}
	v := float64(b)
	for _, suf := range []string{"KiB", "MiB", "GiB"} {
		v /= u
		if v < u || suf == "GiB" {
			return fmt.Sprintf("%.1f %s", v, suf)
		}
	}
	return fmt.Sprintf("%.1f GiB", v)
}
