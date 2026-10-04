package acquire

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
)

// libraryItems plays the catalog: every file under the library root,
// read with the release tokenizer — what a rescan would record.
func (w *world) libraryItems(string) []contracts.CatalogItem {
	var out []contracts.CatalogItem
	_ = filepath.WalkDir(w.lib.Path, func(p string, d os.DirEntry, err error) error {
		if err != nil || d.IsDir() || !strings.HasSuffix(p, ".mkv") {
			return nil
		}
		r := parse(filepath.Base(p), w.lib.Type)
		ep := 0
		if len(r.Episodes) > 0 {
			ep = r.Episodes[0]
		}
		out = append(out, contracts.CatalogItem{ID: p, LibraryID: w.lib.ID, Title: r.Title, Season: r.Season, Episode: ep, FilePath: p})
		return nil
	})
	return out
}

func addIndexer(t *testing.T, m *Manager, w *world) contracts.Indexer {
	t.Helper()
	name, proto, key, url := "tracker-exemplo", contracts.IndexerTorznab, "k", w.index.URL
	ix, err := m.CreateIndexer(IndexerInput{Name: &name, Protocol: &proto, URL: &url, APIKey: &key})
	if err != nil {
		t.Fatal(err)
	}
	zero := 0
	if _, err := m.UpdateIndexer(ix.ID, IndexerInput{MinIntervalMs: &zero}); err != nil {
		t.Fatal(err)
	}
	return ix
}

func monitor(t *testing.T, m *Manager, w *world, mon Monitored) Monitored {
	t.Helper()
	mon.LibraryID, mon.Kind = w.lib.ID, w.lib.Type
	out, err := m.CreateMonitored(mon)
	if err != nil {
		t.Fatal(err)
	}
	return out
}

func TestSearchMonitoredGrabsUntilComplete(t *testing.T) {
	w := newWorld(t)
	m := w.manager(t, t.TempDir(), settings(0))
	addIndexer(t, m, w)
	mon := monitor(t, m, w, Monitored{Title: "Show", From: 1, To: 2})
	wanted, err := m.WantedFor(mon.ID)
	if err != nil || len(wanted.Missing) != 2 {
		t.Fatalf("wanted before: %+v %v", wanted, err)
	}
	rep, err := m.SearchMonitored(mon.ID)
	if err != nil || len(rep.Grabbed) != 1 {
		t.Fatalf("search: %+v %v", rep, err)
	}
	g := waitGrab(t, m, rep.Grabbed[0], GrabDone)
	if g.MonitoredID != mon.ID || len(g.Units) != 2 {
		t.Fatalf("grab must remember what it is for: %+v", g)
	}
	if wanted, _ := m.WantedFor(mon.ID); len(wanted.Missing) != 0 {
		t.Fatalf("still missing after import: %+v", wanted.Missing)
	}
	if rep, _ := m.SearchMonitored(mon.ID); len(rep.Grabbed) != 0 {
		t.Fatalf("a complete title must not grab again: %+v", rep)
	}
	// Clearing the queue must not make the imported files look unknown
	// (and so "upgradable") again: the quality ledger outlives grabs.
	if err := m.Remove(g.ID, true); err != nil {
		t.Fatal(err)
	}
	if rep, _ := m.SearchMonitored(mon.ID); len(rep.Grabbed) != 0 {
		t.Fatalf("grabbed again after the queue was cleared: %+v", rep)
	}
}

func TestSearchSkipsUnitsAlreadyBeingGrabbed(t *testing.T) {
	w := newWorld(t)
	s := settings(0)
	s.MaxActive = 1
	m := w.manager(t, t.TempDir(), s)
	addIndexer(t, m, w)
	mon := monitor(t, m, w, Monitored{Title: "Show", From: 1, To: 2})
	first, _ := m.SearchMonitored(mon.ID)
	second, _ := m.SearchMonitored(mon.ID)
	if len(first.Grabbed) != 1 || len(second.Grabbed) != 0 {
		t.Fatalf("double grab: %+v then %+v", first, second)
	}
}

