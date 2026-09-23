package gateway


// Mutation killers for gateway pure helpers and handler branches that
// the behavioural suites exercise but never assert precisely.

import (
	"context"
	"encoding/json"
	"image/jpeg"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/fsnotify/fsnotify"
	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/kv"
)

// --- pure helpers ---

func TestKillShortID(t *testing.T) {
	if got := shortID(""); got != "00000000" {
		t.Fatalf("empty id: %q", got)
	}
	// Exact values pin the h*31 accumulation: an additive mutant yields
	// 0x00c3 for "ab" instead of 0x0c21.
	if got := shortID("ab"); got != "00000c21" {
		t.Fatalf("ab id: %q", got)
	}
	// Long inputs overflow int; a dropped sign fix or wrong modulus
	// leaves "-..." or a wider string. Assert strict 8-hex output.
	for _, s := range []string{
		strings.Repeat("z", 24), strings.Repeat("y", 31), strings.Repeat("x", 40),
		strings.Repeat("w", 55), strings.Repeat("v", 70),
	} {
		got := shortID(s)
		if len(got) != 8 {
			t.Fatalf("shortID(%q)=%q is not 8 chars", s[:4], got)
		}
		if _, err := strconv.ParseUint(got, 16, 32); err != nil {
			t.Fatalf("shortID(%q)=%q is not hex: %v", s[:4], got, err)
		}
	}
	if shortID("library-one") == shortID("library-two") {
		t.Fatal("distinct inputs must not collide here")
	}
}

func TestKillEffectiveBitrateLimit(t *testing.T) {
	cases := []struct{ user, server, want int }{
		{0, 0, 0}, {100, 0, 100}, {0, 200, 200},
		{100, 200, 100}, {200, 100, 100}, {150, 150, 150},
		{-5, 300, 300}, {300, -5, 300},
	}
	for _, c := range cases {
		if got := effectiveBitrateLimit(c.user, c.server); got != c.want {
			t.Fatalf("effectiveBitrateLimit(%d,%d)=%d want %d", c.user, c.server, got, c.want)
		}
	}
}

func TestKillSourceBitrateKbps(t *testing.T) {
	if got := sourceBitrateKbps(nil); got != 0 {
		t.Fatalf("nil: %d", got)
	}
	info := &contracts.MediaInfo{Streams: []contracts.MediaStream{
		{Type: "video", BitRate: 5_000_000},
		{Type: "audio", BitRate: 128_000},
		{Type: "subtitle", BitRate: 9_999_999}, // never counted
	}}
	if got := sourceBitrateKbps(info); got != 5128 {
		t.Fatalf("sum: %d", got)
	}
}

func TestKillMediaDurationSec(t *testing.T) {
	if mediaDurationSec(nil) != 0 || mediaDurationSec(&contracts.MediaInfo{Duration: 0}) != 0 || mediaDurationSec(&contracts.MediaInfo{Duration: -3}) != 0 {
		t.Fatal("nil/zero/negative duration must report 0")
	}
	if mediaDurationSec(&contracts.MediaInfo{Duration: 90.5}) != 90.5 {
		t.Fatal("duration must pass through")
	}
}

func TestKillOtelSeverity(t *testing.T) {
	cases := []struct {
		l    slog.Level
		want int
	}{
		{slog.LevelDebug, 5}, {slog.LevelInfo, 9}, {slog.LevelWarn, 13}, {slog.LevelError, 17},
		{slog.Level(-1), 5}, {slog.Level(2), 9}, {slog.Level(6), 13}, {slog.Level(100), 17},
	}
	for _, c := range cases {
		if got := otelSeverity(c.l); got != c.want {
			t.Fatalf("otelSeverity(%d)=%d want %d", c.l, got, c.want)
		}
	}
}

func TestKillReqIDOf(t *testing.T) {
	r := httptest.NewRequest("GET", "/", nil)
	if reqIDOf(r) != "" {
		t.Fatal("no context value must yield empty id")
	}
	r = r.WithContext(context.WithValue(r.Context(), reqIDKey, "req-9"))
	if reqIDOf(r) != "req-9" {
		t.Fatalf("id: %q", reqIDOf(r))
	}
}

func TestKillOmarchyThemePath(t *testing.T) {
	t.Setenv("LAIN_OMARCHY_COLORS", "/custom/colors.toml")
	if got := omarchyThemePath(); got != "/custom/colors.toml" {
		t.Fatalf("env override: %q", got)
	}
	t.Setenv("LAIN_OMARCHY_COLORS", "   ")
	got := omarchyThemePath()
	if got == "/custom/colors.toml" || (got != "" && !strings.HasSuffix(got, filepath.Join("theme", "colors.toml"))) {
		t.Fatalf("blank env must fall back to the default path: %q", got)
	}
}

