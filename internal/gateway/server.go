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
	"errors"
	"fmt"
	"io"
	"log/slog"
	"mime"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/auth"
	"github.com/enrell/lain/internal/component"
	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/downloads"
	"github.com/enrell/lain/internal/kv"
	"github.com/enrell/lain/internal/localplay"
	"github.com/enrell/lain/internal/plugins/backup"
	"github.com/enrell/lain/internal/plugins/catalog"
	"github.com/enrell/lain/internal/plugins/comic"
	"github.com/enrell/lain/internal/plugins/ingest"
	"github.com/enrell/lain/internal/plugins/list"
	"github.com/enrell/lain/internal/plugins/listlink"
	pluginlocalplay "github.com/enrell/lain/internal/plugins/localplay"
	"github.com/enrell/lain/internal/plugins/metadata"
	"github.com/enrell/lain/internal/plugins/playback"
	"github.com/enrell/lain/internal/plugins/probe"
	"github.com/enrell/lain/internal/plugins/search"
	"github.com/enrell/lain/internal/plugins/settings"
	"github.com/enrell/lain/internal/plugins/social"
	"github.com/enrell/lain/internal/plugins/source"
	"github.com/enrell/lain/internal/plugins/sourcewatch"
	"github.com/enrell/lain/internal/plugins/theme"
	"github.com/enrell/lain/internal/plugins/thumbnail"
	"github.com/enrell/lain/internal/plugins/transcode"
	"github.com/enrell/lain/internal/plugins/userstate"
	"github.com/enrell/lain/internal/store"
	"github.com/enrell/lain/internal/webui"
)

// Server wires the composition to HTTP.
type Server struct {
	reg            *core.Registry
	auth           *auth.Service
	db             *bolt.DB
	st             *store.Dir
	avatarDir      string
	libs           *LibraryStore
	settings       settingsResolver
	mux            *http.ServeMux
	ver            string
	themePath      string
	transcode      *transcode.Transcoder
	probeTranscode func(contracts.TranscodeSettings) transcode.CapabilitiesReport

	// autoEnrich runs metadata enrichment for overlay-less items after
	// every scan (default behavior). Disable with SetAutoEnrich(false)
	// or LAIN_AUTO_ENRICH=0.
	autoEnrich bool

	// log is the structured server logger (D-026). Discard by
	// default so tests stay silent; serve installs JSON stdout via
	// SetLogger, tests inject buffers the same way.
	log atomic.Pointer[slog.Logger]
	// reqSeq numbers access-log request ids (server-side only).
	reqSeq atomic.Uint64

	scanMu sync.Mutex
	scan   ScanStatus

	// watch reconciles the catalog on file events through the
	// lain.source.watch@1 provider (D-068/D-076): the poll loop starts
	// on StartWatcher; watchDone closes it on shutdown.
	watchProv     *sourcewatch.Provider
	watchStarted  bool
	watchDone     chan struct{}
	watchClose    sync.Once     // watchDone is closed once, never reassigned
	watchDebounce time.Duration // test seam: zero uses the default

	// local spawns mpv/VLC on this machine for loopback browsers (D-072).
	local *localplay.Manager

	// listSync re-imports linked list accounts on a fixed interval
	// (D-081): an immediate catch-up plus one pass per interval, closed
	// on shutdown. stateKey signs the OAuth link states the callback
	// verifies (D-080).
	stateKey         []byte
	listSyncStarted  bool
	listSyncDone     chan struct{}
	listSyncClose    sync.Once     // listSyncDone is closed once, never reassigned
	listSyncInterval time.Duration // test seam: zero uses the default

	// components provisions external providers from <data-dir>/plugins/
	// (D-077). Started in NewWithOptions; nil only on spawn failure.
	components *component.Provisioner

	// downloads fetches URLs onto server disk (downloads.go); started in
	// NewWithOptions, stopped in Close.
	downloads *downloads.Manager
}

