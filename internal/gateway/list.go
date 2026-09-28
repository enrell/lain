// External list linking (D-078..D-081). The lain list is a per-user
// tracking domain next to catalog and userstate; a linked platform
// imports its list into it. The OAuth dance is split by boundary: the
// gateway owns the inbound HTTP (authorize URL, callback, signed
// state), while platform providers behind lain.listlink@1 own the
// remote calls — providers never touch net/http (D-006), and tokens
// are per-user credentials that never leave the server or a log line
// (D-079/D-026).
package gateway

import (
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"time"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/auth"
	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/kv"
	"github.com/enrell/lain/internal/plugins/list"
	"github.com/enrell/lain/internal/plugins/listlink"
	"github.com/enrell/lain/internal/plugins/settings"
)

func (s *Server) routesList() {
	m := s.mux
	m.HandleFunc("GET /api/me/links", s.requireAuth(s.handleLinks))
	m.HandleFunc("GET /api/me/links/{platform}/authorize", s.requireAuth(s.handleLinkAuthorize))
	m.HandleFunc("GET /api/me/links/{platform}/pin", s.requireAuth(s.handleLinkPin))
	m.HandleFunc("POST /api/me/links/{platform}/code", s.requireAuth(s.handleLinkCode))
	m.HandleFunc("DELETE /api/me/links/{platform}", s.requireAuth(s.handleLinkDelete))
	m.HandleFunc("POST /api/me/links/{platform}/sync", s.requireAuth(s.handleLinkSync))
	m.HandleFunc("GET /api/auth/{platform}/callback", s.handleLinkCallback)
	m.HandleFunc("GET /api/list", s.requireAuth(s.handleList))

	m.HandleFunc("GET /api/admin/settings/integrations", s.requireAdmin(s.handleIntegrationsGet))
	m.HandleFunc("PUT /api/admin/settings/integrations", s.requireAdmin(s.handleIntegrationsPut))
}

// linkView is the client-safe projection of a linked account: identity
// and sync state, never the token (D-079).
type linkView struct {
	Platform       string `json:"platform"`
	RemoteUserID   string `json:"remote_user_id"`
	RemoteUsername string `json:"remote_username"`
	LinkedAt       int64  `json:"linked_at"`
	LastSyncAt     int64  `json:"last_sync_at,omitempty"`
	LastSyncError  string `json:"last_sync_error,omitempty"`
	EntryCount     int    `json:"entry_count"`
	TokenExpiresAt int64  `json:"token_expires_at,omitempty"`
	TokenExpired   bool   `json:"token_expired,omitempty"`
}

func publicLink(a contracts.LinkedAccount, now int64) linkView {
	return linkView{
		Platform:       a.Platform,
		RemoteUserID:   a.RemoteUserID,
		RemoteUsername: a.RemoteUsername,
		LinkedAt:       a.LinkedAt,
		LastSyncAt:     a.LastSyncAt,
		LastSyncError:  a.LastSyncError,
		EntryCount:     a.EntryCount,
		TokenExpiresAt: a.TokenExpiresAt,
		TokenExpired:   a.TokenExpired(now),
	}
}

// linkClient returns the operator-configured OAuth client for a
// platform. Unconfigured platforms answer false — the connector may be
// bound, but without an AniList app there is no flow to start (D-080).
func (s *Server) linkClient(platform string) (clientID, clientSecret string, configured bool) {
	integ := s.settings.Integrations()
	switch platform {
	case listlink.AniListPlatform:
		return integ.AniListClientID, integ.AniListClientSecret,
			integ.AniListClientID != "" && integ.AniListClientSecret != ""
	default:
		return "", "", false
	}
}

// listlinkInvoke tries the bound connectors in order; a provider that
// declines the platform is skipped, and the last decline is the answer
// when nothing serves it (the identify ordered-many pattern).
func (s *Server) listlinkInvoke(input any) (any, error) {
	provs, _, err := s.reg.Ordered(contracts.CapListLink)
	if err != nil {
		return nil, err
	}
	var lastErr error = &core.Error{Code: "unsupported-platform", Msg: "no list connector installed"}
	for _, p := range provs {
		out, err := p.Invoke(contracts.CapListLink, input)
		if err != nil {
			if ce, ok := err.(*core.Error); ok && ce.Code == "unsupported-platform" {
				lastErr = err
				continue
			}
			return out, err
		}
		return out, nil
	}
	return nil, lastErr
}

