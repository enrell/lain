package gateway

import (
	"encoding/json"
	"net/http/httptest"
	"net/url"
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/plugins/catalog"
)

// socialWorld is a server with an admin and three regular accounts.
type socialWorld struct {
	srv                *Server
	admin, ana, bo, cy string // tokens
	ids                map[string]string
}

func newSocialWorld(t *testing.T) socialWorld {
	t.Helper()
	srv := testServer(t)
	w := socialWorld{srv: srv, admin: setupAdmin(t, srv), ids: map[string]string{}}
	for _, name := range []string{"ana", "bo", "cy"} {
		rec := do(t, srv, "POST", "/api/users", map[string]string{"username": name, "password": "password123"}, w.admin)
		if rec.Code != 201 {
			t.Fatalf("create %s: %d %s", name, rec.Code, rec.Body.String())
		}
		var u struct{ ID string }
		_ = json.Unmarshal(rec.Body.Bytes(), &u)
		w.ids[name] = u.ID
	}
	w.ana = loginAs(t, srv, "ana", "password123")
	w.bo = loginAs(t, srv, "bo", "password123")
	w.cy = loginAs(t, srv, "cy", "password123")
	return w
}

func decodeJSON[T any](t *testing.T, rec *httptest.ResponseRecorder) T {
	t.Helper()
	var v T
	if err := json.Unmarshal(rec.Body.Bytes(), &v); err != nil {
		t.Fatalf("decode %q: %v", rec.Body.String(), err)
	}
	return v
}

func wantCode(t *testing.T, rec *httptest.ResponseRecorder, code int) {
	t.Helper()
	if rec.Code != code {
		t.Fatalf("status %d, want %d: %s", rec.Code, code, rec.Body.String())
	}
}

func (w socialWorld) befriend(t *testing.T, aTok, aName, bTok, bName string) {
	t.Helper()
	wantCode(t, do(t, w.srv, "POST", "/api/social/friends/"+w.ids[bName], nil, aTok), 200)
	rec := do(t, w.srv, "POST", "/api/social/friends/"+w.ids[aName], nil, bTok)
	wantCode(t, rec, 200)
	if got := decodeJSON[struct{ Relation string }](t, rec).Relation; got != "friend" {
		t.Fatalf("relation after accept: %q", got)
	}
}

func socialSeed(t *testing.T, srv *Server, kind, title string, episode int) contracts.CatalogItem {
	t.Helper()
	out, _, err := srv.reg.CallOne(contracts.CapCatalogWrite, catalog.UpsertInput{
		LibraryID: "lib-social",
		Proposal:  contracts.Proposal{Kind: kind, Title: title, Episode: episode, Confidence: 0.9, PluginID: "test"},
		Candidate: contracts.Candidate{Path: "/media/" + kind + "/" + title + "/" + string(rune('a'+episode)), Size: 1, ModTime: 1, LibraryID: "lib-social"},
	})
	if err != nil {
		t.Fatal(err)
	}
	return out.(contracts.CatalogItem)
}

func TestSocialRoutesRequireAuth(t *testing.T) {
	srv := testServer(t)
	for _, path := range []string{"/api/social/feed", "/api/social/friends", "/api/social/me/settings", "/api/social/users", "/api/social/notifications", "/api/social/collections"} {
		if rec := do(t, srv, "GET", path, nil, ""); rec.Code != 401 {
			t.Errorf("%s: %d", path, rec.Code)
		}
	}
}

