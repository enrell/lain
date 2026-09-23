package main

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-22

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"

	"github.com/enrell/lain/internal/backup"
	"github.com/enrell/lain/internal/kv"
	"strings"
	"testing"
	"time"
)

func TestAPILoginOutcomes(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/api/auth/login" || r.Method != "POST" {
			t.Errorf("unexpected request %s %s", r.Method, r.URL.Path)
		}
		w.Write([]byte(`{"token":"tok123"}`))
	}))
	defer srv.Close()
	tok, err := apiLogin(srv.URL, "u", "p")
	if err != nil || tok != "tok123" {
		t.Fatalf("login: %v %q", err, tok)
	}

	bad := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(401)
		w.Write([]byte("denied"))
	}))
	defer bad.Close()
	if _, err := apiLogin(bad.URL, "u", "p"); err == nil || !strings.Contains(err.Error(), "denied") {
		t.Fatalf("non-200 must surface the body: %v", err)
	}

	mangled := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte(`{"token":""}`))
	}))
	defer mangled.Close()
	if _, err := apiLogin(mangled.URL, "u", "p"); err == nil {
		t.Fatal("empty token must fail")
	}
	if _, err := apiLogin("http://127.0.0.1:1", "u", "p"); err == nil {
		t.Fatal("unreachable server must fail")
	}
}

func TestAPIClientGetPut(t *testing.T) {
	var sawAuth, sawQuery string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		sawAuth = r.Header.Get("Authorization")
		sawQuery = r.URL.RawQuery
		w.Header().Set("Content-Type", "application/json")
		if strings.HasSuffix(r.URL.Path, "fail") {
			w.WriteHeader(503)
			w.Write([]byte("down"))
			return
		}
		if strings.HasSuffix(r.URL.Path, "badjson") {
			w.Write([]byte("not json"))
			return
		}
		w.Write([]byte(`{"items":[],"total":0}`))
	}))
	defer srv.Close()
	c := newAPIClient(srv.URL+"/", "tok")
	if c.server != srv.URL {
		t.Fatalf("trailing slash must trim: %q", c.server)
	}
	var page apiPage
	if err := c.get("/api/search", map[string]string{"q": "a b"}, &page); err != nil {
		t.Fatal(err)
	}
	if sawAuth != "Bearer tok" || !strings.Contains(sawQuery, "q=a+b") {
		t.Fatalf("auth=%q query=%q", sawAuth, sawQuery)
	}
	if err := c.get("/api/fail", nil, &page); err == nil || !strings.Contains(err.Error(), "503") {
		t.Fatalf("status error: %v", err)
	}
	if err := c.get("/api/badjson", nil, &page); err == nil {
		t.Fatal("bad json must fail")
	}
	if err := c.get("/api/search", nil, nil); err != nil {
		t.Fatal("nil out must skip decode")
	}
	var saved apiProgress
	if err := c.put("/api/items/x/progress", map[string]any{"position_sec": 1.0}, &saved); err != nil {
		t.Fatal(err)
	}
	if err := c.put("/api/fail", nil, nil); err == nil || !strings.Contains(err.Error(), "503") {
		t.Fatalf("put status: %v", err)
	}
	if err := c.put("/api/badjson", nil, &saved); err == nil {
		t.Fatal("put bad json must fail")
	}
	if err := c.get("://bad-url", nil, nil); err == nil {
		t.Fatal("bad request URL must fail")
	}
}

func TestResolveNextPaths(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		switch {
		case r.URL.Path == "/api/me/continue":
			w.Write([]byte(`[{"item_id":"done","position_sec":1,"duration_sec":2,"completed":true},{"item_id":"gone","completed":false},{"item_id":"next","completed":false}]`))
		case r.URL.Path == "/api/catalog/gone":
			w.WriteHeader(404)
		case r.URL.Path == "/api/catalog/next":
			w.Write([]byte(`{"id":"next","title":"Show","episode":3}`))
		default:
			w.WriteHeader(404)
		}
	}))
	defer srv.Close()
	c := newAPIClient(srv.URL, "tok")
	it, err := resolveNext(c)
	if err != nil || it.ID != "next" {
		t.Fatalf("resolveNext: %v %+v", err, it)
	}

	empty := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte(`[]`))
	}))
	defer empty.Close()
	if _, err := resolveNext(newAPIClient(empty.URL, "t")); err == nil {
		t.Fatal("empty feed must fail")
	}
}

