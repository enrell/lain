package acquire

import (
	"fmt"
	"math"
	"strings"

	"github.com/enrell/lain/internal/contracts"
)

// Quality profiles and the one decision engine (A-19, A-20). Every
// candidate — manual search, RSS, automatic search — is judged by
// Evaluate; automation only grabs accepted ones, best score first.

// Known resolutions, best first.
var knownResolutions = []string{"2160p", "1080p", "720p", "576p", "540p", "480p"}

var knownSources = map[string]bool{"bluray": true, "web": true, "hdtv": true, "dvd": true}

// Profile is a quality policy.
type Profile struct {
	ID   string `json:"id"`
	Name string `json:"name"`
	// Resolutions are the allowed ones, best first; a release whose
	// resolution is unknown or not listed is rejected.
	Resolutions []string `json:"resolutions"`
	// Sources limits release sources ("bluray", "web", "hdtv", "dvd");
	// empty allows any, including unstated.
	Sources []string `json:"sources"`
	// Cutoff is the resolution at which upgrades stop.
	Cutoff          string   `json:"cutoff"`
	PreferredGroups []string `json:"preferred_groups"`
	BlockedGroups   []string `json:"blocked_groups"`
	BlockedWords    []string `json:"blocked_words"`
	MinSeeders      int      `json:"min_seeders"`
	// MinSizeMB and MaxSizeMB bound the size per unit (episode, chapter
	// or movie); 0 disables a bound.
	MinSizeMB    int  `json:"min_size_mb"`
	MaxSizeMB    int  `json:"max_size_mb"`
	PreferProper bool `json:"prefer_proper"`
}

// DefaultProfile is created on first use: 720p or better, upgrade to
// 1080p, at least one seeder.
func DefaultProfile() Profile {
	return Profile{
		ID: "default", Name: "Default (720p+, upgrade to 1080p)",
		Resolutions: []string{"2160p", "1080p", "720p"}, Sources: []string{}, Cutoff: "1080p",
		PreferredGroups: []string{}, BlockedGroups: []string{}, BlockedWords: []string{},
		MinSeeders: 1, PreferProper: true,
	}
}

func cleanList(xs []string, max int) []string {
	out := []string{}
	seen := map[string]bool{}
	for _, x := range xs {
		x = strings.TrimSpace(x)
		if x == "" || seen[strings.ToLower(x)] || len(out) >= max {
			continue
		}
		seen[strings.ToLower(x)] = true
		out = append(out, x)
	}
	return out
}

// Validate normalizes p or explains why it cannot be saved.
func (p Profile) Validate() (Profile, error) {
	name, err := cleanName(p.Name, 60)
	if err != nil {
		return p, err
	}
	if name == "" {
		return p, errf(CodeInvalid, "profile name required")
	}
	p.Name = name
	seen := map[string]bool{}
	for i, r := range p.Resolutions {
		r = strings.ToLower(strings.TrimSpace(r))
		if rankOf(knownResolutions, r) < 0 {
			return p, errf(CodeInvalid, "unknown resolution %q", r)
		}
		if seen[r] {
			return p, errf(CodeInvalid, "resolution %s listed twice", r)
		}
		seen[r] = true
		p.Resolutions[i] = r
	}
	if len(p.Resolutions) == 0 {
		return p, errf(CodeInvalid, "allow at least one resolution")
	}
	p.Cutoff = strings.ToLower(strings.TrimSpace(p.Cutoff))
	if p.Cutoff == "" {
		p.Cutoff = p.Resolutions[0]
	}
	if !seen[p.Cutoff] {
		return p, errf(CodeInvalid, "the cutoff must be one of the allowed resolutions")
	}
	p.Sources = cleanList(p.Sources, 10)
	for i, s := range p.Sources {
		s = strings.ToLower(s)
		if !knownSources[s] {
			return p, errf(CodeInvalid, "unknown source %q", s)
		}
		p.Sources[i] = s
	}
	p.PreferredGroups = cleanList(p.PreferredGroups, 50)
	p.BlockedGroups = cleanList(p.BlockedGroups, 50)
	p.BlockedWords = cleanList(p.BlockedWords, 50)
	if p.MinSeeders < 0 || p.MinSizeMB < 0 || p.MaxSizeMB < 0 {
		return p, errf(CodeInvalid, "limits cannot be negative")
	}
	if p.MaxSizeMB > 0 && p.MinSizeMB > p.MaxSizeMB {
		return p, errf(CodeInvalid, "minimum size is above the maximum")
	}
	return p, nil
}

func rankOf(list []string, v string) int {
	for i, x := range list {
		if strings.EqualFold(x, v) {
			return i
		}
	}
	return -1
}

