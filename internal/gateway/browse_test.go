package gateway

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-22

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestBrowseListsSubdirectories(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)

	root := t.TempDir()
	for _, d := range []string{"Anime", "Movies", ".hidden"} {
		if err := os.MkdirAll(filepath.Join(root, d), 0o755); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.WriteFile(filepath.Join(root, "file.txt"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}

	rec := do(t, srv, "GET", "/api/browse?path="+root, nil, admin)
	if rec.Code != 200 {
		t.Fatalf("browse: %d %s", rec.Code, rec.Body.String())
	}
	var got browseResult
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	if got.Path != root || got.Parent != filepath.Dir(root) || got.Detected {
		t.Fatalf("envelope: %+v", got)
	}
	if len(got.Dirs) != 2 || got.Dirs[0].Name != "Anime" || got.Dirs[1].Name != "Movies" {
		t.Fatalf("dirs (hidden/files skipped, sorted): %+v", got.Dirs)
	}
	if got.Dirs[0].Path != filepath.Join(root, "Anime") {
		t.Fatalf("dir path: %+v", got.Dirs[0])
	}
}

func TestBrowseValidatesAndGates(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)

	for _, tc := range []struct{ path, err string }{
		{"relative/path", "path must be absolute"},
		{filepath.Join(t.TempDir(), "missing"), "path is not a readable directory"},
	} {
		rec := do(t, srv, "GET", "/api/browse?path="+tc.path, nil, admin)
		if rec.Code != 400 {
			t.Fatalf("browse %q: %d, want 400", tc.path, rec.Code)
		}
		var body map[string]string
		_ = json.Unmarshal(rec.Body.Bytes(), &body)
		if body["error"] != tc.err {
			t.Fatalf("browse %q error %q, want %q", tc.path, body["error"], tc.err)
		}
	}

	// Empty path auto-detects an existing readable directory.
	rec := do(t, srv, "GET", "/api/browse", nil, admin)
	if rec.Code != 200 {
		t.Fatalf("detect: %d %s", rec.Code, rec.Body.String())
	}
	var got browseResult
	_ = json.Unmarshal(rec.Body.Bytes(), &got)
	if !got.Detected {
		t.Fatalf("detect flag: %+v", got)
	}
	if fi, err := os.Stat(got.Path); err != nil || !fi.IsDir() {
		t.Fatalf("detected path not a dir: %+v", got)
	}

	// Regular users cannot browse the server filesystem.
	rec = do(t, srv, "POST", "/api/users", map[string]string{"username": "ana", "password": "password123"}, admin)
	if rec.Code != 201 {
		t.Fatalf("create user: %d", rec.Code)
	}
	user := loginAs(t, srv, "ana", "password123")
	if rec := do(t, srv, "GET", "/api/browse?path=/", nil, user); rec.Code != 403 {
		t.Fatalf("browse as user: %d, want 403", rec.Code)
	}
}
