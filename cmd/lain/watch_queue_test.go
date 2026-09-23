package main

import (
	"encoding/json"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"sync"
	"testing"
)

func TestEpisodeQueueFollowsCatalogOrderAcrossSeasons(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/api/catalog/e2/episodes" {
			t.Errorf("path=%s", r.URL.Path)
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"items":[{"id":"e1","title":"Series","season":1,"episode":1},{"id":"e2","title":"Series","season":1,"episode":2},{"id":"e3","title":"Series","season":1,"episode":3,"missing":true},{"id":"s2e1","title":"Series","season":2,"episode":1}]}`))
	}))
	defer srv.Close()
	items, err := episodeQueue(newAPIClient(srv.URL, "test"), apiItem{ID: "e2", Episode: 2})
	if err != nil {
		t.Fatal(err)
	}
	var ids []string
	for _, item := range items {
		ids = append(ids, item.ID)
	}
	if !reflect.DeepEqual(ids, []string{"e2", "s2e1"}) {
		t.Fatalf("queue=%v", ids)
	}
}

func TestEpisodeMonitorSavesMeasuredProgressPerItem(t *testing.T) {
	var mu sync.Mutex
	saved := make(map[string]apiProgress)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != "PUT" {
			t.Errorf("method=%s", r.Method)
		}
		var p apiProgress
		if err := json.NewDecoder(r.Body).Decode(&p); err != nil {
			t.Error(err)
		}
		p.ItemID = r.URL.Path[len("/api/items/") : len(r.URL.Path)-len("/progress")]
		mu.Lock()
		saved[p.ItemID] = p
		mu.Unlock()
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{}`))
	}))
	defer srv.Close()
	m := &episodeMonitor{client: newAPIClient(srv.URL, "test"), items: []apiItem{{ID: "e1"}, {ID: "e2"}}, last: make(map[int]apiProgress)}
	for _, sample := range []struct {
		index    int
		pos, dur float64
	}{{0, 42, 100}, {0, 42, 100}, {1, 96, 100}} {
		if err := m.save(sample.index, sample.pos, sample.dur); err != nil {
			t.Fatal(err)
		}
	}
	mu.Lock()
	defer mu.Unlock()
	if len(saved) != 2 || saved["e1"].PositionSec != 42 || saved["e1"].Completed ||
		saved["e2"].PositionSec != 96 || !saved["e2"].Completed {
		t.Fatalf("saved=%+v", saved)
	}
}

func TestQueueResumeSkipsCompletedAndShortProgress(t *testing.T) {
	if queueResume(apiProgress{PositionSec: 4}) != 0 || queueResume(apiProgress{PositionSec: 20, Completed: true}) != 0 ||
		queueResume(apiProgress{PositionSec: 20}) != 20 {
		t.Fatal("wrong resume policy")
	}
}

func TestVLCSubtitlePolicy(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Query().Get("client") != "vlc" {
			t.Errorf("missing VLC client: %s", r.URL.RawQuery)
		}
		switch r.URL.Path {
		case "/api/items/matching-audio/playback":
			_, _ = io.WriteString(w, `{"streams":[{"type":"audio","language":"por"},{"type":"subtitle","language":"por"}]}`)
		case "/api/items/matching-subtitle/playback":
			_, _ = io.WriteString(w, `{"streams":[{"type":"audio","language":"jpn"},{"type":"subtitle","language":"por"}]}`)
		case "/api/items/no-match/playback":
			_, _ = io.WriteString(w, `{"streams":[{"type":"audio","language":"jpn"},{"type":"subtitle","language":"eng"}]}`)
		default:
			w.WriteHeader(http.StatusNotFound)
		}
	}))
	defer srv.Close()
	client := newAPIClient(srv.URL, "test")
	for _, tc := range []struct {
		id  string
		off bool
	}{
		{"matching-audio", true}, {"matching-subtitle", false}, {"no-match", true},
	} {
		off, err := vlcSubtitleOff(client, apiItem{ID: tc.id}, "por")
		if err != nil || off != tc.off {
			t.Fatalf("%s: off=%v err=%v", tc.id, off, err)
		}
	}
}

func TestVLCQueueIndexUsesTokenlessTitle(t *testing.T) {
	socket := filepath.Join(t.TempDir(), "vlc.sock")
	listener, err := net.Listen("unix", socket)
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	go func() {
		conn, err := listener.Accept()
		if err != nil {
			return
		}
		defer conn.Close()
		buffer := make([]byte, 256)
		_, _ = conn.Read(buffer)
		_, _ = io.WriteString(conn, "> lain-episode-000001\n")
	}()
	index, err := vlcQueueIndex(socket, clientConfig{Server: "http://example", Token: "secret"}, []apiItem{{ID: "e1"}, {ID: "e2"}})
	if err != nil || index != 1 {
		t.Fatalf("index=%d err=%v", index, err)
	}
}

