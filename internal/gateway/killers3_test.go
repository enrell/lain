package gateway


// Round-3 mutation killers: boundary conditions that survived because
// earlier tests never produced the exact edge input.

import (
	"bytes"
	"encoding/json"
	"io"
	"log/slog"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

// --- libraries.go:41 (sort comparator boundary) ---

func TestKillLibrarySortEqualNames(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	for _, path := range []string{filepath.Join(dir, "a"), filepath.Join(dir, "b")} {
		if err := os.MkdirAll(path, 0o755); err != nil {
			t.Fatal(err)
		}
		rec := do(t, srv, "POST", "/api/libraries", map[string]string{
			"name": "Same", "type": "anime", "path": path,
		}, admin)
		if rec.Code != 201 {
			t.Fatalf("create library: %d %s", rec.Code, rec.Body.String())
		}
	}
	rec := do(t, srv, "GET", "/api/libraries", nil, admin)
	var libs []struct {
		ID   string `json:"id"`
		Name string `json:"name"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &libs); err != nil {
		t.Fatal(err)
	}
	// The KV bucket yields libraries in key order; a `<` comparator keeps
	// it for equal names, a `<=` one swaps the pair.
	if len(libs) != 2 || !(libs[0].ID < libs[1].ID) {
		t.Fatalf("equal-name libraries must keep bucket order: %+v", libs)
	}
}

// --- logging.go:56 (SetLogger with nil transcode must not panic) ---

func TestKillSetLoggerNilTranscode(t *testing.T) {
	srv := testServer(t)
	srv.transcode = nil
	srv.SetLogger(slog.New(slog.DiscardHandler)) // must not panic
}

// --- logging.go:153/154 (exact severity boundaries) ---

func TestKillRequestLogExactBoundaries(t *testing.T) {
	srv := testServer(t)
	var buf bytes.Buffer
	srv.SetLogger(slog.New(slog.NewTextHandler(&buf, &slog.HandlerOptions{Level: slog.LevelDebug})))

	admin := setupAdmin(t, srv)
	// Exactly 400 -> Warn (a >400 mutant logs it at Info).
	do(t, srv, "GET", "/api/enrichments", nil, admin)
	// Exactly 500 -> Error (a >500 mutant logs it at Warn).
	id := catalogOneMKV(t, srv, admin, t.TempDir(), "Show.mkv")
	srv.Registry().Register(stubTranscodeV1{result: contracts.Transcode{}})
	if _, err := srv.Registry().Swap(contracts.CapPlaybackTranscode, []string{"lain-transcode-stub"}, 0); err != nil {
		t.Fatalf("swap: %v", err)
	}
	rec := do(t, srv, "GET", "/api/items/"+id+"/transcode", nil, admin)
	if rec.Code != 500 {
		t.Fatalf("stub transcode: %d", rec.Code)
	}

	// Match level to status on the same access-log line: the handler
	// itself may log "transcode bad result" at ERROR, so a loose
	// Contains("level=ERROR") cannot see a `>500` boundary mutant.
	levelFor := func(status string) string {
		for _, line := range strings.Split(buf.String(), "\n") {
			if strings.Contains(line, "status="+status) && strings.Contains(line, "request") {
				if strings.Contains(line, "level=ERROR") {
					return "ERROR"
				}
				if strings.Contains(line, "level=WARN") {
					return "WARN"
				}
				return "other"
			}
		}
		return ""
	}
	if got := levelFor("400"); got != "WARN" {
		t.Fatalf("400 response logged at %q, want WARN:\n%s", got, buf.String())
	}
	if got := levelFor("500"); got != "ERROR" {
		t.Fatalf("500 response logged at %q, want ERROR:\n%s", got, buf.String())
	}
}

// --- theme.go:67 (home-dir fallback) ---

func TestKillOmarchyThemePathHomeFallback(t *testing.T) {
	home := t.TempDir()
	t.Setenv("LAIN_OMARCHY_COLORS", "")
	t.Setenv("HOME", home)
	want := filepath.Join(home, ".local", "state", "omarchy", "current", "theme", "colors.toml")
	if got := omarchyThemePath(); got != want {
		t.Fatalf("default path: %q want %q", got, want)
	}
}

// --- theme.go:100 (inline-comment strip guards) ---

func TestKillThemeCommentStrip(t *testing.T) {
	const base = "background = \"#101010\"\nforeground = \"#f0f0f0\"\n"
	// A leading '#' at index 0 is the colour itself, never a comment.
	p, ok := parseOmarchyTheme(strings.NewReader(base + "accent = #aabbcc\n"))
	if !ok || p.Accent != "#aabbcc" {
		t.Fatalf("unquoted hex value must survive: %q %v", p.Accent, ok)
	}
	// An inline comment after a bare word strips (i>0 branch).
	p, ok = parseOmarchyTheme(strings.NewReader("mode = light # trailing\n" + base + "accent = \"#aabbcc\"\n"))
	if !ok || p.Mode != "light" {
		t.Fatalf("commented bare mode must parse to light: %q %v", p.Mode, ok)
	}
	// Quotes protect an inner '#': a quoted colour keeps its leading '#'
	// instead of being truncated to a bare quote.
	for _, q := range []string{`'`, `"`} {
		p, ok = parseOmarchyTheme(strings.NewReader(base + "accent = " + q + "#112233" + q + "\n"))
		if !ok || p.Accent != "#112233" {
			t.Fatalf("quoted colour: %q %v", p.Accent, ok)
		}
	}
}

// --- enrich.go:35 (batch limit boundary) ---

func TestKillEnrichBatchExactLimit(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	var sb strings.Builder
	for i := 0; i < maxEnrichBatch; i++ {
		if i > 0 {
			sb.WriteByte(',')
		}
		sb.WriteString("id")
		sb.WriteString(strconv.Itoa(i))
	}
	if rec := do(t, srv, "GET", "/api/enrichments?ids="+sb.String(), nil, admin); rec.Code == 400 {
		t.Fatalf("exactly %d ids must be accepted: %s", maxEnrichBatch, rec.Body.String())
	}
}

// --- enrich.go:173 (empty results are never cached) ---

func TestKillEnrichEmptyNotCached(t *testing.T) {
	srv := testServer(t)
	empty := &fakeMeta{id: "lain-metadata-nfo"}
	withFakeMetadata(t, srv, empty,
		&fakeMeta{id: "lain-metadata-kitsu"},
		&fakeMeta{id: "lain-metadata-anilist"},
		&fakeMeta{id: "lain-metadata-jikan"},
		&fakeMeta{id: "lain-metadata-tvmaze"},
	)
	if _, err := srv.searchMetadata("nothing matches this", "anime", "", ""); err != nil {
		t.Fatalf("first search: %v", err)
	}
	if _, err := srv.searchMetadata("nothing matches this", "anime", "", ""); err != nil {
		t.Fatalf("second search: %v", err)
	}
	if empty.searches != 2 {
		t.Fatalf("empty result must not be cached: searches=%d, want 2", empty.searches)
	}
}

// --- server.go:201/204 (CLI bounds only apply when > 0) ---

func TestKillBootTranscodeOptBounds(t *testing.T) {
	srv, err := NewWithOptions(t.TempDir(), "test", Options{
		transcodeProbe:      noTranscodeProbe,
		TranscodeCacheBytes: 0,
		TranscodeQueueSize:  0,
	})
	if err != nil {
		t.Fatal(err)
	}
	defer srv.Close()
	got := srv.settings.Transcode()
	def := contracts.DefaultTranscodeSettings()
	if got.CacheBytes != def.CacheBytes || got.QueueSize != def.QueueSize {
		t.Fatalf("zero CLI bounds must keep defaults: %+v vs %+v", got, def)
	}
	if got.CacheBytes == 0 || got.QueueSize == 0 {
		t.Fatalf("defaults must be non-zero: %+v", got)
	}
}

// --- server.go:153 (saved composition adoption) ---

func TestKillCompositionBootAdoption(t *testing.T) {
	// A saved composition with a custom binding is observably different
	// from the defaults: the gateway must adopt it, not silently fall
	// back to DefaultComposition.
	dir := t.TempDir()
	comp := `{"version":5,"bindings":{"lain.catalog.read@1":{"mode":"exactly-one","providers":["lain-catalog-bolt"],"generation":42}}}`
	if err := os.WriteFile(filepath.Join(dir, "composition.json"), []byte(comp), 0o600); err != nil {
		t.Fatal(err)
	}
	srv, err := NewWithOptions(dir, "test", Options{transcodeProbe: noTranscodeProbe})
	if err != nil {
		t.Fatal(err)
	}
	defer srv.Close()
	for _, v := range srv.Registry().Composition().View() {
		if v.Capability == "lain.catalog.read@1" {
			if v.Generation != 42 {
				t.Fatalf("saved binding not adopted: %+v", v)
			}
			return
		}
	}
	t.Fatal("saved binding missing from composition view")
}

// --- server.go:155/158 (migration/upgrade console lines) ---

func captureStdout(t *testing.T, fn func()) string {
	t.Helper()
	old := os.Stdout
	r, w, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	os.Stdout = w
	fn()
	_ = w.Close()
	os.Stdout = old
	out, _ := io.ReadAll(r)
	return string(out)
}

func TestKillBootMigrationPrints(t *testing.T) {
	// A composition with a retired provider id and missing capabilities
	// must print both the migration and the upgrade lines.
	retired := `{"version":5,"bindings":{"lain.catalog.read@1":{"mode":"exactly-one","providers":["lain-catalog-file"],"generation":1}}}`
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "composition.json"), []byte(retired), 0o600); err != nil {
		t.Fatal(err)
	}
	out := captureStdout(t, func() {
		srv, err := NewWithOptions(dir, "test", Options{transcodeProbe: noTranscodeProbe})
		if err != nil {
			t.Errorf("boot: %v", err)
			return
		}
		srv.Close()
	})
	if !strings.Contains(out, "migrated retired providers") {
		t.Fatalf("retired id must print a migration line, got: %q", out)
	}
	if !strings.Contains(out, "new capabilities from defaults") {
		t.Fatalf("partial composition must print an upgrade line, got: %q", out)
	}

	// A composition identical to the defaults prints neither line.
	raw, err := json.Marshal(core.DefaultComposition())
	if err != nil {
		t.Fatal(err)
	}
	dir = t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "composition.json"), raw, 0o600); err != nil {
		t.Fatal(err)
	}
	out = captureStdout(t, func() {
		srv, err := NewWithOptions(dir, "test", Options{transcodeProbe: noTranscodeProbe})
		if err != nil {
			t.Errorf("boot: %v", err)
			return
		}
		srv.Close()
	})
	if strings.Contains(out, "migrated retired providers") || strings.Contains(out, "new capabilities") {
		t.Fatalf("default-equal composition must print nothing, got: %q", out)
	}

	// An empty-bindings file must not be adopted at all: a `>= 0`
	// boundary mutant enters the block and prints the upgrade line.
	dir = t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "composition.json"), []byte(`{"version":5,"bindings":{}}`), 0o600); err != nil {
		t.Fatal(err)
	}
	out = captureStdout(t, func() {
		srv, err := NewWithOptions(dir, "test", Options{transcodeProbe: noTranscodeProbe})
		if err != nil {
			t.Errorf("boot: %v", err)
			return
		}
		srv.Close()
	})
	if strings.Contains(out, "new capabilities") {
		t.Fatalf("empty bindings must not be adopted, got: %q", out)
	}
}

