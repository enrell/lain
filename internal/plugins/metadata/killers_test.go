package metadata

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-22

import (
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/kv"
)

func mustJSON(t *testing.T, v any) json.RawMessage {
	t.Helper()
	raw, err := json.Marshal(v)
	if err != nil {
		t.Fatal(err)
	}
	return raw
}

// countingServer returns a handler that records hits and answers with
// the given body/status, so tests can prove whether a request happened.
type countingServer struct {
	hits *atomic.Int32
	srv  *httptest.Server
}

func newCountingServer(t *testing.T, status int, body string) *countingServer {
	t.Helper()
	cs := &countingServer{hits: &atomic.Int32{}}
	cs.srv = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		cs.hits.Add(1)
		w.WriteHeader(status)
		fmt.Fprint(w, body)
	}))
	t.Cleanup(cs.srv.Close)
	return cs
}

// --- Invoke dispatch: capability/kind/input gating ---

func TestInvokeKindGates(t *testing.T) {
	// Each provider expects a different upstream payload shape; the stub
	// answers an empty result in the provider's own envelope.
	emptyBody := map[string]string{
		"anilist": `{"data":{"page":{"media":[]}}}`,
		"jikan":   `{"data":[]}`,
		"kitsu":   `{"data":[]}`,
		"tvmaze":  `[]`,
	}
	providers := []struct {
		name string
		new  func() interface {
			Invoke(string, any) (any, error)
		}
		setURL func(any, string)
		accept []string
	}{
		{"anilist", func() interface {
			Invoke(string, any) (any, error)
		} {
			a := NewAniList()
			a.http.minGap = 0
			return a
		}, func(p any, u string) { p.(*AniList).BaseURL = u },
			[]string{"", "anime", "episode", "video"}},
		{"jikan", func() interface {
			Invoke(string, any) (any, error)
		} {
			j := NewJikan()
			j.http.minGap = 0
			return j
		}, func(p any, u string) { p.(*Jikan).BaseURL = u },
			[]string{"", "anime", "episode", "video"}},
		{"kitsu", func() interface {
			Invoke(string, any) (any, error)
		} {
			k := NewKitsu()
			k.http.minGap = 0
			return k
		}, func(p any, u string) { p.(*Kitsu).BaseURL = u },
			[]string{"", "anime", "episode", "video"}},
		{"tvmaze", func() interface {
			Invoke(string, any) (any, error)
		} {
			tv := NewTVMaze()
			tv.http.minGap = 0
			return tv
		}, func(p any, u string) { p.(*TVMaze).BaseURL = u },
			[]string{"", "anime", "series", "episode", "video"}},
	}
	for _, tc := range providers {
		cs := newCountingServer(t, 200, emptyBody[tc.name])
		p := tc.new()
		tc.setURL(p, cs.srv.URL)
		for _, kind := range tc.accept {
			before := cs.hits.Load()
			out, err := p.Invoke(contracts.CapMetadataSearch, contracts.MetadataSearchInput{Kind: kind, Query: "x"})
			if err != nil {
				t.Fatalf("%s kind %q: %v", tc.name, kind, err)
			}
			if cs.hits.Load() == before {
				t.Fatalf("%s kind %q must reach the server", tc.name, kind)
			}
			_ = out
		}
		before := cs.hits.Load()
		out, err := p.Invoke(contracts.CapMetadataSearch, contracts.MetadataSearchInput{Kind: "movie", Query: "x"})
		if err != nil {
			t.Fatalf("%s kind movie: %v", tc.name, err)
		}
		if cs.hits.Load() != before {
			t.Fatalf("%s kind movie must not reach the server", tc.name)
		}
		if len(out.([]contracts.MetadataCandidate)) != 0 {
			t.Fatalf("%s kind movie must answer empty", tc.name)
		}
	}
}

func TestInvokeRejectsBadInputAndCap(t *testing.T) {
	providers := map[string]interface {
		Invoke(string, any) (any, error)
	}{
		"anilist": NewAniList(),
		"jikan":   NewJikan(),
		"kitsu":   NewKitsu(),
		"tvmaze":  NewTVMaze(),
		"nfo":     NFO{},
	}
	for name, p := range providers {
		if _, err := p.Invoke("lain.bogus@1", contracts.MetadataSearchInput{}); err == nil {
			t.Fatalf("%s: unsupported cap must fail", name)
		}
		if _, err := p.Invoke(contracts.CapMetadataSearch, "nope"); err == nil {
			t.Fatalf("%s: search needs MetadataSearchInput", name)
		}
		if _, err := p.Invoke(contracts.CapMetadataResolve, 42); err == nil {
			t.Fatalf("%s: resolve needs MetadataResolveInput", name)
		}
	}
}

