package gateway

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	"github.com/enrell/lain/internal/plugins/userstate"
	"github.com/enrell/lain/internal/store"
)

func testServer(t *testing.T) *Server {
	t.Helper()
	st, err := store.New(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	ustate, err := userstate.New(st)
	if err != nil {
		t.Fatal(err)
	}
	// New() re-opens the same dir; share it via data dir path.
	dir := st.Root()
	_ = dir
	srv, err := New(st.Root(), "test", ustate)
	if err != nil {
		t.Fatal(err)
	}
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
	if err := os.WriteFile(filepath.Join(root, "[Erai-raws] Frieren - 12 [1080p].mkv"), bytes.Repeat([]byte("0123456789abcdef"), 64), 0o644); err != nil {
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
	var items []map[string]any
	rec = do(t, srv, "GET", "/api/catalog", nil, tok.Token)
	if err := json.Unmarshal(rec.Body.Bytes(), &items); err != nil || len(items) != 1 {
		t.Fatalf("catalog: %d %s", rec.Code, rec.Body.String())
	}
	id := items[0]["id"].(string)

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