// handleLinks lists the caller's linked accounts.
func (s *Server) handleLinks(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	out, _, err := s.reg.CallOne(contracts.CapListAccount, list.ListAccountsInput{UserID: v.UserID})
	if err != nil {
		writeErr(w, http.StatusServiceUnavailable, err.Error())
		return
	}
	accounts, _ := out.([]contracts.LinkedAccount)
	views := make([]linkView, 0, len(accounts))
	now := time.Now().Unix()
	for _, a := range accounts {
		views = append(views, publicLink(a, now))
	}
	writeJSON(w, http.StatusOK, map[string]any{"links": views})
}

// callbackURL is the redirect URI AniList must send the user back to.
// It is derived from the request the client actually used, so the
// operator registers exactly this URL in the developer settings — the
// admin integrations page prints the same value.
func (s *Server) callbackURL(r *http.Request, platform string) string {
	scheme := "http"
	if r.TLS != nil {
		scheme = "https"
	}
	if fp := r.Header.Get("X-Forwarded-Proto"); fp == "https" || fp == "http" {
		scheme = fp
	}
	return scheme + "://" + r.Host + "/api/auth/" + platform + "/callback"
}

// handleLinkAuthorize starts an OAuth link: a signed, short-lived,
// user-bound state goes into the provider's authorize URL, so the
// callback can prove which lain account asked (D-080).
func (s *Server) handleLinkAuthorize(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	platform := r.PathValue("platform")
	clientID, _, configured := s.linkClient(platform)
	if !configured {
		writeJSON(w, http.StatusServiceUnavailable, map[string]string{
			"error": "no OAuth client configured for " + platform,
			"code":  "not-configured",
		})
		return
	}
	out, err := s.listlinkInvoke(contracts.LinkAuthorizeInput{
		Platform:    platform,
		ClientID:    clientID,
		RedirectURI: s.callbackURL(r, platform),
		State:       s.signLinkState(v.UserID, platform),
	})
	if err != nil {
		writeLinkErr(w, err)
		return
	}
	authOut, ok := out.(contracts.LinkAuthorizeOutput)
	if !ok || authOut.URL == "" {
		writeErr(w, http.StatusInternalServerError, "connector returned no url")
		return
	}
	writeJSON(w, http.StatusOK, authOut)
}

// The official Lain application for the auth-pin flow (D-083). The
// credentials are extractable by design — every distributed OAuth
// client's "secret" is (Taiga, Kometa and friends ship theirs in
// public source too); what it buys is zero-setup linking, and the
// registered redirect is AniList's own pin page so it works for every
// self-hosted origin. AniList refuses the implicit grant for
// console-created apps, so the pin page hands back an authorization
// code the server exchanges the usual way.
const (
	anilistPinClientID     = "52227"
	anilistPinClientSecret = "bcKlonBogr6E6ovB4r0DNWxFQ7vz3XUeh32S4HEp"
	anilistPinRedirectURI  = "https://anilist.co/api/v2/oauth/pin"
)

// handleLinkPin answers the authorize URL for the zero-setup link path:
// the user authorizes on AniList and pastes back the code the pin page
// shows.
func (s *Server) handleLinkPin(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	platform := r.PathValue("platform")
	if platform != listlink.AniListPlatform {
		writeJSON(w, http.StatusServiceUnavailable, map[string]string{
			"error": "no pin client configured for " + platform,
			"code":  "not-configured",
		})
		return
	}
	q := url.Values{
		"client_id":     {anilistPinClientID},
		"response_type": {"code"},
		"redirect_uri":  {anilistPinRedirectURI},
	}
	writeJSON(w, http.StatusOK, map[string]string{"url": "https://anilist.co/api/v2/oauth/authorize?" + q.Encode()})
}

