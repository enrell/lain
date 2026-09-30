package identify

import (
	"path/filepath"
	"regexp"
	"strconv"
	"strings"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

// ComicID is the built-in comic/manga identifier (D-085).
const ComicID = "lain-identify-comic"

var (
	reGroup   = regexp.MustCompile(`[\[\(][^\]\)]*[\]\)]`)
	reYear    = regexp.MustCompile(`\b(19|20)\d{2}\b`)
	reVolume  = regexp.MustCompile(`(?i)(?:^|[\s._-])(?:vol(?:ume)?\.?|v)[\s._]*0*(\d{1,4})\b`)
	reChapter = regexp.MustCompile(`(?i)(?:^|[\s._-])(?:ch(?:apter)?\.?|c|ep(?:isode)?\.?|issue|no\.?|#)[\s._]*0*(\d{1,4})(?:\.\d+)?\b`)
	reHash    = regexp.MustCompile(`#[\s._]*0*(\d{1,4})\b`)
	reTrail   = regexp.MustCompile(`[\s._-]0*(\d{1,4})$`)
	reBareNum = regexp.MustCompile(`^0*(\d{1,4})$`)
)

// Comic identifies cbz/cbr/cb7 archives. It declines every other
// extension so the anime parser never sees an archive. Mapping (D-085):
// the series is the title, a volume is Season and a chapter or issue
// number is Episode, each 0 when the file does not carry it — so a
// volume file (S3E0), a chapter file (S0E45) and "Vol 2 Ch 15" (S2E15)
// stay distinguishable. Kind follows the library type.
type Comic struct{}

func (Comic) ID() string             { return ComicID }
func (Comic) Capabilities() []string { return []string{contracts.CapMediaIdentify} }
func (Comic) Health() error          { return nil }

func (Comic) Invoke(cap string, input any) (any, error) {
	if cap != contracts.CapMediaIdentify {
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
	c, ok := input.(contracts.Candidate)
	if !ok {
		return nil, &core.Error{Code: "invalid-message", Msg: "Candidate required"}
	}
	return IdentifyComic(c), nil
}

// IdentifyComic returns a zero Proposal (declined) for non-archives.
func IdentifyComic(c contracts.Candidate) contracts.Proposal {
	base := filepath.Base(c.Path)
	ext := strings.ToLower(strings.TrimPrefix(filepath.Ext(base), "."))
	if ext != "cbz" && ext != "cbr" && ext != "cb7" {
		return contracts.Proposal{}
	}
	kind := contracts.KindComic
	if strings.EqualFold(c.LibraryType, contracts.KindManga) {
		kind = contracts.KindManga
	}
	stem := strings.TrimSuffix(base, filepath.Ext(base))

	year := 0
	for _, g := range reGroup.FindAllString(stem, -1) {
		if y := reYear.FindString(g); y != "" {
			year, _ = strconv.Atoi(y)
			break
		}
	}
	name := strings.Join(strings.Fields(reGroup.ReplaceAllString(stem, " ")), " ")
	name = strings.NewReplacer("_", " ").Replace(name)
	if !strings.Contains(name, " ") {
		name = strings.ReplaceAll(name, ".", " ")
	}

	var vol, ch int
	evidence := []string{"extension"}
	cut := len(name)
	if m := reVolume.FindStringSubmatchIndex(name); m != nil {
		vol, _ = strconv.Atoi(name[m[2]:m[3]])
		cut = min(cut, m[0])
		evidence = append(evidence, "volume")
	}
	if m := reChapter.FindStringSubmatchIndex(name); m != nil {
		ch, _ = strconv.Atoi(name[m[2]:m[3]])
		cut = min(cut, m[0])
		evidence = append(evidence, "chapter")
	} else if m := reHash.FindStringSubmatchIndex(name); m != nil {
		ch, _ = strconv.Atoi(name[m[2]:m[3]])
		cut = min(cut, m[0])
		evidence = append(evidence, "issue")
	}
	if vol == 0 && ch == 0 {
		if m := reTrail.FindStringSubmatchIndex(name); m != nil {
			ch, _ = strconv.Atoi(name[m[2]:m[3]])
			cut = m[0]
			evidence = append(evidence, "trailing-number")
		}
	}
	title := cleanComicTitle(name[:cut])
	if title == "" || reBareNum.MatchString(title) {
		// "Series/Vol 03.cbz": the folder names the series.
		if dir := filepath.Base(filepath.Dir(c.Path)); dir != "." && dir != "/" && dir != string(filepath.Separator) {
			title = cleanComicTitle(dir)
			evidence = append(evidence, "folder")
		}
	}
	if title == "" {
		title = cleanTitle(stem)
	}

	season, episode := vol, ch
	conf := 0.5
	if vol > 0 || ch > 0 {
		conf = 0.85
	}
	return contracts.Proposal{
		Kind: kind, Title: title, Season: season, Episode: episode, Year: year,
		Confidence: conf, Evidence: evidence, PluginID: ComicID,
	}
}

func cleanComicTitle(s string) string {
	s = strings.Trim(strings.Join(strings.Fields(s), " "), " -–—_.:,")
	return cleanTitle(s)
}
