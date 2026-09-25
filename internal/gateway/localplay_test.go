package gateway

import (
	"bytes"
	"encoding/json"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/plugins/catalog"
)

// doAddr is do() with a caller-controlled RemoteAddr — the local-play
// contract hinges on the transport peer being loopback.
func doAddr(t *testing.T, srv *Server, method, path, remote string, body any, token string) *httptest.ResponseRecorder {
	t.Helper()
	var rdr *bytes.Reader
	if body != nil {
		raw, _ := json.Marshal(body)
		rdr = bytes.NewReader(raw)
	} else {
		rdr = bytes.NewReader(nil)
	}
	req := httptest.NewRequest(method, path, rdr)
	req.RemoteAddr = remote
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}
	rec := httptest.NewRecorder()
	srv.Handler().ServeHTTP(rec, req)
	return rec
}

func seedItem(t *testing.T, srv *Server, path, title string, episode int) contracts.CatalogItem {
	t.Helper()
	out, _, err := srv.reg.CallOne(contracts.CapCatalogWrite, catalog.UpsertInput{
		LibraryID: "lib-local",
		Proposal:  contracts.Proposal{Kind: "anime", Title: title, Season: 1, Episode: episode, Confidence: 0.9, PluginID: "test"},
		Candidate: contracts.Candidate{Path: path, Size: 100, ModTime: 1, LibraryID: "lib-local"},
	})
	if err != nil {
		t.Fatal(err)
	}
	return out.(contracts.CatalogItem)
}

func localTestManager(t *testing.T, srv *Server, script string) {
	t.Helper()
	bin := filepath.Join(t.TempDir(), "mpv")
	if err := os.WriteFile(bin, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	srv.local.LookPath = func(string) (string, error) { return bin, nil }
	srv.local.Getenv = func(k string) string {
		if k == "DISPLAY" {
			return ":0"
		}
		return ""
	}
}

func TestLocalPlayCapabilityIsLoopbackOnly(t *testing.T) {
	srv := testServer(t)
	token := setupAdmin(t, srv)
	localTestManager(t, srv, "#!/bin/sh\nexit 0\n")

	rec := doAddr(t, srv, "GET", "/api/localplay", "192.0.2.9:4444", nil, token)
	if rec.Code != 200 {
		t.Fatalf("capability: %d %s", rec.Code, rec.Body.String())
	}
	var cap struct {
		Players []string `json:"players"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &cap); err != nil {
		t.Fatal(err)
	}
	if len(cap.Players) != 0 {
		t.Fatalf("remote peer must see no players: %v", cap.Players)
	}

	rec = doAddr(t, srv, "GET", "/api/localplay", "127.0.0.1:4444", nil, token)
	if err := json.Unmarshal(rec.Body.Bytes(), &cap); err != nil {
		t.Fatal(err)
	}
	if len(cap.Players) != 2 || cap.Players[0] != "mpv" {
		t.Fatalf("loopback peer must see players: %v", cap.Players)
	}

	// A loopback peer behind a proxy is not proof the viewer is local.
	req := httptest.NewRequest("GET", "/api/localplay", nil)
	req.RemoteAddr = "127.0.0.1:4444"
	req.Header.Set("Authorization", "Bearer "+token)
	req.Header.Set("X-Forwarded-For", "203.0.113.5")
	rec = httptest.NewRecorder()
	srv.Handler().ServeHTTP(rec, req)
	if err := json.Unmarshal(rec.Body.Bytes(), &cap); err != nil {
		t.Fatal(err)
	}
	if len(cap.Players) != 0 {
		t.Fatalf("forwarded header must hide players: %v", cap.Players)
	}
}

func TestLocalPlayEndToEndRecordsProgress(t *testing.T) {
	srv := testServer(t)
	token := setupAdmin(t, srv)
	media := filepath.Join(t.TempDir(), "[Fansub-A] Frieren - 12 [1080p].mkv")
	if err := os.WriteFile(media, []byte("video"), 0o644); err != nil {
		t.Fatal(err)
	}
	it := seedItem(t, srv, media, "Frieren", 12)
	localTestManager(t, srv, `#!/bin/sh
state=""
for arg in "$@"; do
  case "$arg" in
    --script-opt=lain-state_dir=*) state="${arg#--script-opt=lain-state_dir=}" ;;
  esac
done
printf '{"position_sec":42,"duration_sec":120}' > "$state/0.json"
`)

	rec := doAddr(t, srv, "POST", "/api/items/"+it.ID+"/play-local", "127.0.0.1:4444", map[string]string{"player": "mpv"}, token)
	if rec.Code != 200 {
		t.Fatalf("play-local: %d %s", rec.Code, rec.Body.String())
	}
	var resp struct {
		Status string `json:"status"`
		Player string `json:"player"`
		Count  int    `json:"count"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &resp); err != nil {
		t.Fatal(err)
	}
	if resp.Status != "playing" || resp.Player != "mpv" || resp.Count != 1 {
		t.Fatalf("unexpected response: %+v", resp)
	}

	deadline := time.Now().Add(5 * time.Second)
	for {
		rec = do(t, srv, "GET", "/api/items/"+it.ID+"/progress", nil, token)
		if rec.Code == 200 {
			var p contracts.Progress
			if err := json.Unmarshal(rec.Body.Bytes(), &p); err == nil && p.PositionSec == 42 {
				return
			}
		}
		if time.Now().After(deadline) {
			t.Fatalf("progress never landed: last=%d %s", rec.Code, rec.Body.String())
		}
		time.Sleep(50 * time.Millisecond)
	}
}

