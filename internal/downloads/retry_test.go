package downloads

import (
	"bytes"
	"net/http"
	"net/http/httptest"
	"os"
	"strconv"
	"sync"
	"testing"
	"time"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/kv"
)

// flakyOrigin serves payload; script decides per request (1-based)
// whether to fail with a status, cut the body after n bytes, or serve.
type flakyOrigin struct {
	*httptest.Server
	mu     sync.Mutex
	n      int
	ranges []string
	ifr    []string
}

type step struct {
	status int  // non-zero: answer this status
	cut    int  // >0: drop the connection after this many body bytes
	noTag  bool // serve without ETag/Last-Modified
}

func newFlakyOrigin(t *testing.T, script []step) *flakyOrigin {
	t.Helper()
	o := &flakyOrigin{}
	o.Server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		o.mu.Lock()
		o.n++
		st := step{}
		if o.n <= len(script) {
			st = script[o.n-1]
		}
		o.ranges = append(o.ranges, r.Header.Get("Range"))
		o.ifr = append(o.ifr, r.Header.Get("If-Range"))
		o.mu.Unlock()
		if st.status != 0 {
			w.WriteHeader(st.status)
			return
		}
		body := payload
		start := 0
		if rg := r.Header.Get("Range"); rg != "" {
			start, _ = strconv.Atoi(rg[len("bytes=") : len(rg)-1])
		}
		w.Header().Set("Accept-Ranges", "bytes")
		if !st.noTag {
			w.Header().Set("ETag", `"v1"`)
		}
		w.Header().Set("Content-Length", strconv.Itoa(len(body)-start))
		if start > 0 {
			w.Header().Set("Content-Range", "bytes "+strconv.Itoa(start)+"-"+strconv.Itoa(len(body)-1)+"/"+strconv.Itoa(len(body)))
			w.WriteHeader(http.StatusPartialContent)
		}
		rest := body[start:]
		if st.cut > 0 {
			_, _ = w.Write(rest[:st.cut])
			w.(http.Flusher).Flush()
			panic(http.ErrAbortHandler) // connection dropped mid-body
		}
		_, _ = w.Write(rest)
	}))
	t.Cleanup(o.Close)
	return o
}

func (o *flakyOrigin) log() (ranges, ifRange []string, hits int) {
	o.mu.Lock()
	defer o.mu.Unlock()
	return append([]string(nil), o.ranges...), append([]string(nil), o.ifr...), o.n
}

func retryManager(t *testing.T, retries int) (*Manager, *bolt.DB) {
	t.Helper()
	db := openDB(t, t.TempDir())
	s := testSettings(t)
	s.MaxRetries = retries
	m := newTestManager(t, db, s)
	m.Backoff = func(int) time.Duration { return 0 }
	return m, db
}

func TestRetryTransientStatusesThenSucceed(t *testing.T) {
	o := newFlakyOrigin(t, []step{{status: 503}, {status: 429}})
	m, db := retryManager(t, 3)
	defer db.Close()
	m.Start()
	defer m.Close()
	j, _ := m.Add(AddInput{URL: o.URL + "/a.mkv"})
	d := waitState(t, m, j.ID, Done)
	if _, _, hits := o.log(); hits != 3 {
		t.Fatalf("hits = %d, want 3", hits)
	}
	if d.Attempts != 0 || d.Code != "" || d.Error != "" {
		t.Fatalf("a finished job keeps no retry trace: %+v", d)
	}
}

func TestPermanentErrorsDoNotRetry(t *testing.T) {
	o := newFlakyOrigin(t, []step{{status: 404}})
	m, db := retryManager(t, 5)
	defer db.Close()
	m.Start()
	defer m.Close()
	j, _ := m.Add(AddInput{URL: o.URL + "/a.mkv"})
	if f := waitState(t, m, j.ID, Failed); f.Attempts != 0 || f.Code != CodeHTTP {
		t.Fatalf("job %+v", f)
	}
	if _, _, hits := o.log(); hits != 1 {
		t.Fatalf("a 404 was retried (%d hits)", hits)
	}
}

func TestRetriesExhaustThenFail(t *testing.T) {
	o := newFlakyOrigin(t, []step{{status: 500}, {status: 502}, {status: 503}, {status: 504}})
	m, db := retryManager(t, 2)
	defer db.Close()
	m.Start()
	defer m.Close()
	j, _ := m.Add(AddInput{URL: o.URL + "/a.mkv"})
	f := waitState(t, m, j.ID, Failed)
	if _, _, hits := o.log(); hits != 3 || f.Attempts != 2 {
		t.Fatalf("hits=%d attempts=%d, want 3 and 2", hits, f.Attempts)
	}
	// A manual resume starts a fresh retry budget and succeeds.
	if r, err := m.Resume(j.ID); err != nil || r.Attempts != 0 {
		t.Fatalf("resume: %+v %v", r, err)
	}
	waitState(t, m, j.ID, Done)
}

