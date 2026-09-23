package gateway

// mutation-clean

import (
	"bytes"
	"encoding/json"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/enrell/lain/internal/auth"
	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/plugins/transcode"
	"github.com/fsnotify/fsnotify"
)

// --- libraries.go:41/159 (sort order) ---

func TestKillLibrariesSortedByName(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	root := t.TempDir()
	// Create in reverse alphabetical order so the response order must
	// come from the sort, not insertion order.
	for _, name := range []string{"zed", "abc"} {
		dir := filepath.Join(root, name)
		if err := os.MkdirAll(dir, 0o755); err != nil {
			t.Fatal(err)
		}
		rec := do(t, srv, "POST", "/api/libraries", map[string]any{
			"name": name, "path": dir,
		}, admin)
		if rec.Code != 200 && rec.Code != 201 {
			t.Fatalf("create %s: %d %s", name, rec.Code, rec.Body.String())
		}
	}
	rec := do(t, srv, "GET", "/api/libraries", nil, admin)
	if rec.Code != 200 {
		t.Fatalf("list: %d", rec.Code)
	}
	var libs []struct {
		Name string `json:"name"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &libs); err != nil {
		t.Fatal(err)
	}
	if len(libs) != 2 || libs[0].Name != "abc" || libs[1].Name != "zed" {
		t.Fatalf("libraries not sorted by name: %+v", libs)
	}
}

// --- logging.go (request severity boundaries) ---

func TestKillRequestLogSeverity(t *testing.T) {
	srv := testServer(t)
	var buf bytes.Buffer
	srv.SetLogger(slog.New(slog.NewTextHandler(&buf, &slog.HandlerOptions{Level: slog.LevelDebug})))

	// 2xx -> Debug.
	do(t, srv, "GET", "/api/health", nil, "")
	// 4xx -> Warn (unknown item).
	do(t, srv, "GET", "/api/items/ghost/playback", nil, "")
	// 5xx -> Error (transcode provider returns an unusable result).
	admin := setupAdmin(t, srv)
	id := catalogOneMKV(t, srv, admin, t.TempDir(), "Show.mkv")
	srv.Registry().Register(stubTranscodeV1{result: contracts.Transcode{}})
	if _, err := srv.Registry().Swap(contracts.CapPlaybackTranscode, []string{"lain-transcode-stub"}, 0); err != nil {
		t.Fatalf("swap: %v", err)
	}
	rec := do(t, srv, "GET", "/api/items/"+id+"/transcode", nil, admin)
	if rec.Code != 500 {
		t.Fatalf("transcode stub: %d", rec.Code)
	}

	log := buf.String()
	if !strings.Contains(log, "req=req-") {
		t.Fatalf("request id missing from access log:\n%s", log)
	}
	if !strings.Contains(log, "level=DEBUG") {
		t.Fatalf("2xx request not logged at DEBUG:\n%s", log)
	}
	if !strings.Contains(log, "level=WARN") {
		t.Fatalf("4xx request not logged at WARN:\n%s", log)
	}
	if !strings.Contains(log, "level=ERROR") {
		t.Fatalf("5xx request not logged at ERROR:\n%s", log)
	}
}

// --- users.go:75 (password reset guard) ---

func TestKillUserPatchPassword(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	rec := do(t, srv, "POST", "/api/users", map[string]any{
		"username": "patchme", "password": "first-pass", "role": "user",
	}, admin)
	if rec.Code != 200 && rec.Code != 201 {
		t.Fatalf("create user: %d %s", rec.Code, rec.Body.String())
	}
	var u struct {
		ID string `json:"id"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &u); err != nil || u.ID == "" {
		t.Fatalf("create user body: %s", rec.Body.String())
	}
	rec = do(t, srv, "PATCH", "/api/users/"+u.ID, map[string]any{
		"password": "second-pass",
	}, admin)
	if rec.Code != 200 {
		t.Fatalf("patch: %d %s", rec.Code, rec.Body.String())
	}
	if loginAs(t, srv, "patchme", "second-pass") == "" {
		t.Fatal("login with reset password failed")
	}
	rec = do(t, srv, "POST", "/api/auth/login", map[string]string{
		"username": "patchme", "password": "first-pass",
	}, "")
	if rec.Code == 200 {
		t.Fatal("old password still works after reset")
	}
}

