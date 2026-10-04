package downloads

import (
	"bytes"
	"context"
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// payload is a tiny fixture: big enough to span several chunks of a
// small test chunk, small enough to never matter for disk space.
var payload = bytes.Repeat([]byte("lain-fixture-0123456789\n"), 4096) // ~96 KiB

func origin(t *testing.T, body []byte, etag string) *httptest.Server {
	t.Helper()
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if etag != "" {
			w.Header().Set("ETag", etag)
		}
		w.Header().Set("Content-Disposition", `attachment; filename="Frieren - 01.cbz"`)
		http.ServeContent(w, r, "x.bin", time.Unix(1700000000, 0), bytes.NewReader(body))
	}))
	t.Cleanup(srv.Close)
	return srv
}

func TestFetchWholeFile(t *testing.T) {
	srv := origin(t, payload, `"v1"`)
	part := filepath.Join(t.TempDir(), "a.part")
	var last int64
	res, err := Fetch(context.Background(), srv.Client(), Request{
		URL: srv.URL, Part: part,
		Progress: func(done, total int64) { last = done },
	})
	if err != nil {
		t.Fatal(err)
	}
	got, _ := os.ReadFile(part)
	if !bytes.Equal(got, payload) || res.Bytes != int64(len(payload)) || res.Total != int64(len(payload)) {
		t.Fatalf("bytes=%d total=%d file=%d", res.Bytes, res.Total, len(got))
	}
	if last != int64(len(payload)) || res.Validator != `"v1"` || res.Filename != "Frieren - 01.cbz" {
		t.Fatalf("progress=%d validator=%q name=%q", last, res.Validator, res.Filename)
	}
}

func TestFetchResumesWithMatchingValidator(t *testing.T) {
	srv := origin(t, payload, `"v1"`)
	part := filepath.Join(t.TempDir(), "a.part")
	_ = os.WriteFile(part, payload[:1000], 0o644)
	var ranges []string
	client := srv.Client()
	client.Transport = roundTripFunc(func(r *http.Request) (*http.Response, error) {
		ranges = append(ranges, r.Header.Get("Range"))
		return http.DefaultTransport.RoundTrip(r)
	})
	res, err := Fetch(context.Background(), client, Request{URL: srv.URL, Part: part, Validator: `"v1"`})
	if err != nil {
		t.Fatal(err)
	}
	got, _ := os.ReadFile(part)
	if !bytes.Equal(got, payload) || res.Restarted {
		t.Fatalf("resume corrupted the file (len %d, restarted %v)", len(got), res.Restarted)
	}
	if len(ranges) != 1 || ranges[0] != "bytes=1000-" {
		t.Fatalf("ranges = %q", ranges)
	}
}

func TestFetchRestartsWhenOriginChanged(t *testing.T) {
	srv := origin(t, payload, `"v2"`)
	part := filepath.Join(t.TempDir(), "a.part")
	_ = os.WriteFile(part, []byte("bytes of an older representation"), 0o644)
	res, err := Fetch(context.Background(), srv.Client(), Request{URL: srv.URL, Part: part, Validator: `"v1"`})
	if err != nil {
		t.Fatal(err)
	}
	got, _ := os.ReadFile(part)
	if !bytes.Equal(got, payload) || !res.Restarted {
		t.Fatalf("stale partial must be discarded (restarted=%v len=%d)", res.Restarted, len(got))
	}
}

func TestFetchWithoutValidatorNeverAsksForARange(t *testing.T) {
	// No validator: a 206 could belong to a different file, so the part
	// is rebuilt from zero.
	srv := origin(t, payload, "")
	part := filepath.Join(t.TempDir(), "a.part")
	_ = os.WriteFile(part, []byte("junk"), 0o644)
	if _, err := Fetch(context.Background(), srv.Client(), Request{URL: srv.URL, Part: part}); err != nil {
		t.Fatal(err)
	}
	got, _ := os.ReadFile(part)
	if !bytes.Equal(got, payload) {
		t.Fatal("part must be rebuilt")
	}
}

func TestFetchOriginIgnoringRangesRestarts(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("ETag", `"v1"`)
		_, _ = w.Write(payload) // 200 with the whole body, Range ignored
	}))
	defer srv.Close()
	part := filepath.Join(t.TempDir(), "a.part")
	_ = os.WriteFile(part, payload[:500], 0o644)
	res, err := Fetch(context.Background(), srv.Client(), Request{URL: srv.URL, Part: part, Validator: `"v1"`})
	if err != nil {
		t.Fatal(err)
	}
	got, _ := os.ReadFile(part)
	if !bytes.Equal(got, payload) || !res.Restarted {
		t.Fatalf("a 200 to a range request must truncate (len %d)", len(got))
	}
}