// --- polite client: retry, status, retry-after, pacing ---

func TestPoliteClientRetryAndStatus(t *testing.T) {
	// 429 without Retry-After -> terminal rate-limit error.
	cs := newCountingServer(t, 429, "slow down")
	c := newPoliteClient(0)
	if _, err := c.get(cs.srv.URL); err == nil || !strings.Contains(err.Error(), "rate limited") {
		t.Fatalf("429 without retry-after: %v", err)
	}
	if cs.hits.Load() != 1 {
		t.Fatalf("no header must not retry, hits=%d", cs.hits.Load())
	}

	// 429 with Retry-After retries once, then succeeds.
	cs2 := newCountingServer(t, 0, "")
	cs2.srv.Config.Handler = http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if cs2.hits.Add(1) == 1 {
			w.Header().Set("Retry-After", "1")
			w.WriteHeader(429)
			return
		}
		w.Write([]byte("ok"))
	})
	raw, err := c.get(cs2.srv.URL)
	if err != nil || string(raw) != "ok" {
		t.Fatalf("retry must recover: %q %v", raw, err)
	}
	if cs2.hits.Load() != 2 {
		t.Fatalf("want exactly 2 attempts, got %d", cs2.hits.Load())
	}

	// Persistent 429: exactly two attempts, then rate-limited error.
	cs3 := newCountingServer(t, 0, "")
	cs3.srv.Config.Handler = http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		cs3.hits.Add(1)
		w.Header().Set("Retry-After", "1")
		w.WriteHeader(429)
	})
	if _, err := c.get(cs3.srv.URL); err == nil || !strings.Contains(err.Error(), "429") {
		t.Fatalf("persistent 429: %v", err)
	}
	if cs3.hits.Load() != 2 {
		t.Fatalf("persistent 429 must stop after one retry, hits=%d", cs3.hits.Load())
	}

	// Non-2xx statuses surface the code.
	cs4 := newCountingServer(t, 500, "boom")
	if _, err := c.get(cs4.srv.URL); err == nil || !strings.Contains(err.Error(), "500") {
		t.Fatalf("500 must surface status: %v", err)
	}
	cs5 := newCountingServer(t, 404, "")
	if _, err := c.postJSON(cs5.srv.URL, []byte("{}")); err == nil || !strings.Contains(err.Error(), "404") {
		t.Fatalf("post 404 must surface status: %v", err)
	}
}

func TestPoliteClientPacesRequests(t *testing.T) {
	cs := newCountingServer(t, 200, "ok")
	c := newPoliteClient(60 * time.Millisecond)
	start := time.Now()
	if _, err := c.get(cs.srv.URL); err != nil {
		t.Fatal(err)
	}
	if _, err := c.get(cs.srv.URL); err != nil {
		t.Fatal(err)
	}
	if el := time.Since(start); el < 60*time.Millisecond {
		t.Fatalf("second request must wait the gap, elapsed=%s", el)
	}
}

func TestParseRetryAfter(t *testing.T) {
	for h, want := range map[string]time.Duration{
		"":    0,
		"5":   5 * time.Second,
		"abc": 0,
		"0":   0,
	} {
		if got := parseRetryAfter(h); got != want {
			t.Errorf("parseRetryAfter(%q)=%s, want %s", h, got, want)
		}
	}
}

// --- search limit bounds reach the upstream request ---

func TestSearchLimitBounds(t *testing.T) {
	var gotLimit string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotLimit = r.URL.Query().Get("limit")
		w.Write([]byte(`{"data":[]}`))
	}))
	defer srv.Close()
	j := NewJikan()
	j.BaseURL = srv.URL
	j.http.minGap = 0
	for in, want := range map[int]string{0: "5", -3: "5", 11: "5", 10: "10", 3: "3"} {
		if _, err := j.Invoke(contracts.CapMetadataSearch, contracts.MetadataSearchInput{Query: "x", Limit: in}); err != nil {
			t.Fatal(err)
		}
		if gotLimit != want {
			t.Errorf("limit %d -> upstream %q, want %q", in, gotLimit, want)
		}
	}
}