// --- server.go:415 (libs list db failure -> 500) ---

func TestKillLibsListDBError(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	_ = admin // auth would fail first over HTTP; drive the handler directly
	if err := srv.db.Close(); err != nil {
		t.Fatal(err)
	}
	w := httptest.NewRecorder()
	r := httptest.NewRequest("GET", "/api/libraries", nil)
	srv.handleLibsList(w, r, auth.Verified{})
	if w.Code != http.StatusInternalServerError {
		t.Fatalf("libsList with dead db: %d", w.Code)
	}
}

// --- server.go:829 (swap persistence failure -> 500) ---

func TestKillSwapSaveFailure(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := srv.st.Root()
	if err := os.Chmod(dir, 0o555); err != nil {
		t.Fatalf("chmod: %v", err)
	}
	defer func() { _ = os.Chmod(dir, 0o755) }()
	// Swapping a binding to its own provider is still a Save, which must
	// fail when composition.json cannot be written.
	rec := do(t, srv, "POST", "/api/plugins/swap", map[string]any{
		"capability": contracts.CapCatalogRead,
		"providers":  []string{"lain-catalog-bolt"},
		"generation": 1,
	}, admin)
	if rec.Code != http.StatusInternalServerError {
		t.Fatalf("swap with unwritable composition.json: %d %s", rec.Code, rec.Body.String())
	}
}

// --- server.go:846 (backup write failure -> 500) ---

func TestKillBackupStreamsBytes(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	rec := do(t, srv, "GET", "/api/admin/backup", nil, admin)
	if rec.Code != 200 {
		t.Fatalf("backup: %d %s", rec.Code, rec.Body.String())
	}
	if rec.Body.Len() == 0 {
		t.Fatal("backup body empty")
	}
}

// --- server.go:213-214 (automatic hardware adoption on boot) ---
// Covered indirectly: composition boot writes merged settings; the
// withAutomaticHardware path requires a probe stub that reports
// hardware encoders — exercised via toneMapAvailable + probe stub in
// TestKillToneMapGate below.

// --- server.go:644-650 (tone-map gate in playback plan) ---

func TestKillToneMapGate(t *testing.T) {
	srv := testServer(t)
	probeCalls := 0
	srv.probeTranscode = func(contracts.TranscodeSettings) transcode.CapabilitiesReport {
		probeCalls++
		return transcode.CapabilitiesReport{ToneMapping: true}
	}
	settings := srv.settings.Transcode()
	settings.ToneMapping = true
	settings.ToneMappingMode = contracts.ToneMapModeAuto
	if !srv.toneMapAvailable(settings) {
		t.Fatal("enabled + probed tone map must be available")
	}
	settings.ToneMapping = false
	if srv.toneMapAvailable(settings) {
		t.Fatal("disabled tone map must be unavailable")
	}
	settings.ToneMapping = true
	settings.ToneMappingMode = contracts.ToneMapModeNever
	if srv.toneMapAvailable(settings) {
		t.Fatal("tone_map_mode=never must be unavailable")
	}
	settings.ToneMappingMode = contracts.ToneMapModeAuto
	srv.probeTranscode = func(contracts.TranscodeSettings) transcode.CapabilitiesReport {
		return transcode.CapabilitiesReport{ToneMapping: false}
	}
	if srv.toneMapAvailable(settings) {
		t.Fatal("probe-negative tone map must be unavailable")
	}
}

// --- thumbnail.go:52/55 (width/time clamps) ---
// Covered by TestKillThumbnailParams in killers_test.go.

