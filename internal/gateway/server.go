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
	"strconv"
	"strings"
	"sync"
	"time"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/auth"
	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/kv"
	"github.com/enrell/lain/internal/plugins/catalog"
	"github.com/enrell/lain/internal/plugins/ingest"
	"github.com/enrell/lain/internal/plugins/metadata"
	"github.com/enrell/lain/internal/plugins/playback"
	"github.com/enrell/lain/internal/plugins/search"
	"github.com/enrell/lain/internal/plugins/source"
	"github.com/enrell/lain/internal/plugins/thumbnail"
	"github.com/enrell/lain/internal/plugins/userstate"
	"github.com/enrell/lain/internal/store"
	"github.com/enrell/lain/internal/webui"
)

// Server wires the composition to HTTP.
type Server struct {
	reg    *core.Registry
	auth   *auth.Service
	db     *bolt.DB
	st     *store.Dir
	cat    *catalog.Service
	ustate *userstate.Service
	libs   *LibraryStore
	mux    *http.ServeMux
	ver    string

	scanMu sync.Mutex
	scan   ScanStatus
}

// Close releases the database handle.
func (s *Server) Close() error { return s.db.Close() }

// ScanStatus is the observable scan state.
type ScanStatus struct {
	State      string               `json:"state"`
	StartedAt  int64                `json:"started_at,omitempty"`
	FinishedAt int64                `json:"finished_at,omitempty"`
	Stats      *contracts.ScanStats `json:"stats,omitempty"`
	Error      string               `json:"error,omitempty"`
}

// New builds the server over a data dir, registering built-ins. The
// database is opened here and owned by the server (see Close).
func New(dataDir, ver string) (*Server, error) {
	db, err := kv.Open(dataDir)
	if err != nil {
		return nil, err
	}
	st, err := store.New(dataDir)
	if err != nil {
		db.Close()
		return nil, err
	}
	if err := kv.ImportLegacy(db, st); err != nil {
		db.Close()
		return nil, fmt.Errorf("legacy import: %w", err)
	}
	a, err := auth.New(db)
	if err != nil {
		db.Close()
		return nil, err
	}
	cat, err := catalog.New(db)
	if err != nil {
		db.Close()
		return nil, err
	}
	ustate, err := userstate.New(db)
	if err != nil {
		db.Close()
		return nil, err
	}
	comp := core.DefaultComposition()
	saved := &core.Composition{}
	if err := st.Load("composition.json", saved); err == nil && len(saved.Bindings) > 0 {
		comp = saved
		if moved := comp.MigrateProviderIDs(); len(moved) > 0 {
			fmt.Printf("lain: migrated retired providers: %v\n", moved)
		}
		if added := comp.Upgrade(core.DefaultComposition()); len(added) > 0 {
			fmt.Printf("lain: new capabilities from defaults: %v\n", added)
		}
	}
	reg := core.NewRegistry(comp)
	reg.Register(source.Provider{})
	reg.Register(identifyAnimeShim{})
	reg.Register(identifyGenericShim{})
	reg.Register(cat)
	reg.Register(ustate)
	reg.Register(searchProvider{cat: cat})
	reg.Register(playback.Planner{})
	reg.Register(thumbnail.New(filepath.Join(dataDir, "thumbnails")))
	reg.Register(metadata.NFO{})
	reg.Register(metadata.NewKitsu())
	reg.Register(metadata.NewAniList())
	reg.Register(metadata.NewJikan())
	runner := &ingest.Runner{Reg: reg, Cat: cat}
	reg.Register(runner)
	if err := comp.Validate(knownSet(reg)); err != nil {
		db.Close()
		return nil, fmt.Errorf("composition: %w", err)
	}
	// Persist the effective composition (first boot writes defaults).
	if err := st.Save("composition.json", reg.Composition()); err != nil {
		db.Close()
		return nil, err
	}
	s := &Server{reg: reg, auth: a, db: db, st: st, cat: cat, ustate: ustate, libs: &LibraryStore{db: db}, mux: http.NewServeMux(), ver: ver, scan: ScanStatus{State: "idle"}}
	s.routes()
	s.routesEnrich()
	s.routesThumbnail()
	// The web UI is the least specific pattern: API, health and media
	// routes registered above keep winning their paths.
	webui.Mount(s.mux)
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
func (s *Server) userOf(r *http.Request) (auth.Verified, bool) {
	tok := ""
	if h := r.Header.Get("Authorization"); strings.HasPrefix(h, "Bearer ") {
		tok = strings.TrimPrefix(h, "Bearer ")
	} else {
		tok = r.URL.Query().Get("token")
	}
	if tok == "" {
		return auth.Verified{}, false
	}
	v, err := s.auth.Verify(tok)
	if err != nil {
		return auth.Verified{}, false
	}
	return v, true
}

func (s *Server) requireAuth(next func(http.ResponseWriter, *http.Request, auth.Verified)) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		v, ok := s.userOf(r)
		if !ok {
			writeErr(w, http.StatusUnauthorized, "unauthorized")
			return
		}
		next(w, r, v)
	}
}

