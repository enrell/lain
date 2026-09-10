// Package gateway is the trusted HTTP boundary: authentication,
// libraries, scan control, catalog reads, playback plans, byte serving
// with Range support, progress, and composition inspection/swap.
//
// Plugin code never touches net/http here: the gateway resolves
// catalog entries and user identity, then invokes providers through
// the registry. Media bytes are served from disk by the gateway after
// authorization; they never travel inside plugin calls.
package gateway

import (
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/enrell/lain/internal/auth"
	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/plugins/catalog"
	"github.com/enrell/lain/internal/plugins/ingest"
	"github.com/enrell/lain/internal/plugins/playback"
	"github.com/enrell/lain/internal/plugins/search"
	"github.com/enrell/lain/internal/plugins/source"
	"github.com/enrell/lain/internal/store"
)

// Server wires the composition to HTTP.
type Server struct {
	reg    *core.Registry
	auth   *auth.Service
	st     *store.Dir
	cat    *catalog.Service
	ustate progressBackend
	mux    *http.ServeMux
	ver    string

	libsMu sync.RWMutex
	libs   map[string]contracts.Library

	scanMu sync.Mutex
	scan   ScanStatus
}

// progressBackend is the userstate provider surface the gateway needs.
// *userstate.Service satisfies it and is registered as a provider.
type progressBackend interface {
	core.Provider
}

// ScanStatus is the observable scan state.
type ScanStatus struct {
	State      string               `json:"state"`
	StartedAt  int64                `json:"started_at,omitempty"`
	FinishedAt int64                `json:"finished_at,omitempty"`
	Stats      *contracts.ScanStats `json:"stats,omitempty"`
	Error      string               `json:"error,omitempty"`
}

// New builds the server over a data dir, registering built-ins.
func New(dataDir, ver string, ustate progressBackend) (*Server, error) {
	st, err := store.New(dataDir)
	if err != nil {
		return nil, err
	}
	a, err := auth.New(st)
	if err != nil {
		return nil, err
	}
	cat, err := catalog.New(st)
	if err != nil {
		return nil, err
	}
	comp := core.DefaultComposition()
	saved := &core.Composition{}
	if err := st.Load("composition.json", saved); err == nil && len(saved.Bindings) > 0 {
		comp = saved
	}
	reg := core.NewRegistry(comp)
	reg.Register(source.Provider{})
	reg.Register(identifyAnimeShim{})
	reg.Register(identifyGenericShim{})
	reg.Register(cat)
	reg.Register(ustate)
	reg.Register(searchProvider{cat: cat})
	reg.Register(playback.Planner{})
	runner := &ingest.Runner{Reg: reg, Cat: cat}
	reg.Register(runner)
	if err := comp.Validate(knownSet(reg)); err != nil {
		return nil, fmt.Errorf("composition: %w", err)
	}
	// Persist the effective composition (first boot writes defaults).
	if err := st.Save("composition.json", reg.Composition()); err != nil {
		return nil, err
	}
	s := &Server{reg: reg, auth: a, st: st, cat: cat, ustate: ustate, mux: http.NewServeMux(), ver: ver, libs: map[string]contracts.Library{}, scan: ScanStatus{State: "idle"}}
	var libs []contracts.Library
	if err := st.Load("libraries.json", &libs); err == nil {
		for _, l := range libs {
			s.libs[l.ID] = l
		}
	}
	if ustate == nil {
		return nil, fmt.Errorf("userstate backend required")
	}
	s.routes()
	return s, nil
}

func knownSet(reg *core.Registry) map[string]bool {
	m := map[string]bool{}
	for _, id := range reg.Providers() {
		m[id] = true
	}
	return m
}

// Handler returns the mux.
func (s *Server) Handler() http.Handler { return s.mux }

// Registry exposes the composition authority (diagnostics/swap).
func (s *Server) Registry() *core.Registry { return s.reg }

func writeJSON(w http.ResponseWriter, code int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(code)
	_ = json.NewEncoder(w).Encode(v)
}

func writeErr(w http.ResponseWriter, code int, msg string) {
	writeJSON(w, code, map[string]string{"error": msg})
}

func (s *Server) decode(w http.ResponseWriter, r *http.Request, v any) bool {
	defer r.Body.Close()
	if err := json.NewDecoder(r.Body).Decode(v); err != nil {
		writeErr(w, http.StatusBadRequest, "invalid json")
		return false
	}
	return true
}