func TestFetchCompletePartAnswered416(t *testing.T) {
	srv := origin(t, payload, `"v1"`)
	part := filepath.Join(t.TempDir(), "a.part")
	_ = os.WriteFile(part, payload, 0o644)
	res, err := Fetch(context.Background(), srv.Client(), Request{URL: srv.URL, Part: part, Validator: `"v1"`})
	if err != nil || res.Bytes != int64(len(payload)) {
		t.Fatalf("complete part: %v bytes=%d", err, res.Bytes)
	}
}

func TestFetchCancelKeepsPart(t *testing.T) {
	release := make(chan struct{})
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Length", "2000")
		_, _ = w.Write(payload[:1000])
		w.(http.Flusher).Flush()
		select {
		case <-r.Context().Done():
		case <-release:
		}
	}))
	defer srv.Close()
	defer close(release)
	part := filepath.Join(t.TempDir(), "a.part")
	ctx, cancel := context.WithCancel(context.Background())
	res, err := Fetch(ctx, srv.Client(), Request{URL: srv.URL, Part: part, Progress: func(done, _ int64) {
		if done >= 1000 {
			cancel()
		}
	}})
	if !errors.Is(err, context.Canceled) {
		t.Fatalf("err = %v, want context.Canceled", err)
	}
	fi, _ := os.Stat(part)
	if res.Bytes != 1000 || fi.Size() != 1000 {
		t.Fatalf("bytes=%d file=%d, want the 1000 written bytes kept", res.Bytes, fi.Size())
	}
}

func TestFetchBudgetRefusesKnownSizeUpFront(t *testing.T) {
	srv := origin(t, payload, `"v1"`)
	part := filepath.Join(t.TempDir(), "a.part")
	_, err := Fetch(context.Background(), srv.Client(), Request{URL: srv.URL, Part: part,
		Reserve: func(n int64) error { return Limits{MaxBytes: 10}.Check(t.TempDir(), 0, n) }})
	if CodeOf(err) != CodeQuota {
		t.Fatalf("err = %v, want %s", err, CodeQuota)
	}
	if _, statErr := os.Stat(part); !os.IsNotExist(statErr) {
		t.Fatal("a refused download must not create its part")
	}
}

func TestFetchBudgetStopsUnknownSizeMidway(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		for i := 0; i < 8; i++ { // chunked: no Content-Length
			_, _ = w.Write(payload[:8000])
			w.(http.Flusher).Flush()
		}
	}))
	defer srv.Close()
	part := filepath.Join(t.TempDir(), "a.part")
	var used int64
	res, err := Fetch(context.Background(), srv.Client(), Request{URL: srv.URL, Part: part,
		Reserve: func(n int64) error {
			if err := (Limits{MaxBytes: 20000}).Check(t.TempDir(), used, n); err != nil {
				return err
			}
			used += n
			return nil
		}})
	if CodeOf(err) != CodeQuota {
		t.Fatalf("err = %v, want %s", err, CodeQuota)
	}
	if res.Bytes > 20000 {
		t.Fatalf("wrote %d bytes past a 20000 budget", res.Bytes)
	}
}

func TestFetchHTTPErrors(t *testing.T) {
	srv := httptest.NewServer(http.NotFoundHandler())
	defer srv.Close()
	_, err := Fetch(context.Background(), srv.Client(), Request{URL: srv.URL, Part: filepath.Join(t.TempDir(), "p")})
	if CodeOf(err) != CodeHTTP || !strings.Contains(err.Error(), "404") {
		t.Fatalf("err = %v", err)
	}
	_, err = Fetch(context.Background(), nil, Request{URL: "::bad", Part: filepath.Join(t.TempDir(), "p")})
	if CodeOf(err) == "" {
		t.Fatalf("bad url must be typed: %v", err)
	}
}

func TestParseContentRange(t *testing.T) {
	for in, want := range map[string][3]int64{
		"bytes 10-99/100": {10, 100, 1},
		"bytes 0-0/*":     {0, -1, 1},
		"bytes */100":     {0, 100, 1},
		"bytes x-1/2":     {0, 0, 0},
		"items 1-2/3":     {0, 0, 0},
		"bytes 1-2":       {0, 0, 0},
	} {
		s, tot, ok := parseContentRange(in)
		if (ok && want[2] == 0) || (!ok && want[2] == 1) || (ok && (s != want[0] || tot != want[1])) {
			t.Errorf("%q -> %d %d %v", in, s, tot, ok)
		}
	}
}

type roundTripFunc func(*http.Request) (*http.Response, error)

func (f roundTripFunc) RoundTrip(r *http.Request) (*http.Response, error) { return f(r) }