func TestLocalPlayerPreferenceKeepsLogin(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	if err := saveConfig(clientConfig{Server: "http://example", Token: "private"}); err != nil {
		t.Fatal(err)
	}
	if err := cmdPlayer([]string{"vlc"}); err != nil {
		t.Fatal(err)
	}
	cfg, err := loadConfig()
	if err != nil || cfg.Player != "vlc" || cfg.Token != "private" {
		t.Fatalf("cfg=%+v err=%v", cfg, err)
	}
	if err := cmdPlayer([]string{"other"}); err == nil {
		t.Fatal("invalid player accepted")
	}
}

func TestLanguageCommandUpdatesAccount(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	var stored string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "Bearer private" {
			w.WriteHeader(http.StatusUnauthorized)
			return
		}
		if r.URL.Path != "/api/me/preferences" {
			w.WriteHeader(http.StatusNotFound)
			return
		}
		if r.Method != "PATCH" {
			t.Errorf("method=%s", r.Method)
		}
		var body struct {
			PreferredLanguage string `json:"preferred_language"`
		}
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			t.Error(err)
		}
		stored = body.PreferredLanguage
		_, _ = io.WriteString(w, `{"preferred_language":"`+stored+`"}`)
	}))
	defer srv.Close()
	if err := saveConfig(clientConfig{Server: srv.URL, Token: "private"}); err != nil {
		t.Fatal(err)
	}
	if err := cmdLanguage([]string{"POR"}); err != nil || stored != "por" {
		t.Fatalf("set=%q err=%v", stored, err)
	}
	if err := cmdLanguage([]string{"--clear"}); err != nil || stored != "" {
		t.Fatalf("clear=%q err=%v", stored, err)
	}
	if err := cmdLanguage([]string{"pt-BR"}); err == nil {
		t.Fatal("invalid language accepted")
	}
	cfg, err := loadConfig()
	if err != nil || !strings.HasPrefix(cfg.Server, "http") || cfg.Token != "private" {
		t.Fatalf("cfg=%+v err=%v", cfg, err)
	}
	if _, err := os.Stat(filepath.Join(os.Getenv("XDG_CONFIG_HOME"), "lain", "config.json")); err != nil {
		t.Fatal(err)
	}
}

func TestMPVQueueSavesEachEpisodeFromOneProcess(t *testing.T) {
	binDir := t.TempDir()
	fake := `#!/bin/sh
for arg in "$@"; do
  case "$arg" in
    --script-opt=lain-state_dir=*) state="${arg#--script-opt=lain-state_dir=}" ;;
  esac
done
printf '{"position_sec":40,"duration_sec":100}' > "$state/0.json"
printf '{"position_sec":98,"duration_sec":100}' > "$state/1.json"
`
	if err := os.WriteFile(filepath.Join(binDir, "mpv"), []byte(fake), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", binDir+string(os.PathListSeparator)+os.Getenv("PATH"))
	var mu sync.Mutex
	saved := make(map[string]apiProgress)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method == "GET" && strings.HasSuffix(r.URL.Path, "/progress") {
			_, _ = io.WriteString(w, `{"position_sec":0,"duration_sec":100}`)
			return
		}
		if r.Method == "PUT" && strings.HasSuffix(r.URL.Path, "/progress") {
			var p apiProgress
			if err := json.NewDecoder(r.Body).Decode(&p); err != nil {
				t.Error(err)
			}
			mu.Lock()
			saved[strings.TrimSuffix(strings.TrimPrefix(r.URL.Path, "/api/items/"), "/progress")] = p
			mu.Unlock()
			_, _ = io.WriteString(w, `{}`)
			return
		}
		w.WriteHeader(http.StatusNotFound)
	}))
	defer srv.Close()
	client := newAPIClient(srv.URL, "private")
	items := []apiItem{{ID: "e1", Title: "Series", Season: 1, Episode: 1}, {ID: "e2", Title: "Series", Season: 1, Episode: 2}}
	if err := playEpisodeQueue(client, clientConfig{Server: srv.URL, Token: "private"}, items, "mpv"); err != nil {
		t.Fatal(err)
	}
	mu.Lock()
	defer mu.Unlock()
	if len(saved) != 2 || saved["e1"].PositionSec != 40 || saved["e1"].Completed ||
		saved["e2"].PositionSec != 98 || !saved["e2"].Completed {
		t.Fatalf("saved=%+v", saved)
	}
}