// userOf verifies Bearer or ?token= (media-element fallback, ).
func (s *Server) userOf(r *http.Request) (string, bool) {
	tok := ""
	if h := r.Header.Get("Authorization"); strings.HasPrefix(h, "Bearer ") {
		tok = strings.TrimPrefix(h, "Bearer ")
	} else {
		tok = r.URL.Query().Get("token")
	}
	if tok == "" {
		return "", false
	}
	id, err := s.auth.Verify(tok)
	if err != nil {
		return "", false
	}
	return id, true
}

func (s *Server) requireAuth(next func(http.ResponseWriter, *http.Request, string)) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		id, ok := s.userOf(r)
		if !ok {
			writeErr(w, http.StatusUnauthorized, "unauthorized")
			return
		}
		next(w, r, id)
	}
}

func (s *Server) routes() {
	m := s.mux
	m.HandleFunc("GET /health", func(w http.ResponseWriter, r *http.Request) {
		writeJSON(w, 200, map[string]string{"status": "ok", "version": s.ver})
	})
	m.HandleFunc("GET /api/health", func(w http.ResponseWriter, r *http.Request) {
		writeJSON(w, 200, map[string]string{"status": "ok", "version": s.ver})
	})
	m.HandleFunc("GET /api/setup/status", s.handleSetupStatus)
	m.HandleFunc("POST /api/setup", s.handleSetup)
	m.HandleFunc("POST /api/auth/login", s.handleLogin)
	m.HandleFunc("GET /api/me", s.requireAuth(s.handleMe))

	m.HandleFunc("GET /api/libraries", s.requireAuth(s.handleLibsList))
	m.HandleFunc("POST /api/libraries", s.requireAuth(s.handleLibCreate))
	m.HandleFunc("POST /api/library/scan", s.requireAuth(s.handleScanStart))
	m.HandleFunc("GET /api/library/scan", s.requireAuth(s.handleScanStatus))

	m.HandleFunc("GET /api/catalog", s.requireAuth(s.handleCatalogList))
	m.HandleFunc("GET /api/catalog/{id}", s.requireAuth(s.handleCatalogGet))
	m.HandleFunc("GET /api/search", s.requireAuth(s.handleSearch))

	m.HandleFunc("GET /api/items/{id}/playback", s.requireAuth(s.handlePlayback))
	m.HandleFunc("GET /api/items/{id}/stream", s.handleStream) // auth inside (query token)
	m.HandleFunc("PUT /api/items/{id}/progress", s.requireAuth(s.handleProgressPut))
	m.HandleFunc("GET /api/items/{id}/progress", s.requireAuth(s.handleProgressGet))

	m.HandleFunc("GET /api/plugins", s.requireAuth(s.handlePlugins))
	m.HandleFunc("POST /api/plugins/swap", s.requireAuth(s.handleSwap))
}

func (s *Server) handleSetupStatus(w http.ResponseWriter, r *http.Request) {
	writeJSON(w, 200, map[string]bool{"setup_required": !s.auth.HasUsers()})
}

func (s *Server) handleSetup(w http.ResponseWriter, r *http.Request) {
	var in struct {
		Username string `json:"username"`
		Password string `json:"password"`
	}
	if !s.decode(w, r, &in) {
		return
	}
	u, err := s.auth.Setup(in.Username, in.Password)
	if err != nil {
		writeErr(w, http.StatusBadRequest, err.Error())
		return
	}
	writeJSON(w, 201, u)
}

func (s *Server) handleLogin(w http.ResponseWriter, r *http.Request) {
	var in struct {
		Username string `json:"username"`
		Password string `json:"password"`
	}
	if !s.decode(w, r, &in) {
		return
	}
	tok, err := s.auth.Login(in.Username, in.Password)
	if err != nil {
		writeErr(w, http.StatusUnauthorized, err.Error())
		return
	}
	writeJSON(w, 200, map[string]string{"token": tok})
}

func (s *Server) handleMe(w http.ResponseWriter, r *http.Request, userID string) {
	u, ok := s.auth.Get(userID)
	if !ok {
		writeErr(w, 404, "unknown user")
		return
	}
	writeJSON(w, 200, u)
}