// Close stops background transforms and the library watcher before
// releasing the database.
func (s *Server) Close() error {
	if s.components != nil {
		s.components.Close()
	}
	if s.downloads != nil {
		s.downloads.Close()
	}
	// The done channels are written once in NewWithOptions and only ever
	// closed here. watchLoop and listSyncLoop read them straight from
	// their select, so reassigning the field after construction would
	// race the reader — the Once absorbs a second Close (test cleanup)
	// without touching the field. The started flags belong to scanMu and
	// are deliberately not read here.
	if s.watchDone != nil {
		s.watchClose.Do(func() { close(s.watchDone) })
	}
	if s.listSyncDone != nil {
		s.listSyncClose.Do(func() { close(s.listSyncDone) })
	}
	if s.reg != nil {
		s.reg.Each(func(p core.Provider) {
			if c, ok := p.(io.Closer); ok {
				_ = c.Close()
			}
		})
	}
	if s.local != nil {
		s.local.Close()
	}
	if s.transcode != nil {
		_ = s.transcode.Close()
	}
	return s.db.Close()
}

// Options controls bounded runtime resources while New retains stable
// defaults for callers and tests.
type Options struct {
	TranscodeCacheBytes int64
	TranscodeQueueSize  int
	// AniListClientID/Secret seed the operator's AniList OAuth app from
	// flags or env (D-080). They only fill an empty configuration — a
	// saved admin-UI choice always wins.
	AniListClientID     string
	AniListClientSecret string
	// transcodeProbe is a deterministic test seam. Production leaves it nil
	// and uses the transcoder's real one-frame capability probe.
	transcodeProbe func(contracts.TranscodeSettings) transcode.CapabilitiesReport
}

// ScanStatus is the observable scan state.
type ScanStatus struct {
	State      string               `json:"state"`
	StartedAt  int64                `json:"started_at,omitempty"`
	FinishedAt int64                `json:"finished_at,omitempty"`
	Stats      *contracts.ScanStats `json:"stats,omitempty"`
	Error      string               `json:"error,omitempty"`
	// Trigger is "manual" for operator scans and "watch" for
	// filesystem-event scans (D-068); empty on older statuses.
	Trigger string `json:"trigger,omitempty"`
}

// New builds the server over a data dir, registering built-ins. The
// database is opened here and owned by the server (see Close).
func New(dataDir, ver string) (*Server, error) {
	return NewWithOptions(dataDir, ver, Options{})
}

