package gateway

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
)

// fakeMeta serves canned metadata through both caps.
type fakeMeta struct {
	id          string
	candidates  []contracts.MetadataCandidate
	record      contracts.MetadataRecord
	failSearch  bool
	failResolve bool
}

func (f *fakeMeta) ID() string { return f.id }
func (f *fakeMeta) Capabilities() []string {
	return []string{contracts.CapMetadataSearch, contracts.CapMetadataResolve}
}
func (f *fakeMeta) Health() error { return nil }
func (f *fakeMeta) Invoke(cap string, input any) (any, error) {
	switch cap {
	case contracts.CapMetadataSearch:
		if f.failSearch {
			return nil, errFakeMetaDown
		}
		return f.candidates, nil
	case contracts.CapMetadataResolve:
		if f.failResolve {
			return nil, errFakeMetaDown
		}
		return f.record, nil
	}
	return nil, errFakeMetaDown
}

type fakeMetaErr string

func (e fakeMetaErr) Error() string { return string(e) }

var errFakeMetaDown = fakeMetaErr("upstream down")

// withFakeMetadata replaces the remote metadata providers with fakes
// (same ids, so bindings keep working).
func withFakeMetadata(t *testing.T, srv *Server, fakes ...*fakeMeta) {
	t.Helper()
	for _, f := range fakes {
		srv.Registry().Register(f)
	}
}

func TestEnrichFlow(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	withFakeMetadata(t, srv,
		&fakeMeta{id: "lain-metadata-nfo", candidates: []contracts.MetadataCandidate{}},
		&fakeMeta{
			id:         "lain-metadata-kitsu",
			candidates: []contracts.MetadataCandidate{{Provider: "lain-metadata-kitsu", RemoteID: "12", Title: "Frieren", Year: 2023}},
			record:     contracts.MetadataRecord{Provider: "lain-metadata-kitsu", RemoteID: "12", Title: "Frieren", Year: 2023, Episodes: 28, Poster: "https://p.jpg"},
		},
		&fakeMeta{id: "lain-metadata-anilist", failSearch: true},
		&fakeMeta{id: "lain-metadata-jikan", failSearch: true},
		&fakeMeta{id: "lain-metadata-tvmaze", failSearch: true},
	)

	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "Frieren - 12.mkv"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "L", "type": "anime", "path": root}, admin)
	if rec.Code != 201 {
		t.Fatalf("library: %d", rec.Code)
	}
	rec = do(t, srv, "POST", "/api/library/scan", nil, admin)
	if rec.Code != 202 {
		t.Fatalf("scan: %d", rec.Code)
	}
	var status struct {
		State string `json:"state"`
	}
	for i := 0; i < 100; i++ {
		rec = do(t, srv, "GET", "/api/library/scan", nil, admin)
		_ = json.Unmarshal(rec.Body.Bytes(), &status)
		if status.State == "done" {
			break
		}
	}
	var page struct {
		Items []map[string]any `json:"items"`
	}
	rec = do(t, srv, "GET", "/api/catalog", nil, admin)
	_ = json.Unmarshal(rec.Body.Bytes(), &page)
	if len(page.Items) != 1 {
		t.Fatalf("catalog: %+v", page)
	}
	id := page.Items[0]["id"].(string)

	// Remotes down except kitsu: merge degrades, enrich still lands.
	rec = do(t, srv, "POST", "/api/catalog/"+id+"/enrich", nil, admin)
	if rec.Code != 200 {
		t.Fatalf("enrich: %d %s", rec.Code, rec.Body.String())
	}
	var e contracts.Enrichment
	if err := json.Unmarshal(rec.Body.Bytes(), &e); err != nil {
		t.Fatal(err)
	}
	if e.Title != "Frieren" || e.Provider != "lain-metadata-kitsu" || e.Poster != "https://p.jpg" {
		t.Fatalf("enrichment: %+v", e)
	}
	rec = do(t, srv, "GET", "/api/catalog/"+id+"/enrich", nil, admin)
	if rec.Code != 200 {
		t.Fatalf("get overlay: %d", rec.Code)
	}
	rec = do(t, srv, "DELETE", "/api/catalog/"+id+"/enrich", nil, admin)
	if rec.Code != 200 {
		t.Fatalf("delete overlay: %d", rec.Code)
	}
	rec = do(t, srv, "GET", "/api/catalog/"+id+"/enrich", nil, admin)
	if rec.Code != 404 {
		t.Fatalf("deleted overlay: %d", rec.Code)
	}
	// Non-admin cannot enrich.
	arec := do(t, srv, "POST", "/api/users", map[string]string{"username": "ana", "password": "password123"}, admin)
	_ = arec
	ana := loginAs(t, srv, "ana", "password123")
	if rec := do(t, srv, "POST", "/api/catalog/"+id+"/enrich", nil, ana); rec.Code != 403 {
		t.Fatalf("enrich as user: %d", rec.Code)
	}
}