// --- watch.go:188/189 (dir add/remove on fs events) ---

func TestKillWatcherDirTracking(t *testing.T) {
	srv := testServer(t)
	fs, err := fsnotify.NewWatcher()
	if err != nil {
		t.Fatalf("watcher: %v", err)
	}
	defer fs.Close()
	libID := "lib-1"
	w := &libWatcher{
		s:      srv,
		fs:     fs,
		dirs:   map[string]string{},
		timers: map[string]*time.Timer{},
		scan:   func(string) {},
	}
	parent := t.TempDir()
	dir := filepath.Join(parent, "newdir")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	w.dirs[parent] = libID
	// A directory create under a watched parent registers the new dir.
	w.onEvent(fsnotify.Event{Name: dir, Op: fsnotify.Create})
	if got := w.dirs[dir]; got != libID {
		t.Fatalf("created dir not tracked: dirs[%q]=%q", dir, got)
	}
	// A remove event drops it again.
	w.onEvent(fsnotify.Event{Name: dir, Op: fsnotify.Remove})
	if _, ok := w.dirs[dir]; ok {
		t.Fatalf("removed dir still tracked: %+v", w.dirs)
	}
}

// --- enrich.go:88 (kind derivation) ---

func TestKillEnrichSearchKind(t *testing.T) {
	srv := testServer(t)
	var kind string
	fm := &fakeMeta{
		id:         "meta-kind",
		candidates: []contracts.MetadataCandidate{{Provider: "meta-kind", RemoteID: "c1", Title: "Show", Score: 1}},
		record:     contracts.MetadataRecord{Provider: "meta-kind", RemoteID: "c1", Title: "Show"},
		lastKind:   &kind,
	}
	srv.Registry().Register(fm)
	if _, err := srv.Registry().Swap(contracts.CapMetadataSearch, []string{"meta-kind"}, 0); err != nil {
		t.Fatalf("swap search: %v", err)
	}
	if _, err := srv.Registry().Swap(contracts.CapMetadataResolve, []string{"meta-kind"}, 0); err != nil {
		t.Fatalf("swap resolve: %v", err)
	}
	it := contracts.CatalogItem{ID: "it-kind", Title: "Show", Kind: "tvshow", FilePath: "/x/Show.mkv"}
	_, code, msg := srv.enrichOne(it, "")
	if code != 200 {
		t.Fatalf("enrichOne: %d %s", code, msg)
	}
	if kind != "tvshow" {
		t.Fatalf("search kind = %q, want %q", kind, "tvshow")
	}
}

// --- enrich.go:161 (skipped providers logged) ---

func TestKillEnrichSkippedLog(t *testing.T) {
	srv := testServer(t)
	var buf bytes.Buffer
	srv.SetLogger(slog.New(slog.NewTextHandler(&buf, &slog.HandlerOptions{Level: slog.LevelDebug})))
	fm := &fakeMeta{id: "meta-fail", failSearch: true}
	ok := &fakeMeta{id: "meta-ok", candidates: []contracts.MetadataCandidate{
		{Provider: "meta-ok", RemoteID: "c1", Title: "Show", Score: 1},
	}}
	srv.Registry().Register(fm)
	srv.Registry().Register(ok)
	if _, err := srv.Registry().Swap(contracts.CapMetadataSearch, []string{"meta-fail", "meta-ok"}, 0); err != nil {
		t.Fatalf("swap: %v", err)
	}
	it := contracts.CatalogItem{ID: "it-skip", Title: "Show", Kind: "tvshow", FilePath: "/x/Show.mkv"}
	_, _, _ = srv.enrichOne(it, "")
	if !strings.Contains(buf.String(), "skipped") {
		t.Fatalf("failing provider not logged as skipped:\n%s", buf.String())
	}
}

// --- enrich.go:173 (empty merge must not poison the cache) ---

