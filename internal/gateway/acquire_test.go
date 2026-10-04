package gateway

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/enrell/lain/internal/acquire"
	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/torrent"
	"github.com/enrell/lain/internal/torrent/trackertest"
)

func init() {
	// Gateway tests never bind the fixed BitTorrent port (51413): the
	// engine of every test server listens on a free port instead.
	os.Setenv("LAIN_ACQUIRE_LISTEN_PORT", "0")
}

func TestAcquireRoutesAreAdminOnly(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	wantCode(t, do(t, srv, "POST", "/api/users", map[string]string{"username": "ana", "password": "password123"}, admin), 201)
	user := loginAs(t, srv, "ana", "password123")
	for _, path := range []string{"/api/acquire/settings", "/api/acquire/indexers", "/api/acquire/grabs", "/api/acquire/search?q=x", "/api/acquire/parse?name=x"} {
		if rec := do(t, srv, "GET", path, nil, user); rec.Code != http.StatusForbidden {
			t.Errorf("%s as user: %d", path, rec.Code)
		}
		if rec := do(t, srv, "GET", path, nil, ""); rec.Code != http.StatusUnauthorized {
			t.Errorf("%s anonymous: %d", path, rec.Code)
		}
	}
}

func wantCode(t *testing.T, rec *httptest.ResponseRecorder, code int) {
	t.Helper()
	if rec.Code != code {
		t.Fatalf("status %d, want %d: %s", rec.Code, code, rec.Body.String())
	}
}

func TestAcquireIndexersNeverReturnKeys(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	rec := do(t, srv, "POST", "/api/acquire/indexers", map[string]any{
		"name": "tracker-exemplo", "protocol": "torznab", "url": "http://127.0.0.1:9/api", "api_key": "super-secret", "categories": []int{5070},
	}, admin)
	wantCode(t, rec, 201)
	if strings.Contains(rec.Body.String(), "super-secret") {
		t.Fatal("create echoed the API key")
	}
	var ix struct {
		ID        string `json:"id"`
		HasAPIKey bool   `json:"has_api_key"`
	}
	_ = json.Unmarshal(rec.Body.Bytes(), &ix)
	if !ix.HasAPIKey {
		t.Fatal("has_api_key must be reported")
	}
	list := do(t, srv, "GET", "/api/acquire/indexers", nil, admin)
	if strings.Contains(list.Body.String(), "super-secret") {
		t.Fatal("list leaked the API key")
	}
	// A failing health check is a result, not a failed request.
	test := do(t, srv, "POST", "/api/acquire/indexers/"+ix.ID+"/test", nil, admin)
	wantCode(t, test, 200)
	if strings.Contains(test.Body.String(), "super-secret") || !strings.Contains(test.Body.String(), `"ok":false`) {
		t.Fatalf("test: %s", test.Body.String())
	}
	wantCode(t, do(t, srv, "PATCH", "/api/acquire/indexers/"+ix.ID, map[string]any{"url": "ftp://x"}, admin), 400)
	wantCode(t, do(t, srv, "DELETE", "/api/acquire/indexers/"+ix.ID, nil, admin), 200)
	wantCode(t, do(t, srv, "DELETE", "/api/acquire/indexers/"+ix.ID, nil, admin), 404)
	// Search with no indexer explains what to do.
	wantCode(t, do(t, srv, "GET", "/api/acquire/search?q=Show", nil, admin), 404)
}

func TestAcquireSettingsAndParse(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	rec := do(t, srv, "GET", "/api/acquire/settings", nil, admin)
	wantCode(t, rec, 200)
	var view struct {
		Settings acquire.Settings `json:"settings"`
		Port     int              `json:"port"`
		Parser   bool             `json:"parser"`
	}
	_ = json.Unmarshal(rec.Body.Bytes(), &view)
	if view.Port == 0 || view.Port == 51413 || view.Parser {
		t.Fatalf("settings view: %+v", view)
	}
	view.Settings.SeedRatio = 2
	wantCode(t, do(t, srv, "PUT", "/api/acquire/settings", view.Settings, admin), 200)
	if got := srv.acquire.Settings().SeedRatio; got != 2 {
		t.Fatalf("seed ratio = %v", got)
	}
	view.Settings.ImportMode = "symlink"
	wantCode(t, do(t, srv, "PUT", "/api/acquire/settings", view.Settings, admin), 400)

	rec = do(t, srv, "GET", "/api/acquire/parse?name="+strings.ReplaceAll("[Fansub-A] Show - 05 [1080p]", " ", "%20")+"&kind=anime", nil, admin)
	wantCode(t, rec, 200)
	var r contracts.Release
	_ = json.Unmarshal(rec.Body.Bytes(), &r)
	// No parser socket in a test data dir: the tokenizer answers.
	if r.Title != "Show" || r.Parser != "lain-release-tokenizer" || len(r.Episodes) != 1 {
		t.Fatalf("parse: %+v", r)
	}
	wantCode(t, do(t, srv, "GET", "/api/acquire/parse", nil, admin), 400)
}

