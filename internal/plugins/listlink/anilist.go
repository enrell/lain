package listlink

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"net/url"
	"strings"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

// AniListPlatform is the platform key the gateway and account store use.
const AniListPlatform = "anilist"

// AniList links a user's AniList account: OAuth2 authorization-code
// grant (AniList has no PKCE and no refresh tokens — access tokens are
// year-lived JWTs), then GraphQL reads for the media list.
type AniList struct {
	// GraphQL and OAuth are endpoint seams; tests point them at
	// httptest servers.
	GraphQL string
	OAuth   string
	http    *politeClient
}

func NewAniList() *AniList {
	return &AniList{
		GraphQL: "https://graphql.anilist.co",
		OAuth:   "https://anilist.co/api/v2/oauth",
		http:    newPoliteClient(700 * time.Millisecond),
	}
}

func (a *AniList) ID() string             { return "lain-listlink-anilist" }
func (a *AniList) Capabilities() []string { return []string{contracts.CapListLink} }
func (a *AniList) Health() error          { return nil }

func unsupportedPlatform(p string) *core.Error {
	return &core.Error{Code: "unsupported-platform", Msg: "anilist connector cannot serve " + p}
}

func (a *AniList) Invoke(cap string, input any) (any, error) {
	if cap != contracts.CapListLink {
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
	switch in := input.(type) {
	case contracts.LinkAuthorizeInput:
		if in.Platform != AniListPlatform {
			return nil, unsupportedPlatform(in.Platform)
		}
		return a.authorize(in)
	case contracts.LinkExchangeInput:
		if in.Platform != AniListPlatform {
			return nil, unsupportedPlatform(in.Platform)
		}
		return a.exchange(in)
	case contracts.LinkFetchInput:
		if in.Platform != AniListPlatform {
			return nil, unsupportedPlatform(in.Platform)
		}
		return a.fetch(in)
	case contracts.LinkPushInput:
		if in.Platform != AniListPlatform {
			return nil, unsupportedPlatform(in.Platform)
		}
		return a.push(in)
	default:
		return nil, &core.Error{Code: "invalid-message", Msg: "LinkAuthorizeInput, LinkExchangeInput, LinkFetchInput or LinkPushInput required"}
	}
}

func (a *AniList) authorize(in contracts.LinkAuthorizeInput) (contracts.LinkAuthorizeOutput, error) {
	if in.ClientID == "" || in.RedirectURI == "" || in.State == "" {
		return contracts.LinkAuthorizeOutput{}, &core.Error{Code: "invalid-message", Msg: "client_id, redirect_uri and state required"}
	}
	q := url.Values{
		"client_id":     {in.ClientID},
		"redirect_uri":  {in.RedirectURI},
		"response_type": {"code"},
		"state":         {in.State},
	}
	return contracts.LinkAuthorizeOutput{URL: a.OAuth + "/authorize?" + q.Encode()}, nil
}

func (a *AniList) exchange(in contracts.LinkExchangeInput) (contracts.LinkedIdentity, error) {
	if in.Code == "" || in.ClientID == "" || in.ClientSecret == "" || in.RedirectURI == "" {
		return contracts.LinkedIdentity{}, &core.Error{Code: "invalid-message", Msg: "code, client_id, client_secret and redirect_uri required"}
	}
	body, _ := json.Marshal(map[string]any{
		"grant_type":    "authorization_code",
		"client_id":     in.ClientID,
		"client_secret": in.ClientSecret,
		"redirect_uri":  in.RedirectURI,
		"code":          in.Code,
	})
	raw, status, err := a.http.postJSON(a.OAuth+"/token", body, "")
	if err != nil {
		return contracts.LinkedIdentity{}, err
	}
	if status/100 != 2 {
		return contracts.LinkedIdentity{}, &core.Error{Code: "invalid-grant", Msg: grantError(raw, status)}
	}
	var tok struct {
		AccessToken string `json:"access_token"`
		TokenType   string `json:"token_type"`
		ExpiresIn   int64  `json:"expires_in"`
	}
	if err := json.Unmarshal(raw, &tok); err != nil || tok.AccessToken == "" {
		return contracts.LinkedIdentity{}, &core.Error{Code: "invalid-grant", Msg: "token response carried no access_token"}
	}
	viewer, err := a.viewer(tok.AccessToken)
	if err != nil {
		return contracts.LinkedIdentity{}, err
	}
	return contracts.LinkedIdentity{
		Token:          tok.AccessToken,
		TokenExpiresAt: jwtExpiry(tok.AccessToken),
		RemoteUserID:   fmt.Sprint(viewer.ID),
		RemoteUsername: viewer.Name,
	}, nil
}

// grantError surfaces AniList's OAuth error description (invalid_grant,
// invalid_client) without ever echoing credentials.
func grantError(raw []byte, status int) string {
	var e struct {
		Error       string `json:"error"`
		Description string `json:"error_description"`
		Message     string `json:"message"`
	}
	_ = json.Unmarshal(raw, &e)
	desc := e.Description
	if desc == "" {
		desc = e.Error
	}
	if desc == "" {
		desc = e.Message
	}
	if desc == "" {
		desc = fmt.Sprintf("status %d", status)
	}
	return desc
}

// jwtExpiry reads the exp claim of an AniList access token (a JWT).
// Tokens we cannot read still work — 0 means "no known expiry".
func jwtExpiry(token string) int64 {
	parts := strings.Split(token, ".")
	if len(parts) != 3 {
		return 0
	}
	raw, err := base64.RawURLEncoding.DecodeString(parts[1])
	if err != nil {
		return 0
	}
	var claims struct {
		Exp int64 `json:"exp"`
	}
	if err := json.Unmarshal(raw, &claims); err != nil {
		return 0
	}
	return claims.Exp
}

type anilistViewer struct {
	ID   int    `json:"id"`
	Name string `json:"name"`
}

func (a *AniList) viewer(token string) (anilistViewer, error) {
	var doc struct {
		Data struct {
			Viewer anilistViewer `json:"Viewer"`
		} `json:"data"`
	}
	if err := a.graphql(token, `query { Viewer { id name } }`, nil, &doc); err != nil {
		return anilistViewer{}, err
	}
	if doc.Data.Viewer.ID == 0 {
		return anilistViewer{}, &core.Error{Code: "invalid-grant", Msg: "viewer query returned no account"}
	}
	return doc.Data.Viewer, nil
}

// graphql runs one authenticated query against the endpoint.
// A 401/403 is a dead or revoked credential — typed "token-invalid" so
// the gateway can mark the link instead of treating it as a transient
// failure (D-081).
func (a *AniList) graphql(token, query string, variables map[string]any, out any) error {
	body, _ := json.Marshal(map[string]any{"query": query, "variables": variables})
	raw, status, err := a.http.postJSON(a.GraphQL, body, token)
	if err != nil {
		return err
	}
	if status == 401 || status == 403 {
		return &core.Error{Code: "token-invalid", Msg: "anilist rejected the access token"}
	}
	if status/100 != 2 {
		return fmt.Errorf("anilist graphql: status %d: %.200s", status, raw)
	}
	if err := json.Unmarshal(raw, out); err != nil {
		return err
	}
	return nil
}

const collectionQuery = `query ($userId: Int, $type: MediaType) {
	MediaListCollection(userId: $userId, type: $type) {
		lists {
			name
			isCustomList
			entries {
				mediaId status progress progressVolumes
				score(format: POINT_10_DECIMAL) repeat notes
				startedAt { year month day }
				completedAt { year month day }
				media {
					id type format episodes chapters
					title { romaji english native }
					coverImage { extraLarge large }
				}
			}
		}
	}
}`

func (a *AniList) fetch(in contracts.LinkFetchInput) (contracts.LinkFetchOutput, error) {
	if in.Token == "" || in.RemoteUserID == "" {
		return contracts.LinkFetchOutput{}, &core.Error{Code: "invalid-message", Msg: "token and remote_user_id required"}
	}
	var uid int
	if _, err := fmt.Sscanf(in.RemoteUserID, "%d", &uid); err != nil || uid <= 0 {
		return contracts.LinkFetchOutput{}, &core.Error{Code: "invalid-message", Msg: "remote_user_id must be numeric"}
	}
	var out contracts.LinkFetchOutput
	for _, mediaType := range []string{"ANIME", "MANGA"} {
		col, err := a.collection(in.Token, uid, mediaType)
		if err != nil {
			return contracts.LinkFetchOutput{}, err
		}
		out.Entries = append(out.Entries, col...)
	}
	if out.Entries == nil {
		out.Entries = []contracts.ListEntry{}
	}
	return out, nil
}

const saveEntryMutation = `mutation ($mediaId: Int, $progress: Int, $status: MediaListStatus) {
	SaveMediaListEntry(mediaId: $mediaId, progress: $progress, status: $status) {
		progress status
	}
}`

// push writes progress (and optionally status) to one entry. GraphQL
// answers 200 with an "errors" array for validation failures, so the
// body is checked, not just the status.
func (a *AniList) push(in contracts.LinkPushInput) (contracts.LinkPushOutput, error) {
	var mediaID int
	if in.Token == "" {
		return contracts.LinkPushOutput{}, &core.Error{Code: "invalid-message", Msg: "token required"}
	}
	if _, err := fmt.Sscanf(in.RemoteID, "%d", &mediaID); err != nil || mediaID <= 0 {
		return contracts.LinkPushOutput{}, &core.Error{Code: "invalid-message", Msg: "remote_id must be numeric"}
	}
	if in.Progress < 0 {
		return contracts.LinkPushOutput{}, &core.Error{Code: "invalid-message", Msg: "progress must not be negative"}
	}
	vars := map[string]any{"mediaId": mediaID, "progress": in.Progress}
	if in.Status != "" {
		vars["status"] = strings.ToUpper(in.Status)
	}
	var doc struct {
		Data struct {
			Save struct {
				Progress int    `json:"progress"`
				Status   string `json:"status"`
			} `json:"SaveMediaListEntry"`
		} `json:"data"`
		Errors []struct {
			Message string `json:"message"`
		} `json:"errors"`
	}
	if err := a.graphql(in.Token, saveEntryMutation, vars, &doc); err != nil {
		return contracts.LinkPushOutput{}, err
	}
	if len(doc.Errors) > 0 {
		return contracts.LinkPushOutput{}, fmt.Errorf("anilist rejected the entry: %.200s", doc.Errors[0].Message)
	}
	return contracts.LinkPushOutput{Progress: doc.Data.Save.Progress, Status: mapStatus(doc.Data.Save.Status)}, nil
}

type alDate struct {
	Year  int `json:"year"`
	Month int `json:"month"`
	Day   int `json:"day"`
}

type alMedia struct {
	ID       int    `json:"id"`
	Type     string `json:"type"`
	Format   string `json:"format"`
	Episodes int    `json:"episodes"`
	Chapters int    `json:"chapters"`
	Title    struct {
		Romaji  string `json:"romaji"`
		English string `json:"english"`
		Native  string `json:"native"`
	} `json:"title"`
	CoverImage struct {
		ExtraLarge string `json:"extraLarge"`
		Large      string `json:"large"`
	} `json:"coverImage"`
}

type alEntry struct {
	MediaID         int     `json:"mediaId"`
	Status          string  `json:"status"`
	Progress        int     `json:"progress"`
	ProgressVolumes int     `json:"progressVolumes"`
	Score           float64 `json:"score"`
	Repeat          int     `json:"repeat"`
	Notes           string  `json:"notes"`
	StartedAt       alDate  `json:"startedAt"`
	CompletedAt     alDate  `json:"completedAt"`
	Media           alMedia `json:"media"`
}

type collectionDoc struct {
	Data struct {
		Collection struct {
			Lists []struct {
				Name         string    `json:"name"`
				IsCustomList bool      `json:"isCustomList"`
				Entries      []alEntry `json:"entries"`
			} `json:"lists"`
		} `json:"MediaListCollection"`
	} `json:"data"`
}

func (a *AniList) collection(token string, uid int, mediaType string) ([]contracts.ListEntry, error) {
	var doc collectionDoc
	err := a.graphql(token, collectionQuery, map[string]any{"userId": uid, "type": mediaType}, &doc)
	if err != nil {
		return nil, err
	}
	// One media can repeat across a user's custom lists; the standard
	// list's copy is the canonical one (D-081 remote authority).
	byMedia := map[int]contracts.ListEntry{}
	custom := map[int]contracts.ListEntry{}
	for _, l := range doc.Data.Collection.Lists {
		for _, e := range l.Entries {
			if e.Media.ID == 0 {
				continue
			}
			entry := mapEntry(e)
			if l.IsCustomList {
				if _, ok := byMedia[e.MediaID]; !ok {
					custom[e.MediaID] = entry
				}
				continue
			}
			byMedia[e.MediaID] = entry
		}
	}
	out := []contracts.ListEntry{}
	for id, e := range custom {
		if _, ok := byMedia[id]; !ok {
			byMedia[id] = e
		}
	}
	for _, e := range byMedia {
		out = append(out, e)
	}
	return out, nil
}

func mapEntry(e alEntry) contracts.ListEntry {
	entry := contracts.ListEntry{
		RemoteID:        fmt.Sprint(e.MediaID),
		Status:          mapStatus(e.Status),
		Progress:        e.Progress,
		ProgressVolumes: e.ProgressVolumes,
		Score:           e.Score,
		Repeat:          e.Repeat,
		Notes:           e.Notes,
		StartedAt:       formatDate(e.StartedAt.Year, e.StartedAt.Month, e.StartedAt.Day),
		CompletedAt:     formatDate(e.CompletedAt.Year, e.CompletedAt.Month, e.CompletedAt.Day),
		Format:          e.Media.Format,
		Title:           firstNonEmpty(e.Media.Title.English, e.Media.Title.Romaji, e.Media.Title.Native),
		Cover:           firstNonEmpty(e.Media.CoverImage.ExtraLarge, e.Media.CoverImage.Large),
	}
	switch e.Media.Type {
	case "ANIME":
		entry.MediaType = contracts.ListMediaAnime
		entry.ProgressTotal = e.Media.Episodes
	case "MANGA":
		entry.MediaType = contracts.ListMediaManga
		entry.ProgressTotal = e.Media.Chapters
	default:
		entry.MediaType = strings.ToLower(e.Media.Type)
	}
	return entry
}

func mapStatus(s string) string {
	switch strings.ToUpper(s) {
	case "CURRENT":
		return contracts.ListStatusCurrent
	case "PLANNING":
		return contracts.ListStatusPlanning
	case "COMPLETED":
		return contracts.ListStatusCompleted
	case "PAUSED":
		return contracts.ListStatusPaused
	case "DROPPED":
		return contracts.ListStatusDropped
	case "REPEATING":
		return contracts.ListStatusRepeating
	default:
		return strings.ToLower(s)
	}
}

func formatDate(y, m, d int) string {
	if y <= 0 {
		return ""
	}
	return fmt.Sprintf("%04d-%02d-%02d", y, m, d)
}

func firstNonEmpty(vs ...string) string {
	for _, v := range vs {
		if strings.TrimSpace(v) != "" {
			return strings.TrimSpace(v)
		}
	}
	return ""
}
