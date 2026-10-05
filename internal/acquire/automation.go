package acquire

import (
	"context"
	"io/fs"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/kv"
)

// Phase 2 automation (docs/slices/acquisition.md, A-17…A-25):
// monitored titles, quality profiles, RSS sync, scheduled and
// on-demand searches, upgrades, the blocklist and stall detection.

// ---- profiles ----

// Profiles lists quality profiles; the default one always exists.
func (m *Manager) Profiles() []Profile {
	ps := m.st.profiles()
	for _, p := range ps {
		if p.ID == "default" {
			return ps
		}
	}
	def := DefaultProfile()
	_ = putJSON(m.st, kv.BAcqProfiles, def.ID, def)
	return m.st.profiles()
}

func (m *Manager) profile(id string) (Profile, error) {
	if id == "" || id == "default" {
		m.Profiles()
		id = "default"
	}
	return getJSON[Profile](m.st, kv.BAcqProfiles, id, "profile")
}

// CreateProfile adds a quality profile.
func (m *Manager) CreateProfile(p Profile) (Profile, error) {
	p.ID = newID()
	p, err := p.Validate()
	if err != nil {
		return p, err
	}
	return p, putJSON(m.st, kv.BAcqProfiles, p.ID, p)
}

// UpdateProfile replaces a profile's policy.
func (m *Manager) UpdateProfile(id string, p Profile) (Profile, error) {
	if _, err := m.profile(id); err != nil {
		return p, err
	}
	p.ID = id
	p, err := p.Validate()
	if err != nil {
		return p, err
	}
	return p, putJSON(m.st, kv.BAcqProfiles, id, p)
}

// DeleteProfile removes an unused profile; the default one stays.
func (m *Manager) DeleteProfile(id string) error {
	if id == "default" {
		return errf(CodeState, "the default profile cannot be deleted")
	}
	for _, mon := range m.st.monitored() {
		if mon.ProfileID == id {
			return errf(CodeState, "%s uses this profile", mon.Title)
		}
	}
	return deleteKey(m.st, kv.BAcqProfiles, id, "profile")
}

// ---- monitored titles ----

// MonitoredList lists monitored titles.
func (m *Manager) MonitoredList() []Monitored { return m.st.monitored() }

func (m *Manager) checkMonitored(mon Monitored, selfID string) (Monitored, error) {
	mon, err := mon.Validate()
	if err != nil {
		return mon, err
	}
	lib, ok := m.d.Library(mon.LibraryID)
	if !ok {
		return mon, errf(CodeNoLibrary, "choose the library to keep complete")
	}
	if lib.Type != mon.Kind {
		return mon, errf(CodeInvalid, "%s is a %s library, not %s", lib.Name, lib.Type, mon.Kind)
	}
	if mon.ProfileID == "" {
		mon.ProfileID = "default"
	}
	if _, err := m.profile(mon.ProfileID); err != nil {
		return mon, err
	}
	for _, other := range m.st.monitored() {
		if other.ID != selfID && other.LibraryID == mon.LibraryID && other.names()[looseKey(mon.Title)] {
			return mon, errf(CodeState, "%s is already monitored in this library", other.Title)
		}
	}
	return mon, nil
}

// CreateMonitored starts keeping a title complete.
func (m *Manager) CreateMonitored(mon Monitored) (Monitored, error) {
	mon, err := m.checkMonitored(mon, "")
	if err != nil {
		return mon, err
	}
	mon.ID, mon.CreatedAt, mon.Enabled = newID(), time.Now().Unix(), true
	return mon, putJSON(m.st, kv.BAcqMonitored, mon.ID, mon)
}