func TestKillDirOf(t *testing.T) {
	if dirOf("") != "" {
		t.Fatal("empty path must stay empty")
	}
	if dirOf("/media/show/ep.mkv") != "/media/show" {
		t.Fatalf("dir: %q", dirOf("/media/show/ep.mkv"))
	}
}

// --- theme palette fixups ---

func TestKillThemeContrastMath(t *testing.T) {
	if got := contrastRatio("#000000", "#ffffff"); got < 20.9 || got > 21.1 {
		t.Fatalf("black/white contrast: %v", got)
	}
	// Symmetric: swapping args must not change the ratio.
	a, b := "#123456", "#fedcba"
	if contrastRatio(a, b) != contrastRatio(b, a) {
		t.Fatal("contrastRatio must be symmetric")
	}
	if contrastColor("#000000") != "#ffffff" || contrastColor("#ffffff") != "#000000" {
		t.Fatal("contrastColor must pick the farther pole")
	}
	// 0x0a sits below the sRGB linearisation knee, 0x0b above it.
	if hexLuminance("#0a0a0a") >= hexLuminance("#0b0b0b") {
		t.Fatal("the sRGB knee must stay monotonic")
	}
}

func TestKillThemeParseModes(t *testing.T) {
	// mode validation: only dark/light are accepted; junk is ignored.
	const base = "background = \"#101010\"\nforeground = \"#f0f0f0\"\naccent = \"#6699cc\"\n"
	p, ok := parseOmarchyTheme(strings.NewReader("mode = \"dark\"\n" + base))
	if !ok || p.Mode != "dark" {
		t.Fatalf("dark mode parse: %+v %v", p, ok)
	}
	p, ok = parseOmarchyTheme(strings.NewReader("mode = \"sepia\"\n" + base))
	if !ok || p.Mode != "dark" {
		t.Fatalf("invalid mode must fall back to dark: %q %v", p.Mode, ok)
	}
	// Inline comments strip only for unquoted non-# values: `dark # x`
	// parses as dark, while `#101010 # x` keeps its leading #.
	p, ok = parseOmarchyTheme(strings.NewReader("mode = dark # a comment\n" + base))
	if !ok || p.Mode != "dark" {
		t.Fatalf("inline comment on a bare value: %q %v", p.Mode, ok)
	}
}

func TestKillThemePaletteFixup(t *testing.T) {
	// A collapsed palette (everything black) forces every fixup branch:
	// SurfaceActive must separate from Surface, Muted must clear the
	// text floor, Line must be visible.
	p := applyLegibilityFloor(themePalette{
		Background: "#000000", Surface: "#000000", SurfaceActive: "#000000",
		Foreground: "#ffffff", Muted: "#000000", Line: "#000000",
		Accent: "#000000",
	})
	if contrastRatio(p.SurfaceActive, p.Surface) < minLineContrast-0.05 {
		t.Fatalf("surface_active must read as a fill: %q on %q", p.SurfaceActive, p.Surface)
	}
	if contrastRatio(p.Muted, p.SurfaceActive) < minTextContrast-0.05 {
		t.Fatalf("muted must clear the text floor: %q on %q", p.Muted, p.SurfaceActive)
	}
	if contrastRatio(p.Line, p.Surface) < minLineContrast-0.05 {
		t.Fatalf("line must be visible: %q on %q", p.Line, p.Surface)
	}
}

// --- handler branches ---

func TestKillCloseWithoutTranscode(t *testing.T) {
	srv := testServer(t)
	srv.transcode = nil
	if err := srv.Close(); err != nil {
		t.Fatalf("close with nil transcode: %v", err)
	}
}

func TestKillLibCreateValidation(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	for _, body := range []map[string]string{
		{},                                 // nothing
		{"name": "x"},                      // no path
		{"path": t.TempDir()},              // no name
		{"name": "x", "path": "/no/such"},  // missing dir
		{"name": "x", "path": "/dev/null"}, // not a directory
	} {
		if rec := do(t, srv, "POST", "/api/libraries", body, admin); rec.Code != 400 {
			t.Fatalf("body %v -> %d", body, rec.Code)
		}
	}
	// Default type fills in; re-creating the same path keeps the id.
	dir := t.TempDir()
	rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "L", "path": dir}, admin)
	if rec.Code != 201 {
		t.Fatalf("create: %d %s", rec.Code, rec.Body.String())
	}
	var lib contracts.Library
	if err := json.Unmarshal(rec.Body.Bytes(), &lib); err != nil || lib.Type != "anime" {
		t.Fatalf("default type: %s", rec.Body.String())
	}
	rec = do(t, srv, "POST", "/api/libraries", map[string]string{"name": "L2", "path": dir}, admin)
	var lib2 contracts.Library
	_ = json.Unmarshal(rec.Body.Bytes(), &lib2)
	if lib2.ID != lib.ID {
		t.Fatal("the same path must keep a stable library id")
	}
}

