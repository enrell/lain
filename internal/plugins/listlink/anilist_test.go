package listlink

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

func newTestAniList(gql, oauth string) *AniList {
	a := NewAniList()
	a.GraphQL = gql
	a.OAuth = oauth
	a.http.minGap = 0
	return a
}

func fakeJWT(exp int64) string {
	head := base64.RawURLEncoding.EncodeToString([]byte(`{"alg":"HS256","typ":"JWT"}`))
	body, _ := json.Marshal(map[string]any{"exp": exp, "sub": 7})
	return head + "." + base64.RawURLEncoding.EncodeToString(body) + ".sig"
}

func TestAuthorizeURL(t *testing.T) {
	a := newTestAniList("http://unused", "https://anilist.co/api/v2/oauth")
	out, err := a.authorize(contracts.LinkAuthorizeInput{
		Platform: "anilist", ClientID: "42",
		RedirectURI: "http://lain.local:9360/api/auth/anilist/callback", State: "abc.def",
	})
	if err != nil {
		t.Fatal(err)
	}
	u, err := url.Parse(out.URL)
	if err != nil {
		t.Fatal(err)
	}
	if u.Host != "anilist.co" || u.Path != "/api/v2/oauth/authorize" {
		t.Fatalf("url: %s", out.URL)
	}
	q := u.Query()
	if q.Get("client_id") != "42" || q.Get("response_type") != "code" ||
		q.Get("redirect_uri") != "http://lain.local:9360/api/auth/anilist/callback" ||
		q.Get("state") != "abc.def" {
		t.Fatalf("query: %s", u.RawQuery)
	}
}

func TestUnsupportedPlatform(t *testing.T) {
	a := newTestAniList("http://unused", "http://unused")
	for _, in := range []any{
		contracts.LinkAuthorizeInput{Platform: "trakt"},
		contracts.LinkExchangeInput{Platform: "trakt"},
		contracts.LinkFetchInput{Platform: "trakt"},
	} {
		_, err := a.Invoke(contracts.CapListLink, in)
		ce, ok := err.(*core.Error)
		if !ok || ce.Code != "unsupported-platform" {
			t.Fatalf("%T: want unsupported-platform, got %v", in, err)
		}
	}
}

func TestExchange(t *testing.T) {
	token := fakeJWT(1893456000)
	var gotTokenBody map[string]any
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/oauth/token":
			_ = json.NewDecoder(r.Body).Decode(&gotTokenBody)
			fmt.Fprintf(w, `{"access_token":%q,"token_type":"Bearer","expires_in":31536000}`, token)
		default:
			if r.Header.Get("Authorization") != "Bearer "+token {
				w.WriteHeader(401)
				return
			}
			fmt.Fprint(w, `{"data":{"Viewer":{"id":7,"name":"lain"}}}`)
		}
	}))
	defer srv.Close()
	a := newTestAniList(srv.URL, srv.URL+"/oauth")
	id, err := a.exchange(contracts.LinkExchangeInput{
		Platform: "anilist", ClientID: "1", ClientSecret: "s",
		Code: "the-code", RedirectURI: "http://x/cb",
	})
	if err != nil {
		t.Fatal(err)
	}
	if id.Token != token || id.RemoteUserID != "7" || id.RemoteUsername != "lain" {
		t.Fatalf("identity: %+v", id)
	}
	if id.TokenExpiresAt != 1893456000 {
		t.Fatalf("expiry: %d", id.TokenExpiresAt)
	}
	if gotTokenBody["code"] != "the-code" || gotTokenBody["grant_type"] != "authorization_code" {
		t.Fatalf("token request: %+v", gotTokenBody)
	}
}

