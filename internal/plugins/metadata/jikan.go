package metadata

import (
	"encoding/json"
	"net/url"
	"strconv"
	"strings"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

// Jikan serves MyAnimeList data over free REST (3 req/s, no key).
// Retries on Retry-After are handled by the polite client.
type Jikan struct {
	BaseURL string
	http    *politeClient
}

func NewJikan() *Jikan {
	return &Jikan{BaseURL: "https://api.jikan.moe", http: newPoliteClient(400 * time.Millisecond)}
}

func (j *Jikan) ID() string { return "lain-metadata-jikan" }
func (j *Jikan) Capabilities() []string {
	return []string{contracts.CapMetadataSearch, contracts.CapMetadataResolve}
}
func (j *Jikan) Health() error { return nil }

func (j *Jikan) Invoke(cap string, input any) (any, error) {
	switch cap {
	case contracts.CapMetadataSearch:
		in, ok := input.(contracts.MetadataSearchInput)
		if !ok {
			return nil, &core.Error{Code: "invalid-message", Msg: "MetadataSearchInput required"}
		}
		if in.Kind != "" && in.Kind != "anime" && in.Kind != "episode" && in.Kind != "video" {
			return []contracts.MetadataCandidate{}, nil
		}
		return j.search(in)
	case contracts.CapMetadataResolve:
		in, ok := input.(contracts.MetadataResolveInput)
		if !ok {
			return nil, &core.Error{Code: "invalid-message", Msg: "MetadataResolveInput required"}
		}
		return j.resolve(in.RemoteID)
	default:
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
}

type jikanEntry struct {
	MalID    int    `json:"mal_id"`
	Title    string `json:"title"`
	TitleEng string `json:"title_english"`
	TitleJp  string `json:"title_japanese"`
	Synopsis string `json:"synopsis"`
	Episodes int    `json:"episodes"`
	Aired    struct {
		From string `json:"from"`
	} `json:"aired"`
	Images struct {
		Jpg struct {
			ImageURL      string `json:"image_url"`
			LargeImageURL string `json:"large_image_url"`
		} `json:"jpg"`
	} `json:"images"`
	Genres []struct {
		Name string `json:"name"`
	} `json:"genres"`
}

func (j *Jikan) search(in contracts.MetadataSearchInput) ([]contracts.MetadataCandidate, error) {
	limit := in.Limit
	if limit <= 0 || limit > 10 {
		limit = 5
	}
	u := j.BaseURL + "/v4/anime?q=" + url.QueryEscape(in.Query) + "&limit=" + strconv.Itoa(limit) + "&order_by=members&sort=desc&sfw=true"
	raw, err := j.http.get(u)
	if err != nil {
		return nil, err
	}
	var doc struct {
		Data []jikanEntry `json:"data"`
	}
	if err := json.Unmarshal(raw, &doc); err != nil {
		return nil, err
	}
	var out []contracts.MetadataCandidate
	for _, e := range doc.Data {
		out = append(out, e.candidate(j.ID()))
	}
	if out == nil {
		out = []contracts.MetadataCandidate{}
	}
	return out, nil
}

func (j *Jikan) resolve(id string) (contracts.MetadataRecord, error) {
	if _, err := strconv.Atoi(id); err != nil {
		return contracts.MetadataRecord{}, &core.Error{Code: "invalid-message", Msg: "numeric id required"}
	}
	raw, err := j.http.get(j.BaseURL + "/v4/anime/" + url.PathEscape(id))
	if err != nil {
		return contracts.MetadataRecord{}, err
	}
	var doc struct {
		Data jikanEntry `json:"data"`
	}
	if err := json.Unmarshal(raw, &doc); err != nil {
		return contracts.MetadataRecord{}, err
	}
	e := doc.Data
	var genres []string
	for _, g := range e.Genres {
		genres = append(genres, g.Name)
	}
	return contracts.MetadataRecord{
		Provider: j.ID(), RemoteID: strconv.Itoa(e.MalID), Title: e.bestTitle(),
		Synonyms: e.synonyms(), Year: yearOf(e.Aired.From), Genres: genres,
		Synopsis: strings.TrimSpace(e.Synopsis), Episodes: e.Episodes,
		Poster: firstOf(e.Images.Jpg.LargeImageURL, e.Images.Jpg.ImageURL),
	}, nil
}

func (e jikanEntry) bestTitle() string {
	if e.TitleEng != "" {
		return e.TitleEng
	}
	return e.Title
}

func (e jikanEntry) synonyms() []string {
	var out []string
	for _, t := range []string{e.Title, e.TitleJp} {
		if t = strings.TrimSpace(t); t != "" && t != e.bestTitle() {
			out = append(out, t)
		}
	}
	return out
}

func (e jikanEntry) candidate(provider string) contracts.MetadataCandidate {
	return contracts.MetadataCandidate{
		Provider: provider, RemoteID: strconv.Itoa(e.MalID), Title: e.bestTitle(),
		Synonyms: e.synonyms(), Year: yearOf(e.Aired.From), Kind: "anime",
		Poster: firstOf(e.Images.Jpg.LargeImageURL, e.Images.Jpg.ImageURL),
	}
}

func yearOf(iso string) int {
	if len(iso) >= 4 {
		if y, err := strconv.Atoi(iso[:4]); err == nil && y >= 1900 && y <= 2099 {
			return y
		}
	}
	return 0
}