// --- server.go:668 (bitrate cap boundary) ---

type fakeMediaProbe struct{ info contracts.MediaInfo }

func (fakeMediaProbe) ID() string { return "lain-probe-stub" }
func (fakeMediaProbe) Capabilities() []string {
	return []string{contracts.CapMediaProbe}
}
func (fakeMediaProbe) Health() error { return nil }
func (f fakeMediaProbe) Invoke(_ string, _ any) (any, error) {
	return f.info, nil
}

func TestKillBitrateCapBoundaries(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	id := catalogOneMP4(t, srv, admin, t.TempDir(), "[Fansub-A] Show.mp4")
	// Probe reports exactly 50000 kbps of video.
	srv.Registry().Register(fakeMediaProbe{info: contracts.MediaInfo{
		Format:   "mp4",
		Duration: 60,
		Streams: []contracts.MediaStream{
			{Type: "video", Codec: "h264", PixelFormat: "yuv420p", BitRate: 50_000_000, Width: 1920, Height: 1080},
		},
	}})
	if _, err := srv.Registry().Swap(contracts.CapMediaProbe, []string{"lain-probe-stub"}, 0); err != nil {
		t.Fatalf("probe swap: %v", err)
	}

	plan := func() contracts.Plan {
		rec := do(t, srv, "GET", "/api/items/"+id+"/playback?client=web", nil, admin)
		var p contracts.Plan
		if err := json.Unmarshal(rec.Body.Bytes(), &p); err != nil {
			t.Fatalf("plan decode: %d %s", rec.Code, rec.Body.String())
		}
		return p
	}

	// No cap anywhere: direct play must stay direct. A `>= 0` or `<= 0`
	// mutant on `maxBitrate > 0` downgrades it to transcode.
	if p := plan(); p.Mode != "direct" {
		t.Fatalf("uncapped plan mode=%q, want direct", p.Mode)
	}

	// Cap exactly equal to the probed bitrate: `>` means the source is
	// not *above* the cap, so direct play stays. `>=` downgrades it.
	rec := do(t, srv, "PUT", "/api/admin/settings/transcode", map[string]any{"remote_bitrate_limit_kbps": 50000}, admin)
	if rec.Code != http.StatusOK {
		t.Fatalf("cap PUT: %d %s", rec.Code, rec.Body.String())
	}
	if p := plan(); p.Mode != "direct" {
		t.Fatalf("cap == source bitrate must stay direct, got %q", p.Mode)
	}
}