func TestExchangeInvalidGrant(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(400)
		fmt.Fprint(w, `{"error":"invalid_grant","error_description":"The authorization code is invalid"}`)
	}))
	defer srv.Close()
	a := newTestAniList(srv.URL, srv.URL)
	_, err := a.exchange(contracts.LinkExchangeInput{
		Platform: "anilist", ClientID: "1", ClientSecret: "s", Code: "bad", RedirectURI: "http://x/cb",
	})
	ce, ok := err.(*core.Error)
	if !ok || ce.Code != "invalid-grant" || !strings.Contains(ce.Msg, "authorization code") {
		t.Fatalf("want typed invalid-grant, got %v", err)
	}
}

const animeCollection = `{"data":{"MediaListCollection":{"lists":[
	{"name":"Watching","isCustomList":false,"entries":[
		{"mediaId":154587,"status":"CURRENT","progress":12,"progressVolumes":0,"score":8.5,"repeat":0,"notes":"",
		 "startedAt":{"year":2026,"month":1,"day":3},"completedAt":{"year":0,"month":0,"day":0},
		 "media":{"id":154587,"type":"ANIME","format":"TV","episodes":28,"chapters":0,
		  "title":{"romaji":"Sousou no Frieren","english":"Frieren: Beyond Journey's End","native":"..."},
		  "coverImage":{"extraLarge":"https://x/frieren-xl.jpg","large":"https://x/frieren.jpg"}}}
	]},
	{"name":"Custom","isCustomList":true,"entries":[
		{"mediaId":154587,"status":"CURRENT","progress":12,"progressVolumes":0,"score":8.5,"repeat":0,"notes":"dup",
		 "startedAt":{"year":0,"month":0,"day":0},"completedAt":{"year":0,"month":0,"day":0},
		 "media":{"id":154587,"type":"ANIME","format":"TV","episodes":28,"chapters":0,
		  "title":{"romaji":"Sousou no Frieren","english":"Frieren: Beyond Journey's End","native":"..."},
		  "coverImage":{"extraLarge":"","large":""}}}
	]}
]}}}`

const mangaCollection = `{"data":{"MediaListCollection":{"lists":[
	{"name":"Reading","isCustomList":false,"entries":[
		{"mediaId":30013,"status":"CURRENT","progress":60,"progressVolumes":7,"score":0,"repeat":0,"notes":"peak",
		 "startedAt":{"year":2025,"month":9,"day":1},"completedAt":{"year":0,"month":0,"day":0},
		 "media":{"id":30013,"type":"MANGA","format":"MANGA","episodes":0,"chapters":0,
		  "title":{"romaji":"One Piece","english":"One Piece","native":"..."},
		  "coverImage":{"extraLarge":"","large":"https://x/op.jpg"}}}
	]}
]}}}`

func TestFetchMapsEntries(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var in struct {
			Variables struct {
				Type string `json:"type"`
			} `json:"variables"`
		}
		_ = json.NewDecoder(r.Body).Decode(&in)
		if in.Variables.Type == "MANGA" {
			fmt.Fprint(w, mangaCollection)
		} else {
			fmt.Fprint(w, animeCollection)
		}
	}))
	defer srv.Close()
	a := newTestAniList(srv.URL, "http://unused")
	out, err := a.fetch(contracts.LinkFetchInput{Platform: "anilist", Token: "tok", RemoteUserID: "7"})
	if err != nil {
		t.Fatal(err)
	}
	if len(out.Entries) != 2 {
		t.Fatalf("entries=%d, want 2 (custom-list dup folded): %+v", len(out.Entries), out.Entries)
	}
	var anime, manga contracts.ListEntry
	for _, e := range out.Entries {
		if e.MediaType == contracts.ListMediaAnime {
			anime = e
		} else {
			manga = e
		}
	}
	if anime.Title != "Frieren: Beyond Journey's End" || anime.Progress != 12 ||
		anime.ProgressTotal != 28 || anime.Status != contracts.ListStatusCurrent ||
		anime.Cover != "https://x/frieren-xl.jpg" || anime.StartedAt != "2026-01-03" ||
		anime.Score != 8.5 || anime.Format != "TV" {
		t.Fatalf("anime entry: %+v", anime)
	}
	if manga.MediaType != contracts.ListMediaManga || manga.ProgressVolumes != 7 ||
		manga.Cover != "https://x/op.jpg" || manga.Notes != "peak" {
		t.Fatalf("manga entry: %+v", manga)
	}
}

