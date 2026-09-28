package gateway

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"net/url"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

// fakeLink stands in for lain-listlink-anilist: registering under the
// same provider ID swaps the binding, so gateway flows exercise the
// real cap dispatch without a network.
type fakeLink struct {
	identity  contracts.LinkedIdentity
	exchangeE error
	entries   []contracts.ListEntry
	fetchErr  error
}

func (f *fakeLink) ID() string             { return "lain-listlink-anilist" }
func (f *fakeLink) Capabilities() []string { return []string{contracts.CapListLink} }
func (f *fakeLink) Health() error          { return nil }

func (f *fakeLink) Invoke(cap string, input any) (any, error) {
	switch in := input.(type) {
	case contracts.LinkAuthorizeInput:
		return contracts.LinkAuthorizeOutput{URL: "https://auth.test/oauth?client_id=" + in.ClientID + "&state=" + in.State + "&redirect_uri=" + in.RedirectURI}, nil
	case contracts.LinkExchangeInput:
		if f.exchangeE != nil {
			return nil, f.exchangeE
		}
		return f.identity, nil
	case contracts.LinkFetchInput:
		if f.fetchErr != nil {
			return nil, f.fetchErr
		}
		return contracts.LinkFetchOutput{Entries: f.entries}, nil
	default:
		return nil, &core.Error{Code: "invalid-message", Msg: "bad link input"}
	}
}

// setupLinkUser creates the admin, returns (token, userID).
func setupLinkUser(t *testing.T, srv *Server) (string, string) {
	t.Helper()
	do(t, srv, "POST", "/api/setup", map[string]string{"username": "admin", "password": "password123"}, "")
	login := do(t, srv, "POST", "/api/auth/login", map[string]string{"username": "admin", "password": "password123"}, "")
	var tok struct {
		Token string `json:"token"`
	}
	if err := json.Unmarshal(login.Body.Bytes(), &tok); err != nil || tok.Token == "" {
		t.Fatalf("login: %s", login.Body.String())
	}
	me := do(t, srv, "GET", "/api/me", nil, tok.Token)
	var u struct {
		ID string `json:"id"`
	}
	if err := json.Unmarshal(me.Body.Bytes(), &u); err != nil || u.ID == "" {
		t.Fatalf("me: %s", me.Body.String())
	}
	return tok.Token, u.ID
}

func configureAniList(t *testing.T, srv *Server, token string) {
	t.Helper()
	rec := do(t, srv, "PUT", "/api/admin/settings/integrations",
		map[string]string{"anilist_client_id": "12345", "anilist_client_secret": "sekret-client"}, token)
	if rec.Code != 200 {
		t.Fatalf("integrations put: %d %s", rec.Code, rec.Body.String())
	}
}

func linkAccount(t *testing.T, srv *Server, tok, uid string) {
	t.Helper()
	state := srv.signLinkState(uid, "anilist")
	rec := do(t, srv, "GET", "/api/auth/anilist/callback?code=abc&state="+state, nil, "")
	if rec.Code != 302 {
		t.Fatalf("callback: %d %s", rec.Code, rec.Body.String())
	}
	if loc := rec.Header().Get("Location"); loc != "/settings?linked=anilist" {
		t.Fatalf("callback redirect: %q", loc)
	}
}

func TestLinkStateRoundTrip(t *testing.T) {
	srv := testServer(t)
	state := srv.signLinkState("u1", "anilist")
	uid, ok := srv.verifyLinkState(state, "anilist")
	if !ok || uid != "u1" {
		t.Fatalf("round trip failed: %v %q", ok, uid)
	}
	if _, ok := srv.verifyLinkState(state, "mal"); ok {
		t.Fatal("state must be bound to its platform")
	}
	if _, ok := srv.verifyLinkState(state+"x", "anilist"); ok {
		t.Fatal("tampered signature must fail")
	}
	for _, bad := range []string{"", "x", "x.y", "x.y.z"} {
		if _, ok := srv.verifyLinkState(bad, "anilist"); ok {
			t.Fatalf("malformed state %q must fail", bad)
		}
	}
	// Expired but well-signed: craft it by hand under the same key.
	raw, _ := json.Marshal(linkState{UID: "u1", Platform: "anilist", Exp: time.Now().Unix() - 1})
	body := base64.RawURLEncoding.EncodeToString(raw)
	mac := hmac.New(sha256.New, srv.stateKey)
	mac.Write([]byte(body))
	expired := body + "." + base64.RawURLEncoding.EncodeToString(mac.Sum(nil))
	if _, ok := srv.verifyLinkState(expired, "anilist"); ok {
		t.Fatal("expired state must fail")
	}
}

