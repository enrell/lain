package offline

import (
	"bytes"
	"context"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strconv"
	"testing"
	"time"

	"github.com/enrell/lain/internal/downloads"
)

var fixture = bytes.Repeat([]byte("offline-fixture\n"), 2048) // 32 KiB

// stream mimics /api/items/{id}/stream: Range-capable, token-checked.
func stream(t *testing.T, bodies map[string][]byte) *httptest.Server {
	t.Helper()
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "Bearer tok" {
			w.WriteHeader(http.StatusUnauthorized)
			return
		}
		b, ok := bodies[r.URL.Path]
		if !ok {
			http.NotFound(w, r)
			return
		}
		w.Header().Set("ETag", `"`+strconv.Itoa(len(b))+`"`)
		http.ServeContent(w, r, "f", time.Unix(1700000000, 0), bytes.NewReader(b))
	}))
	t.Cleanup(srv.Close)
	return srv
}

func open(t *testing.T, cfg Config) *Store {
	t.Helper()
	if cfg.Dir == "" {
		cfg.Dir = t.TempDir()
	}
	s, err := Open(cfg)
	if err != nil {
		t.Fatal(err)
	}
	return s
}

var auth = http.Header{"Authorization": {"Bearer tok"}}

func TestFetchStoresCopyAndPersists(t *testing.T) {
	srv := stream(t, map[string][]byte{"/a": fixture})
	dir := t.TempDir()
	s := open(t, Config{Dir: dir})
	e, err := s.Add(Entry{ItemID: "i1", Title: "Frieren", Season: 1, Episode: 3}, "../[Fansub-A] Frieren - 03.mkv")
	if err != nil {
		t.Fatal(err)
	}
	if e.File != "[Fansub-A] Frieren - 03.mkv" || e.Label() != "Frieren S01E03" {
		t.Fatalf("entry %+v label %q", e, e.Label())
	}
	got, err := s.Fetch(context.Background(), nil, "i1", FetchInput{URL: srv.URL + "/a", Header: auth})
	if err != nil || got.State != Done {
		t.Fatalf("fetch: %+v %v", got, err)
	}
	if data, _ := os.ReadFile(s.LocalPath("i1")); !bytes.Equal(data, fixture) {
		t.Fatal("local copy differs")
	}
	// Reopen: the index survived.
	s2 := open(t, Config{Dir: dir})
	if s2.LocalPath("i1") == "" || len(s2.List()) != 1 {
		t.Fatal("index not persisted")
	}
	// A vanished file is not a local copy.
	os.Remove(s2.LocalPath("i1"))
	if s2.LocalPath("i1") != "" {
		t.Fatal("vanished file still reported")
	}
}

func TestFetchCancelPausesAndResumeContinues(t *testing.T) {
	srv := stream(t, map[string][]byte{"/a": fixture})
	s := open(t, Config{})
	_, _ = s.Add(Entry{ItemID: "i1", Title: "T"}, "t.cbz")
	ctx, cancel := context.WithCancel(context.Background())
	got, err := s.Fetch(ctx, srv.Client(), "i1", FetchInput{URL: srv.URL + "/a", Header: auth,
		Progress: func(done, _ int64) {
			if done > 0 {
				cancel()
			}
		}})
	if err == nil || got.State != Paused || got.Bytes == 0 {
		// The whole fixture may land in one read; then it is simply done.
		if got.State != Done {
			t.Fatalf("canceled fetch: %+v %v", got, err)
		}
	}
	if _, err := s.Resume("i1"); err != nil {
		t.Fatal(err)
	}
	got, err = s.Fetch(context.Background(), srv.Client(), "i1", FetchInput{URL: srv.URL + "/a", Header: auth})
	if err != nil || got.State != Done {
		t.Fatalf("resume: %+v %v", got, err)
	}
	if data, _ := os.ReadFile(s.LocalPath("i1")); !bytes.Equal(data, fixture) {
		t.Fatal("resumed copy differs")
	}
}

