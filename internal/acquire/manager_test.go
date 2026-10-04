package acquire

import (
	"bytes"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/downloads"
	"github.com/enrell/lain/internal/kv"
	"github.com/enrell/lain/internal/plugins/indexer"
	"github.com/enrell/lain/internal/torrent"
	"github.com/enrell/lain/internal/torrent/trackertest"
)

// Every network peer here is in-process on loopback (A-13): a test
// tracker, a seeding engine and a fake Torznab indexer.

func payload(n int, seed byte) []byte {
	b := make([]byte, n)
	x := uint32(seed)*2246822519 + 7
	for i := range b {
		x ^= x << 13
		x ^= x >> 17
		x ^= x << 5
		b[i] = byte(x)
	}
	return b
}

type world struct {
	t       *testing.T
	tracker *trackertest.Tracker
	seed    *torrent.Client
	mi      *torrent.MetaInfo
	raw     []byte
	files   []torrent.SourceFile
	index   *httptest.Server
	lib     contracts.Library
	rescans []string
	mu      sync.Mutex
	limits  downloads.Limits
}

func newWorld(t *testing.T) *world {
	t.Helper()
	tr, err := trackertest.New()
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(tr.Close)
	w := &world{t: t, tracker: tr}
	w.files = []torrent.SourceFile{
		{Path: []string{"[Fansub-A] Show - 01 [1080p].mkv"}, Data: payload(120_000, 1)},
		{Path: []string{"[Fansub-A] Show - 02 [1080p].mkv"}, Data: payload(90_000, 2)},
		{Path: []string{"sample.mkv"}, Data: payload(500, 3)},
		{Path: []string{"info.nfo"}, Data: []byte("placeholder")},
	}
	w.raw, w.mi, err = torrent.Build("[Fansub-A] Show - 01-02 [1080p]", w.files, 32<<10, []string{tr.HTTPURL()})
	if err != nil {
		t.Fatal(err)
	}
	seedDir := t.TempDir()
	for _, f := range w.files {
		p := filepath.Join(append([]string{seedDir, w.mi.Info.Name}, f.Path...)...)
		_ = os.MkdirAll(filepath.Dir(p), 0o755)
		if err := os.WriteFile(p, f.Data, 0o644); err != nil {
			t.Fatal(err)
		}
	}
	w.seed, err = torrent.NewClient(torrent.Config{ListenAddr: "127.0.0.1:0", MinAnnounceInterval: time.Second})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(w.seed.Close)
	if _, err := w.seed.Add(torrent.Spec{MetaInfo: w.mi, Dir: seedDir}); err != nil {
		t.Fatal(err)
	}
	w.index = httptest.NewServer(http.HandlerFunc(w.serveIndexer))
	t.Cleanup(w.index.Close)
	w.lib = contracts.Library{ID: "lib-anime", Name: "Anime", Type: "anime", Path: t.TempDir()}
	w.limits = downloads.Limits{MaxBytes: 50 << 20}
	return w
}

func (w *world) serveIndexer(rw http.ResponseWriter, r *http.Request) {
	switch {
	case r.URL.Path == "/dl/show.torrent":
		_, _ = rw.Write(w.raw)
	case r.URL.Path == "/dl/magnet":
		http.Redirect(rw, r, "magnet:?xt=urn:btih:"+w.mi.InfoHash.Hex()+"&tr="+w.tracker.HTTPURL(), http.StatusFound)
	case r.URL.Query().Get("apikey") != "k":
		_, _ = rw.Write([]byte(`<error code="100" description="bad key"/>`))
	case r.URL.Query().Get("t") == "caps":
		_, _ = rw.Write([]byte(`<caps><searching><search available="yes"/><tv-search available="yes"/></searching><categories><category id="5070" name="Anime"/></categories></caps>`))
	default:
		fmt.Fprintf(rw, `<rss><channel>
<item><title>%s</title><link>%s/dl/show.torrent</link><size>%d</size><torznab:attr name="seeders" value="1"/></item>
<item><title>[Fansub-B] Other Show - 01 [720p]</title><link>%s/dl/none.torrent</link><torznab:attr name="seeders" value="99"/></item>
</channel></rss>`, w.mi.Info.Name, w.index.URL, w.mi.Info.Length, w.index.URL)
	}
}

