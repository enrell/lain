package gateway

import (
	"errors"
	"net/http"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"

	"github.com/enrell/lain/internal/acquire"
	"github.com/enrell/lain/internal/auth"
	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/downloads"
	"github.com/enrell/lain/internal/plugins/release"
)

// Acquisition (docs/slices/acquisition.md). Every route is admin-only
// (A-9): acquisition makes the server contact remote hosts and write to
// disk. Parsing and indexer protocols go through the registry; the
// manager owns the engine, the queue and import.

// parserSocket is the lain-parser socket: <data-dir>/parser.sock unless
// LAIN_PARSER_SOCKET points elsewhere.
func parserSocket(dataDir string) string {
	if p := strings.TrimSpace(os.Getenv("LAIN_PARSER_SOCKET")); p != "" {
		return p
	}
	return filepath.Join(dataDir, "parser.sock")
}

// parseRelease calls lain.release.parse@1; the tokenizer answers if
// every bound provider declines or the capability is withdrawn.
func (s *Server) parseRelease(name, kind string) contracts.Release {
	in := contracts.ReleaseParseInput{Name: name, Kind: kind}
	out, _, ok, err := s.reg.CallFirst(contracts.CapReleaseParse, in, func(v any) bool {
		r, ok := v.(contracts.Release)
		return ok && r.Accepted()
	})
	if ok {
		return out.(contracts.Release)
	}
	if err != nil {
		s.logger().Debug("release parse fell back", "err", err.Error())
	}
	return release.Tokenize(name, kind)
}

func (s *Server) indexerCall(in any) (any, error) {
	out, _, ok, err := s.reg.CallFirst(contracts.CapIndexer, in, func(any) bool { return true })
	if !ok {
		if err == nil {
			err = &acquire.Error{Code: acquire.CodeUnsupported, Msg: "no indexer provider speaks this protocol"}
		}
		return nil, err
	}
	return out, nil
}

// libraryTitles lists the distinct titles already in a library.
func (s *Server) libraryTitles(libraryID string) []string {
	seen := map[string]bool{}
	var out []string
	for _, it := range s.catList() {
		if it.LibraryID == libraryID && !seen[it.Title] {
			seen[it.Title] = true
			out = append(out, it.Title)
		}
	}
	sort.Strings(out)
	return out
}

// startAcquire builds the acquisition manager. A failure never blocks
// boot: the routes answer 503 instead.
func (s *Server) startAcquire(dataDir string) {
	deps := acquire.Deps{
		DB: s.db, DataDir: dataDir, Parse: s.parseRelease, Logger: s.logger(),
		Search: func(in contracts.IndexerSearchInput) ([]contracts.SearchResult, error) {
			out, err := s.indexerCall(in)
			if err != nil {
				return nil, err
			}
			res, _ := out.(contracts.IndexerSearchOutput)
			return res.Results, nil
		},
		Caps: func(ix contracts.Indexer) (contracts.IndexerCaps, error) {
			out, err := s.indexerCall(contracts.IndexerCapsInput{Indexer: ix})
			if err != nil {
				return contracts.IndexerCaps{}, err
			}
			caps, _ := out.(contracts.IndexerCaps)
			return caps, nil
		},
		// One disk budget for both download paths (A-4): the reading
		// slice's limits, and what its HTTP manager already holds.
		Budget: func() (downloads.Limits, int64) {
			return s.downloads.Settings().Limits, s.downloads.Usage().UsedBytes
		},
		Library: s.libByID,
		Titles:  s.libraryTitles,
		Rescan:  s.scanLibrary,
	}
	initial := acquire.DefaultSettings(dataDir)
	if p, err := strconv.Atoi(os.Getenv("LAIN_ACQUIRE_LISTEN_PORT")); err == nil && p >= 0 && p <= 65535 {
		initial.ListenPort = p
		deps.Initial = &initial
	}
	m, err := acquire.New(deps)
	if err != nil {
		// Most likely the listen port is taken: retry on a free port so
		// acquisition still works, and say so.
		s.logger().Warn("acquisition engine failed to start; retrying on a free port", "err", err.Error())
		initial.ListenPort = 0
		deps.Initial = &initial
		if m, err = acquire.New(deps); err != nil {
			s.logger().Error("acquisition disabled", "err", err.Error())
			return
		}
	}
	m.Start()
	s.acquire = m
}