// requireAdmin gates operator functions: libraries, scans, plugins,
// user administration.
func (s *Server) requireAdmin(next func(http.ResponseWriter, *http.Request, auth.Verified)) http.HandlerFunc {
	return s.requireAuth(func(w http.ResponseWriter, r *http.Request, v auth.Verified) {
		if v.Role != auth.RoleAdmin {
			writeErr(w, http.StatusForbidden, "admin required")
			return
		}
		next(w, r, v)
	})
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
	m.HandleFunc("PATCH /api/me/password", s.requireAuth(s.handleMyPassword))
	m.HandleFunc("GET /api/me/continue", s.requireAuth(s.handleContinue))

	m.HandleFunc("GET /api/users", s.requireAdmin(s.handleUsersList))
	m.HandleFunc("POST /api/users", s.requireAdmin(s.handleUserCreate))
	m.HandleFunc("PATCH /api/users/{id}", s.requireAdmin(s.handleUserPatch))

	m.HandleFunc("GET /api/libraries", s.requireAuth(s.handleLibsList))
	m.HandleFunc("POST /api/libraries", s.requireAdmin(s.handleLibCreate))
	m.HandleFunc("DELETE /api/libraries/{id}", s.requireAdmin(s.handleLibDelete))
	m.HandleFunc("POST /api/library/scan", s.requireAdmin(s.handleScanStart))
	m.HandleFunc("GET /api/library/scan", s.requireAuth(s.handleScanStatus))

	m.HandleFunc("GET /api/catalog", s.requireAuth(s.handleCatalogList))
	m.HandleFunc("GET /api/catalog/{id}", s.requireAuth(s.handleCatalogGet))
	m.HandleFunc("GET /api/search", s.requireAuth(s.handleSearch))

	m.HandleFunc("GET /api/items/{id}/playback", s.requireAuth(s.handlePlayback))
	m.HandleFunc("GET /api/items/{id}/stream", s.handleStream) // auth inside (query token)
	m.HandleFunc("PUT /api/items/{id}/progress", s.requireAuth(s.handleProgressPut))
	m.HandleFunc("GET /api/items/{id}/progress", s.requireAuth(s.handleProgressGet))

	m.HandleFunc("GET /api/plugins", s.requireAdmin(s.handlePlugins))
	m.HandleFunc("POST /api/plugins/swap", s.requireAdmin(s.handleSwap))
	m.HandleFunc("GET /api/admin/backup", s.requireAdmin(s.handleBackup))
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

func (s *Server) handleMe(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	u, ok := s.auth.Get(v.UserID)
	if !ok {
		writeErr(w, 404, "unknown user")
		return
	}
	writeJSON(w, 200, u)
}

func (s *Server) handleMyPassword(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	var in struct {
		Old string `json:"old"`
		New string `json:"new"`
	}
	if !s.decode(w, r, &in) {
		return
	}
	if err := s.auth.ChangePassword(v.UserID, in.Old, in.New); err != nil {
		writeErr(w, http.StatusBadRequest, err.Error())
		return
	}
	writeJSON(w, 200, map[string]string{"status": "ok"})
}

// handleContinue returns the user's recorded progress newest-first
// (continue-watching feed).
func (s *Server) handleContinue(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	writeJSON(w, 200, s.ustate.List(v.UserID))
}

func (s *Server) handleLibsList(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	libs, err := s.libs.List()
	if err != nil {
		writeErr(w, 500, err.Error())
		return
	}
	writeJSON(w, 200, libs)
}

func (s *Server) handleLibCreate(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	var in struct {
		Name string `json:"name"`
		Type string `json:"type"`
		Path string `json:"path"`
	}
	if !s.decode(w, r, &in) {
		return
	}
	lib, err := s.libs.Create(in.Name, in.Type, in.Path)
	if err != nil {
		writeErr(w, 400, err.Error())
		return
	}
	writeJSON(w, 201, lib)
}

func (s *Server) handleLibDelete(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	if err := s.libs.Delete(r.PathValue("id")); err != nil {
		writeErr(w, 404, err.Error())
		return
	}
	writeJSON(w, 200, map[string]string{"status": "ok"})
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
	libs, err := s.libs.List()
	if err != nil {
		return nil
	}
	return libs
}

func (s *Server) handleScanStart(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
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

func (s *Server) handleScanStatus(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
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

func pageParams(r *http.Request) contracts.PageParams {
	p := contracts.NormalizePage(atoiQuery(r, "limit"), atoiQuery(r, "offset"), r.URL.Query().Get("sort"))
	p.LibraryID = r.URL.Query().Get("library_id")
	return p
}

func atoiQuery(r *http.Request, key string) int {
	n, _ := strconv.Atoi(r.URL.Query().Get(key))
	return n
}

func (s *Server) handleCatalogList(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	page, err := s.cat.Page(pageParams(r))
	if err != nil {
		writeErr(w, 500, err.Error())
		return
	}
	writeJSON(w, 200, page)
}

func (s *Server) handleCatalogGet(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	it, ok := s.cat.Get(r.PathValue("id"))
	if !ok {
		writeErr(w, 404, "unknown item")
		return
	}
	writeJSON(w, 200, it)
}

func (s *Server) handleSearch(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	q := r.URL.Query().Get("q")
	kind := r.URL.Query().Get("kind")
	p := pageParams(r)
	out, _, err := s.reg.CallOne(contracts.CapSearchQuery, search.QueryInput{Q: q, Kind: kind, Limit: p.Limit, Offset: p.Offset, Sort: p.Sort})
	if err != nil {
		writeErr(w, 503, err.Error())
		return
	}
	writeJSON(w, 200, out)
}

func (s *Server) handlePlayback(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
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

func (s *Server) handleProgressPut(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	var in contracts.Progress
	defer r.Body.Close()
	if err := json.NewDecoder(r.Body).Decode(&in); err != nil {
		writeErr(w, 400, "invalid json")
		return
	}
	in.ItemID = r.PathValue("id")
	out, _, err := s.reg.CallOne(contracts.CapUserProgress, userStatePut(v.UserID, in))
	if err != nil {
		writeErr(w, 503, err.Error())
		return
	}
	writeJSON(w, 200, out)
}

func (s *Server) handleProgressGet(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	out, _, err := s.reg.CallOne(contracts.CapUserProgress, userStateGet(v.UserID, r.PathValue("id")))
	if err != nil {
		writeErr(w, 503, err.Error())
		return
	}
	writeJSON(w, 200, out)
}

func (s *Server) handlePlugins(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	writeJSON(w, 200, map[string]any{
		"composition":   s.reg.Composition().View(),
		"providers":     s.reg.Providers(),
		"provider_info": s.reg.ProviderInfos(),
		"events":        s.reg.Events(),
	})
}

func (s *Server) handleSwap(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
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

// handleBackup streams a consistent database snapshot (admin only).
// The snapshot comes from one read transaction, so backup works while
// scans and streams are running. Pair with GET /api/plugins (which
// carries the composition) for a complete backup set.
func (s *Server) handleBackup(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	w.Header().Set("Content-Type", "application/octet-stream")
	w.Header().Set("Content-Disposition", `attachment; filename="lain.db"`)
	if err := s.db.View(func(tx *bolt.Tx) error {
		_, err := tx.WriteTo(w)
		return err
	}); err != nil {
		// Headers already sent; nothing honest left to write.
		return
	}
}