// NewWithOptions builds the server with explicit runtime bounds.
func NewWithOptions(dataDir, ver string, opts Options) (*Server, error) {
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
	lst, err := list.New(db)
	if err != nil {
		db.Close()
		return nil, err
	}
	soc, err := social.New(db)
	if err != nil {
		db.Close()
		return nil, err
	}
	stateKey, err := loadOrCreateStateKey(db)
	if err != nil {
		db.Close()
		return nil, fmt.Errorf("link state key: %w", err)
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
	reg.Register(identifyComicShim{})
	reg.Register(identifyAnimeShim{})
	reg.Register(identifyGenericShim{})
	reg.Register(&comic.Provider{})
	reg.Register(cat)
	reg.Register(ustate)
	reg.Register(lst)
	reg.Register(soc)
	reg.Register(listlink.NewAniList())
	reg.Register(searchProvider{reg: reg})
	reg.Register(playback.Planner{})
	reg.Register(probe.Provider{})
	reg.Register(thumbnail.New(filepath.Join(dataDir, "thumbnails")))
	tr := transcode.NewConfigured(filepath.Join(dataDir, "transcodes"), transcode.Config{
		MaxCacheBytes: opts.TranscodeCacheBytes,
		QueueSize:     opts.TranscodeQueueSize,
	})
	reg.Register(tr)
	reg.Register(metadata.NFO{})
	reg.Register(metadata.NewKitsu())
	reg.Register(metadata.NewAniList())
	reg.Register(metadata.NewJikan())
	reg.Register(metadata.NewTVMaze())
	reg.Register(settings.Provider{DB: db})
	reg.Register(theme.Provider{})
	reg.Register(metadata.NewEnricher(reg, db))
	local := localplay.New()
	reg.Register(&pluginlocalplay.Provider{Reg: reg, Mgr: local})
	reg.Register(backup.Provider{})
	watchProv := sourcewatch.NewWatcherProvider()
	reg.Register(watchProv)
	runner := &ingest.Runner{Reg: reg}
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
	probeTranscode := tr.Probe
	if opts.transcodeProbe != nil {
		probeTranscode = opts.transcodeProbe
	}
	s := &Server{reg: reg, auth: a, db: db, st: st, libs: &LibraryStore{db: db}, mux: http.NewServeMux(), ver: ver, transcode: tr, probeTranscode: probeTranscode, autoEnrich: true, scan: ScanStatus{State: "idle"}, local: local, watchProv: watchProv, watchDone: make(chan struct{}), stateKey: stateKey, listSyncDone: make(chan struct{})}
	s.settings = settingsResolver{s: s}
	s.avatarDir = filepath.Join(dataDir, "avatars")
	// First boot adopts CLI bounds as the saved policy; later boots keep
	// the operator's admin-UI choices (D-045).
	bootSettings := contracts.DefaultTranscodeSettings()
	if opts.TranscodeCacheBytes > 0 {
		bootSettings.CacheBytes = opts.TranscodeCacheBytes
	}
	if opts.TranscodeQueueSize > 0 {
		bootSettings.QueueSize = opts.TranscodeQueueSize
	}
	hasSettings, err := s.settings.HasTranscode()
	if err != nil {
		db.Close()
		return nil, fmt.Errorf("transcode settings: %w", err)
	}
	if !hasSettings {
		bootSettings = withAutomaticHardware(bootSettings, s.probeTranscode(bootSettings).Hardware)
	}
	if err := s.settings.Ensure(bootSettings); err != nil {
		db.Close()
		return nil, fmt.Errorf("transcode settings: %w", err)
	}
	// Operator-provided OAuth client (D-080): flags/env seed only an
	// empty integrations document — saved settings are authoritative.
	if opts.AniListClientID != "" {
		if err := s.settings.EnsureIntegrations(contracts.IntegrationSettings{
			AniListClientID:     opts.AniListClientID,
			AniListClientSecret: opts.AniListClientSecret,
		}); err != nil {
			db.Close()
			return nil, fmt.Errorf("integration settings: %w", err)
		}
	}
	dl, err := downloads.NewManager(db, downloads.DefaultSettings(dataDir))
	if err != nil {
		db.Close()
		return nil, fmt.Errorf("downloads: %w", err)
	}
	dl.OnDone = s.downloadDone
	dl.Start()
	s.downloads = dl
	s.routes()
	s.routesEnrich()
	s.routesThumbnail()
	s.routesReader()
	s.routesProfile()
	s.routesTranscode()
	s.routesList()
	s.routesDownloads()
	s.routesSocial()
	// The web UI is the least specific pattern: API, health and media
	// routes registered above keep winning their paths.
	webui.Mount(s.mux)
	// Component-mode provisioning (D-077): manifests dropped into
	// <data-dir>/plugins/ spawn supervised provider processes. A
	// provisioning failure degrades to "no external components" — it
	// never blocks boot.
	s.components = component.NewProvisioner(filepath.Join(dataDir, "plugins"), reg, s.logger())
	if err := s.components.Start(); err != nil {
		s.logger().Warn("component provisioning disabled", "err", err.Error())
	}
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
func (s *Server) Handler() http.Handler { return s.withAccessLog(s.mux) }

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

// userOf verifies Bearer or ?token= (media-element fallback).
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
	m.HandleFunc("GET /api/theme", s.handleTheme)
	m.HandleFunc("GET /api/setup/status", s.handleSetupStatus)
	m.HandleFunc("POST /api/setup", s.handleSetup)
	m.HandleFunc("POST /api/auth/login", s.handleLogin)
	m.HandleFunc("GET /api/me", s.requireAuth(s.handleMe))
	m.HandleFunc("PATCH /api/me/preferences", s.requireAuth(s.handleMyPreferences))
	m.HandleFunc("PATCH /api/me/password", s.requireAuth(s.handleMyPassword))
	m.HandleFunc("GET /api/me/continue", s.requireAuth(s.handleContinue))

	m.HandleFunc("GET /api/users", s.requireAdmin(s.handleUsersList))
	m.HandleFunc("POST /api/users", s.requireAdmin(s.handleUserCreate))
	m.HandleFunc("PATCH /api/users/{id}", s.requireAdmin(s.handleUserPatch))

	m.HandleFunc("GET /api/libraries", s.requireAuth(s.handleLibsList))
	m.HandleFunc("POST /api/libraries", s.requireAdmin(s.handleLibCreate))
	m.HandleFunc("DELETE /api/libraries/{id}", s.requireAdmin(s.handleLibDelete))
	m.HandleFunc("GET /api/browse", s.requireAdmin(s.handleBrowse))
	m.HandleFunc("POST /api/library/scan", s.requireAdmin(s.handleScanStart))
	m.HandleFunc("GET /api/library/scan", s.requireAuth(s.handleScanStatus))

	m.HandleFunc("GET /api/catalog", s.requireAuth(s.handleCatalogList))
	m.HandleFunc("GET /api/catalog/{id}", s.requireAuth(s.handleCatalogGet))
	m.HandleFunc("GET /api/catalog/{id}/episodes", s.requireAuth(s.handleCatalogEpisodes))
	m.HandleFunc("GET /api/search", s.requireAuth(s.handleSearch))

	m.HandleFunc("DELETE /api/items/{id}", s.requireAdmin(s.handleItemDelete))
	m.HandleFunc("GET /api/items/{id}/playback", s.requireAuth(s.handlePlayback))
	m.HandleFunc("GET /api/items/{id}/stream", s.handleStream) // auth inside (query token)
	m.HandleFunc("PUT /api/items/{id}/progress", s.requireAuth(s.handleProgressPut))
	m.HandleFunc("GET /api/items/{id}/progress", s.requireAuth(s.handleProgressGet))
	m.HandleFunc("GET /api/localplay", s.requireAuth(s.handleLocalPlay))
	m.HandleFunc("POST /api/items/{id}/play-local", s.requireAuth(s.handlePlayLocal))

	m.HandleFunc("GET /api/plugins", s.requireAdmin(s.handlePlugins))
	m.HandleFunc("POST /api/plugins/swap", s.requireAdmin(s.handleSwap))
	m.HandleFunc("POST /api/plugins/withdraw", s.requireAdmin(s.handleWithdraw))
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
		// Username only: passwords never enter a log line.
		s.logger().Warn("login failed", "req", reqIDOf(r), "username", in.Username)
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

func (s *Server) handleMyPreferences(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	var in struct {
		PreferredLanguage *string `json:"preferred_language"`
	}
	if !s.decode(w, r, &in) {
		return
	}
	if in.PreferredLanguage == nil {
		writeErr(w, http.StatusBadRequest, "preferred_language is required")
		return
	}
	if err := s.auth.SetPreferredLanguage(v.UserID, *in.PreferredLanguage); err != nil {
		writeErr(w, http.StatusBadRequest, err.Error())
		return
	}
	s.handleMe(w, r, v)
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
	list, err := s.ustateList(v.UserID)
	if err != nil {
		writeErr(w, 503, err.Error())
		return
	}
	writeJSON(w, 200, list)
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
	// The watch provider picks the new library up on its next poll —
	// the library set travels inside every poll input.
	writeJSON(w, 201, lib)
}

func (s *Server) handleLibDelete(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	id := r.PathValue("id")
	if err := s.libs.Delete(id); err != nil {
		writeErr(w, 404, err.Error())
		return
	}
	// A library record without its items is a leak: the scan reconciles
	// per root and this root is gone, so nothing would ever collect them.
	removed, err := s.catDeleteLibrary(id)
	if err != nil {
		s.logger().Error("library catalog cleanup failed", "req", reqIDOf(r), "library", id, "err", err.Error())
		writeErr(w, 500, "library removed, but its catalog entries could not be deleted")
		return
	}
	writeJSON(w, 200, map[string]any{"status": "ok", "items_removed": removed})
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
	s.scan = ScanStatus{State: "running", StartedAt: time.Now().Unix(), Trigger: "manual"}
	s.scanMu.Unlock()
	go s.runScan(s.libList(), "manual")
	writeJSON(w, 202, map[string]string{"state": "running"})
}

func (s *Server) handleScanStatus(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	s.scanMu.Lock()
	defer s.scanMu.Unlock()
	writeJSON(w, 200, s.scan)
}

func (s *Server) runScan(libs []contracts.Library, trigger string) {
	s.logger().Info("scan started", "libraries", len(libs), "trigger", trigger)
	out, _, err := s.reg.CallOne(contracts.CapIngestScan, ingest.ScanInput{Libraries: libs})
	s.scanMu.Lock()
	if err != nil {
		s.scan = ScanStatus{State: "error", StartedAt: s.scan.StartedAt, FinishedAt: time.Now().Unix(), Error: err.Error(), Trigger: trigger}
		s.scanMu.Unlock()
		s.logger().Error("scan failed", "err", err.Error(), "trigger", trigger)
		return
	}
	stats := out.(contracts.ScanStats)
	startedAt := s.scan.StartedAt
	s.scanMu.Unlock()
	// Enrichment runs outside the scan lock so status reads never block
	// behind a long backlog pass. State stays "running" throughout.
	enriched := 0
	if s.autoEnrichEnabled() {
		enriched = s.autoEnrichMissing()
	}
	stats.Enriched = enriched
	s.scanMu.Lock()
	defer s.scanMu.Unlock()
	s.scan = ScanStatus{State: "done", StartedAt: startedAt, FinishedAt: stats.FinishedAt, Stats: &stats, Trigger: trigger}
	s.logger().Info("scan done",
		"candidates", stats.Candidates, "identified", stats.Identified,
		"unidentified", stats.Unidentified, "migrated", stats.Migrated,
		"missing", stats.Missing, "restored", stats.Restored,
		"enriched", stats.Enriched, "errors", stats.Errors, "trigger", trigger)
}

// SetAutoEnrich toggles post-scan metadata enrichment (on by default).
func (s *Server) SetAutoEnrich(on bool) {
	s.scanMu.Lock()
	defer s.scanMu.Unlock()
	s.autoEnrich = on
}

func (s *Server) autoEnrichEnabled() bool {
	s.scanMu.Lock()
	defer s.scanMu.Unlock()
	return s.autoEnrich
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
	page, err := s.catPage(pageParams(r))
	if err != nil {
		writeErr(w, 500, err.Error())
		return
	}
	writeJSON(w, 200, page)
}

func (s *Server) handleCatalogGet(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	it, ok := s.catGet(r.PathValue("id"))
	if !ok {
		writeErr(w, 404, "unknown item")
		return
	}
	writeJSON(w, 200, it)
}

// handleCatalogEpisodes answers the title page: every file of the same
// title in watch order. The catalog owns the grouping rule and the
// client only renders it, so the detail page never depends on the
// search plugin (D-056).
func (s *Server) handleCatalogEpisodes(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	items, ok := s.catEpisodes(r.PathValue("id"))
	if !ok {
		writeErr(w, 404, "unknown item")
		return
	}
	writeJSON(w, 200, struct {
		Items []contracts.CatalogItem `json:"items"`
	}{Items: items})
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

func (s *Server) handlePlayback(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	it, ok := s.catGet(r.PathValue("id"))
	if !ok {
		writeErr(w, 404, "unknown item")
		return
	}
	// A catalog row can outlive its file between reconciliations (D-068):
	// say so in the plan instead of letting the client discover a bare
	// stream 404 after pressing play.
	if _, err := os.Stat(it.FilePath); err != nil {
		writeJSON(w, 200, struct {
			contracts.Plan
			DurationSec float64 `json:"duration_sec,omitempty"`
		}{Plan: contracts.Plan{
			Mode:      "unavailable",
			Available: false,
			Reason:    "the file is no longer on disk",
		}})
		return
	}
	client := r.URL.Query().Get("client")
	// The client reports what it can decode (D-058): an absent `caps`
	// means unknown and keeps the conservative browser rules, while a
	// present but empty `caps=` claims nothing — a decision to transcode.
	caps := contracts.ParseCapabilities(r.URL.Query()["caps"])
	settings := s.settings.Transcode()
	var mediaInfo *contracts.MediaInfo
	clientLower := strings.ToLower(client)
	if !strings.Contains(clientLower, "mpv") && !strings.Contains(clientLower, "desktop") {
		if probed, _, probeErr := s.reg.CallOne(contracts.CapMediaProbe, contracts.MediaProbeRequest{FilePath: it.FilePath}); probeErr == nil {
			if info, ok := probed.(contracts.MediaInfo); ok {
				mediaInfo = &info
			}
		} else {
			s.logger().Debug("media probe failed", "req", reqIDOf(r), "item", it.ID, "err", probeErr.Error())
		}
	}
	// Tone mapping is only promised when it is enabled and its probe
	// passed (D-031/D-042); otherwise HDR stays honestly unavailable.
	toneMap := s.toneMapAvailable(settings)
	out, _, err := s.reg.CallOne(contracts.CapPlaybackPlan, playback.PlanInput{
		Request: contracts.PlanRequest{
			ItemID:       it.ID,
			Client:       client,
			Network:      r.URL.Query().Get("network"),
			Capabilities: caps,
		},
		FilePath:  it.FilePath,
		MediaInfo: mediaInfo,
		ToneMap:   toneMap,
	})
	if err != nil {
		writeErr(w, 503, err.Error())
		return
	}
	plan, ok := out.(contracts.Plan)
	if !ok {
		writeErr(w, 500, "bad playback plan")
		return
	}
	// The planner advertises composition capability; the gateway keeps
	// the answer honest at runtime. Without a healthy transcode
	// provider the plan degrades to transcode-required instead of
	// pointing the client at an endpoint that can only 503.
	policy := s.auth.PlaybackPolicy(v.UserID)
	maxBitrate := effectiveBitrateLimit(policy.MaxBitrateKbps, settings.RemoteBitrateLimitKbps)
	// A capped account must not stream above its limit: when the probed
	// source bitrate exceeds the cap, a direct-play plan is downgraded to
	// a capped transcode (Jellyfin's remote bitrate limit).
	if maxBitrate > 0 && plan.Mode == "direct" && sourceBitrateKbps(mediaInfo) > maxBitrate {
		plan.Mode = "transcode"
	}
	if plan.Mode == "transcode" {
		switch {
		case !policy.AllowsVideoTranscode() && !policy.AllowsRemux():
			plan.Available = false
			plan.Mode = "transcode-required"
			plan.Reason = "transcoding is disabled for this user"
		default:
			if _, _, err := s.reg.Ordered(contracts.CapPlaybackTranscode); err != nil {
				plan.Available = false
				plan.Mode = "transcode-required"
				plan.Reason = "this container needs transcode for browser clients (no transcode provider installed)"
			} else if inspected, _, err := s.reg.CallOne(contracts.CapPlaybackTranscodeV3, contracts.TranscodeV3Request{
				Action: contracts.TranscodeInspectAction, FilePath: it.FilePath,
				Delivery: settings.DefaultDelivery, MaxBitrateKbps: maxBitrate,
				Settings: settings,
			}); err == nil {
				if status, ok := inspected.(contracts.TranscodeV3Status); ok {
					plan.Profile = status.Profile
					plan.Session = status.Session
					plan.State = status.State
					plan.Reasons = status.Reasons
				}
			}
		}
	}
	// The plan the client receives also carries the media's own length.
	// An HLS session is an EVENT playlist listing only the segments ffmpeg
	// has already written, so under MSE the element's duration is the
	// produced edge, not the episode: a seek bar drawn from it stops at
	// whatever has been encoded so far. The probed length is what lets the
	// player span the real timeline (and report progress honestly) — the
	// same fact Jellyfin's PlaybackInfo carries as RunTimeTicks. It is the
	// gateway's probe, so the plugin contract stays untouched (D-057).
	writeJSON(w, 200, struct {
		contracts.Plan
		DurationSec float64 `json:"duration_sec,omitempty"`
	}{Plan: plan, DurationSec: mediaDurationSec(mediaInfo)})
}

// toneMapAvailable reports whether a plan may promise HDR tone mapping:
// enabled in settings, not policy-disabled, and confirmed by the
// transcode capability probe.
func (s *Server) toneMapAvailable(settings contracts.TranscodeSettings) bool {
	return settings.ToneMapping && settings.ToneMappingMode != contracts.ToneMapModeNever &&
		s.probeTranscode(settings).ToneMapping
}

// mediaDurationSec is the probed media length in seconds, 0 when the
// probe did not run (mpv/desktop clients) or did not report one. A
// caller must treat 0 as unknown, never as "zero length".
func mediaDurationSec(info *contracts.MediaInfo) float64 {
	if info == nil || info.Duration <= 0 {
		return 0
	}
	return info.Duration
}

// handleStream authorizes (header or ?token=) then serves bytes with
// Range support via ServeContent. The client never learns the layout
// beyond this endpoint.
func (s *Server) handleStream(w http.ResponseWriter, r *http.Request) {
	if _, ok := s.userOf(r); !ok {
		writeErr(w, http.StatusUnauthorized, "unauthorized")
		return
	}
	it, ok := s.catGet(r.PathValue("id"))
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
	// ?download=1 asks a browser to save the original file (an offline
	// copy) instead of playing it inline.
	if r.URL.Query().Get("download") == "1" {
		w.Header().Set("Content-Disposition", mime.FormatMediaType("attachment", map[string]string{"filename": filepath.Base(it.FilePath)}))
	}
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
	// Scrobble only on the transition to completed: the reporter keeps
	// re-sending the final position, and each is not a new finish.
	prev, _ := s.ustateGet(v.UserID, in.ItemID)
	out, _, err := s.reg.CallOne(contracts.CapUserProgress, userStatePut(v.UserID, in))
	if err != nil {
		writeErr(w, 503, err.Error())
		return
	}
	if in.Completed && !prev.Completed {
		go s.scrobble(v.UserID, in.ItemID)
	}
	s.recordSocialProgress(v.UserID, prev, in)
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

// handleWithdraw removes a provider from serving without deleting its
// registration: bindings that referenced it fall back to remaining
// healthy providers, and an exactly-one binding with no alternative
// stays marked degraded rather than serving nothing silently.
func (s *Server) handleWithdraw(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	var in struct {
		Provider string `json:"provider"`
	}
	if !s.decode(w, r, &in) {
		return
	}
	if in.Provider == "" {
		writeErr(w, 400, "provider required")
		return
	}
	if err := s.reg.Withdraw(in.Provider); err != nil {
		if ce, ok := err.(*core.Error); ok {
			writeJSON(w, 400, map[string]any{"error": ce.Msg, "code": ce.Code})
			return
		}
		writeErr(w, 400, err.Error())
		return
	}
	if err := s.st.Save("composition.json", s.reg.Composition()); err != nil {
		writeErr(w, 500, err.Error())
		return
	}
	writeJSON(w, 200, map[string]any{"composition": s.reg.Composition().View()})
}

// handleBackup streams a backup artifact (admin only). The trusted
// core snapshots the live database to a temp file — one read
// transaction, so backup works while scans and streams run — and the
// lain.backup.create@1 provider owns what the artifact becomes (D-076).
// Pair with GET /api/plugins (which carries the composition) for a
// complete backup set.
func (s *Server) handleBackup(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	dir, err := os.MkdirTemp("", "lain-backup-*")
	if err != nil {
		writeErr(w, 500, err.Error())
		return
	}
	defer os.RemoveAll(dir)
	snapshot := filepath.Join(dir, "lain.db")
	if err := s.db.View(func(tx *bolt.Tx) error {
		f, err := os.Create(snapshot)
		if err != nil {
			return err
		}
		_, werr := tx.WriteTo(f)
		return errors.Join(werr, f.Close())
	}); err != nil {
		writeErr(w, 500, err.Error())
		return
	}
	comp, _ := json.Marshal(s.reg.Composition())
	out, _, err := s.reg.CallOne(contracts.CapBackupCreate, backup.CreateInput{
		SnapshotPath: snapshot,
		OutDir:       dir,
		Docs:         map[string]json.RawMessage{"composition.json": comp},
	})
	if err != nil {
		writeErr(w, 503, err.Error())
		return
	}
	artifact, ok := out.(backup.CreateOutput)
	if !ok || artifact.Path == "" {
		writeErr(w, 500, "backup provider returned a bad shape")
		return
	}
	w.Header().Set("Content-Type", "application/octet-stream")
	w.Header().Set("Content-Disposition", `attachment; filename="`+artifact.Filename+`"`)
	http.ServeFile(w, r, artifact.Path)
}