func TestResolveQueryArgsEdges(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		q := r.URL.Query().Get("q")
		w.Header().Set("Content-Type", "application/json")
		if q == "one" {
			w.Write([]byte(`{"items":[{"id":"x","title":"Only"}],"total":1}`))
			return
		}
		w.Write([]byte(`{"items":[],"total":0}`))
	}))
	defer srv.Close()
	c := newAPIClient(srv.URL, "tok")
	devNull, _ := os.Open(os.DevNull)
	defer devNull.Close()

	if _, err := resolveQueryArgs(c, "zzz", devNull, devNull, nil); err == nil {
		t.Fatal("no results must fail")
	}
	it, err := resolveQueryArgs(c, "one", devNull, devNull, nil)
	if err != nil || it.ID != "x" {
		t.Fatalf("single result plays directly: %v %+v", err, it)
	}
}

func TestPickItemTruncatesAndValidates(t *testing.T) {
	var items []apiItem
	for i := 0; i < 25; i++ {
		items = append(items, apiItem{ID: string(rune('a' + i)), Title: "T", Year: 2020})
	}
	inR, inW, _ := os.Pipe()
	outR, outW, _ := os.Pipe()
	go func() {
		inW.WriteString("20\n")
		inW.Close()
	}()
	picked, err := pickItem(items, inR, outW)
	outW.Close()
	if err != nil {
		t.Fatal(err)
	}
	if picked.ID != "t" { // item 20 (0-indexed 19) is 't'
		t.Fatalf("pick 20 must map to the 20th item, got %+v", picked)
	}
	buf := make([]byte, 8192)
	n, _ := outR.Read(buf)
	out := string(buf[:n])
	if !strings.Contains(out, "pick [1-20]") || !strings.Contains(out, "(2020)") {
		t.Fatalf("listing must cap at 20 and show year: %q", out)
	}
	// EOF on the input is an error, not a crash.
	inR2, inW2, _ := os.Pipe()
	inW2.Close()
	if _, err := pickItem(items, inR2, outW); err == nil {
		t.Fatal("EOF must fail")
	}
}

func TestCompletedMarkAndRunErr(t *testing.T) {
	if completedMark(true) != " (completed)" || completedMark(false) != "" {
		t.Fatal("completedMark")
	}
	if err := runErrToNil(nil, "x"); err != nil {
		t.Fatal("nil runErr stays nil")
	}
	if err := runErrToNil(os.ErrNotExist, ""); err != nil {
		t.Fatal("mpv quit with saved progress is not an error")
	}
	if err := runErrToNil(os.ErrNotExist, "no progress"); err == nil || err.Error() != "no progress" {
		t.Fatalf("note must surface: %v", err)
	}
}

func TestCmdLogout(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	t.Setenv("XDG_CONFIG_HOME", "")
	if err := cmdLogout(nil); err != nil {
		t.Fatalf("logout without config must succeed: %v", err)
	}
	if err := saveConfig(clientConfig{Server: "s", Token: "t"}); err != nil {
		t.Fatal(err)
	}
	if err := cmdLogout(nil); err != nil {
		t.Fatal(err)
	}
	if _, err := loadConfig(); err == nil {
		t.Fatal("config must be gone after logout")
	}
}

func TestCmdLoginWithEnvPassword(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	t.Setenv("XDG_CONFIG_HOME", "")
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var body map[string]string
		json.NewDecoder(r.Body).Decode(&body)
		if body["username"] != "lain" || body["password"] != "secretpw" {
			w.WriteHeader(401)
			return
		}
		w.Write([]byte(`{"token":"tok"}`))
	}))
	defer srv.Close()
	t.Setenv("LAIN_PASSWORD", "secretpw")
	if err := cmdLogin([]string{"--server", srv.URL, "--username", "lain"}); err != nil {
		t.Fatal(err)
	}
	cfg, err := loadConfig()
	if err != nil || cfg.Token != "tok" || cfg.Server != srv.URL {
		t.Fatalf("config=%+v err=%v", cfg, err)
	}
}

func TestCmdWatchNotLoggedIn(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	t.Setenv("XDG_CONFIG_HOME", "")
	if err := cmdWatch([]string{"--next"}); err == nil || !strings.Contains(err.Error(), "not logged in") {
		t.Fatalf("watch must require login: %v", err)
	}
}

