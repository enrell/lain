package metadata

import (
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/enrell/lain/internal/contracts"
)

// Fixture titles are fictional: tests must never commit real
// release-group, tracker or catalogue identities (D-010).

func newTVMazeStub(t *testing.T) *TVMaze {
	t.Helper()
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		if r.URL.Path == "/search/shows" {
			w.Write([]byte(`[{"score":0.9,"show":{"id":407,"name":"Harbor Lights","genres":["Drama","Mystery"],"premiered":"2024-02-01","summary":"<p>A keeper <b>returns</b> to the lighthouse.</p>","image":{"medium":"https://m.jpg","original":"https://o.jpg"}}},{"score":0.4,"show":{"id":408,"name":"","genres":[],"premiered":"","summary":"","image":null}}]`))
			return
		}
		if r.URL.Path == "/shows/407" {
			w.Write([]byte(`{"id":407,"name":"Harbor Lights","genres":["Drama","Mystery"],"premiered":"2024-02-01","summary":"<p>A keeper <b>returns</b> to the lighthouse.</p>","image":{"medium":"https://m.jpg","original":"https://o.jpg"}}`))
			return
		}
		w.WriteHeader(404)
	}))
	t.Cleanup(srv.Close)
	tv := NewTVMaze()
	tv.BaseURL = srv.URL
	tv.http.minGap = 0
	return tv
}

func TestTVMazeSearchResolve(t *testing.T) {
	tv := newTVMazeStub(t)

	out, err := tv.Invoke(contracts.CapMetadataSearch, contracts.MetadataSearchInput{Query: "harbor lights", Kind: "series"})
	if err != nil {
		t.Fatal(err)
	}
	list := out.([]contracts.MetadataCandidate)
	// The nameless hit is dropped, never surfaced.
	if len(list) != 1 || list[0].Title != "Harbor Lights" || list[0].Year != 2024 || list[0].RemoteID != "407" {
		t.Fatalf("tvmaze search: %+v", list)
	}
	if list[0].Provider != tv.ID() || list[0].Kind != "series" || list[0].Poster != "https://o.jpg" {
		t.Fatalf("tvmaze candidate shape: %+v", list[0])
	}

	rout, err := tv.Invoke(contracts.CapMetadataResolve, contracts.MetadataResolveInput{Provider: tv.ID(), RemoteID: "407"})
	if err != nil {
		t.Fatal(err)
	}
	rec := rout.(contracts.MetadataRecord)
	if rec.Title != "Harbor Lights" || rec.Year != 2024 || len(rec.Genres) != 2 {
		t.Fatalf("tvmaze resolve: %+v", rec)
	}
	if rec.Synopsis != "A keeper returns to the lighthouse." {
		t.Fatalf("tvmaze synopsis must be plain text, got %q", rec.Synopsis)
	}
	if rec.Poster != "https://o.jpg" || rec.Cover != "" {
		t.Fatalf("tvmaze artwork: poster=%q cover=%q", rec.Poster, rec.Cover)
	}
}

func TestTVMazeAcceptsVideoKinds(t *testing.T) {
	tv := newTVMazeStub(t)
	// Episodes enrich with kind "anime" (see enrichOne); all video
	// kinds must reach the provider, only movies stay out (TVMaze is
	// TV-only).
	for _, kind := range []string{"", "anime", "series", "episode", "video"} {
		out, err := tv.Invoke(contracts.CapMetadataSearch, contracts.MetadataSearchInput{Query: "harbor", Kind: kind})
		if err != nil {
			t.Fatalf("kind %q: %v", kind, err)
		}
		if len(out.([]contracts.MetadataCandidate)) != 1 {
			t.Fatalf("kind %q must search, got %+v", kind, out)
		}
	}
	out, err := tv.Invoke(contracts.CapMetadataSearch, contracts.MetadataSearchInput{Query: "harbor", Kind: "movie"})
	if err != nil {
		t.Fatal(err)
	}
	if len(out.([]contracts.MetadataCandidate)) != 0 {
		t.Fatalf("movie kind must stay silent, got %+v", out)
	}
}

func TestTVMazeUpstreamError(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(500)
	}))
	defer srv.Close()
	tv := NewTVMaze()
	tv.BaseURL = srv.URL
	tv.http.minGap = 0
	// Errors propagate so the merge can skip this provider, never fail.
	if _, err := tv.Invoke(contracts.CapMetadataSearch, contracts.MetadataSearchInput{Query: "x"}); err == nil {
		t.Fatal("search outage must error")
	}
	if _, err := tv.Invoke(contracts.CapMetadataResolve, contracts.MetadataResolveInput{RemoteID: "1"}); err == nil {
		t.Fatal("resolve outage must error")
	}
	if _, err := tv.Invoke("lain.metadata.other@1", contracts.MetadataSearchInput{}); err == nil {
		t.Fatal("wrong capability must fail")
	}
	if _, err := tv.Invoke(contracts.CapMetadataSearch, "not-a-request"); err == nil {
		t.Fatal("wrong input type must fail")
	}
}

func TestTVMazeMissingImage(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.Write([]byte(`{"id":409,"name":"Copper Valley","genres":["Western"],"premiered":"2022-09-10","summary":"<p>Dust.</p>","image":null}`))
	}))
	defer srv.Close()
	tv := NewTVMaze()
	tv.BaseURL = srv.URL
	tv.http.minGap = 0
	rout, err := tv.Invoke(contracts.CapMetadataResolve, contracts.MetadataResolveInput{RemoteID: "409"})
	if err != nil {
		t.Fatal(err)
	}
	rec := rout.(contracts.MetadataRecord)
	if rec.Title != "Copper Valley" || rec.Poster != "" || rec.Synopsis != "Dust." {
		t.Fatalf("imageless resolve: %+v", rec)
	}
}