func TestSocialFriendFlowOverHTTP(t *testing.T) {
	w := newSocialWorld(t)
	// Search finds discoverable accounts and never yourself.
	found := decodeJSON[struct {
		Users []struct {
			User     struct{ ID, Username string }
			Relation string
		}
	}](t, do(t, w.srv, "GET", "/api/social/users?q=B", nil, w.ana))
	if len(found.Users) != 1 || found.Users[0].User.Username != "bo" || found.Users[0].Relation != "none" {
		t.Fatalf("search: %+v", found)
	}
	wantCode(t, do(t, w.srv, "POST", "/api/social/friends/"+w.ids["bo"], nil, w.ana), 200)
	friends := decodeJSON[struct {
		Incoming []struct{ User struct{ Username string } }
	}](t, do(t, w.srv, "GET", "/api/social/friends", nil, w.bo))
	if len(friends.Incoming) != 1 || friends.Incoming[0].User.Username != "ana" {
		t.Fatalf("bo incoming: %+v", friends)
	}
	notes := decodeJSON[struct {
		Items  []contracts.Notification
		Unread int
		Users  map[string]struct{ Username string }
	}](t, do(t, w.srv, "GET", "/api/social/notifications", nil, w.bo))
	if notes.Unread != 1 || notes.Users[w.ids["ana"]].Username != "ana" {
		t.Fatalf("bo notifications: %+v", notes)
	}
	wantCode(t, do(t, w.srv, "POST", "/api/social/friends/"+w.ids["ana"]+"/decline", nil, w.bo), 200)
	wantCode(t, do(t, w.srv, "POST", "/api/social/friends/"+w.ids["ana"]+"/decline", nil, w.bo), 404)
	w.befriend(t, w.ana, "ana", w.bo, "bo")
	wantCode(t, do(t, w.srv, "DELETE", "/api/social/friends/"+w.ids["bo"], nil, w.ana), 200)
	wantCode(t, do(t, w.srv, "POST", "/api/social/friends/nope", nil, w.ana), 404)
	wantCode(t, do(t, w.srv, "POST", "/api/social/friends/"+w.ids["ana"], nil, w.ana), 400)
}

func TestSocialBlockHidesAccount(t *testing.T) {
	w := newSocialWorld(t)
	w.befriend(t, w.ana, "ana", w.bo, "bo")
	wantCode(t, do(t, w.srv, "POST", "/api/social/blocks/"+w.ids["bo"], nil, w.ana), 200)
	// For bo, ana no longer exists: profile, request, search.
	wantCode(t, do(t, w.srv, "GET", "/api/social/users/"+w.ids["ana"], nil, w.bo), 404)
	wantCode(t, do(t, w.srv, "POST", "/api/social/friends/"+w.ids["ana"], nil, w.bo), 404)
	found := decodeJSON[struct{ Users []any }](t, do(t, w.srv, "GET", "/api/social/users?q=ana", nil, w.bo))
	if len(found.Users) != 0 {
		t.Fatalf("blocked account in search: %+v", found)
	}
	// ana sees bo in her blocked list and can lift the block.
	friends := decodeJSON[struct{ Blocked []any }](t, do(t, w.srv, "GET", "/api/social/friends", nil, w.ana))
	if len(friends.Blocked) != 1 {
		t.Fatalf("blocked list: %+v", friends)
	}
	wantCode(t, do(t, w.srv, "DELETE", "/api/social/blocks/"+w.ids["bo"], nil, w.ana), 200)
	wantCode(t, do(t, w.srv, "GET", "/api/social/users/"+w.ids["ana"], nil, w.bo), 200)
}

func TestSocialProfilePrivacy(t *testing.T) {
	w := newSocialWorld(t)
	wantCode(t, do(t, w.srv, "PATCH", "/api/me/profile", map[string]any{"bio": "reads in the rain"}, w.ana), 200)
	item := socialSeed(t, w.srv, "comic", "Saga", 1)
	rec := do(t, w.srv, "PATCH", "/api/social/me/settings", map[string]any{
		"profile":   "friends",
		"favorites": []map[string]string{{"item_id": item.ID}, {"kind": "manga", "title": "Frieren"}},
	}, w.ana)
	wantCode(t, rec, 200)
	st := decodeJSON[contracts.SocialSettings](t, rec)
	if len(st.Favorites) != 2 || st.Favorites[0].Kind != "comic" || st.Favorites[0].Title != "Saga" {
		t.Fatalf("favorites resolved from the catalog: %+v", st.Favorites)
	}
	type profile struct {
		Relation  string
		Bio       *string
		Favorites []contracts.WorkRef
		Visible   struct{ Profile, Activity, Ratings bool }
	}
	p := decodeJSON[profile](t, do(t, w.srv, "GET", "/api/social/users/"+w.ids["ana"], nil, w.bo))
	if p.Bio != nil || len(p.Favorites) != 0 || p.Visible.Profile {
		t.Fatalf("stranger sees a friends-only profile: %+v", p)
	}
	w.befriend(t, w.ana, "ana", w.bo, "bo")
	p = decodeJSON[profile](t, do(t, w.srv, "GET", "/api/social/users/"+w.ids["ana"], nil, w.bo))
	if p.Bio == nil || *p.Bio != "reads in the rain" || len(p.Favorites) != 2 || !p.Visible.Activity || p.Relation != "friend" {
		t.Fatalf("friend profile: %+v", p)
	}
	// Non-discoverable accounts leave search for strangers, not friends.
	wantCode(t, do(t, w.srv, "PATCH", "/api/social/me/settings", map[string]any{"discoverable": false}, w.ana), 200)
	if f := decodeJSON[struct{ Users []any }](t, do(t, w.srv, "GET", "/api/social/users?q=ana", nil, w.cy)); len(f.Users) != 0 {
		t.Fatalf("hidden account found by stranger: %+v", f)
	}
	if f := decodeJSON[struct{ Users []any }](t, do(t, w.srv, "GET", "/api/social/users?q=ana", nil, w.bo)); len(f.Users) != 1 {
		t.Fatalf("friend lost hidden account: %+v", f)
	}
	wantCode(t, do(t, w.srv, "PATCH", "/api/social/me/settings", map[string]any{"activity": "everyone"}, w.ana), 400)
}

