package main

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-23

import (
	"encoding/json"
	"errors"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

func TestParsePlayerURL(t *testing.T) {
	cfg := clientConfig{Server: "https://media.example"}
	for _, player := range []string{"mpv", "vlc"} {
		id, got, err := parsePlayerURL("lain://play?server=https%3A%2F%2Fmedia.example&id=episode-1&player="+player, cfg)
		if err != nil || id != "episode-1" || got != player {
			t.Fatalf("id=%q player=%q err=%v", id, got, err)
		}
	}
	bad := []string{
		"lain://play?server=https%3A%2F%2Fevil.example&id=e&player=mpv",
		"lain://play?server=https%3A%2F%2Fmedia.example&id=e&player=other",
		"lain://play?server=https%3A%2F%2Fmedia.example&id=e%2F..&player=mpv",
		"lain://play?server=https%3A%2F%2Fmedia.example&id=e&player=mpv&player=vlc",
		"lain://play?server=https%3A%2F%2Fmedia.example&id=e&player=mpv&token=secret",
		"lain://play?server=https%3A%2F%2Fmedia.example&id=e&player=mpv#fragment",
	}
	for _, raw := range bad {
		if _, _, err := parsePlayerURL(raw, cfg); err == nil {
			t.Fatalf("accepted %q", raw)
		}
	}
}

func TestVLCPositionFromLocalSocket(t *testing.T) {
	sock := filepath.Join(t.TempDir(), "vlc.sock")
	listener, err := net.Listen("unix", sock)
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
		buf := make([]byte, 100)
		n, _ := conn.Read(buf)
		if !strings.Contains(string(buf[:n]), "get_time") {
			return
		}
		io.WriteString(conn, "VLC media player\n> 42\n> 100\n")
	}()
	pos, dur, err := vlcPosition(sock)
	if err != nil || pos != 42 || dur != 100 {
		t.Fatalf("position=%v duration=%v err=%v", pos, dur, err)
	}
}

func TestRunVLCReadsMeasuredProgress(t *testing.T) {
	sock := filepath.Join(t.TempDir(), "vlc.sock")
	listener, err := net.Listen("unix", sock)
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	stop := make(chan struct{})
	defer close(stop)
	go func() {
		for {
			conn, err := listener.Accept()
			if err != nil {
				return
			}
			io.CopyN(io.Discard, conn, 20)
			io.WriteString(conn, "> 96\n> 100\n")
			conn.Close()
			select {
			case <-stop:
				return
			default:
			}
		}
	}()
	cmd := exec.Command("sh", "-c", "sleep 2")
	cmd.Env = os.Environ()
	pos, dur, ok, err := runVLC(cmd, sock)
	if err != nil || !ok || pos != 96 || dur != 100 {
		t.Fatalf("position=%v duration=%v ok=%v err=%v", pos, dur, ok, err)
	}
}

func TestWatchRejectsUnknownPlayer(t *testing.T) {
	if err := cmdWatch([]string{"--player", "other"}); err == nil || !strings.Contains(err.Error(), "unsupported player") {
		t.Fatalf("unsupported player: %v", err)
	}
}

func TestPlayerExitError(t *testing.T) {
	failure := errors.New("failed")
	if err := playerExitErr("vlc", failure); err == nil || !errors.Is(err, failure) {
		t.Fatalf("VLC failure must surface after progress is saved: %v", err)
	}
	if err := playerExitErr("mpv", failure); err != nil {
		t.Fatalf("mpv plain quit must keep existing behavior: %v", err)
	}
	if err := playerExitErr("vlc", nil); err != nil {
		t.Fatalf("successful VLC exit: %v", err)
	}
}

func TestDryRunRedactsTokenForBothPlayers(t *testing.T) {
	for _, player := range []string{"mpv", "vlc"} {
		outR, outW, err := os.Pipe()
		if err != nil {
			t.Fatal(err)
		}
		original := os.Stdout
		os.Stdout = outW
		_, runErr := playOneWithPlayer(nil, clientConfig{Server: "https://media.example", Token: "secret-token"}, apiItem{ID: "item", Title: "Show"}, player, true)
		os.Stdout = original
		outW.Close()
		output, _ := io.ReadAll(outR)
		outR.Close()
		if runErr != nil || !strings.Contains(string(output), "[authenticated stream URL]") || strings.Contains(string(output), "secret-token") {
			t.Fatalf("player=%s output=%q err=%v", player, output, runErr)
		}
	}
}