// handleLinkCode exchanges the authorization code the pin page handed
// the user under the official shared app — a paste that fails never
// creates a link.
func (s *Server) handleLinkCode(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	platform := r.PathValue("platform")
	if platform != listlink.AniListPlatform {
		writeJSON(w, http.StatusServiceUnavailable, map[string]string{
			"error": "no pin client configured for " + platform,
			"code":  "not-configured",
		})
		return
	}
	var in struct {
		Code string `json:"code"`
	}
	if !s.decode(w, r, &in) {
		return
	}
	code := codeFromPaste(in.Code)
	if code == "" {
		writeErr(w, http.StatusBadRequest, "code is required")
		return
	}
	out, err := s.listlinkInvoke(contracts.LinkExchangeInput{
		Platform:     platform,
		ClientID:     anilistPinClientID,
		ClientSecret: anilistPinClientSecret,
		Code:         code,
		RedirectURI:  anilistPinRedirectURI,
	})
	if err != nil {
		writeLinkErr(w, err)
		return
	}
	identity, ok := out.(contracts.LinkedIdentity)
	if !ok || identity.Token == "" {
		writeErr(w, http.StatusInternalServerError, "connector returned no identity")
		return
	}
	account := contracts.LinkedAccount{
		UserID:         v.UserID,
		Platform:       platform,
		RemoteUserID:   identity.RemoteUserID,
		RemoteUsername: identity.RemoteUsername,
		Token:          identity.Token,
		TokenExpiresAt: identity.TokenExpiresAt,
		LinkedAt:       time.Now().Unix(),
	}
	s.putAccount(account)
	if _, syncErr := s.syncLinkedAccount(account); syncErr != nil {
		s.logger().Warn("first list import failed", "platform", platform, "user", v.UserID, "err", syncErr.Error())
	}
	s.logger().Info("list platform linked", "platform", platform, "user", v.UserID, "remote_user", identity.RemoteUsername, "via", "pin")
	writeJSON(w, http.StatusOK, map[string]any{"link": publicLink(account, time.Now().Unix())})
}

// codeFromPaste accepts the raw code, or the whole pin URL/fragment the
// user copied — `code=` or `access_token=` in any params wins.
func codeFromPaste(raw string) string {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return ""
	}
	for _, key := range []string{"code=", "access_token="} {
		i := strings.Index(raw, key)
		if i < 0 {
			continue
		}
		if vals, err := url.ParseQuery(strings.TrimPrefix(raw[i:], "#")); err == nil {
			if c := vals.Get(strings.TrimSuffix(key, "=")); c != "" {
				return c
			}
		}
	}
	return raw
}

// handleLinkCallback is the OAuth landing: public (no session header
// survives the cross-site redirect), with identity proven by the
// signed state. On success the account is stored and the first import
// runs, then the browser lands back on the account settings page.
func (s *Server) handleLinkCallback(w http.ResponseWriter, r *http.Request) {
	platform := r.PathValue("platform")
	redirectErr := func(code string) {
		http.Redirect(w, r, "/settings?link_error="+code, http.StatusFound)
	}
	if e := r.URL.Query().Get("error"); e != "" {
		redirectErr("denied")
		return
	}
	code := r.URL.Query().Get("code")
	state := r.URL.Query().Get("state")
	uid, ok := s.verifyLinkState(state, platform)
	if !ok || code == "" {
		writeErr(w, http.StatusBadRequest, "bad link state or code")
		return
	}
	// The state proves who asked; the account still has to exist — a
	// deleted or disabled user cannot complete a link.
	if u, ok := s.auth.Get(uid); !ok || u.Disabled {
		writeErr(w, http.StatusBadRequest, "unknown or disabled user")
		return
	}
	clientID, clientSecret, configured := s.linkClient(platform)
	if !configured {
		redirectErr("not-configured")
		return
	}
	out, err := s.listlinkInvoke(contracts.LinkExchangeInput{
		Platform:     platform,
		ClientID:     clientID,
		ClientSecret: clientSecret,
		Code:         code,
		RedirectURI:  s.callbackURL(r, platform),
	})
	if err != nil {
		redirectErr(linkErrCode(err))
		return
	}
	identity, ok := out.(contracts.LinkedIdentity)
	if !ok || identity.Token == "" {
		redirectErr("exchange-failed")
		return
	}
	account := contracts.LinkedAccount{
		UserID:         uid,
		Platform:       platform,
		RemoteUserID:   identity.RemoteUserID,
		RemoteUsername: identity.RemoteUsername,
		Token:          identity.Token,
		TokenExpiresAt: identity.TokenExpiresAt,
		LinkedAt:       time.Now().Unix(),
	}
	s.putAccount(account)
	// The first import rides the link: remote is authoritative (D-081).
	// A failed import still leaves a valid link — the error lands on
	// the account and the next sync retries.
	if _, syncErr := s.syncLinkedAccount(account); syncErr != nil {
		s.logger().Warn("first list import failed", "platform", platform, "user", uid, "err", syncErr.Error())
	}
	s.logger().Info("list platform linked", "platform", platform, "user", uid, "remote_user", identity.RemoteUsername)
	http.Redirect(w, r, "/settings?linked="+platform, http.StatusFound)
}