// --- anilist specifics ---

func TestAniListLimitInVariables(t *testing.T) {
	var gotBody string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		buf := make([]byte, 4096)
		n, _ := r.Body.Read(buf)
		gotBody = string(buf[:n])
		w.Write([]byte(`{"data":{"page":{"media":[]}}}`))
	}))
	defer srv.Close()
	a := NewAniList()
	a.BaseURL = srv.URL
	a.http.minGap = 0
	for in, want := range map[int]string{0: "5", 12: "5", 7: "7", 10: "10"} {
		if _, err := a.Invoke(contracts.CapMetadataSearch, contracts.MetadataSearchInput{Query: "x", Limit: in}); err != nil {
			t.Fatal(err)
		}
		if !strings.Contains(gotBody, `"perPage":`+want) {
			t.Errorf("limit %d: body %s lacks perPage %s", in, gotBody, want)
		}
	}
	if _, err := a.Invoke(contracts.CapMetadataResolve, contracts.MetadataResolveInput{RemoteID: "abc"}); err == nil {
		t.Fatal("non-numeric anilist id must fail")
	}
}

func TestStripHTML(t *testing.T) {
	for in, want := range map[string]string{
		"a<b>x</b>c":  "axc",
		"plain":       "plain",
		"<br>hi":      "hi",
		"unclosed <i": "unclosed",
		"1>0":         "10", // a bare '>' is consumed as tag-close markup
	} {
		if got := stripHTML(in); got != want {
			t.Errorf("stripHTML(%q)=%q, want %q", in, got, want)
		}
	}
}

// --- jikan specifics ---

func TestJikanResolvePaths(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte(`{"data":{"mal_id":7,"title":"Romaji","title_english":"Eng Title","title_japanese":"Romaji","synopsis":" s ","episodes":1,"aired":{"from":"1899-01-01"},"images":{"jpg":{"image_url":"s.jpg","large_image_url":""}},"genres":[{"name":"G"}]}}`))
	}))
	defer srv.Close()
	j := NewJikan()
	j.BaseURL = srv.URL
	j.http.minGap = 0

	if _, err := j.Invoke(contracts.CapMetadataResolve, contracts.MetadataResolveInput{RemoteID: "not-a-number"}); err == nil {
		t.Fatal("non-numeric jikan id must fail")
	}
	out, err := j.Invoke(contracts.CapMetadataResolve, contracts.MetadataResolveInput{RemoteID: "7"})
	if err != nil {
		t.Fatal(err)
	}
	rec := out.(contracts.MetadataRecord)
	if rec.Title != "Eng Title" {
		t.Fatalf("english title preferred: %+v", rec)
	}
	// "Romaji" appears twice upstream (title + title_japanese); it must
	// survive as a synonym, and never echo the chosen title.
	var romaji int
	for _, s := range rec.Synonyms {
		if s == "Romaji" {
			romaji++
		}
		if s == rec.Title {
			t.Fatalf("synonym equal to title must be dropped: %+v", rec.Synonyms)
		}
	}
	if romaji == 0 {
		t.Fatalf("synonyms must carry the non-english titles: %+v", rec.Synonyms)
	}
	if rec.Year != 0 {
		t.Fatalf("1899 must be rejected as a year: %d", rec.Year)
	}
	if rec.Poster != "s.jpg" {
		t.Fatalf("missing large image must fall back: %+v", rec.Poster)
	}
	// No english title falls back to title.
	srv2 := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte(`{"data":{"mal_id":8,"title":"Only Romaji","title_english":"","title_japanese":"","synopsis":"","episodes":0,"aired":{"from":""},"images":{"jpg":{}},"genres":[]}}`))
	}))
	defer srv2.Close()
	j.BaseURL = srv2.URL
	out, err = j.Invoke(contracts.CapMetadataResolve, contracts.MetadataResolveInput{RemoteID: "8"})
	if err != nil {
		t.Fatal(err)
	}
	if rec := out.(contracts.MetadataRecord); rec.Title != "Only Romaji" {
		t.Fatalf("missing english must fall back to title: %+v", rec)
	}
}