func TestKillBrowseFilters(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	root := t.TempDir()
	if err := os.Mkdir(filepath.Join(root, "Season 1"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, "episode.mkv"), []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(filepath.Join(root, ".hidden"), 0o755); err != nil {
		t.Fatal(err)
	}
	rec := do(t, srv, "GET", "/api/browse?path="+root, nil, admin)
	if rec.Code != 200 {
		t.Fatalf("browse: %d %s", rec.Code, rec.Body.String())
	}
	var out struct {
		Dirs []struct {
			Name string `json:"name"`
		} `json:"dirs"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &out); err != nil {
		t.Fatal(err)
	}
	if len(out.Dirs) != 1 || out.Dirs[0].Name != "Season 1" {
		t.Fatalf("files and dotdirs must be filtered: %+v", out.Dirs)
	}
	for _, path := range []string{"relative/dir", "/no/such/dir", filepath.Join(root, "episode.mkv")} {
		if rec := do(t, srv, "GET", "/api/browse?path="+path, nil, admin); rec.Code != 400 {
			t.Fatalf("browse %q -> %d", path, rec.Code)
		}
	}
}

func TestKillThumbnailWidthParams(t *testing.T) {
	if _, err := exec.LookPath("ffmpeg"); err != nil {
		t.Skip("ffmpeg unavailable")
	}
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	id := catalogOneMKV(t, srv, admin, t.TempDir(), "[Fansub-A] Show.mkv")
	token := admin
	widthOf := func(query string) int {
		rec := do(t, srv, "GET", "/api/items/"+id+"/thumbnail?"+query, nil, token)
		if rec.Code != 200 {
			t.Fatalf("thumb %q -> %d %s", query, rec.Code, rec.Body.String())
		}
		cfg, err := jpeg.DecodeConfig(rec.Body)
		if err != nil {
			t.Fatalf("thumb %q not jpeg: %v", query, err)
		}
		return cfg.Width
	}
	if got := widthOf("w=200"); got != 200 {
		t.Fatalf("w=200 -> %d", got)
	}
	if got := widthOf("w=1"); got != minThumbWidth {
		t.Fatalf("w=1 must clamp to min: %d", got)
	}
	if got := widthOf("w=99999"); got != maxThumbWidth {
		t.Fatalf("w=99999 must clamp to max: %d", got)
	}
	if got := widthOf("w=abc"); got != defaultThumbWidth {
		t.Fatalf("unparseable w must use the default: %d", got)
	}
	if got := widthOf("w=0"); got != defaultThumbWidth {
		t.Fatalf("w=0 must use the default: %d", got)
	}
}

func TestKillThumbnailTimeParam(t *testing.T) {
	if _, err := exec.LookPath("ffmpeg"); err != nil {
		t.Skip("ffmpeg unavailable")
	}
	dir := t.TempDir()
	srv, err := NewWithOptions(dir, "test", Options{transcodeProbe: noTranscodeProbe})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { srv.Close() })
	srv.SetAutoEnrich(false)
	admin := setupAdmin(t, srv)
	id := catalogOneMKV(t, srv, admin, t.TempDir(), "[Fansub-A] Show.mkv")
	thumbDir := filepath.Join(dir, "thumbnails")
	count := func() int {
		entries, _ := os.ReadDir(thumbDir)
		return len(entries)
	}
	get := func(q string) {
		rec := do(t, srv, "GET", "/api/items/"+id+"/thumbnail?"+q, nil, admin)
		if rec.Code != 200 {
			t.Fatalf("thumb %q -> %d %s", q, rec.Code, rec.Body.String())
		}
	}
	get("t=7")
	n := count()
	get("t=7") // cached: no new file
	if count() != n {
		t.Fatal("a repeated t must hit the cache")
	}
	get("t=0") // explicit zero is a real seek target, not the default
	if count() != n+1 {
		t.Fatal("t=0 must produce its own cache entry")
	}
	get("t=abc") // unparseable falls back to the default offset
	if count() != n+2 {
		t.Fatal("t=abc must produce the default-offset entry")
	}
	get("") // same default -> cached
	if count() != n+2 {
		t.Fatal("default t must hit the entry t=abc created")
	}
}

func TestKillBrowseCap(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	root := t.TempDir()
	for i := 0; i < 501; i++ {
		if err := os.Mkdir(filepath.Join(root, "d"+strconv.Itoa(i)), 0o755); err != nil {
			t.Fatal(err)
		}
	}
	rec := do(t, srv, "GET", "/api/browse?path="+root, nil, admin)
	if rec.Code != 200 {
		t.Fatalf("browse: %d", rec.Code)
	}
	var out struct {
		Dirs []browseDir `json:"dirs"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &out); err != nil {
		t.Fatal(err)
	}
	if len(out.Dirs) != 500 {
		t.Fatalf("browse must cap at 500 entries, got %d", len(out.Dirs))
	}
}

func TestKillWatcherLifecycle(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	srv.watchDebounce = 25 * time.Millisecond
	if err := srv.StartWatcher(); err != nil {
		t.Fatalf("start: %v", err)
	}
	first := srv.watcher()
	if first == nil {
		t.Fatal("watcher must be installed")
	}
	if first.debounce != 25*time.Millisecond {
		t.Fatalf("test debounce seam ignored: %v", first.debounce)
	}
	if err := srv.StartWatcher(); err != nil {
		t.Fatalf("second start: %v", err)
	}
	if srv.watcher() != first {
		t.Fatal("a second StartWatcher must be a no-op")
	}
	// Creating a library registers its tree; deleting it drops the watch.
	dir := t.TempDir()
	rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "W", "path": dir}, admin)
	if rec.Code != 201 {
		t.Fatalf("create: %d %s", rec.Code, rec.Body.String())
	}
	var lib contracts.Library
	if err := json.Unmarshal(rec.Body.Bytes(), &lib); err != nil {
		t.Fatal(err)
	}
	first.mu.Lock()
	_, watched := first.dirs[filepath.Clean(dir)]
	first.mu.Unlock()
	if !watched {
		t.Fatal("created library root must be watched")
	}
	rec = do(t, srv, "DELETE", "/api/libraries/"+lib.ID, nil, admin)
	if rec.Code != 200 {
		t.Fatalf("delete: %d %s", rec.Code, rec.Body.String())
	}
	first.mu.Lock()
	_, watched = first.dirs[filepath.Clean(dir)]
	first.mu.Unlock()
	if watched {
		t.Fatal("deleted library root must be unwatched")
	}
}