// handleLinkDelete unlinks: the token and the platform's imported
// entries are removed — the platform owns them (D-079).
func (s *Server) handleLinkDelete(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	platform := r.PathValue("platform")
	out, _, err := s.reg.CallOne(contracts.CapListAccount, list.DeleteAccountInput{UserID: v.UserID, Platform: platform})
	if err != nil {
		writeErr(w, http.StatusServiceUnavailable, err.Error())
		return
	}
	found, _ := out.(bool)
	if !found {
		writeErr(w, http.StatusNotFound, "no "+platform+" account linked")
		return
	}
	removed := 0
	if out, _, err := s.reg.CallOne(contracts.CapListWrite, list.DeletePlatformInput{UserID: v.UserID, Platform: platform}); err == nil {
		removed, _ = out.(int)
	} else {
		s.logger().Warn("platform entries delete failed", "platform", platform, "user", v.UserID, "err", err.Error())
	}
	s.logger().Info("list platform unlinked", "platform", platform, "user", v.UserID)
	writeJSON(w, http.StatusOK, map[string]any{"status": "ok", "removed": removed})
}

// handleLinkSync re-imports one linked account on demand.
func (s *Server) handleLinkSync(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	platform := r.PathValue("platform")
	account, ok := s.getAccount(v.UserID, platform)
	if !ok {
		writeErr(w, http.StatusNotFound, "no "+platform+" account linked")
		return
	}
	stats, err := s.syncLinkedAccount(account)
	if err != nil {
		writeLinkErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"status": "ok", "stats": stats})
}

// handleList returns the caller's unified list, filterable by media
// type and status.
func (s *Server) handleList(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	q := r.URL.Query()
	out, _, err := s.reg.CallOne(contracts.CapListRead, list.ListInput{
		UserID: v.UserID,
		Type:   q.Get("type"),
		Status: q.Get("status"),
	})
	if err != nil {
		writeErr(w, http.StatusServiceUnavailable, err.Error())
		return
	}
	entries, _ := out.([]contracts.ListEntry)
	writeJSON(w, http.StatusOK, map[string]any{"entries": entries})
}

// syncLinkedAccount imports the remote list into the user's entries:
// remote-authoritative replace plus sync bookkeeping on the account.
// One sync per server at a time — the work is a handful of remote
// calls, and a ticker must never pile onto a manual sync.
var linkSyncMu sync.Mutex

func (s *Server) syncLinkedAccount(a contracts.LinkedAccount) (contracts.ListSyncStats, error) {
	linkSyncMu.Lock()
	defer linkSyncMu.Unlock()
	if a.TokenExpired(time.Now().Unix()) {
		err := &core.Error{Code: "token-expired", Msg: "the " + a.Platform + " token expired; link again"}
		s.markSyncError(a, "token-expired")
		return contracts.ListSyncStats{}, err
	}
	out, err := s.listlinkInvoke(contracts.LinkFetchInput{
		Platform:     a.Platform,
		Token:        a.Token,
		RemoteUserID: a.RemoteUserID,
	})
	if err != nil {
		s.markSyncError(a, linkErrCode(err))
		return contracts.ListSyncStats{}, err
	}
	fetched, ok := out.(contracts.LinkFetchOutput)
	if !ok {
		return contracts.ListSyncStats{}, &core.Error{Code: "internal", Msg: "connector returned a bad list shape"}
	}
	put, _, err := s.reg.CallOne(contracts.CapListWrite, list.PutPlatformInput{
		UserID: a.UserID, Platform: a.Platform, Entries: fetched.Entries,
	})
	if err != nil {
		return contracts.ListSyncStats{}, err
	}
	stats, _ := put.(contracts.ListSyncStats)
	a.LastSyncAt = time.Now().Unix()
	a.LastSyncError = ""
	a.EntryCount = len(fetched.Entries)
	s.putAccount(a)
	s.logger().Info("list synced", "platform", a.Platform, "user", a.UserID, "upserted", stats.Upserted, "removed", stats.Removed)
	return stats, nil
}

