package acquire

import (
	"errors"
	"fmt"
	"io"
	"math"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/kv"
	"github.com/enrell/lain/internal/subtitle"
)

// Phase 3: subtitle acquisition (docs/slices/acquisition.md, A-30…A-36).
// Providers are searched through lain.subtitle@1; the core fetches the
// file, checks it against the media (A-33), normalizes it to UTF-8
// (A-34) and writes it next to the media without overwriting (A-35).

// ---- provider accounts ----

// SubtitleProviderInput creates or patches an account; nil fields stay.
// An empty secret keeps the stored one, "-" clears it.
type SubtitleProviderInput struct {
	Name     *string `json:"name"`
	Kind     *string `json:"kind"`
	BaseURL  *string `json:"base_url"`
	APIKey   *string `json:"api_key"`
	Username *string `json:"username"`
	Password *string `json:"password"`
	Enabled  *bool   `json:"enabled"`
	Priority *int    `json:"priority"`
}

func patchSecret(dst *string, in *string) error {
	if in == nil {
		return nil
	}
	switch v := strings.TrimSpace(*in); v {
	case "":
	case "-":
		*dst = ""
	default:
		if len(v) > 512 {
			return errf(CodeInvalid, "credential too long")
		}
		*dst = v
	}
	return nil
}

func (in SubtitleProviderInput) apply(p *contracts.SubtitleProvider) error {
	if in.Name != nil {
		n, err := cleanName(*in.Name, 60)
		if err != nil {
			return err
		}
		p.Name = n
	}
	if in.Kind != nil {
		p.Kind = *in.Kind
	}
	if in.BaseURL != nil {
		p.BaseURL = strings.TrimSpace(*in.BaseURL)
	}
	if in.Username != nil {
		u, err := cleanName(*in.Username, 100)
		if err != nil {
			return err
		}
		p.Username = u
	}
	for _, s := range []struct {
		dst *string
		in  *string
	}{{&p.APIKey, in.APIKey}, {&p.Password, in.Password}} {
		if err := patchSecret(s.dst, s.in); err != nil {
			return err
		}
	}
	if in.Enabled != nil {
		p.Enabled = *in.Enabled
	}
	if in.Priority != nil {
		p.Priority = *in.Priority
	}
	if p.Name == "" {
		return errf(CodeInvalid, "name required")
	}
	if p.Kind != contracts.SubtitleOpenSubtitles {
		return errf(CodeInvalid, "kind must be %s", contracts.SubtitleOpenSubtitles)
	}
	if p.BaseURL != "" {
		u, err := url.Parse(p.BaseURL)
		if err != nil || (u.Scheme != "http" && u.Scheme != "https") || u.Host == "" || u.User != nil {
			return errf(CodeInvalid, "base_url must be http(s) without credentials")
		}
	}
	if p.APIKey == "" {
		return errf(CodeInvalid, "an API key is required")
	}
	return nil
}

// SubtitleProviders lists accounts without secrets.
func (m *Manager) SubtitleProviders() []contracts.SubtitleProvider {
	out := listJSON[contracts.SubtitleProvider](m.st, kv.BAcqSubProviders)
	sort.SliceStable(out, func(i, j int) bool { return out[i].Priority < out[j].Priority })
	for i := range out {
		out[i] = out[i].Public()
	}
	return out
}

// SubtitleProviderSecrets reports which secrets an account has stored.
func (m *Manager) SubtitleProviderSecrets(id string) (apiKey, password bool) {
	p, err := getJSON[contracts.SubtitleProvider](m.st, kv.BAcqSubProviders, id, "subtitle provider")
	return err == nil && p.APIKey != "", err == nil && p.Password != ""
}

// CreateSubtitleProvider adds an account (enabled).
func (m *Manager) CreateSubtitleProvider(in SubtitleProviderInput) (contracts.SubtitleProvider, error) {
	p := contracts.SubtitleProvider{ID: newID(), Enabled: true, CreatedAt: time.Now().Unix()}
	if err := in.apply(&p); err != nil {
		return p, err
	}
	return p.Public(), putJSON(m.st, kv.BAcqSubProviders, p.ID, p)
}