func TestSocialProgressFeedsActivity(t *testing.T) {
	w := newSocialWorld(t)
	w.befriend(t, w.ana, "ana", w.bo, "bo")
	ep := socialSeed(t, w.srv, "manga", "Frieren", 3)
	wantCode(t, do(t, w.srv, "PUT", "/api/items/"+ep.ID+"/progress", map[string]any{"position_sec": 4, "duration_sec": 40}, w.ana), 200)
	wantCode(t, do(t, w.srv, "PUT", "/api/items/"+ep.ID+"/progress", map[string]any{"position_sec": 5, "duration_sec": 40}, w.ana), 200)
	wantCode(t, do(t, w.srv, "PUT", "/api/items/"+ep.ID+"/progress", map[string]any{"position_sec": 40, "duration_sec": 40, "completed": true}, w.ana), 200)
	feed := decodeJSON[struct {
		Items []contracts.Activity
		Users map[string]struct{ Username string }
	}](t, do(t, w.srv, "GET", "/api/social/feed", nil, w.bo))
	if len(feed.Items) != 2 || feed.Items[0].Type != "completed" || feed.Items[1].Type != "progress" {
		t.Fatalf("feed: %+v", feed.Items)
	}
	a := feed.Items[0]
	if a.Work.Kind != "manga" || a.Work.Title != "Frieren" || a.Work.ItemID != ep.ID || a.Episode != 3 || feed.Users[w.ids["ana"]].Username != "ana" {
		t.Fatalf("row: %+v users=%v", a, feed.Users)
	}
	// Friends-only by default: a stranger's feed and profile activity are empty.
	if f := decodeJSON[struct{ Items []any }](t, do(t, w.srv, "GET", "/api/social/feed", nil, w.cy)); len(f.Items) != 0 {
		t.Fatalf("stranger feed: %+v", f)
	}
	if f := decodeJSON[struct{ Items []any }](t, do(t, w.srv, "GET", "/api/social/users/"+w.ids["ana"]+"/activity", nil, w.cy)); len(f.Items) != 0 {
		t.Fatalf("stranger activity: %+v", f)
	}
}

