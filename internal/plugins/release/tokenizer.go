package release

import (
	"path/filepath"
	"strconv"
	"strings"
	"unicode"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/plugins/catalog"
)

// Tokenize parses a release name with hand-written rules: no model, no
// regex. It is the fallback of lain.release.parse@1 and the source of
// technical tags (source, codec, proper) for the model's records.
//
// Grammar it understands, in any of the separator styles (spaces, dots,
// underscores):
//
//	[Group] Title - 05 (1080p) [CRC32]        absolute anime episode
//	[Group] Title - 01-12 [Batch]             absolute range
//	Title.S01E02E03.1080p.WEB-DL.x264-GROUP   season/episodes, suffix group
//	Title S02 Complete 1080p BluRay           season pack
//	Title Season 2 Episode 5
//	Title (2019) 2160p UHD BluRay             movie with year
//	Title v01 c001 / Title Vol. 2 Ch. 15      volumes and chapters
func Tokenize(name, kind string) contracts.Release {
	r := contracts.Release{Parser: TokenizerID}
	name = strings.TrimSpace(stripExt(name))
	if name == "" {
		return r
	}

	// Leading [Group].
	if strings.HasPrefix(name, "[") {
		if end := strings.IndexByte(name, ']'); end > 1 {
			g := strings.TrimSpace(name[1:end])
			if g != "" && !isTechTag(g) {
				r.Group = g
			}
			name = strings.TrimSpace(name[end+1:])
		}
	}

	// Bracketed tags anywhere else are technical metadata, never title.
	var tags []string
	name, tags = pullBrackets(name)

	// Scene style: dots/underscores as separators. A trailing "-GROUP"
	// after the last technical token is the group.
	name = strings.NewReplacer("_", " ").Replace(name)
	if !strings.Contains(name, " ") && strings.Count(name, ".") >= 2 {
		name = dotsToSpaces(name)
	}
	if r.Group == "" {
		if i := strings.LastIndexByte(name, '-'); i > 0 && i < len(name)-1 {
			cand := name[i+1:]
			if !strings.ContainsAny(cand, " ") && !isNumber(cand) && len(cand) <= 32 {
				before := strings.Fields(name[:i])
				if len(before) > 0 && isTechTag(lastWord(before)) {
					r.Group = cand
					name = name[:i]
				}
			}
		}
	}

	toks := strings.Fields(name)
	titleEnd := len(toks)
	markAt := func(i int) {
		if i < titleEnd {
			titleEnd = i
		}
	}
	reading := kind == contracts.KindManga || kind == contracts.KindComic

	for i := 0; i < len(toks); i++ {
		raw := toks[i]
		l := strings.ToLower(strings.Trim(raw, ",;"))
		switch {
		case parseSeasonEpisodes(l, &r):
			markAt(i)
		case len(l) >= 2 && l[0] == 's' && isNumber(l[1:]) && len(l) <= 4:
			r.Season, _ = strconv.Atoi(l[1:])
			r.SeasonPack = true
			markAt(i)
		case (l == "season" || l == "saison") && i+1 < len(toks) && isNumber(toks[i+1]):
			r.Season, _ = strconv.Atoi(toks[i+1])
			r.SeasonPack = true
			markAt(i)
			i++
		case (l == "episode" || l == "ep") && i+1 < len(toks) && isNumber(toks[i+1]):
			n, _ := strconv.Atoi(toks[i+1])
			r.Episodes = []int{n}
			r.SeasonPack = false
			markAt(i)
			i++
		case l == "-" && i+1 < len(toks) && i > 0:
			// "Title - 05", "Title - 01-12", "Title - 05v2".
			if eps, ver, ok := parseAbsolute(strings.ToLower(toks[i+1])); ok && !reading {
				r.Episodes, r.Absolute = eps, r.Season == 0
				if ver > 0 {
					r.Version = ver
				}
				markAt(i)
				i++
			} else {
				markAt(i) // a dash ends the title: what follows is an episode title or tags
			}
		case reading && (l == "vol" || l == "vol." || l == "volume") && i+1 < len(toks) && isNumber(strings.Trim(toks[i+1], ".")):
			r.Volume, _ = strconv.Atoi(strings.Trim(toks[i+1], "."))
			markAt(i)
			i++
		case reading && (l == "ch" || l == "ch." || l == "chapter") && i+1 < len(toks) && isNumber(strings.Trim(toks[i+1], ".")):
			r.Chapter, _ = strconv.Atoi(strings.Trim(toks[i+1], "."))
			markAt(i)
			i++
		case reading && len(l) >= 2 && (l[0] == 'v' || l[0] == 'c') && isNumber(l[1:]) && i > 0:
			n, _ := strconv.Atoi(l[1:])
			if l[0] == 'v' {
				r.Volume = n
			} else {
				r.Chapter = n
			}
			markAt(i)
		case reading && len(l) >= 2 && l[0] == '#' && isNumber(l[1:]) && i > 0:
			r.Chapter, _ = strconv.Atoi(l[1:])
			markAt(i)
		case isYearToken(l) && i > 0:
			// The last plausible year wins: "1917 2019" is a title then a year.
			r.Year, _ = strconv.Atoi(strings.Trim(l, "()"))
			markAt(i)
		case l == "complete" || l == "batch":
			if len(r.Episodes) == 0 {
				r.SeasonPack = true
			}
			markAt(i)
		case isTechTag(l):
			applyTag(l, &r)
			markAt(i)
		case reading && i > 0 && isNumber(l) && len(l) <= 4 && r.Chapter == 0 && !isYearToken(l) && trailingNumber(toks[i+1:]):
			// "Saga 054", "Saga 054 (2018)": a trailing bare number is the
			// issue or chapter in a reading library.
			r.Chapter, _ = strconv.Atoi(l)
			markAt(i)
		case !reading && i > 0 && isNumber(l) && len(l) <= 4 && l != "0" && r.Episodes == nil && r.Season == 0 && !isYearToken(l):
			// "Title 05 [1080p]": a bare number after the title is an
			// absolute episode only when tags follow it.
			if followedByTags(toks[i+1:]) || i == len(toks)-1 && len(tags) > 0 {
				n, _ := strconv.Atoi(l)
				r.Episodes, r.Absolute = []int{n}, true
				markAt(i)
			}
		}
		// Words after the first marker can still carry tags.
		if i >= titleEnd {
			applyTag(l, &r)
		}
	}
	for _, tg := range tags {
		for _, w := range strings.Fields(strings.ToLower(strings.NewReplacer("-", " ", ",", " ", "_", " ").Replace(tg))) {
			applyTag(w, &r)
		}
		if l := strings.ToLower(tg); l == "batch" || l == "complete" {
			if len(r.Episodes) == 0 {
				r.SeasonPack = true
			}
		}
	}
	if reading && r.Volume == 0 && r.Chapter == 0 && r.Season > 0 {
		r.Volume = r.Season
	}

	title := strings.Join(toks[:titleEnd], " ")
	title = strings.Trim(title, " -–:.")
	if r.Year > 0 && titleEnd > 0 {
		title = strings.TrimSuffix(strings.TrimSpace(title), "(")
	}
	r.Title = cleanTitle(title)
	if r.Title == "" && r.Year > 0 {
		// "1917 (2019)": the "year" that ended the title was the title.
		r.Title = strconv.Itoa(r.Year)
		r.Year = 0
	}
	return r
}

