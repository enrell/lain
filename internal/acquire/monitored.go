package acquire

import (
	"path/filepath"
	"sort"
	"strings"

	"github.com/enrell/lain/internal/contracts"
)

// Monitored titles and what they want (A-17, A-18, A-24). Everything
// here is pure: what the library has is read from catalog items handed
// in, never written (D-008).

// Numbering modes.
const (
	NumberAbsolute = "absolute" // anime: one running episode count
	NumberSeasonal = "seasonal" // SxxEyy
	NumberChapter  = "chapter"  // manga/comics by chapter or issue
	NumberVolume   = "volume"   // manga/comics by volume
	NumberMovie    = "movie"
)

// Unit is one wanted thing: an episode (Season 0 = absolute), a
// chapter or volume (Season 0), or a movie ({0,1}). Number 0 with a
// Season is a whole-season pack marker (length unknown).
type Unit struct {
	Season int `json:"season"`
	Number int `json:"number"`
}

// SeasonStart says season Season begins at absolute episode First.
type SeasonStart struct {
	Season int `json:"season"`
	First  int `json:"first"`
}

// SeasonMap converts between absolute and seasonal numbering; split
// cours are simply further seasons (A-24).
type SeasonMap []SeasonStart

// Validate requires ascending seasons and strictly increasing starts.
func (m SeasonMap) Validate() error {
	for i, s := range m {
		if s.Season < 1 || s.First < 1 {
			return errf(CodeInvalid, "season map entries need season ≥ 1 and first episode ≥ 1")
		}
		if i > 0 && (s.Season <= m[i-1].Season || s.First <= m[i-1].First) {
			return errf(CodeInvalid, "season map must ascend: each season starts after the previous one")
		}
	}
	if len(m) > 100 {
		return errf(CodeInvalid, "at most 100 seasons")
	}
	return nil
}

func (m SeasonMap) index(season int) int {
	for i, s := range m {
		if s.Season == season {
			return i
		}
	}
	return -1
}

// ToAbsolute maps SxxEyy to an absolute episode.
func (m SeasonMap) ToAbsolute(season, ep int) (int, bool) {
	i := m.index(season)
	if i < 0 || ep < 1 {
		return 0, false
	}
	abs := m[i].First + ep - 1
	if i+1 < len(m) && abs >= m[i+1].First {
		return 0, false // past the season's end
	}
	return abs, true
}

// ToSeasonal maps an absolute episode to SxxEyy.
func (m SeasonMap) ToSeasonal(abs int) (int, int, bool) {
	for i := len(m) - 1; i >= 0; i-- {
		if abs >= m[i].First {
			return m[i].Season, abs - m[i].First + 1, true
		}
	}
	return 0, 0, false
}

// SeasonLength is known for every season but the last.
func (m SeasonMap) SeasonLength(season int) (int, bool) {
	i := m.index(season)
	if i < 0 || i+1 >= len(m) {
		return 0, false
	}
	return m[i+1].First - m[i].First, true
}

// SeasonWant monitors one season: episodes From..To (To 0 = ongoing).
type SeasonWant struct {
	Season int `json:"season"`
	From   int `json:"from"`
	To     int `json:"to,omitempty"`
}

// Monitored is a title automation keeps complete.
type Monitored struct {
	ID        string    `json:"id"`
	LibraryID string    `json:"library_id"`
	Kind      string    `json:"kind"` // the library type
	Title     string    `json:"title"`
	Aliases   []string  `json:"aliases"`
	Year      int       `json:"year,omitempty"`
	ProfileID string    `json:"profile_id"`
	Numbering string    `json:"numbering"`
	SeasonMap SeasonMap `json:"season_map,omitempty"`
	// From/To bound absolute episodes, chapters or volumes (To 0 =
	// ongoing: anything newer than the highest present is wanted).
	From    int          `json:"from,omitempty"`
	To      int          `json:"to,omitempty"`
	Seasons []SeasonWant `json:"seasons,omitempty"`
	Enabled bool         `json:"enabled"`
	// LastSearchAt is the last automatic search for missing units.
	LastSearchAt int64 `json:"last_search_at,omitempty"`
	CreatedAt    int64 `json:"created_at"`
}

var numberingByKind = map[string][]string{
	"movie":             {NumberMovie},
	"series":            {NumberSeasonal},
	"anime":             {NumberAbsolute, NumberSeasonal},
	contracts.KindManga: {NumberChapter, NumberVolume},
	contracts.KindComic: {NumberChapter, NumberVolume},
}