// UpdateMonitored replaces a monitored title's settings.
func (m *Manager) UpdateMonitored(id string, mon Monitored) (Monitored, error) {
	old, err := getJSON[Monitored](m.st, kv.BAcqMonitored, id, "monitored title")
	if err != nil {
		return mon, err
	}
	mon, err = m.checkMonitored(mon, id)
	if err != nil {
		return mon, err
	}
	mon.ID, mon.CreatedAt, mon.LastSearchAt = id, old.CreatedAt, old.LastSearchAt
	if mon.MetadataEpisodes == 0 {
		mon.MetadataEpisodes = old.MetadataEpisodes
	}
	return mon, putJSON(m.st, kv.BAcqMonitored, id, mon)
}

// Monitored returns one monitored title.
func (m *Manager) Monitored(id string) (Monitored, error) {
	return getJSON[Monitored](m.st, kv.BAcqMonitored, id, "monitored title")
}

// DeleteMonitored stops monitoring; grabs and files are untouched.
func (m *Manager) DeleteMonitored(id string) error {
	return deleteKey(m.st, kv.BAcqMonitored, id, "monitored title")
}

func (m *Manager) items(libraryID string) []contracts.CatalogItem {
	if m.d.Items == nil {
		return nil
	}
	return m.d.Items(libraryID)
}

// WantedFor computes what a monitored title still needs.
func (m *Manager) WantedFor(id string) (Wanted, error) {
	mon, err := getJSON[Monitored](m.st, kv.BAcqMonitored, id, "monitored title")
	if err != nil {
		return Wanted{}, err
	}
	p, err := m.profile(mon.ProfileID)
	if err != nil {
		return Wanted{}, err
	}
	return mon.Wanted(m.items(mon.LibraryID), p, m.qualityOf(mon.Kind)), nil
}

// qualityOf reads a library file's resolution from the import ledger
// first, then from its name.
func (m *Manager) qualityOf(kind string) QualityOf {
	byName := FileQuality(m.d.Parse, kind)
	return func(p string) string {
		if q, err := getJSON[string](m.st, kv.BAcqQuality, p, "quality"); err == nil {
			return q
		}
		return byName(p)
	}
}

// ---- blocklist ----

// Blocklist lists blocked releases, newest first.
func (m *Manager) Blocklist() []BlockEntry { return m.st.blocklist() }

// Unblock removes one entry.
func (m *Manager) Unblock(id string) error {
	return deleteKey(m.st, kv.BAcqBlocklist, id, "blocklist entry")
}

// block records a failed grab's release (A-22).
func (m *Manager) block(g Grab, reason string) {
	if g.InfoHash == "" && g.Title == "" {
		return
	}
	for _, e := range m.st.blocklist() {
		if (g.InfoHash != "" && e.InfoHash == g.InfoHash) || strings.EqualFold(e.Title, g.Title) {
			return
		}
	}
	e := BlockEntry{ID: newID(), InfoHash: g.InfoHash, Title: g.Title, IndexerID: g.IndexerID, MonitoredID: g.MonitoredID, Reason: clipText(reason, 300), At: time.Now().Unix()}
	if err := putJSON(m.st, kv.BAcqBlocklist, e.ID, e); err != nil {
		m.log.Warn("blocklist write failed", "grab", g.ID, "err", err.Error())
	}
}

func blocked(list []BlockEntry, r contracts.SearchResult) bool {
	for _, e := range list {
		if (r.InfoHash != "" && strings.EqualFold(e.InfoHash, r.InfoHash)) || strings.EqualFold(e.Title, r.Title) {
			return true
		}
	}
	return false
}

// ---- stall detection ----

type mark struct {
	bytes int64
	at    time.Time
}

func (m *Manager) stallWindow(s Settings) time.Duration {
	if m.d.StallAfter > 0 {
		return m.d.StallAfter
	}
	return time.Duration(s.StallHours) * time.Hour
}