func TestKillWatcherRemoveEvent(t *testing.T) {
	srv := testServer(t)
	w, _ := newTestWatcher(t, srv, time.Hour)
	dir := t.TempDir()
	w.watchTree(dir, "lib1")
	// A remove event forgets the dir so a recycled path cannot
	// mis-attribute later events to this library.
	w.onEvent(fsnotify.Event{Name: dir, Op: fsnotify.Remove})
	w.mu.Lock()
	_, ok := w.dirs[filepath.Clean(dir)]
	w.mu.Unlock()
	if ok {
		t.Fatal("removed dir must leave the watch map")
	}
}

func TestKillNewAdoptsCLIBounds(t *testing.T) {
	dir := t.TempDir()
	srv, err := NewWithOptions(dir, "test", Options{
		transcodeProbe:      noTranscodeProbe,
		TranscodeCacheBytes: 5 << 30,
		TranscodeQueueSize:  9,
	})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { srv.Close() })
	admin := setupAdmin(t, srv)
	rec := do(t, srv, "GET", "/api/admin/settings/transcode", nil, admin)
	if rec.Code != 200 {
		t.Fatalf("settings: %d %s", rec.Code, rec.Body.String())
	}
	var got struct {
		Settings struct {
			CacheBytes int64 `json:"cache_bytes"`
			QueueSize  int   `json:"queue_size"`
		} `json:"settings"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	if got.Settings.CacheBytes != 5<<30 || got.Settings.QueueSize != 9 {
		t.Fatalf("CLI bounds must seed the first-boot policy: %+v", got.Settings)
	}
}

func TestKillSwapStatusCodes(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	// Current generation first.
	rec := do(t, srv, "GET", "/api/plugins", nil, admin)
	if rec.Code != 200 {
		t.Fatalf("plugins: %d", rec.Code)
	}
	var comp struct {
		Generation uint64 `json:"generation"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &comp); err != nil {
		t.Fatal(err)
	}
	// Stale generation -> 409.
	rec = do(t, srv, "POST", "/api/plugins/swap", map[string]any{
		"capability": contracts.CapSourceEnumerate, "providers": []string{}, "generation": comp.Generation + 9,
	}, admin)
	if rec.Code != 409 {
		t.Fatalf("stale generation -> %d %s", rec.Code, rec.Body.String())
	}
	// Unknown capability/provider -> a 4xx rejection (not a panic).
	rec = do(t, srv, "POST", "/api/plugins/swap", map[string]any{
		"capability": "no.such.cap@1", "providers": []string{"ghost"}, "generation": comp.Generation,
	}, admin)
	if rec.Code == 200 || rec.Code >= 500 {
		t.Fatalf("bad swap -> %d %s", rec.Code, rec.Body.String())
	}
}