// UpdateSubtitleProvider patches an account.
func (m *Manager) UpdateSubtitleProvider(id string, in SubtitleProviderInput) (contracts.SubtitleProvider, error) {
	p, err := getJSON[contracts.SubtitleProvider](m.st, kv.BAcqSubProviders, id, "subtitle provider")
	if err != nil {
		return p, err
	}
	if err := in.apply(&p); err != nil {
		return p, err
	}
	return p.Public(), putJSON(m.st, kv.BAcqSubProviders, id, p)
}

// DeleteSubtitleProvider removes an account and its secrets.
func (m *Manager) DeleteSubtitleProvider(id string) error {
	return deleteKey(m.st, kv.BAcqSubProviders, id, "subtitle provider")
}

// ---- what a file needs ----

func (m *Manager) probe(path string) contracts.MediaInfo {
	if m.d.Probe == nil {
		return contracts.MediaInfo{}
	}
	info, err := m.d.Probe(path)
	if err != nil {
		m.log.Info("probe failed; subtitle checks run without stream data", "path", path, "err", err.Error())
		return contracts.MediaInfo{}
	}
	return info
}

// MissingSubtitles lists the profile's languages a file still lacks: a
// language counts as present through an embedded subtitle stream, a
// full (not forced) sidecar, or — unless SubtitleEvenWithAudio — an
// audio stream in that language (A-32, D-071).
func (m *Manager) MissingSubtitles(path string, p Profile) []string {
	if len(p.SubtitleLanguages) == 0 {
		return []string{}
	}
	have := map[string]bool{}
	for _, s := range m.probe(path).Streams {
		lang := subtitle.Language(s.Language)
		if lang == "" {
			continue
		}
		if s.Type == "subtitle" && !s.Forced {
			have[lang] = true
		}
		if s.Type == "audio" && !p.SubtitleEvenWithAudio {
			have[lang] = true
		}
	}
	for _, sc := range subtitle.Discover(path) {
		if !sc.Forced {
			have[sc.Language] = true
		}
	}
	out := []string{}
	for _, l := range p.SubtitleLanguages {
		if !have[l] {
			out = append(out, l)
		}
	}
	return out
}

// ---- searching and ranking ----

// SubtitleTarget describes the media a subtitle is for.
type SubtitleTarget struct {
	Path    string `json:"-"`
	Kind    string `json:"kind"`
	Title   string `json:"title"`
	Season  int    `json:"season,omitempty"`
	Episode int    `json:"episode,omitempty"`
	Year    int    `json:"year,omitempty"`
}

// SubtitleChoice is a candidate with its verdict (manual search shows
// rejected ones with their reasons, like releases, D-127).
type SubtitleChoice struct {
	contracts.SubtitleCandidate
	Accepted   bool     `json:"accepted"`
	Rejections []string `json:"rejections,omitempty"`
	Score      int      `json:"score"`
}

func subBlockKey(providerID, fileID string) string { return providerID + "\x00" + fileID }