func (w *world) manager(t *testing.T, dataDir string, s Settings) *Manager {
	t.Helper()
	db, err := kv.Open(dataDir)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	tz := indexer.NewTorznab()
	s.Dir = filepath.Join(dataDir, "acquire")
	m, err := New(Deps{
		DB: db, DataDir: dataDir, Parse: parse, Initial: &s, Tick: 100 * time.Millisecond,
		ListenHost: "127.0.0.1", Engine: &torrent.Config{MinAnnounceInterval: time.Second},
		Search: func(in contracts.IndexerSearchInput) ([]contracts.SearchResult, error) {
			out, err := tz.Invoke(contracts.CapIndexer, in)
			if err != nil {
				return nil, err
			}
			return out.(contracts.IndexerSearchOutput).Results, nil
		},
		Caps: func(ix contracts.Indexer) (contracts.IndexerCaps, error) {
			out, err := tz.Invoke(contracts.CapIndexer, contracts.IndexerCapsInput{Indexer: ix})
			if err != nil {
				return contracts.IndexerCaps{}, err
			}
			return out.(contracts.IndexerCaps), nil
		},
		Budget: func() (downloads.Limits, int64) {
			w.mu.Lock()
			defer w.mu.Unlock()
			return w.limits, 0
		},
		Library: func(id string) (contracts.Library, bool) { return w.lib, id == w.lib.ID },
		Titles:  func(string) []string { return []string{"SHOW"} },
		Rescan: func(id string) {
			w.mu.Lock()
			w.rescans = append(w.rescans, id)
			w.mu.Unlock()
		},
	})
	if err != nil {
		t.Fatal(err)
	}
	m.Start()
	t.Cleanup(m.Close)
	return m
}

func settings(ratio float64) Settings {
	s := DefaultSettings("/unused")
	s.ListenPort, s.SeedRatio, s.SeedMinutes = 0, ratio, 0
	return s
}

func waitGrab(t *testing.T, m *Manager, id string, states ...string) Grab {
	t.Helper()
	deadline := time.Now().Add(20 * time.Second)
	for time.Now().Before(deadline) {
		g, err := m.Get(id)
		if err != nil {
			t.Fatal(err)
		}
		for _, s := range states {
			if g.State == s {
				return g
			}
		}
		if g.State == GrabFailed {
			t.Fatalf("grab failed: %s %s", g.Code, g.Error)
		}
		time.Sleep(50 * time.Millisecond)
	}
	g, _ := m.Get(id)
	t.Fatalf("grab state %s, want %v (%+v)", g.State, states, g)
	return g
}

func (w *world) assertImported(t *testing.T) {
	t.Helper()
	for i, name := range []string{"SHOW - 01.mkv", "SHOW - 02.mkv"} {
		got, err := os.ReadFile(filepath.Join(w.lib.Path, "SHOW", name))
		if err != nil || !bytes.Equal(got, w.files[i].Data) {
			t.Fatalf("%s: %v", name, err)
		}
	}
	entries, _ := os.ReadDir(filepath.Join(w.lib.Path, "SHOW"))
	if len(entries) != 2 {
		t.Fatalf("library holds %d files, want 2 (sample and nfo skipped)", len(entries))
	}
	w.mu.Lock()
	defer w.mu.Unlock()
	if len(w.rescans) == 0 || w.rescans[0] != w.lib.ID {
		t.Fatalf("rescans = %v", w.rescans)
	}
}

func TestSearchGrabImportWithoutSeeding(t *testing.T) {
	w := newWorld(t)
	m := w.manager(t, t.TempDir(), settings(0))

	name, proto, key := "tracker-exemplo", contracts.IndexerTorznab, "k"
	url := w.index.URL
	ix, err := m.CreateIndexer(IndexerInput{Name: &name, Protocol: &proto, URL: &url, APIKey: &key})
	if err != nil {
		t.Fatal(err)
	}
	if ix.APIKey != "" || !m.HasAPIKey(ix.ID) {
		t.Fatal("the key must be stored but never returned")
	}
	if tested, err := m.TestIndexer(ix.ID); err != nil || tested.Caps == nil || !tested.Caps.TVSearch {
		t.Fatalf("test: %+v %v", tested, err)
	}
	res, err := m.Search(SearchQuery{Query: "Show", Kind: "anime"})
	if err != nil || len(res.Failures) != 0 || len(res.Candidates) != 2 {
		t.Fatalf("search: %+v %v", res, err)
	}
	best := res.Candidates[0]
	if best.Release.Title != "Show" || len(best.Rejections) != 0 || len(best.Release.Episodes) != 2 {
		t.Fatalf("ranking: %+v", res.Candidates)
	}
	if len(res.Candidates[1].Rejections) == 0 {
		t.Fatal("another show must be marked as not matching")
	}

	g, err := m.Grab(GrabInput{Result: &best.SearchResult, LibraryID: w.lib.ID})
	if err != nil {
		t.Fatal(err)
	}
	if g.Size != w.mi.Info.Length || g.InfoHash != w.mi.InfoHash.Hex() {
		t.Fatalf("grab = %+v", g)
	}
	done := waitGrab(t, m, g.ID, GrabDone)
	w.assertImported(t)
	if !done.DataRemoved || len(done.Imported) != 2 {
		t.Fatalf("no seeding: torrent copy must be removed after import: %+v", done)
	}
	if _, err := os.Stat(filepath.Join(m.Settings().Dir, g.ID, w.mi.Info.Name)); !os.IsNotExist(err) {
		t.Fatalf("torrent data left behind: %v", err)
	}
	if u := m.Usage(); u.UsedBytes != 0 {
		t.Fatalf("usage after removal = %+v", u)
	}
}

