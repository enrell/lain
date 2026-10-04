package indexer

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/testutil/contract"
)

// Fixture feeds use placeholder names only (D-010).
const capsXML = `<?xml version="1.0" encoding="UTF-8"?>
<caps>
  <limits max="100" default="50"/>
  <searching>
    <search available="yes" supportedParams="q"/>
    <tv-search available="yes" supportedParams="q,season,ep"/>
    <movie-search available="no"/>
  </searching>
  <categories>
    <category id="5000" name="TV"><subcat id="5070" name="Anime"/></category>
    <category id="7000" name="Books"/>
  </categories>
</caps>`

const feedXML = `<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:torznab="http://torznab.com/schemas/2015/feed">
<channel>
  <item>
    <title>[Fansub-A] Show - 05 [1080p]</title>
    <guid>https://tracker-exemplo.invalid/t/1</guid>
    <link>https://tracker-exemplo.invalid/dl/1.torrent</link>
    <comments>https://tracker-exemplo.invalid/t/1</comments>
    <pubDate>Mon, 02 Jan 2006 15:04:05 -0700</pubDate>
    <size>734003200</size>
    <enclosure url="https://tracker-exemplo.invalid/dl/1.torrent" length="734003200" type="application/x-bittorrent"/>
    <torznab:attr name="seeders" value="42"/>
    <torznab:attr name="peers" value="50"/>
    <torznab:attr name="infohash" value="0123456789ABCDEF0123456789ABCDEF01234567"/>
    <torznab:attr name="category" value="5070"/>
    <torznab:attr name="downloadvolumefactor" value="0"/>
  </item>
  <item>
    <title>Show S01E02 720p</title>
    <link>magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567</link>
    <torznab:attr name="size" value="1000"/>
  </item>
  <item>
    <title>no link at all</title>
  </item>
  <item>
    <title>bad link</title>
    <link>javascript:alert(1)</link>
  </item>
</channel>
</rss>`

type fakeIndexer struct {
	*httptest.Server
	mu    sync.Mutex
	last  string
	feeds map[string]string
}

func newFake(t *testing.T) *fakeIndexer {
	f := &fakeIndexer{feeds: map[string]string{"caps": capsXML, "search": feedXML, "tvsearch": feedXML}}
	f.Server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		f.mu.Lock()
		f.last = r.URL.RawQuery
		f.mu.Unlock()
		if r.URL.Path != "/api" {
			http.NotFound(w, r)
			return
		}
		if r.URL.Query().Get("apikey") != "secret-key" {
			_, _ = w.Write([]byte(`<?xml version="1.0"?><error code="100" description="Incorrect user credentials secret-key"/>`))
			return
		}
		feed, ok := f.feeds[r.URL.Query().Get("t")]
		if !ok {
			_, _ = w.Write([]byte(`<error code="203" description="Function not available"/>`))
			return
		}
		_, _ = w.Write([]byte(feed))
	}))
	t.Cleanup(f.Close)
	return f
}

func (f *fakeIndexer) query() string {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.last
}

func ix(f *fakeIndexer, key string) contracts.Indexer {
	return contracts.Indexer{ID: "ix1", Name: "tracker-exemplo", Protocol: contracts.IndexerTorznab, URL: f.URL, APIKey: key, Categories: []int{5070}}
}

func TestCaps(t *testing.T) {
	f := newFake(t)
	out, err := NewTorznab().Invoke(contracts.CapIndexer, contracts.IndexerCapsInput{Indexer: ix(f, "secret-key")})
	if err != nil {
		t.Fatal(err)
	}
	c := out.(contracts.IndexerCaps)
	if !c.Search || !c.TVSearch || c.MovieSearch || c.Limit != 100 || len(c.Categories) != 3 || c.Categories[1].Name != "TV/Anime" {
		t.Fatalf("caps = %+v", c)
	}
}