// SearchSubtitles asks every enabled provider and ranks the answers:
// hash match first, then release group, then resolution/source, then
// popularity (A-31).
func (m *Manager) SearchSubtitles(t SubtitleTarget, p Profile, languages []string) ([]SubtitleChoice, error) {
	if m.d.SubtitleSearch == nil {
		return nil, errf(CodeUnsupported, "no subtitle provider is available")
	}
	providers := listJSON[contracts.SubtitleProvider](m.st, kv.BAcqSubProviders)
	sort.SliceStable(providers, func(i, j int) bool { return providers[i].Priority < providers[j].Priority })
	var enabled []contracts.SubtitleProvider
	for _, pr := range providers {
		if pr.Enabled {
			enabled = append(enabled, pr)
		}
	}
	if len(enabled) == 0 {
		return nil, errf(CodeNotFound, "no enabled subtitle provider; add one under Settings › Acquisition")
	}
	hash, _ := subtitle.MovieHash(t.Path)
	var size int64
	if fi, err := os.Stat(t.Path); err == nil {
		size = fi.Size()
	}
	file := m.d.Parse(filepath.Base(t.Path), t.Kind)
	wanted := map[string]bool{}
	for _, l := range languages {
		wanted[l] = true
	}
	var out []SubtitleChoice
	var lastErr error
	for _, pr := range enabled {
		cands, err := m.d.SubtitleSearch(contracts.SubtitleSearchInput{
			Provider: pr, Hash: hash, Size: size, Query: t.Title, Movie: t.Kind == "movie",
			Year: t.Year, Season: t.Season, Episode: t.Episode, Languages: languages,
		})
		if err != nil {
			lastErr = err
			m.log.Info("subtitle search failed", "provider", pr.ID, "err", err.Error())
			continue
		}
		for _, c := range cands {
			if c.ProviderID == "" {
				c.ProviderID = pr.ID
			}
			out = append(out, m.judgeSubtitle(c, p, wanted, t, file))
		}
	}
	if out == nil && lastErr != nil {
		return nil, lastErr
	}
	sort.SliceStable(out, func(i, j int) bool {
		if out[i].Accepted != out[j].Accepted {
			return out[i].Accepted
		}
		if !out[i].Accepted {
			return false // rejected ones keep provider order
		}
		return out[i].Score > out[j].Score
	})
	if out == nil {
		out = []SubtitleChoice{}
	}
	return out, nil
}

func (m *Manager) judgeSubtitle(c contracts.SubtitleCandidate, p Profile, wanted map[string]bool, t SubtitleTarget, file contracts.Release) SubtitleChoice {
	ch := SubtitleChoice{SubtitleCandidate: c}
	reject := func(format string, args ...any) { ch.Rejections = append(ch.Rejections, fmt.Sprintf(format, args...)) }
	if !wanted[c.Language] {
		reject("language %s not wanted", c.Language)
	}
	if c.Forced {
		reject("forced (foreign parts only)")
	}
	if c.HI && p.SubtitleHI == HIExclude {
		reject("hearing-impaired excluded by the profile")
	}
	if t.Episode > 0 && c.Episode > 0 && c.Episode != t.Episode {
		reject("for episode %d, not %d", c.Episode, t.Episode)
	}
	if t.Season > 0 && c.Season > 0 && c.Season != t.Season {
		reject("for season %d, not %d", c.Season, t.Season)
	}
	if _, err := getJSON[string](m.st, kv.BAcqSubBlock, subBlockKey(c.ProviderID, c.FileID), "refused subtitle"); err == nil {
		reject("refused earlier by the sync check")
	}
	if c.HashMatch {
		ch.Score += 1000
	}
	if c.Release != "" {
		rel := m.d.Parse(c.Release, t.Kind)
		if file.Group != "" && strings.EqualFold(rel.Group, file.Group) {
			ch.Score += 100
		}
		if file.Resolution != "" && rel.Resolution == file.Resolution {
			ch.Score += 20
		}
		if file.Source != "" && rel.Source == file.Source {
			ch.Score += 20
		}
	}
	if c.HI && p.SubtitleHI == HIPrefer {
		ch.Score += 30
	}
	ch.Score += int(5 * math.Log2(float64(max(c.Downloads, 0)+1)))
	ch.Accepted = len(ch.Rejections) == 0
	return ch
}

// ---- downloading and placing ----

// SubtitleRecord is a sidecar Lain wrote (the A-35 ledger).
type SubtitleRecord struct {
	Path       string `json:"path"`
	MediaPath  string `json:"media_path"`
	ProviderID string `json:"provider_id"`
	FileID     string `json:"file_id"`
	Language   string `json:"language"`
	HashMatch  bool   `json:"hash_match,omitempty"`
	At         int64  `json:"at"`
}