// Validate normalizes m or explains why it cannot be saved.
func (m Monitored) Validate() (Monitored, error) {
	allowed, ok := numberingByKind[m.Kind]
	if !ok {
		return m, errf(CodeInvalid, "unknown kind %q", m.Kind)
	}
	if m.LibraryID == "" {
		return m, errf(CodeNoLibrary, "choose the library to keep complete")
	}
	t, err := cleanName(m.Title, 200)
	if err != nil {
		return m, err
	}
	if t == "" {
		return m, errf(CodeInvalid, "title required")
	}
	m.Title = t
	if len(m.Aliases) > 10 {
		return m, errf(CodeInvalid, "at most 10 aliases")
	}
	aliases := []string{}
	for _, a := range m.Aliases {
		a, err := cleanName(a, 200)
		if err != nil {
			return m, err
		}
		if a != "" && looseKey(a) != looseKey(m.Title) {
			aliases = append(aliases, a)
		}
	}
	m.Aliases = aliases
	if m.Numbering == "" {
		m.Numbering = allowed[0]
	}
	if rankOf(allowed, m.Numbering) < 0 {
		return m, errf(CodeInvalid, "a %s cannot use %s numbering", m.Kind, m.Numbering)
	}
	if err := m.SeasonMap.Validate(); err != nil {
		return m, err
	}
	if m.From < 0 || m.To < 0 || (m.To > 0 && m.To < m.From) {
		return m, errf(CodeInvalid, "the range must run from a lower number to a higher one")
	}
	if m.From == 0 && m.Numbering != NumberMovie && m.Numbering != NumberSeasonal {
		m.From = 1
	}
	seen := map[int]bool{}
	for i, s := range m.Seasons {
		if s.Season < 1 || seen[s.Season] || s.From < 0 || (s.To > 0 && s.To < s.From) {
			return m, errf(CodeInvalid, "each monitored season needs a number ≥ 1, once, and a valid range")
		}
		if s.From == 0 {
			m.Seasons[i].From = 1
		}
		seen[s.Season] = true
	}
	sort.Slice(m.Seasons, func(i, j int) bool { return m.Seasons[i].Season < m.Seasons[j].Season })
	if m.Year < 0 || m.Year > 3000 {
		return m, errf(CodeInvalid, "bad year")
	}
	return m, nil
}

// names are every key the title answers to.
func (m Monitored) names() map[string]bool {
	out := map[string]bool{looseKey(m.Title): true}
	for _, a := range m.Aliases {
		out[looseKey(a)] = true
	}
	return out
}

// unitOf reads a present catalog item as a unit for this title.
func (m Monitored) unitOf(it contracts.CatalogItem) (Unit, bool) {
	switch m.Numbering {
	case NumberMovie:
		return Unit{0, 1}, true
	case NumberChapter:
		return Unit{0, it.Episode}, it.Episode > 0
	case NumberVolume:
		return Unit{0, it.Season}, it.Season > 0
	case NumberSeasonal:
		if it.Season == 0 && it.Episode > 0 {
			if s, e, ok := m.SeasonMap.ToSeasonal(it.Episode); ok {
				return Unit{s, e}, true
			}
		}
		return Unit{it.Season, it.Episode}, it.Season > 0 && it.Episode > 0
	default: // absolute
		if it.Season > 0 && it.Episode > 0 {
			if abs, ok := m.SeasonMap.ToAbsolute(it.Season, it.Episode); ok {
				return Unit{0, abs}, true
			}
		}
		return Unit{0, it.Episode}, it.Episode > 0
	}
}

// UpgradeUnit is a present file below the profile cutoff.
type UpgradeUnit struct {
	Unit    Unit   `json:"unit"`
	Quality string `json:"quality"`
	Path    string `json:"path"`
}

// Wanted is what a monitored title still needs.
type Wanted struct {
	Missing []Unit `json:"missing"`
	// OpenFrom: for ongoing absolute/chapter/volume titles, every number
	// from here on is wanted.
	OpenFrom *Unit `json:"open_from,omitempty"`
	// OpenSeasons: for ongoing seasons, episodes from this number on.
	OpenSeasons map[int]int   `json:"open_seasons,omitempty"`
	Upgrades    []UpgradeUnit `json:"upgrades"`
	Present     int           `json:"present"`
}

// QualityOf tells the resolution of a library file ("" = unknown).
type QualityOf func(path string) string

// FileQuality reads the resolution from a file name.
func FileQuality(parse Parser, kind string) QualityOf {
	return func(p string) string { return parse(filepath.Base(p), kind).Resolution }
}

