package downloads

import (
	"bytes"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/kv"
)

// gatedOrigin serves payload with Range support, but stalls each
// response after `after` bytes until the gate opens — so tests can act
// on a job that is provably mid-transfer.
type gatedOrigin struct {
	*httptest.Server
	gate   chan struct{}
	once   sync.Once
	ranges []string
	mu     sync.Mutex
	hits   atomic.Int32
}

func newGatedOrigin(t *testing.T, after int) *gatedOrigin {
	t.Helper()
	o := &gatedOrigin{gate: make(chan struct{})}
	o.Server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		o.hits.Add(1)
		o.mu.Lock()
		o.ranges = append(o.ranges, r.Header.Get("Range"))
		o.mu.Unlock()
		w.Header().Set("ETag", `"fixture"`)
		bw := &stallWriter{ResponseWriter: w, after: after, gate: o.gate, done: r.Context().Done()}
		http.ServeContent(bw, r, "f.bin", time.Unix(1700000000, 0), bytes.NewReader(payload))
	}))
	t.Cleanup(func() { o.open(); o.Close() })
	return o
}

func (o *gatedOrigin) open() { o.once.Do(func() { close(o.gate) }) }

func (o *gatedOrigin) rangeLog() []string {
	o.mu.Lock()
	defer o.mu.Unlock()
	return append([]string(nil), o.ranges...)
}

type stallWriter struct {
	http.ResponseWriter
	after   int
	written int
	gate    chan struct{}
	done    <-chan struct{}
}

func (s *stallWriter) Write(p []byte) (int, error) {
	if s.written+len(p) > s.after {
		select {
		case <-s.gate:
		default:
			n := max(s.after-s.written, 0)
			if n > 0 {
				_, _ = s.ResponseWriter.Write(p[:n])
				s.written += n
			}
			s.ResponseWriter.(http.Flusher).Flush()
			select {
			case <-s.gate:
			case <-s.done:
				return n, http.ErrAbortHandler
			}
			m, err := s.ResponseWriter.Write(p[n:])
			s.written += m
			return n + m, err
		}
	}
	n, err := s.ResponseWriter.Write(p)
	s.written += n
	return n, err
}

func openDB(t *testing.T, dir string) *bolt.DB {
	t.Helper()
	db, err := kv.Open(dir)
	if err != nil {
		t.Fatal(err)
	}
	return db
}

func newTestManager(t *testing.T, db *bolt.DB, s Settings) *Manager {
	t.Helper()
	m, err := NewManager(db, s)
	if err != nil {
		t.Fatal(err)
	}
	return m
}

func testSettings(t *testing.T) Settings {
	return Settings{Dir: t.TempDir(), Concurrency: 2}
}

func waitState(t *testing.T, m *Manager, id string, want State) Job {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for {
		j, err := m.Get(id)
		if err == nil && j.State == want {
			return j
		}
		if time.Now().After(deadline) {
			t.Fatalf("job %s: state %s (%s %s), want %s", id, j.State, j.Code, j.Error, want)
		}
		time.Sleep(5 * time.Millisecond)
	}
}

func waitBytes(t *testing.T, m *Manager, id string, atLeast int64) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for {
		if j, _ := m.Get(id); j.Bytes >= atLeast {
			return
		}
		if time.Now().After(deadline) {
			t.Fatalf("job %s never reached %d bytes", id, atLeast)
		}
		time.Sleep(5 * time.Millisecond)
	}
}

func TestManagerDownloadsIntoDirAndReportsDone(t *testing.T) {
	db := openDB(t, t.TempDir())
	defer db.Close()
	o := newGatedOrigin(t, len(payload)) // never stalls
	m := newTestManager(t, db, testSettings(t))
	var got []Job
	var mu sync.Mutex
	m.OnDone = func(j Job) { mu.Lock(); got = append(got, j); mu.Unlock() }
	m.Start()
	defer m.Close()

	lib := t.TempDir()
	j, err := m.Add(AddInput{URL: o.URL + "/media/Frieren%20-%2001.mkv", Dir: lib, LibraryID: "lib-1", CreatedBy: "u1"})
	if err != nil {
		t.Fatal(err)
	}
	if j.Name != "Frieren - 01.mkv" || j.State != Queued {
		t.Fatalf("added %+v", j)
	}
	done := waitState(t, m, j.ID, Done)
	data, err := os.ReadFile(filepath.Join(lib, "Frieren - 01.mkv"))
	if err != nil || !bytes.Equal(data, payload) || done.Path != filepath.Join(lib, "Frieren - 01.mkv") {
		t.Fatalf("file: %v path=%s", err, done.Path)
	}
	if _, err := os.Stat(done.Part); !os.IsNotExist(err) {
		t.Fatal("the part must be renamed away")
	}
	mu.Lock()
	defer mu.Unlock()
	if len(got) != 1 || got[0].LibraryID != "lib-1" {
		t.Fatalf("OnDone = %+v", got)
	}
}