// SubtitleLedger lists sidecars Lain wrote, newest first.
func (m *Manager) SubtitleLedger() []SubtitleRecord {
	out := listJSON[SubtitleRecord](m.st, kv.BAcqSubtitles)
	sort.SliceStable(out, func(i, j int) bool { return out[i].At > out[j].At })
	return out
}

// syncCheck applies A-33 against the media's duration (seconds; 0 =
// unknown, the check is skipped).
func syncCheck(st subtitle.Stats, duration float64) error {
	if duration <= 0 {
		return nil
	}
	d := time.Duration(duration * float64(time.Second))
	limit := max(time.Duration(float64(d)*1.10), d+2*time.Minute)
	switch {
	case st.Last > limit:
		return errf(CodeSubtitleMismatch, "the subtitle runs to %s but the video is %s long (another cut, episode or frame rate)", st.Last.Round(time.Second), d.Round(time.Second))
	case st.First > d:
		return errf(CodeSubtitleMismatch, "the subtitle starts after the video ends")
	}
	return nil
}

func formatOf(name string) string {
	switch ext := strings.ToLower(strings.TrimPrefix(filepath.Ext(name), ".")); ext {
	case "srt", "ass", "ssa", "vtt":
		return ext
	}
	return "srt"
}

// DownloadSubtitle fetches a candidate, checks it and writes it next to
// mediaPath. An existing sidecar of the same name is kept unless
// replace is set, in which case it is moved to the holding folder
// (D-128) — never deleted.
func (m *Manager) DownloadSubtitle(mediaPath string, c contracts.SubtitleCandidate, replace bool) (SubtitleRecord, error) {
	if m.d.SubtitleDownload == nil {
		return SubtitleRecord{}, errf(CodeUnsupported, "no subtitle provider is available")
	}
	if !m.inLibrary(mediaPath) {
		return SubtitleRecord{}, errf(CodeInvalid, "subtitles are only written next to library files")
	}
	lang := subtitle.Language(c.Language)
	if lang == "" {
		return SubtitleRecord{}, errf(CodeInvalid, "unknown subtitle language")
	}
	pr, err := getJSON[contracts.SubtitleProvider](m.st, kv.BAcqSubProviders, c.ProviderID, "subtitle provider")
	if err != nil {
		return SubtitleRecord{}, err
	}
	link, err := m.d.SubtitleDownload(contracts.SubtitleDownloadInput{Provider: pr, FileID: c.FileID})
	if err != nil {
		return SubtitleRecord{}, errf(CodeFetch, "%v", err)
	}
	raw, err := m.fetchSubtitle(link.Link)
	if err != nil {
		return SubtitleRecord{}, err
	}
	refuse := func(err error) (SubtitleRecord, error) {
		_ = putJSON(m.st, kv.BAcqSubBlock, subBlockKey(c.ProviderID, c.FileID), err.Error())
		return SubtitleRecord{}, err
	}
	cues, err := subtitle.Parse(raw)
	if err != nil {
		return refuse(errf(CodeSubtitleMismatch, "the file holds no readable subtitles"))
	}
	if err := syncCheck(subtitle.Measure(cues), m.probe(mediaPath).Duration); err != nil {
		return refuse(err)
	}
	name := c.FileName
	if name == "" {
		name = link.FileName
	}
	dst := subtitle.SidecarPath(mediaPath, lang, c.Region, c.Forced, c.HI, formatOf(name))
	if _, err := os.Lstat(dst); err == nil {
		if !replace {
			return SubtitleRecord{}, errf(CodeState, "%s already exists", filepath.Base(dst))
		}
		if err := m.holdFile(dst, "subtitles"); err != nil {
			return SubtitleRecord{}, err
		}
	}
	if err := writeNew(dst, subtitle.DecodeText(raw)); err != nil {
		return SubtitleRecord{}, err
	}
	rec := SubtitleRecord{Path: dst, MediaPath: mediaPath, ProviderID: c.ProviderID, FileID: c.FileID, Language: lang, HashMatch: c.HashMatch, At: time.Now().Unix()}
	if err := putJSON(m.st, kv.BAcqSubtitles, dst, rec); err != nil {
		m.log.Warn("subtitle ledger write failed", "path", dst, "err", err.Error())
	}
	m.log.Info("subtitle placed", "path", dst, "provider", c.ProviderID, "hash_match", c.HashMatch)
	return rec, nil
}

