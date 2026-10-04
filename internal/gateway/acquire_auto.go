package gateway

import (
	"net/http"

	"github.com/enrell/lain/internal/acquire"
	"github.com/enrell/lain/internal/auth"
	"github.com/enrell/lain/internal/plugins/catalog"
)

// Acquisition Phase 2 routes (docs/slices/acquisition.md, A-17…A-25):
// quality profiles, monitored titles and their wanted units, on-demand
// search and RSS, the blocklist and held (replaced) files. Admin only.

func (s *Server) routesAcquireAuto() {
	m := s.mux
	a := func(h acqHandler) http.HandlerFunc { return s.requireAdmin(s.acq(h)) }
	m.HandleFunc("GET /api/acquire/profiles", a(s.handleAcqProfiles))
	m.HandleFunc("POST /api/acquire/profiles", a(s.handleAcqProfileCreate))
	m.HandleFunc("PUT /api/acquire/profiles/{id}", a(s.handleAcqProfileUpdate))
	m.HandleFunc("DELETE /api/acquire/profiles/{id}", a(s.handleAcqProfileDelete))
	m.HandleFunc("GET /api/acquire/monitored", a(s.handleAcqMonitored))
	m.HandleFunc("POST /api/acquire/monitored", a(s.handleAcqMonitoredCreate))
	m.HandleFunc("GET /api/acquire/monitored/{id}", a(s.handleAcqMonitoredGet))
	m.HandleFunc("PUT /api/acquire/monitored/{id}", a(s.handleAcqMonitoredUpdate))
	m.HandleFunc("DELETE /api/acquire/monitored/{id}", a(s.handleAcqMonitoredDelete))
	m.HandleFunc("GET /api/acquire/monitored/{id}/wanted", a(s.handleAcqWantedOne))
	m.HandleFunc("POST /api/acquire/monitored/{id}/search", a(s.handleAcqMonitoredSearch))
	m.HandleFunc("GET /api/acquire/wanted", a(s.handleAcqWanted))
	m.HandleFunc("POST /api/acquire/rss", a(s.handleAcqRSS))
	m.HandleFunc("GET /api/acquire/automation", a(s.handleAcqAutomation))
	m.HandleFunc("GET /api/acquire/blocklist", a(s.handleAcqBlocklist))
	m.HandleFunc("DELETE /api/acquire/blocklist/{id}", a(s.handleAcqUnblock))
	m.HandleFunc("GET /api/acquire/replaced", a(s.handleAcqReplaced))
	m.HandleFunc("POST /api/acquire/replaced/purge", a(s.handleAcqPurge))
}

// episodeCount asks metadata providers (cached) for a title's episode
// total: the closest-titled candidate's resolved record (A-18).
func (s *Server) episodeCount(title, kind string) int {
	cands, err := s.searchMetadata(title, kind, "", "")
	if err != nil || len(cands) == 0 {
		return 0
	}
	key := catalog.TitleKey(title)
	best := cands[0]
	for _, c := range cands {
		if catalog.TitleKey(c.Title) == key {
			best = c
			break
		}
	}
	rec, err := s.resolveMetadata(best.Provider, best.RemoteID)
	if err != nil {
		return 0
	}
	return rec.Episodes
}

func (s *Server) handleAcqProfiles(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	writeJSON(w, http.StatusOK, map[string]any{"profiles": s.acquire.Profiles()})
}

func (s *Server) handleAcqProfileCreate(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	var in acquire.Profile
	if !s.decode(w, r, &in) {
		return
	}
	p, err := s.acquire.CreateProfile(in)
	if err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusCreated, p)
}

func (s *Server) handleAcqProfileUpdate(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	var in acquire.Profile
	if !s.decode(w, r, &in) {
		return
	}
	p, err := s.acquire.UpdateProfile(r.PathValue("id"), in)
	if err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, p)
}

func (s *Server) handleAcqProfileDelete(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	if err := s.acquire.DeleteProfile(r.PathValue("id")); err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"removed": true})
}

func (s *Server) handleAcqMonitored(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	writeJSON(w, http.StatusOK, map[string]any{"monitored": s.acquire.MonitoredList()})
}

func (s *Server) handleAcqMonitoredCreate(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	var in acquire.Monitored
	if !s.decode(w, r, &in) {
		return
	}
	mon, err := s.acquire.CreateMonitored(in)
	if err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusCreated, mon)
}

func (s *Server) handleAcqMonitoredGet(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	mon, err := s.acquire.Monitored(r.PathValue("id"))
	if err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, mon)
}

func (s *Server) handleAcqMonitoredUpdate(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	var in acquire.Monitored
	if !s.decode(w, r, &in) {
		return
	}
	mon, err := s.acquire.UpdateMonitored(r.PathValue("id"), in)
	if err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, mon)
}

func (s *Server) handleAcqMonitoredDelete(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	if err := s.acquire.DeleteMonitored(r.PathValue("id")); err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"removed": true})
}

func (s *Server) handleAcqWantedOne(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	wanted, err := s.acquire.WantedFor(r.PathValue("id"))
	if err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, wanted)
}

// wantedRow is one monitored title in the Wanted overview.
type wantedRow struct {
	acquire.Monitored
	Wanted       acquire.Wanted `json:"wanted"`
	MissingCount int            `json:"missing_count"`
	UpgradeCount int            `json:"upgrade_count"`
}

func (s *Server) handleAcqWanted(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	rows := []wantedRow{}
	for _, mon := range s.acquire.MonitoredList() {
		wanted, err := s.acquire.WantedFor(mon.ID)
		if err != nil {
			continue
		}
		rows = append(rows, wantedRow{Monitored: mon, Wanted: wanted, MissingCount: len(wanted.Missing), UpgradeCount: len(wanted.Upgrades)})
	}
	writeJSON(w, http.StatusOK, map[string]any{"titles": rows, "automation": s.acquire.AutomationStatus()})
}

func (s *Server) handleAcqMonitoredSearch(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	rep, err := s.acquire.SearchMonitored(r.PathValue("id"))
	if err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, rep)
}

func (s *Server) handleAcqRSS(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	rep, err := s.acquire.RSSSync()
	if err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, rep)
}

func (s *Server) handleAcqAutomation(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	writeJSON(w, http.StatusOK, s.acquire.AutomationStatus())
}

func (s *Server) handleAcqBlocklist(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	writeJSON(w, http.StatusOK, map[string]any{"blocklist": s.acquire.Blocklist()})
}

func (s *Server) handleAcqUnblock(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	if err := s.acquire.Unblock(r.PathValue("id")); err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"removed": true})
}

func (s *Server) handleAcqReplaced(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	writeJSON(w, http.StatusOK, map[string]any{"files": s.acquire.Replaced()})
}

func (s *Server) handleAcqPurge(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	n, err := s.acquire.PurgeReplaced()
	if err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"deleted": n})
}
