package contracts

// CapReleaseParse reads a release name (a torrent or NZB title, or a
// file name inside one) into structured fields (docs/slices/
// acquisition.md, A-6). Ordered-many, first accepted: the lain-parser
// model first, the hand-rolled tokenizer as fallback. A provider that
// abstains returns a zero Release (empty Title).
const CapReleaseParse = "lain.release.parse@1"

// ReleaseParseInput names the release. Kind is an optional hint
// ("anime", "series", "movie", "manga", "comic") that picks the
// reading grammar for archives.
type ReleaseParseInput struct {
	Name string `json:"name"`
	Kind string `json:"kind,omitempty"`
}

// Release is a parsed release name. Zero numbers mean "not stated".
type Release struct {
	Title        string `json:"title"`
	EpisodeTitle string `json:"episode_title,omitempty"`
	Year         int    `json:"year,omitempty"`
	Season       int    `json:"season,omitempty"`
	// Episodes lists every episode the release covers (a pack lists a
	// range). Absolute is true when they are absolute numbers (anime
	// with no season marker).
	Episodes []int `json:"episodes,omitempty"`
	Absolute bool  `json:"absolute,omitempty"`
	// Volume and Chapter are the reading-slice equivalents (D-085).
	Volume  int `json:"volume,omitempty"`
	Chapter int `json:"chapter,omitempty"`
	// SeasonPack marks a release covering whole seasons with no episode
	// list ("S01 Complete", "Season 2 Batch").
	SeasonPack bool   `json:"season_pack,omitempty"`
	Group      string `json:"group,omitempty"`
	Resolution string `json:"resolution,omitempty"` // "2160p", "1080p", ...
	Source     string `json:"source,omitempty"`     // "bluray", "web", "hdtv", "dvd"
	Codec      string `json:"codec,omitempty"`      // "h264", "h265", "av1"
	Proper     bool   `json:"proper,omitempty"`
	Repack     bool   `json:"repack,omitempty"`
	Version    int    `json:"version,omitempty"` // "v2" re-releases
	// Parser is the provider that produced the record.
	Parser string `json:"parser"`
}

// Accepted reports a usable parse.
func (r Release) Accepted() bool { return r.Title != "" }