func TestYearOfBounds(t *testing.T) {
	for in, want := range map[string]int{
		"":             0,
		"abc":          0,
		"1899-01-01":   0,
		"1900-01-01":   1900,
		"2099-12-31":   2099,
		"2100-01-01":   0,
		"xxxx-01-01":   0,
		"2023":         2023, // exactly 4 chars
		"2023-09-29T0": 2023,
	} {
		if got := yearOf(in); got != want {
			t.Errorf("yearOf(%q)=%d, want %d", in, got, want)
		}
	}
}

func TestJikanUpstreamErrors(t *testing.T) {
	cs := newCountingServer(t, 503, "down")
	j := NewJikan()
	j.BaseURL = cs.srv.URL
	j.http.minGap = 0
	if _, err := j.Invoke(contracts.CapMetadataSearch, contracts.MetadataSearchInput{Query: "x"}); err == nil {
		t.Fatal("upstream 503 must fail search")
	}
	if _, err := j.Invoke(contracts.CapMetadataResolve, contracts.MetadataResolveInput{RemoteID: "7"}); err == nil {
		t.Fatal("upstream 503 must fail resolve")
	}
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte("not json"))
	}))
	defer srv.Close()
	j.BaseURL = srv.URL
	if _, err := j.Invoke(contracts.CapMetadataSearch, contracts.MetadataSearchInput{Query: "x"}); err == nil {
		t.Fatal("garbage JSON must fail search")
	}
	if _, err := j.Invoke(contracts.CapMetadataResolve, contracts.MetadataResolveInput{RemoteID: "7"}); err == nil {
		t.Fatal("garbage JSON must fail resolve")
	}
}

// --- kitsu specifics ---

func TestKitsuFallbacks(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/api/edge/anime" {
			// canonicalTitle empty -> en title; startDate garbage -> year 0;
			// no "original" image -> falls back to lower tiers.
			w.Write([]byte(`{"data":[{"id":"5","attributes":{"canonicalTitle":"","titles":{"en":"English Title","en_jp":"Romanized","bad":"  "},"synopsis":"s","startDate":"xx","episodeCount":1,"posterImage":{"tiny":"https://t.jpg"},"coverImage":{"small":"https://cs.jpg"}}}]}`))
			return
		}
		w.WriteHeader(404)
	}))
	defer srv.Close()
	k := NewKitsu()
	k.BaseURL = srv.URL
	k.http.minGap = 0
	out, err := k.Invoke(contracts.CapMetadataSearch, contracts.MetadataSearchInput{Query: "x"})
	if err != nil {
		t.Fatal(err)
	}
	list := out.([]contracts.MetadataCandidate)
	if len(list) != 1 || list[0].Title != "English Title" || list[0].Year != 0 {
		t.Fatalf("kitsu fallbacks: %+v", list)
	}
	if list[0].Poster != "https://t.jpg" {
		t.Fatalf("poster must fall back through tiers: %+v", list[0].Poster)
	}
	for _, s := range list[0].Synonyms {
		if strings.TrimSpace(s) == "" {
			t.Fatalf("blank titles must not be synonyms: %+v", list[0].Synonyms)
		}
	}
}

