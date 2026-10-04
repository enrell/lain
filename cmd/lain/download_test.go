package main

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/enrell/lain/internal/downloads"
	"github.com/enrell/lain/internal/offline"
)

// isolateDownloadEnv points config and data dirs at temp dirs, so no
// test reads or writes the developer's real offline store.
func isolateDownloadEnv(t *testing.T) (configHome, dataHome string) {
	t.Helper()
	configHome, dataHome = t.TempDir(), t.TempDir()
	t.Setenv("HOME", t.TempDir())
	t.Setenv("XDG_CONFIG_HOME", configHome)
	t.Setenv("XDG_DATA_HOME", dataHome)
	return configHome, dataHome
}

var episodeBytes = bytes.Repeat([]byte("tiny-episode\n"), 512)

// fakeServer serves the stream and progress endpoints the offline
// commands use, recording progress writes.
type fakeServer struct {
	*httptest.Server
	mu        sync.Mutex
	progress  map[string]apiProgress
	puts      []apiProgress
	streamHit int
}

func newFakeServer(t *testing.T) *fakeServer {
	t.Helper()
	f := &fakeServer{progress: map[string]apiProgress{}}
	f.Server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "Bearer tok" {
			w.WriteHeader(http.StatusUnauthorized)
			return
		}
		parts := strings.Split(strings.Trim(r.URL.Path, "/"), "/") // api items {id} x
		if len(parts) != 4 || parts[0] != "api" || parts[1] != "items" {
			http.NotFound(w, r)
			return
		}
		id := parts[2]
		f.mu.Lock()
		defer f.mu.Unlock()
		switch {
		case parts[3] == "stream":
			f.streamHit++
			w.Header().Set("ETag", `"e1"`)
			http.ServeContent(w, r, "x.mkv", time.Unix(1700000000, 0), bytes.NewReader(episodeBytes))
		case parts[3] == "progress" && r.Method == http.MethodGet:
			_ = json.NewEncoder(w).Encode(f.progress[id])
		case parts[3] == "progress" && r.Method == http.MethodPut:
			var p apiProgress
			_ = json.NewDecoder(r.Body).Decode(&p)
			p.ItemID = id
			f.progress[id] = p
			f.puts = append(f.puts, p)
			_ = json.NewEncoder(w).Encode(p)
		default:
			http.NotFound(w, r)
		}
	}))
	t.Cleanup(f.Close)
	return f
}

func openTestStore(t *testing.T, cfg offline.Config) *offline.Store {
	t.Helper()
	s, err := offline.Open(cfg)
	if err != nil {
		t.Fatal(err)
	}
	return s
}

func TestRunOfflineQueueDownloadsAndSkipsOtherServers(t *testing.T) {
	srv := newFakeServer(t)
	s := openTestStore(t, offline.Config{Dir: t.TempDir()})
	_, _ = s.Add(offline.Entry{ItemID: "ep1", Server: srv.URL, Title: "Frieren", Season: 1, Episode: 1}, "Frieren - 01.mkv")
	_, _ = s.Add(offline.Entry{ItemID: "ep2", Server: "http://elsewhere.test", Title: "Other"}, "o.mkv")
	_, _ = s.Add(offline.Entry{ItemID: "ep3", Server: srv.URL, Title: "Frieren", Season: 1, Episode: 3}, "Frieren - 03.mkv")
	_, _ = s.Pause("ep3")

	var out bytes.Buffer
	cfg := clientConfig{Server: srv.URL, Token: "tok"}
	if err := runOfflineQueueCtx(context.Background(), s, cfg, srv.Client(), &out); err != nil {
		t.Fatalf("%v\n%s", err, out.String())
	}
	data, _ := os.ReadFile(s.LocalPath("ep1"))
	if !bytes.Equal(data, episodeBytes) {
		t.Fatal("ep1 not stored")
	}
	if s.LocalPath("ep3") != "" || srv.streamHit != 1 {
		t.Fatalf("paused entry must not download (hits %d)", srv.streamHit)
	}
	if !strings.Contains(out.String(), "done: Frieren S01E01") || !strings.Contains(out.String(), "skipped: Other belongs to http://elsewhere.test") {
		t.Fatalf("output:\n%s", out.String())
	}
}