// markSyncError records a stable code on the account — never a remote
// response body, which could carry data the user did not ask to keep.
func (s *Server) markSyncError(a contracts.LinkedAccount, code string) {
	a.LastSyncError = code
	s.putAccount(a)
}

// linkErrCode reduces an error to the wire code the account and the
// client should see.
func linkErrCode(err error) string {
	if ce, ok := err.(*core.Error); ok {
		return ce.Code
	}
	return "sync-failed"
}

func writeLinkErr(w http.ResponseWriter, err error) {
	code := linkErrCode(err)
	status := http.StatusServiceUnavailable
	switch code {
	case "unsupported-platform":
		status = http.StatusNotFound
	case "token-invalid", "token-expired":
		status = http.StatusUnauthorized
	case "invalid-message", "invalid-grant":
		status = http.StatusBadRequest
	}
	writeJSON(w, status, map[string]string{"error": err.Error(), "code": code})
}

func (s *Server) getAccount(userID, platform string) (contracts.LinkedAccount, bool) {
	out, _, err := s.reg.CallOne(contracts.CapListAccount, list.GetAccountInput{UserID: userID, Platform: platform})
	if err != nil {
		return contracts.LinkedAccount{}, false
	}
	a, ok := out.(contracts.LinkedAccount)
	return a, ok
}

func (s *Server) putAccount(a contracts.LinkedAccount) {
	if _, _, err := s.reg.CallOne(contracts.CapListAccount, list.PutAccountInput{Account: a}); err != nil {
		s.logger().Warn("list account write failed", "platform", a.Platform, "user", a.UserID, "err", err.Error())
	}
}

// --- OAuth state: HMAC-signed, expiring, bound to uid+platform (D-080)

type linkState struct {
	UID      string `json:"uid"`
	Platform string `json:"p"`
	Exp      int64  `json:"exp"`
	Nonce    string `json:"n"`
}

const linkStateTTL = 10 * time.Minute

// loadOrCreateStateKey provisions the signing key for link states in
// the meta bucket — same pattern as the auth secret.
func loadOrCreateStateKey(db *bolt.DB) ([]byte, error) {
	var key []byte
	err := db.Update(func(tx *bolt.Tx) error {
		raw := tx.Bucket(kv.BMeta).Get([]byte("oauth_state_key"))
		if raw != nil {
			key = append(key, raw...)
			return nil
		}
		key = make([]byte, 32)
		if _, err := rand.Read(key); err != nil {
			return err
		}
		return tx.Bucket(kv.BMeta).Put([]byte("oauth_state_key"), key)
	})
	if err != nil {
		return nil, err
	}
	if len(key) < 32 {
		return nil, &core.Error{Code: "internal", Msg: "corrupt oauth state key"}
	}
	return key, nil
}

func (s *Server) signLinkState(uid, platform string) string {
	nonce := make([]byte, 12)
	_, _ = rand.Read(nonce)
	st := linkState{
		UID: uid, Platform: platform, Nonce: base64.RawURLEncoding.EncodeToString(nonce),
		Exp: time.Now().Add(linkStateTTL).Unix(),
	}
	raw, _ := json.Marshal(st)
	body := base64.RawURLEncoding.EncodeToString(raw)
	mac := hmac.New(sha256.New, s.stateKey)
	mac.Write([]byte(body))
	return body + "." + base64.RawURLEncoding.EncodeToString(mac.Sum(nil))
}

func (s *Server) verifyLinkState(state, platform string) (string, bool) {
	body, sig, found := strings.Cut(state, ".")
	if !found {
		return "", false
	}
	mac := hmac.New(sha256.New, s.stateKey)
	mac.Write([]byte(body))
	want := base64.RawURLEncoding.EncodeToString(mac.Sum(nil))
	if !hmac.Equal([]byte(want), []byte(sig)) {
		return "", false
	}
	raw, err := base64.RawURLEncoding.DecodeString(body)
	if err != nil {
		return "", false
	}
	var st linkState
	if err := json.Unmarshal(raw, &st); err != nil {
		return "", false
	}
	if st.Platform != platform || st.UID == "" || time.Now().Unix() > st.Exp {
		return "", false
	}
	return st.UID, true
}