func TestManagerPauseKeepsBytesAndResumeContinues(t *testing.T) {
	db := openDB(t, t.TempDir())
	defer db.Close()
	o := newGatedOrigin(t, 10_000)
	m := newTestManager(t, db, testSettings(t))
	m.Start()
	defer m.Close()
	j, _ := m.Add(AddInput{URL: o.URL + "/a.cbz"})
	waitBytes(t, m, j.ID, 10_000)
	// Pause answers with the settled state, not "running".
	if got, err := m.Pause(j.ID); err != nil || got.State != Paused {
		t.Fatalf("pause: %+v %v", got, err)
	}
	p := waitState(t, m, j.ID, Paused)
	if fi, err := os.Stat(p.Part); err != nil || fi.Size() != 10_000 || p.Bytes != 10_000 {
		t.Fatalf("paused part: %v size=%v bytes=%d", err, fi, p.Bytes)
	}
	o.open()
	if _, err := m.Resume(j.ID); err != nil {
		t.Fatal(err)
	}
	d := waitState(t, m, j.ID, Done)
	data, _ := os.ReadFile(d.Path)
	if !bytes.Equal(data, payload) {
		t.Fatalf("resumed file differs (%d bytes)", len(data))
	}
	if r := o.rangeLog(); len(r) != 2 || r[0] != "" || r[1] != "bytes=10000-" {
		t.Fatalf("ranges = %q", r)
	}
}

func TestManagerCancelDeletesPart(t *testing.T) {
	db := openDB(t, t.TempDir())
	defer db.Close()
	o := newGatedOrigin(t, 5_000)
	m := newTestManager(t, db, testSettings(t))
	m.Start()
	defer m.Close()
	j, _ := m.Add(AddInput{URL: o.URL + "/a.mkv"})
	waitBytes(t, m, j.ID, 5_000)
	if got, err := m.Cancel(j.ID); err != nil || got.State != Canceled {
		t.Fatalf("cancel: %+v %v", got, err)
	}
	c := waitState(t, m, j.ID, Canceled)
	if _, err := os.Stat(c.Part); !os.IsNotExist(err) {
		t.Fatal("canceled part must be deleted")
	}
	if _, err := m.Resume(j.ID); CodeOf(err) != CodeState {
		t.Fatalf("resume canceled: %v", err)
	}
	if _, err := m.Cancel(j.ID); err != nil {
		t.Fatalf("cancel is idempotent: %v", err)
	}
}

func TestManagerPauseQueuedAndStateErrors(t *testing.T) {
	db := openDB(t, t.TempDir())
	defer db.Close()
	m := newTestManager(t, db, testSettings(t)) // not started: jobs stay queued
	j, _ := m.Add(AddInput{URL: "http://origin.test/a.mkv"})
	if p, err := m.Pause(j.ID); err != nil || p.State != Paused {
		t.Fatalf("pause queued: %+v %v", p, err)
	}
	if err := m.Remove(j.ID); CodeOf(err) != CodeState {
		t.Fatalf("remove paused: %v", err)
	}
	if _, err := m.Pause("nope"); CodeOf(err) != CodeNotFound {
		t.Fatalf("unknown: %v", err)
	}
	if _, err := m.Cancel(j.ID); err != nil {
		t.Fatal(err)
	}
	if err := m.Remove(j.ID); err != nil {
		t.Fatal(err)
	}
	if len(m.List()) != 0 {
		t.Fatal("removed job still listed")
	}
}

func TestManagerRestartResumesRunningJob(t *testing.T) {
	dataDir := t.TempDir()
	o := newGatedOrigin(t, 20_000)
	s := testSettings(t)

	db := openDB(t, dataDir)
	m := newTestManager(t, db, s)
	m.Start()
	j, _ := m.Add(AddInput{URL: o.URL + "/a.mkv"})
	waitBytes(t, m, j.ID, 20_000)
	m.Close() // shutdown mid-transfer
	db.Close()

	db = openDB(t, dataDir)
	defer db.Close()
	m = newTestManager(t, db, s)
	r, err := m.Get(j.ID)
	if err != nil || r.State != Queued || r.Bytes != 20_000 {
		t.Fatalf("after restart: %+v %v", r, err)
	}
	o.open()
	m.Start()
	defer m.Close()
	d := waitState(t, m, j.ID, Done)
	data, _ := os.ReadFile(d.Path)
	if !bytes.Equal(data, payload) {
		t.Fatal("restart corrupted the file")
	}
	if r := o.rangeLog(); r[len(r)-1] != "bytes=20000-" {
		t.Fatalf("restart must resume by range, ranges = %q", r)
	}
}