func TestRunOfflineQueueStopsWhenFull(t *testing.T) {
	srv := newFakeServer(t)
	s := openTestStore(t, offline.Config{Dir: t.TempDir(), Limits: downloads.Limits{MaxBytes: 100}})
	_, _ = s.Add(offline.Entry{ItemID: "ep1", Server: srv.URL, Title: "Big"}, "big.mkv")
	var out bytes.Buffer
	err := runOfflineQueueCtx(context.Background(), s, clientConfig{Server: srv.URL, Token: "tok"}, srv.Client(), &out)
	if err == nil || !strings.Contains(out.String(), "gc --watched") {
		t.Fatalf("err=%v out=%s", err, out.String())
	}
	if e, _ := s.Get("ep1"); e.State != offline.Failed || e.Code != downloads.CodeQuota {
		t.Fatalf("entry %+v", e)
	}
}

func TestSyncOfflinePushesPendingAndRefreshesWatched(t *testing.T) {
	srv := newFakeServer(t)
	srv.progress["ep2"] = apiProgress{Completed: true}
	s := openTestStore(t, offline.Config{Dir: t.TempDir()})
	for _, id := range []string{"ep1", "ep2"} {
		_, _ = s.Add(offline.Entry{ItemID: id, Server: srv.URL, Title: id}, id+".mkv")
	}
	cfg := clientConfig{Server: srv.URL, Token: "tok"}
	if err := runOfflineQueueCtx(context.Background(), s, cfg, srv.Client(), io.Discard); err != nil {
		t.Fatal(err)
	}
	s.RecordOffline("ep1", offline.Progress{PositionSec: 600, DurationSec: 1440})
	var out bytes.Buffer
	if err := syncOffline(s, cfg, newAPIClient(srv.URL, "tok"), &out); err != nil {
		t.Fatal(err)
	}
	if len(srv.puts) != 1 || srv.puts[0].ItemID != "ep1" || srv.puts[0].PositionSec != 600 || srv.puts[0].Completed {
		t.Fatalf("puts = %+v", srv.puts)
	}
	if len(s.Pending()) != 0 {
		t.Fatal("synced progress must clear")
	}
	if e, _ := s.Get("ep2"); !e.Watched {
		t.Fatal("server-completed copy must be marked watched")
	}
	if e, _ := s.Get("ep1"); e.Watched {
		t.Fatal("half-watched copy must not be watched")
	}
}

func TestSyncOfflineKeepsPendingWhenServerFails(t *testing.T) {
	s := openTestStore(t, offline.Config{Dir: t.TempDir()})
	_, _ = s.Add(offline.Entry{ItemID: "ep1", Title: "x"}, "x.mkv")
	s.RecordOffline("ep1", offline.Progress{PositionSec: 5, DurationSec: 10})
	dead := httptest.NewServer(http.NotFoundHandler())
	dead.Close()
	if err := syncOffline(s, clientConfig{Server: dead.URL, Token: "tok"}, newAPIClient(dead.URL, "tok"), io.Discard); err == nil {
		t.Fatal("unreachable server must error")
	}
	if len(s.Pending()) != 1 {
		t.Fatal("pending progress lost on a failed sync")
	}
}

