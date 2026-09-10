package gateway

import (
	"fmt"
	"net/http"
	"path/filepath"

	"github.com/enrell/lain/internal/auth"
	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/plugins/metadata"
)

func (s *Server) routesEnrich() {
	s.mux.HandleFunc("POST /api/catalog/{id}/enrich", s.requireAdmin(s.handleEnrich))
	s.mux.HandleFunc("GET /api/catalog/{id}/enrich", s.requireAuth(s.handleEnrichGet))
	s.mux.HandleFunc("DELETE /api/catalog/{id}/enrich", s.requireAdmin(s.handleEnrichDelete))
}

func (s *Server) handleEnrich(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	id := r.PathValue("id")
	item, ok := s.cat.Get(id)
	if !ok {
		writeErr(w, 404, "unknown item")
		return
	}
	only := r.URL.Query().Get("provider")
	kind := "anime"
	if item.Kind != "" && item.Kind != "episode" && item.Kind != "video" {
		kind = item.Kind
	}
	merged, err := s.searchMetadata(item.Title, kind, dirOf(item.FilePath), only)
	if err != nil {
		writeErr(w, 503, err.Error())
		return
	}
	best, ok := metadata.BestPick(merged)
	if !ok {
		writeErr(w, 404, "no metadata found")
		return
	}
	rec, err := s.resolveMetadata(best.Provider, best.RemoteID)
	if err != nil {
		writeErr(w, 503, err.Error())
		return
	}
	saver := metadata.NewSaver(s.db)
	saved, err := saver.Save(id, rec)
	if err != nil {
		writeErr(w, 500, err.Error())
		return
	}
	writeJSON(w, 200, saved)
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
// only restricts to one provider (empty = all bound).
func (s *Server) searchMetadata(query, kind, dir, only string) ([]contracts.MetadataCandidate, error) {
	cache := metadata.NewCache(s.db)
	if only == "" {
		if hit, ok := cache.GetSearch(kind, query); ok {
			return hit, nil
		}
	}
	in := contracts.MetadataSearchInput{Query: query, Kind: kind, Limit: 5, Dir: dir}
	out, _, err := s.reg.CallMerge(contracts.CapMetadataSearch, in, func(outputs []any, ids []string) any {
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
	if err != nil {
		return nil, err
	}
	merged, _ := out.([]contracts.MetadataCandidate)
	if only == "" {
		_ = cache.PutSearch(kind, query, merged)
	}
	return merged, nil
}

func boxAll(list []contracts.MetadataCandidate) []any {
	return []any{list}
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