// --- server.go:827 (dependency-unavailable -> 422) ---

type unhealthyProvider struct{ id, cap string }

func (u unhealthyProvider) ID() string             { return u.id }
func (u unhealthyProvider) Capabilities() []string { return []string{u.cap} }
func (u unhealthyProvider) Health() error          { return errFakeMetaDown }
func (u unhealthyProvider) Invoke(string, any) (any, error) {
	return nil, errFakeMetaDown
}

func TestKillSwapDependencyUnavailable(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	rec := do(t, srv, "GET", "/api/plugins", nil, admin)
	var comp struct {
		Generation uint64 `json:"generation"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &comp); err != nil {
		t.Fatal(err)
	}
	srv.Registry().Register(unhealthyProvider{id: "sick-source", cap: contracts.CapSourceEnumerate})
	rec = do(t, srv, "POST", "/api/plugins/swap", map[string]any{
		"capability": contracts.CapSourceEnumerate,
		"providers":  []string{"sick-source"},
		"generation": comp.Generation,
	}, admin)
	if rec.Code != 422 {
		t.Fatalf("unhealthy provider swap -> %d, want 422: %s", rec.Code, rec.Body.String())
	}
}

// --- transcode.go:496/501 (signMapURI edges) ---

func TestKillSignMapURIEdges(t *testing.T) {
	// URI= at column 0: start==0 must still sign, not bail out.
	out := signMapURI(`URI="init.mp4"`, "sess", "tok")
	if !strings.Contains(out, "session=sess") {
		t.Fatalf("URI at offset 0 must be signed: %q", out)
	}
	// URI= without a closing quote returns the line untouched.
	if out := signMapURI(`#EXT-X-MAP:URI="unterminated`, "s", "t"); out != `#EXT-X-MAP:URI="unterminated` {
		t.Fatalf("unterminated URI must pass through: %q", out)
	}
	// Missing URI attr returns the line untouched.
	if out := signMapURI(`#EXT-X-MAP:BYTERANGE=100`, "s", "t"); out != `#EXT-X-MAP:BYTERANGE=100` {
		t.Fatalf("no URI attr must pass through: %q", out)
	}
}

// --- transcode.go:478 (blank playlist lines pass through unsigned) ---

func TestKillRewritePlaylistBlankLine(t *testing.T) {
	raw := "#EXTM3U\n\nseg0.m4s\n"
	out := rewriteHLSPlaylist(raw, "sess", "tok")
	lines := strings.Split(out, "\n")
	if len(lines) < 3 || lines[1] != "" {
		t.Fatalf("blank line must pass through untouched: %q", out)
	}
	if !strings.Contains(lines[2], "session=sess") {
		t.Fatalf("segment must be signed: %q", out)
	}
}

// --- watch.go:56/57 (default debounce) ---

func TestKillWatcherDefaultDebounce(t *testing.T) {
	srv := testServer(t)
	if err := srv.StartWatcher(); err != nil {
		t.Fatalf("start watcher: %v", err)
	}
	w := srv.watcher()
	if w == nil {
		t.Fatal("watcher must exist after StartWatcher")
	}
	if w.debounce != 2*time.Second {
		t.Fatalf("default debounce=%v, want 2s", w.debounce)
	}
}
