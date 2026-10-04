package gateway

import (
	"archive/zip"
	"bytes"
	"encoding/json"
	"image"
	"image/png"
	"os"
	"path/filepath"
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/plugins/catalog"
)

func writeCBZ(t *testing.T, pages int) string {
	t.Helper()
	var img bytes.Buffer
	_ = png.Encode(&img, image.NewGray(image.Rect(0, 0, 16, 24)))
	p := filepath.Join(t.TempDir(), "Tiny Blade - Vol 01.cbz")
	f, _ := os.Create(p)
	zw := zip.NewWriter(f)
	for i := pages; i >= 1; i-- { // reverse insertion order: the index must sort
		w, _ := zw.Create("p" + string(rune('0'+i)) + ".png")
		_, _ = w.Write(img.Bytes())
	}
	_ = zw.Close()
	f.Close()
	return p
}

func seedReadable(t *testing.T, srv *Server, path, kind string) contracts.CatalogItem {
	t.Helper()
	out, _, err := srv.reg.CallOne(contracts.CapCatalogWrite, catalog.UpsertInput{
		LibraryID: "lib-read",
		Proposal:  contracts.Proposal{Kind: kind, Title: "Tiny Blade", Episode: 1, Confidence: 0.9, PluginID: "test"},
		Candidate: contracts.Candidate{Path: path, Size: 100, ModTime: 1, LibraryID: "lib-read"},
	})
	if err != nil {
		t.Fatal(err)
	}
	return out.(contracts.CatalogItem)
}

func TestReaderPagesAndBytes(t *testing.T) {
	srv := testServer(t)
	tok := setupAdmin(t, srv)
	it := seedReadable(t, srv, writeCBZ(t, 3), "manga")

	rec := do(t, srv, "GET", "/api/items/"+it.ID+"/pages", nil, tok)
	if rec.Code != 200 {
		t.Fatalf("pages: %d %s", rec.Code, rec.Body.String())
	}
	var v struct {
		Kind, Format, Direction string
		Pages                   []map[string]any
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &v); err != nil {
		t.Fatal(err)
	}
	if v.Kind != "manga" || v.Direction != "rtl" || v.Format != "cbz" || len(v.Pages) != 3 {
		t.Fatalf("view = %+v", v)
	}
	if _, leaked := v.Pages[0]["Name"]; leaked {
		t.Fatal("entry names must not reach clients")
	}
	if v.Pages[0]["width"].(float64) != 16 {
		t.Fatalf("page width = %v", v.Pages[0]["width"])
	}

	rec = do(t, srv, "GET", "/api/items/"+it.ID+"/pages/1", nil, tok)
	if rec.Code != 200 || rec.Header().Get("Content-Type") != "image/png" || rec.Body.Len() == 0 {
		t.Fatalf("page: %d %s len=%d", rec.Code, rec.Header().Get("Content-Type"), rec.Body.Len())
	}
	// <img> carries the token in the query string.
	if rec := do(t, srv, "GET", "/api/items/"+it.ID+"/pages/0?token="+tok, nil, ""); rec.Code != 200 {
		t.Fatalf("query token: %d", rec.Code)
	}
	if rec := do(t, srv, "GET", "/api/items/"+it.ID+"/pages/0", nil, ""); rec.Code != 401 {
		t.Fatalf("anonymous page: %d", rec.Code)
	}
	for path, want := range map[string]int{
		"/pages/3": 404, "/pages/-1": 400, "/pages/x": 400,
	} {
		if rec := do(t, srv, "GET", "/api/items/"+it.ID+path, nil, tok); rec.Code != want {
			t.Errorf("%s: %d, want %d", path, rec.Code, want)
		}
	}
	etag := do(t, srv, "GET", "/api/items/"+it.ID+"/pages/1", nil, tok).Header().Get("ETag")
	if etag == "" {
		t.Fatal("no etag")
	}
}

func TestReaderRefusesNonReadableAndComicDirection(t *testing.T) {
	srv := testServer(t)
	tok := setupAdmin(t, srv)
	video := seedItem(t, srv, "/x/a.mkv", "Show", 1)
	if rec := do(t, srv, "GET", "/api/items/"+video.ID+"/pages", nil, tok); rec.Code != 400 {
		t.Fatalf("video pages: %d", rec.Code)
	}
	if rec := do(t, srv, "GET", "/api/items/nope/pages", nil, tok); rec.Code != 404 {
		t.Fatalf("unknown: %d", rec.Code)
	}
	comic := seedReadable(t, srv, writeCBZ(t, 1), "comic")
	rec := do(t, srv, "GET", "/api/items/"+comic.ID+"/pages", nil, tok)
	var v struct{ Direction string }
	_ = json.Unmarshal(rec.Body.Bytes(), &v)
	if v.Direction != "ltr" {
		t.Fatalf("comic direction = %q", v.Direction)
	}
	bad := filepath.Join(t.TempDir(), "bad.cbz")
	_ = os.WriteFile(bad, []byte("nope"), 0o600)
	broken := seedReadable(t, srv, bad, "comic")
	if rec := do(t, srv, "GET", "/api/items/"+broken.ID+"/pages", nil, tok); rec.Code != 422 {
		t.Fatalf("corrupt archive: %d %s", rec.Code, rec.Body.String())
	}
}

func TestReaderInfoOverridesDirection(t *testing.T) {
	srv := testServer(t)
	tok := setupAdmin(t, srv)
	p := filepath.Join(t.TempDir(), "Tiny Blade 01.cbz")
	f, _ := os.Create(p)
	zw := zip.NewWriter(f)
	w, _ := zw.Create("ComicInfo.xml")
	_, _ = w.Write([]byte("<ComicInfo><Series>Tiny Blade</Series><Manga>YesAndRightToLeft</Manga></ComicInfo>"))
	var img bytes.Buffer
	_ = png.Encode(&img, image.NewGray(image.Rect(0, 0, 4, 6)))
	w, _ = zw.Create("p1.png")
	_, _ = w.Write(img.Bytes())
	_ = zw.Close()
	f.Close()
	// A comic library defaults to ltr; the archive declares rtl.
	it := seedReadable(t, srv, p, "comic")
	rec := do(t, srv, "GET", "/api/items/"+it.ID+"/pages", nil, tok)
	var v struct {
		Direction string
		Info      *contracts.ComicInfo
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &v); err != nil {
		t.Fatal(err)
	}
	if v.Direction != "rtl" || v.Info == nil || v.Info.Series != "Tiny Blade" {
		t.Fatalf("view = %+v info=%+v", v, v.Info)
	}
}