// Wanted compares the title's range with the library's catalog items.
// quality tells each present file's resolution, for upgrades (A-21).
func (m Monitored) Wanted(items []contracts.CatalogItem, p Profile, quality QualityOf) Wanted {
	names := m.names()
	present := map[Unit]contracts.CatalogItem{}
	for _, it := range items {
		if it.Missing || !names[looseKey(it.Title)] {
			continue
		}
		if u, ok := m.unitOf(it); ok {
			present[u] = it
		}
	}
	w := Wanted{Missing: []Unit{}, Upgrades: []UpgradeUnit{}, Present: len(present)}
	want := func(u Unit) {
		if _, ok := present[u]; !ok {
			w.Missing = append(w.Missing, u)
		}
	}
	switch m.Numbering {
	case NumberMovie:
		want(Unit{0, 1})
	case NumberSeasonal:
		for _, s := range m.Seasons {
			top := s.To
			if top == 0 {
				top = s.From - 1
				for u := range present {
					if u.Season == s.Season && u.Number > top {
						top = u.Number
					}
				}
				if w.OpenSeasons == nil {
					w.OpenSeasons = map[int]int{}
				}
				w.OpenSeasons[s.Season] = top + 1
			}
			for n := s.From; n <= top; n++ {
				want(Unit{s.Season, n})
			}
		}
	default:
		top := m.To
		if top == 0 {
			top = m.From - 1
			for u := range present {
				if u.Number > top {
					top = u.Number
				}
			}
			w.OpenFrom = &Unit{0, top + 1}
		}
		for n := m.From; n <= top; n++ {
			want(Unit{0, n})
		}
	}
	sort.Slice(w.Missing, func(i, j int) bool { return less(w.Missing[i], w.Missing[j]) })
	if m.Numbering != NumberChapter && m.Numbering != NumberVolume {
		// Reading releases carry no resolution: no upgrades for them.
		for u, it := range present {
			q := quality(it.FilePath)
			if !MeetsCutoff(p, q) {
				w.Upgrades = append(w.Upgrades, UpgradeUnit{Unit: u, Quality: q, Path: it.FilePath})
			}
		}
		sort.Slice(w.Upgrades, func(i, j int) bool { return less(w.Upgrades[i].Unit, w.Upgrades[j].Unit) })
	}
	return w
}

func less(a, b Unit) bool {
	if a.Season != b.Season {
		return a.Season < b.Season
	}
	return a.Number < b.Number
}

// Needs reports whether u is wanted (missing or past an open edge). A
// season-pack marker {s,0} is needed when anything in season s is.
func (w Wanted) Needs(u Unit) bool {
	for _, x := range w.Missing {
		if x == u || (u.Number == 0 && u.Season > 0 && x.Season == u.Season) {
			return true
		}
	}
	if w.OpenFrom != nil && u.Season == w.OpenFrom.Season && u.Number >= w.OpenFrom.Number {
		return true
	}
	if from, ok := w.OpenSeasons[u.Season]; ok && (u.Number == 0 || u.Number >= from) {
		return true
	}
	return false
}

// Upgradable returns the present file of u when it is below cutoff.
func (w Wanted) Upgradable(u Unit) (UpgradeUnit, bool) {
	for _, x := range w.Upgrades {
		if x.Unit == u {
			return x, true
		}
	}
	return UpgradeUnit{}, false
}

// Covers lists the units a parsed release provides for this title, or
// nil when it is another title or cannot be placed.
func (m Monitored) Covers(rel contracts.Release) []Unit {
	if !m.names()[looseKey(rel.Title)] {
		return nil
	}
	switch m.Numbering {
	case NumberMovie:
		if m.Year > 0 && rel.Year > 0 && m.Year != rel.Year {
			return nil
		}
		return []Unit{{0, 1}}
	case NumberChapter:
		if rel.Chapter > 0 {
			return []Unit{{0, rel.Chapter}}
		}
		return nil
	case NumberVolume:
		if rel.Volume > 0 && rel.Chapter == 0 {
			return []Unit{{0, rel.Volume}}
		}
		return nil
	case NumberSeasonal:
		var out []Unit
		switch {
		case len(rel.Episodes) > 0 && (rel.Absolute || rel.Season == 0):
			for _, e := range rel.Episodes {
				s, n, ok := m.SeasonMap.ToSeasonal(e)
				if !ok {
					return nil
				}
				out = append(out, Unit{s, n})
			}
		case len(rel.Episodes) > 0:
			for _, e := range rel.Episodes {
				out = append(out, Unit{rel.Season, e})
			}
		case rel.SeasonPack && rel.Season > 0:
			out = []Unit{{rel.Season, 0}}
		}
		return out
	default: // absolute
		var out []Unit
		switch {
		case len(rel.Episodes) > 0 && rel.Season > 0 && !rel.Absolute:
			for _, e := range rel.Episodes {
				abs, ok := m.SeasonMap.ToAbsolute(rel.Season, e)
				if !ok {
					return nil
				}
				out = append(out, Unit{0, abs})
			}
		case len(rel.Episodes) > 0:
			for _, e := range rel.Episodes {
				out = append(out, Unit{0, e})
			}
		case rel.SeasonPack && rel.Season > 0:
			first, ok1 := m.SeasonMap.ToAbsolute(rel.Season, 1)
			n, ok2 := m.SeasonMap.SeasonLength(rel.Season)
			if !ok1 || !ok2 {
				return nil
			}
			for e := first; e < first+n; e++ {
				out = append(out, Unit{0, e})
			}
		}
		return out
	}
}

// keyOf is a monitored title's identity inside one library.
func keyOf(libraryID, title string) string {
	return libraryID + "\x00" + strings.TrimSpace(looseKey(title))
}
