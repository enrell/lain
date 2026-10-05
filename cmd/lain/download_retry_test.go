package main

import (
	"bytes"
	"context"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/enrell/lain/internal/offline"
)

// flakyStream serves episodeBytes; plan(hit) decides per request
// whether to fail with a status, cut the body after n bytes, or (0, 0)
// serve normally.
type flakyStream struct {
	*httptest.Server
	mu   sync.Mutex
	hits int
}

// cutWriter stops the body after left bytes, so the client sees a
// dropped transfer.
type cutWriter struct {
	http.ResponseWriter
	left int
}

func (w *cutWriter) Write(p []byte) (int, error) {
	if w.left <= 0 {
		return 0, errors.New("cut")
	}
	if len(p) > w.left {
		p = p[:w.left]
	}
	n, err := w.ResponseWriter.Write(p)
	w.left -= n
	return n, err
}

func newFlakyStream(t *testing.T, plan func(hit int) (status, cut int)) *flakyStream {
	t.Helper()
	f := &flakyStream{}
	f.Server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		f.mu.Lock()
		f.hits++
		hit := f.hits
		f.mu.Unlock()
		status, cut := plan(hit)
		if status != 0 {
			w.WriteHeader(status)
			return
		}
		w.Header().Set("ETag", `"e1"`)
		if cut > 0 {
			w = &cutWriter{ResponseWriter: w, left: cut}
		}
		http.ServeContent(w, r, "x.mkv", time.Unix(1700000000, 0), bytes.NewReader(episodeBytes))
	}))
	t.Cleanup(f.Close)
	return f
}

func (f *flakyStream) Hits() int {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.hits
}

// noBackoff makes retries immediate for the duration of a test.
func noBackoff(t *testing.T) {
	t.Helper()
	old := offlineBackoff
	offlineBackoff = func(int) time.Duration { return 0 }
	t.Cleanup(func() { offlineBackoff = old })
}

func retryStore(t *testing.T, server string, retries int) *offline.Store {
	t.Helper()
	s := openTestStore(t, offline.Config{Dir: t.TempDir(), MaxRetries: retries})
	if _, err := s.Add(offline.Entry{ItemID: "ep1", Server: server, Title: "Frieren", Season: 1, Episode: 1}, "Frieren - 01.mkv"); err != nil {
		t.Fatal(err)
	}
	return s
}

func TestRunOfflineQueueRetriesTransientFailures(t *testing.T) {
	noBackoff(t)
	srv := newFlakyStream(t, func(hit int) (int, int) {
		if hit <= 2 {
			return http.StatusServiceUnavailable, 0
		}
		return 0, 0
	})
	s := retryStore(t, srv.URL, 5)
	var out bytes.Buffer
	if err := runOfflineQueueCtx(context.Background(), s, clientConfig{Server: srv.URL, Token: "tok"}, srv.Client(), &out); err != nil {
		t.Fatalf("%v\n%s", err, out.String())
	}
	if e, _ := s.Get("ep1"); e.State != offline.Done {
		t.Fatalf("entry %+v", e)
	}
	if srv.Hits() != 3 || strings.Count(out.String(), "retrying: Frieren S01E01") != 2 {
		t.Fatalf("hits %d, output:\n%s", srv.Hits(), out.String())
	}
}

func TestRunOfflineQueueGivesUpAfterMaxRetries(t *testing.T) {
	noBackoff(t)
	srv := newFlakyStream(t, func(int) (int, int) { return http.StatusBadGateway, 0 })
	s := retryStore(t, srv.URL, 2)
	var out bytes.Buffer
	err := runOfflineQueueCtx(context.Background(), s, clientConfig{Server: srv.URL, Token: "tok"}, srv.Client(), &out)
	if err == nil {
		t.Fatalf("want failure, output:\n%s", out.String())
	}
	if e, _ := s.Get("ep1"); e.State != offline.Failed {
		t.Fatalf("entry %+v", e)
	}
	if srv.Hits() != 3 {
		t.Fatalf("hits %d, want 1 + 2 retries", srv.Hits())
	}
}

func TestRunOfflineQueueDoesNotRetryPermanentFailures(t *testing.T) {
	noBackoff(t)
	srv := newFlakyStream(t, func(int) (int, int) { return http.StatusNotFound, 0 })
	s := retryStore(t, srv.URL, 5)
	if err := runOfflineQueueCtx(context.Background(), s, clientConfig{Server: srv.URL, Token: "tok"}, srv.Client(), io.Discard); err == nil {
		t.Fatal("want failure")
	}
	if srv.Hits() != 1 {
		t.Fatalf("hits %d, a 404 must not retry", srv.Hits())
	}
}

func TestRunOfflineQueueProgressResetsRetries(t *testing.T) {
	noBackoff(t)
	// Every early attempt drops after 1 KiB but makes progress, so one
	// allowed retry is enough however many drops there are.
	srv := newFlakyStream(t, func(hit int) (int, int) {
		if hit <= 4 {
			return 0, 1024
		}
		return 0, 0
	})
	s := retryStore(t, srv.URL, 1)
	var out bytes.Buffer
	if err := runOfflineQueueCtx(context.Background(), s, clientConfig{Server: srv.URL, Token: "tok"}, srv.Client(), &out); err != nil {
		t.Fatalf("%v\n%s", err, out.String())
	}
	data, _ := os.ReadFile(s.LocalPath("ep1"))
	if !bytes.Equal(data, episodeBytes) {
		t.Fatalf("resumed file differs (%d bytes), output:\n%s", len(data), out.String())
	}
	if srv.Hits() != 5 {
		t.Fatalf("hits %d, want 4 drops then a finish", srv.Hits())
	}
}

func TestRunOfflineQueueInterruptDuringBackoffKeepsEntryQueued(t *testing.T) {
	srv := newFlakyStream(t, func(int) (int, int) { return http.StatusServiceUnavailable, 0 })
	s := retryStore(t, srv.URL, 5)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	old := offlineBackoff
	offlineBackoff = func(int) time.Duration { cancel(); return time.Hour }
	t.Cleanup(func() { offlineBackoff = old })

	var out bytes.Buffer
	if err := runOfflineQueueCtx(ctx, s, clientConfig{Server: srv.URL, Token: "tok"}, srv.Client(), &out); err != nil {
		t.Fatalf("an interrupt is not a failure: %v\n%s", err, out.String())
	}
	if e, _ := s.Get("ep1"); e.State != offline.Queued {
		t.Fatalf("entry %+v, want queued for the next run", e)
	}
	if srv.Hits() != 1 || !strings.Contains(out.String(), "stopped: Frieren S01E01") {
		t.Fatalf("hits %d, output:\n%s", srv.Hits(), out.String())
	}
}
