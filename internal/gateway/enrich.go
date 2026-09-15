package gateway

import (
	"fmt"
	"net/http"
	"path/filepath"
	"strings"

	"github.com/enrell/lain/internal/auth"
	"github.com/enrell/lain/internal/contracts"
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
	saver := metadata.NewSaver(s.db)
	writeJSON(w, 200, map[string]any{"items": saver.GetMany(ids)})
}

func (s *Server) handleEnrich(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	id := r.PathValue("id")
	item, ok := s.cat.Get(id)
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

// autoEnrichMissing enriches every catalog item that has no overlay yet.
// Every failure is skipped: a missing match or a down provider must never
// fail the scan that just succeeded. Items enrich one at a time, so
// provider APIs see a sequential trickle, never a burst.
func (s *Server) autoEnrichMissing() int {
	saver := metadata.NewSaver(s.db)
	done := 0
	for _, item := range s.cat.List() {
		if _, ok := saver.Get(item.ID); ok {
			continue
		}
		if _, _, errMsg := s.enrichOne(item, ""); errMsg != "" {
			continue
		}
		done++
	}
	return done
}

// enrichOne searches, resolves, and saves one overlay with the Auto
// provider set (only == ""), or a single named provider. It returns the
// HTTP status its caller should report alongside any error message.
func (s *Server) enrichOne(item contracts.CatalogItem, only string) (contracts.Enrichment, int, string) {
	kind := "anime"
	if item.Kind != "" && item.Kind != "episode" && item.Kind != "video" {
		kind = item.Kind
	}
	merged, err := s.searchMetadata(item.Title, kind, dirOf(item.FilePath), only)
	if err != nil {
		s.logger().Warn("enrich search failed", "item", item.ID, "title", item.Title, "err", err.Error())
		return contracts.Enrichment{}, 503, err.Error()
	}
	best, ok := metadata.BestPick(merged)
	if !ok {
		s.logger().Warn("enrich no match", "item", item.ID, "title", item.Title, "kind", kind)
		return contracts.Enrichment{}, 404, "no metadata found"
	}
	rec, err := s.resolveMetadata(best.Provider, best.RemoteID)
	if err != nil {
		s.logger().Warn("enrich resolve failed", "item", item.ID, "title", item.Title,
			"provider", best.Provider, "remote_id", best.RemoteID, "err", err.Error())
		return contracts.Enrichment{}, 503, err.Error()
	}
	saver := metadata.NewSaver(s.db)
	saved, err := saver.Save(item.ID, rec)
	if err != nil {
		s.logger().Error("enrich save failed", "item", item.ID, "title", item.Title, "err", err.Error())
		return contracts.Enrichment{}, 500, err.Error()
	}
	s.logger().Debug("enrich ok", "item", item.ID, "title", saved.Title, "provider", saved.Provider)
	return saved, 200, ""
}

func (s *Server) handleEnrichGet(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	saver := metadata.NewSaver(s.db)
	e, ok := saver.Get(r.PathValue("id"))
	if !ok {
		writeErr(w, 404, "not enriched")
		return
	}
	writeJSON(w, 200, e)
}

func (s *Server) handleEnrichDelete(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	saver := metadata.NewSaver(s.db)
	if err := saver.Delete(r.PathValue("id")); err != nil {
		writeErr(w, 500, err.Error())
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