func TestSeedingStopsAtRatio(t *testing.T) {
	w := newWorld(t)
	m := w.manager(t, t.TempDir(), settings(1.0))
	g, err := m.Grab(GrabInput{URL: w.index.URL + "/dl/show.torrent", LibraryID: w.lib.ID})
	if err != nil {
		t.Fatal(err)
	}
	waitGrab(t, m, g.ID, GrabSeeding)
	w.assertImported(t)
	// Hardlinked: the library file and the seeding copy share an inode.
	a, _ := os.Stat(filepath.Join(w.lib.Path, "SHOW", "SHOW - 01.mkv"))
	b, _ := os.Stat(filepath.Join(m.Settings().Dir, g.ID, w.mi.Info.Name, w.files[0].Path[0]))
	if !os.SameFile(a, b) {
		t.Fatal("import must hardlink on the same filesystem")
	}
	if u := m.Usage(); u.UsedBytes != w.mi.Info.Length {
		t.Fatalf("seeding data must count against the budget: %+v", u)
	}
	// The original seeder leaves; a fresh leecher finds Lain through
	// the tracker.
	w.seed.Close()
	leech, err := torrent.NewClient(torrent.Config{ListenAddr: "127.0.0.1:0", MinAnnounceInterval: time.Second})
	if err != nil {
		t.Fatal(err)
	}
	defer leech.Close()
	if _, err := leech.Add(torrent.Spec{MetaInfo: w.mi, Dir: t.TempDir()}); err != nil {
		t.Fatal(err)
	}
	final := waitGrab(t, m, g.ID, GrabDone)
	if final.Uploaded < final.Size || !final.DataRemoved {
		t.Fatalf("ratio stop: %+v", final)
	}
	if _, err := os.Stat(filepath.Join(w.lib.Path, "SHOW", "SHOW - 01.mkv")); err != nil {
		t.Fatal("removing the seeding copy must keep the library file")
	}
}

func TestMagnetRedirectFromIndexer(t *testing.T) {
	w := newWorld(t)
	m := w.manager(t, t.TempDir(), settings(0))
	g, err := m.Grab(GrabInput{URL: w.index.URL + "/dl/magnet", LibraryID: w.lib.ID})
	if err != nil {
		t.Fatal(err)
	}
	if g.Source != "magnet" || g.State != GrabMetadata {
		t.Fatalf("grab = %+v", g)
	}
	done := waitGrab(t, m, g.ID, GrabDone)
	if done.Size != w.mi.Info.Length {
		t.Fatalf("size after metadata = %d", done.Size)
	}
	w.assertImported(t)
}

func TestBudgetRefusesBigGrabs(t *testing.T) {
	w := newWorld(t)
	w.limits = downloads.Limits{MaxBytes: 1000}
	m := w.manager(t, t.TempDir(), settings(0))
	_, err := m.Grab(GrabInput{URL: w.index.URL + "/dl/show.torrent", LibraryID: w.lib.ID})
	if CodeOf(err) != CodeQuota {
		t.Fatalf("over budget: %v", err)
	}
	if len(m.Grabs()) != 0 {
		t.Fatal("a refused grab must not be recorded")
	}
	// Magnets are refused only when the store is already full; the
	// metadata check fails them once their size is known.
	g, err := m.Grab(GrabInput{URL: w.index.URL + "/dl/magnet", LibraryID: w.lib.ID})
	if err != nil {
		t.Fatal(err)
	}
	failed := waitGrabFailed(t, m, g.ID)
	if failed.Code != CodeQuota || !failed.DataRemoved {
		t.Fatalf("magnet over budget: %+v", failed)
	}
}