func (m *Manager) inLibrary(p string) bool {
	for _, root := range m.libraryRoots() {
		if within(root, p) {
			return true
		}
	}
	return false
}

func (m *Manager) fetchSubtitle(link string) ([]byte, error) {
	u, err := url.Parse(link)
	if err != nil || (u.Scheme != "http" && u.Scheme != "https") {
		return nil, errf(CodeFetch, "the provider gave no usable link")
	}
	res, err := m.d.HTTP.Get(link)
	if err != nil {
		return nil, errf(CodeFetch, "%s did not answer", u.Host)
	}
	defer res.Body.Close()
	if res.StatusCode != http.StatusOK {
		return nil, errf(CodeFetch, "%s answered %d", u.Host, res.StatusCode)
	}
	raw, err := io.ReadAll(io.LimitReader(res.Body, subtitle.MaxBytes+1))
	if err != nil || len(raw) > subtitle.MaxBytes {
		return nil, errf(CodeFetch, "the subtitle from %s is unreadable or too large", u.Host)
	}
	return raw, nil
}

// holdFile moves a library file into <dir>/replaced/<group>/ (D-128).
func (m *Manager) holdFile(p, group string) error {
	dst := filepath.Join(m.holdDir(), group, filepath.Base(p))
	for i := 1; ; i++ {
		if _, err := os.Lstat(dst); os.IsNotExist(err) {
			break
		}
		dst = filepath.Join(m.holdDir(), group, filepath.Base(p)+"."+itoa(i))
	}
	_, err := Place(p, dst, ImportMove)
	return err
}

// writeNew writes data at dst without ever replacing an existing file.
func writeNew(dst string, data []byte) error {
	tmp := dst + ".lain-sub"
	f, err := os.OpenFile(tmp, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o644)
	if err != nil {
		return errf(CodeImport, "create: %v", err)
	}
	_, werr := f.Write(data)
	if cerr := f.Close(); werr == nil {
		werr = cerr
	}
	if werr != nil {
		_ = os.Remove(tmp)
		return errf(CodeImport, "write: %v", werr)
	}
	if err := os.Link(tmp, dst); err != nil {
		_ = os.Remove(tmp)
		if errors.Is(err, os.ErrExist) {
			return errf(CodeState, "%s already exists", filepath.Base(dst))
		}
		return errf(CodeImport, "place: %v", err)
	}
	return os.Remove(tmp)
}

// ---- automation ----

// FileSubtitles is one file of a monitored title and its subtitle state.
type FileSubtitles struct {
	ItemID   string             `json:"item_id"`
	Path     string             `json:"path"`
	Missing  []string           `json:"missing"`
	Sidecars []subtitle.Sidecar `json:"sidecars"`
}

func (m *Manager) titleFiles(mon Monitored) []contracts.CatalogItem {
	names := mon.names()
	var out []contracts.CatalogItem
	for _, it := range m.items(mon.LibraryID) {
		if !it.Missing && names[looseKey(it.Title)] && videoExts[strings.ToLower(filepath.Ext(it.FilePath))] {
			out = append(out, it)
		}
	}
	sort.Slice(out, func(i, j int) bool { return out[i].FilePath < out[j].FilePath })
	return out
}

// SubtitleStatus reports each video file of a monitored title.
func (m *Manager) SubtitleStatus(id string) ([]FileSubtitles, error) {
	mon, err := m.Monitored(id)
	if err != nil {
		return nil, err
	}
	p, err := m.profile(mon.ProfileID)
	if err != nil {
		return nil, err
	}
	out := []FileSubtitles{}
	for _, it := range m.titleFiles(mon) {
		sc := subtitle.Discover(it.FilePath)
		if sc == nil {
			sc = []subtitle.Sidecar{}
		}
		out = append(out, FileSubtitles{ItemID: it.ID, Path: it.FilePath, Missing: m.MissingSubtitles(it.FilePath, p), Sidecars: sc})
	}
	return out, nil
}

