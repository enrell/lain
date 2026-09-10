package metadata

import (
	"encoding/xml"
	"os"
	"path/filepath"
	"strconv"
	"strings"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

// NFO reads Kodi-style sidecar files next to the media. Fully offline,
// user-curated, always first in the default precedence: when the user
// wrote down what something is, the network does not get a vote.
// RemoteID is the absolute .nfo path (local-only by nature).
type NFO struct{}

func (NFO) ID() string { return "lain-metadata-nfo" }
func (NFO) Capabilities() []string {
	return []string{contracts.CapMetadataSearch, contracts.CapMetadataResolve}
}
func (NFO) Health() error { return nil }

func (NFO) Invoke(cap string, input any) (any, error) {
	switch cap {
	case contracts.CapMetadataSearch:
		in, ok := input.(contracts.MetadataSearchInput)
		if !ok {
			return nil, &core.Error{Code: "invalid-message", Msg: "MetadataSearchInput required"}
		}
		return searchNFO(in)
	case contracts.CapMetadataResolve:
		in, ok := input.(contracts.MetadataResolveInput)
		if !ok {
			return nil, &core.Error{Code: "invalid-message", Msg: "MetadataResolveInput required"}
		}
		return resolveNFO(in.RemoteID)
	default:
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
}

// kodiNFO covers movie, tvshow and episodedetails documents.
type kodiNFO struct {
	XMLName       xml.Name `xml:""`
	Title         string   `xml:"title"`
	OriginalTitle string   `xml:"originaltitle"`
	Plot          string   `xml:"plot"`
	Premiered     string   `xml:"premiered"`
	Year          string   `xml:"year"`
	Genres        []string `xml:"genre"`
	Thumbs        []struct {
		Aspect string `xml:"aspect,attr"`
		URL    string `xml:",chardata"`
	} `xml:"thumb"`
	Fanart struct {
		Thumbs []struct {
			URL string `xml:",chardata"`
		} `xml:"thumb"`
	} `xml:"fanart"`
}

func searchNFO(in contracts.MetadataSearchInput) ([]contracts.MetadataCandidate, error) {
	out := []contracts.MetadataCandidate{}
	if in.Dir == "" {
		return out, nil
	}
	entries, err := os.ReadDir(in.Dir)
	if err != nil {
		return out, nil // unreadable dir: no local opinion, not an error
	}
	checked := 0
	for _, e := range entries {
		if e.IsDir() || checked >= 8 {
			continue
		}
		name := e.Name()
		low := strings.ToLower(name)
		if !strings.HasSuffix(low, ".nfo") {
			continue
		}
		// Show/movie roots always count; other sidecars only when the
		// filename mentions the query (a folder of episode nfos must
		// not all match one search).
		if low != "tvshow.nfo" && low != "movie.nfo" && !strings.Contains(low, strings.ToLower(in.Query)) {
			continue
		}
		checked++
		rec, err := resolveNFO(filepath.Join(in.Dir, name))
		if err != nil {
			continue
		}
		out = append(out, contracts.MetadataCandidate{
			Provider: "lain-metadata-nfo", RemoteID: filepath.Join(in.Dir, name),
			Title: rec.Title, Year: rec.Year, Kind: in.Kind, Poster: rec.Poster,
		})
		if len(out) >= in.Limit && in.Limit > 0 {
			break
		}
	}
	return out, nil
}

func resolveNFO(path string) (contracts.MetadataRecord, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return contracts.MetadataRecord{}, err
	}
	var doc kodiNFO
	dec := xml.NewDecoder(strings.NewReader(string(raw)))
	dec.Strict = false
	if err := dec.Decode(&doc); err != nil {
		return contracts.MetadataRecord{}, err
	}
	title := strings.TrimSpace(doc.Title)
	if title == "" {
		title = strings.TrimSpace(doc.OriginalTitle)
	}
	year := 0
	if y, err := strconv.Atoi(strings.TrimSpace(doc.Year)); err == nil && y >= 1900 && y <= 2099 {
		year = y
	} else if len(doc.Premiered) >= 4 {
		if y, err := strconv.Atoi(doc.Premiered[:4]); err == nil {
			year = y
		}
	}
	poster, cover := "", ""
	for _, t := range doc.Thumbs {
		if u := strings.TrimSpace(t.URL); u != "" && t.Aspect == "poster" && poster == "" {
			poster = u
		}
	}
	for _, t := range doc.Thumbs {
		if u := strings.TrimSpace(t.URL); u != "" {
			if poster == "" {
				poster = u
			} else if cover == "" && u != poster {
				cover = u
			}
		}
	}
	if len(doc.Fanart.Thumbs) > 0 && cover == "" {
		cover = strings.TrimSpace(doc.Fanart.Thumbs[0].URL)
	}
	return contracts.MetadataRecord{
		Provider: "lain-metadata-nfo", RemoteID: path, Title: title,
		Year: year, Genres: doc.Genres, Synopsis: strings.TrimSpace(doc.Plot),
		Poster: poster, Cover: cover,
	}, nil
}
