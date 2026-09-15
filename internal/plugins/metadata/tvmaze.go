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

// TVMaze serves series metadata from the TVMaze JSON API (public reads
// need no key). TV-first: it answers series/episode/video/anime kinds
// and stays silent for movies, which TVMaze does not catalogue. Anime
// keeps its existing precedence (Kitsu/AniList/Jikan stay ahead), so
// this provider only adds coverage where they have none: live-action
// series. BaseURL is overridable for tests.
type TVMaze struct {
	BaseURL string
	http    *politeClient
}

func NewTVMaze() *TVMaze {
	return &TVMaze{BaseURL: "https://api.tvmaze.com", http: newPoliteClient(1 * time.Second)}
}

func (t *TVMaze) ID() string { return "lain-metadata-tvmaze" }
func (t *TVMaze) Capabilities() []string {
	return []string{contracts.CapMetadataSearch, contracts.CapMetadataResolve}
}
func (t *TVMaze) Health() error { return nil }

func (t *TVMaze) Invoke(cap string, input any) (any, error) {
	switch cap {
	case contracts.CapMetadataSearch:
		in, ok := input.(contracts.MetadataSearchInput)
		if !ok {
			return nil, &core.Error{Code: "invalid-message", Msg: "MetadataSearchInput required"}
		}
		if in.Kind != "" && in.Kind != "anime" && in.Kind != "series" && in.Kind != "episode" && in.Kind != "video" {
			return []contracts.MetadataCandidate{}, nil
		}
		return t.search(in)
	case contracts.CapMetadataResolve:
		in, ok := input.(contracts.MetadataResolveInput)
		if !ok {
			return nil, &core.Error{Code: "invalid-message", Msg: "MetadataResolveInput required"}
		}
		return t.resolve(in.RemoteID)
	default:
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
}

type tvmazeImage struct {
	Medium   string `json:"medium"`
	Original string `json:"original"`
}

// best prefers the full-size portrait; the medium thumbnail keeps a
// poster when the original is missing.
func (img *tvmazeImage) best() string {
	if img == nil {
		return ""
	}
	if img.Original != "" {
		return img.Original
	}
	return img.Medium
}

type tvmazeShow struct {
	ID        int          `json:"id"`
	Name      string       `json:"name"`
	Genres    []string     `json:"genres"`
	Premiered string       `json:"premiered"`
	Summary   string       `json:"summary"`
	Image     *tvmazeImage `json:"image"`
}

func tvmazeYear(premiered string) int {
	if len(premiered) >= 4 {
		if y, err := strconv.Atoi(premiered[:4]); err == nil {
			return y
		}
	}
	return 0
}

func (t *TVMaze) search(in contracts.MetadataSearchInput) ([]contracts.MetadataCandidate, error) {
	u := t.BaseURL + "/search/shows?q=" + url.QueryEscape(in.Query)
	raw, err := t.http.get(u)
	if err != nil {
		return nil, err
	}
	var doc []struct {
		Show tvmazeShow `json:"show"`
	}
	if err := json.Unmarshal(raw, &doc); err != nil {
		return nil, err
	}
	limit := in.Limit
	if limit <= 0 || limit > 10 {
		limit = 5
	}
	var out []contracts.MetadataCandidate
	for _, hit := range doc {
		s := hit.Show
		if strings.TrimSpace(s.Name) == "" {
			continue
		}
		out = append(out, contracts.MetadataCandidate{
			Provider: t.ID(), RemoteID: strconv.Itoa(s.ID),
			Title: strings.TrimSpace(s.Name), Year: tvmazeYear(s.Premiered),
			Kind: "series", Poster: s.Image.best(),
		})
		if len(out) >= limit {
			break
		}
	}
	if out == nil {
		out = []contracts.MetadataCandidate{}
	}
	return out, nil
}

func (t *TVMaze) resolve(id string) (contracts.MetadataRecord, error) {
	raw, err := t.http.get(t.BaseURL + "/shows/" + url.PathEscape(id))
	if err != nil {
		return contracts.MetadataRecord{}, err
	}
	var s tvmazeShow
	if err := json.Unmarshal(raw, &s); err != nil {
		return contracts.MetadataRecord{}, err
	}
	if strings.TrimSpace(s.Name) == "" {
		return contracts.MetadataRecord{}, &core.Error{Code: "dependency-unavailable", Msg: "tvmaze: empty show"}
	}
	return contracts.MetadataRecord{
		Provider: t.ID(), RemoteID: strconv.Itoa(s.ID),
		Title: strings.TrimSpace(s.Name), Year: tvmazeYear(s.Premiered),
		Genres: s.Genres, Synopsis: stripHTML(s.Summary),
		Poster: s.Image.best(),
		// Cover stays empty: TVMaze ships one portrait image per show,
		// no backdrop. Episode counts need a second call and stay out
		// of this slice; identity and artwork are what grids need.
	}, nil
}
