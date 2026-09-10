// Hand-rolled release-name parser: one tokenizer pass plus token
// classification, zero regex. Release names are human-written with a
// small pattern inventory ([Group], SxxExx, "- N", 4th Season, years,
// technical tags), so explicit token rules cover them faster and more
// predictably than stacked regular expressions.
package identify

import (
	"path/filepath"
	"regexp"
	"strings"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

const (
	AnimeID   = "lain-identify-anime"
	GenericID = "lain-identify-generic"
)

// reBrackets/reSep survive for Generic, the extension fallback.
var (
	reBrackets = regexp.MustCompile(`[\[\(][^\]\)]*[\]\)]`)
	reSep      = regexp.MustCompile(`[._]+`)
)

// videoExts are stripped repeatedly: "file.mkv.mp4" names the video,
// not a video about mkv.
var videoExts = map[string]bool{
	"mkv": true, "mp4": true, "avi": true, "mov": true,
	"m4v": true, "webm": true,
}

// techTokens are release tags, never title words. Dash-split artifacts
// ("WEB-DL" -> "web","dl") are included as separate entries.
var techTokens = map[string]bool{
	"2160p": true, "1080p": true, "720p": true, "480p": true,
	"1080i": true, "720i": true,
	"bluray": true, "webdl": true, "web": true, "dl": true,
	"webrip": true, "hdtv": true, "dvdrip": true, "dvd": true,
	"bdrip": true, "brrip": true,
	"x264": true, "x265": true, "h264": true, "h265": true,
	"hevc": true, "avc": true, "av1": true, "xvid": true, "divx": true,
	"flac": true, "aac": true, "ac3": true, "opus": true, "dts": true,
	"hd": true, "vorbis": true, "truehd": true, "atmos": true,
	"dual": true, "audio": true, "multi": true, "subs": true, "sub": true,
	"subtitle": true, "subtitles": true, "multiple": true,
	"dubbed": true, "subbed": true, "dublado": true,
	"hdr": true, "hdr10": true, "dolby": true, "vision": true,
	"10bit": true, "8bit": true,
	"remastered": true, "extended": true, "unrated": true,
	"repack": true, "proper": true, "uncut": true,
}

// resTokens / codecTokens / srcTokens feed confidence + evidence with
// the same meaning the regex version had.
var resTokens = map[string]bool{
	"2160p": true, "1080p": true, "720p": true, "480p": true,
}

var codecTokens = map[string]bool{
	"x264": true, "x265": true, "h264": true, "h265": true,
	"hevc": true, "avc": true, "av1": true,
}

var srcTokens = map[string]bool{
	"bluray": true, "webdl": true, "web": true, "webrip": true,
	"hdtv": true, "dvd": true, "dvdrip": true, "bdrip": true, "brrip": true,
}

// isTech reports release tags, including numbered variants the map
// holds unnumbered ("multi" covers "multi4", "hdr" covers "hdr10").
// Pure numbers never qualify: stripping "24" must not erase episodes.
func isTech(l string) bool {
	if techTokens[l] {
		return true
	}
	i := len(l)
	for i > 0 && l[i-1] >= '0' && l[i-1] <= '9' {
		i--
	}
	return i > 0 && i < len(l) && techTokens[l[:i]]
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

// token is one separator-delimited word with its bracket context.
// Bracketed words feed evidence but never the title: "[1080p]" tags
// the release without naming it.
type token struct {
	text      string // original case, for titles
	lower     string
	bracketed bool
}

// isSep reports tokenizer separators. Brackets split words AND mark
// context; dash/underscore/dot/space split words.
func isSep(c byte) bool {
	switch c {
	case ' ', '\t', '.', '_', '-', '[', ']', '(', ')':
		return true
	}
	return false
}

func isDigit(c byte) bool { return c >= '0' && c <= '9' }

// splitTokens cuts s into words in one pass. Digit runs glued to a dot
// digit ("Opus 2.0", "DUAL 5.1") stay one token so versions never read
// as episode numbers.
func splitTokens(s string) []token {
	var out []token
	depth := 0
	i, n := 0, len(s)
	for i < n {
		c := s[i]
		if c == '[' || c == '(' {
			depth++
			i++
			continue
		}
		if c == ']' || c == ')' {
			if depth > 0 {
				depth--
			}
			i++
			continue
		}
		if isSep(c) {
			i++
			continue
		}
		start := i
		allDigits := true
		for i < n && !isSep(s[i]) && s[i] != '[' && s[i] != '(' && s[i] != ']' && s[i] != ')' {
			if s[i] == '.' && allDigits && i+1 < n && isDigit(s[i+1]) {
				i++ // glued version: "2.0" stays whole
				continue
			}
			if !isDigit(s[i]) {
				allDigits = false
			}
			i++
		}
		_ = allDigits
		t := s[start:i]
		out = append(out, token{text: t, lower: strings.ToLower(t), bracketed: depth > 0})
	}
	return out
}

// atoi parses a small ASCII digit run.
func atoi(s string) (int, bool) {
	if s == "" || len(s) > 4 {
		return 0, false
	}
	v := 0
	for i := 0; i < len(s); i++ {
		if !isDigit(s[i]) {
			return 0, false
		}
		v = v*10 + int(s[i]-'0')
	}
	return v, true
}

// parseSxxExx matches S01E05 (any case, any zero padding).
func parseSxxExx(t string) (season, episode int, ok bool) {
	// t is already lowercase.
	if len(t) < 4 || t[0] != 's' {
		return 0, 0, false
	}
	e := -1
	for i := 1; i < len(t); i++ {
		if t[i] == 'e' {
			e = i
			break
		}
		if !isDigit(t[i]) {
			return 0, 0, false
		}
	}
	if e < 0 || e+1 >= len(t) {
		return 0, 0, false
	}
	sn, ok1 := atoi(t[1:e])
	en, ok2 := atoi(t[e+1:])
	if !ok1 || !ok2 || len(t[1:e]) > 2 || len(t[e+1:]) > 3 {
		return 0, 0, false
	}
	return sn, en, true
}

// parseNxM matches 2x13 (digits on both sides of x).
func parseNxM(t string) (a, b int, ok bool) {
	x := -1
	for i := 0; i < len(t); i++ {
		if t[i] == 'x' {
			x = i
			break
		}
		if !isDigit(t[i]) {
			return 0, 0, false
		}
	}
	if x <= 0 || x+1 >= len(t) || x > 2 {
		return 0, 0, false
	}
	an, ok1 := atoi(t[:x])
	bn, ok2 := atoi(t[x+1:])
	if !ok1 || !ok2 || len(t[x+1:]) > 3 {
		return 0, 0, false
	}
	return an, bn, true
}

// stripVersionSuffix turns "24v2" into 24.
func stripVersionSuffix(t string) string {
	for i := len(t) - 1; i >= 0; i-- {
		if t[i] == 'v' && i > 0 {
			if _, ok := atoi(t[:i]); ok {
				if _, ok := atoi(t[i+1:]); ok {
					return t[:i]
				}
			}
			return t
		}
		if !isDigit(t[i]) {
			return t
		}
	}
	return t
}

// isYear reports isolated release years (never inside 1080p: the
// tokenizer splits those into "1080p", which is tech, not digits).
func isYear(t string) bool {
	if len(t) != 4 {
		return false
	}
	y, ok := atoi(t)
	return ok && y >= 1900 && y <= 2099
}

// parseOrdinal matches "4th" -> 4.
func parseOrdinal(t string) (int, bool) {
	var suf string
	switch {
	case strings.HasSuffix(t, "st"):
		suf = "st"
	case strings.HasSuffix(t, "nd"):
		suf = "nd"
	case strings.HasSuffix(t, "rd"):
		suf = "rd"
	case strings.HasSuffix(t, "th"):
		suf = "th"
	default:
		return 0, false
	}
	return atoi(t[:len(t)-len(suf)])
}

// IdentifyAnime returns a proposal and whether it found release evidence.
// No evidence (plain "movie.mp4") declines so the next provider runs.
func IdentifyAnime(c contracts.Candidate) (contracts.Proposal, bool) {
	stem := stemOf(filepath.Base(c.Path))
	evidence := []string{}
	addEvidence := func(e string) {
		for _, x := range evidence {
			if x == e {
				return
			}
		}
		evidence = append(evidence, e)
	}

	// Leading "[Group]" is the release group, not the title.
	group := ""
	rest := strings.TrimLeft(stem, " \t")
	if strings.HasPrefix(rest, "[") {
		if end := strings.IndexByte(rest, ']'); end > 1 {
			group = rest[1:end]
			addEvidence("release-group")
			rest = rest[end+1:]
		}
	}

	toks := splitTokens(rest)
	season, episode := 0, 0
	strongEp := false
	year := 0
	hasTech, hasRes := false, false
	epOutIdx := -1 // unbracketed-word count before the episode marker
	seasonSkip := map[int]bool{}

	outCount := 0
	for i, tk := range toks {
		l := tk.lower
		if resTokens[l] {
			hasRes, hasTech = true, true
			addEvidence("resolution")
		} else if codecTokens[l] {
			hasTech = true
			addEvidence("codec")
		} else if srcTokens[l] {
			hasTech = true
			addEvidence("source")
		} else if isTech(l) {
			hasTech = true
		}
		if isYear(l) && year == 0 {
			year = mustAtoi(l)
			addEvidence("year")
		}
		if tk.bracketed {
			continue
		}
		// Season words with one-token lookahead ("4th Season",
		// "Season 2"), unbracketed only.
		if season == 0 {
			if n, ok := parseOrdinal(l); ok && n <= 30 && nextIs(toks, i, "season") {
				season = n
				seasonSkip[i], seasonSkip[i+1] = true, true
				addEvidence("season-word")
			} else if l == "season" && i+1 < len(toks) {
				if n, ok := atoi(toks[i+1].lower); ok && n <= 30 {
					season = n
					seasonSkip[i], seasonSkip[i+1] = true, true
					addEvidence("season-word")
				}
			}
		}
		if epOutIdx < 0 || !strongEp {
			if sn, en, ok := parseSxxExx(l); ok && !strongEp {
				season, episode = sn, en
				strongEp = true
				epOutIdx = outCount
				addEvidence("sxxexx")
			} else if an, bn, ok := parseNxM(l); ok && !strongEp {
				season, episode = an, bn
				strongEp = true
				epOutIdx = outCount
				addEvidence("nxm")
			} else if epOutIdx < 0 {
				if en, ok := weakEpisode(l); ok {
					episode = en
					epOutIdx = outCount
					addEvidence("episode-number")
				}
			}
		}
		outCount++
	}
	if epOutIdx < 0 {
		// Parity fallback: unambiguous markers hidden inside brackets
		// ("Show [S01E05]"). Title keeps every outside word.
		for _, tk := range toks {
			if !tk.bracketed {
				continue
			}
			if sn, en, ok := parseSxxExx(tk.lower); ok {
				season, episode = sn, en
				strongEp = true
				epOutIdx = outCount
				addEvidence("sxxexx")
				break
			}
			if an, bn, ok := parseNxM(tk.lower); ok {
				season, episode = an, bn
				strongEp = true
				epOutIdx = outCount
				addEvidence("nxm")
				break
			}
		}
	}
	if year > 0 && episode == year && !strongEp {
		// A lone 4-digit number is a year, not episode 1995.
		episode = 0
		epOutIdx = -1 // no marker, no title cut either
		kept := evidence[:0]
		for _, e := range evidence {
			if e != "episode-number" {
				kept = append(kept, e)
			}
		}
		evidence = kept
	}
	if group == "" && !strongEp && !(year > 0 && hasTech) {
		// Without a group: only unambiguous episode markers, or a year
		// corroborated by technical tags (dot-style movie releases),
		// count. Everything else declines to generic.
		return contracts.Proposal{}, false
	}

	// Title: unbracketed words before the episode marker, minus tags.
	var words []string
	n := 0
	for i, tk := range toks {
		if tk.bracketed {
			continue
		}
		if seasonSkip[i] {
			n++
			continue
		}
		if epOutIdx >= 0 && n >= epOutIdx {
			break
		}
		l := tk.lower
		if isTech(l) || resTokens[l] || codecTokens[l] || srcTokens[l] {
			n++
			continue
		}
		if isYear(l) {
			n++
			continue
		}
		if _, ok := parseOrdinal(l); ok {
			n++
			continue
		}
		words = append(words, tk.text)
		n++
	}
	if len(words) == 0 {
		return contracts.Proposal{}, false
	}

	conf := 0.7
	if group != "" {
		conf += 0.1
	}
	if hasRes {
		conf += 0.08
	}
	if conf > 0.96 {
		conf = 0.96
	}
	kind := "episode"
	if season == 0 && episode == 0 {
		kind = "video"
	}
	return contracts.Proposal{
		Kind: kind, Title: strings.Join(words, " "), Season: season,
		Episode: episode, Year: year,
		Confidence: conf, Evidence: evidence, PluginID: AnimeID,
	}, true
}

// nextIs reports whether the following token equals want (unbracketed).
func nextIs(toks []token, i int, want string) bool {
	return i+1 < len(toks) && !toks[i+1].bracketed && toks[i+1].lower == want
}

// weakEpisode matches bare numbers ("12", "24v2") and E-prefixed
// numbers ("E05", "EP12"). Years never reach here: 4-digit 19xx/20xx
// tokens classify as year in the main loop (checked first by the
// caller ordering: weakEpisode itself refuses nothing, so the caller
// must prefer the year reading — see the year-guard).
func weakEpisode(l string) (int, bool) {
	if strings.HasPrefix(l, "ep") {
		return atoi(l[2:])
	}
	if strings.HasPrefix(l, "e") && len(l) > 1 {
		if n, ok := atoi(l[1:]); ok {
			return n, true
		}
	}
	return atoi(stripVersionSuffix(l))
}

func mustAtoi(s string) int {
	n, _ := atoi(s)
	return n
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
