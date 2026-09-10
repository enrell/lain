// Package ingest is the default scan orchestrator
// (lain.ingest.scan@1). It knows the pipeline order but not the
// policies: sources enumerate, identifiers propose, the catalog owns
// identity. Swapping any of them changes behavior without touching
// this file.
package ingest

import (
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

// Run walks every library, identifies each candidate through the
// ordered-many binding, and writes accepted proposals to the catalog.
// Unidentified files are counted, never dropped with an error.
func (r *Runner) Run(in ScanInput) (contracts.ScanStats, error) {
	stats := contracts.ScanStats{StartedAt: time.Now().Unix(), Libraries: len(in.Libraries)}
	for _, lib := range in.Libraries {
		cands, err := source.Enumerate(source.EnumerateInput{
			Root: lib.Path, LibraryID: lib.ID, Type: lib.Type,
		})
		if err != nil {
			stats.Errors++
			continue
		}
		for _, c := range cands {
			stats.Candidates++
			out, _, accepted, err := r.Reg.CallFirst(contracts.CapMediaIdentify, c, func(v any) bool {
				p, ok := v.(contracts.Proposal)
				return ok && p.Accepted()
			})
			if err != nil || !accepted {
				stats.Errors++
				continue
			}
			p := out.(contracts.Proposal)
			if _, err := r.Cat.Upsert(catalog.UpsertInput{
				LibraryID: lib.ID, Proposal: p, Candidate: c,
			}); err != nil {
				stats.Errors++
				continue
			}
			stats.Identified++
		}
	}
	stats.Unidentified = stats.Candidates - stats.Identified
	stats.FinishedAt = time.Now().Unix()
	return stats, nil
}
