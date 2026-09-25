package gateway

import (
	"encoding/json"
	"fmt"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/enrell/lain/internal/contracts"
)

// subsRequireFFmpeg skips unless the local build has both tools the
// on-demand extraction needs: ffprobe reads the track, ffmpeg converts.
func subsRequireFFmpeg(t *testing.T) {
	t.Helper()
	if _, err := exec.LookPath("ffmpeg"); err != nil {
		t.Skip("ffmpeg not installed")
	}
	if _, err := exec.LookPath("ffprobe"); err != nil {
		t.Skip("ffprobe not installed")
	}
}

// subsErrorCode reads the stable {"error","code"} envelope of a failed
// transcode endpoint so the assertion names the contract, not the text.
func subsErrorCode(t *testing.T, rec *httptest.ResponseRecorder) string {
	t.Helper()
	var body struct {
		Code string `json:"code"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &body); err != nil {
		t.Fatalf("error body: %d %s", rec.Code, rec.Body.String())
	}
	return body.Code
}

// subsCatalogStub catalogs one junk .mkv so the request-validation paths
// run without ffmpeg.
func subsCatalogStub(t *testing.T, srv *Server, admin string) string {
	t.Helper()
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "[Fansub-A] Stub Show.mkv"), []byte("not media"), 0o600); err != nil {
		t.Fatal(err)
	}
	if rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "SUBS", "type": "movie", "path": dir}, admin); rec.Code != 201 {
		t.Fatalf("library: %d %s", rec.Code, rec.Body.String())
	}
	if rec := do(t, srv, "POST", "/api/library/scan", nil, admin); rec.Code != 202 {
		t.Fatalf("scan: %d %s", rec.Code, rec.Body.String())
	}
	waitScan(t, srv, admin)
	var page contracts.CatalogPage
	rec := do(t, srv, "GET", "/api/catalog", nil, admin)
	if err := json.Unmarshal(rec.Body.Bytes(), &page); err != nil || len(page.Items) != 1 {
		t.Fatalf("catalog: %d %s", rec.Code, rec.Body.String())
	}
	return page.Items[0].ID
}

// subsCatalogMKV synthesizes a subbed H.264/AAC Matroska, catalogs it,
// and returns the item id, its directory and the subtitle stream index
// the playback plan reports.
func subsCatalogMKV(t *testing.T, srv *Server, admin string) (id, dir string, index int) {
	t.Helper()
	subsRequireFFmpeg(t)
	dir = t.TempDir()
	srt := filepath.Join(dir, "subs.srt")
	if err := os.WriteFile(srt, []byte("1\n00:00:00,000 --> 00:00:01,000\nHello from [Fansub-A]\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	out := filepath.Join(dir, "[Fansub-A] Subbed Show.mkv")
	args := []string{"-hide_banner", "-loglevel", "error",
		"-f", "lavfi", "-i", "testsrc=size=160x120:rate=10:duration=2",
		"-f", "lavfi", "-i", "sine=frequency=440:sample_rate=22050:duration=2",
		"-i", srt,
		"-map", "0:v", "-map", "1:a", "-map", "2:s",
		"-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-c:s", "srt",
		"-y", out}
	if combo, err := exec.Command("ffmpeg", args...).CombinedOutput(); err != nil {
		t.Skipf("cannot synthesize subbed mkv: %v: %s", err, combo)
	}

	if rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "SUBS", "type": "movie", "path": dir}, admin); rec.Code != 201 {
		t.Fatalf("library: %d %s", rec.Code, rec.Body.String())
	}
	if rec := do(t, srv, "POST", "/api/library/scan", nil, admin); rec.Code != 202 {
		t.Fatalf("scan: %d %s", rec.Code, rec.Body.String())
	}
	waitScan(t, srv, admin)
	var page contracts.CatalogPage
	rec := do(t, srv, "GET", "/api/catalog", nil, admin)
	if err := json.Unmarshal(rec.Body.Bytes(), &page); err != nil || len(page.Items) != 1 {
		t.Fatalf("catalog: %d %s", rec.Code, rec.Body.String())
	}
	id = page.Items[0].ID

	var plan contracts.Plan
	rec = do(t, srv, "GET", "/api/items/"+id+"/playback?client=web", nil, admin)
	if err := json.Unmarshal(rec.Body.Bytes(), &plan); err != nil {
		t.Fatalf("plan: %d %s", rec.Code, rec.Body.String())
	}
	index = -1
	for _, s := range plan.Streams {
		if s.Type == "subtitle" {
			index = s.Index
		}
	}
	if index < 0 {
		t.Fatalf("plan has no subtitle stream: %+v", plan.Streams)
	}
	return id, dir, index
}

// TestSubtitlesStreamRequiresSessionOrStream pins the 400 when the
// handler cannot tell what to serve.
func TestSubtitlesStreamRequiresSessionOrStream(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	id := subsCatalogStub(t, srv, admin)

	rec := do(t, srv, "GET", "/api/items/"+id+"/subtitles", nil, admin)
	if rec.Code != 400 {
		t.Fatalf("no session or stream: %d %s, want 400", rec.Code, rec.Body.String())
	}
	if code := subsErrorCode(t, rec); code != "invalid-message" {
		t.Fatalf("code=%q, want invalid-message", code)
	}
}

// TestSubtitlesStreamRejectsNonNumericIndex pins the parse guard: a
// stream that is not an index never reaches the plugin.
func TestSubtitlesStreamRejectsNonNumericIndex(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	id := subsCatalogStub(t, srv, admin)

	rec := do(t, srv, "GET", "/api/items/"+id+"/subtitles?stream=abc&token="+admin, nil, "")
	if rec.Code != 400 {
		t.Fatalf("stream=abc: %d %s, want 400", rec.Code, rec.Body.String())
	}
	if code := subsErrorCode(t, rec); code != "invalid-message" {
		t.Fatalf("code=%q, want invalid-message", code)
	}
}

// TestSubtitlesStreamRejectsUnknownTrack pins the plugin refusal over
// HTTP: a probed MKV without stream 99 comes back as 400 invalid-message
// (the handler maps invalid-message to 400; unsupported-media would be
// 422).
func TestSubtitlesStreamRejectsUnknownTrack(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	id := catalogOneMKV(t, srv, admin, t.TempDir(), "Show.mkv")

	rec := do(t, srv, "GET", "/api/items/"+id+"/subtitles?stream=99&token="+admin, nil, "")
	if rec.Code != 400 {
		t.Fatalf("unknown track: %d %s, want 400", rec.Code, rec.Body.String())
	}
	if code := subsErrorCode(t, rec); code != "invalid-message" {
		t.Fatalf("code=%q, want invalid-message", code)
	}
}

// TestSubtitlesStreamAuthAndUnknownItem pins the media-endpoint auth
// shape: unauthenticated is 401, unknown item is 404.
func TestSubtitlesStreamAuthAndUnknownItem(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)

	if rec := do(t, srv, "GET", "/api/items/nope/subtitles?stream=0", nil, ""); rec.Code != 401 {
		t.Fatalf("unauthenticated: %d, want 401", rec.Code)
	}
	if rec := do(t, srv, "GET", "/api/items/nope/subtitles?stream=0&token="+admin, nil, ""); rec.Code != 404 {
		t.Fatalf("unknown item: %d, want 404", rec.Code)
	}
}

// TestSubtitlesStreamServesExtractedWebVTT proves the on-the-fly path: a
// chosen SRT track is extracted and served as WebVTT, and the body never
// carries a filesystem path or the provider cache location.
func TestSubtitlesStreamServesExtractedWebVTT(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	id, dir, index := subsCatalogMKV(t, srv, admin)

	rec := do(t, srv, "GET", fmt.Sprintf("/api/items/%s/subtitles?stream=%d&token=%s", id, index, admin), nil, "")
	if rec.Code != 200 {
		t.Fatalf("subtitle extraction: %d %s", rec.Code, rec.Body.String())
	}
	if ct := rec.Header().Get("Content-Type"); ct != "text/vtt; charset=utf-8" {
		t.Fatalf("content-type %q, want text/vtt; charset=utf-8", ct)
	}
	body := rec.Body.String()
	if !strings.HasPrefix(body, "WEBVTT") || !strings.Contains(body, "Hello from [Fansub-A]") {
		t.Fatalf("bad WebVTT: %q", body)
	}
	if strings.Contains(body, dir) || strings.Contains(body, "transcodes") || strings.Contains(body, ".vtt") {
		t.Fatalf("subtitle body leaks a filesystem path: %q", body)
	}
}

// TestSubtitlesStreamHonoursDisabledSetting pins the operator toggle
// over HTTP: with allow_subtitle_extraction=false the same request turns
// into a stable 403 forbidden.
func TestSubtitlesStreamHonoursDisabledSetting(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	id, _, index := subsCatalogMKV(t, srv, admin)

	rec := do(t, srv, "PUT", "/api/admin/settings/transcode", map[string]any{"allow_subtitle_extraction": false}, admin)
	if rec.Code != 200 {
		t.Fatalf("settings put: %d %s", rec.Code, rec.Body.String())
	}

	rec = do(t, srv, "GET", fmt.Sprintf("/api/items/%s/subtitles?stream=%d&token=%s", id, index, admin), nil, "")
	if rec.Code != 403 {
		t.Fatalf("disabled extraction: %d %s, want 403", rec.Code, rec.Body.String())
	}
	if code := subsErrorCode(t, rec); code != "forbidden" {
		t.Fatalf("code=%q, want forbidden", code)
	}
}

// TestSubtitlesStreamKeepsPlaybackPlan pins the invariant the feature
// exists for: fetching subtitles on the side must not change how the
// item itself is planned. For an MKV the browser plan is a remux
// ("transcode" mode: playback.browserDirect only direct-plays mp4/m4v/
// webm), so the assertion is that the mode is unchanged by the fetch.
func TestSubtitlesStreamKeepsPlaybackPlan(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	id, _, index := subsCatalogMKV(t, srv, admin)

	planNow := func() contracts.Plan {
		t.Helper()
		rec := do(t, srv, "GET", "/api/items/"+id+"/playback?client=web", nil, admin)
		if rec.Code != 200 {
			t.Fatalf("plan: %d %s", rec.Code, rec.Body.String())
		}
		var plan contracts.Plan
		if err := json.Unmarshal(rec.Body.Bytes(), &plan); err != nil {
			t.Fatalf("plan body: %s", rec.Body.String())
		}
		return plan
	}

	before := planNow()
	if rec := do(t, srv, "GET", fmt.Sprintf("/api/items/%s/subtitles?stream=%d&token=%s", id, index, admin), nil, ""); rec.Code != 200 {
		t.Fatalf("subtitle extraction: %d %s", rec.Code, rec.Body.String())
	}
	after := planNow()
	if before.Mode != after.Mode || !after.Available {
		t.Fatalf("plan changed after subtitle extraction: before=%+v after=%+v", before, after)
	}
}