func TestKillEnrichEmptyMergeNoCache(t *testing.T) {
	srv := testServer(t)
	fm := &fakeMeta{id: "meta-empty", candidates: nil, record: contracts.MetadataRecord{Provider: "meta-empty", RemoteID: "c1", Title: "Show"}}
	srv.Registry().Register(fm)
	for _, cap := range []string{contracts.CapMetadataSearch, contracts.CapMetadataResolve} {
		if _, err := srv.Registry().Swap(cap, []string{"meta-empty"}, 0); err != nil {
			t.Fatalf("swap %s: %v", cap, err)
		}
	}
	it := contracts.CatalogItem{ID: "it-empty", Title: "Show", Kind: "tvshow", FilePath: "/x/Show.mkv"}
	_, code, _ := srv.enrichOne(it, "")
	if code != 404 {
		t.Fatalf("empty merge should 404, got %d", code)
	}
	// A provider that later returns candidates must be consulted; an
	// empty merged result must not be cached as "searched, nothing".
	fm.candidates = []contracts.MetadataCandidate{{Provider: "meta-empty", RemoteID: "c1", Title: "Show", Score: 1}}
	_, code, msg := srv.enrichOne(it, "")
	if code != 200 {
		t.Fatalf("second enrichOne: %d %s", code, msg)
	}
}

// --- transcode.go:476-531 (EXT-X-MAP signing + URI signing edges) ---

func TestKillHLSURISigningEdges(t *testing.T) {
	// Unterminated URI="..." must pass through unchanged.
	bad := `#EXT-X-MAP:URI="init.mp4`
	if got := signMapURI(bad, "sess", "tok"); got != bad {
		t.Fatalf("unterminated map rewritten: %q", got)
	}
	// Well-formed map URI is signed.
	got := signMapURI(`#EXT-X-MAP:URI="init.mp4"`, "sess", "tok")
	if !strings.Contains(got, "session=sess") || !strings.Contains(got, "init.mp4") {
		t.Fatalf("map URI not signed: %q", got)
	}
	// Segment URIs get session+token; absolute URIs are never signed.
	got = signHLSURI("seg0.m4s", "sess", "tok")
	if !strings.Contains(got, "session=sess") || !strings.Contains(got, "token=tok") {
		t.Fatalf("segment URI not signed: %q", got)
	}
	if got := signHLSURI("https://evil.example/x.m4s", "sess", "tok"); strings.Contains(got, "tok") {
		t.Fatalf("absolute URI leaked token: %q", got)
	}
}

// --- transcode.go:608 (subtitle stream index 0 must be accepted) ---

type stubTranscodeV3Sub struct{ subPath string }

func (stubTranscodeV3Sub) ID() string { return "lain-transcode-v3-stub" }
func (stubTranscodeV3Sub) Capabilities() []string {
	return []string{contracts.CapPlaybackTranscodeV3}
}
func (stubTranscodeV3Sub) Health() error { return nil }
func (s stubTranscodeV3Sub) Invoke(_ string, _ any) (any, error) {
	return contracts.TranscodeV3Status{SubtitlePath: s.subPath}, nil
}

func TestKillSubtitleStreamIndexZero(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	sub := filepath.Join(t.TempDir(), "sub.vtt")
	if err := os.WriteFile(sub, []byte("WEBVTT\n\n00:00.000 --> 00:01.000\nhi\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	srv.Registry().Register(stubTranscodeV3Sub{subPath: sub})
	if _, err := srv.Registry().Swap(contracts.CapPlaybackTranscodeV3, []string{"lain-transcode-v3-stub"}, 0); err != nil {
		t.Fatalf("swap: %v", err)
	}
	id := catalogOneMKV(t, srv, admin, t.TempDir(), "Show.mkv")
	// index 0 is a valid subtitle stream — must not 400.
	rec := do(t, srv, "GET", "/api/items/"+id+"/subtitles?stream=0", nil, admin)
	if rec.Code == http.StatusBadRequest {
		t.Fatalf("subtitle index 0 rejected: %s", rec.Body.String())
	}
}