// fake mpv: parses --script-opts=lain-state=PATH and writes progress.
func installFakeMpv(t *testing.T, pos, dur float64, exitCode string) {
	t.Helper()
	dir := t.TempDir()
	script := "#!/bin/sh\nfor a in \"$@\"; do\n  case \"$a\" in\n    *lain-state=*) f=${a#*lain-state=};;\n  esac\ndone\n" +
		"printf '{\"position_sec\":" + ftoa(pos) + ",\"duration_sec\":" + ftoa(dur) + "}' > \"$f\"\nexit " + exitCode + "\n"
	p := filepath.Join(dir, "mpv")
	if err := os.WriteFile(p, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", dir+string(os.PathListSeparator)+os.Getenv("PATH"))
}

func ftoa(f float64) string {
	b, _ := json.Marshal(f)
	return string(b)
}

func TestPlayOneWithFakeMpv(t *testing.T) {
	installFakeMpv(t, 96, 100, "0")
	var put map[string]any
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		json.NewDecoder(r.Body).Decode(&put)
		w.Write([]byte(`{"item_id":"e1","position_sec":96,"duration_sec":100,"completed":true}`))
	}))
	defer srv.Close()
	client := newAPIClient(srv.URL, "tok")
	res, err := playOne(client, clientConfig{Server: srv.URL, Token: "t"}, apiItem{ID: "e1", Title: "Show", Season: 1, Episode: 2}, false)
	if err != nil {
		t.Fatal(err)
	}
	if !res.completed || res.position != 96 || res.duration != 100 {
		t.Fatalf("result=%+v", res)
	}
	if put["completed"] != true {
		t.Fatalf("95%% watched must report completed: %v", put)
	}
}

func TestPlayOneMpvQuitKeepsProgress(t *testing.T) {
	installFakeMpv(t, 10, 100, "2") // plain quit still saves progress
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.Write([]byte(`{"item_id":"e1","position_sec":10,"duration_sec":100,"completed":false}`))
	}))
	defer srv.Close()
	client := newAPIClient(srv.URL, "tok")
	res, err := playOne(client, clientConfig{Server: srv.URL, Token: "t"}, apiItem{ID: "e1", Title: "Show"}, false)
	if err != nil {
		t.Fatalf("mpv non-zero exit is not an error once progress saved: %v", err)
	}
	if res.completed {
		t.Fatal("10%% is not completed")
	}
}

func TestPlayOneNoStateFile(t *testing.T) {
	// mpv that never writes the state file and exits non-zero.
	dir := t.TempDir()
	p := filepath.Join(dir, "mpv")
	if err := os.WriteFile(p, []byte("#!/bin/sh\nexit 1\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", dir+string(os.PathListSeparator)+os.Getenv("PATH"))
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {}))
	defer srv.Close()
	client := newAPIClient(srv.URL, "t")
	_, err := playOne(client, clientConfig{Server: srv.URL, Token: "t"}, apiItem{ID: "e1"}, false)
	if err == nil || !strings.Contains(err.Error(), "no progress") {
		t.Fatalf("missing state + mpv error must surface the note: %v", err)
	}
}

func TestPlayOneDryRun(t *testing.T) {
	res, err := playOne(nil, clientConfig{Server: "http://x", Token: "t"}, apiItem{ID: "e1", Title: "Show", Episode: 3}, true)
	if err != nil || res.completed {
		t.Fatalf("dry run: %v %+v", err, res)
	}
}

func TestCmdWatchDryRunNext(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	t.Setenv("XDG_CONFIG_HOME", "")
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		switch r.URL.Path {
		case "/api/me/continue":
			w.Write([]byte(`[{"item_id":"e1","completed":false}]`))
		case "/api/catalog/e1":
			w.Write([]byte(`{"id":"e1","title":"Show","season":1,"episode":1}`))
		default:
			w.WriteHeader(404)
		}
	}))
	defer srv.Close()
	if err := saveConfig(clientConfig{Server: srv.URL, Token: "tok"}); err != nil {
		t.Fatal(err)
	}
	if err := cmdWatch([]string{"--next", "--dry-run"}); err != nil {
		t.Fatalf("dry-run watch: %v", err)
	}
}

func TestCmdDoctorAndPlugins(t *testing.T) {
	dir := t.TempDir()
	if err := cmdDoctor([]string{"--data-dir", dir}); err != nil {
		t.Fatalf("doctor: %v", err)
	}
	if err := cmdPlugins([]string{"--data-dir", filepath.Join(dir, "db")}); err != nil {
		t.Fatalf("plugins: %v", err)
	}
}