func TestOpenURLUsesLocalLoginAndSavesProgress(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	installFakeMpv(t, 96, 100, "0")
	var saved map[string]any
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "Bearer local-token" {
			w.WriteHeader(http.StatusUnauthorized)
			return
		}
		switch {
		case r.Method == "GET" && r.URL.Path == "/api/catalog/episode-1":
			io.WriteString(w, `{"id":"episode-1","title":"Show"}`)
		case r.Method == "GET" && r.URL.Path == "/api/items/episode-1/progress":
			io.WriteString(w, `{"position_sec":12,"duration_sec":100}`)
		case r.Method == "PUT" && r.URL.Path == "/api/items/episode-1/progress":
			json.NewDecoder(r.Body).Decode(&saved)
			io.WriteString(w, `{"item_id":"episode-1","position_sec":96,"duration_sec":100,"completed":true}`)
		default:
			w.WriteHeader(http.StatusNotFound)
		}
	}))
	defer srv.Close()
	if err := saveConfig(clientConfig{Server: srv.URL, Token: "local-token"}); err != nil {
		t.Fatal(err)
	}
	link := "lain://play?server=" + url.QueryEscape(srv.URL) + "&id=episode-1&player=mpv"
	if err := cmdOpenURL([]string{link}); err != nil {
		t.Fatal(err)
	}
	if saved["completed"] != true || saved["position_sec"] != float64(96) {
		t.Fatalf("progress=%v", saved)
	}
	if err := cmdOpenURL(nil); err == nil {
		t.Fatal("missing link must fail")
	}
	if err := cmdOpenURL([]string{"lain://play?server=https%3A%2F%2Fevil.example&id=e&player=mpv"}); err == nil {
		t.Fatal("different server must fail")
	}
}

func TestOpenURLRequiresLocalLogin(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	if err := cmdOpenURL([]string{"lain://play?server=https%3A%2F%2Fmedia.example&id=e&player=mpv"}); err == nil || !strings.Contains(err.Error(), "not logged in") {
		t.Fatalf("missing login: %v", err)
	}
}

func TestWatchDirectIDAndVLCResume(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	dir := t.TempDir()
	capture := filepath.Join(dir, "args")
	t.Setenv("CAPTURE", capture)
	if err := os.WriteFile(filepath.Join(dir, "vlc"), []byte("#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$CAPTURE\"\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", dir+string(os.PathListSeparator)+os.Getenv("PATH"))
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/api/catalog/e1":
			io.WriteString(w, `{"id":"e1","title":"Show"}`)
		case "/api/items/e1/progress":
			io.WriteString(w, `{"position_sec":12,"duration_sec":100}`)
		default:
			w.WriteHeader(http.StatusNotFound)
		}
	}))
	defer srv.Close()
	if err := saveConfig(clientConfig{Server: srv.URL, Token: "tok"}); err != nil {
		t.Fatal(err)
	}
	if err := cmdWatch([]string{"--id", "e1", "--player", "vlc", "--once"}); err != nil {
		t.Fatal(err)
	}
	args, err := os.ReadFile(capture)
	if err != nil || !strings.Contains(string(args), "--start-time=12") || !strings.Contains(string(args), "/api/items/e1/stream?token=tok") {
		t.Fatalf("VLC args=%q err=%v", args, err)
	}
	if err := cmdWatch([]string{"--id", "missing", "--dry-run"}); err == nil {
		t.Fatal("unknown item must fail before opening a player")
	}
}

func TestWatchSurfacesNextEpisodeFailureOnTTY(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	installFakeMpv(t, 96, 100, "0")
	original := watchIsATTY
	watchIsATTY = func(*os.File) bool { return true }
	t.Cleanup(func() { watchIsATTY = original })
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/api/catalog/e1":
			io.WriteString(w, `{"id":"e1","title":"Show","season":1,"episode":1}`)
		case "/api/items/e1/progress":
			if r.Method == "PUT" {
				io.WriteString(w, `{"item_id":"e1","position_sec":96,"duration_sec":100,"completed":true}`)
			} else {
				io.WriteString(w, "null")
			}
		case "/api/search":
			w.WriteHeader(http.StatusServiceUnavailable)
		default:
			w.WriteHeader(http.StatusNotFound)
		}
	}))
	defer srv.Close()
	if err := saveConfig(clientConfig{Server: srv.URL, Token: "tok"}); err != nil {
		t.Fatal(err)
	}
	if err := cmdWatch([]string{"--id", "e1"}); err == nil || !strings.Contains(err.Error(), "503") {
		t.Fatalf("next episode error must surface: %v", err)
	}
}