var mediaExts = map[string]bool{
	"mkv": true, "mp4": true, "avi": true, "m4v": true, "webm": true, "mov": true, "ts": true,
	"cbz": true, "cbr": true, "cb7": true, "torrent": true, "nzb": true,
}

func stripExt(name string) string {
	name = filepath.Base(strings.ReplaceAll(name, "\\", "/"))
	for {
		ext := strings.ToLower(strings.TrimPrefix(filepath.Ext(name), "."))
		if !mediaExts[ext] {
			return name
		}
		name = strings.TrimSuffix(name, filepath.Ext(name))
	}
}

// pullBrackets removes [..] and (..) groups, returning their contents.
// A parenthesized year stays in the text so it can mark a movie.
func pullBrackets(s string) (string, []string) {
	var b strings.Builder
	var tags []string
	for i := 0; i < len(s); i++ {
		c := s[i]
		if c != '[' && c != '(' && c != '{' {
			b.WriteByte(c)
			continue
		}
		close := map[byte]byte{'[': ']', '(': ')', '{': '}'}[c]
		end := strings.IndexByte(s[i+1:], close)
		if end < 0 {
			b.WriteByte(' ')
			continue
		}
		inner := s[i+1 : i+1+end]
		if c == '(' && isYearToken(inner) {
			b.WriteString(" " + inner + " ")
		} else {
			tags = append(tags, inner)
			b.WriteByte(' ')
		}
		i += end + 1
	}
	return b.String(), tags
}