func TestKitsuLimitAndEdges(t *testing.T) {
	var gotLimit string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/api/edge/anime" {
			gotLimit = r.URL.Query().Get("page[limit]")
			// Exactly-4 startDate exercises the len>=4 boundary.
			w.Write([]byte(`{"data":[{"id":"9","attributes":{"canonicalTitle":"Nine","startDate":"2023","posterImage":{},"coverImage":{}}}]}`))
			return
		}
		if r.URL.Path == "/api/edge/anime/9" {
			// Resolve: canonicalTitle empty falls back to titles.en.
			w.Write([]byte(`{"data":{"id":"9","attributes":{"canonicalTitle":"","titles":{"en":"Nine EN"},"startDate":"2019","posterImage":{},"coverImage":{}}}}`))
			return
		}
		w.WriteHeader(404)
	}))
	defer srv.Close()
	k := NewKitsu()
	k.BaseURL = srv.URL
	k.http.minGap = 0
	for in, want := range map[int]string{0: "5", -4: "5", 12: "5", 10: "10", 4: "4"} {
		if _, err := k.Invoke(contracts.CapMetadataSearch, contracts.MetadataSearchInput{Query: "x", Limit: in}); err != nil {
			t.Fatal(err)
		}
		if gotLimit != want {
			t.Errorf("limit %d -> page[limit]=%q, want %q", in, gotLimit, want)
		}
	}
	out, err := k.Invoke(contracts.CapMetadataSearch, contracts.MetadataSearchInput{Query: "x"})
	if err != nil {
		t.Fatal(err)
	}
	if list := out.([]contracts.MetadataCandidate); len(list) != 1 || list[0].Year != 2023 {
		t.Fatalf("4-char startDate must parse: %+v", list)
	}
	rout, err := k.Invoke(contracts.CapMetadataResolve, contracts.MetadataResolveInput{RemoteID: "9"})
	if err != nil {
		t.Fatal(err)
	}
	if rec := rout.(contracts.MetadataRecord); rec.Title != "Nine EN" || rec.Year != 2019 {
		t.Fatalf("resolve fallbacks: %+v", rec)
	}
}

func TestFirstImageAnyTier(t *testing.T) {
	if got := firstImage(map[string]string{"weird": "w.jpg"}); got != "w.jpg" {
		t.Fatalf("unknown tier must still answer, got %q", got)
	}
	if got := firstImage(map[string]string{"original": "", "tiny": "t.jpg"}); got != "t.jpg" {
		t.Fatalf("empty preferred tiers must be skipped, got %q", got)
	}
	if got := firstImage(map[string]string{"a": "", "b": ""}); got != "" {
		t.Fatalf("all empty must answer empty, got %q", got)
	}
}

// --- tvmaze specifics ---

func TestTVMazeLimitsAndSkips(t *testing.T) {
	shows := `[{"show":{"id":1,"name":"One","premiered":"2020-01-01"}},
	           {"show":{"id":2,"name":"  "}},
	           {"show":{"id":3,"name":"Three","premiered":"bad"}},
	           {"show":{"id":4,"name":"Four","image":{"medium":"m.jpg"}}}]`
	for i := 5; i <= 12; i++ {
		shows = strings.TrimSuffix(shows, "]") + fmt.Sprintf(`,{"show":{"id":%d,"name":"S%d"}}]`, i, i)
	}
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte(shows))
	}))
	defer srv.Close()
	tv := NewTVMaze()
	tv.BaseURL = srv.URL
	tv.http.minGap = 0
	out, err := tv.Invoke(contracts.CapMetadataSearch, contracts.MetadataSearchInput{Query: "x", Limit: 2})
	if err != nil {
		t.Fatal(err)
	}
	list := out.([]contracts.MetadataCandidate)
	if len(list) != 2 || list[0].RemoteID != "1" || list[1].RemoteID != "3" {
		t.Fatalf("blank names skipped, limit honored: %+v", list)
	}
	if list[0].Year != 2020 || list[1].Year != 0 {
		t.Fatalf("premiered parsing: %+v", list)
	}
	// 11 non-blank shows: limit 0 defaults to 5, limit 10 takes ten,
	// limit >10 clamps to 5.
	for lim, want := range map[int]int{0: 5, -2: 5, 10: 10, 11: 5} {
		out, err = tv.Invoke(contracts.CapMetadataSearch, contracts.MetadataSearchInput{Query: "x", Limit: lim})
		if err != nil {
			t.Fatal(err)
		}
		if got := len(out.([]contracts.MetadataCandidate)); got != want {
			t.Fatalf("limit %d -> %d results, want %d", lim, got, want)
		}
	}
}

func TestTVMazeResolveEmptyShow(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte(`{"id":1,"name":" ","summary":"<p>Hi.</p>","image":{"medium":"m.jpg"}}`))
	}))
	defer srv.Close()
	tv := NewTVMaze()
	tv.BaseURL = srv.URL
	tv.http.minGap = 0
	if _, err := tv.Invoke(contracts.CapMetadataResolve, contracts.MetadataResolveInput{RemoteID: "1"}); err == nil {
		t.Fatal("empty show name must fail")
	}
	if got := tvmazeYear("202"); got != 0 {
		t.Fatalf("short premiered must give 0, got %d", got)
	}
	if got := tvmazeYear("2020"); got != 2020 {
		t.Fatalf("exactly-4 premiered must parse, got %d", got)
	}
	var nilImg *tvmazeImage
	if nilImg.best() != "" {
		t.Fatal("nil image must give empty poster")
	}
}