func TestLinksRequireAuth(t *testing.T) {
	srv := testServer(t)
	for _, path := range []string{"/api/me/links", "/api/me/links/anilist/authorize", "/api/list"} {
		if rec := do(t, srv, "GET", path, nil, ""); rec.Code != 401 {
			t.Fatalf("%s must require auth: %d", path, rec.Code)
		}
	}
	if rec := do(t, srv, "DELETE", "/api/me/links/anilist", nil, ""); rec.Code != 401 {
		t.Fatalf("delete must require auth: %d", rec.Code)
	}
	if rec := do(t, srv, "POST", "/api/me/links/anilist/sync", nil, ""); rec.Code != 401 {
		t.Fatalf("sync must require auth: %d", rec.Code)
	}
}

func TestAuthorizeNotConfigured(t *testing.T) {
	srv := testServer(t)
	tok, _ := setupLinkUser(t, srv)
	rec := do(t, srv, "GET", "/api/me/links/anilist/authorize", nil, tok)
	if rec.Code != 503 {
		t.Fatalf("unconfigured authorize: %d", rec.Code)
	}
	var out struct {
		Code string `json:"code"`
	}
	_ = json.Unmarshal(rec.Body.Bytes(), &out)
	if out.Code != "not-configured" {
		t.Fatalf("want not-configured, got %s", rec.Body.String())
	}
	// An unknown platform is not configured either.
	rec = do(t, srv, "GET", "/api/me/links/nowhere/authorize", nil, tok)
	if rec.Code != 503 {
		t.Fatalf("unknown platform: %d", rec.Code)
	}
}

func TestAuthorizeConfiguredBindsUser(t *testing.T) {
	srv := testServer(t)
	tok, uid := setupLinkUser(t, srv)
	configureAniList(t, srv, tok)
	rec := do(t, srv, "GET", "/api/me/links/anilist/authorize", nil, tok)
	if rec.Code != 200 {
		t.Fatalf("authorize: %d %s", rec.Code, rec.Body.String())
	}
	var out struct {
		URL string `json:"url"`
	}
	_ = json.Unmarshal(rec.Body.Bytes(), &out)
	if !strings.Contains(out.URL, "anilist.co") || !strings.Contains(out.URL, "client_id=12345") {
		t.Fatalf("bad authorize url: %s", out.URL)
	}
	if !strings.Contains(out.URL, "redirect_uri=") || !strings.Contains(out.URL, "%2Fapi%2Fauth%2Fanilist%2Fcallback") {
		t.Fatalf("authorize url must carry the callback: %s", out.URL)
	}
	// The state must verify back to this user and platform.
	q := strings.SplitN(out.URL, "state=", 2)[1]
	state := strings.SplitN(q, "&", 2)[0]
	got, ok := srv.verifyLinkState(state, "anilist")
	if !ok || got != uid {
		t.Fatalf("state not bound to user: %v %q", ok, got)
	}
}

