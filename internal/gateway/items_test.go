package gateway

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	"github.com/enrell/lain/internal/contracts"
)

// TestItemDelete removes an item's file through the admin endpoint and
// pins the D-073/D-068 semantics: the catalog row stays as `missing`
// and recorded progress survives the deletion.
func TestItemDelete(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)

	root := t.TempDir()
	file := filepath.Join(root, "Frieren - 12.mkv")
	if err := os.WriteFile(file, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	if rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "L", "type": "anime", "path": root}, admin); rec.Code != 201 {
		t.Fatalf("library: %d %s", rec.Code, rec.Body.String())
	}
	if rec := do(t, srv, "POST", "/api/library/scan", nil, admin); rec.Code != 202 {
		t.Fatalf("scan: %d", rec.Code)
	}
	waitScan(t, srv, admin)

	var page contracts.CatalogPage
	rec := do(t, srv, "GET", "/api/catalog", nil, admin)
	if err := json.Unmarshal(rec.Body.Bytes(), &page); err != nil || len(page.Items) != 1 {
		t.Fatalf("catalog: %d %s", rec.Code, rec.Body.String())
	}
	id := page.Items[0].ID

	// Progress must survive the file going away.
	rec = do(t, srv, "PUT", "/api/items/"+id+"/progress", map[string]any{
		"position_sec": 120, "duration_sec": 1420, "completed": false,
	}, admin)
	if rec.Code != 200 {
		t.Fatalf("progress put: %d %s", rec.Code, rec.Body.String())
	}

	rec = do(t, srv, "DELETE", "/api/items/"+id, nil, admin)
	if rec.Code != 200 {
		t.Fatalf("delete: %d %s", rec.Code, rec.Body.String())
	}
	var it contracts.CatalogItem
	if err := json.Unmarshal(rec.Body.Bytes(), &it); err != nil || !it.Missing {
		t.Fatalf("deleted item must answer missing: %s", rec.Body.String())
	}
	if _, err := os.Stat(file); !os.IsNotExist(err) {
		t.Fatalf("file must be gone: %v", err)
	}

	rec = do(t, srv, "GET", "/api/catalog/"+id, nil, admin)
	if err := json.Unmarshal(rec.Body.Bytes(), &it); err != nil || !it.Missing {
		t.Fatalf("catalog entry must read missing: %d %s", rec.Code, rec.Body.String())
	}
	rec = do(t, srv, "GET", "/api/items/"+id+"/progress", nil, admin)
	var p contracts.Progress
	if err := json.Unmarshal(rec.Body.Bytes(), &p); err != nil || p.PositionSec != 120 {
		t.Fatalf("progress must survive the delete: %d %s", rec.Code, rec.Body.String())
	}

	// Deleting again is idempotent: the file is already gone and the
	// end state — missing entry — is the same.
	rec = do(t, srv, "DELETE", "/api/items/"+id, nil, admin)
	if rec.Code != 200 {
		t.Fatalf("second delete: %d %s", rec.Code, rec.Body.String())
	}
}

// TestItemDeleteGates covers the refusal paths: the endpoint is
// admin-only and unknown items 404.
func TestItemDeleteGates(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)

	if rec := do(t, srv, "DELETE", "/api/items/nope", nil, admin); rec.Code != 404 {
		t.Fatalf("unknown item: %d", rec.Code)
	}
	if rec := do(t, srv, "DELETE", "/api/items/nope", nil, ""); rec.Code != 401 {
		t.Fatalf("anonymous: %d", rec.Code)
	}

	if rec := do(t, srv, "POST", "/api/users", map[string]string{"username": "ana", "password": "password123"}, admin); rec.Code != 201 {
		t.Fatalf("create user: %d", rec.Code)
	}
	ana := loginAs(t, srv, "ana", "password123")
	if rec := do(t, srv, "DELETE", "/api/items/nope", nil, ana); rec.Code != 403 {
		t.Fatalf("non-admin: %d", rec.Code)
	}
}

// TestSetMissingFlipsOneItem exercises the catalog primitive the delete
// handler relies on: one item flagged, a sibling untouched, and the
// flag reversible when the file returns.
func TestSetMissingFlipsOneItem(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)

	root := t.TempDir()
	for _, name := range []string{"Frieren - 11.mkv", "Frieren - 12.mkv"} {
		if err := os.WriteFile(filepath.Join(root, name), []byte("x"), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	if rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "L", "type": "anime", "path": root}, admin); rec.Code != 201 {
		t.Fatalf("library: %d %s", rec.Code, rec.Body.String())
	}
	if rec := do(t, srv, "POST", "/api/library/scan", nil, admin); rec.Code != 202 {
		t.Fatalf("scan: %d", rec.Code)
	}
	waitScan(t, srv, admin)

	var page contracts.CatalogPage
	rec := do(t, srv, "GET", "/api/catalog", nil, admin)
	if err := json.Unmarshal(rec.Body.Bytes(), &page); err != nil || len(page.Items) != 2 {
		t.Fatalf("catalog: %d %s", rec.Code, rec.Body.String())
	}
	victim, sibling := page.Items[0].ID, page.Items[1].ID

	found, err := srv.cat.SetMissing(victim, true)
	if err != nil || !found {
		t.Fatalf("SetMissing: found=%v err=%v", found, err)
	}
	if found, err := srv.cat.SetMissing("ghost", true); err != nil || found {
		t.Fatalf("unknown id must report not-found: found=%v err=%v", found, err)
	}

	rec = do(t, srv, "GET", "/api/catalog/"+victim, nil, admin)
	var it, sib contracts.CatalogItem
	if err := json.Unmarshal(rec.Body.Bytes(), &it); err != nil || !it.Missing {
		t.Fatalf("victim must be missing: %s", rec.Body.String())
	}
	rec = do(t, srv, "GET", "/api/catalog/"+sibling, nil, admin)
	if err := json.Unmarshal(rec.Body.Bytes(), &sib); err != nil || sib.Missing {
		t.Fatalf("sibling must stay present: %s", rec.Body.String())
	}

	if _, err := srv.cat.SetMissing(victim, false); err != nil {
		t.Fatalf("unflag: %v", err)
	}
	rec = do(t, srv, "GET", "/api/catalog/"+victim, nil, admin)
	var restored contracts.CatalogItem
	if err := json.Unmarshal(rec.Body.Bytes(), &restored); err != nil || restored.Missing {
		t.Fatalf("restored item must clear the flag: %s", rec.Body.String())
	}
}
