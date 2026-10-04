// Package subtitles serves lain.subtitle@1 (docs/slices/acquisition.md,
// A-30). The OpenSubtitles.com provider speaks the public REST API v1
// with the operator's own API key; account credentials and the API key
// travel in the contract input and never reach a log line or an error.
package subtitles

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/url"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/subtitle"
)

// OpenSubtitlesID is the provider id.
const OpenSubtitlesID = "lain-subtitle-opensubtitles"

const (
	defaultBase = "https://api.opensubtitles.com/api/v1"
	userAgent   = "Lain v0.1"
	maxBody     = 2 << 20
	maxResults  = 200
	tokenTTL    = 12 * time.Hour
)

// OpenSubtitles is the provider. Its only state is a login-token cache
// per account, so a download does not log in every time.
type OpenSubtitles struct {
	Client *http.Client

	mu     sync.Mutex
	tokens map[string]cachedToken
}

type cachedToken struct {
	token string
	until time.Time
}

// NewOpenSubtitles returns the provider with a bounded HTTP client.
func NewOpenSubtitles() *OpenSubtitles {
	return &OpenSubtitles{Client: &http.Client{Timeout: 30 * time.Second}, tokens: map[string]cachedToken{}}
}

func (*OpenSubtitles) ID() string             { return OpenSubtitlesID }
func (*OpenSubtitles) Capabilities() []string { return []string{contracts.CapSubtitle} }
func (*OpenSubtitles) Health() error          { return nil }