// Decision is the verdict on one candidate.
type Decision struct {
	Accepted   bool     `json:"accepted"`
	Rejections []string `json:"rejections,omitempty"`
	Score      int      `json:"score"`
	Quality    string   `json:"quality"`
}

func hasGroup(groups []string, rel contracts.Release, title string) (string, bool) {
	lt := strings.ToLower(title)
	for _, g := range groups {
		lg := strings.ToLower(g)
		if strings.EqualFold(rel.Group, g) || strings.HasSuffix(lt, "-"+lg) || strings.Contains(lt, "["+lg+"]") {
			return g, true
		}
	}
	return "", false
}

// Evaluate judges a candidate against a profile. units is how many
// episodes, chapters or movies the release covers (size bounds are per
// unit); values below 1 count as 1.
func Evaluate(p Profile, r contracts.SearchResult, rel contracts.Release, units int) Decision {
	d := Decision{Quality: rel.Resolution}
	reject := func(format string, args ...any) { d.Rejections = append(d.Rejections, fmt.Sprintf(format, args...)) }
	if units < 1 {
		units = 1
	}
	if r.Protocol == contracts.ProtocolUsenet {
		reject("usenet: no usenet client yet")
	}
	rank := rankOf(p.Resolutions, rel.Resolution)
	switch {
	case rel.Resolution == "":
		reject("unknown resolution")
	case rank < 0:
		reject("resolution %s not allowed", rel.Resolution)
	}
	if len(p.Sources) > 0 && rankOf(p.Sources, rel.Source) < 0 {
		src := rel.Source
		if src == "" {
			src = "unstated"
		}
		reject("source %s not allowed", src)
	}
	if g, ok := hasGroup(p.BlockedGroups, rel, r.Title); ok {
		reject("blocked group %s", g)
	}
	lt := strings.ToLower(r.Title)
	for _, w := range p.BlockedWords {
		if containsWord(lt, strings.ToLower(w)) {
			reject("blocked word %q", w)
		}
	}
	if r.Protocol == contracts.ProtocolTorrent && r.Seeders < max(p.MinSeeders, 0) {
		reject("too few seeders (%d < %d)", r.Seeders, p.MinSeeders)
	}
	if r.Size > 0 {
		per := r.Size / int64(units) >> 20
		if p.MinSizeMB > 0 && per < int64(p.MinSizeMB) {
			reject("too small: %d MB per unit, minimum %d", per, p.MinSizeMB)
		}
		if p.MaxSizeMB > 0 && per > int64(p.MaxSizeMB) {
			reject("too large: %d MB per unit, maximum %d", per, p.MaxSizeMB)
		}
	}
	if rank >= 0 {
		d.Score += (len(p.Resolutions) - rank) * 100
	}
	if _, ok := hasGroup(p.PreferredGroups, rel, r.Title); ok {
		d.Score += 150
	}
	if p.PreferProper && (rel.Proper || rel.Repack || rel.Version > 1) {
		d.Score += 20
	}
	d.Score += int(5 * math.Log2(float64(max(r.Seeders, 0)+1)))
	if r.Freeleech {
		d.Score += 2
	}
	d.Accepted = len(d.Rejections) == 0
	return d
}

// containsWord matches w as a whole token in s (separators: anything
// that is not a letter or digit), so "cam" does not hit "camera".
func containsWord(s, w string) bool {
	if w == "" {
		return false
	}
	for i := 0; ; {
		j := strings.Index(s[i:], w)
		if j < 0 {
			return false
		}
		j += i
		before := j == 0 || !isAlnum(s[j-1])
		after := j+len(w) == len(s) || !isAlnum(s[j+len(w)])
		if before && after {
			return true
		}
		i = j + 1
	}
}

func isAlnum(c byte) bool {
	return c >= 'a' && c <= 'z' || c >= '0' && c <= '9' || c >= 'A' && c <= 'Z'
}

// MeetsCutoff reports a quality at or above the profile cutoff.
func MeetsCutoff(p Profile, res string) bool {
	r, c := rankOf(p.Resolutions, res), rankOf(p.Resolutions, p.Cutoff)
	return r >= 0 && c >= 0 && r <= c
}

// IsUpgrade reports whether cand is worth grabbing over have: have is
// below the cutoff (unknown counts as worst) and cand is allowed and
// strictly better.
func IsUpgrade(p Profile, have, cand string) bool {
	if MeetsCutoff(p, have) {
		return false
	}
	c := rankOf(p.Resolutions, cand)
	if c < 0 {
		return false
	}
	h := rankOf(p.Resolutions, have)
	return h < 0 || c < h
}
