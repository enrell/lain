// Package indexer serves lain.indexer@1 (docs/slices/acquisition.md,
// A-7). The Torznab provider speaks the Newznab API family, so direct
// Torznab/Newznab indexers and aggregators such as Jackett and Prowlarr
// all work through it. It holds no indexer state: definitions,
// credentials and rate limits come from the caller.
package indexer

import (
	"context"
	"encoding/xml"
	"io"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

// TorznabID is the provider id.
const TorznabID = "lain-indexer-torznab"

const (
	maxResponse = 8 << 20
	maxResults  = 1000
)

// Torznab is the Newznab-family client.
type Torznab struct {
	Client *http.Client
}

// NewTorznab returns a provider with a bounded HTTP client.
func NewTorznab() *Torznab {
	return &Torznab{Client: &http.Client{Timeout: 30 * time.Second}}
}

func (*Torznab) ID() string             { return TorznabID }
func (*Torznab) Capabilities() []string { return []string{contracts.CapIndexer} }
func (*Torznab) Health() error          { return nil }

func (t *Torznab) Invoke(cap string, input any) (any, error) {
	if cap != contracts.CapIndexer {
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
	switch in := input.(type) {
	case contracts.IndexerCapsInput:
		if err := check(in.Indexer); err != nil {
			return nil, err
		}
		return t.caps(in.Indexer)
	case contracts.IndexerSearchInput:
		if err := check(in.Indexer); err != nil {
			return nil, err
		}
		return t.search(in)
	default:
		return nil, &core.Error{Code: "invalid-message", Msg: "IndexerCapsInput or IndexerSearchInput required"}
	}
}

func check(ix contracts.Indexer) error {
	if ix.Protocol != contracts.IndexerTorznab && ix.Protocol != contracts.IndexerNewznab {
		return &core.Error{Code: "unsupported-protocol", Msg: "not a torznab/newznab indexer"}
	}
	u, err := url.Parse(ix.URL)
	if err != nil || (u.Scheme != "http" && u.Scheme != "https") || u.Host == "" {
		return &core.Error{Code: "invalid-message", Msg: "indexer url must be http(s)"}
	}
	return nil
}

// endpoint builds {url}/api?…; a URL already ending in /api is kept.
// Jackett/Prowlarr per-indexer URLs end in ".../api" or "/" — both work.
func endpoint(ix contracts.Indexer, q url.Values) string {
	u, _ := url.Parse(ix.URL)
	if !strings.HasSuffix(u.Path, "/api") {
		u.Path = strings.TrimSuffix(u.Path, "/") + "/api"
	}
	if ix.APIKey != "" {
		q.Set("apikey", ix.APIKey)
	}
	u.RawQuery = q.Encode()
	return u.String()
}

// fail builds an error that names the host only: request URLs carry the
// API key and must never reach a message or a log line.
func fail(ix contracts.Indexer, code, msg string) error {
	host := ix.URL
	if u, err := url.Parse(ix.URL); err == nil {
		host = u.Host
	}
	return &core.Error{Code: code, Msg: host + ": " + msg}
}

func (t *Torznab) get(ctx context.Context, ix contracts.Indexer, q url.Values) ([]byte, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, endpoint(ix, q), nil)
	if err != nil {
		return nil, fail(ix, "invalid-message", "bad request")
	}
	req.Header.Set("User-Agent", "Lain")
	res, err := t.Client.Do(req)
	if err != nil {
		return nil, fail(ix, "dependency-unavailable", "unreachable")
	}
	defer res.Body.Close()
	body, err := io.ReadAll(io.LimitReader(res.Body, maxResponse+1))
	if err != nil {
		return nil, fail(ix, "dependency-unavailable", "read failed")
	}
	if len(body) > maxResponse {
		return nil, fail(ix, "dependency-unavailable", "response too large")
	}
	if e := apiError(body); e != nil {
		code := "indexer-error"
		if e.Code == "100" || e.Code == "101" || e.Code == "102" {
			code = "auth-failed"
		}
		return nil, fail(ix, code, sanitize(e.Description, ix.APIKey))
	}
	switch {
	case res.StatusCode == http.StatusUnauthorized || res.StatusCode == http.StatusForbidden:
		return nil, fail(ix, "auth-failed", "credentials rejected")
	case res.StatusCode == http.StatusTooManyRequests:
		return nil, fail(ix, "rate-limited", "too many requests")
	case res.StatusCode != http.StatusOK:
		return nil, fail(ix, "dependency-unavailable", "http "+strconv.Itoa(res.StatusCode))
	}
	return body, nil
}

func sanitize(s, secret string) string {
	if secret != "" {
		s = strings.ReplaceAll(s, secret, "***")
	}
	if len(s) > 200 {
		s = s[:200]
	}
	return s
}

type xmlError struct {
	XMLName     xml.Name `xml:"error"`
	Code        string   `xml:"code,attr"`
	Description string   `xml:"description,attr"`
}

func apiError(body []byte) *xmlError {
	trim := strings.TrimSpace(string(body[:min(len(body), 512)]))
	if !strings.Contains(trim, "<error") {
		return nil
	}
	var e xmlError
	if err := xml.Unmarshal(body, &e); err != nil || e.XMLName.Local != "error" {
		return nil
	}
	return &e
}

type xmlCaps struct {
	Limits struct {
		Max     int `xml:"max,attr"`
		Default int `xml:"default,attr"`
	} `xml:"limits"`
	Searching struct {
		Search      xmlAvail `xml:"search"`
		TVSearch    xmlAvail `xml:"tv-search"`
		MovieSearch xmlAvail `xml:"movie-search"`
		BookSearch  xmlAvail `xml:"book-search"`
	} `xml:"searching"`
	Categories []xmlCategory `xml:"categories>category"`
}

type xmlAvail struct {
	Available string `xml:"available,attr"`
}

func (a xmlAvail) yes() bool { return strings.EqualFold(a.Available, "yes") }

type xmlCategory struct {
	ID     int           `xml:"id,attr"`
	Name   string        `xml:"name,attr"`
	Subcat []xmlCategory `xml:"subcat"`
}

func (t *Torznab) caps(ix contracts.Indexer) (contracts.IndexerCaps, error) {
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	body, err := t.get(ctx, ix, url.Values{"t": {"caps"}})
	if err != nil {
		return contracts.IndexerCaps{}, err
	}
	var x xmlCaps
	if err := xml.Unmarshal(body, &x); err != nil {
		return contracts.IndexerCaps{}, fail(ix, "indexer-error", "caps response is not valid XML")
	}
	out := contracts.IndexerCaps{
		Search: x.Searching.Search.yes(), TVSearch: x.Searching.TVSearch.yes(),
		MovieSearch: x.Searching.MovieSearch.yes(), BookSearch: x.Searching.BookSearch.yes(),
		Limit: x.Limits.Max, Categories: []contracts.IndexerCategory{},
	}
	for _, c := range x.Categories {
		out.Categories = append(out.Categories, contracts.IndexerCategory{ID: c.ID, Name: clip(c.Name, 100)})
		for _, s := range c.Subcat {
			out.Categories = append(out.Categories, contracts.IndexerCategory{ID: s.ID, Name: clip(c.Name+"/"+s.Name, 100)})
		}
		if len(out.Categories) > 2000 {
			break
		}
	}
	return out, nil
}

type xmlRSS struct {
	Items []xmlItem `xml:"channel>item"`
}

type xmlItem struct {
	Title     string `xml:"title"`
	GUID      string `xml:"guid"`
	Link      string `xml:"link"`
	Comments  string `xml:"comments"`
	PubDate   string `xml:"pubDate"`
	Size      int64  `xml:"size"`
	Enclosure struct {
		URL    string `xml:"url,attr"`
		Length int64  `xml:"length,attr"`
		Type   string `xml:"type,attr"`
	} `xml:"enclosure"`
	Attrs []xmlAttr `xml:"attr"`
}

// xmlAttr matches torznab:attr and newznab:attr (namespace-agnostic).
type xmlAttr struct {
	Name  string `xml:"name,attr"`
	Value string `xml:"value,attr"`
}

func (t *Torznab) search(in contracts.IndexerSearchInput) (contracts.IndexerSearchOutput, error) {
	ix := in.Indexer
	q := url.Values{}
	fn := "search"
	switch in.Kind {
	case contracts.SearchTV:
		fn = "tvsearch"
		if in.Season > 0 {
			q.Set("season", strconv.Itoa(in.Season))
		}
		if in.Episode > 0 {
			q.Set("ep", strconv.Itoa(in.Episode))
		}
	case contracts.SearchMovie:
		fn = "movie"
		if in.Year > 0 {
			q.Set("year", strconv.Itoa(in.Year))
		}
	case contracts.SearchBook:
		fn = "book"
	}
	if ix.Caps != nil {
		// Fall back to a plain search when the indexer lacks the function.
		if (fn == "tvsearch" && !ix.Caps.TVSearch) || (fn == "movie" && !ix.Caps.MovieSearch) || (fn == "book" && !ix.Caps.BookSearch) {
			fn = "search"
			q = url.Values{}
		}
	}
	q.Set("t", fn)
	if s := strings.TrimSpace(in.Query); s != "" {
		q.Set("q", clip(s, 200))
	}
	cats := in.Categories
	if len(cats) == 0 {
		cats = ix.Categories
	}
	if len(cats) > 0 {
		parts := make([]string, len(cats))
		for i, c := range cats {
			parts[i] = strconv.Itoa(c)
		}
		q.Set("cat", strings.Join(parts, ","))
	}
	if in.Limit > 0 {
		q.Set("limit", strconv.Itoa(min(in.Limit, maxResults)))
	}
	q.Set("extended", "1")
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	body, err := t.get(ctx, ix, q)
	if err != nil {
		return contracts.IndexerSearchOutput{}, err
	}
	results, err := parseRSS(body, ix)
	if err != nil {
		return contracts.IndexerSearchOutput{}, fail(ix, "indexer-error", "results are not valid RSS")
	}
	return contracts.IndexerSearchOutput{Results: results}, nil
}

// parseRSS reads a Torznab/Newznab result feed.
func parseRSS(body []byte, ix contracts.Indexer) ([]contracts.SearchResult, error) {
	var rss xmlRSS
	if err := xml.Unmarshal(body, &rss); err != nil {
		return nil, err
	}
	out := []contracts.SearchResult{}
	for _, it := range rss.Items {
		if len(out) >= maxResults {
			break
		}
		r := contracts.SearchResult{
			IndexerID: ix.ID, Title: clip(strings.TrimSpace(it.Title), 300), GUID: clip(it.GUID, 500),
			CommentsURL: safeURL(it.Comments), Size: it.Size,
		}
		if r.Title == "" {
			continue
		}
		r.Protocol = contracts.ProtocolTorrent
		if ix.Protocol == contracts.IndexerNewznab || strings.Contains(it.Enclosure.Type, "nzb") {
			r.Protocol = contracts.ProtocolUsenet
		}
		link := it.Enclosure.URL
		if link == "" {
			link = it.Link
		}
		if strings.HasPrefix(link, "magnet:") {
			r.Magnet = clip(link, 4096)
		} else {
			r.Link = safeURL(link)
		}
		if r.Size == 0 {
			r.Size = it.Enclosure.Length
		}
		if ts, err := time.Parse(time.RFC1123Z, strings.TrimSpace(it.PubDate)); err == nil {
			r.PublishedAt = ts.Unix()
		} else if ts, err := time.Parse(time.RFC1123, strings.TrimSpace(it.PubDate)); err == nil {
			r.PublishedAt = ts.Unix()
		}
		for _, a := range it.Attrs {
			v := strings.TrimSpace(a.Value)
			switch strings.ToLower(a.Name) {
			case "seeders":
				r.Seeders, _ = strconv.Atoi(v)
			case "peers":
				r.Peers, _ = strconv.Atoi(v)
			case "size":
				if n, err := strconv.ParseInt(v, 10, 64); err == nil && r.Size == 0 {
					r.Size = n
				}
			case "infohash":
				if len(v) == 40 {
					r.InfoHash = strings.ToLower(v)
				}
			case "magneturl":
				if strings.HasPrefix(v, "magnet:") {
					r.Magnet = clip(v, 4096)
				}
			case "category":
				if n, err := strconv.Atoi(v); err == nil && len(r.Categories) < 20 {
					r.Categories = append(r.Categories, n)
				}
			case "downloadvolumefactor":
				r.Freeleech = v == "0"
			}
		}
		if r.Size < 0 {
			r.Size = 0
		}
		if r.Link == "" && r.Magnet == "" {
			continue
		}
		out = append(out, r)
	}
	return out, nil
}

func safeURL(s string) string {
	s = strings.TrimSpace(s)
	u, err := url.Parse(s)
	if err != nil || (u.Scheme != "http" && u.Scheme != "https") || len(s) > 4096 {
		return ""
	}
	return s
}

func clip(s string, n int) string {
	if len(s) > n {
		return s[:n]
	}
	return s
}