func TestCallbackLinksAndImports(t *testing.T) {
	srv := testServer(t)
	tok, uid := setupLinkUser(t, srv)
	configureAniList(t, srv, tok)
	srv.reg.Register(&fakeLink{
		identity: contracts.LinkedIdentity{
			Token: "sekret-token", RemoteUserID: "42", RemoteUsername: "watcher",
		},
		entries: []contracts.ListEntry{
			{RemoteID: "1", MediaType: "anime", Title: "Frieren", Status: "current", Progress: 12, ProgressTotal: 28},
			{RemoteID: "2", MediaType: "manga", Title: "Solo Quest", Status: "planning"},
		},
	})
	linkAccount(t, srv, tok, uid)

	rec := do(t, srv, "GET", "/api/me/links", nil, tok)
	if rec.Code != 200 {
		t.Fatalf("links: %d %s", rec.Code, rec.Body.String())
	}
	if strings.Contains(rec.Body.String(), "sekret") {
		t.Fatalf("token leaked in links payload: %s", rec.Body.String())
	}
	var links struct {
		Links []linkView `json:"links"`
	}
	_ = json.Unmarshal(rec.Body.Bytes(), &links)
	if len(links.Links) != 1 || links.Links[0].RemoteUsername != "watcher" || links.Links[0].EntryCount != 2 {
		t.Fatalf("bad link view: %s", rec.Body.String())
	}

	rec = do(t, srv, "GET", "/api/list", nil, tok)
	var list struct {
		Entries []contracts.ListEntry `json:"entries"`
	}
	_ = json.Unmarshal(rec.Body.Bytes(), &list)
	if len(list.Entries) != 2 || list.Entries[0].UserID != uid || list.Entries[0].Platform != "anilist" {
		t.Fatalf("bad list: %s", rec.Body.String())
	}
	// Type filter narrows to the manga only.
	rec = do(t, srv, "GET", "/api/list?type=manga", nil, tok)
	_ = json.Unmarshal(rec.Body.Bytes(), &list)
	if len(list.Entries) != 1 || list.Entries[0].MediaType != "manga" {
		t.Fatalf("type filter: %s", rec.Body.String())
	}
}

func TestCallbackBadInputs(t *testing.T) {
	srv := testServer(t)
	tok, uid := setupLinkUser(t, srv)
	configureAniList(t, srv, tok)
	// Remote refused: redirect with an error flag, no panic, no link.
	rec := do(t, srv, "GET", "/api/auth/anilist/callback?error=access_denied", nil, "")
	if rec.Code != 302 || rec.Header().Get("Location") != "/settings?link_error=denied" {
		t.Fatalf("denied callback: %d %q", rec.Code, rec.Header().Get("Location"))
	}
	// Bad state: hard 400.
	rec = do(t, srv, "GET", "/api/auth/anilist/callback?code=x&state=bogus", nil, "")
	if rec.Code != 400 {
		t.Fatalf("bad state: %d", rec.Code)
	}
	// Exchange failure: exchange-error redirect, no link stored.
	srv.reg.Register(&fakeLink{exchangeE: &core.Error{Code: "invalid-grant", Msg: "expired"}})
	state := srv.signLinkState(uid, "anilist")
	rec = do(t, srv, "GET", "/api/auth/anilist/callback?code=bad&state="+state, nil, "")
	if rec.Code != 302 || !strings.HasPrefix(rec.Header().Get("Location"), "/settings?link_error=") {
		t.Fatalf("failed exchange: %d %q", rec.Code, rec.Header().Get("Location"))
	}
	rec = do(t, srv, "GET", "/api/me/links", nil, tok)
	var links struct {
		Links []linkView `json:"links"`
	}
	_ = json.Unmarshal(rec.Body.Bytes(), &links)
	if len(links.Links) != 0 {
		t.Fatalf("failed exchange must not link: %s", rec.Body.String())
	}
}

func TestUnlinkRemovesAccountAndEntries(t *testing.T) {
	srv := testServer(t)
	tok, uid := setupLinkUser(t, srv)
	configureAniList(t, srv, tok)
	srv.reg.Register(&fakeLink{
		identity: contracts.LinkedIdentity{Token: "t", RemoteUserID: "42"},
		entries:  []contracts.ListEntry{{RemoteID: "1", Title: "Frieren"}},
	})
	linkAccount(t, srv, tok, uid)
	rec := do(t, srv, "DELETE", "/api/me/links/anilist", nil, tok)
	if rec.Code != 200 {
		t.Fatalf("unlink: %d %s", rec.Code, rec.Body.String())
	}
	var list struct {
		Entries []contracts.ListEntry `json:"entries"`
	}
	_ = json.Unmarshal(do(t, srv, "GET", "/api/list", nil, tok).Body.Bytes(), &list)
	if len(list.Entries) != 0 {
		t.Fatalf("unlink must remove platform entries: %d", len(list.Entries))
	}
	if rec := do(t, srv, "DELETE", "/api/me/links/anilist", nil, tok); rec.Code != 404 {
		t.Fatalf("second unlink must 404: %d", rec.Code)
	}
}