// --- Scheduled import (D-081)

const listSyncInterval = 6 * time.Hour

// StartListSync begins the periodic import: an immediate catch-up pass
// then one pass per interval until shutdown. Serve calls it unless
// --list-sync=0 / LAIN_LIST_SYNC=0; a second call is a no-op.
func (s *Server) StartListSync() {
	s.scanMu.Lock()
	if s.listSyncStarted {
		s.scanMu.Unlock()
		return
	}
	s.listSyncStarted = true
	s.scanMu.Unlock()
	go s.syncAllLinks()
	go s.listSyncLoop()
}

func (s *Server) listSyncLoop() {
	interval := s.listSyncInterval
	if interval <= 0 {
		interval = listSyncInterval
	}
	tick := time.NewTicker(interval)
	defer tick.Stop()
	for {
		select {
		case <-s.listSyncDone:
			return
		case <-tick.C:
			s.syncAllLinks()
		}
	}
}

// syncAllLinks re-imports every linked account. Each failure is its
// own — one dead token never stalls the others.
func (s *Server) syncAllLinks() {
	out, _, err := s.reg.CallOne(contracts.CapListAccount, list.ListAccountsInput{})
	if err != nil {
		s.logger().Warn("list accounts read failed", "err", err.Error())
		return
	}
	accounts, _ := out.([]contracts.LinkedAccount)
	for _, a := range accounts {
		if _, err := s.syncLinkedAccount(a); err != nil {
			s.logger().Warn("scheduled list sync failed", "platform", a.Platform, "user", a.UserID, "err", linkErrCode(err))
		}
	}
}

// --- Operator integrations settings (D-080)

func (a settingsResolver) Integrations() contracts.IntegrationSettings {
	out, _, err := a.s.reg.CallOne(contracts.CapIntegrationSettings, nil)
	if err != nil {
		return contracts.IntegrationSettings{}
	}
	st, ok := out.(contracts.IntegrationSettings)
	if !ok {
		return contracts.IntegrationSettings{}
	}
	return st
}

func (a settingsResolver) SaveIntegrations(in contracts.IntegrationSettings) error {
	_, _, err := a.s.reg.CallOne(contracts.CapIntegrationSettings, settings.IntPutInput{Settings: in})
	return err
}

func (a settingsResolver) EnsureIntegrations(base contracts.IntegrationSettings) error {
	_, _, err := a.s.reg.CallOne(contracts.CapIntegrationSettings, settings.IntEnsureInput{Base: base})
	return err
}

// handleIntegrationsGet reports which platforms are configured. The
// secret is never returned — only whether one is stored.
func (s *Server) handleIntegrationsGet(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	integ := s.settings.Integrations()
	writeJSON(w, http.StatusOK, map[string]any{
		"platforms": map[string]any{
			"anilist": map[string]any{
				"client_id":    integ.AniListClientID,
				"secret_set":   integ.AniListClientSecret != "",
				"callback_url": s.callbackURL(r, "anilist"),
			},
		},
	})
}

// handleIntegrationsPut saves the operator's OAuth client values. An
// empty secret keeps the stored one — the admin UI only sends it when
// it changes.
func (s *Server) handleIntegrationsPut(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	var in struct {
		AniListClientID     *string `json:"anilist_client_id"`
		AniListClientSecret *string `json:"anilist_client_secret"`
	}
	if !s.decode(w, r, &in) {
		return
	}
	cur := s.settings.Integrations()
	if in.AniListClientID != nil {
		cur.AniListClientID = strings.TrimSpace(*in.AniListClientID)
	}
	if in.AniListClientSecret != nil && *in.AniListClientSecret != "" {
		cur.AniListClientSecret = strings.TrimSpace(*in.AniListClientSecret)
	}
	if err := s.settings.SaveIntegrations(cur); err != nil {
		writeErr(w, http.StatusInternalServerError, err.Error())
		return
	}
	s.logger().Info("integrations settings updated", "req", reqIDOf(r))
	s.handleIntegrationsGet(w, r, auth.Verified{})
}
