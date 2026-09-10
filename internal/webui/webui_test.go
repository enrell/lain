package webui

import (
	"io/fs"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"testing/fstest"
)

func testFS() fstest.MapFS {
	return fstest.MapFS{
		"index.html":             &fstest.MapFile{Data: []byte("<!doctype html><div id=spa>LAIN-SPA</div>")},
		"_app/immutable/app.js":  &fstest.MapFile{Data: []byte("console.log('app')")},
		"_app/immutable/app.css": &fstest.MapFile{Data: []byte("body{color:red}")},
		"favicon.svg":            &fstest.MapFile{Data: []byte("<svg/>")},
		"fonts/inter.woff2":      &fstest.MapFile{Data: []byte("wOF2")},
	}
}

func do(t *testing.T, h http.Handler, method, target string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest(method, target, nil)
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	return rec
}

func TestSPARoutesServeIndex(t *testing.T) {
	h := Handler(testFS())
	for _, p := range []string{"/", "/library", "/library/lib-1", "/item/abc", "/player/abc", "/settings/plugins", "/deep/nested/route"} {
		rec := do(t, h, "GET", p)
		if rec.Code != 200 {
			t.Fatalf("GET %s: %d", p, rec.Code)
		}
		if ct := rec.Header().Get("Content-Type"); !strings.HasPrefix(ct, "text/html") {
			t.Fatalf("GET %s content-type %q", p, ct)
		}
		if !strings.Contains(rec.Body.String(), "LAIN-SPA") {
			t.Fatalf("GET %s did not serve the SPA shell: %q", p, rec.Body.String())
		}
		if cc := rec.Header().Get("Cache-Control"); cc != "no-cache" {
			t.Fatalf("index cache-control %q", cc)
		}
	}
}

func TestAssetsServedWithTypesAndCaching(t *testing.T) {
	h := Handler(testFS())
	rec := do(t, h, "GET", "/_app/immutable/app.js")
	if rec.Code != 200 || rec.Body.String() != "console.log('app')" {
		t.Fatalf("js asset: %d %q", rec.Code, rec.Body.String())
	}
	if ct := rec.Header().Get("Content-Type"); !strings.Contains(ct, "javascript") {
		t.Fatalf("js content-type %q", ct)
	}
	if cc := rec.Header().Get("Cache-Control"); !strings.Contains(cc, "immutable") {
		t.Fatalf("immutable cache-control %q", cc)
	}
	rec = do(t, h, "GET", "/_app/immutable/app.css")
	if !strings.Contains(rec.Header().Get("Content-Type"), "text/css") {
		t.Fatalf("css content-type %q", rec.Header().Get("Content-Type"))
	}
	rec = do(t, h, "GET", "/fonts/inter.woff2")
	if rec.Header().Get("Content-Type") != "font/woff2" {
		t.Fatalf("font content-type %q", rec.Header().Get("Content-Type"))
	}
	if cc := rec.Header().Get("Cache-Control"); !strings.Contains(cc, "max-age=3600") {
		t.Fatalf("asset cache-control %q", cc)
	}
}

func TestMissingAssetsAre404(t *testing.T) {
	h := Handler(testFS())
	for _, p := range []string{"/_app/immutable/missing.js", "/logo.png", "/favicon.ico", "/data.json"} {
		rec := do(t, h, "GET", p)
		if rec.Code != 404 {
			t.Fatalf("GET %s: %d, want 404 (body %q)", p, rec.Code, rec.Body.String())
		}
		if !strings.Contains(rec.Header().Get("Content-Type"), "application/json") {
			t.Fatalf("GET %s content-type %q", p, rec.Header().Get("Content-Type"))
		}
	}
}

func TestAPIPathsNeverServeHTML(t *testing.T) {
	h := Handler(testFS())
	for _, p := range []string{"/api/nope", "/api/catalog/unknown/extra", "/health/extra", "/api"} {
		rec := do(t, h, "GET", p)
		if rec.Code != 404 {
			t.Fatalf("GET %s: %d, want 404", p, rec.Code)
		}
		if strings.Contains(rec.Body.String(), "LAIN-SPA") {
			t.Fatalf("GET %s leaked the SPA shell", p)
		}
	}
}

func TestPathTraversalRejected(t *testing.T) {
	h := Handler(testFS())
	req := httptest.NewRequest("GET", "/", nil)
	req.URL.Path = "/../etc/passwd"
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != 400 {
		t.Fatalf("traversal: %d, want 400", rec.Code)
	}
	// url.Parse decodes %2e to '.', so the escaped form reaches the
	// handler as a real traversal too.
	rec = do(t, h, "GET", "/%2e%2e/secret")
	if rec.Code != 400 {
		t.Fatalf("encoded traversal: %d, want 400", rec.Code)
	}
}

func TestMethodNotAllowed(t *testing.T) {
	h := Handler(testFS())
	rec := do(t, h, "POST", "/library")
	if rec.Code != 405 || rec.Header().Get("Allow") != "GET, HEAD" {
		t.Fatalf("POST: %d allow=%q", rec.Code, rec.Header().Get("Allow"))
	}
}

func TestHeadReturnsHeaders(t *testing.T) {
	h := Handler(testFS())
	rec := do(t, h, "HEAD", "/_app/immutable/app.js")
	if rec.Code != 200 {
		t.Fatalf("HEAD: %d", rec.Code)
	}
	if rec.Body.Len() != 0 && rec.Body.Len() != len("console.log('app')") {
		t.Fatalf("HEAD body len %d", rec.Body.Len())
	}
}

func TestMissingIndexGivesBuildNotice(t *testing.T) {
	h := Handler(fstest.MapFS{".gitkeep": &fstest.MapFile{Data: []byte("")}})
	rec := do(t, h, "GET", "/")
	if rec.Code != http.StatusServiceUnavailable {
		t.Fatalf("unbuilt: %d", rec.Code)
	}
	if !strings.Contains(rec.Body.String(), "web UI") {
		t.Fatalf("unbuilt notice: %q", rec.Body.String())
	}
}

func TestNestedDirectoryLookupServesIndexNotDirectoryList(t *testing.T) {
	fsys := fstest.MapFS{
		"index.html": &fstest.MapFile{Data: []byte("SPA")},
		"docs/x.txt": &fstest.MapFile{Data: []byte("x")},
	}
	h := Handler(fsys)
	rec := do(t, h, "GET", "/docs")
	if rec.Code != 200 || rec.Body.String() != "SPA" {
		t.Fatalf("directory path: %d %q", rec.Code, rec.Body.String())
	}
	var _ fs.FS = fsys
}