func (o *OpenSubtitles) Invoke(cap string, input any) (any, error) {
	if cap != contracts.CapSubtitle {
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
	switch in := input.(type) {
	case contracts.SubtitleSearchInput:
		base, err := check(in.Provider)
		if err != nil {
			return nil, err
		}
		return o.search(base, in)
	case contracts.SubtitleDownloadInput:
		base, err := check(in.Provider)
		if err != nil {
			return nil, err
		}
		return o.download(base, in)
	default:
		return nil, &core.Error{Code: "invalid-message", Msg: "SubtitleSearchInput or SubtitleDownloadInput required"}
	}
}

func check(p contracts.SubtitleProvider) (*url.URL, error) {
	if p.Kind != contracts.SubtitleOpenSubtitles {
		return nil, &core.Error{Code: "unsupported-provider", Msg: "not an OpenSubtitles account"}
	}
	raw := p.BaseURL
	if raw == "" {
		raw = defaultBase
	}
	u, err := url.Parse(raw)
	if err != nil || (u.Scheme != "http" && u.Scheme != "https") || u.Host == "" || u.User != nil {
		return nil, &core.Error{Code: "invalid-message", Msg: "provider url must be http(s)"}
	}
	if strings.TrimSpace(p.APIKey) == "" {
		return nil, &core.Error{Code: "invalid-message", Msg: "an OpenSubtitles API key is required"}
	}
	return u, nil
}

// fail names the host only: URLs and bodies may echo credentials.
func fail(base *url.URL, code, msg string) error {
	return &core.Error{Code: code, Msg: base.Host + ": " + msg}
}

func (o *OpenSubtitles) do(ctx context.Context, base *url.URL, p contracts.SubtitleProvider, method, path string, query url.Values, body any, bearer string, out any) error {
	u := *base
	u.Path = strings.TrimSuffix(u.Path, "/") + path
	if query != nil {
		u.RawQuery = encodeSorted(query)
	}
	var rdr io.Reader
	if body != nil {
		raw, _ := json.Marshal(body)
		rdr = bytes.NewReader(raw)
	}
	req, err := http.NewRequestWithContext(ctx, method, u.String(), rdr)
	if err != nil {
		return fail(base, "invalid-message", "bad request")
	}
	req.Header.Set("Api-Key", p.APIKey)
	req.Header.Set("User-Agent", userAgent)
	req.Header.Set("Accept", "application/json")
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	if bearer != "" {
		req.Header.Set("Authorization", "Bearer "+bearer)
	}
	res, err := o.Client.Do(req)
	if err != nil {
		return fail(base, "dependency-unavailable", "unreachable")
	}
	defer res.Body.Close()
	raw, err := io.ReadAll(io.LimitReader(res.Body, maxBody+1))
	if err != nil || len(raw) > maxBody {
		return fail(base, "dependency-unavailable", "unreadable response")
	}
	switch {
	case res.StatusCode == http.StatusUnauthorized || res.StatusCode == http.StatusForbidden:
		return fail(base, "auth-failed", "credentials rejected")
	case res.StatusCode == http.StatusTooManyRequests:
		return fail(base, "rate-limited", "too many requests or daily download quota used")
	case res.StatusCode == http.StatusNotFound:
		return fail(base, "not-found", "no such subtitle")
	case res.StatusCode >= 300:
		return fail(base, "dependency-unavailable", "http "+strconv.Itoa(res.StatusCode))
	}
	if err := json.Unmarshal(raw, out); err != nil {
		return fail(base, "dependency-unavailable", "response is not valid JSON")
	}
	return nil
}

// encodeSorted writes lowercase, alphabetically sorted parameters, as
// the API asks (it redirects otherwise).
func encodeSorted(q url.Values) string {
	keys := make([]string, 0, len(q))
	for k := range q {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	var parts []string
	for _, k := range keys {
		parts = append(parts, url.QueryEscape(strings.ToLower(k))+"="+url.QueryEscape(strings.ToLower(q.Get(k))))
	}
	return strings.Join(parts, "&")
}

// apiLanguages turns ISO 639-2 codes into the API's tags, asking for
// both regional variants where the API splits a language.
func apiLanguages(langs []string) string {
	var out []string
	seen := map[string]bool{}
	add := func(t string) {
		if !seen[t] {
			seen[t] = true
			out = append(out, t)
		}
	}
	for _, l := range langs {
		switch l {
		case "por":
			add("pt-br")
			add("pt-pt")
		case "zho":
			add("zh-cn")
			add("zh-tw")
		default:
			if t := subtitle.Tag(l, ""); t != "" {
				add(strings.ToLower(t))
			}
		}
	}
	sort.Strings(out)
	return strings.Join(out, ",")
}

type searchResponse struct {
	Data []struct {
		Attributes struct {
			Language        string `json:"language"`
			DownloadCount   int    `json:"download_count"`
			HearingImpaired bool   `json:"hearing_impaired"`
			ForeignOnly     bool   `json:"foreign_parts_only"`
			Release         string `json:"release"`
			HashMatch       bool   `json:"moviehash_match"`
			Feature         struct {
				Season  int `json:"season_number"`
				Episode int `json:"episode_number"`
			} `json:"feature_details"`
			Files []struct {
				FileID   int64  `json:"file_id"`
				FileName string `json:"file_name"`
			} `json:"files"`
		} `json:"attributes"`
	} `json:"data"`
}

func (o *OpenSubtitles) search(base *url.URL, in contracts.SubtitleSearchInput) (contracts.SubtitleSearchOutput, error) {
	q := url.Values{}
	if in.Hash != "" {
		if len(in.Hash) != 16 {
			return contracts.SubtitleSearchOutput{}, &core.Error{Code: "invalid-message", Msg: "hash must be 16 hex digits"}
		}
		q.Set("moviehash", in.Hash)
	}
	if s := strings.TrimSpace(in.Query); s != "" {
		q.Set("query", clip(s, 200))
	}
	if q.Get("moviehash") == "" && q.Get("query") == "" {
		return contracts.SubtitleSearchOutput{}, &core.Error{Code: "invalid-message", Msg: "a hash or a query is required"}
	}
	if langs := apiLanguages(in.Languages); langs != "" {
		q.Set("languages", langs)
	}
	if in.Movie {
		q.Set("type", "movie")
		if in.Year > 0 {
			q.Set("year", strconv.Itoa(in.Year))
		}
	} else if in.Episode > 0 {
		q.Set("type", "episode")
		q.Set("episode_number", strconv.Itoa(in.Episode))
		if in.Season > 0 {
			q.Set("season_number", strconv.Itoa(in.Season))
		}
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	var res searchResponse
	if err := o.do(ctx, base, in.Provider, http.MethodGet, "/subtitles", q, nil, "", &res); err != nil {
		return contracts.SubtitleSearchOutput{}, err
	}
	out := contracts.SubtitleSearchOutput{Candidates: []contracts.SubtitleCandidate{}}
	for _, d := range res.Data {
		a := d.Attributes
		lang := subtitle.Language(a.Language)
		if lang == "" || len(a.Files) == 0 || len(out.Candidates) >= maxResults {
			continue
		}
		c := contracts.SubtitleCandidate{
			ProviderID: in.Provider.ID, FileID: strconv.FormatInt(a.Files[0].FileID, 10), Language: lang,
			Release: clip(a.Release, 300), FileName: clip(a.Files[0].FileName, 300), HashMatch: a.HashMatch,
			HI: a.HearingImpaired, Forced: a.ForeignOnly, Downloads: a.DownloadCount,
			Season: a.Feature.Season, Episode: a.Feature.Episode,
		}
		if strings.Contains(a.Language, "-") {
			c.Region = clip(a.Language, 10)
		}
		out.Candidates = append(out.Candidates, c)
	}
	return out, nil
}

// token logs in once per account and caches the token.
func (o *OpenSubtitles) token(ctx context.Context, base *url.URL, p contracts.SubtitleProvider) (string, error) {
	if p.Username == "" {
		return "", nil // anonymous: the API key's own (smaller) quota
	}
	key := base.String() + "\x00" + p.Username
	o.mu.Lock()
	if t, ok := o.tokens[key]; ok && time.Now().Before(t.until) {
		o.mu.Unlock()
		return t.token, nil
	}
	o.mu.Unlock()
	var res struct {
		Token string `json:"token"`
	}
	if err := o.do(ctx, base, p, http.MethodPost, "/login", nil, map[string]string{"username": p.Username, "password": p.Password}, "", &res); err != nil {
		return "", err
	}
	if res.Token == "" {
		return "", fail(base, "auth-failed", "login returned no token")
	}
	o.mu.Lock()
	o.tokens[key] = cachedToken{token: res.Token, until: time.Now().Add(tokenTTL)}
	o.mu.Unlock()
	return res.Token, nil
}

func (o *OpenSubtitles) download(base *url.URL, in contracts.SubtitleDownloadInput) (contracts.SubtitleDownloadOutput, error) {
	id, err := strconv.ParseInt(in.FileID, 10, 64)
	if err != nil || id <= 0 {
		return contracts.SubtitleDownloadOutput{}, &core.Error{Code: "invalid-message", Msg: "file_id must be a positive number"}
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	tok, err := o.token(ctx, base, in.Provider)
	if err != nil {
		return contracts.SubtitleDownloadOutput{}, err
	}
	var res struct {
		Link      string `json:"link"`
		FileName  string `json:"file_name"`
		Remaining *int   `json:"remaining"`
	}
	if err := o.do(ctx, base, in.Provider, http.MethodPost, "/download", nil, map[string]int64{"file_id": id}, tok, &res); err != nil {
		return contracts.SubtitleDownloadOutput{}, err
	}
	u, err := url.Parse(res.Link)
	if err != nil || (u.Scheme != "http" && u.Scheme != "https") {
		return contracts.SubtitleDownloadOutput{}, fail(base, "dependency-unavailable", "no usable download link")
	}
	out := contracts.SubtitleDownloadOutput{Link: res.Link, FileName: clip(res.FileName, 300), Remaining: -1}
	if res.Remaining != nil {
		out.Remaining = *res.Remaining
	}
	return out, nil
}

func clip(s string, n int) string {
	if len(s) > n {
		return s[:n]
	}
	return s
}