func TestManagerQuotaFailsJobAndRefusesWhenFull(t *testing.T) {
	db := openDB(t, t.TempDir())
	defer db.Close()
	o := newGatedOrigin(t, len(payload))
	s := testSettings(t)
	s.MaxBytes = int64(len(payload)) + 100
	m := newTestManager(t, db, s)
	m.Start()
	defer m.Close()

	first, _ := m.Add(AddInput{URL: o.URL + "/a.mkv"})
	waitState(t, m, first.ID, Done)
	second, err := m.Add(AddInput{URL: o.URL + "/b.mkv"})
	if err != nil {
		t.Fatal(err)
	}
	f := waitState(t, m, second.ID, Failed)
	if f.Code != CodeQuota {
		t.Fatalf("second job: %+v", f)
	}
	if fi, err := os.Stat(f.Part); err == nil && fi.Size() > 100 {
		t.Fatalf("a refused job wrote %d bytes", fi.Size())
	}
	// Raising the budget is a setting, and the failed job resumes.
	s.MaxBytes = 0
	if _, err := m.SetSettings(s); err != nil {
		t.Fatal(err)
	}
	if _, err := m.Resume(second.ID); err != nil {
		t.Fatal(err)
	}
	waitState(t, m, second.ID, Done)

	// Over budget, Add refuses at the door.
	s.MaxBytes = 10
	_, _ = m.SetSettings(s)
	if _, err := m.Add(AddInput{URL: o.URL + "/c.mkv"}); CodeOf(err) != CodeQuota {
		t.Fatalf("add over budget: %v", err)
	}
	if u := m.Usage(); u.UsedBytes != 2*int64(len(payload)) || u.MaxBytes != 10 {
		t.Fatalf("usage = %+v", u)
	}
}

func TestManagerDiskFloorFailsJob(t *testing.T) {
	defer func(f func(string) (int64, error)) { freeBytes = f }(freeBytes)
	freeBytes = func(string) (int64, error) { return 1 << 20, nil }
	db := openDB(t, t.TempDir())
	defer db.Close()
	o := newGatedOrigin(t, len(payload))
	s := testSettings(t)
	s.MinFreeBytes = 1<<20 - 1000 // less than one payload of headroom
	m := newTestManager(t, db, s)
	m.Start()
	defer m.Close()
	j, _ := m.Add(AddInput{URL: o.URL + "/a.mkv"})
	if f := waitState(t, m, j.ID, Failed); f.Code != CodeDiskFull {
		t.Fatalf("job: %+v", f)
	}
}

func TestManagerConcurrencyBound(t *testing.T) {
	db := openDB(t, t.TempDir())
	defer db.Close()
	o := newGatedOrigin(t, 1_000)
	s := testSettings(t)
	s.Concurrency = 1
	m := newTestManager(t, db, s)
	m.Start()
	defer m.Close()
	a, _ := m.Add(AddInput{URL: o.URL + "/a.mkv"})
	b, _ := m.Add(AddInput{URL: o.URL + "/b.mkv"})
	waitBytes(t, m, a.ID, 1_000)
	time.Sleep(50 * time.Millisecond)
	if jb, _ := m.Get(b.ID); jb.State != Queued {
		t.Fatalf("second job must wait: %s", jb.State)
	}
	o.open()
	waitState(t, m, a.ID, Done)
	waitState(t, m, b.ID, Done)
}

func TestManagerNeverOverwritesAndUsesOriginName(t *testing.T) {
	db := openDB(t, t.TempDir())
	defer db.Close()
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if strings.HasSuffix(r.URL.Path, "/dl") {
			w.Header().Set("Content-Disposition", `attachment; filename="../Origin Name.cbz"`)
		}
		_, _ = w.Write(payload[:100])
	}))
	defer srv.Close()
	s := testSettings(t)
	_ = os.WriteFile(filepath.Join(s.Dir, "taken.mkv"), []byte("keep me"), 0o644)
	m := newTestManager(t, db, s)
	m.Start()
	defer m.Close()

	a, _ := m.Add(AddInput{URL: srv.URL + "/x", Name: "taken.mkv"})
	b, _ := m.Add(AddInput{URL: srv.URL + "/dl"})
	da, db2 := waitState(t, m, a.ID, Done), waitState(t, m, b.ID, Done)
	if filepath.Base(da.Path) != "taken (2).mkv" {
		t.Fatalf("collision path = %s", da.Path)
	}
	if keep, _ := os.ReadFile(filepath.Join(s.Dir, "taken.mkv")); string(keep) != "keep me" {
		t.Fatal("an existing file was overwritten")
	}
	if db2.Path != filepath.Join(s.Dir, "Origin Name.cbz") {
		t.Fatalf("origin-named path = %s", db2.Path)
	}
}