func TestInterruptedTransferResumesByRange(t *testing.T) {
	o := newFlakyOrigin(t, []step{{cut: 10_000}, {cut: 5_000}})
	m, db := retryManager(t, 3)
	defer db.Close()
	m.Start()
	defer m.Close()
	j, _ := m.Add(AddInput{URL: o.URL + "/a.mkv"})
	d := waitState(t, m, j.ID, Done)
	data, _ := os.ReadFile(d.Path)
	if !bytes.Equal(data, payload) {
		t.Fatalf("resumed file differs (%d bytes)", len(data))
	}
	ranges, ifr, _ := o.log()
	if len(ranges) != 3 || ranges[0] != "" || ranges[1] != "bytes=10000-" || ranges[2] != "bytes=15000-" || ifr[1] != `"v1"` {
		t.Fatalf("ranges=%q if-range=%q", ranges, ifr)
	}
}

func TestInterruptedTransferWithoutValidatorResumesWhenRangesAdvertised(t *testing.T) {
	o := newFlakyOrigin(t, []step{{cut: 8_000, noTag: true}, {noTag: true}})
	m, db := retryManager(t, 3)
	defer db.Close()
	m.Start()
	defer m.Close()
	j, _ := m.Add(AddInput{URL: o.URL + "/a.mkv"})
	d := waitState(t, m, j.ID, Done)
	data, _ := os.ReadFile(d.Path)
	if !bytes.Equal(data, payload) {
		t.Fatal("file differs")
	}
	ranges, ifr, _ := o.log()
	if ranges[1] != "bytes=8000-" || ifr[1] != "" {
		t.Fatalf("ranges=%q if-range=%q", ranges, ifr)
	}
}

func TestOldSettingsGainRetryDefault(t *testing.T) {
	dataDir := t.TempDir()
	db := openDB(t, dataDir)
	defer db.Close()
	_ = db.Update(func(tx *bolt.Tx) error {
		return tx.Bucket(kv.BDownloads).Put([]byte("settings"), []byte(`{"dir":"/srv/dl","max_bytes":1,"min_free_bytes":0,"concurrency":1,"keep_finished_days":0}`))
	})
	m := newTestManager(t, db, DefaultSettings(dataDir))
	if s := m.Settings(); s.MaxRetries != DefaultMaxRetries || s.Dir != "/srv/dl" {
		t.Fatalf("settings = %+v", s)
	}
	if _, err := m.SetSettings(Settings{Dir: "/x", Concurrency: 1, MaxRetries: 21}); CodeOf(err) != CodeInvalid {
		t.Fatalf("max_retries 21: %v", err)
	}
}

func TestRetryAfterAndBackoff(t *testing.T) {
	if d := retryAfter("7"); d != 7*time.Second {
		t.Fatalf("seconds: %v", d)
	}
	if d := retryAfter(time.Now().Add(90 * time.Second).UTC().Format(http.TimeFormat)); d < 80*time.Second || d > 90*time.Second {
		t.Fatalf("date: %v", d)
	}
	if retryAfter("") != 0 || retryAfter("soon") != 0 || retryAfter("99999") != maxRetryAfter {
		t.Fatal("bad or huge Retry-After")
	}
	for attempt, base := range map[int]time.Duration{1: 5 * time.Second, 3: 20 * time.Second, 50: 10 * time.Minute} {
		d := DefaultBackoff(attempt)
		if d < base*8/10 || d > base*12/10 {
			t.Errorf("backoff(%d) = %v, want %v ±20%%", attempt, d, base)
		}
	}
}

func TestRetryWaitsForRetryAt(t *testing.T) {
	o := newFlakyOrigin(t, []step{{status: 503}})
	m, db := retryManager(t, 3)
	defer db.Close()
	m.Backoff = func(int) time.Duration { return 2 * time.Second }
	m.Start()
	defer m.Close()
	j, _ := m.Add(AddInput{URL: o.URL + "/a.mkv"})
	deadline := time.Now().Add(5 * time.Second)
	for {
		g, _ := m.Get(j.ID)
		if g.Attempts == 1 && g.State == Queued {
			if g.RetryAt <= time.Now().Unix() || g.Code != CodeHTTP {
				t.Fatalf("queued retry %+v", g)
			}
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("never queued a retry: %+v", g)
		}
		time.Sleep(5 * time.Millisecond)
	}
	if _, _, hits := o.log(); hits != 1 {
		t.Fatalf("retried before RetryAt (%d hits)", hits)
	}
	waitState(t, m, j.ID, Done) // the retry timer fires on its own
}
