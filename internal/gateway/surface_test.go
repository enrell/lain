package gateway

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/plugins/catalog"
	"github.com/enrell/lain/internal/plugins/metadata"
)

// TestCatalogLibraryFilter proves the catalog page can be scoped to one
// library, which is what the web UI's library route needs.
func TestCatalogLibraryFilter(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)

	mk := func(name string, files ...string) string {
		root := t.TempDir()
		for _, f := range files {
			if err := os.WriteFile(filepath.Join(root, f), []byte("x"), 0o644); err != nil {
				t.Fatal(err)
			}
		}
		rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": name, "type": "anime", "path": root}, admin)
		if rec.Code != 201 {
			t.Fatalf("library %s: %d %s", name, rec.Code, rec.Body.String())
		}
		var lib contracts.Library
		if err := json.Unmarshal(rec.Body.Bytes(), &lib); err != nil {
			t.Fatal(err)
		}
		return lib.ID
	}
	libA := mk("A", "Alpha.mkv", "Beta.mkv")
	libB := mk("B", "Gamma.mkv")
	if rec := do(t, srv, "POST", "/api/library/scan", nil, admin); rec.Code != 202 {
		t.Fatalf("scan: %d", rec.Code)
	}
	waitScan(t, srv, admin)

	var page contracts.CatalogPage
	rec := do(t, srv, "GET", "/api/catalog?library_id="+libA, nil, admin)
	if rec.Code != 200 {
		t.Fatalf("catalog: %d %s", rec.Code, rec.Body.String())
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &page); err != nil {
		t.Fatal(err)
	}
	if page.Total != 2 || len(page.Items) != 2 {
		t.Fatalf("library A page: %+v", page)
	}
	for _, it := range page.Items {
		if it.LibraryID != libA {
			t.Fatalf("item %s belongs to %s, want %s", it.ID, it.LibraryID, libA)
		}
	}
	rec = do(t, srv, "GET", "/api/catalog?library_id="+libB+"&limit=1", nil, admin)
	_ = json.Unmarshal(rec.Body.Bytes(), &page)
	if page.Total != 1 || len(page.Items) != 1 || page.Items[0].Title != "Gamma" {
		t.Fatalf("library B page: %+v", page)
	}
	// Unknown library is an empty page, not an error.
	rec = do(t, srv, "GET", "/api/catalog?library_id=lib-nope", nil, admin)
	_ = json.Unmarshal(rec.Body.Bytes(), &page)
	if rec.Code != 200 || page.Total != 0 {
		t.Fatalf("unknown library: %d %+v", rec.Code, page)
	}
}

// TestStaticMountDoesNotShadowAPI proves the SPA catch-all registered
// on the gateway mux never shadows existing API routes: authenticated
// API paths still answer as API, and unknown API paths answer JSON.
func TestStaticMountDoesNotShadowAPI(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	if rec := do(t, srv, "GET", "/api/catalog", nil, admin); rec.Code != 200 {
		t.Fatalf("catalog behind SPA: %d %s", rec.Code, rec.Body.String())
	}
	rec := do(t, srv, "GET", "/api/definitely-not-a-route", nil, admin)
	if rec.Code != 404 || !strings.Contains(rec.Header().Get("Content-Type"), "application/json") {
		t.Fatalf("unknown api: %d %q", rec.Code, rec.Header().Get("Content-Type"))
	}
	if rec := do(t, srv, "GET", "/api/catalog", nil, ""); rec.Code != 401 {
		t.Fatalf("unauthenticated api: %d, want 401", rec.Code)
	}
	if rec := do(t, srv, "GET", "/health", nil, ""); rec.Code != 200 {
		t.Fatalf("health behind SPA: %d", rec.Code)
	}
}