func TestFindEntry(t *testing.T) {
	s := openTestStore(t, offline.Config{Dir: t.TempDir()})
	_, _ = s.Add(offline.Entry{ItemID: "abc123", Title: "Frieren", Season: 1, Episode: 1}, "a.mkv")
	_, _ = s.Add(offline.Entry{ItemID: "abd456", Title: "Frieren", Season: 1, Episode: 2}, "b.mkv")
	_, _ = s.Add(offline.Entry{ItemID: "zzz", Title: "Dandadan"}, "c.mkv")
	for ref, want := range map[string]string{"abc123": "abc123", "abd": "abd456", "dandadan": "zzz", "S01E02": "abd456"} {
		if e, err := findEntry(s, ref); err != nil || e.ItemID != want {
			t.Errorf("findEntry(%q) = %q %v, want %q", ref, e.ItemID, err, want)
		}
	}
	for _, ref := range []string{"ab", "frieren", "nothing", ""} {
		if _, err := findEntry(s, ref); err == nil {
			t.Errorf("findEntry(%q) must be ambiguous or missing", ref)
		}
	}
}

func TestDownloadConfigRoundTrip(t *testing.T) {
	_, dataHome := isolateDownloadEnv(t)
	cfg, err := loadOfflineConfig()
	if err != nil || cfg.Dir != filepath.Join(dataHome, "lain", "offline") || cfg.MaxBytes != offline.DefaultMaxBytes || !cfg.EvictWatched {
		t.Fatalf("defaults = %+v %v", cfg, err)
	}
	dir := t.TempDir()
	if err := downloadConfig([]string{"--dir", dir, "--max", "2GiB", "--min-free", "0", "--evict-watched", "off"}); err != nil {
		t.Fatal(err)
	}
	cfg, _ = loadOfflineConfig()
	if cfg.Dir != dir || cfg.MaxBytes != 2<<30 || cfg.MinFreeBytes != 0 || cfg.EvictWatched {
		t.Fatalf("saved = %+v", cfg)
	}
	for _, bad := range [][]string{{"--max", "-1"}, {"--min-free", "lots"}, {"--evict-watched", "maybe"}} {
		if err := downloadConfig(bad); err == nil {
			t.Errorf("config %v accepted", bad)
		}
	}
}

func TestWatchPrefersLocalCopy(t *testing.T) {
	isolateDownloadEnv(t)
	srv := newFakeServer(t)
	cfg, _ := loadOfflineConfig()
	cfg.MinFreeBytes = 0 // test temp dirs sit on small filesystems
	if err := saveOfflineConfig(cfg); err != nil {
		t.Fatal(err)
	}
	s := openTestStore(t, cfg)
	_, _ = s.Add(offline.Entry{ItemID: "ep1", Server: srv.URL, Title: "x"}, "x.mkv")
	if err := runOfflineQueueCtx(context.Background(), s, clientConfig{Server: srv.URL, Token: "tok"}, srv.Client(), io.Discard); err != nil {
		t.Fatal(err)
	}
	ccfg := clientConfig{Server: srv.URL, Token: "tok"}
	if got := queueURL(ccfg, apiItem{ID: "ep1"}); got != s.LocalPath("ep1") || got == "" {
		t.Fatalf("queueURL = %q, want the local copy", got)
	}
	if got := queueURL(ccfg, apiItem{ID: "ep2"}); !strings.HasPrefix(got, srv.URL+"/api/items/ep2/stream?token=") {
		t.Fatalf("queueURL without a copy = %q", got)
	}
}

func TestCmdDownloadUsageErrors(t *testing.T) {
	isolateDownloadEnv(t)
	if err := cmdDownload(nil); err == nil {
		t.Fatal("no subcommand must error")
	}
	if err := cmdDownload([]string{"explode"}); err == nil {
		t.Fatal("unknown subcommand must error")
	}
	if err := cmdDownload([]string{"add", "frieren"}); err == nil || !strings.Contains(err.Error(), "not logged in") {
		t.Fatalf("add without login: %v", err)
	}
	if err := cmdDownload([]string{"rm", "nothing"}); err == nil {
		t.Fatal("rm of an unknown entry must error")
	}
	if err := cmdDownload([]string{"list"}); err != nil {
		t.Fatalf("list on an empty store: %v", err)
	}
}
