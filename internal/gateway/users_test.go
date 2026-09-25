package gateway

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func loginAs(t *testing.T, srv *Server, username, password string) string {
	t.Helper()
	rec := do(t, srv, "POST", "/api/auth/login", map[string]string{"username": username, "password": password}, "")
	if rec.Code != 200 {
		t.Fatalf("login %s: %d %s", username, rec.Code, rec.Body.String())
	}
	var tok struct {
		Token string `json:"token"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &tok); err != nil || tok.Token == "" {
		t.Fatalf("no token for %s", username)
	}
	return tok.Token
}

func setupAdmin(t *testing.T, srv *Server) string {
	t.Helper()
	rec := do(t, srv, "POST", "/api/setup", map[string]string{"username": "admin", "password": "password123"}, "")
	if rec.Code != 201 {
		t.Fatalf("setup: %d %s", rec.Code, rec.Body.String())
	}
	return loginAs(t, srv, "admin", "password123")
}

func TestMultiUserAdminGates(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)

	// Admin creates a regular user.
	rec := do(t, srv, "POST", "/api/users", map[string]string{"username": "ana", "password": "password123"}, admin)
	if rec.Code != 201 {
		t.Fatalf("create user: %d %s", rec.Code, rec.Body.String())
	}
	user := loginAs(t, srv, "ana", "password123")

	// Non-admin cannot manage users, libraries, scans or plugins.
	for _, tc := range []struct{ method, path string }{
		{"POST", "/api/users"},
		{"GET", "/api/users"},
		{"POST", "/api/libraries"},
		{"POST", "/api/library/scan"},
		{"GET", "/api/plugins"},
		{"POST", "/api/plugins/swap"},
	} {
		rec := do(t, srv, tc.method, tc.path, map[string]string{"username": "x"}, user)
		if rec.Code != 403 {
			t.Errorf("%s %s as user: %d, want 403", tc.method, tc.path, rec.Code)
		}
	}
	// ...but reads and watches fine.
	if rec := do(t, srv, "GET", "/api/catalog", nil, user); rec.Code != 200 {
		t.Errorf("catalog as user: %d", rec.Code)
	}

	// Admin disables ana: her token dies immediately, login refused.
	var created struct {
		ID string `json:"id"`
	}
	rec = do(t, srv, "POST", "/api/users", map[string]string{"username": "ana2", "password": "password123"}, admin)
	_ = json.Unmarshal(rec.Body.Bytes(), &created)
	ana2 := loginAs(t, srv, "ana2", "password123")
	rec = do(t, srv, "PATCH", "/api/users/"+created.ID, map[string]any{"disabled": true}, admin)
	if rec.Code != 200 {
		t.Fatalf("disable: %d %s", rec.Code, rec.Body.String())
	}
	if rec := do(t, srv, "GET", "/api/me", nil, ana2); rec.Code != 401 {
		t.Errorf("disabled token: %d, want 401", rec.Code)
	}

	// Admin cannot disable or demote themselves.
	rec = do(t, srv, "PATCH", "/api/users/user-admin", map[string]any{"disabled": true}, admin)
	if rec.Code != 400 {
		t.Errorf("self-disable: %d, want 400", rec.Code)
	}
	rec = do(t, srv, "PATCH", "/api/users/user-admin", map[string]any{"role": "user"}, admin)
	if rec.Code != 400 {
		t.Errorf("self-demote: %d, want 400", rec.Code)
	}
}

func TestProgressIsolatedPerUser(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	rec := do(t, srv, "POST", "/api/users", map[string]string{"username": "ana", "password": "password123"}, admin)
	if rec.Code != 201 {
		t.Fatalf("create: %d %s", rec.Code, rec.Body.String())
	}
	user := loginAs(t, srv, "ana", "password123")

	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "ep.mkv"), bytes.Repeat([]byte("x"), 64), 0o644); err != nil {
		t.Fatal(err)
	}
	rec = do(t, srv, "POST", "/api/libraries", map[string]string{"name": "L", "type": "anime", "path": root}, admin)
	if rec.Code != 201 {
		t.Fatalf("library: %d %s", rec.Code, rec.Body.String())
	}
	rec = do(t, srv, "POST", "/api/library/scan", nil, admin)
	if rec.Code != 202 {
		t.Fatalf("scan: %d", rec.Code)
	}
	waitScan(t, srv, admin)
	var page struct {
		Items []map[string]any `json:"items"`
		Total int              `json:"total"`
	}
	rec = do(t, srv, "GET", "/api/catalog", nil, admin)
	_ = json.Unmarshal(rec.Body.Bytes(), &page)
	if len(page.Items) != 1 || page.Total != 1 {
		t.Fatalf("catalog: %+v", page)
	}
	id := page.Items[0]["id"].(string)

	// ana watches halfway; admin sees nothing.
	rec = do(t, srv, "PUT", "/api/items/"+id+"/progress", map[string]any{"position_sec": 300.0, "duration_sec": 600.0}, user)
	if rec.Code != 200 {
		t.Fatalf("progress put: %d", rec.Code)
	}
	rec = do(t, srv, "GET", "/api/items/"+id+"/progress", nil, admin)
	if rec.Code != 200 || bytes.Contains(rec.Body.Bytes(), []byte("300")) {
		t.Fatalf("admin must not see ana's progress: %d %s", rec.Code, rec.Body.String())
	}
	rec = do(t, srv, "GET", "/api/me/continue", nil, user)
	if rec.Code != 200 || !bytes.Contains(rec.Body.Bytes(), []byte(id)) {
		t.Fatalf("continue feed: %d %s", rec.Code, rec.Body.String())
	}
	rec = do(t, srv, "GET", "/api/me/continue", nil, admin)
	if rec.Code != 200 || bytes.Contains(rec.Body.Bytes(), []byte(id)) {
		t.Fatalf("admin continue must be empty: %d %s", rec.Code, rec.Body.String())
	}
}

func TestPasswordRotationKillsSession(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	old := admin
	rec := do(t, srv, "PATCH", "/api/me/password", map[string]string{"old": "password123", "new": "newpassword123"}, admin)
	if rec.Code != 200 {
		t.Fatalf("rotate: %d %s", rec.Code, rec.Body.String())
	}
	if rec := do(t, srv, "GET", "/api/me", nil, old); rec.Code != 401 {
		t.Errorf("old token after rotation: %d, want 401", rec.Code)
	}
	_ = loginAs(t, srv, "admin", "newpassword123")
}