func TestLocalPlayRemoteForbidden(t *testing.T) {
	srv := testServer(t)
	token := setupAdmin(t, srv)
	rec := doAddr(t, srv, "POST", "/api/items/whatever/play-local", "192.0.2.9:4444", map[string]string{}, token)
	if rec.Code != 403 {
		t.Fatalf("remote play-local must be 403: %d", rec.Code)
	}
	rec = do(t, srv, "POST", "/api/items/whatever/play-local", map[string]string{}, "")
	if rec.Code != 401 {
		t.Fatalf("unauthenticated play-local must be 401: %d", rec.Code)
	}
}

func TestLocalPlayUnknownAndMissing(t *testing.T) {
	srv := testServer(t)
	token := setupAdmin(t, srv)
	localTestManager(t, srv, "#!/bin/sh\nexit 0\n")

	rec := doAddr(t, srv, "POST", "/api/items/nope/play-local", "127.0.0.1:4444", map[string]string{}, token)
	if rec.Code != 404 {
		t.Fatalf("unknown item: %d %s", rec.Code, rec.Body.String())
	}

	gone := seedItem(t, srv, filepath.Join(t.TempDir(), "deleted.mkv"), "Gone", 0)
	rec = doAddr(t, srv, "POST", "/api/items/"+gone.ID+"/play-local", "127.0.0.1:4444", map[string]string{}, token)
	if rec.Code != 409 {
		t.Fatalf("missing file must be 409 (D-068): %d %s", rec.Code, rec.Body.String())
	}
}

func TestLocalPlayNoPlayerAndBusy(t *testing.T) {
	srv := testServer(t)
	token := setupAdmin(t, srv)
	media := filepath.Join(t.TempDir(), "film.mkv")
	if err := os.WriteFile(media, []byte("video"), 0o644); err != nil {
		t.Fatal(err)
	}
	it := seedItem(t, srv, media, "Film", 0)

	// No display: capability empty, play-local refuses with 503.
	srv.local.Getenv = func(string) string { return "" }
	rec := doAddr(t, srv, "POST", "/api/items/"+it.ID+"/play-local", "127.0.0.1:4444", map[string]string{}, token)
	if rec.Code != 503 {
		t.Fatalf("no player: %d %s", rec.Code, rec.Body.String())
	}

	localTestManager(t, srv, "#!/bin/sh\nsleep 30\n")
	bin := filepath.Join(t.TempDir(), "mpv")
	if err := os.WriteFile(bin, []byte("#!/bin/sh\nsleep 30\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	srv.local.LookPath = func(name string) (string, error) {
		if name != "mpv" {
			return "", exec.ErrNotFound
		}
		return bin, nil
	}
	rec = doAddr(t, srv, "POST", "/api/items/"+it.ID+"/play-local", "127.0.0.1:4444", map[string]string{"player": "vlc"}, token)
	if rec.Code != 400 {
		t.Fatalf("vlc binary absent (fake is mpv): %d %s", rec.Code, rec.Body.String())
	}
	rec = doAddr(t, srv, "POST", "/api/items/"+it.ID+"/play-local", "127.0.0.1:4444", map[string]string{}, token)
	if rec.Code != 200 {
		t.Fatalf("play: %d %s", rec.Code, rec.Body.String())
	}
	rec = doAddr(t, srv, "POST", "/api/items/"+it.ID+"/play-local", "127.0.0.1:4444", map[string]string{}, token)
	if rec.Code != 409 {
		t.Fatalf("second play must be busy: %d %s", rec.Code, rec.Body.String())
	}
	srv.local.Close()
	rec = doAddr(t, srv, "POST", "/api/items/"+it.ID+"/play-local", "127.0.0.1:4444", map[string]string{}, token)
	if rec.Code != 200 {
		t.Fatalf("after Close the slot must free: %d %s", rec.Code, rec.Body.String())
	}
}