func TestSyncNowAndTokenInvalid(t *testing.T) {
	srv := testServer(t)
	tok, uid := setupLinkUser(t, srv)
	configureAniList(t, srv, tok)
	fake := &fakeLink{
		identity: contracts.LinkedIdentity{Token: "t", RemoteUserID: "42"},
		entries:  []contracts.ListEntry{{RemoteID: "1", Title: "Frieren"}},
	}
	srv.reg.Register(fake)
	linkAccount(t, srv, tok, uid)

	// Remote shrank: the missing entry is replaced away (D-081).
	fake.entries = []contracts.ListEntry{
		{RemoteID: "9", Title: "New Pick", Status: "current"},
	}
	rec := do(t, srv, "POST", "/api/me/links/anilist/sync", nil, tok)
	if rec.Code != 200 {
		t.Fatalf("sync: %d %s", rec.Code, rec.Body.String())
	}
	var syncOut struct {
		Stats contracts.ListSyncStats `json:"stats"`
	}
	_ = json.Unmarshal(rec.Body.Bytes(), &syncOut)
	if syncOut.Stats.Upserted != 1 || syncOut.Stats.Removed != 1 {
		t.Fatalf("sync stats: %s", rec.Body.String())
	}

	// Revoked token: 401, stable code, entry stays visible (D-081).
	fake.fetchErr = &core.Error{Code: "token-invalid", Msg: "unauthorized"}
	rec = do(t, srv, "POST", "/api/me/links/anilist/sync", nil, tok)
	if rec.Code != 401 {
		t.Fatalf("dead token sync: %d", rec.Code)
	}
	var links struct {
		Links []linkView `json:"links"`
	}
	_ = json.Unmarshal(do(t, srv, "GET", "/api/me/links", nil, tok).Body.Bytes(), &links)
	if links.Links[0].LastSyncError != "token-invalid" {
		t.Fatalf("link must record token-invalid: %s", links.Links[0].LastSyncError)
	}
	var list struct {
		Entries []contracts.ListEntry `json:"entries"`
	}
	_ = json.Unmarshal(do(t, srv, "GET", "/api/list", nil, tok).Body.Bytes(), &list)
	if len(list.Entries) != 1 || list.Entries[0].RemoteID != "9" {
		t.Fatalf("dead token must keep imported entries: %d", len(list.Entries))
	}
}

func TestScheduledSyncCoversAccounts(t *testing.T) {
	srv := testServer(t)
	tok, uid := setupLinkUser(t, srv)
	configureAniList(t, srv, tok)
	fake := &fakeLink{
		identity: contracts.LinkedIdentity{Token: "t", RemoteUserID: "42"},
		entries:  []contracts.ListEntry{{RemoteID: "1", Title: "Frieren"}},
	}
	srv.reg.Register(fake)
	linkAccount(t, srv, tok, uid)

	fake.entries = []contracts.ListEntry{{RemoteID: "7", Title: "Catch Up"}}
	srv.syncAllLinks()
	var list struct {
		Entries []contracts.ListEntry `json:"entries"`
	}
	_ = json.Unmarshal(do(t, srv, "GET", "/api/list", nil, tok).Body.Bytes(), &list)
	if len(list.Entries) != 1 || list.Entries[0].RemoteID != "7" {
		t.Fatalf("scheduled pass must re-import: %d", len(list.Entries))
	}
	// An already-expired token is marked, not fetched.
	a, ok := srv.getAccount(uid, "anilist")
	if !ok {
		t.Fatal("account vanished")
	}
	a.TokenExpiresAt = time.Now().Unix() - 1
	srv.putAccount(a)
	fake.fetchErr = &core.Error{Code: "fetch-failed", Msg: "must not be called"}
	srv.syncAllLinks()
	a, _ = srv.getAccount(uid, "anilist")
	if a.LastSyncError != "token-expired" {
		t.Fatalf("expired token must short-circuit: %q", a.LastSyncError)
	}
}

