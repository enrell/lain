package torrent_test

import (
	"bytes"
	"net/netip"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/enrell/lain/internal/torrent"
	"github.com/enrell/lain/internal/torrent/trackertest"
)

// Every swarm in these tests is in-process on 127.0.0.1 (A-13).

func content(n int, seed byte) []byte {
	b := make([]byte, n)
	x := uint32(seed)*2654435761 + 1
	for i := range b {
		x ^= x << 13
		x ^= x >> 17
		x ^= x << 5
		b[i] = byte(x)
	}
	return b
}

type fixtureSet struct {
	files []torrent.SourceFile
	mi    *torrent.MetaInfo
	raw   []byte
}

func makeFixture(t *testing.T, trackers []string) fixtureSet {
	t.Helper()
	files := []torrent.SourceFile{
		{Path: []string{"[Fansub-A] Show - 01 [1080p].mkv"}, Data: content(150_000, 1)},
		{Path: []string{"[Fansub-A] Show - 02 [1080p].mkv"}, Data: content(97_001, 2)},
		{Path: []string{"Extras", "empty.txt"}, Data: nil},
		{Path: []string{"Extras", "note.txt"}, Data: content(10, 3)},
	}
	raw, mi, err := torrent.Build("[Fansub-A] Show", files, 32<<10, trackers)
	if err != nil {
		t.Fatal(err)
	}
	return fixtureSet{files: files, mi: mi, raw: raw}
}

func writeFiles(t *testing.T, dir string, f fixtureSet) {
	t.Helper()
	for _, sf := range f.files {
		p := filepath.Join(append([]string{dir, f.mi.Info.Name}, sf.Path...)...)
		if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(p, sf.Data, 0o644); err != nil {
			t.Fatal(err)
		}
	}
}

func assertFiles(t *testing.T, dir string, f fixtureSet) {
	t.Helper()
	for _, sf := range f.files {
		p := filepath.Join(append([]string{dir, f.mi.Info.Name}, sf.Path...)...)
		got, err := os.ReadFile(p)
		if err != nil {
			t.Fatalf("%s: %v", p, err)
		}
		if !bytes.Equal(got, sf.Data) {
			t.Fatalf("%s differs (%d vs %d bytes)", p, len(got), len(sf.Data))
		}
	}
}

func newClient(t *testing.T, cfg torrent.Config) *torrent.Client {
	t.Helper()
	cfg.ListenAddr = "127.0.0.1:0"
	if cfg.MinAnnounceInterval == 0 {
		cfg.MinAnnounceInterval = time.Second
	}
	c, err := torrent.NewClient(cfg)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(c.Close)
	return c
}

func waitState(t *testing.T, tor *torrent.Torrent, want torrent.State, timeout time.Duration) {
	t.Helper()
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		if tor.Stats().State == want {
			return
		}
		time.Sleep(20 * time.Millisecond)
	}
	t.Fatalf("state %s after %v, want %s (stats %+v)", tor.Stats().State, timeout, want, tor.Stats())
}

func startSeed(t *testing.T, f fixtureSet) *torrent.Torrent {
	t.Helper()
	dir := t.TempDir()
	writeFiles(t, dir, f)
	seed, err := newClient(t, torrent.Config{}).Add(torrent.Spec{MetaInfo: f.mi, Dir: dir})
	if err != nil {
		t.Fatal(err)
	}
	waitState(t, seed, torrent.StateSeeding, 5*time.Second)
	return seed
}

