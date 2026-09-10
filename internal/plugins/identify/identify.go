// Package identify holds the two built-in filename identifiers.
//
//   - anime: specialized for fansub/release names ("[Group] Title - 12
//     [1080p][HEVC]"). High confidence only on real release evidence.
//   - generic: fallback that classifies by extension and cleans the
//     basename. It always accepts so the pipeline never drops a file.
//
// The ordered-many binding tries anime first, then generic. Swapping in
// a community parser means inserting it before generic; removing anime
// leaves generic serving with lower confidence, never a failed scan.
package identify

import (
	"path/filepath"
	"regexp"
	"strconv"
	"strings"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

const (
	AnimeID   = "lain-identify-anime"
	GenericID = "lain-identify-generic"
)

var (
	reGroup    = regexp.MustCompile(`^\s*\[([^\]]+)\]`)
	reSxxExx   = regexp.MustCompile(`(?i)[Ss](\d{1,2})[Ee](\d{1,3})`)
	reNxM      = regexp.MustCompile(`(\d{1,2})[xX](\d{1,3})`)
	reDashEp   = regexp.MustCompile(`(?:^|[\s_\.\-])(?:[Ee](?:p|P)?\.?\s?)?(\d{1,4})(?:v\d+)?(?:\s*[\[\(]|\s|$)`)
	reRes      = regexp.MustCompile(`(?i)(2160p|1080p|720p|480p)`)
	reCodec    = regexp.MustCompile(`(?i)(x264|x265|h\.?264|h\.?265|hevc|avc|av1)`)
	reSrc      = regexp.MustCompile(`(?i)(blu-?ray|web-?dl|webrip|hdtv|dvd)`)
	reJunk     = regexp.MustCompile(`(?i)[\[\(](1080p|720p|2160p|480p|[^)\]]*(x264|x265|hevc|avc|av1|flac|aac|ac3|opus|8bit|10bit|multi|dual)[^)\]]*)[\]\)]`)
	reBrackets = regexp.MustCompile(`[\[\(][^\]\)]*[\]\)]`)
	reSep      = regexp.MustCompile(`[._]+`)
)

// Anime is the release-name specialist.
type Anime struct{}

func (Anime) ID() string             { return AnimeID }
func (Anime) Capabilities() []string { return []string{contracts.CapMediaIdentify} }
func (Anime) Health() error          { return nil }

func (Anime) Invoke(cap string, input any) (any, error) {
	if cap != contracts.CapMediaIdentify {
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
	c, ok := input.(contracts.Candidate)
	if !ok {
		return nil, &core.Error{Code: "invalid-message", Msg: "Candidate required"}
	}
	p, _ := IdentifyAnime(c)
	return p, nil
}

// IdentifyAnime returns a proposal and whether it found release evidence.
// No evidence (plain "movie.mp4") declines so the next provider runs.
func IdentifyAnime(c contracts.Candidate) (contracts.Proposal, bool) {
	base := filepath.Base(c.Path)
	name := strings.TrimSuffix(base, filepath.Ext(base))
	evidence := []string{}

	group := ""
	if m := reGroup.FindStringSubmatch(name); m != nil {
		group = m[1]
		evidence = append(evidence, "release-group")
	}
	season, episode := 0, 0
	if m := reSxxExx.FindStringSubmatch(name); m != nil {
		season, _ = strconv.Atoi(m[1])
		episode, _ = strconv.Atoi(m[2])
		evidence = append(evidence, "sxxexx")
	} else if m := reNxM.FindStringSubmatch(name); m != nil {
		season, _ = strconv.Atoi(m[1])
		episode, _ = strconv.Atoi(m[2])
		evidence = append(evidence, "nxm")
	} else if m := reDashEp.FindStringSubmatch(name + " "); m != nil {
		episode, _ = strconv.Atoi(m[1])
		evidence = append(evidence, "episode-number")
	}
	if len(evidence) == 0 || (len(evidence) == 1 && evidence[0] == "episode-number" && group == "") {
		// Bare numbers without group or SxxExx are too weak: decline.
		if group == "" {
			return contracts.Proposal{}, false
		}
	}
	title := name
	if group != "" {
		title = strings.TrimSpace(strings.TrimPrefix(title, "["+group+"]"))
	}
	// Cut at the episode marker, drop technical tags.
	if idx := episodeMarkerIndex(title, season, episode); idx > 0 {
		title = title[:idx]
	}
	title = reJunk.ReplaceAllString(title, " ")
	title = reBrackets.ReplaceAllString(title, " ")
	title = strings.ReplaceAll(title, "-", " ")
	title = reSep.ReplaceAllString(title, " ")
	title = strings.Join(strings.Fields(title), " ")
	if title == "" {
		return contracts.Proposal{}, false
	}
	conf := 0.7
	if group != "" {
		conf += 0.1
	}
	if reRes.MatchString(name) {
		conf += 0.08
		evidence = append(evidence, "resolution")
	}
	if reCodec.MatchString(name) {
		evidence = append(evidence, "codec")
	}
	if reSrc.MatchString(name) {
		evidence = append(evidence, "source")
	}
	if conf > 0.96 {
		conf = 0.96
	}
	kind := "episode"
	if season == 0 && episode == 0 {
		kind = "video"
	}
	return contracts.Proposal{
		Kind: kind, Title: title, Season: season, Episode: episode,
		Confidence: conf, Evidence: evidence, PluginID: AnimeID,
	}, true
}

func episodeMarkerIndex(title string, season, episode int) int {
	if episode > 0 {
		re := regexp.MustCompile(`[\s_\.\-]` + strconv.Itoa(episode) + `(?:v\d+)?(?:$|[\s_\.\-\[\(])`)
		if loc := re.FindStringIndex(title); loc != nil {
			return loc[0]
		}
		if season > 0 {
			re2 := regexp.MustCompile(`(?i)S0*` + strconv.Itoa(season) + `E0*` + strconv.Itoa(episode))
			if loc := re2.FindStringIndex(title); loc != nil {
				return loc[0]
			}
		}
	}
	return -1
}

// Generic classifies by extension and always accepts.
type Generic struct{}

func (Generic) ID() string             { return GenericID }
func (Generic) Capabilities() []string { return []string{contracts.CapMediaIdentify} }
func (Generic) Health() error          { return nil }

func (Generic) Invoke(cap string, input any) (any, error) {
	if cap != contracts.CapMediaIdentify {
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
	c, ok := input.(contracts.Candidate)
	if !ok {
		return nil, &core.Error{Code: "invalid-message", Msg: "Candidate required"}
	}
	return IdentifyGeneric(c), nil
}

// IdentifyGeneric never declines: worst case it is an untitled file of a
// known kind with low confidence.
func IdentifyGeneric(c contracts.Candidate) contracts.Proposal {
	base := filepath.Base(c.Path)
	ext := strings.ToLower(strings.TrimPrefix(filepath.Ext(base), "."))
	name := strings.TrimSuffix(base, filepath.Ext(base))
	clean := reBrackets.ReplaceAllString(name, " ")
	clean = strings.ReplaceAll(clean, "-", " ")
	title := strings.Join(strings.Fields(reSep.ReplaceAllString(clean, " ")), " ")
	if title == "" {
		title = base
	}
	kind := "video"
	switch ext {
	case "mp3", "flac", "m4a", "aac", "ogg", "opus", "wav":
		kind = "audio"
	case "pdf", "epub", "mobi", "azw3":
		kind = "book"
	case "cbz", "cbr", "cb7":
		kind = "comic"
	case "jpg", "jpeg", "png", "webp", "gif", "heic", "avif":
		kind = "photo"
	}
	return contracts.Proposal{
		Kind: kind, Title: title, Confidence: 0.4,
		Evidence: []string{"extension"}, PluginID: GenericID,
	}
}