func TestSearchParsesFeed(t *testing.T) {
	f := newFake(t)
	in := ix(f, "secret-key")
	in.Caps = &contracts.IndexerCaps{TVSearch: true}
	out, err := NewTorznab().Invoke(contracts.CapIndexer, contracts.IndexerSearchInput{Indexer: in, Query: "Show", Kind: contracts.SearchTV, Season: 1, Episode: 2, Limit: 20})
	if err != nil {
		t.Fatal(err)
	}
	q := f.query()
	for _, want := range []string{"t=tvsearch", "season=1", "ep=2", "cat=5070", "q=Show", "limit=20", "extended=1"} {
		if !strings.Contains(q, want) {
			t.Errorf("query %q lacks %q", q, want)
		}
	}
	res := out.(contracts.IndexerSearchOutput).Results
	if len(res) != 2 {
		t.Fatalf("results = %+v", res)
	}
	a := res[0]
	if a.Title != "[Fansub-A] Show - 05 [1080p]" || a.Seeders != 42 || a.Peers != 50 || a.Size != 734003200 ||
		a.InfoHash != "0123456789abcdef0123456789abcdef01234567" || a.Link == "" || !a.Freeleech ||
		a.Protocol != contracts.ProtocolTorrent || a.PublishedAt == 0 || len(a.Categories) != 1 {
		t.Fatalf("first = %+v", a)
	}
	if res[1].Magnet == "" || res[1].Size != 1000 {
		t.Fatalf("magnet item = %+v", res[1])
	}
}

func TestSearchFallsBackWithoutFunction(t *testing.T) {
	f := newFake(t)
	in := ix(f, "secret-key")
	in.Caps = &contracts.IndexerCaps{Search: true, MovieSearch: false}
	if _, err := NewTorznab().Invoke(contracts.CapIndexer, contracts.IndexerSearchInput{Indexer: in, Query: "Film", Kind: contracts.SearchMovie, Year: 2001}); err != nil {
		t.Fatal(err)
	}
	if q := f.query(); !strings.Contains(q, "t=search") || strings.Contains(q, "year=") {
		t.Fatalf("query %q", q)
	}
}

func TestErrorsNeverLeakTheKey(t *testing.T) {
	f := newFake(t)
	_, err := NewTorznab().Invoke(contracts.CapIndexer, contracts.IndexerCapsInput{Indexer: ix(f, "wrong-key")})
	ce, ok := err.(*core.Error)
	if !ok || ce.Code != "auth-failed" {
		t.Fatalf("auth: %v", err)
	}
	// The fake echoes the right key in its error text; the wrong one is
	// what we sent. Neither may appear.
	if strings.Contains(ce.Msg, "wrong-key") {
		t.Fatalf("message leaks the key: %q", ce.Msg)
	}
	key := "secret-key"
	f.feeds = map[string]string{}
	_, err = NewTorznab().Invoke(contracts.CapIndexer, contracts.IndexerSearchInput{Indexer: ix(f, key)})
	if err == nil || strings.Contains(err.Error(), key) {
		t.Fatalf("function error: %v", err)
	}
}

func TestRejectsBadIndexers(t *testing.T) {
	p := NewTorznab()
	_, err := p.Invoke(contracts.CapIndexer, contracts.IndexerCapsInput{Indexer: contracts.Indexer{Protocol: "rss", URL: "http://x"}})
	if ce, ok := err.(*core.Error); !ok || ce.Code != "unsupported-protocol" {
		t.Fatalf("protocol: %v", err)
	}
	_, err = p.Invoke(contracts.CapIndexer, contracts.IndexerCapsInput{Indexer: contracts.Indexer{Protocol: "torznab", URL: "file:///etc/passwd"}})
	if ce, ok := err.(*core.Error); !ok || ce.Code != "invalid-message" {
		t.Fatalf("scheme: %v", err)
	}
}

func TestContract(t *testing.T) {
	contract.Run(t, NewTorznab(), []contract.Cap{{Name: contracts.CapIndexer, Sample: contracts.IndexerCapsInput{Indexer: contracts.Indexer{Protocol: "torznab", URL: "http://127.0.0.1:9/"}}}})
}

func FuzzParseRSS(f *testing.F) {
	f.Add([]byte(feedXML))
	f.Add([]byte(`<rss><channel><item><title>x</title><link>http://a/b</link></item></channel></rss>`))
	f.Fuzz(func(t *testing.T, body []byte) {
		res, err := parseRSS(body, contracts.Indexer{ID: "x", Protocol: "torznab"})
		if err != nil {
			return
		}
		if len(res) > maxResults {
			t.Fatal("unbounded results")
		}
		for _, r := range res {
			if r.Link == "" && r.Magnet == "" {
				t.Fatal("result without a download")
			}
			if r.Link != "" && !strings.HasPrefix(r.Link, "http") {
				t.Fatalf("unsafe link %q", r.Link)
			}
		}
	})
}
