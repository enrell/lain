package gateway

import (
	"encoding/json"
	"testing"

	"github.com/enrell/lain/internal/contracts"
)

func entry(remote, title, status string, progress, total int) contracts.ListEntry {
	return contracts.ListEntry{
		Platform: "anilist", RemoteID: remote, MediaType: contracts.ListMediaAnime,
		Title: title, Status: status, Progress: progress, ProgressTotal: total,
	}
}

func TestPlanScrobble(t *testing.T) {
	ep := func(title string, season, episode int) contracts.CatalogItem {
		return contracts.CatalogItem{Kind: "anime", Title: title, Season: season, Episode: episode}
	}
	cases := []struct {
		name    string
		item    contracts.CatalogItem
		entries []contracts.ListEntry
		want    scrobbleDecision
		ok      bool
	}{
		{"advances a tracked entry", ep("Frieren", 1, 5), []contracts.ListEntry{entry("1", "frieren", "current", 4, 28)}, scrobbleDecision{"1", 5, "current"}, true},
		{"promotes planning to current", ep("Frieren", 0, 1), []contracts.ListEntry{entry("1", "Frieren", "planning", 0, 28)}, scrobbleDecision{"1", 1, "current"}, true},
		{"last episode completes", ep("Frieren", 1, 28), []contracts.ListEntry{entry("1", "Frieren", "current", 27, 28)}, scrobbleDecision{"1", 28, "completed"}, true},
		{"never regresses", ep("Frieren", 1, 3), []contracts.ListEntry{entry("1", "Frieren", "current", 5, 28)}, scrobbleDecision{}, false},
		{"leaves completed entries alone", ep("Frieren", 1, 3), []contracts.ListEntry{entry("1", "Frieren", "completed", 28, 28)}, scrobbleDecision{}, false},
		{"skips later seasons", ep("Frieren", 2, 3), []contracts.ListEntry{entry("1", "Frieren", "current", 1, 28)}, scrobbleDecision{}, false},
		{"skips untracked titles", ep("Other", 1, 1), []contracts.ListEntry{entry("1", "Frieren", "current", 0, 28)}, scrobbleDecision{}, false},
		{"skips episodes past the total", ep("Frieren", 1, 40), []contracts.ListEntry{entry("1", "Frieren", "current", 3, 28)}, scrobbleDecision{}, false},
		{"skips ambiguous matches", ep("Frieren", 1, 2), []contracts.ListEntry{entry("1", "Frieren", "current", 0, 12), entry("2", "frieren", "planning", 0, 12)}, scrobbleDecision{}, false},
		{"ignores manga", ep("Frieren", 1, 2), []contracts.ListEntry{{Platform: "anilist", RemoteID: "9", MediaType: "manga", Title: "Frieren", Status: "current"}}, scrobbleDecision{}, false},
		{"skips movies", contracts.CatalogItem{Kind: "movie", Title: "Frieren", Episode: 1}, []contracts.ListEntry{entry("1", "Frieren", "current", 0, 1)}, scrobbleDecision{}, false},
	}
	for _, c := range cases {
		got, ok := planScrobble(c.item, "anilist", c.entries)
		if ok != c.ok || got != c.want {
			t.Errorf("%s: got %+v,%v want %+v,%v", c.name, got, ok, c.want, c.ok)
		}
	}
}

func scrobbleFixture(t *testing.T) (*Server, *fakeLink, string, string) {
	t.Helper()
	srv := testServer(t)
	tok, uid := setupLinkUser(t, srv)
	configureAniList(t, srv, tok)
	fl := &fakeLink{
		identity: contracts.LinkedIdentity{Token: "sekret-token", RemoteUserID: "42", RemoteUsername: "watcher"},
		entries:  []contracts.ListEntry{{RemoteID: "1", MediaType: "anime", Title: "Frieren", Status: "current", Progress: 1, ProgressTotal: 28}},
	}
	srv.reg.Register(fl)
	linkAccount(t, srv, tok, uid)
	return srv, fl, tok, uid
}

func TestScrobbleIsOptIn(t *testing.T) {
	srv, fl, tok, uid := scrobbleFixture(t)
	it := seedItem(t, srv, "/tmp/frieren-02.mkv", "Frieren", 2)

	srv.scrobble(uid, it.ID)
	if len(fl.pushes) != 0 {
		t.Fatalf("scrobble must be off until the user enables it: %+v", fl.pushes)
	}

	rec := do(t, srv, "PATCH", "/api/me/links/anilist", map[string]bool{"scrobble": true}, tok)
	if rec.Code != 200 {
		t.Fatalf("patch: %d %s", rec.Code, rec.Body.String())
	}
	var view linkView
	_ = json.Unmarshal(rec.Body.Bytes(), &view)
	if !view.Scrobble {
		t.Fatalf("view: %s", rec.Body.String())
	}

	srv.scrobble(uid, it.ID)
	if len(fl.pushes) != 1 {
		t.Fatalf("want one push, got %+v", fl.pushes)
	}
	p := fl.pushes[0]
	if p.RemoteID != "1" || p.Progress != 2 || p.Status != "current" || p.Token != "sekret-token" {
		t.Fatalf("push: %+v", p)
	}

	// The stored entry followed, so finishing the same episode again is a no-op.
	srv.scrobble(uid, it.ID)
	if len(fl.pushes) != 1 {
		t.Fatalf("repeat finish pushed again: %+v", fl.pushes)
	}
	list := do(t, srv, "GET", "/api/list", nil, tok)
	var body struct {
		Entries []contracts.ListEntry `json:"entries"`
	}
	_ = json.Unmarshal(list.Body.Bytes(), &body)
	if len(body.Entries) != 1 || body.Entries[0].Progress != 2 {
		t.Fatalf("local entry not updated: %s", list.Body.String())
	}
}

func TestScrobblePatchRejectsBadInput(t *testing.T) {
	srv, _, tok, _ := scrobbleFixture(t)
	if rec := do(t, srv, "PATCH", "/api/me/links/anilist", map[string]string{"x": "y"}, tok); rec.Code != 400 {
		t.Fatalf("missing field: %d", rec.Code)
	}
	if rec := do(t, srv, "PATCH", "/api/me/links/trakt", map[string]bool{"scrobble": true}, tok); rec.Code != 404 {
		t.Fatalf("unlinked platform: %d", rec.Code)
	}
	if rec := do(t, srv, "PATCH", "/api/me/links/anilist", map[string]bool{"scrobble": true}, ""); rec.Code != 401 {
		t.Fatalf("unauthenticated: %d", rec.Code)
	}
}