func TestIntegrationsAdminEndpoints(t *testing.T) {
	srv := testServer(t)
	tok, _ := setupLinkUser(t, srv)
	// Empty: configured:false shape.
	rec := do(t, srv, "GET", "/api/admin/settings/integrations", nil, tok)
	var out struct {
		Platforms map[string]struct {
			ClientID    string `json:"client_id"`
			SecretSet   bool   `json:"secret_set"`
			CallbackURL string `json:"callback_url"`
		} `json:"platforms"`
	}
	_ = json.Unmarshal(rec.Body.Bytes(), &out)
	an := out.Platforms["anilist"]
	if an.SecretSet || an.ClientID != "" || !strings.HasSuffix(an.CallbackURL, "/api/auth/anilist/callback") {
		t.Fatalf("empty integrations: %s", rec.Body.String())
	}
	// PUT stores the pair; the secret never comes back.
	rec = do(t, srv, "PUT", "/api/admin/settings/integrations",
		map[string]string{"anilist_client_id": "12345", "anilist_client_secret": "sekret-client"}, tok)
	if strings.Contains(rec.Body.String(), "sekret-client") {
		t.Fatalf("secret leaked: %s", rec.Body.String())
	}
	_ = json.Unmarshal(rec.Body.Bytes(), &out)
	if out.Platforms["anilist"].ClientID != "12345" || !out.Platforms["anilist"].SecretSet {
		t.Fatalf("integrations put: %s", rec.Body.String())
	}
	// A PUT without the secret keeps the stored one.
	rec = do(t, srv, "PUT", "/api/admin/settings/integrations",
		map[string]string{"anilist_client_id": "67890"}, tok)
	_ = json.Unmarshal(rec.Body.Bytes(), &out)
	if out.Platforms["anilist"].ClientID != "67890" || !out.Platforms["anilist"].SecretSet {
		t.Fatalf("secret must persist: %s", rec.Body.String())
	}
	rec = do(t, srv, "GET", "/api/me/links/anilist/authorize", nil, tok)
	if rec.Code != 200 || !strings.Contains(rec.Body.String(), "client_id=67890") {
		t.Fatalf("secret must still configure the flow: %d %s", rec.Code, rec.Body.String())
	}
}