func TestSocialWorkRatingsCommentsAndShare(t *testing.T) {
	w := newSocialWorld(t)
	w.befriend(t, w.ana, "ana", w.bo, "bo")
	ep1 := socialSeed(t, w.srv, "anime", "Mushishi", 1)
	ep2 := socialSeed(t, w.srv, "anime", "Mushishi", 2)
	// Rating an episode rates the work: both episodes read the same view.
	wantCode(t, do(t, w.srv, "PUT", "/api/social/work/rating", map[string]any{"work": map[string]string{"item_id": ep1.ID}, "score": 9, "review": "slow and luminous"}, w.ana), 200)
	wantCode(t, do(t, w.srv, "PUT", "/api/social/work/rating", map[string]any{"work": map[string]string{"kind": "anime", "title": "MUSHISHI"}, "score": 7}, w.bo), 200)
	type view struct {
		Mine    *contracts.Rating
		Ratings []contracts.Rating
		Average float64
		Scored  int
		Comment []contracts.Comment `json:"comments"`
		Users   map[string]struct{ Username string }
	}
	v := decodeJSON[view](t, do(t, w.srv, "GET", "/api/social/work?item="+ep2.ID, nil, w.bo))
	if v.Mine == nil || v.Mine.Score != 7 || len(v.Ratings) != 1 || v.Ratings[0].Review != "slow and luminous" || v.Average != 8 {
		t.Fatalf("bo view: %+v", v)
	}
	if v.Users[w.ids["ana"]].Username != "ana" {
		t.Fatalf("users map: %+v", v.Users)
	}
	// cy is a stranger: friends-only ratings stay hidden.
	v = decodeJSON[view](t, do(t, w.srv, "GET", "/api/social/work?kind=anime&title="+url.QueryEscape("Mushishi"), nil, w.cy))
	if len(v.Ratings) != 0 || v.Scored != 0 {
		t.Fatalf("stranger view: %+v", v)
	}
	wantCode(t, do(t, w.srv, "GET", "/api/social/work?item=missing", nil, w.cy), 404)
	wantCode(t, do(t, w.srv, "PUT", "/api/social/work/rating", map[string]any{"work": map[string]string{"item_id": ep1.ID}, "score": 12}, w.ana), 400)

	// Comments: public acts, replies notify, authors and admins delete.
	rec := do(t, w.srv, "POST", "/api/social/work/comments", map[string]any{"work": map[string]string{"item_id": ep1.ID}, "body": "the swamp episode"}, w.cy)
	wantCode(t, rec, 201)
	top := decodeJSON[contracts.Comment](t, rec)
	rec = do(t, w.srv, "POST", "/api/social/work/comments", map[string]any{"work": map[string]string{"item_id": ep2.ID}, "body": "yes!", "reply_to": top.ID}, w.ana)
	wantCode(t, rec, 201)
	if n := decodeJSON[struct{ Unread int }](t, do(t, w.srv, "GET", "/api/social/notifications", nil, w.cy)); n.Unread != 1 {
		t.Fatalf("reply notification: %+v", n)
	}
	wantCode(t, do(t, w.srv, "DELETE", "/api/social/comments/"+top.ID, nil, w.bo), 403)
	wantCode(t, do(t, w.srv, "DELETE", "/api/social/comments/"+top.ID, nil, w.admin), 200)
	if v := decodeJSON[view](t, do(t, w.srv, "GET", "/api/social/work?item="+ep1.ID, nil, w.bo)); len(v.Comment) != 1 {
		t.Fatalf("comments after moderation: %+v", v.Comment)
	}

	// Shares go to friends only.
	wantCode(t, do(t, w.srv, "POST", "/api/social/share", map[string]any{"to": []string{w.ids["cy"]}, "work": map[string]string{"item_id": ep1.ID}}, w.ana), 403)
	wantCode(t, do(t, w.srv, "POST", "/api/social/share", map[string]any{"to": []string{w.ids["bo"]}, "work": map[string]string{"item_id": ep1.ID}, "message": "watch this"}, w.ana), 200)
	n := decodeJSON[struct{ Items []contracts.Notification }](t, do(t, w.srv, "GET", "/api/social/notifications", nil, w.bo))
	if n.Items[0].Type != "share" || n.Items[0].Work == nil || n.Items[0].Work.Title != "Mushishi" {
		t.Fatalf("share notification: %+v", n.Items[0])
	}
	wantCode(t, do(t, w.srv, "POST", "/api/social/notifications/read", map[string]any{"all": true}, w.bo), 200)
	if n := decodeJSON[struct{ Unread int }](t, do(t, w.srv, "GET", "/api/social/notifications", nil, w.bo)); n.Unread != 0 {
		t.Fatalf("unread after read-all: %d", n.Unread)
	}

	wantCode(t, do(t, w.srv, "DELETE", "/api/social/work/rating?item="+ep1.ID, nil, w.ana), 200)
	if r := decodeJSON[struct{ Ratings []any }](t, do(t, w.srv, "GET", "/api/social/users/"+w.ids["ana"]+"/ratings", nil, w.bo)); len(r.Ratings) != 0 {
		t.Fatalf("ratings after delete: %+v", r)
	}
}

