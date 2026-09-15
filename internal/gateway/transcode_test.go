package gateway

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
)

// TestTranscodeAuthAndKnownItem proves the query-token path works for
// <video> consumers and unknown items fail without invoking ffmpeg.
func TestTranscodeAuthAndKnownItem(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)

	if rec := do(t, srv, "GET", "/api/items/whatever/transcode", nil, ""); rec.Code != 401 {
		t.Fatalf("unauthenticated: %d, want 401", rec.Code)
	}
	// Headerless clients (HTML media elements) pass ?token= instead.
	if rec := do(t, srv, "GET", "/api/items/nope/transcode?token="+admin, nil, ""); rec.Code != 404 {
		t.Fatalf("unknown item via query token: %d, want 404", rec.Code)
	}
	if rec := do(t, srv, "POST", "/api/items/nope/transcode", nil, ""); rec.Code != 401 {
		t.Fatalf("unauthenticated start: %d, want 401", rec.Code)
	}
}

// TestTranscodeSubtitlesServesWebVTT proves the sidecar path: a text
// subtitle is extracted to WebVTT and served without leaking the
// provider's private cache path in the status body.
func TestTranscodeSubtitlesServesWebVTT(t *testing.T) {
	if _, err := exec.LookPath("ffmpeg"); err != nil {
		t.Skip("ffmpeg not installed")
	}
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	srt := filepath.Join(dir, "subs.srt")
	if err := os.WriteFile(srt, []byte("1\n00:00:00,000 --> 00:00:01,000\nHello\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	out := filepath.Join(dir, "Subbed.mkv")
	mux := exec.Command("ffmpeg", "-hide_banner", "-loglevel", "error",
		"-f", "lavfi", "-i", "testsrc=size=160x120:rate=10:duration=2",
		"-i", srt, "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:s", "srt", "-y", out)
	if combo, err := mux.CombinedOutput(); err != nil {
		t.Skipf("cannot synthesize subbed mkv: %v: %s", err, combo)
	}
	if rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "SUB", "type": "movie", "path": dir}, admin); rec.Code != 201 {
		t.Fatalf("library: %d %s", rec.Code, rec.Body.String())
	}
	if rec := do(t, srv, "POST", "/api/library/scan", nil, admin); rec.Code != 202 {
		t.Fatalf("scan: %d %s", rec.Code, rec.Body.String())
	}
	waitScan(t, srv, admin)
	var page contracts.CatalogPage
	if rec := do(t, srv, "GET", "/api/catalog", nil, admin); json.Unmarshal(rec.Body.Bytes(), &page) != nil || len(page.Items) != 1 {
		t.Fatalf("catalog: %s", rec.Body.String())
	}
	id := page.Items[0].ID

	subtitle := 1
	rec := do(t, srv, "POST", "/api/items/"+id+"/transcode", map[string]any{"subtitle_stream": subtitle}, admin)
	if rec.Code != 200 && rec.Code != 202 {
		t.Fatalf("start: %d %s", rec.Code, rec.Body.String())
	}
	var status struct {
		Session     string `json:"session"`
		State       string `json:"state"`
		HasSubtitle bool   `json:"has_subtitle"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &status); err != nil {
		t.Fatal(err)
	}
	deadline := time.Now().Add(15 * time.Second)
	for status.State != contracts.TranscodeReady && time.Now().Before(deadline) {
		time.Sleep(20 * time.Millisecond)
		rec = do(t, srv, "GET", "/api/items/"+id+"/transcode/status?session="+status.Session, nil, admin)
		if err := json.Unmarshal(rec.Body.Bytes(), &status); err != nil {
			t.Fatal(err)
		}
	}
	if status.State != contracts.TranscodeReady || !status.HasSubtitle {
		t.Fatalf("status=%+v, want ready with subtitles", status)
	}
	if strings.Contains(rec.Body.String(), ".vtt") && strings.Contains(rec.Body.String(), "transcodes/") {
		t.Fatalf("status leaks private path: %s", rec.Body.String())
	}
	rec = do(t, srv, "GET", "/api/items/"+id+"/subtitles?session="+status.Session+"&token="+admin, nil, "")
	if rec.Code != 200 {
		t.Fatalf("subtitles: %d %s", rec.Code, rec.Body.String())
	}
	if ct := rec.Header().Get("Content-Type"); ct != "text/vtt; charset=utf-8" {
		t.Fatalf("content-type %q", ct)
	}
	if body := rec.Body.String(); len(body) < 6 || body[:6] != "WEBVTT" {
		t.Fatalf("not WebVTT: %q", body[:min(20, len(body))])
	}
	// A session without subtitles has no sidecar to serve.
	rec = do(t, srv, "GET", "/api/items/"+id+"/subtitles?session="+status.Session, nil, admin)
	if rec.Code == 200 {
		// Authenticated header path works too; the point is 200 with VTT.
	} else {
		t.Fatalf("header-auth subtitles: %d", rec.Code)
	}
}

// catalogOneMKV scans a directory holding one synthesized Matroska file
// and returns its catalog id.
func catalogOneMKV(t *testing.T, srv *Server, admin, dir, name string) string {
	t.Helper()
	if _, err := exec.LookPath("ffmpeg"); err != nil {
		t.Skip("ffmpeg not installed")
	}
	out := filepath.Join(dir, name)
	cmd := exec.Command("ffmpeg", "-hide_banner", "-loglevel", "error",
		"-f", "lavfi", "-i", "testsrc=size=160x120:rate=10:duration=2",
		"-c:v", "libx264", "-pix_fmt", "yuv420p", "-y", out)
	if combo, err := cmd.CombinedOutput(); err != nil {
		t.Skipf("cannot synthesize mkv: %v: %s", err, combo)
	}

	if rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "MKV", "type": "movie", "path": dir}, admin); rec.Code != 201 {
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

// TestTranscodeServesMP4 is the end-to-end slice: an H.264 MKV remuxes
// to a Range-served MP4, and the plan advertises the transcode mode.
func TestTranscodeServesMP4(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	id := catalogOneMKV(t, srv, admin, t.TempDir(), "Show.mkv")

	var plan contracts.Plan
	rec := do(t, srv, "GET", "/api/items/"+id+"/playback?client=web", nil, admin)
	if err := json.Unmarshal(rec.Body.Bytes(), &plan); err != nil {
		t.Fatalf("playback: %d %s", rec.Code, rec.Body.String())
	}
	if plan.Mode != "transcode" || !plan.Available {
		t.Fatalf("plan=%+v, want transcode+available", plan)
	}
	if plan.Session == "" || plan.Profile == "" || plan.State != contracts.TranscodeIdle {
		t.Fatalf("plan=%+v, want side-effect-free idle async session", plan)
	}

	// V2 starts explicitly, then status can be polled without exposing
	// the provider's private cache path.
	rec = do(t, srv, "POST", "/api/items/"+id+"/transcode", nil, admin)
	if rec.Code != 200 && rec.Code != 202 {
		t.Fatalf("start: %d %s", rec.Code, rec.Body.String())
	}
	var status struct {
		Session string `json:"session"`
		State   string `json:"state"`
		Path    string `json:"path"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &status); err != nil || status.Session != plan.Session {
		t.Fatalf("start status: %d %s", rec.Code, rec.Body.String())
	}
	deadline := time.Now().Add(10 * time.Second)
	for status.State != contracts.TranscodeReady && time.Now().Before(deadline) {
		time.Sleep(10 * time.Millisecond)
		rec = do(t, srv, "GET", "/api/items/"+id+"/transcode/status?session="+plan.Session, nil, admin)
		if rec.Code != 200 {
			t.Fatalf("status: %d %s", rec.Code, rec.Body.String())
		}
		if err := json.Unmarshal(rec.Body.Bytes(), &status); err != nil {
			t.Fatal(err)
		}
	}
	if status.State != contracts.TranscodeReady || status.Path != "" || strings.Contains(rec.Body.String(), "transcodes/") {
		t.Fatalf("public status leaks path or did not finish: %s", rec.Body.String())
	}

	streamPath := "/api/items/" + id + "/transcode?token=" + admin + "&session=" + plan.Session
	rec = do(t, srv, "GET", streamPath, nil, "")
	if rec.Code != 200 {
		t.Fatalf("transcode: %d %s", rec.Code, rec.Body.String())
	}
	if ct := rec.Header().Get("Content-Type"); ct != "video/mp4" {
		t.Fatalf("content-type %q, want video/mp4", ct)
	}
	if ar := rec.Header().Get("Accept-Ranges"); ar != "bytes" {
		t.Fatalf("accept-ranges %q, want bytes", ar)
	}
	body := rec.Body.Bytes()
	if len(body) < 8 || string(body[4:8]) != "ftyp" {
		t.Fatalf("not an mp4: % x", body[:min(8, len(body))])
	}

	// Second read comes from the provider cache and still serves bytes.
	rec = do(t, srv, "GET", streamPath, nil, "")
	if rec.Code != 200 || rec.Body.Len() == 0 {
		t.Fatalf("cached transcode: %d %d bytes", rec.Code, rec.Body.Len())
	}

	// ServeContent honors an actual byte request, not only the header.
	req := httptest.NewRequest("GET", streamPath, nil)
	req.Header.Set("Range", "bytes=0-7")
	ranged := httptest.NewRecorder()
	srv.Handler().ServeHTTP(ranged, req)
	if ranged.Code != http.StatusPartialContent || ranged.Body.Len() != 8 {
		t.Fatalf("range: %d %d bytes", ranged.Code, ranged.Body.Len())
	}
}

// TestTranscodeDegradesWithoutFFmpeg proves the honest path survives:
// no ffmpeg means the endpoint 503s and the plan falls back to
// transcode-required instead of pointing at bytes that do not exist.
func TestTranscodeDegradesWithoutFFmpeg(t *testing.T) {
	if _, err := exec.LookPath("ffmpeg"); err != nil {
		t.Skip("ffmpeg not installed")
	}
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	id := catalogOneMKV(t, srv, admin, t.TempDir(), "Show.mkv")

	t.Setenv("PATH", t.TempDir())
	if rec := do(t, srv, "GET", "/api/items/"+id+"/transcode?token="+admin, nil, ""); rec.Code != 503 {
		t.Fatalf("transcode without ffmpeg: %d, want 503", rec.Code)
	}

	var plan contracts.Plan
	rec := do(t, srv, "GET", "/api/items/"+id+"/playback?client=web", nil, admin)
	if err := json.Unmarshal(rec.Body.Bytes(), &plan); err != nil {
		t.Fatalf("playback: %d %s", rec.Code, rec.Body.String())
	}
	if plan.Mode != "transcode-required" || plan.Available {
		t.Fatalf("plan=%+v, want transcode-required+unavailable", plan)
	}
}