func TestInstallPlayerHandler(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_DATA_HOME", dir)
	binDir := t.TempDir()
	xdg := filepath.Join(binDir, "xdg-mime")
	if err := os.WriteFile(xdg, []byte("#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$XDG_DATA_HOME/xdg-args\"\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", binDir+string(os.PathListSeparator)+os.Getenv("PATH"))
	if err := cmdInstallPlayerHandler(); err != nil {
		t.Fatal(err)
	}
	entry, err := os.ReadFile(filepath.Join(dir, "applications", "lain-player.desktop"))
	if err != nil || !strings.Contains(string(entry), "open-url %u") || !strings.Contains(string(entry), "MimeType=x-scheme-handler/lain;") || !strings.Contains(string(entry), "Terminal=true") {
		t.Fatalf("entry=%q err=%v", entry, err)
	}
	args, err := os.ReadFile(filepath.Join(dir, "xdg-args"))
	if err != nil || string(args) != "default\nlain-player.desktop\nx-scheme-handler/lain\n" {
		t.Fatalf("xdg args=%q err=%v", args, err)
	}
}

func TestInstallPlayerHandlerErrors(t *testing.T) {
	originalExecutable, originalEval, originalHome := playerExecutable, playerEvalSymlinks, playerUserHome
	t.Cleanup(func() {
		playerExecutable, playerEvalSymlinks, playerUserHome = originalExecutable, originalEval, originalHome
	})
	playerExecutable = func() (string, error) { return "", errors.New("executable unavailable") }
	if err := cmdInstallPlayerHandler(); err == nil {
		t.Fatal("executable error must surface")
	}
	playerExecutable = func() (string, error) { return "/nonexistent/lain", nil }
	playerEvalSymlinks = func(string) (string, error) { return "", errors.New("broken symlink") }
	if err := cmdInstallPlayerHandler(); err == nil {
		t.Fatal("symlink error must surface")
	}
	playerEvalSymlinks = func(string) (string, error) { return "/test/lain", nil }
	t.Setenv("XDG_DATA_HOME", "")
	playerUserHome = func() (string, error) { return "", errors.New("home unavailable") }
	if err := cmdInstallPlayerHandler(); err == nil {
		t.Fatal("home error must surface")
	}
	playerUserHome = func() (string, error) { return t.TempDir(), nil }
	dataFile := filepath.Join(t.TempDir(), "data-file")
	if err := os.WriteFile(dataFile, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	t.Setenv("XDG_DATA_HOME", dataFile)
	if err := cmdInstallPlayerHandler(); err == nil {
		t.Fatal("mkdir error must surface")
	}
	dataDir := t.TempDir()
	t.Setenv("XDG_DATA_HOME", dataDir)
	if err := os.MkdirAll(filepath.Join(dataDir, "applications", "lain-player.desktop"), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := cmdInstallPlayerHandler(); err == nil {
		t.Fatal("write error must surface")
	}
	t.Setenv("XDG_DATA_HOME", t.TempDir())
	binDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(binDir, "xdg-mime"), []byte("#!/bin/sh\nexit 1\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", binDir+string(os.PathListSeparator)+os.Getenv("PATH"))
	if err := cmdInstallPlayerHandler(); err == nil || !strings.Contains(err.Error(), "register") {
		t.Fatalf("registration error: %v", err)
	}
}

func TestVLCPositionRejectsBrokenResponse(t *testing.T) {
	for _, response := range []string{"not a number\n", strings.Repeat("x", 70000)} {
		sock := filepath.Join(t.TempDir(), "vlc.sock")
		listener, err := net.Listen("unix", sock)
		if err != nil {
			t.Fatal(err)
		}
		go func() {
			conn, err := listener.Accept()
			if err != nil {
				return
			}
			defer conn.Close()
			io.WriteString(conn, response)
		}()
		if _, _, err := vlcPosition(sock); err == nil {
			t.Fatal("broken VLC response must fail")
		}
		listener.Close()
	}
}
