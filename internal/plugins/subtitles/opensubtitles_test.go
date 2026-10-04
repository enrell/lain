package subtitles

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/testutil/contract"
)

// fakeOS imitates the OpenSubtitles.com REST API v1 closely enough to
// pin the client's wire behavior. Tests never reach the real service.
type fakeOS struct {
	*httptest.Server
	mu        sync.Mutex
	queries   []string
	logins    int
	downloads []string
}

func newFakeOS(t *testing.T) *fakeOS {
	f := &fakeOS{}
	f.Server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Api-Key") != "key-1" {
			w.WriteHeader(http.StatusUnauthorized)
			_, _ = w.Write([]byte(`{"message":"Invalid API key key-wrong"}`))
			return
		}
		if !strings.HasPrefix(r.Header.Get("User-Agent"), "Lain v") {
			w.WriteHeader(http.StatusForbidden)
			return
		}
		f.mu.Lock()
		defer f.mu.Unlock()
		switch {
		case r.Method == "POST" && r.URL.Path == "/api/v1/login":
			var in struct{ Username, Password string }
			_ = json.NewDecoder(r.Body).Decode(&in)
			if in.Password != "secret-pass" {
				w.WriteHeader(http.StatusUnauthorized)
				_, _ = w.Write([]byte(`{"message":"bad credentials secret-pass"}`))
				return
			}
			f.logins++
			_, _ = w.Write([]byte(`{"token":"jwt-1","status":200}`))
		case r.Method == "GET" && r.URL.Path == "/api/v1/subtitles":
			f.queries = append(f.queries, r.URL.RawQuery)
			_, _ = w.Write([]byte(`{"total_count":3,"data":[
{"id":"1","type":"subtitle","attributes":{"language":"en","download_count":900,"hearing_impaired":false,"foreign_parts_only":false,"release":"[Fansub-A] Show - 05 [1080p]","moviehash_match":true,
 "feature_details":{"title":"Show","season_number":1,"episode_number":5},"files":[{"file_id":111,"file_name":"Show.05.en.srt"}]}},
{"id":"2","type":"subtitle","attributes":{"language":"pt-BR","download_count":40,"hearing_impaired":true,"foreign_parts_only":false,"release":"Show S01E05 WEB",
 "feature_details":{"title":"Show","season_number":1,"episode_number":5},"files":[{"file_id":222,"file_name":"Show.S01E05.pt-BR.srt"}]}},
{"id":"3","type":"subtitle","attributes":{"language":"xx","files":[]}}
]}`))
		case r.Method == "POST" && r.URL.Path == "/api/v1/download":
			var in struct {
				FileID int `json:"file_id"`
			}
			_ = json.NewDecoder(r.Body).Decode(&in)
			f.downloads = append(f.downloads, r.Header.Get("Authorization"))
			if in.FileID != 111 {
				w.WriteHeader(http.StatusNotFound)
				return
			}
			_, _ = w.Write([]byte(`{"link":"` + f.URL + `/files/111.srt","file_name":"Show.05.en.srt","remaining":4}`))
		default:
			http.NotFound(w, r)
		}
	}))
	t.Cleanup(f.Close)
	return f
}

func prov(f *fakeOS, key string) contracts.SubtitleProvider {
	return contracts.SubtitleProvider{ID: "p1", Kind: contracts.SubtitleOpenSubtitles, BaseURL: f.URL + "/api/v1", APIKey: key, Enabled: true}
}

func TestSearchByHashAndEpisode(t *testing.T) {
	f := newFakeOS(t)
	out, err := NewOpenSubtitles().Invoke(contracts.CapSubtitle, contracts.SubtitleSearchInput{
		Provider: prov(f, "key-1"), Hash: "8e245d9679d31e12", Size: 12909756, Query: "Show",
		Season: 1, Episode: 5, Languages: []string{"eng", "por"},
	})
	if err != nil {
		t.Fatal(err)
	}
	cands := out.(contracts.SubtitleSearchOutput).Candidates
	if len(cands) != 2 {
		t.Fatalf("candidates: %+v", cands)
	}
	a, b := cands[0], cands[1]
	if a.FileID != "111" || a.Language != "eng" || !a.HashMatch || a.Downloads != 900 || a.ProviderID != "p1" || a.Episode != 5 {
		t.Fatalf("first: %+v", a)
	}
	if b.Language != "por" || b.Region != "pt-BR" || !b.HI {
		t.Fatalf("second: %+v", b)
	}
	// The API wants lowercase, alphabetically sorted parameters.
	q := f.queries[0]
	want := "episode_number=5&languages=en%2Cpt-br%2Cpt-pt&moviehash=8e245d9679d31e12&query=show&season_number=1&type=episode"
	if q != want {
		t.Fatalf("query\n got %s\nwant %s", q, want)
	}
}