// dotsToSpaces turns scene separators into spaces but keeps decimals
// ("5.1", "H.264") and abbreviations' internal dots.
func dotsToSpaces(s string) string {
	b := []byte(s)
	for i := range b {
		if b[i] != '.' {
			continue
		}
		// A decimal is one digit, dot, one digit: "5.1", "2.0" (audio).
		// "E02.1080p" is a separator between two numbers.
		digitsAround := i > 0 && i < len(b)-1 && isDigit(b[i-1]) && isDigit(b[i+1]) &&
			(i < 2 || !isDigit(b[i-2])) && (i+2 >= len(b) || !isDigit(b[i+2]))
		codecDot := i > 0 && i < len(b)-1 && (b[i-1] == 'H' || b[i-1] == 'h') && isDigit(b[i+1])
		if !digitsAround && !codecDot {
			b[i] = ' '
		}
	}
	return string(b)
}

func parseSeasonEpisodes(l string, r *contracts.Release) bool {
	// SxxEyy[Ezz|-Ezz|-zz]...
	if len(l) < 4 || l[0] != 's' {
		return false
	}
	i := 1
	for i < len(l) && isDigit(l[i]) {
		i++
	}
	if i == 1 || i >= len(l) || l[i] != 'e' || i > 4 {
		return false
	}
	season, _ := strconv.Atoi(l[1:i])
	var eps []int
	rest := l[i:]
	for rest != "" {
		isRange := rest[0] == '-'
		if rest[0] == 'e' || isRange {
			rest = strings.TrimPrefix(strings.TrimPrefix(rest, "-"), "e")
		} else {
			return false
		}
		j := 0
		for j < len(rest) && isDigit(rest[j]) {
			j++
		}
		if j == 0 || j > 4 {
			return false
		}
		n, _ := strconv.Atoi(rest[:j])
		if len(eps) > 0 && isRange && n > eps[len(eps)-1] && n-eps[len(eps)-1] < 500 {
			for k := eps[len(eps)-1] + 1; k <= n; k++ {
				eps = append(eps, k)
			}
		} else {
			eps = append(eps, n)
		}
		rest = rest[j:]
	}
	r.Season, r.Episodes, r.SeasonPack = season, eps, false
	return true
}

// parseAbsolute reads "05", "05v2", "01-12", "1100".
func parseAbsolute(s string) ([]int, int, bool) {
	ver := 0
	if i := strings.IndexByte(s, 'v'); i > 0 && isNumber(s[i+1:]) {
		ver, _ = strconv.Atoi(s[i+1:])
		s = s[:i]
	}
	if a, b, ok := strings.Cut(s, "-"); ok && isNumber(a) && isNumber(b) {
		x, _ := strconv.Atoi(a)
		y, _ := strconv.Atoi(b)
		if y < x || y-x > 2000 {
			return nil, 0, false
		}
		eps := make([]int, 0, y-x+1)
		for k := x; k <= y; k++ {
			eps = append(eps, k)
		}
		return eps, ver, true
	}
	if isNumber(s) && len(s) <= 4 && !isYearToken(s) {
		n, _ := strconv.Atoi(s)
		return []int{n}, ver, true
	}
	return nil, 0, false
}