func TestEnrichNoMatch(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	withFakeMetadata(t, srv,
		&fakeMeta{id: "lain-metadata-nfo"},
		&fakeMeta{id: "lain-metadata-kitsu"},
		&fakeMeta{id: "lain-metadata-anilist"},
		&fakeMeta{id: "lain-metadata-jikan"},
		&fakeMeta{id: "lain-metadata-tvmaze"},
	)
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "Whatever.mkv"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	if rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "L", "type": "anime", "path": root}, admin); rec.Code != 201 {
		t.Fatalf("library: %d", rec.Code)
	}
	if rec := do(t, srv, "POST", "/api/library/scan", nil, admin); rec.Code != 202 {
		t.Fatalf("scan: %d", rec.Code)
	}
	var status struct {
		State string `json:"state"`
	}
	for i := 0; i < 100; i++ {
		rec := do(t, srv, "GET", "/api/library/scan", nil, admin)
		_ = json.Unmarshal(rec.Body.Bytes(), &status)
		if status.State == "done" {
			break
		}
	}
	var page struct {
		Items []map[string]any `json:"items"`
	}
	rec := do(t, srv, "GET", "/api/catalog", nil, admin)
	_ = json.Unmarshal(rec.Body.Bytes(), &page)
	id := page.Items[0]["id"].(string)
	rec = do(t, srv, "POST", "/api/catalog/"+id+"/enrich", nil, admin)
	if rec.Code != 404 {
		t.Fatalf("empty merge must 404, got %d %s", rec.Code, rec.Body.String())
	}
}

func scanToDone(t *testing.T, srv *Server, admin string) map[string]any {
	t.Helper()
	if rec := do(t, srv, "POST", "/api/library/scan", nil, admin); rec.Code != 202 {
		t.Fatalf("scan: %d", rec.Code)
	}
	var status struct {
		State string         `json:"state"`
		Stats map[string]any `json:"stats"`
	}
	for i := 0; i < 200; i++ {
		rec := do(t, srv, "GET", "/api/library/scan", nil, admin)
		_ = json.Unmarshal(rec.Body.Bytes(), &status)
		if status.State == "done" || status.State == "error" {
			break
		}
	}
	if status.State != "done" {
		t.Fatalf("scan state %q", status.State)
	}
	return status.Stats
}

// Scans enrich overlay-less items by default: the web grid and the
// desktop show metadata without any manual enrich step.
func TestScanAutoEnrichesByDefault(t *testing.T) {
	srv := testServer(t)
	srv.SetAutoEnrich(true)
	admin := setupAdmin(t, srv)
	withFakeMetadata(t, srv,
		&fakeMeta{id: "lain-metadata-nfo", candidates: []contracts.MetadataCandidate{}},
		&fakeMeta{
			id:         "lain-metadata-kitsu",
			candidates: []contracts.MetadataCandidate{{Provider: "lain-metadata-kitsu", RemoteID: "12", Title: "Frieren", Year: 2023}},
			record:     contracts.MetadataRecord{Provider: "lain-metadata-kitsu", RemoteID: "12", Title: "Frieren", Year: 2023},
		},
		&fakeMeta{id: "lain-metadata-anilist", failSearch: true},
		&fakeMeta{id: "lain-metadata-jikan", failSearch: true},
		&fakeMeta{id: "lain-metadata-tvmaze", failSearch: true},
	)
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "Frieren - 12.mkv"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	if rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "L", "type": "anime", "path": root}, admin); rec.Code != 201 {
		t.Fatalf("library: %d", rec.Code)
	}
	stats := scanToDone(t, srv, admin)
	if stats["enriched"] != float64(1) {
		t.Fatalf("enriched=%v, want 1", stats["enriched"])
	}
	var page struct {
		Items []map[string]any `json:"items"`
	}
	rec := do(t, srv, "GET", "/api/catalog", nil, admin)
	_ = json.Unmarshal(rec.Body.Bytes(), &page)
	id := page.Items[0]["id"].(string)
	rec = do(t, srv, "GET", "/api/catalog/"+id+"/enrich", nil, admin)
	if rec.Code != 200 {
		t.Fatalf("overlay missing: %d %s", rec.Code, rec.Body.String())
	}
}

func TestScanSkipsAutoEnrichWhenDisabled(t *testing.T) {
	srv := testServer(t)
	srv.SetAutoEnrich(false)
	admin := setupAdmin(t, srv)
	withFakeMetadata(t, srv,
		&fakeMeta{
			id:         "lain-metadata-kitsu",
			candidates: []contracts.MetadataCandidate{{Provider: "lain-metadata-kitsu", RemoteID: "12", Title: "Frieren", Year: 2023}},
			record:     contracts.MetadataRecord{Provider: "lain-metadata-kitsu", RemoteID: "12", Title: "Frieren", Year: 2023},
		},
		&fakeMeta{id: "lain-metadata-tvmaze"},
	)
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "Frieren - 12.mkv"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	if rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "L", "type": "anime", "path": root}, admin); rec.Code != 201 {
		t.Fatalf("library: %d", rec.Code)
	}
	stats := scanToDone(t, srv, admin)
	if stats["enriched"] != float64(0) {
		t.Fatalf("enriched=%v, want 0", stats["enriched"])
	}
}