func TestFetchFailureIsTypedAndRetryable(t *testing.T) {
	srv := stream(t, map[string][]byte{})
	s := open(t, Config{})
	_, _ = s.Add(Entry{ItemID: "i1", Title: "T"}, "t.mkv")
	got, err := s.Fetch(context.Background(), nil, "i1", FetchInput{URL: srv.URL + "/missing", Header: auth})
	if err == nil || got.State != Failed || got.Code != downloads.CodeHTTP {
		t.Fatalf("got %+v %v", got, err)
	}
	if again, _ := s.Add(Entry{ItemID: "i1"}, "t.mkv"); again.State != Queued {
		t.Fatalf("re-adding a failed entry requeues it: %+v", again)
	}
}

func TestQuotaEvictsWatchedCopiesLRU(t *testing.T) {
	srv := stream(t, map[string][]byte{"/a": fixture, "/b": fixture, "/c": fixture})
	s := open(t, Config{Limits: downloads.Limits{MaxBytes: int64(2*len(fixture) + 10)}, EvictWatched: true})
	clock := int64(1000)
	s.now = func() time.Time { clock++; return time.Unix(clock, 0) }
	for _, id := range []string{"a", "b"} {
		_, _ = s.Add(Entry{ItemID: id, Title: id}, id+".mkv")
		if _, err := s.Fetch(context.Background(), nil, id, FetchInput{URL: srv.URL + "/" + id, Header: auth}); err != nil {
			t.Fatal(err)
		}
	}
	s.SetWatched("a", true)
	s.SetWatched("b", true)
	s.Touch("a") // b is now least recently used

	_, _ = s.Add(Entry{ItemID: "c", Title: "c"}, "c.mkv")
	var evicted []string
	got, err := s.Fetch(context.Background(), nil, "c", FetchInput{URL: srv.URL + "/c", Header: auth,
		Evicted: func(e Entry) { evicted = append(evicted, e.ItemID) }})
	if err != nil || got.State != Done {
		t.Fatalf("c: %+v %v", got, err)
	}
	if len(evicted) != 1 || evicted[0] != "b" {
		t.Fatalf("evicted %v, want [b]", evicted)
	}
	if s.LocalPath("a") == "" || s.Used() != int64(2*len(fixture)) {
		t.Fatalf("a kept? %q used=%d", s.LocalPath("a"), s.Used())
	}
}

func TestQuotaNeverEvictsUnwatchedOrUnsynced(t *testing.T) {
	srv := stream(t, map[string][]byte{"/a": fixture, "/b": fixture})
	s := open(t, Config{Limits: downloads.Limits{MaxBytes: int64(len(fixture) + 10)}, EvictWatched: true})
	_, _ = s.Add(Entry{ItemID: "a", Title: "a"}, "a.mkv")
	_, _ = s.Fetch(context.Background(), nil, "a", FetchInput{URL: srv.URL + "/a", Header: auth})
	s.RecordOffline("a", Progress{PositionSec: 1400, DurationSec: 1440, Completed: true})

	_, _ = s.Add(Entry{ItemID: "b", Title: "b"}, "b.mkv")
	got, err := s.Fetch(context.Background(), nil, "b", FetchInput{URL: srv.URL + "/b", Header: auth})
	if downloads.CodeOf(err) != downloads.CodeQuota || got.State != Failed {
		t.Fatalf("b: %+v %v", got, err)
	}
	if s.LocalPath("a") == "" {
		t.Fatal("a copy with unsynced progress was evicted")
	}
	// Synced, it becomes a candidate.
	s.ClearPending("a")
	_, _ = s.Resume("b")
	if got, err := s.Fetch(context.Background(), nil, "b", FetchInput{URL: srv.URL + "/b", Header: auth}); err != nil || got.State != Done {
		t.Fatalf("b after sync: %+v %v", got, err)
	}
	if s.LocalPath("a") != "" {
		t.Fatal("watched, synced copy should have made room")
	}
}

