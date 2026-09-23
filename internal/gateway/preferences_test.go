package gateway

import (
	"encoding/json"
	"testing"
)

func TestPreferredLanguageBelongsToAuthenticatedAccount(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	rec := do(t, srv, "POST", "/api/users", map[string]string{"username": "viewer", "password": "password123"}, admin)
	if rec.Code != 201 {
		t.Fatalf("create: %d %s", rec.Code, rec.Body.String())
	}
	viewer := loginAs(t, srv, "viewer", "password123")
	if rec := do(t, srv, "PATCH", "/api/me/preferences", map[string]string{"preferred_language": "por"}, ""); rec.Code != 401 {
		t.Fatalf("anonymous update: %d", rec.Code)
	}
	rec = do(t, srv, "PATCH", "/api/me/preferences", map[string]string{"preferred_language": "POR"}, viewer)
	if rec.Code != 200 {
		t.Fatalf("update: %d %s", rec.Code, rec.Body.String())
	}
	var updated struct {
		PreferredLanguage string `json:"preferred_language"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &updated); err != nil || updated.PreferredLanguage != "por" {
		t.Fatalf("updated=%+v err=%v", updated, err)
	}
	rec = do(t, srv, "GET", "/api/me", nil, loginAs(t, srv, "viewer", "password123"))
	if err := json.Unmarshal(rec.Body.Bytes(), &updated); err != nil || updated.PreferredLanguage != "por" {
		t.Fatalf("relogin=%+v err=%v", updated, err)
	}
	rec = do(t, srv, "GET", "/api/me", nil, admin)
	updated = struct {
		PreferredLanguage string `json:"preferred_language"`
	}{}
	if string(rec.Body.Bytes()) == "" || json.Unmarshal(rec.Body.Bytes(), &updated) != nil || updated.PreferredLanguage != "" {
		t.Fatalf("preference leaked to admin: %s", rec.Body.String())
	}
	for _, bad := range []map[string]string{{"preferred_language": "pt-BR"}, {"preferred_language": "123"}, {}} {
		if rec := do(t, srv, "PATCH", "/api/me/preferences", bad, viewer); rec.Code != 400 {
			t.Fatalf("invalid preference accepted: %v => %d", bad, rec.Code)
		}
	}
	rec = do(t, srv, "PATCH", "/api/me/preferences", map[string]string{"preferred_language": ""}, viewer)
	if rec.Code != 200 {
		t.Fatalf("clear: %d %s", rec.Code, rec.Body.String())
	}
}