func TestAcquireGrabImportsIntoALibrary(t *testing.T) {
	tr, err := trackertest.New()
	if err != nil {
		t.Fatal(err)
	}
	defer tr.Close()
	data := make([]byte, 70_000)
	for i := range data {
		data[i] = byte(i * 31)
	}
	raw, mi, err := torrent.Build("[Fansub-A] Show - 03 [1080p].mkv", []torrent.SourceFile{{Data: data}}, 32<<10, []string{tr.HTTPURL()})
	if err != nil {
		t.Fatal(err)
	}
	seedDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(seedDir, mi.Info.Name), data, 0o644); err != nil {
		t.Fatal(err)
	}
	seed, err := torrent.NewClient(torrent.Config{ListenAddr: "127.0.0.1:0"})
	if err != nil {
		t.Fatal(err)
	}
	defer seed.Close()
	if _, err := seed.Add(torrent.Spec{MetaInfo: mi, Dir: seedDir}); err != nil {
		t.Fatal(err)
	}
	files := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { _, _ = w.Write(raw) }))
	defer files.Close()

	srv := testServer(t)
	admin := setupAdmin(t, srv)
	libDir := t.TempDir()
	rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "Anime", "type": "anime", "path": libDir}, admin)
	wantCode(t, rec, 201)
	var lib contracts.Library
	_ = json.Unmarshal(rec.Body.Bytes(), &lib)
	// The shared budget (A-4) is the downloads settings; the temp dir's
	// filesystem may be smaller than the default free-space floor.
	ds := srv.downloads.Settings()
	ds.MinFreeBytes = 0
	if _, err := srv.downloads.SetSettings(ds); err != nil {
		t.Fatal(err)
	}
	s := srv.acquire.Settings()
	s.SeedRatio, s.SeedMinutes = 0, 0 // import, then free the disk
	if _, err := srv.acquire.SetSettings(s); err != nil {
		t.Fatal(err)
	}

	wantCode(t, do(t, srv, "POST", "/api/acquire/grabs", map[string]any{"url": files.URL + "/x.torrent", "library_id": "nope"}, admin), 400)
	rec = do(t, srv, "POST", "/api/acquire/grabs", map[string]any{"url": files.URL + "/x.torrent", "library_id": lib.ID}, admin)
	wantCode(t, rec, 201)
	var g acquire.Grab
	_ = json.Unmarshal(rec.Body.Bytes(), &g)
	deadline := time.Now().Add(20 * time.Second)
	for time.Now().Before(deadline) {
		got, _ := srv.acquire.Get(g.ID)
		if got.State == acquire.GrabDone || got.State == acquire.GrabFailed {
			g = got
			break
		}
		time.Sleep(50 * time.Millisecond)
	}
	if g.State != acquire.GrabDone {
		t.Fatalf("grab = %+v", g)
	}
	placed := filepath.Join(libDir, "Show", "[Fansub-A] Show - 03.mkv")
	if b, err := os.ReadFile(placed); err != nil || len(b) != len(data) {
		t.Fatalf("library file: %v", err)
	}
	// The rescan the import triggered catalogs the new episode.
	for time.Now().Before(deadline) {
		for _, it := range srv.catList() {
			if it.FilePath == placed {
				if it.Episode != 3 {
					t.Fatalf("cataloged as %+v", it)
				}
				goto found
			}
		}
		time.Sleep(50 * time.Millisecond)
	}
	t.Fatal("imported episode never reached the catalog")
found:
	wantCode(t, do(t, srv, "POST", "/api/acquire/grabs/"+g.ID+"/pause", nil, admin), 409)
	wantCode(t, do(t, srv, "POST", "/api/acquire/grabs/"+g.ID+"/explode", nil, admin), 404)
	wantCode(t, do(t, srv, "DELETE", "/api/acquire/grabs/"+g.ID+"?data=1", nil, admin), 200)
	if _, err := os.Stat(placed); err != nil {
		t.Fatal("removing a grab must never touch the library copy")
	}
}

