package main

import (
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestPositionalSkipsFlagValues(t *testing.T) {
	got := positional([]string{"frieren", "--pick", "2", "--once", "--server", "http://x"})
	if len(got) != 1 || got[0] != "frieren" {
		t.Fatalf("positional=%v", got)
	}
	if got := positional([]string{"--next"}); len(got) != 0 {
		t.Fatalf("positional=%v", got)
	}
}

func TestPickFlag(t *testing.T) {
	if n := pickFlag([]string{"lain", "watch", "x", "--pick", "3"}); n != 3 {
		t.Fatalf("n=%d", n)
	}
	if n := pickFlag([]string{"--pick=2"}); n != 2 {
		t.Fatalf("n=%d", n)
	}
	if n := pickFlag([]string{"--once"}); n != 0 {
		t.Fatalf("n=%d", n)
	}
}

func TestPickItemReadsChoice(t *testing.T) {
	items := []apiItem{{ID: "a", Title: "Show", Episode: 1}, {ID: "b", Title: "Show", Episode: 2}}
	inR, inW, _ := os.Pipe()
	outR, outW, _ := os.Pipe()
	go func() {
		inW.WriteString("2\n")
		inW.Close()
	}()
	picked, err := pickItem(items, inR, outW)
	outW.Close()
	if err != nil {
		t.Fatal(err)
	}
	if picked.ID != "b" {
		t.Fatalf("picked=%+v", picked)
	}
	buf := make([]byte, 4096)
	n, _ := outR.Read(buf)
	if !strings.Contains(string(buf[:n]), "Show") {
		t.Fatalf("listing missing titles: %q", string(buf[:n]))
	}
}

func TestPickItemRejectsBadChoice(t *testing.T) {
	items := []apiItem{{ID: "a", Title: "Show"}}
	inR, inW, _ := os.Pipe()
	outR, outW, _ := os.Pipe()
	go func() {
		inW.WriteString("9\n")
		inW.Close()
	}()
	if _, err := pickItem(items, inR, outW); err == nil {
		t.Fatal("out-of-range pick must fail")
	}
	outW.Close()
	outR.Close()
}

func TestReadStateFile(t *testing.T) {
	p := filepath.Join(t.TempDir(), "s.json")
	if err := os.WriteFile(p, []byte(`{"position_sec":12.5,"duration_sec":100}`), 0o644); err != nil {
		t.Fatal(err)
	}
	pos, dur, ok := readStateFile(p)
	if !ok || pos != 12.5 || dur != 100 {
		t.Fatalf("pos=%v dur=%v ok=%v", pos, dur, ok)
	}
	if _, _, ok := readStateFile(p + ".missing"); ok {
		t.Fatal("missing file must not parse")
	}
}

func TestConfigRoundtripUsesHome(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	// XDG_CONFIG_HOME unset -> ~/.config
	t.Setenv("XDG_CONFIG_HOME", "")
	if err := saveConfig(clientConfig{Server: "http://x:1", Token: "tok"}); err != nil {
		t.Fatal(err)
	}
	got, err := loadConfig()
	if err != nil {
		t.Fatal(err)
	}
	if got.Server != "http://x:1" || got.Token != "tok" {
		t.Fatalf("config=%+v", got)
	}
	p, _ := configPath()
	fi, err := os.Stat(p)
	if err != nil || fi.Mode().Perm() != 0o600 {
		t.Fatalf("config must be 0600: %v %v", fi, err)
	}
}

func TestResolveQuerySingleAndPick(t *testing.T) {
	items := []apiItem{
		{ID: "e1", Title: "Show", Season: 1, Episode: 1},
		{ID: "e2", Title: "Show", Season: 1, Episode: 2},
	}
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.Write([]byte(`{"items":[{"id":"e1","title":"Show","season":1,"episode":1},{"id":"e2","title":"Show","season":1,"episode":2}],"total":2,"limit":500,"offset":0}`))
	}))
	defer srv.Close()
	client := newAPIClient(srv.URL, "tok")

	devNull, _ := os.Open(os.DevNull)
	got, err := resolveQueryArgs(client, "show", devNull, devNull, []string{"watch", "show", "--pick", "2"})
	if err != nil {
		t.Fatal(err)
	}
	if got.ID != "e2" {
		t.Fatalf("picked=%+v", got)
	}
	_ = items
}

func TestFindNextEpisode(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.Write([]byte(`{"items":[{"id":"e1","title":"Show","season":1,"episode":1},{"id":"e2","title":"Show","season":1,"episode":2},{"id":"s2","title":"Show","season":2,"episode":1}],"total":3,"limit":500,"offset":0}`))
	}))
	defer srv.Close()
	client := newAPIClient(srv.URL, "tok")
	next, ok, err := findNextEpisode(client, apiItem{ID: "e1", Title: "Show", Season: 1, Episode: 1})
	if err != nil || !ok || next.ID != "e2" {
		t.Fatalf("next=%+v ok=%v err=%v", next, ok, err)
	}
	if _, ok, _ := findNextEpisode(client, apiItem{ID: "e2", Title: "Show", Season: 1, Episode: 2}); ok {
		t.Fatal("no ep3 must report false")
	}
	if _, ok, _ := findNextEpisode(client, apiItem{ID: "m", Title: "Movie"}); ok {
		t.Fatal("non-episodic must report false")
	}
}