func (s *Server) routesAcquire() {
	m := s.mux
	m.HandleFunc("GET /api/acquire/settings", s.requireAdmin(s.acq(s.handleAcqSettingsGet)))
	m.HandleFunc("PUT /api/acquire/settings", s.requireAdmin(s.acq(s.handleAcqSettingsPut)))
	m.HandleFunc("GET /api/acquire/indexers", s.requireAdmin(s.acq(s.handleAcqIndexers)))
	m.HandleFunc("POST /api/acquire/indexers", s.requireAdmin(s.acq(s.handleAcqIndexerCreate)))
	m.HandleFunc("PATCH /api/acquire/indexers/{id}", s.requireAdmin(s.acq(s.handleAcqIndexerPatch)))
	m.HandleFunc("DELETE /api/acquire/indexers/{id}", s.requireAdmin(s.acq(s.handleAcqIndexerDelete)))
	m.HandleFunc("POST /api/acquire/indexers/{id}/test", s.requireAdmin(s.acq(s.handleAcqIndexerTest)))
	m.HandleFunc("GET /api/acquire/search", s.requireAdmin(s.acq(s.handleAcqSearch)))
	m.HandleFunc("GET /api/acquire/parse", s.requireAdmin(s.acq(s.handleAcqParse)))
	m.HandleFunc("GET /api/acquire/grabs", s.requireAdmin(s.acq(s.handleAcqGrabs)))
	m.HandleFunc("POST /api/acquire/grabs", s.requireAdmin(s.acq(s.handleAcqGrab)))
	m.HandleFunc("POST /api/acquire/grabs/{id}/{action}", s.requireAdmin(s.acq(s.handleAcqGrabAction)))
	m.HandleFunc("DELETE /api/acquire/grabs/{id}", s.requireAdmin(s.acq(s.handleAcqGrabDelete)))
}

type acqHandler func(http.ResponseWriter, *http.Request, auth.Verified)

// acq answers 503 while acquisition is unavailable.
func (s *Server) acq(h acqHandler) acqHandler {
	return func(w http.ResponseWriter, r *http.Request, v auth.Verified) {
		if s.acquire == nil {
			writeJSON(w, http.StatusServiceUnavailable, map[string]string{"error": "acquisition is unavailable; see the server log", "code": "unavailable"})
			return
		}
		h(w, r, v)
	}
}

func writeAcqErr(w http.ResponseWriter, err error) {
	status := http.StatusBadGateway // an indexer or tracker failed
	code := acquire.CodeOf(err)
	switch code {
	case acquire.CodeInvalid, acquire.CodeNoLibrary:
		status = http.StatusBadRequest
	case acquire.CodeNotFound:
		status = http.StatusNotFound
	case acquire.CodeState:
		status = http.StatusConflict
	case acquire.CodeQuota, acquire.CodeDiskFull:
		status = http.StatusInsufficientStorage
	case acquire.CodeUnsupported:
		status = http.StatusUnprocessableEntity
	case acquire.CodeImport:
		status = http.StatusInternalServerError
	}
	msg := err.Error()
	var ae *acquire.Error
	if errors.As(err, &ae) {
		msg = ae.Msg
	}
	if code == "" {
		code = "error"
	}
	writeJSON(w, status, map[string]string{"error": msg, "code": code})
}

// indexerView adds has_api_key; the key itself never leaves (A-7).
type indexerView struct {
	contracts.Indexer
	HasAPIKey bool `json:"has_api_key"`
}

func (s *Server) indexerView(ix contracts.Indexer) indexerView {
	return indexerView{Indexer: ix.Public(), HasAPIKey: s.acquire.HasAPIKey(ix.ID)}
}

type acqView struct {
	Settings acquire.Settings `json:"settings"`
	Usage    acquire.Usage    `json:"usage"`
	Port     uint16           `json:"port"`
	// Parser reports whether the lain-parser model answers.
	Parser bool `json:"parser"`
}

func (s *Server) handleAcqSettingsGet(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	writeJSON(w, http.StatusOK, acqView{Settings: s.acquire.Settings(), Usage: s.acquire.Usage(), Port: s.acquire.Port(), Parser: s.parserHealthy()})
}