func TestSwarmDownloadOverHTTPTracker(t *testing.T) {
	tr, err := trackertest.New()
	if err != nil {
		t.Fatal(err)
	}
	defer tr.Close()
	f := makeFixture(t, []string{tr.HTTPURL()})
	seed := startSeed(t, f)

	dir := t.TempDir()
	completed := make(chan struct{})
	leech, err := newClient(t, torrent.Config{}).Add(torrent.Spec{
		MetaInfo: f.mi, Dir: dir,
		OnComplete: func(*torrent.Torrent) { close(completed) },
	})
	if err != nil {
		t.Fatal(err)
	}
	select {
	case <-completed:
	case <-time.After(15 * time.Second):
		t.Fatalf("no completion: leech %+v seed %+v", leech.Stats(), seed.Stats())
	}
	assertFiles(t, dir, f)
	st := leech.Stats()
	if st.State != torrent.StateSeeding || st.Completed != f.mi.Info.Length || st.Downloaded < f.mi.Info.Length {
		t.Fatalf("leech stats %+v", st)
	}
	if seed.Stats().Uploaded < f.mi.Info.Length {
		t.Fatalf("seed uploaded %d of %d", seed.Stats().Uploaded, f.mi.Info.Length)
	}
	deadline := time.Now().Add(5 * time.Second)
	for tr.Count(torrent.EventCompleted) == 0 && time.Now().Before(deadline) {
		time.Sleep(20 * time.Millisecond)
	}
	if tr.Count(torrent.EventCompleted) != 1 {
		t.Fatalf("completed announces = %d", tr.Count(torrent.EventCompleted))
	}
	files := leech.Files()
	if len(files) != 4 || files[0].Rel != filepath.Join("[Fansub-A] Show", "[Fansub-A] Show - 01 [1080p].mkv") {
		t.Fatalf("files = %+v", files)
	}
}

func TestMagnetFetchesMetadataOverUDPTracker(t *testing.T) {
	tr, err := trackertest.New()
	if err != nil {
		t.Fatal(err)
	}
	defer tr.Close()
	f := makeFixture(t, []string{tr.UDPURL()})
	startSeed(t, f)

	dir := t.TempDir()
	gotMeta := make(chan struct{})
	done := make(chan struct{})
	leech, err := newClient(t, torrent.Config{}).Add(torrent.Spec{
		InfoHash: f.mi.InfoHash, Trackers: []string{tr.UDPURL()}, Dir: dir,
		OnMetadata: func(*torrent.Torrent) { close(gotMeta) },
		OnComplete: func(*torrent.Torrent) { close(done) },
	})
	if err != nil {
		t.Fatal(err)
	}
	if leech.Info() != nil {
		t.Fatal("magnet must start without metadata")
	}
	for _, ch := range []chan struct{}{gotMeta, done} {
		select {
		case <-ch:
		case <-time.After(15 * time.Second):
			t.Fatalf("stalled: %+v", leech.Stats())
		}
	}
	if !bytes.Equal(leech.Info().Raw, f.mi.Info.Raw) {
		t.Fatal("fetched metadata differs")
	}
	assertFiles(t, dir, f)
}

func TestDirectPeersWithoutTracker(t *testing.T) {
	f := makeFixture(t, nil)
	dirA := t.TempDir()
	writeFiles(t, dirA, f)
	ca := newClient(t, torrent.Config{})
	if _, err := ca.Add(torrent.Spec{MetaInfo: f.mi, Dir: dirA}); err != nil {
		t.Fatal(err)
	}
	dir := t.TempDir()
	done := make(chan struct{})
	_, err := newClient(t, torrent.Config{}).Add(torrent.Spec{
		MetaInfo: f.mi, Dir: dir,
		Peers:      []netip.AddrPort{netip.AddrPortFrom(netip.MustParseAddr("127.0.0.1"), ca.Port())},
		OnComplete: func(*torrent.Torrent) { close(done) },
	})
	if err != nil {
		t.Fatal(err)
	}
	select {
	case <-done:
	case <-time.After(15 * time.Second):
		t.Fatal("no completion with a direct peer")
	}
	assertFiles(t, dir, f)
}

func TestCorruptSeederIsSurvived(t *testing.T) {
	f := makeFixture(t, nil)
	good := t.TempDir()
	writeFiles(t, good, f)
	// The bad seeder claims every piece (resume bitfield + full-size
	// files) but its first file is garbage.
	bad := t.TempDir()
	writeFiles(t, bad, f)
	p := filepath.Join(bad, f.mi.Info.Name, f.files[0].Path[0])
	if err := os.WriteFile(p, content(len(f.files[0].Data), 99), 0o644); err != nil {
		t.Fatal(err)
	}
	all := torrent.NewBitfield(f.mi.Info.NumPieces())
	for i := 0; i < f.mi.Info.NumPieces(); i++ {
		all.Set(i)
	}
	cb := newClient(t, torrent.Config{})
	if _, err := cb.Add(torrent.Spec{MetaInfo: f.mi, Dir: bad, Have: all}); err != nil {
		t.Fatal(err)
	}
	cg := newClient(t, torrent.Config{})
	if _, err := cg.Add(torrent.Spec{MetaInfo: f.mi, Dir: good}); err != nil {
		t.Fatal(err)
	}
	dir := t.TempDir()
	done := make(chan struct{})
	lo := netip.MustParseAddr("127.0.0.1")
	_, err := newClient(t, torrent.Config{}).Add(torrent.Spec{
		MetaInfo: f.mi, Dir: dir,
		Peers:      []netip.AddrPort{netip.AddrPortFrom(lo, cb.Port()), netip.AddrPortFrom(lo, cg.Port())},
		OnComplete: func(*torrent.Torrent) { close(done) },
	})
	if err != nil {
		t.Fatal(err)
	}
	select {
	case <-done:
	case <-time.After(30 * time.Second):
		t.Fatal("no completion beside a corrupt seeder")
	}
	assertFiles(t, dir, f)
}