// --- merge scoring and plausibility ---

func TestMergeOrderingAndScores(t *testing.T) {
	// Same normalized title across providers: precedence order decides
	// among non-exact; an exact hit always wins regardless of order.
	a := []contracts.MetadataCandidate{
		{Provider: "p1", RemoteID: "1", Title: "Frieren Season 2"},
		{Provider: "p1", RemoteID: "2", Title: "Frieren"},
	}
	b := []contracts.MetadataCandidate{
		{Provider: "p2", RemoteID: "3", Title: "Frieren"},
	}
	merged := MergeCandidates("frieren", []any{a, b}, []string{"p1", "p2"}, 10)
	// Exact matches (p1#2, p2#3) lead; p1#2 wins on precedence.
	if merged[0].RemoteID != "2" || merged[1].RemoteID != "3" || merged[2].RemoteID != "1" {
		t.Fatalf("ordering: %+v", merged)
	}
	// Scores: positional (len(all)-i) plus +1000 for exact matches.
	if merged[0].Score != float64(len(merged))+1000 {
		t.Fatalf("exact score: %v", merged[0].Score)
	}
	if merged[2].Score != float64(len(merged)-2) {
		t.Fatalf("positional score: %v", merged[2].Score)
	}
}

func TestMergeLimitCap(t *testing.T) {
	var many []contracts.MetadataCandidate
	for i := 0; i < 25; i++ {
		many = append(many, contracts.MetadataCandidate{
			Provider: "p", RemoteID: fmt.Sprint(i), Title: fmt.Sprintf("Show %02d", i)})
	}
	if out := MergeCandidates("", []any{many}, []string{"p"}, 0); len(out) != 10 {
		t.Fatalf("limit 0 must default to 10, got %d", len(out))
	}
	if out := MergeCandidates("", []any{many}, []string{"p"}, 20); len(out) != 20 {
		t.Fatalf("limit 20 is in range, got %d", len(out))
	}
	if out := MergeCandidates("", []any{many}, []string{"p"}, 25); len(out) != 10 {
		t.Fatalf("limit >20 must clamp to 10, got %d", len(out))
	}
	if out := MergeCandidates("", []any{many}, []string{"p"}, 3); len(out) != 3 {
		t.Fatalf("limit 3 must cap, got %d", len(out))
	}
}

func TestPlausibleMatchBounds(t *testing.T) {
	for _, tc := range []struct {
		want, have string
		ok         bool
	}{
		{"frieren", "frieren", true},
		{"frieren", "frieren2ndseason", true},  // have longer, prefix
		{"frierenmovie", "frieren", true},      // want longer, prefix
		{"abc", "abcdef", false},               // shorter than 4 chars never matches
		{"abcd", "abcde", true},                // exactly 4 is enough
		{"frieren", "totallydifferent", false}, // no prefix relation
		{"frieren", "", false},                 // empty never matches
	} {
		if got := plausibleMatch(tc.want, tc.have); got != tc.ok {
			t.Errorf("plausibleMatch(%q,%q)=%v, want %v", tc.want, tc.have, got, tc.ok)
		}
	}
}

// --- nfo specifics ---