// stalled reports a download whose completed bytes have not moved for
// the stall window (A-22). The window starts when Lain first sees the
// grab, so a restart never fails a download on the spot.
func (m *Manager) stalled(g Grab, st ClientStatus, s Settings, now time.Time) bool {
	window := m.stallWindow(s)
	if window <= 0 {
		return false
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.marks == nil {
		m.marks = map[string]mark{}
	}
	mk, ok := m.marks[g.ID]
	if !ok || st.Completed != mk.bytes {
		m.marks[g.ID] = mark{bytes: st.Completed, at: now}
		return false
	}
	return now.Sub(mk.at) >= window
}

// ---- searching ----

// Report is what one automation pass did.
type Report struct {
	Grabbed    []string `json:"grabbed"`
	Considered int      `json:"considered"`
	Rejected   int      `json:"rejected"`
	// Paused is set when the budget stopped the pass (A-25).
	Paused   string           `json:"paused,omitempty"`
	Failures []IndexerFailure `json:"failures,omitempty"`
}

// AutomationState is shown in the UI.
type AutomationState struct {
	Enabled   bool   `json:"enabled"`
	Paused    string `json:"paused,omitempty"`
	LastRSSAt int64  `json:"last_rss_at,omitempty"`
}

// AutomationStatus reports the automation state.
func (m *Manager) AutomationStatus() AutomationState {
	m.mu.Lock()
	defer m.mu.Unlock()
	st := AutomationState{Enabled: m.settings.Automation, Paused: m.autoPaused}
	if !m.lastRSS.IsZero() {
		st.LastRSSAt = m.lastRSS.Unix()
	}
	return st
}

// query asks every enabled indexer; an empty text is an RSS read.
func (m *Manager) query(text, kind string) ([]contracts.SearchResult, []IndexerFailure) {
	ctx, cancel := context.WithTimeout(context.Background(), 45*time.Second)
	defer cancel()
	var out []contracts.SearchResult
	var fails []IndexerFailure
	for _, ix := range m.st.indexers() {
		if !ix.Enabled {
			continue
		}
		err := m.wait(ctx, ix)
		var res []contracts.SearchResult
		if err == nil {
			in := contracts.IndexerSearchInput{Indexer: ix, Query: text, Limit: 100}
			if text != "" {
				in.Kind = searchKind(kind)
			}
			res, err = m.d.Search(in)
		}
		if err != nil {
			fails = append(fails, IndexerFailure{IndexerID: ix.ID, Name: ix.Name, Error: err.Error()})
			continue
		}
		out = append(out, res...)
	}
	return out, fails
}

type option struct {
	r       contracts.SearchResult
	score   int
	covers  []Unit
	replace []string
}

// consider picks the releases worth grabbing for one monitored title
// and grabs them, best first, never twice for the same unit.
func (m *Manager) consider(mon Monitored, results []contracts.SearchResult, rep *Report) {
	p, err := m.profile(mon.ProfileID)
	if err != nil {
		return
	}
	w := mon.Wanted(m.items(mon.LibraryID), p, m.qualityOf(mon.Kind))
	taken := map[Unit]bool{}
	for _, g := range m.st.grabs() {
		if g.MonitoredID != mon.ID {
			continue
		}
		switch g.State {
		case GrabQueued, GrabMetadata, GrabDownloading, GrabImporting, GrabPaused:
			for _, u := range g.Units {
				taken[u] = true
			}
		}
	}
	bl := m.st.blocklist()
	var opts []option
	for _, r := range results {
		rel := m.d.Parse(r.Title, mon.Kind)
		covers := mon.Covers(rel)
		if len(covers) == 0 {
			continue
		}
		rep.Considered++
		var needed int
		var replace []string
		for _, u := range covers {
			if taken[u] {
				continue
			}
			if w.Needs(u) {
				needed++
			} else if up, ok := w.Upgradable(u); ok && IsUpgrade(p, up.Quality, rel.Resolution) {
				replace = append(replace, up.Path)
			}
		}
		if needed == 0 && len(replace) == 0 {
			continue
		}
		if blocked(bl, r) {
			rep.Rejected++
			continue
		}
		units := len(covers)
		if units == 1 && covers[0].Number == 0 {
			units = 12 // a season pack of unknown length: a typical cour
		}
		d := Evaluate(p, r, rel, units)
		if !d.Accepted {
			rep.Rejected++
			continue
		}
		opts = append(opts, option{r: r, score: d.Score, covers: covers, replace: replace})
	}
	sort.SliceStable(opts, func(i, j int) bool { return opts[i].score > opts[j].score })
	for _, o := range opts {
		fresh := false
		for _, u := range o.covers {
			if !taken[u] {
				fresh = true
			}
		}
		if !fresh {
			continue
		}
		r := o.r
		g, err := m.Grab(GrabInput{Result: &r, LibraryID: mon.LibraryID, MonitoredID: mon.ID, Units: o.covers, ReplaceFrom: o.replace})
		if err != nil {
			switch CodeOf(err) {
			case CodeQuota, CodeDiskFull:
				rep.Paused = "download budget full: " + err.(*Error).Msg
				m.setPaused(rep.Paused)
				return
			}
			m.log.Info("automatic grab skipped", "monitored", mon.ID, "err", err.Error())
			continue
		}
		m.setPaused("")
		for _, u := range o.covers {
			taken[u] = true
		}
		rep.Grabbed = append(rep.Grabbed, g.ID)
	}
}

func (m *Manager) setPaused(reason string) {
	m.mu.Lock()
	m.autoPaused = reason
	m.mu.Unlock()
}

// SearchMonitored searches indexers for one monitored title now.
func (m *Manager) SearchMonitored(id string) (Report, error) {
	mon, err := getJSON[Monitored](m.st, kv.BAcqMonitored, id, "monitored title")
	if err != nil {
		return Report{}, err
	}
	rep := Report{Grabbed: []string{}}
	if mon.Numbering == NumberAbsolute && m.d.EpisodeCount != nil {
		if n := m.d.EpisodeCount(mon.Title, mon.Kind); n > 0 && n < 100_000 {
			mon.MetadataEpisodes = n
		}
	}
	results, fails := m.query(mon.Title, mon.Kind)
	rep.Failures = fails
	m.consider(mon, results, &rep)
	mon.LastSearchAt = time.Now().Unix()
	_ = putJSON(m.st, kv.BAcqMonitored, mon.ID, mon)
	return rep, nil
}

// RSSSync reads every indexer's latest releases once and offers them
// to every enabled monitored title.
func (m *Manager) RSSSync() (Report, error) {
	rep := Report{Grabbed: []string{}}
	results, fails := m.query("", "")
	rep.Failures = fails
	for _, mon := range m.st.monitored() {
		if mon.Enabled {
			m.consider(mon, results, &rep)
			if rep.Paused != "" {
				break
			}
		}
	}
	m.mu.Lock()
	m.lastRSS = time.Now()
	m.mu.Unlock()
	return rep, nil
}

func rssDue(s Settings, last, now time.Time) bool {
	return s.Automation && (last.IsZero() || now.Sub(last) >= time.Duration(s.RSSMinutes)*time.Minute)
}

func searchDue(s Settings, mon Monitored, now time.Time) bool {
	return s.Automation && mon.Enabled && s.SearchHours > 0 &&
		now.Sub(time.Unix(mon.LastSearchAt, 0)) >= time.Duration(s.SearchHours)*time.Hour
}

// schedule runs due automation in the background, one pass at a time.
func (m *Manager) schedule(now time.Time) {
	m.mu.Lock()
	s, last, busy := m.settings, m.lastRSS, m.autoBusy
	if busy || !s.Automation {
		m.mu.Unlock()
		return
	}
	m.autoBusy = true
	m.mu.Unlock()
	m.wg.Add(1)
	go func() {
		defer m.wg.Done()
		defer func() {
			m.mu.Lock()
			m.autoBusy = false
			m.mu.Unlock()
		}()
		if rssDue(s, last, now) {
			if _, err := m.RSSSync(); err != nil {
				m.log.Warn("rss sync failed", "err", err.Error())
			}
			return
		}
		// One missing-search per pass spreads indexer load.
		for _, mon := range m.st.monitored() {
			if searchDue(s, mon, now) {
				_, _ = m.SearchMonitored(mon.ID)
				return
			}
		}
		// Then one subtitle pass per tick (A-36).
		for _, mon := range m.st.monitored() {
			if subtitleDue(s, mon, now) {
				_, _ = m.SubtitlesForMonitored(mon.ID)
				return
			}
		}
	}()
}

// ---- upgrades: the holding folder ----

type heldFile struct{ from, to string }

func (m *Manager) holdDir() string { return filepath.Join(m.Settings().Dir, "replaced") }

// holdReplaced moves the library files an upgrade replaces into
// <dir>/replaced/<grab>/, refusing anything outside the grab's library.
func (m *Manager) holdReplaced(g Grab, lib contracts.Library) ([]heldFile, error) {
	var held []heldFile
	for _, p := range g.ReplaceFrom {
		if !within(lib.Path, p) {
			m.restoreHeld(held)
			return nil, errf(CodeImport, "refusing to replace a file outside the library: %s", p)
		}
		if _, err := os.Lstat(p); err != nil {
			continue // already gone (an earlier attempt held it)
		}
		dst := filepath.Join(m.holdDir(), g.ID, filepath.Base(p))
		for i := 1; ; i++ {
			if _, err := os.Lstat(dst); os.IsNotExist(err) {
				break
			}
			dst = filepath.Join(m.holdDir(), g.ID, filepath.Base(p)+"."+itoa(i))
		}
		if _, err := Place(p, dst, ImportMove); err != nil {
			m.restoreHeld(held)
			return nil, err
		}
		held = append(held, heldFile{from: p, to: dst})
	}
	return held, nil
}

// restoreHeld puts held files back where their path is still free.
func (m *Manager) restoreHeld(held []heldFile) {
	for _, h := range held {
		if _, err := Place(h.to, h.from, ImportMove); err != nil {
			m.log.Warn("could not restore a replaced file; it stays held", "path", h.to, "err", err.Error())
		}
	}
}

// HeldFile is a library file an upgrade replaced.
type HeldFile struct {
	GrabID string `json:"grab_id"`
	Path   string `json:"path"`
	Size   int64  `json:"size"`
	At     int64  `json:"at"`
}

// Replaced lists held files. They are never deleted automatically.
func (m *Manager) Replaced() []HeldFile {
	out := []HeldFile{}
	root := m.holdDir()
	_ = filepath.WalkDir(root, func(p string, d fs.DirEntry, err error) error {
		if err != nil || d.IsDir() {
			return nil
		}
		info, err := d.Info()
		if err != nil {
			return nil
		}
		rel, _ := filepath.Rel(root, p)
		out = append(out, HeldFile{GrabID: strings.SplitN(rel, string(filepath.Separator), 2)[0], Path: p, Size: info.Size(), At: info.ModTime().Unix()})
		return nil
	})
	return out
}

// PurgeReplaced deletes every held file — an explicit admin action.
func (m *Manager) PurgeReplaced() (int, error) {
	n := 0
	for _, h := range m.Replaced() {
		if err := os.Remove(h.Path); err != nil {
			return n, errf(CodeImport, "could not delete %s: %v", h.Path, err)
		}
		n++
	}
	entries, _ := os.ReadDir(m.holdDir())
	for _, e := range entries {
		_ = os.Remove(filepath.Join(m.holdDir(), e.Name()))
	}
	return n, nil
}

func itoa(i int) string {
	if i == 0 {
		return "0"
	}
	var b []byte
	for ; i > 0; i /= 10 {
		b = append([]byte{byte('0' + i%10)}, b...)
	}
	return string(b)
}
