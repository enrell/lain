package gateway

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestSidecarsAreListedAndServedAsWebVTT(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	wantCode(t, do(t, srv, "POST", "/api/users", map[string]string{"username": "ana", "password": "password123"}, admin), 201)
	user := loginAs(t, srv, "ana", "password123")

	dir := t.TempDir()
	media := filepath.Join(dir, "[Fansub-A] Show - 01.mkv")
	if err := os.WriteFile(media, []byte("video"), 0o644); err != nil {
		t.Fatal(err)
	}
	it := seedItem(t, srv, media, "Show", 1)
	files := map[string]string{
		"[Fansub-A] Show - 01.en.srt":           "1\n00:00:01,000 --> 00:00:02,000\nHello\n",
		"[Fansub-A] Show - 01.pt-BR.forced.ass": "[Events]\nFormat: Start, End, Text\nDialogue: 0:00:03.00,0:00:04.00,Ol\xe1\n",
		"[Fansub-A] Show - 01.fr.srt":           "not a subtitle\n",
		"[Fansub-A] Show - 02.en.srt":           "1\n00:00:01,000 --> 00:00:02,000\nOther episode\n",
	}
	for n, body := range files {
		if err := os.WriteFile(filepath.Join(dir, n), []byte(body), 0o644); err != nil {
			t.Fatal(err)
		}
	}

	wantCode(t, do(t, srv, "GET", "/api/items/"+it.ID+"/sidecars", nil, ""), 401)
	rec := do(t, srv, "GET", "/api/items/"+it.ID+"/sidecars", nil, user) // any signed-in user
	wantCode(t, rec, 200)
	var list struct {
		Sidecars []struct {
			Index    int    `json:"index"`
			Name     string `json:"name"`
			Language string `json:"language"`
			Forced   bool   `json:"forced"`
			Path     string `json:"path"`
		}
	}
	_ = json.Unmarshal(rec.Body.Bytes(), &list)
	if len(list.Sidecars) != 3 || list.Sidecars[0].Language != "eng" || list.Sidecars[2].Language != "por" || !list.Sidecars[2].Forced {
		t.Fatalf("list: %s", rec.Body.String())
	}
	if strings.Contains(rec.Body.String(), dir) {
		t.Fatal("server paths must not reach clients")
	}

	// Media elements cannot send headers: ?token= works like for streams.
	rec = do(t, srv, "GET", "/api/items/"+it.ID+"/sidecars/0?token="+user, nil, "")
	wantCode(t, rec, 200)
	if ct := rec.Header().Get("Content-Type"); !strings.HasPrefix(ct, "text/vtt") {
		t.Fatalf("content type %q", ct)
	}
	if rec.Body.String() != "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nHello\n\n" {
		t.Fatalf("vtt: %q", rec.Body.String())
	}
	rec = do(t, srv, "GET", "/api/items/"+it.ID+"/sidecars/2", nil, user)
	wantCode(t, rec, 200)
	if !strings.Contains(rec.Body.String(), "Olá") {
		t.Fatalf("ASS in Windows-1252 must arrive as UTF-8 WebVTT: %q", rec.Body.String())
	}
	wantCode(t, do(t, srv, "GET", "/api/items/"+it.ID+"/sidecars/1", nil, user), 422) // unreadable file
	wantCode(t, do(t, srv, "GET", "/api/items/"+it.ID+"/sidecars/9", nil, user), 404)
	wantCode(t, do(t, srv, "GET", "/api/items/"+it.ID+"/sidecars/-1", nil, user), 404)
	wantCode(t, do(t, srv, "GET", "/api/items/"+it.ID+"/sidecars/..%2F..%2Fetc", nil, user), 404)
	wantCode(t, do(t, srv, "GET", "/api/items/nope/sidecars", nil, user), 404)
}
