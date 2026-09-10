// Package ingest is the default scan orchestrator
// (lain.ingest.scan@1). It knows the pipeline order but not the
// policies: sources enumerate, identifiers propose, the catalog owns
// identity. Swapping any of them changes behavior without touching
// this file.
//
// Cost model: one library walk per root (parallel across roots), one
// catalog persist for all upserts, one more only when pruning removed
// something. Disk writes per scan are constant in library size.
package ingest

import (
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
	Cat *catalog.Service
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
// catalog until every root finished: a crash mid-scan leaves the
// previous catalog intact instead of half-written.
type libResult struct {
	libraryID  string
	items      []contracts.CatalogItem
	present    map[string]bool
	accessible bool
	cleanWalk  bool
	candidates int
	identified int
	errors     int
	walkErrors int
	dirs       int
}

// Run walks every library root in parallel, identifies each candidate
// through the ordered-many binding, persists all upserts at once, then
// prunes files that vanished — but only for roots that were fully
// walked. An unmounted drive or a mid-walk I/O error never reads as
// deletions.
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

	var all []contracts.CatalogItem
	for i := range results {
		res := &results[i]
		stats.Candidates += res.candidates
		stats.Identified += res.identified
		stats.Errors += res.errors
		stats.WalkErrors += res.walkErrors
		stats.Dirs += res.dirs
		all = append(all, res.items...)
	}
	if err := r.Cat.UpsertBatch(all); err != nil {
		return stats, err
	}
	for i := range results {
		res := &results[i]
		if !res.accessible || !res.cleanWalk {
			continue
		}
		n, err := r.Cat.PruneMissing(res.libraryID, res.present)
		if err != nil {
			stats.Errors++
			continue
		}
		stats.Pruned += n
	}
	stats.Unidentified = stats.Candidates - stats.Identified
	stats.FinishedAt = time.Now().Unix()
	return stats, nil
}

// scanRoot walks one root to completion in memory.
func (r *Runner) scanRoot(lib contracts.Library) libResult {
	res := libResult{libraryID: lib.ID, present: map[string]bool{}}
	cands, es, err := source.Enumerate(source.EnumerateInput{
		Root: lib.Path, LibraryID: lib.ID, Type: lib.Type,
	})
	res.dirs = es.Dirs
	res.walkErrors = es.WalkErrors
	if err != nil {
		res.errors++
		return res // root gone: accessible stays false, prune skipped
	}
	res.accessible = es.Accessible
	res.cleanWalk = es.WalkErrors == 0
	for _, c := range cands {
		res.candidates++
		out, _, accepted, err := r.Reg.CallFirst(contracts.CapMediaIdentify, c, func(v any) bool {
			p, ok := v.(contracts.Proposal)
			return ok && p.Accepted()
		})
		if err != nil || !accepted {
			res.errors++
			continue
		}
		it := catalog.NewItem(lib.ID, out.(contracts.Proposal), c)
		res.items = append(res.items, it)
		res.present[it.ID] = true
		res.identified++
	}
	return res
}