func TestSocialCollectionsOverHTTP(t *testing.T) {
	w := newSocialWorld(t)
	w.befriend(t, w.ana, "ana", w.bo, "bo")
	vol := socialSeed(t, w.srv, "manga", "Yotsuba", 1)
	type colResp struct {
		Collection contracts.Collection
		Users      map[string]struct{ Username string }
	}
	rec := do(t, w.srv, "POST", "/api/social/collections", map[string]any{"name": "Comfort", "user_id": w.ids["bo"]}, w.ana)
	wantCode(t, rec, 201)
	col := decodeJSON[colResp](t, rec).Collection
	if col.OwnerID != w.ids["ana"] || col.Visibility != "private" {
		t.Fatalf("owner must come from the token: %+v", col)
	}
	wantCode(t, do(t, w.srv, "POST", "/api/social/collections/"+col.ID+"/items", map[string]any{"work": map[string]string{"item_id": vol.ID}, "note": "volume 1"}, w.ana), 200)
	wantCode(t, do(t, w.srv, "POST", "/api/social/collections/"+col.ID+"/items", map[string]any{"work": map[string]string{"kind": "movie", "title": "Paprika"}}, w.ana), 200)
	wantCode(t, do(t, w.srv, "GET", "/api/social/collections/"+col.ID, nil, w.bo), 404)
	wantCode(t, do(t, w.srv, "POST", "/api/social/collections/"+col.ID+"/share", map[string]any{"to": []string{w.ids["bo"]}}, w.ana), 200)
	got := decodeJSON[colResp](t, do(t, w.srv, "GET", "/api/social/collections/"+col.ID, nil, w.bo))
	if len(got.Collection.Items) != 2 || got.Collection.Items[0].Work.Kind != "manga" || got.Users[w.ids["ana"]].Username != "ana" {
		t.Fatalf("shared collection: %+v", got)
	}
	wantCode(t, do(t, w.srv, "POST", "/api/social/collections/"+col.ID+"/items", map[string]any{"work": map[string]string{"kind": "movie", "title": "x"}}, w.bo), 403)
	list := decodeJSON[struct{ Collections []contracts.Collection }](t, do(t, w.srv, "GET", "/api/social/collections", nil, w.bo))
	if len(list.Collections) != 1 {
		t.Fatalf("bo's collections incl. shared: %+v", list)
	}
	// Unshare, then make it public: strangers see it on the profile.
	wantCode(t, do(t, w.srv, "DELETE", "/api/social/collections/"+col.ID+"/share/"+w.ids["bo"], nil, w.ana), 200)
	wantCode(t, do(t, w.srv, "GET", "/api/social/collections/"+col.ID, nil, w.bo), 404)
	wantCode(t, do(t, w.srv, "PATCH", "/api/social/collections/"+col.ID, map[string]any{"visibility": "public"}, w.ana), 200)
	prof := decodeJSON[struct{ Collections []any }](t, do(t, w.srv, "GET", "/api/social/users/"+w.ids["ana"]+"/collections", nil, w.cy))
	if len(prof.Collections) != 1 {
		t.Fatalf("public collection on profile: %+v", prof)
	}
	wantCode(t, do(t, w.srv, "DELETE", "/api/social/collections/"+col.ID+"/items?kind=movie&title=Paprika", nil, w.ana), 200)
	wantCode(t, do(t, w.srv, "DELETE", "/api/social/collections/"+col.ID, nil, w.cy), 404)
	wantCode(t, do(t, w.srv, "DELETE", "/api/social/collections/"+col.ID, nil, w.ana), 200)
}

func TestSocialDisabledAccountsDisappear(t *testing.T) {
	w := newSocialWorld(t)
	w.befriend(t, w.ana, "ana", w.bo, "bo")
	ep := socialSeed(t, w.srv, "series", "Severance", 1)
	wantCode(t, do(t, w.srv, "PUT", "/api/items/"+ep.ID+"/progress", map[string]any{"position_sec": 1, "duration_sec": 10}, w.bo), 200)
	wantCode(t, do(t, w.srv, "PATCH", "/api/users/"+w.ids["bo"], map[string]any{"disabled": true}, w.admin), 200)
	if f := decodeJSON[struct{ Items []any }](t, do(t, w.srv, "GET", "/api/social/feed", nil, w.ana)); len(f.Items) != 0 {
		t.Fatalf("disabled account in feed: %+v", f)
	}
	wantCode(t, do(t, w.srv, "GET", "/api/social/users/"+w.ids["bo"], nil, w.ana), 404)
	if f := decodeJSON[struct{ Friends []any }](t, do(t, w.srv, "GET", "/api/social/friends", nil, w.ana)); len(f.Friends) != 0 {
		t.Fatalf("disabled friend listed: %+v", f)
	}
}
