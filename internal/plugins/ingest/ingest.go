// Package ingest is the default scan orchestrator
// (lain.ingest.scan@1). It knows the pipeline order but not the
// policies: sources enumerate, identifiers propose, the catalog owns
// identity. Swapping any of them changes behavior without touching
// this file. Catalog persistence goes through the registry
// (lain.catalog.write@1), so the catalog binding is load-bearing even
// inside a scan (D-075).
//
// Cost model: one library walk per root (parallel across roots), one
// catalog commit per walked root. Disk writes per scan stay constant in
// library size.
package ingest

import (
	"fmt"
	"sync"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/plugins/catalog"
	"github.com/enrell/lain/internal/plugins/source"
)

// ID is the built-in ingest provider id.
const ID = "lain-ingest-default"

// Runner executes scans against the registry.
type Runner struct {
	Reg *core.Registry
	// Trace, when non-nil, collects per-phase durations of Run.
	// Used by `lain bench scan`; production leaves it nil.
	Trace *ScanTrace
}

// ScanInput selects which libraries to walk.
type ScanInput struct {
	Libraries []contracts.Library `json:"libraries"`
}

func (r *Runner) ID() string             { return ID }
func (r *Runner) Capabilities() []string { return []string{contracts.CapIngestScan} }
func (r *Runner) Health() error          { return nil }

func (r *Runner) Invoke(cap string, input any) (any, error) {
	if cap != contracts.CapIngestScan {
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
	in, ok := input.(ScanInput)
	if !ok {
		return nil, &core.Error{Code: "invalid-message", Msg: "ScanInput required"}
	}
	return r.Run(in)
}

// libResult is one root's in-memory outcome. Nothing touches the
// catalog until its root finished: the commit is per-library, so a
// crash mid-scan can only leave a whole root's previous catalog intact,
// never a half-written one.
type libResult struct {
	libraryID  string
	name       string
	path       string
	entries    []catalog.ScanEntry
	accessible bool
	cleanWalk  bool
	candidates int
	identified int
	errors     int
	walkErrors int
	dirs       int
}

// Run walks every library root in parallel, identifies each candidate
// through the ordered-many binding, then commits each root through the
// catalog write binding (D-075): identity reconciliation (D-019) and
// missing-state marking (D-068) run inside the catalog per library, and
// only roots that were fully walked may mark — an unmounted drive or a
// mid-walk I/O error never reads as deletions.
func (r *Runner) Run(in ScanInput) (contracts.ScanStats, error) {
	stats := contracts.ScanStats{StartedAt: time.Now().Unix(), Libraries: len(in.Libraries)}
	results := make([]libResult, len(in.Libraries))
	var wg sync.WaitGroup
	for i, lib := range in.Libraries {
		wg.Add(1)
		go func(i int, lib contracts.Library) {
			defer wg.Done()
			results[i] = r.scanRoot(lib)
		}(i, lib)
	}
	wg.Wait()

	for i := range results {
		res := &results[i]
		stats.Candidates += res.candidates
		stats.Identified += res.identified
		stats.Errors += res.errors
		stats.WalkErrors += res.walkErrors
		stats.Dirs += res.dirs
		// An operator has to know which root to look at, not just how many
		// failed. Reasons are the two ways a walk degrades: the root could
		// not be read at all, or directories inside it could not be read.
		switch {
		case !res.accessible:
			stats.Unreadable = append(stats.Unreadable, contracts.UnreadableRoot{
				LibraryID: res.libraryID, Name: res.name, Path: res.path,
				Reason: "the path could not be read",
			})
		case res.walkErrors > 0:
			stats.Unreadable = append(stats.Unreadable, contracts.UnreadableRoot{
				LibraryID: res.libraryID, Name: res.name, Path: res.path,
				Reason: fmt.Sprintf("%d directories could not be read", res.walkErrors),
			})
		}
	}
	// One commit per walked root. A failed commit counts as an error and
	// leaves that root's previous catalog intact instead of aborting the
	// other libraries.
	t0 := time.Now()
	for i := range results {
		res := &results[i]
		out, _, err := r.Reg.CallOne(contracts.CapCatalogWrite, catalog.CommitScanInput{
			LibraryID:        res.libraryID,
			Entries:          res.entries,
			ReconcileMissing: res.accessible && res.cleanWalk,
		})
		if err != nil {
			stats.Errors++
			continue
		}
		commit, ok := out.(catalog.CommitScanOutput)
		if !ok {
			stats.Errors++
			continue
		}
		stats.Migrated += commit.Migrated
		stats.Missing += commit.Missing
		stats.Restored += commit.Restored
	}
	r.Trace.addPersist(time.Since(t0))
	stats.Unidentified = stats.Candidates - stats.Identified
	stats.FinishedAt = time.Now().Unix()
	return stats, nil
}

// scanRoot walks one root to completion in memory.
func (r *Runner) scanRoot(lib contracts.Library) libResult {
	res := libResult{libraryID: lib.ID, name: lib.Name, path: lib.Path}
	t0 := time.Now()
	out, _, err := r.Reg.CallOne(contracts.CapSourceEnumerate, source.EnumerateInput{
		Root: lib.Path, LibraryID: lib.ID, Type: lib.Type,
	})
	r.Trace.addEnumerate(time.Since(t0))
	if err != nil {
		res.errors++
		return res // root gone: accessible stays false, prune skipped
	}
	enumerated, ok := out.(source.EnumerateOutput)
	if !ok {
		res.errors++
		return res
	}
	cands := enumerated.Candidates
	es := enumerated.Stats
	res.dirs = es.Dirs
	res.walkErrors = es.WalkErrors
	res.accessible = es.Accessible
	res.cleanWalk = es.WalkErrors == 0
	t0 = time.Now()
	for _, c := range cands {
		res.candidates++
		out, _, accepted, err := r.Reg.CallFirst(contracts.CapMediaIdentify, c, func(v any) bool {
			p, ok := v.(contracts.Proposal)
			return ok && p.Accepted()
		})
		// The file still exists even when no identifier accepts it —
		// an identify failure is not a deletion (D-011/D-068). The
		// catalog counts every entry's path as present, so an
		// already-cataloged item at that path keeps its state.
		entry := catalog.ScanEntry{Candidate: c}
		if err != nil || !accepted {
			res.errors++
		} else {
			entry.Proposal = out.(contracts.Proposal)
			entry.Identified = true
			res.identified++
		}
		res.entries = append(res.entries, entry)
	}
	r.Trace.addIdentify(time.Since(t0))
	return res
}
