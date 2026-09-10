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

// Kitsu serves anime metadata from the Kitsu JSON:API (no key for
// public reads). BaseURL is overridable for tests.
type Kitsu struct {
	BaseURL string
	http    *politeClient
}

func NewKitsu() *Kitsu {
	return &Kitsu{BaseURL: "https://kitsu.io", http: newPoliteClient(1 * time.Second)}
}

func (k *Kitsu) ID() string { return "lain-metadata-kitsu" }
func (k *Kitsu) Capabilities() []string {
	return []string{contracts.CapMetadataSearch, contracts.CapMetadataResolve}
}
func (k *Kitsu) Health() error { return nil }

func (k *Kitsu) Invoke(cap string, input any) (any, error) {
	switch cap {
	case contracts.CapMetadataSearch:
		in, ok := input.(contracts.MetadataSearchInput)
		if !ok {
			return nil, &core.Error{Code: "invalid-message", Msg: "MetadataSearchInput required"}
		}
		if in.Kind != "" && in.Kind != "anime" && in.Kind != "episode" && in.Kind != "video" {
			return []contracts.MetadataCandidate{}, nil
		}
		return k.search(in)
	case contracts.CapMetadataResolve:
		in, ok := input.(contracts.MetadataResolveInput)
		if !ok {
			return nil, &core.Error{Code: "invalid-message", Msg: "MetadataResolveInput required"}
		}
		return k.resolve(in.RemoteID)
	default:
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
}

type kitsuDoc struct {
	Data []struct {
		ID         string `json:"id"`
		Attributes struct {
			CanonicalTitle string            `json:"canonicalTitle"`
			Titles         map[string]string `json:"titles"`
			Synopsis       string            `json:"synopsis"`
			StartDate      string            `json:"startDate"`
			EpisodeCount   int               `json:"episodeCount"`
			PosterImage    map[string]string `json:"posterImage"`
			CoverImage     map[string]string `json:"coverImage"`
		} `json:"attributes"`
	} `json:"data"`
}

func (k *Kitsu) search(in contracts.MetadataSearchInput) ([]contracts.MetadataCandidate, error) {
	limit := in.Limit
	if limit <= 0 || limit > 10 {
		limit = 5
	}
	u := k.BaseURL + "/api/edge/anime?filter[text]=" + url.QueryEscape(in.Query) + "&page[limit]=" + strconv.Itoa(limit)
	raw, err := k.http.get(u)
	if err != nil {
		return nil, err
	}
	var doc kitsuDoc
	if err := json.Unmarshal(raw, &doc); err != nil {
		return nil, err
	}
	var out []contracts.MetadataCandidate
	for _, d := range doc.Data {
		a := d.Attributes
		title := a.CanonicalTitle
		if title == "" {
			title = a.Titles["en"]
		}
		if title == "" {
			title = a.Titles["en_jp"]
		}
		year := 0
		if len(a.StartDate) >= 4 {
			if y, err := strconv.Atoi(a.StartDate[:4]); err == nil {
				year = y
			}
		}
		out = append(out, contracts.MetadataCandidate{
			Provider: k.ID(), RemoteID: d.ID, Title: title,
			Synonyms: titleMap(a.Titles), Year: year, Kind: "anime",
			Poster: firstImage(a.PosterImage),
		})
	}
	if out == nil {
		out = []contracts.MetadataCandidate{}
	}
	return out, nil
}

func (k *Kitsu) resolve(id string) (contracts.MetadataRecord, error) {
	raw, err := k.http.get(k.BaseURL + "/api/edge/anime/" + url.PathEscape(id))
	if err != nil {
		return contracts.MetadataRecord{}, err
	}
	var doc struct {
		Data struct {
			ID         string `json:"id"`
			Attributes struct {
				CanonicalTitle string            `json:"canonicalTitle"`
				Titles         map[string]string `json:"titles"`
				Synopsis       string            `json:"synopsis"`
				StartDate      string            `json:"startDate"`
				EpisodeCount   int               `json:"episodeCount"`
				PosterImage    map[string]string `json:"posterImage"`
				CoverImage     map[string]string `json:"coverImage"`
			} `json:"attributes"`
		} `json:"data"`
	}
	if err := json.Unmarshal(raw, &doc); err != nil {
		return contracts.MetadataRecord{}, err
	}
	a := doc.Data.Attributes
	title := a.CanonicalTitle
	if title == "" {
		title = a.Titles["en"]
	}
	year := 0
	if len(a.StartDate) >= 4 {
		if y, err := strconv.Atoi(a.StartDate[:4]); err == nil {
			year = y
		}
	}
	return contracts.MetadataRecord{
		Provider: k.ID(), RemoteID: doc.Data.ID, Title: title,
		Synonyms: titleMap(a.Titles), Year: year,
		Synopsis: strings.TrimSpace(a.Synopsis), Episodes: a.EpisodeCount,
		Poster: firstImage(a.PosterImage), Cover: firstImage(a.CoverImage),
	}, nil
}

func titleMap(m map[string]string) []string {
	var out []string
	for _, v := range m {
		if v = strings.TrimSpace(v); v != "" {
			out = append(out, v)
		}
	}
	return out
}

func firstImage(m map[string]string) string {
	for _, k := range []string{"original", "large", "medium", "small", "tiny"} {
		if v := m[k]; v != "" {
			return v
		}
	}
	for _, v := range m {
		if v != "" {
			return v
		}
	}
	return ""
}