func TestRemoveRefusesUnsyncedUnlessForced(t *testing.T) {
	srv := stream(t, map[string][]byte{"/a": fixture})
	s := open(t, Config{})
	_, _ = s.Add(Entry{ItemID: "a", Title: "a"}, "a.mkv")
	_, _ = s.Fetch(context.Background(), nil, "a", FetchInput{URL: srv.URL + "/a", Header: auth})
	s.RecordOffline("a", Progress{PositionSec: 10, DurationSec: 100})
	if _, err := s.Remove("a", false); downloads.CodeOf(err) != downloads.CodeState {
		t.Fatalf("unforced remove: %v", err)
	}
	freed, err := s.Remove("a", true)
	if err != nil || freed != int64(len(fixture)) || len(s.List()) != 0 {
		t.Fatalf("forced remove: %d %v", freed, err)
	}
}

func TestGC(t *testing.T) {
	srv := stream(t, map[string][]byte{"/a": fixture, "/b": fixture})
	s := open(t, Config{})
	for _, id := range []string{"a", "b"} {
		_, _ = s.Add(Entry{ItemID: id, Title: id}, id+".mkv")
		_, _ = s.Fetch(context.Background(), nil, id, FetchInput{URL: srv.URL + "/" + id, Header: auth})
	}
	_ = os.WriteFile(filepath.Join(s.Config().Dir, ".parts", "ghost.part"), fixture[:100], 0o644)
	os.Remove(s.LocalPath("a")) // deleted by hand
	s.SetWatched("b", true)

	rep, err := s.GC(false)
	if err != nil || rep.OrphanParts != 1 || rep.Vanished != 1 || len(rep.Evicted) != 0 || rep.FreedBytes != 100 {
		t.Fatalf("gc: %+v %v", rep, err)
	}
	rep, err = s.GC(true)
	if err != nil || len(rep.Evicted) != 1 || rep.FreedBytes != int64(len(fixture)) || len(s.List()) != 0 {
		t.Fatalf("gc --watched: %+v %v", rep, err)
	}
}

func TestLockIsExclusiveAndRecoversStale(t *testing.T) {
	s := open(t, Config{})
	unlock, err := s.Lock()
	if err != nil {
		t.Fatal(err)
	}
	// Another live process holds it: simulate with our parent pid.
	unlock()
	_ = os.WriteFile(filepath.Join(s.Config().Dir, ".lock"), []byte(strconv.Itoa(os.Getppid())+"\n"), 0o600)
	if _, err := s.Lock(); downloads.CodeOf(err) != downloads.CodeState {
		t.Fatalf("held lock: %v", err)
	}
	// A dead pid is stale.
	_ = os.WriteFile(filepath.Join(s.Config().Dir, ".lock"), []byte("999999999\n"), 0o600)
	unlock, err = s.Lock()
	if err != nil {
		t.Fatalf("stale lock: %v", err)
	}
	unlock()
}

func TestOpenRejectsRelativeDirAndCorruptIndex(t *testing.T) {
	if _, err := Open(Config{Dir: "rel"}); downloads.CodeOf(err) != downloads.CodeInvalid {
		t.Fatalf("relative: %v", err)
	}
	dir := t.TempDir()
	_ = os.WriteFile(filepath.Join(dir, "index.json"), []byte("{"), 0o600)
	if _, err := Open(Config{Dir: dir}); downloads.CodeOf(err) != downloads.CodeIO {
		t.Fatalf("corrupt: %v", err)
	}
}

func TestAddKeepsNamesUnique(t *testing.T) {
	s := open(t, Config{})
	a, _ := s.Add(Entry{ItemID: "a", Title: "x"}, "same.cbz")
	b, _ := s.Add(Entry{ItemID: "b", Title: "x"}, "same.cbz")
	c, _ := s.Add(Entry{ItemID: "c", Title: "Only Title"}, "")
	if a.File != "same.cbz" || b.File != "same (2).cbz" || c.File != "Only Title" {
		t.Fatalf("names %q %q %q", a.File, b.File, c.File)
	}
	if _, err := s.Add(Entry{}, "x"); downloads.CodeOf(err) != downloads.CodeInvalid {
		t.Fatal("empty item id accepted")
	}
}