func TestCmdServeValidationErrors(t *testing.T) {
	if err := cmdServe([]string{"--transcode-cache-size", "bogus", "--data-dir", t.TempDir()}); err == nil {
		t.Fatal("bad cache size must fail before listening")
	}
	if err := cmdServe([]string{"--log-level", "bogus", "--data-dir", t.TempDir()}); err == nil {
		t.Fatal("bad log level must fail before listening")
	}
}

func TestFlagOrEnvPrecedence(t *testing.T) {
	t.Setenv("LAIN_TEST_X", "envval")
	if got := flagOrEnv([]string{"--test-x", "flagval"}, "test-x", "LAIN_TEST_X", "def"); got != "flagval" {
		t.Fatalf("flag wins: %q", got)
	}
	if got := flagOrEnv(nil, "test-x", "LAIN_TEST_X", "def"); got != "envval" {
		t.Fatalf("env second: %q", got)
	}
	t.Setenv("LAIN_TEST_X", "")
	if got := flagOrEnv(nil, "test-x", "LAIN_TEST_X", "def"); got != "def" {
		t.Fatalf("default last: %q", got)
	}
	for _, v := range []string{"0", "false", "FALSE"} {
		if !envOff(v) {
			t.Fatalf("envOff(%q)", v)
		}
	}
	for _, v := range []string{"", "1", "true", "no"} {
		if envOff(v) {
			t.Fatalf("envOff(%q) must be false", v)
		}
	}
}

func TestCmdServeBadDataDir(t *testing.T) {
	f, err := os.CreateTemp(t.TempDir(), "file")
	if err != nil {
		t.Fatal(err)
	}
	f.Close()
	// A file as data dir fails server construction before listening.
	if err := cmdServe([]string{"--data-dir", f.Name()}); err == nil {
		t.Fatal("file-as-datadir must fail")
	}
}

func TestCmdLoginReadsUsernameFromStdin(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	t.Setenv("XDG_CONFIG_HOME", "")
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var body map[string]string
		json.NewDecoder(r.Body).Decode(&body)
		if body["username"] == "fromstdin" && body["password"] == "pw" {
			w.Write([]byte(`{"token":"tok"}`))
			return
		}
		w.WriteHeader(401)
	}))
	defer srv.Close()
	t.Setenv("LAIN_PASSWORD", "pw")
	inR, inW, _ := os.Pipe()
	go func() {
		inW.WriteString("fromstdin\n")
		inW.Close()
	}()
	old := os.Stdin
	os.Stdin = inR
	defer func() { os.Stdin = old }()
	if err := cmdLogin([]string{"--server", srv.URL}); err != nil {
		t.Fatal(err)
	}
	cfg, _ := loadConfig()
	if cfg.Token != "tok" {
		t.Fatalf("config=%+v", cfg)
	}
}

func TestCmdLoginPasswordPromptFailsOnPipe(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	t.Setenv("XDG_CONFIG_HOME", "")
	t.Setenv("LAIN_PASSWORD", "")
	inR, inW, _ := os.Pipe()
	go func() {
		inW.WriteString("user\n")
		inW.Close()
	}()
	old := os.Stdin
	os.Stdin = inR
	defer func() { os.Stdin = old }()
	// term.ReadPassword on a non-tty fd errors; cmdLogin must surface it.
	if err := cmdLogin([]string{"--server", "http://x", "--username", "u"}); err == nil {
		t.Fatal("password prompt on a pipe must fail")
	}
}

func TestCmdWatchQueryDryRun(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	t.Setenv("XDG_CONFIG_HOME", "")
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.Write([]byte(`{"items":[{"id":"e1","title":"Show","season":1,"episode":1}],"total":1}`))
	}))
	defer srv.Close()
	if err := saveConfig(clientConfig{Server: srv.URL, Token: "tok"}); err != nil {
		t.Fatal(err)
	}
	// A query resolves via search; dry-run never launches mpv.
	if err := cmdWatch([]string{"show", "--dry-run"}); err != nil {
		t.Fatalf("query dry-run: %v", err)
	}
}

func TestCmdWatchResolveError(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	t.Setenv("XDG_CONFIG_HOME", "")
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(500)
	}))
	defer srv.Close()
	if err := saveConfig(clientConfig{Server: srv.URL, Token: "tok"}); err != nil {
		t.Fatal(err)
	}
	if err := cmdWatch([]string{"--next", "--dry-run"}); err == nil {
		t.Fatal("server error must propagate")
	}
}