func (s *Server) parserHealthy() bool {
	provs, _, err := s.reg.Ordered(contracts.CapReleaseParse)
	if err != nil {
		return false
	}
	for _, p := range provs {
		if p.ID() == release.ModelID {
			return p.Health() == nil
		}
	}
	return false
}

func (s *Server) handleAcqSettingsPut(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	in := s.acquire.Settings()
	if !s.decode(w, r, &in) {
		return
	}
	out, err := s.acquire.SetSettings(in)
	if err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, acqView{Settings: out, Usage: s.acquire.Usage(), Port: s.acquire.Port(), Parser: s.parserHealthy()})
}

func (s *Server) handleAcqIndexers(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	out := []indexerView{}
	for _, ix := range s.acquire.Indexers() {
		out = append(out, s.indexerView(ix))
	}
	writeJSON(w, http.StatusOK, map[string]any{"indexers": out})
}

func (s *Server) handleAcqIndexerCreate(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	var in acquire.IndexerInput
	if !s.decode(w, r, &in) {
		return
	}
	ix, err := s.acquire.CreateIndexer(in)
	if err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusCreated, s.indexerView(ix))
}

func (s *Server) handleAcqIndexerPatch(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	var in acquire.IndexerInput
	if !s.decode(w, r, &in) {
		return
	}
	ix, err := s.acquire.UpdateIndexer(r.PathValue("id"), in)
	if err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, s.indexerView(ix))
}

func (s *Server) handleAcqIndexerDelete(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	if err := s.acquire.DeleteIndexer(r.PathValue("id")); err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"removed": true})
}

// handleAcqIndexerTest answers 200 with the recorded health either way:
// a failing indexer is a result of the test, not a failed request.
func (s *Server) handleAcqIndexerTest(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	ix, err := s.acquire.TestIndexer(r.PathValue("id"))
	if acquire.CodeOf(err) == acquire.CodeNotFound {
		writeAcqErr(w, err)
		return
	}
	resp := map[string]any{"indexer": s.indexerView(ix), "ok": err == nil}
	if err != nil {
		resp["error"] = ix.LastError
	}
	writeJSON(w, http.StatusOK, resp)
}

func (s *Server) handleAcqSearch(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	q := r.URL.Query()
	in := acquire.SearchQuery{Query: q.Get("q"), Kind: q.Get("kind"), Season: atoiQuery(r, "season"), Episode: atoiQuery(r, "episode"), Year: atoiQuery(r, "year")}
	if ids := q.Get("indexers"); ids != "" {
		in.IndexerIDs = strings.Split(ids, ",")
	}
	out, err := s.acquire.Search(in)
	if err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, out)
}

func (s *Server) handleAcqParse(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	name := r.URL.Query().Get("name")
	if strings.TrimSpace(name) == "" || len(name) > 1024 {
		writeAcqErr(w, &acquire.Error{Code: acquire.CodeInvalid, Msg: "name required (up to 1024 bytes)"})
		return
	}
	writeJSON(w, http.StatusOK, s.acquire.Parse(name, r.URL.Query().Get("kind")))
}

func (s *Server) handleAcqGrabs(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	writeJSON(w, http.StatusOK, map[string]any{"grabs": s.acquire.Grabs(), "usage": s.acquire.Usage()})
}

func (s *Server) handleAcqGrab(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	var in acquire.GrabInput
	if !s.decode(w, r, &in) {
		return
	}
	in.CreatedBy = v.UserID
	g, err := s.acquire.Grab(in)
	if err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusCreated, g)
}

func (s *Server) handleAcqGrabAction(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	id := r.PathValue("id")
	var (
		g   acquire.Grab
		err error
	)
	switch r.PathValue("action") {
	case "pause":
		g, err = s.acquire.Pause(id)
	case "resume":
		g, err = s.acquire.Resume(id)
	case "import":
		g, err = s.acquire.RetryImport(id)
	default:
		writeAcqErr(w, &acquire.Error{Code: acquire.CodeNotFound, Msg: "unknown action"})
		return
	}
	if err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, g)
}

func (s *Server) handleAcqGrabDelete(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	if err := s.acquire.Remove(r.PathValue("id"), r.URL.Query().Get("data") == "1"); err != nil {
		writeAcqErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"removed": true})
}
