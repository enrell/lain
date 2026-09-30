package gateway

import (
	"bytes"
	"encoding/json"
	"image"
	"image/png"
	"net/http/httptest"
	"strings"
	"testing"
)

func pngAvatar(w, h int) []byte {
	var b bytes.Buffer
	_ = png.Encode(&b, image.NewRGBA(image.Rect(0, 0, w, h)))
	return b.Bytes()
}

func putRaw(t *testing.T, srv *Server, path string, body []byte, token string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest("PUT", path, bytes.NewReader(body))
	req.Header.Set("Authorization", "Bearer "+token)
	rec := httptest.NewRecorder()
	srv.Handler().ServeHTTP(rec, req)
	return rec
}

type profileView struct {
	ID      string `json:"id"`
	Profile struct {
		DisplayName string `json:"display_name"`
		Bio         string `json:"bio"`
		Avatar      struct {
			Kind    string `json:"kind"`
			Mascot  string `json:"mascot"`
			Version int64  `json:"version"`
		} `json:"avatar"`
	} `json:"profile"`
}

func decodeProfile(t *testing.T, rec *httptest.ResponseRecorder) profileView {
	t.Helper()
	var v profileView
	if err := json.Unmarshal(rec.Body.Bytes(), &v); err != nil {
		t.Fatalf("decode: %v %s", err, rec.Body.String())
	}
	return v
}

func TestProfilePatch(t *testing.T) {
	srv := testServer(t)
	tok := setupAdmin(t, srv)
	rec := do(t, srv, "PATCH", "/api/me/profile", map[string]any{
		"display_name": "  Lain   Iwakura ", "bio": "present day\npresent time", "mascot": "wired",
	}, tok)
	if rec.Code != 200 {
		t.Fatalf("patch: %d %s", rec.Code, rec.Body.String())
	}
	v := decodeProfile(t, rec)
	if v.Profile.DisplayName != "Lain Iwakura" || v.Profile.Bio != "present day\npresent time" ||
		v.Profile.Avatar.Kind != "mascot" || v.Profile.Avatar.Mascot != "wired" || v.Profile.Avatar.Version == 0 {
		t.Fatalf("profile = %+v", v.Profile)
	}
	// A partial patch leaves the other fields alone.
	v = decodeProfile(t, do(t, srv, "PATCH", "/api/me/profile", map[string]any{"bio": ""}, tok))
	if v.Profile.DisplayName != "Lain Iwakura" || v.Profile.Bio != "" || v.Profile.Avatar.Mascot != "wired" {
		t.Fatalf("partial patch = %+v", v.Profile)
	}
	if me := decodeProfile(t, do(t, srv, "GET", "/api/me", nil, tok)); me.Profile.DisplayName != "Lain Iwakura" {
		t.Fatalf("/api/me profile = %+v", me.Profile)
	}
	for _, bad := range []map[string]any{
		{"mascot": "../etc"},
		{"display_name": strings.Repeat("x", 41)},
		{"bio": strings.Repeat("x", 161)},
		{"display_name": "a\x07b"},
	} {
		if rec := do(t, srv, "PATCH", "/api/me/profile", bad, tok); rec.Code != 400 {
			t.Errorf("%v: %d", bad, rec.Code)
		}
	}
	if rec := do(t, srv, "PATCH", "/api/me/profile", map[string]any{"bio": "x"}, ""); rec.Code != 401 {
		t.Fatalf("anonymous patch: %d", rec.Code)
	}
}

func TestAvatarUploadServeAndReplace(t *testing.T) {
	srv := testServer(t)
	tok := setupAdmin(t, srv)
	img := pngAvatar(64, 64)
	rec := putRaw(t, srv, "/api/me/avatar", img, tok)
	if rec.Code != 200 {
		t.Fatalf("upload: %d %s", rec.Code, rec.Body.String())
	}
	v := decodeProfile(t, rec)
	if v.Profile.Avatar.Kind != "upload" {
		t.Fatalf("avatar = %+v", v.Profile.Avatar)
	}
	get := do(t, srv, "GET", "/api/users/"+v.ID+"/avatar?token="+tok, nil, "")
	if get.Code != 200 || get.Header().Get("Content-Type") != "image/png" || !bytes.Equal(get.Body.Bytes(), img) {
		t.Fatalf("serve: %d %s", get.Code, get.Header().Get("Content-Type"))
	}
	if rec := do(t, srv, "GET", "/api/users/"+v.ID+"/avatar", nil, ""); rec.Code != 401 {
		t.Fatalf("anonymous avatar: %d", rec.Code)
	}
	// Choosing a mascot retires the uploaded file.
	do(t, srv, "PATCH", "/api/me/profile", map[string]any{"mascot": "moth"}, tok)
	if rec := do(t, srv, "GET", "/api/users/"+v.ID+"/avatar", nil, tok); rec.Code != 404 {
		t.Fatalf("after mascot: %d", rec.Code)
	}
	putRaw(t, srv, "/api/me/avatar", img, tok)
	rec = do(t, srv, "DELETE", "/api/me/avatar", nil, tok)
	if v := decodeProfile(t, rec); rec.Code != 200 || v.Profile.Avatar.Kind != "" {
		t.Fatalf("delete: %d %+v", rec.Code, v.Profile.Avatar)
	}
	if rec := do(t, srv, "GET", "/api/users/"+v.ID+"/avatar", nil, tok); rec.Code != 404 {
		t.Fatalf("after delete: %d", rec.Code)
	}
}

func TestAvatarUploadRejects(t *testing.T) {
	srv := testServer(t)
	tok := setupAdmin(t, srv)
	cases := map[string]struct {
		body []byte
		want int
	}{
		"not an image": {[]byte("<svg onload=alert(1)>"), 415},
		"huge canvas":  {pngAvatar(5000, 1), 415},
		"too big":      {bytes.Repeat([]byte{0}, maxAvatarBytes+1), 413},
	}
	for name, c := range cases {
		if rec := putRaw(t, srv, "/api/me/avatar", c.body, tok); rec.Code != c.want {
			t.Errorf("%s: %d, want %d", name, rec.Code, c.want)
		}
	}
}
