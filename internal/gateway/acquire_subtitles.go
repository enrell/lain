package gateway

import (
	"net/http"
	"strings"

	"github.com/enrell/lain/internal/acquire"
	"github.com/enrell/lain/internal/auth"
	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/subtitle"
)

// Subtitle acquisition routes (docs/slices/acquisition.md, Phase 3).
// Admin only; players read sidecars through /api/items/{id}/sidecars.

func (s *Server) routesAcquireSubtitles() {
	m := s.mux
	a := func(h acqHandler) http.HandlerFunc { return s.requireAdmin(s.acq(h)) }
	m.HandleFunc("GET /api/acquire/subtitle-providers", a(s.handleSubProviders))
	m.HandleFunc("POST /api/acquire/subtitle-providers", a(s.handleSubProviderCreate))
	m.HandleFunc("PUT /api/acquire/subtitle-providers/{id}", a(s.handleSubProviderUpdate))
	m.HandleFunc("DELETE /api/acquire/subtitle-providers/{id}", a(s.handleSubProviderDelete))
	m.HandleFunc("GET /api/acquire/items/{id}/subtitles", a(s.handleItemSubtitleSearch))
	m.HandleFunc("POST /api/acquire/items/{id}/subtitles", a(s.handleItemSubtitleDownload))
	m.HandleFunc("GET /api/acquire/monitored/{id}/subtitles", a(s.handleMonitoredSubtitles))
	m.HandleFunc("POST /api/acquire/monitored/{id}/subtitles", a(s.handleMonitoredSubtitlePass))
	m.HandleFunc("GET /api/acquire/items/{id}/subtitles/status", a(s.handleItemSubtitleStatus))
	m.HandleFunc("DELETE /api/acquire/items/{id}/sidecars/{name}", a(s.handleSidecarRemove))
	m.HandleFunc("GET /api/acquire/subtitles", a(s.handleSubtitleLedger))
	m.HandleFunc("GET /api/acquire/subtitle-refusals", a(s.handleSubtitleRefusals))
	m.HandleFunc("DELETE /api/acquire/subtitle-refusals", a(s.handleSubtitleRefusalsClear))
	m.HandleFunc("DELETE /api/acquire/subtitle-refusals/{provider}/{file}", a(s.handleSubtitleRefusalClear))
}

// subtitleDeps wires the probe and lain.subtitle@1 into acquisition.
func (s *Server) subtitleDeps(d *acquire.Deps) {
	d.Probe = func(path string) (contracts.MediaInfo, error) {
		out, _, err := s.reg.CallOne(contracts.CapMediaProbe, contracts.MediaProbeRequest{FilePath: path})
		if err != nil {
			return contracts.MediaInfo{}, err
		}
		info, _ := out.(contracts.MediaInfo)
		return info, nil
	}
	d.SubtitleSearch = func(in contracts.SubtitleSearchInput) ([]contracts.SubtitleCandidate, error) {
		out, err := s.subtitleCall(in)
		if err != nil {
			return nil, err
		}
		res, _ := out.(contracts.SubtitleSearchOutput)
		return res.Candidates, nil
	}
	d.SubtitleDownload = func(in contracts.SubtitleDownloadInput) (contracts.SubtitleDownloadOutput, error) {
		out, err := s.subtitleCall(in)
		if err != nil {
			return contracts.SubtitleDownloadOutput{}, err
		}
		res, _ := out.(contracts.SubtitleDownloadOutput)
		return res, nil
	}
}

func (s *Server) subtitleCall(in any) (any, error) {
	out, _, ok, err := s.reg.CallFirst(contracts.CapSubtitle, in, func(any) bool { return true })
	if !ok {
		if err == nil {
			err = &acquire.Error{Code: acquire.CodeUnsupported, Msg: "no subtitle provider speaks this kind"}
		}
		return nil, err
	}
	return out, nil
}

type subProviderView struct {
	contracts.SubtitleProvider
	HasAPIKey   bool `json:"has_api_key"`
	HasPassword bool `json:"has_password"`
}

func (s *Server) subProviderView(p contracts.SubtitleProvider) subProviderView {
	k, pw := s.acquire.SubtitleProviderSecrets(p.ID)
	return subProviderView{SubtitleProvider: p.Public(), HasAPIKey: k, HasPassword: pw}
}

func (s *Server) handleSubProviders(w http.ResponseWriter, _ *http.Request, _ auth.Verified) {
	out := []subProviderView{}
	for _, p := range s.acquire.SubtitleProviders() {
		out = append(out, s.subProviderView(p))
	}
	writeJSON(w, http.StatusOK, map[string]any{"providers": out})
}

func (s *Server) handleSubProviderCreate(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	var in acquire.SubtitleProviderInput
	if !s.decode(w, r, &in) {
		return
	}
	p, err := s.acquire.CreateSubtitleProvider(in)
	if err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusCreated, s.subProviderView(p))
}

