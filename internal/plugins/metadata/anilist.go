package metadata

import (
	"encoding/json"
	"fmt"
	"strings"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

// AniList serves anime metadata over GraphQL (public reads need no
// key; the service rate-limits aggressively, hence the polite gap).
type AniList struct {
	BaseURL string
	http    *politeClient
}

func NewAniList() *AniList {
	return &AniList{BaseURL: "https://graphql.anilist.co", http: newPoliteClient(700 * time.Millisecond)}
}

func (a *AniList) ID() string { return "lain-metadata-anilist" }
func (a *AniList) Capabilities() []string {
	return []string{contracts.CapMetadataSearch, contracts.CapMetadataResolve}
}
func (a *AniList) Health() error { return nil }

func (a *AniList) Invoke(cap string, input any) (any, error) {
	switch cap {
	case contracts.CapMetadataSearch:
		in, ok := input.(contracts.MetadataSearchInput)
		if !ok {
			return nil, &core.Error{Code: "invalid-message", Msg: "MetadataSearchInput required"}
		}
		if in.Kind != "" && in.Kind != "anime" && in.Kind != "episode" && in.Kind != "video" {
			return []contracts.MetadataCandidate{}, nil
		}
		return a.search(in)
	case contracts.CapMetadataResolve:
		in, ok := input.(contracts.MetadataResolveInput)
		if !ok {
			return nil, &core.Error{Code: "invalid-message", Msg: "MetadataResolveInput required"}
		}
		return a.resolve(in.RemoteID)
	default:
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
}

const anilistFields = `id title { romaji english native } description coverImage { large extraLarge } bannerImage episodes startDate { year } genres synonyms`

func (a *AniList) search(in contracts.MetadataSearchInput) ([]contracts.MetadataCandidate, error) {
	limit := in.Limit
	if limit <= 0 || limit > 10 {
		limit = 5
	}
	body, _ := json.Marshal(map[string]any{
		"query":     fmt.Sprintf(`query ($search: String, $perPage: Int) { Page(page: 1, perPage: $perPage) { media(search: $search, type: ANIME) { %s } } }`, anilistFields),
		"variables": map[string]any{"search": in.Query, "perPage": limit},
	})
	raw, err := a.http.postJSON(a.BaseURL, body)
	if err != nil {
		return nil, err
	}
	var doc struct {
		Data struct {
			Page struct {
				Media []anilistMedia `json:"media"`
			} `json:"page"`
		} `json:"data"`
	}
	if err := json.Unmarshal(raw, &doc); err != nil {
		return nil, err
	}
	var out []contracts.MetadataCandidate
	for _, m := range doc.Data.Page.Media {
		out = append(out, contracts.MetadataCandidate{
			Provider: a.ID(), RemoteID: fmt.Sprint(m.ID), Title: m.bestTitle(),
			Synonyms: m.Synonyms, Year: m.StartDate.Year, Kind: "anime",
			Poster: firstOf(m.CoverImage.ExtraLarge, m.CoverImage.Large),
		})
	}
	if out == nil {
		out = []contracts.MetadataCandidate{}
	}
	return out, nil
}

func (a *AniList) resolve(id string) (contracts.MetadataRecord, error) {
	var aid int
	if _, err := fmt.Sscanf(id, "%d", &aid); err != nil {
		return contracts.MetadataRecord{}, &core.Error{Code: "invalid-message", Msg: "numeric id required"}
	}
	body, _ := json.Marshal(map[string]any{
		"query":     fmt.Sprintf(`query ($id: Int) { Media(id: $id) { %s } }`, anilistFields),
		"variables": map[string]any{"id": aid},
	})
	raw, err := a.http.postJSON(a.BaseURL, body)
	if err != nil {
		return contracts.MetadataRecord{}, err
	}
	var doc struct {
		Data struct {
			Media anilistMedia `json:"media"`
		} `json:"data"`
	}
	if err := json.Unmarshal(raw, &doc); err != nil {
		return contracts.MetadataRecord{}, err
	}
	m := doc.Data.Media
	return contracts.MetadataRecord{
		Provider: a.ID(), RemoteID: fmt.Sprint(m.ID), Title: m.bestTitle(),
		Synonyms: m.Synonyms, Year: m.StartDate.Year, Genres: m.Genres,
		Synopsis: stripHTML(m.Description), Episodes: m.Episodes,
		Poster: firstOf(m.CoverImage.ExtraLarge, m.CoverImage.Large), Cover: m.BannerImage,
	}, nil
}

type anilistMedia struct {
	ID    int `json:"id"`
	Title struct {
		Romaji  string `json:"romaji"`
		English string `json:"english"`
		Native  string `json:"native"`
	} `json:"title"`
	Description string `json:"description"`
	CoverImage  struct {
		Large      string `json:"large"`
		ExtraLarge string `json:"extraLarge"`
	} `json:"coverImage"`
	BannerImage string `json:"bannerImage"`
	Episodes    int    `json:"episodes"`
	StartDate   struct {
		Year int `json:"year"`
	} `json:"startDate"`
	Genres   []string `json:"genres"`
	Synonyms []string `json:"synonyms"`
}

func (m anilistMedia) bestTitle() string {
	if m.Title.English != "" {
		return m.Title.English
	}
	return m.Title.Romaji
}

func firstOf(v ...string) string {
	for _, s := range v {
		if s = strings.TrimSpace(s); s != "" {
			return s
		}
	}
	return ""
}

// stripHTML drops the <br>/<i> markup AniList descriptions carry.
func stripHTML(s string) string {
	var b strings.Builder
	inTag := false
	for _, r := range s {
		switch {
		case r == '<':
			inTag = true
		case r == '>':
			inTag = false
		case !inTag:
			b.WriteRune(r)
		}
	}
	return strings.TrimSpace(b.String())
}