func TestKillEnrichBatchLimit(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	if rec := do(t, srv, "GET", "/api/enrichments", nil, admin); rec.Code != 400 {
		t.Fatalf("missing ids -> %d", rec.Code)
	}
	var sb strings.Builder
	for i := 0; i <= maxEnrichBatch; i++ {
		if i > 0 {
			sb.WriteByte(',')
		}
		sb.WriteString("id")
		sb.WriteString(strconv.Itoa(i))
	}
	if rec := do(t, srv, "GET", "/api/enrichments?ids="+sb.String(), nil, admin); rec.Code != 400 {
		t.Fatalf("over-limit ids -> %d", rec.Code)
	}
}

func TestKillUserPatchGuards(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	var me struct {
		ID string `json:"id"`
	}
	rec := do(t, srv, "GET", "/api/me", nil, admin)
	if err := json.Unmarshal(rec.Body.Bytes(), &me); err != nil || me.ID == "" {
		t.Fatalf("me: %s", rec.Body.String())
	}
	// Self-demotion is refused.
	if rec := do(t, srv, "PATCH", "/api/users/"+me.ID, map[string]any{"role": "user"}, admin); rec.Code != 400 {
		t.Fatalf("self-demote -> %d %s", rec.Code, rec.Body.String())
	}
	// Unknown user id -> 4xx.
	if rec := do(t, srv, "PATCH", "/api/users/ghost", map[string]any{"role": "user"}, admin); rec.Code != 400 {
		t.Fatalf("unknown user -> %d %s", rec.Code, rec.Body.String())
	}
}

func TestKillSettingsHasTranscodeCorrupt(t *testing.T) {
	srv := testServer(t)
	err := srv.db.Update(func(tx *bolt.Tx) error {
		return tx.Bucket(kv.BMeta).Put([]byte(transcodeSettingsKey), []byte("{not json"))
	})
	if err != nil {
		t.Fatal(err)
	}
	if ok, err := srv.settings.HasTranscode(); err == nil || ok {
		t.Fatalf("corrupt settings must surface an error: %v %v", ok, err)
	}
}

func TestKillTranscodeCancelValidation(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	// No session param -> invalid-message, no provider call.
	rec := do(t, srv, "DELETE", "/api/items/x/transcode", nil, admin)
	if rec.Code != 400 {
		t.Fatalf("no session -> %d %s", rec.Code, rec.Body.String())
	}
}

