package gateway

import (
	"archive/zip"
	"bytes"
	"encoding/json"
	"image"
	"image/png"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

// cbzFixture is a two-page archive of a few hundred bytes.
func cbzFixture(t *testing.T) []byte {
	t.Helper()
	var img bytes.Buffer
	_ = png.Encode(&img, image.NewGray(image.Rect(0, 0, 8, 12)))
	var buf bytes.Buffer
	zw := zip.NewWriter(&buf)
	for _, n := range []string{"001.png", "002.png"} {
		w, _ := zw.Create(n)
		_, _ = w.Write(img.Bytes())
	}
	if err := zw.Close(); err != nil {
		t.Fatal(err)
	}
	return buf.Bytes()
}

// permissiveDownloads lifts the free-space floor: test temp dirs live on
// small filesystems (tmpfs) and the floor is covered in the package.
func permissiveDownloads(t *testing.T, srv *Server, tok string) {
	t.Helper()
	rec := do(t, srv, "PUT", "/api/downloads/settings", map[string]any{
		"dir": t.TempDir(), "max_bytes": 1 << 20, "min_free_bytes": 0, "concurrency": 2, "keep_finished_days": 30,
	}, tok)
	if rec.Code != 200 {
		t.Fatalf("settings: %d %s", rec.Code, rec.Body.String())
	}
}

func waitDownload(t *testing.T, srv *Server, tok, id, want string) map[string]any {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	var j map[string]any
	for time.Now().Before(deadline) {
		rec := do(t, srv, "GET", "/api/downloads/"+id, nil, tok)
		_ = json.Unmarshal(rec.Body.Bytes(), &j)
		if j["state"] == want {
			return j
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("download %s: %v, want %s", id, j, want)
	return nil
}

func TestDownloadIntoMangaLibraryIsScannedAndReadable(t *testing.T) {
	srv := testServer(t)
	tok := setupAdmin(t, srv)
	permissiveDownloads(t, srv, tok)
	body := cbzFixture(t)
	origin := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.ServeContent(w, r, "x.cbz", time.Unix(1700000000, 0), bytes.NewReader(body))
	}))
	defer origin.Close()

	root := t.TempDir()
	rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "Manga", "type": "manga", "path": root}, tok)
	var lib struct{ ID string }
	_ = json.Unmarshal(rec.Body.Bytes(), &lib)

	rec = do(t, srv, "POST", "/api/downloads", map[string]string{
		"url": origin.URL + "/files/Tiny%20Blade%20-%20Vol%2001.cbz", "library_id": lib.ID,
	}, tok)
	if rec.Code != 201 {
		t.Fatalf("add: %d %s", rec.Code, rec.Body.String())
	}
	var job struct{ ID string }
	_ = json.Unmarshal(rec.Body.Bytes(), &job)
	j := waitDownload(t, srv, tok, job.ID, "done")
	if !strings.HasSuffix(j["path"].(string), "Tiny Blade - Vol 01.cbz") {
		t.Fatalf("path = %v", j["path"])
	}
	// The finished download rescans its library: the volume shows up
	// as a manga item and its pages index.
	waitScan(t, srv, tok)
	rec = do(t, srv, "GET", "/api/catalog", nil, tok)
	var page struct {
		Items []struct{ ID, Kind, Title string }
	}
	_ = json.Unmarshal(rec.Body.Bytes(), &page)
	if len(page.Items) != 1 || page.Items[0].Kind != "manga" || page.Items[0].Title != "Tiny Blade" {
		t.Fatalf("catalog = %+v", page.Items)
	}
	rec = do(t, srv, "GET", "/api/items/"+page.Items[0].ID+"/pages", nil, tok)
	if rec.Code != 200 || !strings.Contains(rec.Body.String(), `"direction":"rtl"`) {
		t.Fatalf("pages: %d %s", rec.Code, rec.Body.String())
	}
	// The listing carries usage against the limits.
	rec = do(t, srv, "GET", "/api/downloads", nil, tok)
	var view struct {
		Jobs  []map[string]any
		Usage struct {
			UsedBytes int64 `json:"used_bytes"`
			MaxBytes  int64 `json:"max_bytes"`
		}
	}
	_ = json.Unmarshal(rec.Body.Bytes(), &view)
	if len(view.Jobs) != 1 || view.Usage.UsedBytes != int64(len(body)) || view.Usage.MaxBytes != 1<<20 {
		t.Fatalf("view = %+v", view)
	}
}