func TestDownloadLogsInWhenAccountGiven(t *testing.T) {
	f := newFakeOS(t)
	p := prov(f, "key-1")
	o := NewOpenSubtitles()
	out, err := o.Invoke(contracts.CapSubtitle, contracts.SubtitleDownloadInput{Provider: p, FileID: "111"})
	if err != nil {
		t.Fatal(err)
	}
	d := out.(contracts.SubtitleDownloadOutput)
	if !strings.HasSuffix(d.Link, "/files/111.srt") || d.Remaining != 4 {
		t.Fatalf("download: %+v", d)
	}
	if f.downloads[0] != "" || f.logins != 0 {
		t.Fatal("anonymous download must not log in")
	}
	p.Username, p.Password = "lain", "secret-pass"
	if _, err := o.Invoke(contracts.CapSubtitle, contracts.SubtitleDownloadInput{Provider: p, FileID: "111"}); err != nil {
		t.Fatal(err)
	}
	if f.logins != 1 || f.downloads[1] != "Bearer jwt-1" {
		t.Fatalf("logins %d auth %q", f.logins, f.downloads[1])
	}
	// The token is cached per account.
	_, _ = o.Invoke(contracts.CapSubtitle, contracts.SubtitleDownloadInput{Provider: p, FileID: "111"})
	if f.logins != 1 {
		t.Fatalf("logged in again: %d", f.logins)
	}
}

func TestErrorsNeverLeakSecrets(t *testing.T) {
	f := newFakeOS(t)
	_, err := NewOpenSubtitles().Invoke(contracts.CapSubtitle, contracts.SubtitleSearchInput{Provider: prov(f, "key-wrong"), Query: "x", Languages: []string{"eng"}})
	ce, ok := err.(*core.Error)
	if !ok || ce.Code != "auth-failed" || strings.Contains(ce.Msg, "key-wrong") {
		t.Fatalf("api key error: %v", err)
	}
	p := prov(f, "key-1")
	p.Username, p.Password = "lain", "wrong-pass"
	_, err = NewOpenSubtitles().Invoke(contracts.CapSubtitle, contracts.SubtitleDownloadInput{Provider: p, FileID: "111"})
	if ce, ok := err.(*core.Error); !ok || ce.Code != "auth-failed" || strings.Contains(ce.Msg, "wrong-pass") {
		t.Fatalf("login error: %v", err)
	}
}

func TestRejectsBadInput(t *testing.T) {
	o := NewOpenSubtitles()
	for _, in := range []any{
		contracts.SubtitleSearchInput{Provider: contracts.SubtitleProvider{Kind: "other"}},
		contracts.SubtitleSearchInput{Provider: contracts.SubtitleProvider{Kind: contracts.SubtitleOpenSubtitles, APIKey: "k", BaseURL: "file:///etc"}, Query: "x"},
		contracts.SubtitleSearchInput{Provider: contracts.SubtitleProvider{Kind: contracts.SubtitleOpenSubtitles, APIKey: "k"}},
		contracts.SubtitleDownloadInput{Provider: contracts.SubtitleProvider{Kind: contracts.SubtitleOpenSubtitles, APIKey: "k"}, FileID: "../x"},
	} {
		_, err := o.Invoke(contracts.CapSubtitle, in)
		if _, ok := err.(*core.Error); !ok {
			t.Errorf("%+v: %v", in, err)
		}
	}
	if _, err := o.Invoke(contracts.CapSubtitle, contracts.SubtitleSearchInput{Provider: contracts.SubtitleProvider{Kind: "other"}}); err.(*core.Error).Code != "unsupported-provider" {
		t.Fatalf("decline code: %v", err)
	}
}

func TestContract(t *testing.T) {
	contract.Run(t, NewOpenSubtitles(), []contract.Cap{{Name: contracts.CapSubtitle, Sample: contracts.SubtitleSearchInput{
		Provider: contracts.SubtitleProvider{Kind: contracts.SubtitleOpenSubtitles, APIKey: "k", BaseURL: "http://127.0.0.1:9/api/v1"}, Query: "x", Languages: []string{"eng"},
	}}})
}