func TestFetchTokenInvalid(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(401)
	}))
	defer srv.Close()
	a := newTestAniList(srv.URL, "http://unused")
	_, err := a.fetch(contracts.LinkFetchInput{Platform: "anilist", Token: "dead", RemoteUserID: "7"})
	ce, ok := err.(*core.Error)
	if !ok || ce.Code != "token-invalid" {
		t.Fatalf("want token-invalid, got %v", err)
	}
}

func TestJWTExpiry(t *testing.T) {
	if got := jwtExpiry("not-a-jwt"); got != 0 {
		t.Fatalf("bad jwt exp=%d", got)
	}
	if got := jwtExpiry(fakeJWT(1893456000)); got != 1893456000 {
		t.Fatalf("exp=%d", got)
	}
	_ = time.Now
}

func TestPushWritesProgressAndStatus(t *testing.T) {
	var got map[string]any
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "Bearer tok" {
			w.WriteHeader(401)
			return
		}
		_ = json.NewDecoder(r.Body).Decode(&got)
		fmt.Fprint(w, `{"data":{"SaveMediaListEntry":{"progress":4,"status":"CURRENT"}}}`)
	}))
	defer srv.Close()
	a := newTestAniList(srv.URL, srv.URL)
	out, err := a.Invoke(contracts.CapListLink, contracts.LinkPushInput{
		Platform: "anilist", Token: "tok", RemoteID: "154587", Progress: 4, Status: contracts.ListStatusCurrent,
	})
	if err != nil {
		t.Fatal(err)
	}
	if o := out.(contracts.LinkPushOutput); o.Progress != 4 || o.Status != contracts.ListStatusCurrent {
		t.Fatalf("output: %+v", o)
	}
	vars := got["variables"].(map[string]any)
	if vars["mediaId"] != float64(154587) || vars["progress"] != float64(4) || vars["status"] != "CURRENT" {
		t.Fatalf("variables: %+v", vars)
	}
}

func TestPushOmitsEmptyStatus(t *testing.T) {
	var got map[string]any
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_ = json.NewDecoder(r.Body).Decode(&got)
		fmt.Fprint(w, `{"data":{"SaveMediaListEntry":{"progress":1,"status":"CURRENT"}}}`)
	}))
	defer srv.Close()
	a := newTestAniList(srv.URL, srv.URL)
	if _, err := a.push(contracts.LinkPushInput{Platform: "anilist", Token: "t", RemoteID: "1", Progress: 1}); err != nil {
		t.Fatal(err)
	}
	if _, has := got["variables"].(map[string]any)["status"]; has {
		t.Fatalf("empty status must not be sent: %+v", got)
	}
}

func TestPushErrors(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		fmt.Fprint(w, `{"errors":[{"message":"Not Found."}],"data":{"SaveMediaListEntry":null}}`)
	}))
	defer srv.Close()
	a := newTestAniList(srv.URL, srv.URL)
	if _, err := a.push(contracts.LinkPushInput{Platform: "anilist", Token: "t", RemoteID: "1", Progress: 1}); err == nil || !strings.Contains(err.Error(), "Not Found") {
		t.Fatalf("graphql errors must surface, got %v", err)
	}
	for _, in := range []contracts.LinkPushInput{
		{Platform: "anilist", RemoteID: "1"},
		{Platform: "anilist", Token: "t", RemoteID: "x"},
		{Platform: "anilist", Token: "t", RemoteID: "1", Progress: -1},
	} {
		_, err := a.push(in)
		if ce, ok := err.(*core.Error); !ok || ce.Code != "invalid-message" {
			t.Fatalf("%+v: want invalid-message, got %v", in, err)
		}
	}
	if _, err := a.Invoke(contracts.CapListLink, contracts.LinkPushInput{Platform: "trakt"}); err == nil {
		t.Fatal("other platforms must be declined")
	}
}