func TestReportNextPrintsAndSwallowsErrors(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.Write([]byte(`{"items":[{"id":"e2","title":"Show","season":1,"episode":2}],"total":1}`))
	}))
	defer srv.Close()
	c := newAPIClient(srv.URL, "tok")
	reportNext(c, apiItem{ID: "e1", Title: "Show", Season: 1, Episode: 1})
	reportNext(c, apiItem{ID: "e9", Title: "Show", Season: 1, Episode: 9}) // no next: silent
	bad := newAPIClient("http://127.0.0.1:1", "t")
	reportNext(bad, apiItem{ID: "e1", Title: "Show", Episode: 1}) // error: silent
}

func TestCmdBenchPositive(t *testing.T) {
	media := t.TempDir()
	if err := os.WriteFile(filepath.Join(media, "ep.mkv"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	out := filepath.Join(t.TempDir(), "bench-out")
	if err := cmdBench([]string{"scan", "--path", media, "--runs", "1", "--out", out, "--data-dir", t.TempDir()}); err != nil {
		t.Fatalf("bench scan: %v", err)
	}
	entries, err := os.ReadDir(out)
	if err != nil || len(entries) != 1 {
		t.Fatalf("report dir: %v %v", entries, err)
	}
}

func TestNewHTTPServerConfig(t *testing.T) {
	s := newHTTPServer("127.0.0.1", "9360", nil)
	if s.Addr != "127.0.0.1:9360" {
		t.Fatalf("addr=%q", s.Addr)
	}
	if s.ReadTimeout != 15*time.Second {
		t.Fatalf("read timeout=%v", s.ReadTimeout)
	}
	if s.WriteTimeout != 0 {
		t.Fatalf("write timeout must be 0 for streams, got %v", s.WriteTimeout)
	}
}

func TestCmdWatchQueryError(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	t.Setenv("XDG_CONFIG_HOME", "")
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(500)
	}))
	defer srv.Close()
	if err := saveConfig(clientConfig{Server: srv.URL, Token: "tok"}); err != nil {
		t.Fatal(err)
	}
	if err := cmdWatch([]string{"query", "--dry-run"}); err == nil {
		t.Fatal("search failure must propagate")
	}
}

func TestCmdWatchEmptyQueryDryRun(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	t.Setenv("XDG_CONFIG_HOME", "")
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		switch r.URL.Path {
		case "/api/me/continue":
			w.Write([]byte(`[{"item_id":"e1","completed":false}]`))
		case "/api/catalog/e1":
			w.Write([]byte(`{"id":"e1","title":"Show","episode":1}`))
		default:
			w.WriteHeader(404)
		}
	}))
	defer srv.Close()
	if err := saveConfig(clientConfig{Server: srv.URL, Token: "tok"}); err != nil {
		t.Fatal(err)
	}
	// No --next and no query: resolveNext via the empty-query arm.
	if err := cmdWatch([]string{"--dry-run"}); err != nil {
		t.Fatalf("empty-query dry-run: %v", err)
	}
}

func TestCmdBackupOnlineIntoExistingDir(t *testing.T) {
	testConfigHome(t)
	dbDir := t.TempDir()
	db, err := kv.Open(dbDir)
	if err != nil {
		t.Fatal(err)
	}
	db.Close()
	dbBytes, err := os.ReadFile(filepath.Join(dbDir, backup.DBFile))
	if err != nil {
		t.Fatal(err)
	}
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/health":
			w.WriteHeader(200)
		case "/api/admin/backup":
			w.Write(dbBytes)
		case "/api/plugins":
			w.Header().Set("Content-Type", "application/json")
			w.Write([]byte(`{"composition":[]}`))
		default:
			w.WriteHeader(404)
		}
	}))
	defer ts.Close()
	if err := saveConfig(clientConfig{Server: ts.URL, Token: "tok"}); err != nil {
		t.Fatal(err)
	}
	// An existing --out dir gets a timestamped child inside it.
	out := t.TempDir()
	dir, err := cmdBackupOnline(nil, out)
	if err != nil {
		t.Fatal(err)
	}
	if filepath.Dir(dir) != out || !strings.HasPrefix(filepath.Base(dir), "lain-backup-") {
		t.Fatalf("backup dir must nest under --out: %s", dir)
	}
}