func TestResumeAndRecheck(t *testing.T) {
	f := makeFixture(t, nil)
	seedDir := t.TempDir()
	writeFiles(t, seedDir, f)
	cs := newClient(t, torrent.Config{})
	if _, err := cs.Add(torrent.Spec{MetaInfo: f.mi, Dir: seedDir}); err != nil {
		t.Fatal(err)
	}
	peers := []netip.AddrPort{netip.AddrPortFrom(netip.MustParseAddr("127.0.0.1"), cs.Port())}

	// Throttled download, paused part-way.
	dir := t.TempDir()
	c1 := newClient(t, torrent.Config{DownloadRate: 64 << 10})
	tor, err := c1.Add(torrent.Spec{MetaInfo: f.mi, Dir: dir, Peers: peers})
	if err != nil {
		t.Fatal(err)
	}
	deadline := time.Now().Add(10 * time.Second)
	for tor.Stats().Have < 2 && time.Now().Before(deadline) {
		time.Sleep(20 * time.Millisecond)
	}
	tor.Pause()
	have := tor.Stats().Have
	if have < 2 || have == f.mi.Info.NumPieces() {
		t.Fatalf("want a partial download, have %d of %d", have, f.mi.Info.NumPieces())
	}
	if err := tor.Remove(false); err != nil {
		t.Fatal(err)
	}

	// A fresh client without a resume bitfield rechecks what is on disk.
	c2 := newClient(t, torrent.Config{})
	tor2, err := c2.Add(torrent.Spec{MetaInfo: f.mi, Dir: dir, Paused: true})
	if err != nil {
		t.Fatal(err)
	}
	done := make(chan struct{})
	tor2.Resume()
	waitState(t, tor2, torrent.StateDownloading, 5*time.Second)
	if got := tor2.Stats().Have; got < have {
		t.Fatalf("recheck found %d pieces, had %d", got, have)
	}
	_ = tor2.Remove(false)
	tor3, err := c2.Add(torrent.Spec{MetaInfo: f.mi, Dir: dir, Peers: peers, OnComplete: func(*torrent.Torrent) { close(done) }})
	if err != nil {
		t.Fatal(err)
	}
	select {
	case <-done:
	case <-time.After(15 * time.Second):
		t.Fatalf("resume stalled: %+v", tor3.Stats())
	}
	assertFiles(t, dir, f)

	// A completed torrent restarts straight to seeding from its bitfield.
	bf := tor3.Bitfield()
	_ = tor3.Remove(false)
	tor4, err := c2.Add(torrent.Spec{MetaInfo: f.mi, Dir: dir, Have: bf})
	if err != nil {
		t.Fatal(err)
	}
	waitState(t, tor4, torrent.StateSeeding, 5*time.Second)

	// Remove with data deletes the files and the empty folders.
	if err := tor4.Remove(true); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(dir, f.mi.Info.Name)); !os.IsNotExist(err) {
		t.Fatalf("torrent folder survived removal: %v", err)
	}
	if _, ok := c2.Torrent(f.mi.InfoHash); ok {
		t.Fatal("removed torrent still registered")
	}
}

func TestAddTwiceIsRejected(t *testing.T) {
	f := makeFixture(t, nil)
	c := newClient(t, torrent.Config{})
	if _, err := c.Add(torrent.Spec{MetaInfo: f.mi, Dir: t.TempDir(), Paused: true}); err != nil {
		t.Fatal(err)
	}
	if _, err := c.Add(torrent.Spec{MetaInfo: f.mi, Dir: t.TempDir(), Paused: true}); err != torrent.ErrExists {
		t.Fatalf("second add: %v", err)
	}
}