func (s *Server) persistLibs() error {
	s.libsMu.RLock()
	list := make([]contracts.Library, 0, len(s.libs))
	for _, l := range s.libs {
		list = append(list, l)
	}
	s.libsMu.RUnlock()
	return s.st.Save("libraries.json", list)
}

func (s *Server) handleLibsList(w http.ResponseWriter, r *http.Request, _ string) {
	s.libsMu.RLock()
	list := make([]contracts.Library, 0, len(s.libs))
	for _, l := range s.libs {
		list = append(list, l)
	}
	s.libsMu.RUnlock()
	writeJSON(w, 200, list)
}

func (s *Server) handleLibCreate(w http.ResponseWriter, r *http.Request, _ string) {
	var in struct {
		Name string `json:"name"`
		Type string `json:"type"`
		Path string `json:"path"`
	}
	if !s.decode(w, r, &in) {
		return
	}
	if in.Name == "" || in.Path == "" {
		writeErr(w, 400, "name and path required")
		return
	}
	fi, err := os.Stat(in.Path)
	if err != nil || !fi.IsDir() {
		writeErr(w, 400, "path is not a readable directory")
		return
	}
	if in.Type == "" {
		in.Type = "anime"
	}
	lib := contracts.Library{
		ID: "lib-" + shortID(in.Path), Name: in.Name, Type: in.Type,
		Path: in.Path, Source: source.ID, CreatedAt: time.Now().Unix(),
	}
	s.libsMu.Lock()
	s.libs[lib.ID] = lib
	s.libsMu.Unlock()
	if err := s.persistLibs(); err != nil {
		writeErr(w, 500, err.Error())
		return
	}
	writeJSON(w, 201, lib)
}

func shortID(s string) string {
	h := 0
	for i := 0; i < len(s); i++ {
		h = h*31 + int(s[i])
	}
	if h < 0 {
		h = -h
	}
	return fmt.Sprintf("%08x", h%0xffffffff)
}

func (s *Server) libList() []contracts.Library {
	s.libsMu.RLock()
	defer s.libsMu.RUnlock()
	out := make([]contracts.Library, 0, len(s.libs))
	for _, l := range s.libs {
		out = append(out, l)
	}
	return out
}

func (s *Server) handleScanStart(w http.ResponseWriter, r *http.Request, _ string) {
	s.scanMu.Lock()
	if s.scan.State == "running" {
		s.scanMu.Unlock()
		writeErr(w, 409, "scan already running")
		return
	}
	s.scan = ScanStatus{State: "running", StartedAt: time.Now().Unix()}
	s.scanMu.Unlock()
	go s.runScan()
	writeJSON(w, 202, map[string]string{"state": "running"})
}

func (s *Server) handleScanStatus(w http.ResponseWriter, r *http.Request, _ string) {
	s.scanMu.Lock()
	defer s.scanMu.Unlock()
	writeJSON(w, 200, s.scan)
}

func (s *Server) runScan() {
	libs := s.libList()
	out, _, err := s.reg.CallOne(contracts.CapIngestScan, ingest.ScanInput{Libraries: libs})
	s.scanMu.Lock()
	defer s.scanMu.Unlock()
	if err != nil {
		s.scan = ScanStatus{State: "error", StartedAt: s.scan.StartedAt, FinishedAt: time.Now().Unix(), Error: err.Error()}
		return
	}
	stats := out.(contracts.ScanStats)
	s.scan = ScanStatus{State: "done", StartedAt: s.scan.StartedAt, FinishedAt: stats.FinishedAt, Stats: &stats}
}

func (s *Server) handleCatalogList(w http.ResponseWriter, r *http.Request, _ string) {
	items := s.cat.Search("", "")
	writeJSON(w, 200, items)
}

func (s *Server) handleCatalogGet(w http.ResponseWriter, r *http.Request, _ string) {
	it, ok := s.cat.Get(r.PathValue("id"))
	if !ok {
		writeErr(w, 404, "unknown item")
		return
	}
	writeJSON(w, 200, it)
}

func (s *Server) handleSearch(w http.ResponseWriter, r *http.Request, _ string) {
	q := r.URL.Query().Get("q")
	kind := r.URL.Query().Get("kind")
	out, _, err := s.reg.CallOne(contracts.CapSearchQuery, search.QueryInput{Q: q, Kind: kind})
	if err != nil {
		writeErr(w, 503, err.Error())
		return
	}
	writeJSON(w, 200, out)
}