func TestDownloadRoutesAreAdminOnlyAndTyped(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	if rec := do(t, srv, "POST", "/api/users", map[string]string{"username": "ana", "password": "password123"}, admin); rec.Code != 201 {
		t.Fatalf("user: %d", rec.Code)
	}
	user := loginAs(t, srv, "ana", "password123")
	for _, c := range []struct{ method, path string }{
		{"GET", "/api/downloads"}, {"POST", "/api/downloads"}, {"GET", "/api/downloads/settings"},
		{"PUT", "/api/downloads/settings"}, {"POST", "/api/downloads/cleanup"},
		{"POST", "/api/downloads/x/pause"}, {"DELETE", "/api/downloads/x"},
	} {
		if rec := do(t, srv, c.method, c.path, map[string]string{}, user); rec.Code != 403 {
			t.Errorf("%s %s as user: %d", c.method, c.path, rec.Code)
		}
		if rec := do(t, srv, c.method, c.path, map[string]string{}, ""); rec.Code != 401 {
			t.Errorf("%s %s anonymous: %d", c.method, c.path, rec.Code)
		}
	}
	permissiveDownloads(t, srv, admin)

	expect := func(rec *httptest.ResponseRecorder, status int, code string) {
		t.Helper()
		var body map[string]string
		_ = json.Unmarshal(rec.Body.Bytes(), &body)
		if rec.Code != status || body["code"] != code {
			t.Fatalf("got %d %s, want %d %s", rec.Code, rec.Body.String(), status, code)
		}
	}
	expect(do(t, srv, "POST", "/api/downloads", map[string]string{"url": "file:///etc/passwd"}, admin), 400, "invalid-request")
	expect(do(t, srv, "POST", "/api/downloads", map[string]string{"url": "http://x.test/a", "library_id": "nope"}, admin), 404, "not-found")
	expect(do(t, srv, "POST", "/api/downloads/nope/pause", nil, admin), 404, "not-found")
	expect(do(t, srv, "PUT", "/api/downloads/settings", map[string]any{"dir": "rel", "concurrency": 1}, admin), 400, "invalid-request")
	if rec := do(t, srv, "POST", "/api/downloads/nope/explode", nil, admin); rec.Code != 404 {
		t.Fatalf("unknown action: %d", rec.Code)
	}

	// A queued job that cannot be removed until it stops: 409.
	rec := do(t, srv, "POST", "/api/downloads", map[string]string{"url": "http://127.0.0.1:1/never"}, admin)
	var job struct{ ID string }
	_ = json.Unmarshal(rec.Body.Bytes(), &job)
	if rec := do(t, srv, "POST", "/api/downloads/"+job.ID+"/pause", nil, admin); rec.Code != 200 {
		// It may already have failed against the closed port; both are
		// stopped states that block nothing below.
		if rec.Code != 409 {
			t.Fatalf("pause: %d %s", rec.Code, rec.Body.String())
		}
	}
	if rec := do(t, srv, "POST", "/api/downloads/"+job.ID+"/cancel", nil, admin); rec.Code != 200 {
		t.Fatalf("cancel: %d %s", rec.Code, rec.Body.String())
	}
	if rec := do(t, srv, "DELETE", "/api/downloads/"+job.ID, nil, admin); rec.Code != 200 {
		t.Fatalf("remove: %d %s", rec.Code, rec.Body.String())
	}
	if rec := do(t, srv, "POST", "/api/downloads/cleanup", nil, admin); rec.Code != 200 {
		t.Fatalf("cleanup: %d", rec.Code)
	}
}

func TestStreamDownloadDisposition(t *testing.T) {
	srv := testServer(t)
	tok := setupAdmin(t, srv)
	it := seedReadable(t, srv, writeCBZ(t, 2), "manga")
	rec := do(t, srv, "GET", "/api/items/"+it.ID+"/stream?download=1", nil, tok)
	if rec.Code != 200 || rec.Header().Get("Content-Disposition") != `attachment; filename="Tiny Blade - Vol 01.cbz"` {
		t.Fatalf("download: %d %q", rec.Code, rec.Header().Get("Content-Disposition"))
	}
	rec = do(t, srv, "GET", "/api/items/"+it.ID+"/stream", nil, tok)
	if rec.Header().Get("Content-Disposition") != "" {
		t.Fatal("plain streams stay inline")
	}
}