func waitScan(t *testing.T, srv *Server, token string) {
	t.Helper()
	var status struct {
		State string `json:"state"`
		Error string `json:"error"`
	}
	// The scan runs in a goroutine; poll with a real wait so the slower
	// -race builds do not exhaust the iterations before "done".
	for i := 0; i < 500; i++ {
		rec := do(t, srv, "GET", "/api/library/scan", nil, token)
		_ = json.Unmarshal(rec.Body.Bytes(), &status)
		switch status.State {
		case "done":
			return
		case "error":
			t.Fatalf("scan failed: %s", status.Error)
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatal("scan did not finish")
}

// TestEnrichBatchRead proves the grid overlay read: one transaction,
// missing overlays absent, input bounded.
func TestEnrichBatchRead(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)

	it, err := srv.cat.Upsert(catalog.UpsertInput{
		LibraryID: "lib-x",
		Proposal:  contracts.Proposal{Kind: "anime", Title: "Show", Confidence: 1},
		Candidate: contracts.Candidate{Path: "/media/show.mkv", Size: 4, ModTime: 1},
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := metadata.NewSaver(srv.db).Save(it.ID, contracts.MetadataRecord{
		Provider: "p", RemoteID: "1", Title: "Show", Poster: "https://p",
	}); err != nil {
		t.Fatal(err)
	}

	var out struct {
		Items []contracts.Enrichment `json:"items"`
	}
	rec := do(t, srv, "GET", "/api/enrichments?ids="+it.ID+",missing", nil, admin)
	if rec.Code != 200 {
		t.Fatalf("batch: %d %s", rec.Code, rec.Body.String())
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &out); err != nil {
		t.Fatal(err)
	}
	if len(out.Items) != 1 || out.Items[0].ItemID != it.ID {
		t.Fatalf("batch items: %+v", out.Items)
	}

	if rec := do(t, srv, "GET", "/api/enrichments", nil, admin); rec.Code != 400 {
		t.Fatalf("no ids: %d, want 400", rec.Code)
	}
	wide := strings.Repeat("x,", maxEnrichBatch) + "x"
	if rec := do(t, srv, "GET", "/api/enrichments?ids="+wide, nil, admin); rec.Code != 400 {
		t.Fatalf("too many ids: %d, want 400", rec.Code)
	}
}

// TestPluginsExposesProviderInfo proves the operator surface can offer
// valid swap candidates with capabilities and health.
func TestPluginsExposesProviderInfo(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	rec := do(t, srv, "GET", "/api/plugins", nil, admin)
	if rec.Code != 200 {
		t.Fatalf("plugins: %d", rec.Code)
	}
	var out struct {
		Composition []struct {
			Capability string   `json:"capability"`
			Providers  []string `json:"providers"`
			Generation uint64   `json:"generation"`
		} `json:"composition"`
		ProviderInfo []struct {
			ID           string   `json:"id"`
			Capabilities []string `json:"capabilities"`
			Healthy      bool     `json:"healthy"`
		} `json:"provider_info"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &out); err != nil {
		t.Fatal(err)
	}
	byID := map[string][]string{}
	healthy := map[string]bool{}
	for _, p := range out.ProviderInfo {
		byID[p.ID] = p.Capabilities
		healthy[p.ID] = p.Healthy
	}
	caps := byID["lain-catalog-bolt"]
	if len(caps) != 2 || !healthy["lain-catalog-bolt"] {
		t.Fatalf("catalog provider info: %v healthy=%v", caps, healthy["lain-catalog-bolt"])
	}
	found := false
	for _, b := range out.Composition {
		if b.Capability == contracts.CapCatalogRead && len(b.Providers) == 1 {
			for _, c := range byID[b.Providers[0]] {
				if c == contracts.CapCatalogRead {
					found = true
				}
			}
		}
	}
	if !found {
		t.Fatal("composition provider must expose its capability in provider_info")
	}
	// Empty collections must serialize as [] — a null events log broke
	// the web UI once.
	if !bytes.Contains(rec.Body.Bytes(), []byte(`"events":[]`)) {
		t.Fatalf("events must encode as an empty array, got: %s", rec.Body.String())
	}
}