func (s *Server) handlePlayback(w http.ResponseWriter, r *http.Request, _ string) {
	it, ok := s.cat.Get(r.PathValue("id"))
	if !ok {
		writeErr(w, 404, "unknown item")
		return
	}
	out, _, err := s.reg.CallOne(contracts.CapPlaybackPlan, playback.PlanInput{
		Request:  contracts.PlanRequest{ItemID: it.ID, Client: r.URL.Query().Get("client"), Network: r.URL.Query().Get("network")},
		FilePath: it.FilePath,
	})
	if err != nil {
		writeErr(w, 503, err.Error())
		return
	}
	writeJSON(w, 200, out)
}

// handleStream authorizes (header or ?token=) then serves bytes with
// Range support via ServeContent. The client never learns the layout
// beyond this endpoint.
func (s *Server) handleStream(w http.ResponseWriter, r *http.Request) {
	if _, ok := s.userOf(r); !ok {
		writeErr(w, http.StatusUnauthorized, "unauthorized")
		return
	}
	it, ok := s.cat.Get(r.PathValue("id"))
	if !ok {
		writeErr(w, 404, "unknown item")
		return
	}
	f, err := os.Open(it.FilePath)
	if err != nil {
		writeErr(w, 404, "file unavailable")
		return
	}
	defer f.Close()
	fi, err := f.Stat()
	if err != nil {
		writeErr(w, 500, err.Error())
		return
	}
	w.Header().Set("Content-Type", contentType(it.FilePath))
	w.Header().Set("Accept-Ranges", "bytes")
	http.ServeContent(w, r, filepath.Base(it.FilePath), fi.ModTime(), f)
}

func contentType(path string) string {
	switch strings.ToLower(strings.TrimPrefix(filepath.Ext(path), ".")) {
	case "mkv":
		return "video/x-matroska"
	case "mp4", "m4v":
		return "video/mp4"
	case "webm":
		return "video/webm"
	case "avi":
		return "video/x-msvideo"
	case "mp3":
		return "audio/mpeg"
	case "flac":
		return "audio/flac"
	case "ogg", "opus":
		return "audio/ogg"
	default:
		return "application/octet-stream"
	}
}

func (s *Server) handleProgressPut(w http.ResponseWriter, r *http.Request, userID string) {
	var in contracts.Progress
	defer r.Body.Close()
	if err := json.NewDecoder(r.Body).Decode(&in); err != nil {
		writeErr(w, 400, "invalid json")
		return
	}
	in.ItemID = r.PathValue("id")
	out, _, err := s.reg.CallOne(contracts.CapUserProgress, userStatePut(userID, in))
	if err != nil {
		writeErr(w, 503, err.Error())
		return
	}
	writeJSON(w, 200, out)
}

func (s *Server) handleProgressGet(w http.ResponseWriter, r *http.Request, userID string) {
	out, _, err := s.reg.CallOne(contracts.CapUserProgress, userStateGet(userID, r.PathValue("id")))
	if err != nil {
		writeErr(w, 503, err.Error())
		return
	}
	writeJSON(w, 200, out)
}

func (s *Server) handlePlugins(w http.ResponseWriter, r *http.Request, _ string) {
	writeJSON(w, 200, map[string]any{
		"composition": s.reg.Composition().View(),
		"providers":   s.reg.Providers(),
		"events":      s.reg.Events(),
	})
}

func (s *Server) handleSwap(w http.ResponseWriter, r *http.Request, _ string) {
	var in struct {
		Capability string   `json:"capability"`
		Providers  []string `json:"providers"`
		Generation uint64   `json:"generation"`
	}
	if !s.decode(w, r, &in) {
		return
	}
	gen, err := s.reg.Swap(in.Capability, in.Providers, in.Generation)
	if err != nil {
		if ce, ok := err.(*core.Error); ok {
			code := 400
			if ce.Code == "stale-generation" {
				code = 409
			} else if ce.Code == "dependency-unavailable" {
				code = 422
			}
			writeJSON(w, code, map[string]any{"error": ce.Msg, "code": ce.Code, "generation": gen})
			return
		}
		writeErr(w, 400, err.Error())
		return
	}
	if err := s.st.Save("composition.json", s.reg.Composition()); err != nil {
		writeErr(w, 500, err.Error())
		return
	}
	writeJSON(w, 200, map[string]any{"generation": gen, "composition": s.reg.Composition().View()})
}
