package gateway

import (
	"fmt"
	"net/http"
	"path/filepath"
	"strings"

	"github.com/enrell/lain/internal/auth"
	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/plugins/metadata"
)

func (s *Server) routesEnrich() {
	s.mux.HandleFunc("POST /api/catalog/{id}/enrich", s.requireAdmin(s.handleEnrich))
	s.mux.HandleFunc("GET /api/catalog/{id}/enrich", s.requireAuth(s.handleEnrichGet))
	s.mux.HandleFunc("DELETE /api/catalog/{id}/enrich", s.requireAdmin(s.handleEnrichDelete))
	s.mux.HandleFunc("GET /api/enrichments", s.requireAuth(s.handleEnrichBatch))
}

// maxEnrichBatch bounds one batch overlay read: grids ask for a page of
// items, not the whole catalog.
const maxEnrichBatch = 200

// enrichErr maps a capability failure to the status the enrich surface
// reported before it was a capability.
func enrichErr(w http.ResponseWriter, err error) {
	code := http.StatusInternalServerError
	if ce, ok := err.(*core.Error); ok {
		switch ce.Code {
		case "not-found":
			code = http.StatusNotFound
		case "dependency-unavailable":
			code = http.StatusServiceUnavailable
		}
	}
	writeErr(w, code, err.Error())
}

// handleEnrichBatch returns the overlays that exist for a set of item
// ids in one read transaction: artwork for a grid without N+1
// requests. Missing overlays are simply absent from the result.
func (s *Server) handleEnrichBatch(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	raw := strings.TrimSpace(r.URL.Query().Get("ids"))
	if raw == "" {
		writeErr(w, 400, "ids required")
		return
	}
	parts := strings.Split(raw, ",")
	if len(parts) > maxEnrichBatch {
		writeErr(w, 400, fmt.Sprintf("too many ids (max %d)", maxEnrichBatch))
		return
	}
	ids := make([]string, 0, len(parts))
	for _, p := range parts {
		if p = strings.TrimSpace(p); p != "" {
			ids = append(ids, p)
		}
	}
	out, _, err := s.reg.CallOne(contracts.CapMetadataEnrich, metadata.BatchInput{IDs: ids})
	if err != nil {
		enrichErr(w, err)
		return
	}
	batch, _ := out.(metadata.BatchOutput)
	writeJSON(w, 200, map[string]any{"items": batch.Items})
}

func (s *Server) handleEnrich(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	id := r.PathValue("id")
	item, ok := s.catGet(id)
	if !ok {
		writeErr(w, 404, "unknown item")
		return
	}
	saved, code, errMsg := s.enrichOne(item, r.URL.Query().Get("provider"))
	if errMsg != "" {
		writeErr(w, code, errMsg)
		return
	}
	writeJSON(w, 200, saved)
}

// enrichOne runs one item through the enrich capability and maps the
// outcome to the (enrichment, httpCode, message) triple the surface
// reported before the capability existed.
func (s *Server) enrichOne(item contracts.CatalogItem, only string) (contracts.Enrichment, int, string) {
	out, _, err := s.reg.CallOne(contracts.CapMetadataEnrich, metadata.EnrichInput{Item: item, Only: only})
	if err != nil {
		code := http.StatusInternalServerError
		if ce, ok := err.(*core.Error); ok {
			switch ce.Code {
			case "not-found":
				code = http.StatusNotFound
			case "dependency-unavailable":
				code = http.StatusServiceUnavailable
			}
		}
		return contracts.Enrichment{}, code, err.Error()
	}
	saved, _ := out.(contracts.Enrichment)
	return saved, http.StatusOK, ""
}

// autoEnrichMissing asks the enrich provider to backfill overlays for
// every catalog item lacking one. A failure here degrades to zero
// overlays — the scan that just ran still succeeded.
func (s *Server) autoEnrichMissing() int {
	out, _, err := s.reg.CallOne(contracts.CapMetadataEnrich, metadata.BackfillInput{})
	if err != nil {
		s.logger().Warn("auto-enrich failed", "err", err.Error())
		return 0
	}
	report, _ := out.(metadata.BackfillOutput)
	return report.Enriched
}

func (s *Server) handleEnrichGet(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	out, _, err := s.reg.CallOne(contracts.CapMetadataEnrich, metadata.GetInput{ItemID: r.PathValue("id")})
	if err != nil {
		enrichErr(w, err)
		return
	}
	writeJSON(w, 200, out)
}

func (s *Server) handleEnrichDelete(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	if _, _, err := s.reg.CallOne(contracts.CapMetadataEnrich, metadata.DeleteInput{ItemID: r.PathValue("id")}); err != nil {
		enrichErr(w, err)
		return
	}
	writeJSON(w, 200, map[string]string{"status": "ok"})
}

// searchMetadata serves from cache, else fans out and caches the merge.
// only restricts to one provider (empty = all bound). Empty merges are
// never cached: a down provider must not poison the next attempt.
func (s *Server) searchMetadata(query, kind, dir, only string) ([]contracts.MetadataCandidate, error) {
	cache := metadata.NewCache(s.db)
	if only == "" {
		if hit, ok := cache.GetSearch(kind, query); ok {
			return hit, nil
		}
	}
	in := contracts.MetadataSearchInput{Query: query, Kind: kind, Limit: 5, Dir: dir}
	out, ids, skipped, err := s.reg.CallMergeReport(contracts.CapMetadataSearch, in, func(outputs []any, ids []string) any {
		if only != "" {
			for i, id := range ids {
				if id == only {
					return metadata.MergeCandidates(query, outputs[i:i+1], ids[i:i+1], 10)
				}
			}
			return []contracts.MetadataCandidate{}
		}
		return metadata.MergeCandidates(query, outputs, ids, 10)
	})
	// Skipped providers degrade the merge silently by contract; the
	// causes live here at debug so a dead upstream is one grep away.
	// Logged before the error return so total outages keep attribution.
	if len(skipped) > 0 {
		causes := make([]string, 0, len(skipped))
		for _, f := range skipped {
			causes = append(causes, f.Provider+": "+f.Err.Error())
		}
		s.logger().Debug("metadata providers skipped", "query", query, "kind", kind, "causes", strings.Join(causes, "; "))
	}
	if err != nil {
		return nil, err
	}
	merged, _ := out.([]contracts.MetadataCandidate)
	s.logger().Debug("metadata search", "query", query, "kind", kind, "hits", len(merged), "from", strings.Join(ids, ","))
	if only == "" && len(merged) > 0 {
		_ = cache.PutSearch(kind, query, merged)
	}
	return merged, nil
}

// resolveMetadata serves from cache, else calls the winning provider
// directly (still through registry authority).
func (s *Server) resolveMetadata(provider, remoteID string) (contracts.MetadataRecord, error) {
	cache := metadata.NewCache(s.db)
	if rec, ok := cache.GetRecord(provider, remoteID); ok {
		return rec, nil
	}
	out, err := s.reg.InvokeProvider(contracts.CapMetadataResolve, provider, contracts.MetadataResolveInput{Provider: provider, RemoteID: remoteID})
	if err != nil {
		return contracts.MetadataRecord{}, err
	}
	rec, ok := out.(contracts.MetadataRecord)
	if !ok {
		return contracts.MetadataRecord{}, fmt.Errorf("bad record shape from %s", provider)
	}
	_ = cache.PutRecord(rec)
	return rec, nil
}

func dirOf(path string) string {
	if path == "" {
		return ""
	}
	return filepath.Dir(path)
}