func waitGrabFailed(t *testing.T, m *Manager, id string) Grab {
	t.Helper()
	deadline := time.Now().Add(20 * time.Second)
	for time.Now().Before(deadline) {
		if g, _ := m.Get(id); g.State == GrabFailed {
			return g
		}
		time.Sleep(50 * time.Millisecond)
	}
	g, _ := m.Get(id)
	t.Fatalf("never failed: %+v", g)
	return g
}

func TestGrabValidation(t *testing.T) {
	w := newWorld(t)
	m := w.manager(t, t.TempDir(), settings(0))
	for _, c := range []struct {
		in   GrabInput
		code string
	}{
		{GrabInput{URL: w.index.URL + "/dl/show.torrent", LibraryID: "nope"}, CodeNoLibrary},
		{GrabInput{LibraryID: w.lib.ID}, CodeInvalid},
		{GrabInput{Magnet: "magnet:?xt=urn:btih:" + w.mi.InfoHash.Hex(), LibraryID: w.lib.ID}, CodeUnsupported},
		{GrabInput{URL: "file:///etc/passwd", LibraryID: w.lib.ID}, CodeInvalid},
		{GrabInput{URL: w.index.URL + "/dl/none.torrent", LibraryID: w.lib.ID}, CodeFetch},
		{GrabInput{Result: &contracts.SearchResult{Protocol: contracts.ProtocolUsenet, Link: "http://x/y.nzb"}, LibraryID: w.lib.ID}, CodeUnsupported},
	} {
		if _, err := m.Grab(c.in); CodeOf(err) != c.code {
			t.Errorf("%+v: %v, want %s", c.in, err, c.code)
		}
	}
	// The same torrent twice is refused.
	if _, err := m.Grab(GrabInput{URL: w.index.URL + "/dl/show.torrent", LibraryID: w.lib.ID}); err != nil {
		t.Fatal(err)
	}
	if _, err := m.Grab(GrabInput{URL: w.index.URL + "/dl/show.torrent", LibraryID: w.lib.ID}); CodeOf(err) != CodeState {
		t.Fatalf("duplicate: %v", err)
	}
}

func TestRestartResumesAPausedGrab(t *testing.T) {
	w := newWorld(t)
	data := t.TempDir()
	m1 := w.manager(t, data, settings(0))
	g, err := m1.Grab(GrabInput{URL: w.index.URL + "/dl/show.torrent", LibraryID: w.lib.ID})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := m1.Pause(g.ID); err != nil {
		t.Fatal(err)
	}
	m1.Close()
	// Close the first database before reopening the data dir.
	m1.d.DB.Close()

	m2 := w.manager(t, data, settings(0))
	if got, _ := m2.Get(g.ID); got.State != GrabPaused {
		t.Fatalf("after restart: %+v", got)
	}
	if _, err := m2.Resume(g.ID); err != nil {
		t.Fatal(err)
	}
	waitGrab(t, m2, g.ID, GrabDone)
	w.assertImported(t)
}

func TestIndexerInputValidation(t *testing.T) {
	w := newWorld(t)
	m := w.manager(t, t.TempDir(), settings(0))
	name, proto := "x", contracts.IndexerTorznab
	for _, u := range []string{"ftp://x", "http://user:pass@x/", "not a url"} {
		u := u
		if _, err := m.CreateIndexer(IndexerInput{Name: &name, Protocol: &proto, URL: &u}); CodeOf(err) != CodeInvalid {
			t.Errorf("%s: %v", u, err)
		}
	}
	u, key := w.index.URL, "first"
	ix, err := m.CreateIndexer(IndexerInput{Name: &name, Protocol: &proto, URL: &u, APIKey: &key})
	if err != nil {
		t.Fatal(err)
	}
	empty := ""
	if _, err := m.UpdateIndexer(ix.ID, IndexerInput{APIKey: &empty}); err != nil || !m.HasAPIKey(ix.ID) {
		t.Fatal("an empty key in a patch keeps the stored key")
	}
	clear := "-"
	if _, err := m.UpdateIndexer(ix.ID, IndexerInput{APIKey: &clear}); err != nil || m.HasAPIKey(ix.ID) {
		t.Fatal(`"-" clears the key`)
	}
}