func TestKillTranscodeCancelBindsItem(t *testing.T) {
	if _, err := exec.LookPath("ffmpeg"); err != nil {
		t.Skip("ffmpeg unavailable")
	}
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dirA, dirB := t.TempDir(), t.TempDir()
	idA := catalogOneMKV(t, srv, admin, dirA, "[Fansub-A] Show A.mkv")
	// A second item in another library gives a different FilePath.
	mk := exec.Command("ffmpeg", "-hide_banner", "-loglevel", "error",
		"-f", "lavfi", "-i", "testsrc=size=160x120:rate=10:duration=2",
		"-c:v", "libx264", "-pix_fmt", "yuv420p", "-y", filepath.Join(dirB, "[Fansub-A] Show B.mkv"))
	if out, err := mk.CombinedOutput(); err != nil {
		t.Skipf("cannot synthesize mkv: %v: %s", err, out)
	}
	if rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "B", "type": "movie", "path": dirB}, admin); rec.Code != 201 {
		t.Fatalf("library B: %d %s", rec.Code, rec.Body.String())
	}
	if rec := do(t, srv, "POST", "/api/library/scan", nil, admin); rec.Code != 202 {
		t.Fatalf("scan: %d %s", rec.Code, rec.Body.String())
	}
	waitScan(t, srv, admin)
	var page contracts.CatalogPage
	rec := do(t, srv, "GET", "/api/catalog", nil, admin)
	if err := json.Unmarshal(rec.Body.Bytes(), &page); err != nil {
		t.Fatal(err)
	}
	idB := ""
	for _, it := range page.Items {
		if strings.HasPrefix(it.FilePath, dirB) {
			idB = it.ID
		}
	}
	if idB == "" || idB == idA {
		t.Fatalf("item B not cataloged: %s", rec.Body.String())
	}

	rec = do(t, srv, "POST", "/api/items/"+idA+"/transcode", nil, admin)
	if rec.Code != 202 && rec.Code != 200 {
		t.Fatalf("start: %d %s", rec.Code, rec.Body.String())
	}
	var st struct {
		Session string `json:"session"`
		State   string `json:"state"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &st); err != nil || st.Session == "" {
		t.Fatalf("start body: %s", rec.Body.String())
	}
	// Cancelling through the WRONG item must not touch the session:
	// the gateway binds file_path to the path item, and the provider
	// refuses a mismatch.
	rec = do(t, srv, "DELETE", "/api/items/"+idB+"/transcode?session="+st.Session, nil, admin)
	if rec.Code != 200 {
		t.Fatalf("wrong-item cancel: %d %s", rec.Code, rec.Body.String())
	}
	rec = do(t, srv, "GET", "/api/items/"+idA+"/transcode/status?session="+st.Session, nil, admin)
	var after struct {
		State string `json:"state"`
	}
	_ = json.Unmarshal(rec.Body.Bytes(), &after)
	if after.State == contracts.TranscodeIdle || after.State == "" {
		t.Fatalf("session must survive a cancel bound to another item: %s", rec.Body.String())
	}
	// Cancelling through the owning item works.
	rec = do(t, srv, "DELETE", "/api/items/"+idA+"/transcode?session="+st.Session, nil, admin)
	if rec.Code != 200 {
		t.Fatalf("own-item cancel: %d %s", rec.Code, rec.Body.String())
	}
	// A cancelled session leaves the jobs map entirely: status reports
	// idle or not-found, never an active state.
	rec = do(t, srv, "GET", "/api/items/"+idA+"/transcode/status?session="+st.Session, nil, admin)
	var final struct {
		State string `json:"state"`
	}
	_ = json.Unmarshal(rec.Body.Bytes(), &final)
	if final.State == contracts.TranscodeQueued || final.State == contracts.TranscodeRunning || final.State == contracts.TranscodeReady {
		t.Fatalf("cancelled session must not stay active: %s", rec.Body.String())
	}
}

func TestKillSubtitleStreamIndex(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	id := catalogOneMKV(t, srv, admin, t.TempDir(), "[Fansub-A] Show.mkv")
	for _, q := range []string{"stream=-1", "stream=abc"} {
		rec := do(t, srv, "GET", "/api/items/"+id+"/subtitles?"+q, nil, admin)
		if rec.Code != 400 {
			t.Fatalf("%s -> %d %s", q, rec.Code, rec.Body.String())
		}
	}
	// Neither session nor stream -> invalid-message.
	rec := do(t, srv, "GET", "/api/items/"+id+"/subtitles", nil, admin)
	if rec.Code != 400 {
		t.Fatalf("no selector -> %d %s", rec.Code, rec.Body.String())
	}
}

func TestKillPartialRestrictionKeepsTranscode(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	id := catalogOneMKV(t, srv, admin, t.TempDir(), "[Fansub-A] Show.mkv")
	rec := do(t, srv, "POST", "/api/users", map[string]string{"username": "bea", "password": "password123"}, admin)
	if rec.Code != http.StatusCreated {
		t.Fatalf("create user: %d %s", rec.Code, rec.Body.String())
	}
	var created struct {
		ID string `json:"id"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &created); err != nil {
		t.Fatal(err)
	}
	// Video transcode off but remux on: the && must not collapse to ||,
	// or remux-capable users would be refused.
	rec = do(t, srv, "PATCH", "/api/users/"+created.ID, map[string]any{
		"playback": map[string]any{"allow_video_transcode": false, "allow_remux": true},
	}, admin)
	if rec.Code != 200 {
		t.Fatalf("patch: %d %s", rec.Code, rec.Body.String())
	}
	user := loginAs(t, srv, "bea", "password123")
	rec = do(t, srv, "GET", "/api/items/"+id+"/playback?client=web", nil, user)
	var plan contracts.Plan
	if err := json.Unmarshal(rec.Body.Bytes(), &plan); err != nil {
		t.Fatal(err)
	}
	if !plan.Available || plan.Mode == "transcode-required" {
		t.Fatalf("remux-capable user must not be refused: %+v", plan)
	}
}

// --- playlist rewrite edges (transcode.go:476-498) ---

func TestKillRewritePlaylistEdges(t *testing.T) {
	raw := "#EXTM3U\n" +
		"   \n" + // whitespace-only line survives verbatim
		"#EXT-X-VERSION:7\n" +
		"#EXT-X-MAP:URI=\"init.mp4\"\n" +
		"#EXT-X-MAP:BYTERANGE=\"720@0\"\n" + // MAP without URI stays raw
		"#EXT-X-MAP:URI=\"broken.mp4\n" + // unterminated quote stays raw
		"seg00000.m4s\n"
	out := rewriteHLSPlaylist(raw, "sess", "tok")
	lines := strings.Split(out, "\n")
	if lines[1] != "   " {
		t.Fatalf("whitespace line rewritten: %q", lines[1])
	}
	if lines[2] != "#EXT-X-VERSION:7" {
		t.Fatalf("tag rewritten: %q", lines[2])
	}
	if !strings.Contains(lines[3], `URI="init.mp4?session=sess&token=tok"`) {
		t.Fatalf("map uri not signed: %q", lines[3])
	}
	if lines[4] != `#EXT-X-MAP:BYTERANGE="720@0"` {
		t.Fatalf("uri-less map rewritten: %q", lines[4])
	}
	if lines[5] != `#EXT-X-MAP:URI="broken.mp4` {
		t.Fatalf("unterminated map rewritten: %q", lines[5])
	}
	if !strings.Contains(lines[6], "seg00000.m4s?session=sess&token=tok") {
		t.Fatalf("segment not signed: %q", lines[6])
	}
}