func TestManagerRejectsBadInput(t *testing.T) {
	db := openDB(t, t.TempDir())
	defer db.Close()
	m := newTestManager(t, db, testSettings(t))
	for _, u := range []string{"", "ftp://x/a", "file:///etc/passwd", "http://", "/relative"} {
		if _, err := m.Add(AddInput{URL: u}); CodeOf(err) != CodeInvalid {
			t.Errorf("Add(%q) = %v", u, err)
		}
	}
	for _, s := range []Settings{
		{Dir: "relative", Concurrency: 1},
		{Dir: "/x", Concurrency: 0},
		{Dir: "/x", Concurrency: 9},
		{Dir: "/x", Concurrency: 1, Limits: Limits{MaxBytes: -1}},
	} {
		if _, err := m.SetSettings(s); CodeOf(err) != CodeInvalid {
			t.Errorf("SetSettings(%+v) = %v", s, err)
		}
	}
}

func TestManagerSettingsPersist(t *testing.T) {
	dataDir := t.TempDir()
	db := openDB(t, dataDir)
	m := newTestManager(t, db, DefaultSettings(dataDir))
	if m.Settings().MaxBytes != DefaultMaxBytes {
		t.Fatalf("defaults = %+v", m.Settings())
	}
	want := Settings{Dir: "/srv/dl", Limits: Limits{MaxBytes: 1 << 40, MinFreeBytes: 1 << 30}, Concurrency: 3, KeepFinishedDays: 7}
	if _, err := m.SetSettings(want); err != nil {
		t.Fatal(err)
	}
	db.Close()
	db = openDB(t, dataDir)
	defer db.Close()
	if got := newTestManager(t, db, DefaultSettings(dataDir)).Settings(); got != want {
		t.Fatalf("settings after reopen = %+v", got)
	}
}

func TestManagerCleanup(t *testing.T) {
	db := openDB(t, t.TempDir())
	defer db.Close()
	s := testSettings(t)
	s.KeepFinishedDays = 1
	m := newTestManager(t, db, s)
	now := time.Unix(1_800_000_000, 0)
	m.now = func() time.Time { return now }

	failed, _ := m.Add(AddInput{URL: "http://origin.test/f.mkv"})
	old, _ := m.Add(AddInput{URL: "http://origin.test/o.mkv"})
	kept, _ := m.Add(AddInput{URL: "http://origin.test/k.mkv"})
	finished := filepath.Join(s.Dir, "done.mkv")
	_ = os.WriteFile(finished, payload[:10], 0o644)
	m.mu.Lock()
	_ = os.WriteFile(m.jobs[failed.ID].Part, payload[:300], 0o644)
	m.jobs[failed.ID].Bytes = 300
	m.setState(m.jobs[failed.ID], Failed, CodeHTTP, "boom")
	m.jobs[old.ID].Path = finished
	m.setState(m.jobs[old.ID], Done, "", "")
	m.jobs[old.ID].UpdatedAt = now.Add(-48 * time.Hour).Unix()
	m.mu.Unlock()
	orphan := filepath.Join(s.Dir, ".lain-deadbeef.part")
	_ = os.WriteFile(orphan, payload[:50], 0o644)

	rep, err := m.Cleanup()
	if err != nil {
		t.Fatal(err)
	}
	if rep.Parts != 2 || rep.FreedBytes != 350 || rep.Records != 1 {
		t.Fatalf("report = %+v", rep)
	}
	if _, err := os.Stat(finished); err != nil {
		t.Fatal("cleanup must never delete a finished file")
	}
	if _, err := m.Get(old.ID); CodeOf(err) != CodeNotFound {
		t.Fatal("expired record must go")
	}
	if _, err := m.Get(kept.ID); err != nil {
		t.Fatal("queued job must stay")
	}
	if f, _ := m.Get(failed.ID); f.Bytes != 0 {
		t.Fatalf("failed job bytes after cleanup = %d", f.Bytes)
	}
}