func TestRSSSyncFeedsMonitoredTitles(t *testing.T) {
	w := newWorld(t)
	m := w.manager(t, t.TempDir(), settings(0))
	addIndexer(t, m, w)
	mon := monitor(t, m, w, Monitored{Title: "Show"}) // ongoing: anything from 1 on
	rep, err := m.RSSSync()
	if err != nil || len(rep.Grabbed) != 1 {
		t.Fatalf("rss: %+v %v", rep, err)
	}
	waitGrab(t, m, rep.Grabbed[0], GrabDone)
	w.assertImported(t)
	if wanted, _ := m.WantedFor(mon.ID); wanted.OpenFrom == nil || *wanted.OpenFrom != (Unit{0, 3}) {
		t.Fatalf("open edge after import: %+v", wanted.OpenFrom)
	}
}

func TestStalledGrabIsBlocklistedAndNotRetried(t *testing.T) {
	w := newWorld(t)
	w.seed.Close() // nobody seeds: the download never moves
	m := w.manager(t, t.TempDir(), settings(0))
	m.d.StallAfter = 400 * time.Millisecond
	addIndexer(t, m, w)
	mon := monitor(t, m, w, Monitored{Title: "Show", From: 1, To: 2})
	rep, _ := m.SearchMonitored(mon.ID)
	if len(rep.Grabbed) != 1 {
		t.Fatalf("search: %+v", rep)
	}
	g := waitGrabFailed(t, m, rep.Grabbed[0])
	if g.Code != CodeStalled || !g.DataRemoved {
		t.Fatalf("stalled grab: %+v", g)
	}
	bl := m.Blocklist()
	if len(bl) != 1 || bl[0].InfoHash != w.mi.InfoHash.Hex() || bl[0].MonitoredID != mon.ID {
		t.Fatalf("blocklist: %+v", bl)
	}
	again, _ := m.SearchMonitored(mon.ID)
	if len(again.Grabbed) != 0 {
		t.Fatalf("a blocklisted release was grabbed again: %+v", again)
	}
	// Clearing the entry makes the release eligible again.
	if err := m.Unblock(bl[0].ID); err != nil {
		t.Fatal(err)
	}
	if len(m.Blocklist()) != 0 {
		t.Fatal("unblock left the entry")
	}
}

func TestUpgradeReplacesBelowCutoffAndKeepsTheOldFile(t *testing.T) {
	w := newWorld(t)
	old := map[string]string{}
	for _, n := range []string{"[Fansub-B] SHOW - 01 [720p].mkv", "[Fansub-B] SHOW - 02 [720p].mkv"} {
		p := filepath.Join(w.lib.Path, "SHOW", n)
		_ = os.MkdirAll(filepath.Dir(p), 0o755)
		if err := os.WriteFile(p, []byte("old "+n), 0o644); err != nil {
			t.Fatal(err)
		}
		old[n] = p
	}
	m := w.manager(t, t.TempDir(), settings(0))
	addIndexer(t, m, w)
	mon := monitor(t, m, w, Monitored{Title: "Show", From: 1, To: 2})
	wanted, _ := m.WantedFor(mon.ID)
	if len(wanted.Missing) != 0 || len(wanted.Upgrades) != 2 {
		t.Fatalf("wanted: %+v", wanted)
	}
	rep, _ := m.SearchMonitored(mon.ID)
	if len(rep.Grabbed) != 1 {
		t.Fatalf("upgrade search: %+v", rep)
	}
	g := waitGrab(t, m, rep.Grabbed[0], GrabDone)
	if !g.Upgrade || len(g.Replaced) != 2 {
		t.Fatalf("upgrade grab: %+v", g)
	}
	w.assertImported(t) // the 1080p files are in place
	for n, p := range old {
		if _, err := os.Stat(p); !os.IsNotExist(err) {
			t.Fatalf("%s should have left the library: %v", n, err)
		}
	}
	// The replaced files are held, intact, never deleted automatically.
	held := m.Replaced()
	if len(held) != 2 {
		t.Fatalf("held files: %+v", held)
	}
	for _, h := range held {
		b, err := os.ReadFile(h.Path)
		if err != nil || !strings.HasPrefix(string(b), "old ") {
			t.Fatalf("held file %s: %v", h.Path, err)
		}
	}
	if wanted, _ := m.WantedFor(mon.ID); len(wanted.Upgrades) != 0 {
		t.Fatalf("1080p meets the cutoff; no more upgrades: %+v", wanted.Upgrades)
	}
	// Purging is a separate, explicit act.
	if n, err := m.PurgeReplaced(); err != nil || n != 2 {
		t.Fatalf("purge: %d %v", n, err)
	}
}