// Empty merges must not poison the cache: a transient outage followed
// by a healthy provider still enriches on retry.
func TestEnrichEmptyNotCached(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	withFakeMetadata(t, srv,
		&fakeMeta{id: "lain-metadata-nfo", candidates: []contracts.MetadataCandidate{}},
		&fakeMeta{id: "lain-metadata-kitsu", candidates: []contracts.MetadataCandidate{}},
		&fakeMeta{id: "lain-metadata-anilist", candidates: []contracts.MetadataCandidate{}},
		&fakeMeta{id: "lain-metadata-jikan", candidates: []contracts.MetadataCandidate{}},
		&fakeMeta{id: "lain-metadata-tvmaze", candidates: []contracts.MetadataCandidate{}},
	)
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "Darling in the FranXX - 12.mkv"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	if rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "L", "type": "anime", "path": root}, admin); rec.Code != 201 {
		t.Fatalf("library: %d", rec.Code)
	}
	stats := scanToDone(t, srv, admin)
	if stats["enriched"] != float64(0) {
		t.Fatalf("enriched=%v, want 0 while all providers are empty", stats["enriched"])
	}
	var page struct {
		Items []map[string]any `json:"items"`
	}
	rec := do(t, srv, "GET", "/api/catalog", nil, admin)
	_ = json.Unmarshal(rec.Body.Bytes(), &page)
	id := page.Items[0]["id"].(string)
	if rec := do(t, srv, "POST", "/api/catalog/"+id+"/enrich", nil, admin); rec.Code != 404 {
		t.Fatalf("empty merge must 404, got %d %s", rec.Code, rec.Body.String())
	}
	withFakeMetadata(t, srv,
		&fakeMeta{
			id:         "lain-metadata-kitsu",
			candidates: []contracts.MetadataCandidate{{Provider: "lain-metadata-kitsu", RemoteID: "21", Title: "Darling in the FranXX", Year: 2018}},
			record:     contracts.MetadataRecord{Provider: "lain-metadata-kitsu", RemoteID: "21", Title: "Darling in the FranXX", Year: 2018},
		},
	)
	if rec := do(t, srv, "POST", "/api/catalog/"+id+"/enrich", nil, admin); rec.Code != 200 {
		t.Fatalf("retry after outage must succeed, got %d %s", rec.Code, rec.Body.String())
	}
}

// No per-scan cap: one scan enriches the whole backlog, however large.
func TestScanEnrichesEverything(t *testing.T) {
	srv := testServer(t)
	srv.SetAutoEnrich(true)
	admin := setupAdmin(t, srv)
	withFakeMetadata(t, srv,
		&fakeMeta{id: "lain-metadata-nfo", candidates: []contracts.MetadataCandidate{}},
		&fakeMeta{
			id:         "lain-metadata-kitsu",
			candidates: []contracts.MetadataCandidate{{Provider: "lain-metadata-kitsu", RemoteID: "1", Title: "Bulk", Year: 2024}},
			record:     contracts.MetadataRecord{Provider: "lain-metadata-kitsu", RemoteID: "1", Title: "Bulk", Year: 2024},
		},
		&fakeMeta{id: "lain-metadata-anilist", failSearch: true},
		&fakeMeta{id: "lain-metadata-jikan", failSearch: true},
		&fakeMeta{id: "lain-metadata-tvmaze", failSearch: true},
	)
	root := t.TempDir()
	const n = 120
	for i := 0; i < n; i++ {
		name := filepath.Join(root, "Bulk S01E"+string(rune('0'+i/100))+string(rune('0'+(i/10)%10))+string(rune('0'+i%10))+".mkv")
		if err := os.WriteFile(name, []byte("x"), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	if rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "L", "type": "anime", "path": root}, admin); rec.Code != 201 {
		t.Fatalf("library: %d", rec.Code)
	}
	if rec := do(t, srv, "POST", "/api/library/scan", nil, admin); rec.Code != 202 {
		t.Fatalf("scan: %d", rec.Code)
	}
	var status struct {
		State string         `json:"state"`
		Stats map[string]any `json:"stats"`
	}
	for i := 0; i < 600; i++ {
		rec := do(t, srv, "GET", "/api/library/scan", nil, admin)
		_ = json.Unmarshal(rec.Body.Bytes(), &status)
		if status.State == "done" || status.State == "error" {
			break
		}
		time.Sleep(100 * time.Millisecond)
	}
	if status.State != "done" {
		t.Fatalf("scan state %q", status.State)
	}
	stats := status.Stats
	if stats["enriched"] != float64(n) {
		t.Fatalf("enriched=%v, want %d", stats["enriched"], n)
	}
}
