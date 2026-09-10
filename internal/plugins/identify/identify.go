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
	reDashEp   = regexp.MustCompile(`(?:^|[\s_\.\-\[\(])(?:[Ee](?:p|P)?\.?\s?)?(\d{1,4})(?:v\d+)?(?:\s*[\[\(]|\s|$)`)
	reRes      = regexp.MustCompile(`(?i)(2160p|1080p|720p|480p)`)
	reCodec    = regexp.MustCompile(`(?i)(x264|x265|h\.?264|h\.?265|hevc|avc|av1)`)
	reSrc      = regexp.MustCompile(`(?i)(blu-?ray|web-?dl|webrip|hdtv|dvd)`)
	reJunk     = regexp.MustCompile(`(?i)[\[\(](1080p|720p|2160p|480p|[^)\]]*(x264|x265|hevc|avc|av1|flac|aac|ac3|opus|8bit|10bit|multi|dual)[^)\]]*)[\]\)]`)
	reBrackets = regexp.MustCompile(`[\[\(][^\]\)]*[\]\)]`)
	reSep      = regexp.MustCompile(`[._]+`)
	// "4th Season", "Season 2": season words used by release groups.
	reSeasonWord = regexp.MustCompile(`(?i)(?:\b(\d{1,2})(?:st|nd|rd|th)\s+season\b|\bseason\s+(\d{1,2})\b)`)
	// Isolated 4-digit year (dots, spaces, brackets — never inside 1080p).
	reYear = regexp.MustCompile(`(?:^|[\s\.\_\-\[\(])((?:19|20)\d{2})(?:$|[\s\.\_\-\]\)])`)
	// Standalone technical tokens in dot-style scene names
	// ("Film.2017.1080p.BluRay.AV1"): stripped after tokenization.
	reTech = regexp.MustCompile(`(?i)\b(2160p|1080p|720p|480p|blu-?ray|web-?dl|webrip|hdtv|dvdrip|x264|x265|h264|h265|hevc|avc|av1|flac|aac|ac3|opus|dts(?:-hd)?|dual(?:-audio)?|multi\d*|hdr\d*|10bit|8bit|remastered|extended|unrated|repack|proper)\b`)
)

// videoExts are stripped repeatedly: "file.mkv.mp4" names the video,
// not a video about mkv.
var videoExts = map[string]bool{
	"mkv": true, "mp4": true, "avi": true, "mov": true,
	"m4v": true, "webm": true,
}

// stemOf removes the extension, then any further trailing video
// extensions left by double-suffixed scene names.
func stemOf(base string) string {
	name := strings.TrimSuffix(base, filepath.Ext(base))
	for {
		ext := strings.ToLower(strings.TrimPrefix(filepath.Ext(name), "."))
		if ext == "" || !videoExts[ext] {
			return name
		}
		name = strings.TrimSuffix(name, filepath.Ext(name))
	}
}

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
	name := stemOf(filepath.Base(c.Path))
	evidence := []string{}

	group := ""
	if m := reGroup.FindStringSubmatch(name); m != nil {
		group = m[1]
		evidence = append(evidence, "release-group")
	}
	season, episode := 0, 0
	strongEp := false // SxxExx/NxM markers are unambiguous; bare numbers are not
	if m := reSxxExx.FindStringSubmatch(name); m != nil {
		season, _ = strconv.Atoi(m[1])
		episode, _ = strconv.Atoi(m[2])
		evidence = append(evidence, "sxxexx")
		strongEp = true
	} else if m := reNxM.FindStringSubmatch(name); m != nil {
		season, _ = strconv.Atoi(m[1])
		episode, _ = strconv.Atoi(m[2])
		evidence = append(evidence, "nxm")
		strongEp = true
	} else if m := reDashEp.FindStringSubmatch(name + " "); m != nil {
		episode, _ = strconv.Atoi(m[1])
		evidence = append(evidence, "episode-number")
	}
	// "4th Season" / "Season 2" words fill a missing season.
	if season == 0 {
		if m := reSeasonWord.FindStringSubmatch(name); m != nil {
			for _, g := range m[1:] {
				if g == "" {
					continue
				}
				if n, err := strconv.Atoi(g); err == nil {
					season = n
					evidence = append(evidence, "season-word")
					break
				}
			}
		}
	}
	year := 0
	if m := reYear.FindStringSubmatch(name + " "); m != nil {
		year, _ = strconv.Atoi(m[1])
		evidence = append(evidence, "year")
	}
	if year > 0 && episode == year && !strongEp {
		// A lone 4-digit number is a year, not episode 1995: drop the
		// episode reading so "(1995)" movies decline to generic.
		episode = 0
		kept := evidence[:0]
		for _, e := range evidence {
			if e != "episode-number" {
				kept = append(kept, e)
			}
		}
		evidence = kept
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
	if year > 0 {
		title = strings.ReplaceAll(title, strconv.Itoa(year), " ")
	}
	if season > 0 {
		title = reSeasonWord.ReplaceAllString(title, " ")
	}
	title = strings.ReplaceAll(title, "-", " ")
	title = reSep.ReplaceAllString(title, " ")
	title = reTech.ReplaceAllString(title, " ")
	title = strings.Join(strings.Fields(title), " ")
	if title == "" {
		return contracts.Proposal{}, false
	}
	conf := 0.7
	hasTech := false
	if group != "" {
		conf += 0.1
	}
	if reRes.MatchString(name) {
		conf += 0.08
		evidence = append(evidence, "resolution")
		hasTech = true
	}
	if reCodec.MatchString(name) {
		evidence = append(evidence, "codec")
		hasTech = true
	}
	if reSrc.MatchString(name) {
		evidence = append(evidence, "source")
		hasTech = true
	}
	if group == "" && !strongEp && !(year > 0 && hasTech) {
		// Without a group: only unambiguous episode markers, or a year
		// corroborated by technical tags (dot-style movie releases),
		// count. Everything else declines to generic.
		return contracts.Proposal{}, false
	}
	if conf > 0.96 {
		conf = 0.96
	}
	kind := "episode"
	if season == 0 && episode == 0 {
		kind = "video"
	}
	return contracts.Proposal{
		Kind: kind, Title: title, Season: season, Episode: episode, Year: year,
		Confidence: conf, Evidence: evidence, PluginID: AnimeID,
	}, true
}

func episodeMarkerIndex(title string, season, episode int) int {
	if episode > 0 {
		// Specific SxxExx shape first: it is unambiguous, while a bare
		// number can misfire on technical tags ("Opus 2.0").
		re2 := regexp.MustCompile(`(?i)S0*` + strconv.Itoa(season) + `E0*` + strconv.Itoa(episode))
		if loc := re2.FindStringIndex(title); loc != nil {
			return loc[0]
		}
		re := regexp.MustCompile(`[\s_\.\-]0*` + strconv.Itoa(episode) + `(?:v\d+)?(?:$|[\s_\.\-\[\(])`)
		if loc := re.FindStringIndex(title); loc != nil {
			return loc[0]
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