// --- HLS content types (transcode.go:428-431) ---

func TestKillHLSContentTypes(t *testing.T) {
	if _, err := exec.LookPath("ffmpeg"); err != nil {
		t.Skip("ffmpeg unavailable")
	}
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	id := catalogOneMKV(t, srv, admin, t.TempDir(), "Show.mkv")

	rec := do(t, srv, "POST", "/api/items/"+id+"/transcode", map[string]any{"delivery": "hls"}, admin)
	if rec.Code != 200 && rec.Code != 202 {
		t.Fatalf("start: %d %s", rec.Code, rec.Body.String())
	}
	var status struct {
		Session string `json:"session"`
		State   string `json:"state"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &status); err != nil || status.Session == "" {
		t.Fatalf("start: %s", rec.Body.String())
	}
	deadline := time.Now().Add(25 * time.Second)
	for status.State != contracts.TranscodeReady && time.Now().Before(deadline) {
		time.Sleep(30 * time.Millisecond)
		rec = do(t, srv, "GET", "/api/items/"+id+"/transcode/status?session="+status.Session, nil, admin)
		if err := json.Unmarshal(rec.Body.Bytes(), &status); err != nil {
			t.Fatal(err)
		}
	}
	if status.State != contracts.TranscodeReady {
		t.Fatalf("session never became ready: %+v", status)
	}
	base := "/api/items/" + id + "/transcode/hls/"
	q := "?session=" + status.Session + "&token=" + admin

	rec = do(t, srv, "GET", base+"index.m3u8"+q, nil, "")
	playlist := rec.Body.String()
	if rec.Code != 200 || !strings.Contains(playlist, "#EXTM3U") {
		t.Fatalf("playlist: %d %s", rec.Code, playlist)
	}
	if ct := rec.Header().Get("Content-Type"); !strings.Contains(ct, "mpegurl") && !strings.Contains(ct, "m3u") {
		t.Fatalf("playlist content-type: %q", ct)
	}
	initURI := playlistMapURI(t, playlist)
	rec = do(t, srv, "GET", base+initURI, nil, "")
	if rec.Code != 200 {
		t.Fatalf("init: %d %s", rec.Code, rec.Body.String())
	}
	if ct := rec.Header().Get("Content-Type"); ct != "video/mp4" {
		t.Fatalf("init.mp4 content-type: %q", ct)
	}
	segURI := playlistSegmentURI(t, playlist)
	rec = do(t, srv, "GET", base+segURI, nil, "")
	if rec.Code != 200 {
		t.Fatalf("segment: %d %s", rec.Code, rec.Body.String())
	}
	if ct := rec.Header().Get("Content-Type"); ct != "video/iso.segment" {
		t.Fatalf("segment content-type: %q", ct)
	}
	// Bad file names are rejected before any filesystem touch.
	for _, name := range []string{"..%2f..%2fetc", "playlist.m3u8.exe", "foo.txt"} {
		if rec := do(t, srv, "GET", base+name+q, nil, ""); rec.Code != 404 {
			t.Fatalf("hls file %q -> %d", name, rec.Code)
		}
	}
}

// --- transcode v1 bad result (transcode.go:368-374) ---

type stubTranscodeV1 struct{ result contracts.Transcode }

func (stubTranscodeV1) ID() string { return "lain-transcode-stub" }
func (stubTranscodeV1) Capabilities() []string {
	return []string{contracts.CapPlaybackTranscode}
}
func (stubTranscodeV1) Health() error { return nil }
func (s stubTranscodeV1) Invoke(cap string, _ any) (any, error) {
	return s.result, nil
}

func TestKillTranscodeBadResult(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	id := catalogOneMKV(t, srv, admin, t.TempDir(), "Show.mkv")
	srv.Registry().Register(stubTranscodeV1{result: contracts.Transcode{}})
	if _, err := srv.Registry().Swap(contracts.CapPlaybackTranscode, []string{"lain-transcode-stub"}, 0); err != nil {
		t.Fatalf("swap: %v", err)
	}
	rec := do(t, srv, "GET", "/api/items/"+id+"/transcode", nil, admin)
	if rec.Code != 500 {
		t.Fatalf("empty prepared path must 500, got %d %s", rec.Code, rec.Body.String())
	}
}

// --- composition.json migration on boot (server.go:152-160) ---

func TestKillCompositionBootBranches(t *testing.T) {
	// Corrupt composition.json: Load fails, defaults are used, the
	// server still boots.
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "composition.json"), []byte("{nope"), 0o600); err != nil {
		t.Fatal(err)
	}
	srv, err := NewWithOptions(dir, "test", Options{transcodeProbe: noTranscodeProbe})
	if err != nil {
		t.Fatalf("corrupt composition must not block boot: %v", err)
	}
	srv.Close()

	// Empty bindings: comp stays default; a mutant that adopts it
	// (`len >= 0`) fails validation and breaks the boot.
	dir = t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "composition.json"), []byte(`{"version":5,"bindings":{}}`), 0o600); err != nil {
		t.Fatal(err)
	}
	srv, err = NewWithOptions(dir, "test", Options{transcodeProbe: noTranscodeProbe})
	if err != nil {
		t.Fatalf("empty composition must not be adopted: %v", err)
	}
	srv.Close()

	// A retired provider id is migrated in place on load.
	dir = t.TempDir()
	comp := `{"version":5,"bindings":{"lain.catalog.read@1":{"mode":"exactly-one","providers":["lain-catalog-file"],"generation":1},"lain.catalog.write@1":{"mode":"exactly-one","providers":["lain-catalog-file"],"generation":1}}}`
	if err := os.WriteFile(filepath.Join(dir, "composition.json"), []byte(comp), 0o600); err != nil {
		t.Fatal(err)
	}
	srv, err = NewWithOptions(dir, "test", Options{transcodeProbe: noTranscodeProbe})
	if err != nil {
		t.Fatalf("migrated composition must boot: %v", err)
	}
	t.Cleanup(func() { srv.Close() })
	view := srv.Registry().Composition().View()
	raw, _ := json.Marshal(view)
	if !strings.Contains(string(raw), "lain-catalog-bolt") || strings.Contains(string(raw), "lain-catalog-file") {
		t.Fatalf("retired provider id must be migrated: %s", raw)
	}
}

// --- enrich: provider filter, skipped providers, cache (enrich.go) ---

func TestKillEnrichProviderFilter(t *testing.T) {
	srv := testServer(t)
	nfo := &fakeMeta{id: "lain-metadata-nfo", candidates: []contracts.MetadataCandidate{
		{Provider: "lain-metadata-nfo", RemoteID: "n1", Title: "Frieren"},
	}}
	kitsu := &fakeMeta{id: "lain-metadata-kitsu", candidates: []contracts.MetadataCandidate{
		{Provider: "lain-metadata-kitsu", RemoteID: "k1", Title: "Frieren"},
	}}
	withFakeMetadata(t, srv, nfo, kitsu,
		&fakeMeta{id: "lain-metadata-anilist", failSearch: true},
		&fakeMeta{id: "lain-metadata-jikan", failSearch: true},
		&fakeMeta{id: "lain-metadata-tvmaze", failSearch: true},
	)

	// An unfiltered search populates the cache.
	first, err := srv.searchMetadata("frieren", "anime", "", "")
	if err != nil || len(first) == 0 {
		t.Fatalf("first search: %+v %v", first, err)
	}
	// only=<id> bypasses the cache and selects just that provider.
	nfo.candidates = []contracts.MetadataCandidate{{Provider: "lain-metadata-nfo", RemoteID: "n2", Title: "Frieren"}}
	only, err := srv.searchMetadata("frieren", "anime", "", "lain-metadata-nfo")
	if err != nil || len(only) != 1 || only[0].RemoteID != "n2" {
		t.Fatalf("only= must skip the cache and filter: %+v %v", only, err)
	}
	only, err = srv.searchMetadata("frieren", "anime", "", "lain-metadata-kitsu")
	if err != nil {
		t.Fatal(err)
	}
	if len(only) != 1 || only[0].Provider != "lain-metadata-kitsu" {
		t.Fatalf("only=kitsu: %+v", only)
	}
	// only=<unknown> merges nothing — never a false cache write.
	only, err = srv.searchMetadata("frieren", "anime", "", "lain-metadata-ghost")
	if err != nil || len(only) != 0 {
		t.Fatalf("only=ghost must merge nothing: %+v %v", only, err)
	}
	// Unfiltered searches are served from the cache afterwards.
	nfo.candidates = nil // provider answers must not matter anymore
	cached, err := srv.searchMetadata("frieren", "anime", "", "")
	if err != nil {
		t.Fatal(err)
	}
	if len(cached) != len(first) || cached[0].RemoteID != first[0].RemoteID {
		t.Fatalf("cached search must return the first merge: %+v vs %+v", cached, first)
	}
}
