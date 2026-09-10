package gateway

import (
	"bytes"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	bolt "go.etcd.io/bbolt"
)

func testServer(t *testing.T) *Server {
	t.Helper()
	srv, err := New(t.TempDir(), "test")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { srv.Close() })
	return srv
}

func do(t *testing.T, srv *Server, method, path string, body any, token string) *httptest.ResponseRecorder {
	t.Helper()
	var rdr *bytes.Reader
	if body != nil {
		raw, _ := json.Marshal(body)
		rdr = bytes.NewReader(raw)
	} else {
		rdr = bytes.NewReader(nil)
	}
	req := httptest.NewRequest(method, path, rdr)
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}
	rec := httptest.NewRecorder()
	srv.Handler().ServeHTTP(rec, req)
	return rec
}

func TestSetupLoginMe(t *testing.T) {
	srv := testServer(t)
	rec := do(t, srv, "POST", "/api/setup", map[string]string{"username": "admin", "password": "password123"}, "")
	if rec.Code != 201 {
		t.Fatalf("setup: %d %s", rec.Code, rec.Body.String())
	}
	rec = do(t, srv, "POST", "/api/setup", map[string]string{"username": "x", "password": "password123"}, "")
	if rec.Code != 400 {
		t.Fatalf("second setup must refuse: %d", rec.Code)
	}
	rec = do(t, srv, "POST", "/api/auth/login", map[string]string{"username": "admin", "password": "password123"}, "")
	if rec.Code != 200 {
		t.Fatalf("login: %d %s", rec.Code, rec.Body.String())
	}
	var tok struct {
		Token string `json:"token"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &tok); err != nil || tok.Token == "" {
		t.Fatalf("no token: %s", rec.Body.String())
	}
	rec = do(t, srv, "GET", "/api/me", nil, tok.Token)
	if rec.Code != 200 {
		t.Fatalf("me: %d %s", rec.Code, rec.Body.String())
	}
	rec = do(t, srv, "GET", "/api/me", nil, "bogus")
	if rec.Code != 401 {
		t.Fatalf("bogus token must 401: %d", rec.Code)
	}
}

func TestLibraryScanStreamProgress(t *testing.T) {
	srv := testServer(t)
	do(t, srv, "POST", "/api/setup", map[string]string{"username": "admin", "password": "password123"}, "")
	login := do(t, srv, "POST", "/api/auth/login", map[string]string{"username": "admin", "password": "password123"}, "")
	var tok struct {
		Token string `json:"token"`
	}
	_ = json.Unmarshal(login.Body.Bytes(), &tok)

	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "[Fansub-A] Frieren - 12 [1080p].mkv"), bytes.Repeat([]byte("0123456789abcdef"), 64), 0o644); err != nil {
		t.Fatal(err)
	}
	rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "Anime", "type": "anime", "path": root}, tok.Token)
	if rec.Code != 201 {
		t.Fatalf("library: %d %s", rec.Code, rec.Body.String())
	}
	rec = do(t, srv, "POST", "/api/library/scan", nil, tok.Token)
	if rec.Code != 202 {
		t.Fatalf("scan start: %d %s", rec.Code, rec.Body.String())
	}
	// Scan runs async; poll status briefly.
	var status struct {
		State string `json:"state"`
	}
	for i := 0; i < 100; i++ {
		rec = do(t, srv, "GET", "/api/library/scan", nil, tok.Token)
		_ = json.Unmarshal(rec.Body.Bytes(), &status)
		if status.State == "done" || status.State == "error" {
			break
		}
	}
	if status.State != "done" {
		t.Fatalf("scan state %q", status.State)
	}
	rec = do(t, srv, "GET", "/api/search?q=frieren", nil, tok.Token)
	if rec.Code != 200 || !bytes.Contains(rec.Body.Bytes(), []byte("Frieren")) {
		t.Fatalf("search: %d %s", rec.Code, rec.Body.String())
	}
	var page struct {
		Items []map[string]any `json:"items"`
		Total int              `json:"total"`
	}
	rec = do(t, srv, "GET", "/api/catalog", nil, tok.Token)
	if err := json.Unmarshal(rec.Body.Bytes(), &page); err != nil || len(page.Items) != 1 || page.Total != 1 {
		t.Fatalf("catalog: %d %s", rec.Code, rec.Body.String())
	}
	id := page.Items[0]["id"].(string)

	rec = do(t, srv, "GET", "/api/items/"+id+"/playback?client=mpv", nil, tok.Token)
	if rec.Code != 200 || !bytes.Contains(rec.Body.Bytes(), []byte(`"mode":"direct"`)) {
		t.Fatalf("playback: %d %s", rec.Code, rec.Body.String())
	}
	// Stream with Range + query token (no header).
	req := httptest.NewRequest("GET", "/api/items/"+id+"/stream?token="+tok.Token, nil)
	req.Header.Set("Range", "bytes=0-15")
	rec2 := httptest.NewRecorder()
	srv.Handler().ServeHTTP(rec2, req)
	if rec2.Code != 206 {
		t.Fatalf("range stream: %d", rec2.Code)
	}
	if rec2.Body.Len() != 16 {
		t.Fatalf("range body %d bytes, want 16", rec2.Body.Len())
	}

	prog := map[string]any{"position_sec": 120.5, "duration_sec": 1400.0}
	rec = do(t, srv, "PUT", "/api/items/"+id+"/progress", prog, tok.Token)
	if rec.Code != 200 {
		t.Fatalf("progress put: %d %s", rec.Code, rec.Body.String())
	}
	rec = do(t, srv, "GET", "/api/items/"+id+"/progress", nil, tok.Token)
	if rec.Code != 200 || !bytes.Contains(rec.Body.Bytes(), []byte("120.5")) {
		t.Fatalf("progress get: %d %s", rec.Code, rec.Body.String())
	}
	_ = http.StatusOK
}

func TestCatalogPagingEnvelope(t *testing.T) {
	srv := testServer(t)
	do(t, srv, "POST", "/api/setup", map[string]string{"username": "admin", "password": "password123"}, "")
	tok := func() string {
		rec := do(t, srv, "POST", "/api/auth/login", map[string]string{"username": "admin", "password": "password123"}, "")
		var v struct {
			Token string `json:"token"`
		}
		_ = json.Unmarshal(rec.Body.Bytes(), &v)
		return v.Token
	}()

	root := t.TempDir()
	for _, f := range []string{"b.mkv", "a.mkv", "c.mkv"} {
		if err := os.WriteFile(filepath.Join(root, f), []byte("x"), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	if rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "L", "type": "anime", "path": root}, tok); rec.Code != 201 {
		t.Fatalf("library: %d", rec.Code)
	}
	if rec := do(t, srv, "POST", "/api/library/scan", nil, tok); rec.Code != 202 {
		t.Fatalf("scan: %d", rec.Code)
	}
	var status struct {
		State string `json:"state"`
	}
	for i := 0; i < 100; i++ {
		rec := do(t, srv, "GET", "/api/library/scan", nil, tok)
		_ = json.Unmarshal(rec.Body.Bytes(), &status)
		if status.State == "done" {
			break
		}
	}
	var page struct {
		Items []map[string]any `json:"items"`
		Total int              `json:"total"`
		Limit int              `json:"limit"`
	}
	rec := do(t, srv, "GET", "/api/catalog?limit=2&offset=1", nil, tok)
	if err := json.Unmarshal(rec.Body.Bytes(), &page); err != nil {
		t.Fatal(err)
	}
	if page.Total != 3 || len(page.Items) != 1+1 || page.Limit != 2 {
		t.Fatalf("page: %+v", page)
	}
	if page.Items[0]["title"] != "b" || page.Items[1]["title"] != "c" {
		t.Fatalf("order: %+v", page.Items)
	}
	// Limit is capped, offset past end yields empty items with exact total.
	rec = do(t, srv, "GET", "/api/catalog?limit=99999&offset=99", nil, tok)
	_ = json.Unmarshal(rec.Body.Bytes(), &page)
	if page.Limit != 500 || len(page.Items) != 0 || page.Total != 3 {
		t.Fatalf("capped page: %+v", page)
	}
	// Search pages through the same envelope.
	var spage struct {
		Items []map[string]any `json:"items"`
		Total int              `json:"total"`
	}
	rec = do(t, srv, "GET", "/api/search?q=a&limit=1", nil, tok)
	_ = json.Unmarshal(rec.Body.Bytes(), &spage)
	if spage.Total != 1 || len(spage.Items) != 1 {
		t.Fatalf("search page: %+v", spage)
	}
}

func TestAdminBackupStreamsValidDB(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	rec := do(t, srv, "POST", "/api/users", map[string]string{"username": "ana", "password": "password123"}, admin)
	if rec.Code != 201 {
		t.Fatalf("create: %d", rec.Code)
	}
	ana := loginAs(t, srv, "ana", "password123")
	if rec := do(t, srv, "GET", "/api/admin/backup", nil, ana); rec.Code != 403 {
		t.Fatalf("backup as user: %d, want 403", rec.Code)
	}
	rec = do(t, srv, "GET", "/api/admin/backup", nil, admin)
	if rec.Code != 200 {
		t.Fatalf("backup as admin: %d", rec.Code)
	}
	if ct := rec.Header().Get("Content-Type"); ct != "application/octet-stream" {
		t.Fatalf("content-type %q", ct)
	}
	// The stream must be an openable bolt file with our buckets.
	dir := t.TempDir()
	snap := filepath.Join(dir, "snap.db")
	if err := os.WriteFile(snap, rec.Body.Bytes(), 0o600); err != nil {
		t.Fatal(err)
	}
	db, err := bolt.Open(snap, 0o600, &bolt.Options{ReadOnly: true})
	if err != nil {
		t.Fatalf("snapshot not a bolt db: %v", err)
	}
	defer db.Close()
	if err := db.View(func(tx *bolt.Tx) error {
		for _, b := range [][]byte{[]byte("users"), []byte("libraries"), []byte("items")} {
			if tx.Bucket(b) == nil {
				return errMissingBucket(string(b))
			}
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}

func errMissingBucket(b string) error { return errors.New("missing bucket " + b) }