func TestNFOScanRules(t *testing.T) {
	dir := t.TempDir()
	write := func(name, body string) {
		if err := os.WriteFile(filepath.Join(dir, name), []byte(body), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	write("tvshow.nfo", `<tvshow><title>Root Show</title></tvshow>`)
	write("frieren-s01e01.nfo", `<episodedetails><title>Ep One</title></episodedetails>`)
	write("other-show.nfo", `<episodedetails><title>Unrelated</title></episodedetails>`)
	write("broken.nfo", `not xml at all <<<`)
	write("frieren-sub.nfo", `<episodedetails><title>Nested</title></episodedetails>`)
	if err := os.Mkdir(filepath.Join(dir, "subdir.nfo"), 0o755); err != nil {
		t.Fatal(err)
	}

	var n NFO
	out, err := n.Invoke(contracts.CapMetadataSearch, contracts.MetadataSearchInput{Query: "frieren", Dir: dir})
	if err != nil {
		t.Fatal(err)
	}
	list := out.([]contracts.MetadataCandidate)
	titles := map[string]bool{}
	for _, c := range list {
		titles[c.Title] = true
	}
	// tvshow.nfo always counts; query-matching sidecars count;
	// unrelated/broken/dir entries do not.
	if !titles["Root Show"] || !titles["Ep One"] || !titles["Nested"] {
		t.Fatalf("expected nfo hits: %+v", list)
	}
	if titles["Unrelated"] {
		t.Fatal("non-matching sidecar must be skipped")
	}
}

func TestNFOCapAndLimit(t *testing.T) {
	dir := t.TempDir()
	for i := 0; i < 12; i++ {
		body := fmt.Sprintf(`<episodedetails><title>Match %02d</title></episodedetails>`, i)
		if err := os.WriteFile(filepath.Join(dir, fmt.Sprintf("match%02d.nfo", i)), []byte(body), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	var n NFO
	out, _ := n.Invoke(contracts.CapMetadataSearch, contracts.MetadataSearchInput{Query: "match", Dir: dir})
	if got := len(out.([]contracts.MetadataCandidate)); got > 8 {
		t.Fatalf("scan must cap at 8 candidates, got %d", got)
	}
	out, _ = n.Invoke(contracts.CapMetadataSearch, contracts.MetadataSearchInput{Query: "match", Dir: dir, Limit: 2})
	if got := len(out.([]contracts.MetadataCandidate)); got != 2 {
		t.Fatalf("limit 2 must cap, got %d", got)
	}
	// Unreadable dir is silence, not an error.
	out, err := n.Invoke(contracts.CapMetadataSearch, contracts.MetadataSearchInput{Query: "x", Dir: filepath.Join(dir, "nope")})
	if err != nil || len(out.([]contracts.MetadataCandidate)) != 0 {
		t.Fatalf("missing dir must be empty, not error: %v", err)
	}
}

func TestNFOYearAndArtFallbacks(t *testing.T) {
	dir := t.TempDir()
	cases := map[string]string{
		"y1899.nfo":     `<movie><title>A</title><year>1899</year><premiered>2015-05-05</premiered></movie>`,
		"y1900.nfo":     `<movie><title>A2</title><year>1900</year></movie>`,
		"y2099.nfo":     `<movie><title>A3</title><year>2099</year></movie>`,
		"prem4.nfo":     `<movie><title>B2</title><premiered>2015</premiered></movie>`,
		"y2100.nfo":     `<movie><title>B</title><year>2100</year><premiered>xx</premiered></movie>`,
		"yrange.nfo":    `<movie><title>C</title><year>1999</year></movie>`,
		"thumbs.nfo":    `<movie><title>D</title><thumb aspect="banner">b.jpg</thumb><thumb aspect="poster">p.jpg</thumb><thumb aspect="clearlogo">l.jpg</thumb></movie>`,
		"origtitle.nfo": `<movie><originaltitle>Alt Title</originaltitle></movie>`,
	}
	for name, body := range cases {
		if err := os.WriteFile(filepath.Join(dir, name), []byte(body), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	get := func(name string) contracts.MetadataRecord {
		rec, err := resolveNFO(filepath.Join(dir, name))
		if err != nil {
			t.Fatal(err)
		}
		return rec
	}
	if r := get("y1899.nfo"); r.Year != 2015 {
		t.Fatalf("out-of-range year falls back to premiered: %d", r.Year)
	}
	if r := get("y1900.nfo"); r.Year != 1900 {
		t.Fatalf("1900 is the inclusive bound: %d", r.Year)
	}
	if r := get("y2099.nfo"); r.Year != 2099 {
		t.Fatalf("2099 is the inclusive bound: %d", r.Year)
	}
	if r := get("y2100.nfo"); r.Year != 0 {
		t.Fatalf("2100+ bad premiered must give 0: %d", r.Year)
	}
	if r := get("prem4.nfo"); r.Year != 2015 {
		t.Fatalf("exactly-4 premiered must parse: %d", r.Year)
	}
	if r := get("yrange.nfo"); r.Year != 1999 {
		t.Fatalf("in-range year kept: %d", r.Year)
	}
	if r := get("thumbs.nfo"); r.Poster != "p.jpg" || r.Cover != "b.jpg" {
		t.Fatalf("poster must pick aspect=poster, cover the first other: %+v", r)
	}
	if r := get("origtitle.nfo"); r.Title != "Alt Title" {
		t.Fatalf("empty title falls back to originaltitle: %+v", r)
	}
}

// --- store/cache gaps ---

func TestGetManySkipsMissingAndCorrupt(t *testing.T) {
	db, err := openTestDB(t)
	if err != nil {
		t.Fatal(err)
	}
	s := NewSaver(db)
	if _, err := s.Save("good", contracts.MetadataRecord{Provider: "p", RemoteID: "1", Title: "G"}); err != nil {
		t.Fatal(err)
	}
	if err := db.Update(func(tx *bolt.Tx) error {
		return tx.Bucket(kv.BEnrich).Put([]byte("corrupt"), []byte("{not json"))
	}); err != nil {
		t.Fatal(err)
	}
	got := s.GetMany([]string{"missing", "corrupt", "good"})
	if len(got) != 1 || got[0].ItemID != "good" {
		t.Fatalf("GetMany must skip missing and corrupt: %+v", got)
	}
}

func TestCacheExpiry(t *testing.T) {
	db, err := openTestDB(t)
	if err != nil {
		t.Fatal(err)
	}
	c := NewCache(db)
	// A stale search entry (older than SearchTTL) reads as a miss;
	// a fresh one hits. Write cacheEntry directly to control At.
	stale := cacheEntry{At: time.Now().Add(-SearchTTL - time.Hour).Unix(), Data: mustJSON(t, []contracts.MetadataCandidate{{Title: "X"}})}
	fresh := cacheEntry{At: time.Now().Unix(), Data: mustJSON(t, []contracts.MetadataCandidate{{Title: "Y"}})}
	if err := db.Update(func(tx *bolt.Tx) error {
		if err := kv.PutJSON(tx, kv.BCache, searchKey("anime", "old"), stale); err != nil {
			return err
		}
		return kv.PutJSON(tx, kv.BCache, searchKey("anime", "new"), fresh)
	}); err != nil {
		t.Fatal(err)
	}
	if _, ok := c.GetSearch("anime", "old"); ok {
		t.Fatal("stale search must miss")
	}
	if got, ok := c.GetSearch("anime", "new"); !ok || got[0].Title != "Y" {
		t.Fatal("fresh search must hit")
	}
	// Same for records.
	staleR := cacheEntry{At: time.Now().Add(-RecordTTL - time.Hour).Unix(), Data: mustJSON(t, contracts.MetadataRecord{Title: "Old"})}
	freshR := cacheEntry{At: time.Now().Unix(), Data: mustJSON(t, contracts.MetadataRecord{Title: "New"})}
	if err := db.Update(func(tx *bolt.Tx) error {
		if err := kv.PutJSON(tx, kv.BCache, recordKey("p", "old"), staleR); err != nil {
			return err
		}
		return kv.PutJSON(tx, kv.BCache, recordKey("p", "new"), freshR)
	}); err != nil {
		t.Fatal(err)
	}
	if _, ok := c.GetRecord("p", "old"); ok {
		t.Fatal("stale record must miss")
	}
	if r, ok := c.GetRecord("p", "new"); !ok || r.Title != "New" {
		t.Fatal("fresh record must hit")
	}
	// Corrupt payloads read as miss. Write raw bytes: PutJSON cannot
	// encode an invalid RawMessage.
	bad := fmt.Sprintf(`{"at":%d,"data":"{"}`, time.Now().Unix())
	if err := db.Update(func(tx *bolt.Tx) error {
		if err := tx.Bucket(kv.BCache).Put(searchKey("anime", "bad"), []byte(bad)); err != nil {
			return err
		}
		return tx.Bucket(kv.BCache).Put(recordKey("p", "bad"), []byte(bad))
	}); err != nil {
		t.Fatal(err)
	}
	if _, ok := c.GetSearch("anime", "bad"); ok {
		t.Fatal("corrupt cached search must miss")
	}
	if _, ok := c.GetRecord("p", "bad"); ok {
		t.Fatal("corrupt cached record must miss")
	}
}