func (s *Server) handleSubProviderUpdate(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	var in acquire.SubtitleProviderInput
	if !s.decode(w, r, &in) {
		return
	}
	p, err := s.acquire.UpdateSubtitleProvider(r.PathValue("id"), in)
	if err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, s.subProviderView(p))
}

func (s *Server) handleSubProviderDelete(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	if err := s.acquire.DeleteSubtitleProvider(r.PathValue("id")); err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"removed": true})
}

// itemTarget resolves a catalog item and the profile that governs it:
// its monitored title's profile when it has one, else the default.
func (s *Server) itemTarget(w http.ResponseWriter, id string) (contracts.CatalogItem, acquire.Profile, bool) {
	it, ok := s.catGet(id)
	if !ok {
		writeJSON(w, http.StatusNotFound, map[string]string{"error": "unknown item", "code": acquire.CodeNotFound})
		return it, acquire.Profile{}, false
	}
	p, err := s.acquire.ProfileForItem(it)
	if err != nil {
		writeAcqErr(w, err)
		return it, p, false
	}
	return it, p, true
}

func (s *Server) handleItemSubtitleSearch(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	it, p, ok := s.itemTarget(w, r.PathValue("id"))
	if !ok {
		return
	}
	var langs []string
	if raw := strings.TrimSpace(r.URL.Query().Get("languages")); raw != "" {
		for _, l := range strings.Split(raw, ",") {
			code := subtitle.Language(strings.TrimSpace(l))
			if code == "" {
				writeAcqErr(w, &acquire.Error{Code: acquire.CodeInvalid, Msg: "unknown language " + strings.TrimSpace(l)})
				return
			}
			langs = append(langs, code)
		}
	} else {
		langs = p.SubtitleLanguages
	}
	if len(langs) == 0 {
		writeAcqErr(w, &acquire.Error{Code: acquire.CodeInvalid, Msg: "name the languages to search, or set them in the quality profile"})
		return
	}
	t := acquire.SubtitleTarget{Path: it.FilePath, Kind: it.Kind, Title: it.Title, Season: it.Season, Episode: it.Episode, Year: it.Year}
	choices, err := s.acquire.SearchSubtitles(t, p, langs)
	if err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"choices": choices, "languages": langs})
}

func (s *Server) handleItemSubtitleDownload(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	it, ok := s.catGet(r.PathValue("id"))
	if !ok {
		writeJSON(w, http.StatusNotFound, map[string]string{"error": "unknown item", "code": acquire.CodeNotFound})
		return
	}
	var in struct {
		contracts.SubtitleCandidate
		Replace bool `json:"replace"`
	}
	if !s.decode(w, r, &in) {
		return
	}
	rec, err := s.acquire.DownloadSubtitle(it.FilePath, in.SubtitleCandidate, in.Replace)
	if err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, rec)
}

func (s *Server) handleMonitoredSubtitles(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	st, err := s.acquire.SubtitleStatus(r.PathValue("id"))
	if err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"files": st})
}

func (s *Server) handleMonitoredSubtitlePass(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	n, err := s.acquire.SubtitlesForMonitored(r.PathValue("id"))
	if err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"written": n})
}

func (s *Server) handleSubtitleLedger(w http.ResponseWriter, _ *http.Request, _ auth.Verified) {
	writeJSON(w, http.StatusOK, map[string]any{"subtitles": s.acquire.SubtitleLedger()})
}

func (s *Server) handleItemSubtitleStatus(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	it, ok := s.catGet(r.PathValue("id"))
	if !ok {
		writeJSON(w, http.StatusNotFound, map[string]string{"error": "unknown item", "code": acquire.CodeNotFound})
		return
	}
	st, langs, err := s.acquire.ItemSubtitleStatus(it)
	if err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"file": st, "languages": langs})
}

// handleSidecarRemove moves a sidecar into the holding folder; nothing
// is deleted (A-35, D-128).
func (s *Server) handleSidecarRemove(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	it, ok := s.catGet(r.PathValue("id"))
	if !ok {
		writeJSON(w, http.StatusNotFound, map[string]string{"error": "unknown item", "code": acquire.CodeNotFound})
		return
	}
	held, err := s.acquire.RemoveSidecar(it.FilePath, r.PathValue("name"))
	if err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, held)
}

func (s *Server) handleSubtitleRefusals(w http.ResponseWriter, _ *http.Request, _ auth.Verified) {
	writeJSON(w, http.StatusOK, map[string]any{"refusals": s.acquire.SubtitleRefusals()})
}

func (s *Server) handleSubtitleRefusalClear(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	if err := s.acquire.ClearSubtitleRefusal(r.PathValue("provider"), r.PathValue("file")); err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"removed": true})
}

func (s *Server) handleSubtitleRefusalsClear(w http.ResponseWriter, _ *http.Request, _ auth.Verified) {
	writeJSON(w, http.StatusOK, map[string]any{"cleared": s.acquire.ClearSubtitleRefusals()})
}