func TestAcquireAutomationRoutes(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	wantCode(t, do(t, srv, "POST", "/api/users", map[string]string{"username": "ana", "password": "password123"}, admin), 201)
	user := loginAs(t, srv, "ana", "password123")
	for _, path := range []string{"/api/acquire/profiles", "/api/acquire/monitored", "/api/acquire/wanted", "/api/acquire/blocklist", "/api/acquire/replaced", "/api/acquire/automation"} {
		if rec := do(t, srv, "GET", path, nil, user); rec.Code != http.StatusForbidden {
			t.Errorf("%s as user: %d", path, rec.Code)
		}
	}

	// Profiles: the default exists; a bad one is refused; a good one saves.
	var profiles struct{ Profiles []acquire.Profile }
	rec := do(t, srv, "GET", "/api/acquire/profiles", nil, admin)
	wantCode(t, rec, 200)
	_ = json.Unmarshal(rec.Body.Bytes(), &profiles)
	if len(profiles.Profiles) != 1 || profiles.Profiles[0].ID != "default" {
		t.Fatalf("profiles: %+v", profiles)
	}
	wantCode(t, do(t, srv, "POST", "/api/acquire/profiles", map[string]any{"name": "x", "resolutions": []string{"potato"}}, admin), 400)
	rec = do(t, srv, "POST", "/api/acquire/profiles", map[string]any{"name": "4K", "resolutions": []string{"2160p"}}, admin)
	wantCode(t, rec, 201)
	var p acquire.Profile
	_ = json.Unmarshal(rec.Body.Bytes(), &p)

	// Monitored titles need a library of the same kind.
	libDir := t.TempDir()
	rec = do(t, srv, "POST", "/api/libraries", map[string]string{"name": "Anime", "type": "anime", "path": libDir}, admin)
	wantCode(t, rec, 201)
	var lib contracts.Library
	_ = json.Unmarshal(rec.Body.Bytes(), &lib)
	wantCode(t, do(t, srv, "POST", "/api/acquire/monitored", map[string]any{"library_id": lib.ID, "kind": "movie", "title": "Film"}, admin), 400)
	rec = do(t, srv, "POST", "/api/acquire/monitored", map[string]any{"library_id": lib.ID, "kind": "anime", "title": "Show", "from": 1, "to": 3, "profile_id": p.ID}, admin)
	wantCode(t, rec, 201)
	var mon acquire.Monitored
	_ = json.Unmarshal(rec.Body.Bytes(), &mon)
	wantCode(t, do(t, srv, "DELETE", "/api/acquire/profiles/"+p.ID, nil, admin), 409)

	rec = do(t, srv, "GET", "/api/acquire/monitored/"+mon.ID+"/wanted", nil, admin)
	wantCode(t, rec, 200)
	var wanted acquire.Wanted
	_ = json.Unmarshal(rec.Body.Bytes(), &wanted)
	if len(wanted.Missing) != 3 {
		t.Fatalf("wanted: %+v", wanted)
	}
	rec = do(t, srv, "GET", "/api/acquire/wanted", nil, admin)
	wantCode(t, rec, 200)
	if !strings.Contains(rec.Body.String(), `"title":"Show"`) || !strings.Contains(rec.Body.String(), `"missing_count":3`) {
		t.Fatalf("wanted overview: %s", rec.Body.String())
	}
	// A search with no indexer explains itself; RSS reports nothing grabbed.
	// The metadata episode lookup is stubbed: tests never reach the
	// network (D-120).
	srv.episodeCountSeam = func(string, string) int { return 0 }
	rec = do(t, srv, "POST", "/api/acquire/monitored/"+mon.ID+"/search", nil, admin)
	wantCode(t, rec, 200)
	rec = do(t, srv, "POST", "/api/acquire/rss", nil, admin)
	wantCode(t, rec, 200)
	if !strings.Contains(rec.Body.String(), `"grabbed":[]`) {
		t.Fatalf("rss: %s", rec.Body.String())
	}
	mon.To = 5
	wantCode(t, do(t, srv, "PUT", "/api/acquire/monitored/"+mon.ID, mon, admin), 200)
	wantCode(t, do(t, srv, "GET", "/api/acquire/blocklist", nil, admin), 200)
	wantCode(t, do(t, srv, "DELETE", "/api/acquire/blocklist/nope", nil, admin), 404)
	wantCode(t, do(t, srv, "GET", "/api/acquire/replaced", nil, admin), 200)
	wantCode(t, do(t, srv, "POST", "/api/acquire/replaced/purge", nil, admin), 200)
	rec = do(t, srv, "GET", "/api/acquire/automation", nil, admin)
	wantCode(t, rec, 200)
	if !strings.Contains(rec.Body.String(), `"enabled":false`) {
		t.Fatalf("automation is off by default: %s", rec.Body.String())
	}
	wantCode(t, do(t, srv, "DELETE", "/api/acquire/monitored/"+mon.ID, nil, admin), 200)
	wantCode(t, do(t, srv, "DELETE", "/api/acquire/profiles/"+p.ID, nil, admin), 200)
}