// SubtitlePass fetches the best accepted subtitle for every missing
// language of the given files (A-36). It returns the sidecars written.
func (m *Manager) SubtitlePass(mon Monitored, files []contracts.CatalogItem) int {
	p, err := m.profile(mon.ProfileID)
	if err != nil || len(p.SubtitleLanguages) == 0 || m.d.SubtitleSearch == nil {
		return 0
	}
	written := 0
	for _, it := range files {
		missing := m.MissingSubtitles(it.FilePath, p)
		if len(missing) == 0 {
			continue
		}
		t := SubtitleTarget{Path: it.FilePath, Kind: mon.Kind, Title: mon.Title, Season: it.Season, Episode: it.Episode, Year: mon.Year}
		choices, err := m.SearchSubtitles(t, p, missing)
		if err != nil {
			m.log.Info("subtitle pass skipped a file", "path", it.FilePath, "err", err.Error())
			continue
		}
		done := map[string]bool{}
		for _, ch := range choices {
			if !ch.Accepted || done[ch.Language] {
				continue
			}
			if _, err := m.DownloadSubtitle(it.FilePath, ch.SubtitleCandidate, false); err != nil {
				m.log.Info("subtitle not placed", "path", it.FilePath, "file", ch.FileID, "err", err.Error())
				continue
			}
			done[ch.Language] = true
			written++
		}
	}
	mon.LastSubtitleAt = time.Now().Unix()
	_ = putJSON(m.st, kv.BAcqMonitored, mon.ID, mon)
	return written
}

// SubtitlesForMonitored runs a pass over all of a title's files now.
func (m *Manager) SubtitlesForMonitored(id string) (int, error) {
	mon, err := m.Monitored(id)
	if err != nil {
		return 0, err
	}
	return m.SubtitlePass(mon, m.titleFiles(mon)), nil
}

// subtitlesAfterImport runs a pass over freshly imported files in the
// background (the library rescan has catalogued them by then).
func (m *Manager) subtitlesAfterImport(g Grab) {
	if g.MonitoredID == "" || len(g.Imported) == 0 || m.d.SubtitleSearch == nil {
		return
	}
	mon, err := m.Monitored(g.MonitoredID)
	if err != nil {
		return
	}
	paths := map[string]bool{}
	for _, p := range g.Imported {
		paths[p] = true
	}
	var files []contracts.CatalogItem
	for _, it := range m.titleFiles(mon) {
		if paths[it.FilePath] {
			files = append(files, it)
		}
	}
	if len(files) == 0 {
		// Not catalogued yet (a slow rescan): fall back to the paths, read
		// the numbers from the names.
		for p := range paths {
			r := m.d.Parse(filepath.Base(p), mon.Kind)
			it := contracts.CatalogItem{FilePath: p, Season: r.Season}
			if len(r.Episodes) > 0 {
				it.Episode = r.Episodes[0]
			}
			files = append(files, it)
		}
	}
	m.SubtitlePass(mon, files)
}

func subtitleDue(s Settings, mon Monitored, now time.Time) bool {
	return s.Automation && mon.Enabled && s.SubtitleHours > 0 &&
		now.Sub(time.Unix(mon.LastSubtitleAt, 0)) >= time.Duration(s.SubtitleHours)*time.Hour
}

// ProfileForItem is the profile governing a catalog item: its monitored
// title's (same library, same title) or the default profile.
func (m *Manager) ProfileForItem(it contracts.CatalogItem) (Profile, error) {
	for _, mon := range m.st.monitored() {
		if mon.LibraryID == it.LibraryID && mon.names()[looseKey(it.Title)] {
			return m.profile(mon.ProfileID)
		}
	}
	return m.profile("")
}