func TestCodeFromPaste(t *testing.T) {
	for in, want := range map[string]string{
		"":           "",
		"   ":        "",
		"def502abc":  "def502abc",
		"  abc123  ": "abc123",
		"https://anilist.co/api/v2/oauth/pin?code=def502abc": "def502abc",
		"?code=abc&state=x": "abc",
		"code=abc":          "abc",
		"https://anilist.co/api/v2/oauth/pin#access_token=tok&token_type=Bearer": "tok",
	} {
		if got := codeFromPaste(in); got != want {
			t.Fatalf("codeFromPaste(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestPinEndpoint(t *testing.T) {
	srv := testServer(t)
	tok, _ := setupLinkUser(t, srv)
	rec := do(t, srv, "GET", "/api/me/links/anilist/pin", nil, tok)
	if rec.Code != 200 {
		t.Fatalf("pin: %d", rec.Code)
	}
	var out struct {
		URL string `json:"url"`
	}
	_ = json.Unmarshal(rec.Body.Bytes(), &out)
	u, err := url.Parse(out.URL)
	if err != nil {
		t.Fatal(err)
	}
	if u.Host != "anilist.co" || u.Path != "/api/v2/oauth/authorize" {
		t.Fatalf("url: %s", out.URL)
	}
	q := u.Query()
	if q.Get("client_id") != anilistPinClientID || q.Get("response_type") != "code" ||
		q.Get("redirect_uri") != anilistPinRedirectURI {
		t.Fatalf("query: %s", u.RawQuery)
	}
	// Platforms without a pin client still answer not-configured.
	if rec := do(t, srv, "GET", "/api/me/links/nowhere/pin", nil, tok); rec.Code != 503 {
		t.Fatalf("pin unknown platform: %d", rec.Code)
	}
}

func TestLinkByPastedCode(t *testing.T) {
	srv := testServer(t)
	tok, uid := setupLinkUser(t, srv)
	srv.reg.Register(&fakeLink{
		identity: contracts.LinkedIdentity{Token: "tok", RemoteUserID: "42", RemoteUsername: "watcher"},
		entries:  []contracts.ListEntry{{RemoteID: "1", Title: "Frieren", MediaType: "anime"}},
	})
	// A whole pin URL pasted by hand normalizes to the code inside.
	rec := do(t, srv, "POST", "/api/me/links/anilist/code",
		map[string]string{"code": "https://anilist.co/api/v2/oauth/pin?code=def502abc"}, tok)
	if rec.Code != 200 {
		t.Fatalf("code link: %d %s", rec.Code, rec.Body.String())
	}
	a, ok := srv.getAccount(uid, "anilist")
	if !ok || a.Token != "tok" || a.RemoteUsername != "watcher" {
		t.Fatalf("account: %+v", a)
	}
	var links struct {
		Links []linkView `json:"links"`
	}
	_ = json.Unmarshal(do(t, srv, "GET", "/api/me/links", nil, tok).Body.Bytes(), &links)
	if len(links.Links) != 1 || links.Links[0].EntryCount != 1 {
		t.Fatalf("link after code exchange: %+v", links.Links)
	}
	// Unknown platform is not-configured, not a connector call.
	if rec := do(t, srv, "POST", "/api/me/links/nowhere/code", map[string]string{"code": "x"}, tok); rec.Code != 503 {
		t.Fatalf("code unknown platform: %d", rec.Code)
	}
}

func TestLinkByPastedCodeRejects(t *testing.T) {
	srv := testServer(t)
	tok, _ := setupLinkUser(t, srv)
	// Empty and whitespace pastes are 400s.
	for _, body := range []map[string]string{{"code": ""}, {"code": "   "}} {
		if rec := do(t, srv, "POST", "/api/me/links/anilist/code", body, tok); rec.Code != 400 {
			t.Fatalf("empty code: %d", rec.Code)
		}
	}
	// A rejected code leaves no link behind.
	srv.reg.Register(&fakeLink{exchangeE: &core.Error{Code: "invalid-grant", Msg: "the authorization code is invalid"}})
	rec := do(t, srv, "POST", "/api/me/links/anilist/code", map[string]string{"code": "dead"}, tok)
	if rec.Code != 400 {
		t.Fatalf("dead code: %d", rec.Code)
	}
}

func TestIntegrationsRequireAdmin(t *testing.T) {
	srv := testServer(t)
	adminTok, _ := setupLinkUser(t, srv)
	// A non-admin user: create through the admin users endpoint.
	rec := do(t, srv, "POST", "/api/users", map[string]string{"username": "bob", "password": "password123"}, adminTok)
	if rec.Code != 201 && rec.Code != 200 {
		t.Fatalf("create user: %d %s", rec.Code, rec.Body.String())
	}
	login := do(t, srv, "POST", "/api/auth/login", map[string]string{"username": "bob", "password": "password123"}, "")
	var tok struct {
		Token string `json:"token"`
	}
	_ = json.Unmarshal(login.Body.Bytes(), &tok)
	if rec := do(t, srv, "GET", "/api/admin/settings/integrations", nil, tok.Token); rec.Code != 403 {
		t.Fatalf("non-admin integrations read: %d", rec.Code)
	}
	if rec := do(t, srv, "PUT", "/api/admin/settings/integrations", map[string]string{"anilist_client_id": "x"}, tok.Token); rec.Code != 403 {
		t.Fatalf("non-admin integrations write: %d", rec.Code)
	}
}

func TestEnvSeedFillsEmptyIntegrations(t *testing.T) {
	srv, err := NewWithOptions(t.TempDir(), "test", Options{
		transcodeProbe:      noTranscodeProbe,
		AniListClientID:     "env-id",
		AniListClientSecret: "env-secret",
	})
	if err != nil {
		t.Fatal(err)
	}
	defer srv.Close()
	integ := srv.settings.Integrations()
	if integ.AniListClientID != "env-id" || integ.AniListClientSecret != "env-secret" {
		t.Fatalf("env seed: %+v", integ)
	}
	// A second boot over the same dir keeps the saved document even
	// with different env values.
	dir := filepath.Dir(srv.db.Path())
	srv.Close()
	srv2, err := NewWithOptions(dir, "test", Options{
		transcodeProbe:      noTranscodeProbe,
		AniListClientID:     "different-id",
		AniListClientSecret: "different-secret",
	})
	if err != nil {
		t.Fatal(err)
	}
	defer srv2.Close()
	integ = srv2.settings.Integrations()
	if integ.AniListClientID != "env-id" {
		t.Fatalf("saved integrations must win over env: %+v", integ)
	}
}
