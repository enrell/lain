package gateway

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/enrell/lain/internal/plugins/catalog"
	"github.com/enrell/lain/internal/contracts"
)

// fakeSubtitleAPI imitates the OpenSubtitles.com API v1: one search hit
// whose download link points back at itself. Tests never reach the
// real service.
func fakeSubtitleAPI(t *testing.T) *httptest.Server {
	var srv *httptest.Server
	srv = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if !strings.HasPrefix(r.URL.Path, "/f/") && r.Header.Get("Api-Key") != "key-1" { // file links are keyless
			w.WriteHeader(http.StatusUnauthorized)
			return
		}
		switch r.URL.Path {
		case "/api/v1/subtitles":
			_, _ = w.Write([]byte(`{"data":[{"attributes":{"language":"en","download_count":3,"moviehash_match":true,
"release":"[Fansub-A] Show - 01","feature_details":{"season_number":1,"episode_number":1},"files":[{"file_id":7,"file_name":"a.srt"}]}}]}`))
		case "/api/v1/download":
			_, _ = w.Write([]byte(`{"link":"` + srv.URL + `/f/7.srt","remaining":5}`))
		case "/f/7.srt":
			_, _ = w.Write([]byte("1\n00:00:01,000 --> 00:00:02,000\nHi\n"))
		default:
			http.NotFound(w, r)
		}
	}))
	t.Cleanup(srv.Close)
	return srv
}

func TestAcquireSubtitleRoutes(t *testing.T) {
	t.Setenv("PATH", t.TempDir()) // no host ffprobe: duration unknown, the sync check is skipped
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	wantCode(t, do(t, srv, "POST", "/api/users", map[string]string{"username": "ana", "password": "password123"}, admin), 201)
	user := loginAs(t, srv, "ana", "password123")
	api := fakeSubtitleAPI(t)

	libDir := t.TempDir()
	rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "Anime", "type": "anime", "path": libDir}, admin)
	wantCode(t, rec, 201)
	var lib contracts.Library
	_ = json.Unmarshal(rec.Body.Bytes(), &lib)
	media := filepath.Join(libDir, "Show", "[Fansub-A] Show - 01.mkv")
	_ = os.MkdirAll(filepath.Dir(media), 0o755)
	if err := os.WriteFile(media, make([]byte, 200_000), 0o644); err != nil {
		t.Fatal(err)
	}
	out, _, err := srv.reg.CallOne(contracts.CapCatalogWrite, catalog.UpsertInput{
		LibraryID: lib.ID,
		Proposal:  contracts.Proposal{Kind: "anime", Title: "Show", Season: 1, Episode: 1, Confidence: 0.9, PluginID: "test"},
		Candidate: contracts.Candidate{Path: media, Size: 200_000, ModTime: 1, LibraryID: lib.ID},
	})
	if err != nil {
		t.Fatal(err)
	}
	it := out.(contracts.CatalogItem)

	// Accounts: admin only, secrets never echoed.
	body := map[string]any{"name": "OS", "kind": "opensubtitles", "base_url": api.URL + "/api/v1", "api_key": "key-1", "password": "pw-secret"}
	wantCode(t, do(t, srv, "POST", "/api/acquire/subtitle-providers", body, user), 403)
	rec = do(t, srv, "POST", "/api/acquire/subtitle-providers", body, admin)
	wantCode(t, rec, 201)
	rec = do(t, srv, "GET", "/api/acquire/subtitle-providers", nil, admin)
	wantCode(t, rec, 200)
	if b := rec.Body.String(); strings.Contains(b, "key-1") || strings.Contains(b, "pw-secret") || !strings.Contains(b, `"has_api_key":true`) {
		t.Fatalf("providers leak or miss flags: %s", b)
	}

	// Manual search for one item, then download the chosen candidate.
	wantCode(t, do(t, srv, "GET", "/api/acquire/items/"+it.ID+"/subtitles?languages=eng", nil, user), 403)
	rec = do(t, srv, "GET", "/api/acquire/items/"+it.ID+"/subtitles?languages=en", nil, admin)
	wantCode(t, rec, 200)
	var res struct {
		Choices []struct {
			ProviderID string `json:"provider_id"`
			FileID     string `json:"file_id"`
			Language   string `json:"language"`
			HashMatch  bool   `json:"hash_match"`
			Accepted   bool   `json:"accepted"`
		} `json:"choices"`
	}
	_ = json.Unmarshal(rec.Body.Bytes(), &res)
	if len(res.Choices) != 1 || !res.Choices[0].Accepted || res.Choices[0].Language != "eng" || !res.Choices[0].HashMatch {
		t.Fatalf("choices: %s", rec.Body)
	}
	c := res.Choices[0]
	rec = do(t, srv, "POST", "/api/acquire/items/"+it.ID+"/subtitles", map[string]any{"provider_id": c.ProviderID, "file_id": c.FileID, "language": c.Language, "file_name": "a.srt"}, admin)
	wantCode(t, rec, 200)
	if _, err := os.Stat(filepath.Join(libDir, "Show", "[Fansub-A] Show - 01.en.srt")); err != nil {
		t.Fatalf("sidecar not written: %v (%s)", err, rec.Body)
	}
	// A second identical download conflicts instead of overwriting.
	wantCode(t, do(t, srv, "POST", "/api/acquire/items/"+it.ID+"/subtitles", map[string]any{"provider_id": c.ProviderID, "file_id": c.FileID, "language": c.Language, "file_name": "a.srt"}, admin), 409)
	// Players see it.
	rec = do(t, srv, "GET", "/api/items/"+it.ID+"/sidecars", nil, user)
	if !strings.Contains(rec.Body.String(), `"language":"eng"`) {
		t.Fatalf("sidecars: %s", rec.Body)
	}
	rec = do(t, srv, "GET", "/api/acquire/subtitles", nil, admin)
	if !strings.Contains(rec.Body.String(), `"file_id":"7"`) {
		t.Fatalf("ledger: %s", rec.Body)
	}
	wantCode(t, do(t, srv, "GET", "/api/acquire/items/nope/subtitles", nil, admin), 404)
	wantCode(t, do(t, srv, "GET", "/api/acquire/items/"+it.ID+"/subtitles?languages=zz", nil, admin), 400)
}