var resolutions = map[string]string{
	"2160p": "2160p", "4k": "2160p", "uhd": "2160p", "1080p": "1080p", "1080i": "1080p",
	"720p": "720p", "576p": "576p", "540p": "540p", "480p": "480p",
}

var sources = map[string]string{
	"bluray": "bluray", "blu-ray": "bluray", "bdrip": "bluray", "brrip": "bluray", "bd": "bluray", "bdremux": "bluray", "remux": "bluray",
	"web": "web", "webdl": "web", "web-dl": "web", "webrip": "web", "amzn": "web", "nf": "web",
	"hdtv": "hdtv", "dvd": "dvd", "dvdrip": "dvd", "dvd5": "dvd", "dvd9": "dvd",
}

var codecs = map[string]string{
	"x264": "h264", "h264": "h264", "h.264": "h264", "avc": "h264",
	"x265": "h265", "h265": "h265", "h.265": "h265", "hevc": "h265",
	"av1": "av1", "xvid": "xvid",
}

var otherTech = map[string]bool{
	"10bit": true, "8bit": true, "hi10p": true, "hdr": true, "hdr10": true, "dv": true, "dolby": true, "vision": true,
	"aac": true, "ac3": true, "eac3": true, "flac": true, "opus": true, "dts": true, "truehd": true, "atmos": true, "ddp5.1": true,
	"dual-audio": true, "dual": true, "audio": true, "multi": true, "subs": true, "multisub": true, "dubbed": true, "subbed": true,
	"proper": true, "repack": true, "extended": true, "uncut": true, "remastered": true, "dl": true, "internal": true,
}

func isTechTag(l string) bool {
	l = strings.ToLower(l)
	_, a := resolutions[l]
	_, b := sources[l]
	_, c := codecs[l]
	return a || b || c || otherTech[l] || strings.HasPrefix(l, "ddp") || strings.HasPrefix(l, "aac2")
}

func applyTag(l string, r *contracts.Release) {
	if v, ok := resolutions[l]; ok && r.Resolution == "" {
		r.Resolution = v
	}
	if v, ok := sources[l]; ok && r.Source == "" {
		r.Source = v
	}
	if v, ok := codecs[l]; ok && r.Codec == "" {
		r.Codec = v
	}
	switch l {
	case "proper":
		r.Proper = true
	case "repack", "rerip":
		r.Repack = true
	}
}

func followedByTags(rest []string) bool {
	for _, t := range rest {
		l := strings.ToLower(t)
		if isTechTag(l) {
			return true
		}
		if l != "-" {
			return false
		}
	}
	return false
}

// trailingNumber reports whether only years and tags follow.
func trailingNumber(rest []string) bool {
	for _, t := range rest {
		l := strings.ToLower(t)
		if !isYearToken(l) && !isTechTag(l) && l != "-" {
			return false
		}
	}
	return true
}

func isYearToken(s string) bool {
	s = strings.Trim(s, "()")
	if len(s) != 4 || !isNumber(s) {
		return false
	}
	n, _ := strconv.Atoi(s)
	return n >= 1900 && n <= 2100
}

func isDigit(c byte) bool { return c >= '0' && c <= '9' }

func isNumber(s string) bool {
	if s == "" {
		return false
	}
	for i := 0; i < len(s); i++ {
		if !isDigit(s[i]) {
			return false
		}
	}
	return true
}

func lastWord(w []string) string { return strings.ToLower(w[len(w)-1]) }

// cleanTitle collapses whitespace and drops control characters.
func cleanTitle(s string) string {
	s = strings.Map(func(r rune) rune {
		if unicode.IsControl(r) {
			return -1
		}
		return r
	}, s)
	s = strings.Join(strings.Fields(s), " ")
	if len(s) > 200 {
		s = s[:200]
	}
	return s
}

// TitleKey is the catalog's grouping key, re-exported for matching.
func TitleKey(title string) string { return catalog.TitleKey(title) }