func TestAutomationPausesOnAFullBudget(t *testing.T) {
	w := newWorld(t)
	w.limits.MaxBytes = 1000
	m := w.manager(t, t.TempDir(), settings(0))
	addIndexer(t, m, w)
	mon := monitor(t, m, w, Monitored{Title: "Show", From: 1, To: 2})
	rep, err := m.SearchMonitored(mon.ID)
	if err != nil {
		t.Fatal(err)
	}
	if len(rep.Grabbed) != 0 || rep.Paused == "" {
		t.Fatalf("budget: %+v", rep)
	}
	if st := m.AutomationStatus(); !strings.Contains(st.Paused, "budget") {
		t.Fatalf("status: %+v", st)
	}
}

func TestMonitoredBookkeeping(t *testing.T) {
	w := newWorld(t)
	m := w.manager(t, t.TempDir(), settings(0))
	if _, err := m.CreateMonitored(Monitored{LibraryID: w.lib.ID, Kind: "movie", Title: "Film"}); CodeOf(err) != CodeInvalid {
		t.Fatalf("kind must match the library type: %v", err)
	}
	mon := monitor(t, m, w, Monitored{Title: "Show"})
	if mon.ProfileID != "default" || !mon.Enabled {
		t.Fatalf("defaults: %+v", mon)
	}
	if _, err := m.CreateMonitored(Monitored{LibraryID: w.lib.ID, Kind: "anime", Title: "SHOW!"}); CodeOf(err) != CodeState {
		t.Fatalf("duplicate title: %v", err)
	}
	if _, err := m.CreateMonitored(Monitored{LibraryID: w.lib.ID, Kind: "anime", Title: "Other", ProfileID: "nope"}); CodeOf(err) != CodeNotFound {
		t.Fatalf("unknown profile: %v", err)
	}
	p, err := m.CreateProfile(Profile{Name: "4K only", Resolutions: []string{"2160p"}})
	if err != nil {
		t.Fatal(err)
	}
	mon.ProfileID = p.ID
	if _, err := m.UpdateMonitored(mon.ID, mon); err != nil {
		t.Fatal(err)
	}
	if err := m.DeleteProfile(p.ID); CodeOf(err) != CodeState {
		t.Fatalf("a profile in use cannot be deleted: %v", err)
	}
	if err := m.DeleteMonitored(mon.ID); err != nil {
		t.Fatal(err)
	}
	if err := m.DeleteProfile(p.ID); err != nil {
		t.Fatal(err)
	}
	if len(m.Profiles()) != 1 {
		t.Fatalf("the default profile stays: %+v", m.Profiles())
	}
}

func TestAutomationSchedule(t *testing.T) {
	s := DefaultSettings("/d")
	now := time.Unix(1_800_000_000, 0)
	if rssDue(s, time.Time{}, now) {
		t.Fatal("automation is off by default")
	}
	s.Automation = true
	if !rssDue(s, time.Time{}, now) || rssDue(s, now.Add(-time.Minute), now) || !rssDue(s, now.Add(-31*time.Minute), now) {
		t.Fatal("rss cadence")
	}
	mon := Monitored{Enabled: true, LastSearchAt: now.Add(-13 * time.Hour).Unix()}
	if !searchDue(s, mon, now) {
		t.Fatal("missing search after 12 h")
	}
	mon.LastSearchAt = now.Add(-time.Hour).Unix()
	if searchDue(s, mon, now) {
		t.Fatal("searched too soon")
	}
	s.SearchHours = 0
	mon.LastSearchAt = 0
	if searchDue(s, mon, now) {
		t.Fatal("search_hours 0 means on demand only")
	}
}
