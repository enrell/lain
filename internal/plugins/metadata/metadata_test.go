package metadata

import (
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/kv"
)

func TestKitsuSearchResolve(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		if r.URL.Path == "/api/edge/anime" {
			w.Write([]byte(`{"data":[{"id":"12","attributes":{"canonicalTitle":"Frieren","titles":{"en":"Frieren","en_jp":"Sousou no Frieren"},"synopsis":"An elf mage.","startDate":"2023-09-29","episodeCount":28,"posterImage":{"original":"https://p.jpg"},"coverImage":{"original":"https://c.jpg"}}}]}`))
			return
		}
		if r.URL.Path == "/api/edge/anime/12" {
			w.Write([]byte(`{"data":{"id":"12","attributes":{"canonicalTitle":"Frieren","titles":{"en":"Frieren"},"synopsis":"An elf mage.","startDate":"2023-09-29","episodeCount":28,"posterImage":{"original":"https://p.jpg"},"coverImage":{"original":"https://c.jpg"}}}}`))
			return
		}
		w.WriteHeader(404)
	}))
	defer srv.Close()
	k := NewKitsu()
	k.BaseURL = srv.URL
	k.http.minGap = 0

	out, err := k.Invoke(contracts.CapMetadataSearch, contracts.MetadataSearchInput{Query: "frieren"})
	if err != nil {
		t.Fatal(err)
	}
	list := out.([]contracts.MetadataCandidate)
	if len(list) != 1 || list[0].Title != "Frieren" || list[0].Year != 2023 || list[0].RemoteID != "12" {
		t.Fatalf("kitsu search: %+v", list)
	}
	rout, err := k.Invoke(contracts.CapMetadataResolve, contracts.MetadataResolveInput{Provider: k.ID(), RemoteID: "12"})
	if err != nil {
		t.Fatal(err)
	}
	rec := rout.(contracts.MetadataRecord)
	if rec.Episodes != 28 || rec.Poster != "https://p.jpg" || rec.Cover != "https://c.jpg" {
		t.Fatalf("kitsu resolve: %+v", rec)
	}
}

func TestAniListSearchResolve(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.Write([]byte(`{"data":{"page":{"media":[{"id":20965,"title":{"romaji":"Frieren","english":"Frieren: Beyond Journey's End","native":"葬送のフリーレン"},"description":"An elf mage.<br>Second line.","coverImage":{"large":"https://l.jpg","extraLarge":"https://xl.jpg"},"bannerImage":"https://b.jpg","episodes":28,"startDate":{"year":2023},"genres":["Adventure"],"synonyms":[]}]}}}`))
	}))
	defer srv.Close()
	a := NewAniList()
	a.BaseURL = srv.URL
	a.http.minGap = 0

	out, err := a.Invoke(contracts.CapMetadataSearch, contracts.MetadataSearchInput{Query: "frieren"})
	if err != nil {
		t.Fatal(err)
	}
	list := out.([]contracts.MetadataCandidate)
	if len(list) != 1 || list[0].RemoteID != "20965" || list[0].Title != "Frieren: Beyond Journey's End" {
		t.Fatalf("anilist search: %+v", list)
	}
	// Resolve hits the same stub (id path differs, body is what matters).
	rout, err := a.Invoke(contracts.CapMetadataResolve, contracts.MetadataResolveInput{Provider: a.ID(), RemoteID: "20965"})
	if err != nil {
		t.Fatal(err)
	}
	_ = rout
}

func TestAniListResolveShape(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.Write([]byte(`{"data":{"media":{"id":1,"title":{"romaji":"R","english":"","native":"N"},"description":"A <i>great</i> show.","coverImage":{"large":"https://l.jpg","extraLarge":""},"bannerImage":"","episodes":12,"startDate":{"year":2020},"genres":["Action"],"synonyms":["S"]}}}`))
	}))
	defer srv.Close()
	a := NewAniList()
	a.BaseURL = srv.URL
	a.http.minGap = 0
	rout, err := a.Invoke(contracts.CapMetadataResolve, contracts.MetadataResolveInput{Provider: a.ID(), RemoteID: "1"})
	if err != nil {
		t.Fatal(err)
	}
	rec := rout.(contracts.MetadataRecord)
	if rec.Title != "R" || rec.Synopsis != "A great show." || rec.Poster != "https://l.jpg" || rec.Year != 2020 {
		t.Fatalf("anilist resolve: %+v", rec)
	}
}

func TestJikanSearchResolve(t *testing.T) {
	mux := http.NewServeMux()
	mux.HandleFunc("/v4/anime", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.Write([]byte(`{"data":[{"mal_id":52991,"title":"Sousou no Frieren","title_english":"Frieren: Beyond Journey's End","title_japanese":"葬送のフリーレン","synopsis":"Elf.","episodes":28,"aired":{"from":"2023-09-29T00:00:00+00:00"},"images":{"jpg":{"image_url":"https://s.jpg","large_image_url":"https://xl.jpg"}},"genres":[{"name":"Adventure"}]}]}`))
	})
	srv := httptest.NewServer(mux)
	defer srv.Close()
	j := NewJikan()
	j.BaseURL = srv.URL
	// No politeness wait in tests.
	j.http.minGap = 0

	out, err := j.Invoke(contracts.CapMetadataSearch, contracts.MetadataSearchInput{Query: "frieren"})
	if err != nil {
		t.Fatal(err)
	}
	list := out.([]contracts.MetadataCandidate)
	if len(list) != 1 || list[0].RemoteID != "52991" || list[0].Year != 2023 {
		t.Fatalf("jikan search: %+v", list)
	}
	if _, err := j.Invoke(contracts.CapMetadataResolve, contracts.MetadataResolveInput{Provider: j.ID(), RemoteID: "52991"}); err == nil {
		t.Fatal("resolve hits /v4/anime/{id} which the stub lacks; want error")
	}
}

func TestNFOOffline(t *testing.T) {
	dir := t.TempDir()
	nfo := `<?xml version="1.0" encoding="UTF-8"?>
<tvshow>
  <title>Local Show</title>
  <plot>A home-ripped series.</plot>
  <premiered>2021-03-01</premiered>
  <genre>Drama</genre><genre>Mystery</genre>
  <thumb aspect="poster">file:///posters/local.jpg</thumb>
  <fanart><thumb>file:///fanart/bg.jpg</thumb></fanart>
</tvshow>`
	if err := os.WriteFile(filepath.Join(dir, "tvshow.nfo"), []byte(nfo), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "notes.txt"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	var n NFO
	out, err := n.Invoke(contracts.CapMetadataSearch, contracts.MetadataSearchInput{Query: "local", Dir: dir})
	if err != nil {
		t.Fatal(err)
	}
	list := out.([]contracts.MetadataCandidate)
	if len(list) != 1 || list[0].Title != "Local Show" || list[0].Year != 2021 {
		t.Fatalf("nfo search: %+v", list)
	}
	rout, err := n.Invoke(contracts.CapMetadataResolve, contracts.MetadataResolveInput{Provider: n.ID(), RemoteID: filepath.Join(dir, "tvshow.nfo")})
	if err != nil {
		t.Fatal(err)
	}
	rec := rout.(contracts.MetadataRecord)
	if rec.Poster != "file:///posters/local.jpg" || rec.Cover != "file:///fanart/bg.jpg" || len(rec.Genres) != 2 {
		t.Fatalf("nfo resolve: %+v", rec)
	}
	// No dir, no opinion — never an error.
	empty, err := n.Invoke(contracts.CapMetadataSearch, contracts.MetadataSearchInput{Query: "x"})
	if err != nil {
		t.Fatal(err)
	}
	if len(empty.([]contracts.MetadataCandidate)) != 0 {
		t.Fatal("nfo without dir must stay silent")
	}
}

func TestMergeDedupAndPrecedence(t *testing.T) {
	a := []contracts.MetadataCandidate{
		{Provider: "p1", RemoteID: "1", Title: "Frieren"},
		{Provider: "p1", RemoteID: "2", Title: "Frieren: Extra"},
	}
	b := []contracts.MetadataCandidate{
		{Provider: "p2", RemoteID: "9", Title: "frieren"}, // same normalized, other provider: kept
		{Provider: "p2", RemoteID: "1", Title: "Frieren"}, // same provider+title shape, other id: kept (ids differ)
	}
	merged := MergeCandidates("Frieren", []any{a, b}, []string{"p1", "p2"}, 10)
	if len(merged) != 3 {
		t.Fatalf("merged=%d, want 3 (p2's duplicate title collapses)", len(merged))
	}
	if merged[0].Title != "Frieren" || merged[0].Provider != "p1" {
		t.Fatalf("exact+precedence winner wrong: %+v", merged[0])
	}
	best, ok := BestPick(merged)
	if !ok || best.Provider != "p1" {
		t.Fatalf("best=%+v ok=%v", best, ok)
	}
	if _, ok := BestPick(nil); ok {
		t.Fatal("empty merge must report false")
	}
}

func TestNormalizeTitle(t *testing.T) {
	if normalizeTitle("Darling in the FranXX") != normalizeTitle("darling-in-the-franxx") {
		t.Fatal("punctuation/case must fold")
	}
}

func openTestDB(t *testing.T) (*bolt.DB, error) {
	t.Helper()
	db, err := kv.Open(t.TempDir())
	if err != nil {
		return nil, err
	}
	t.Cleanup(func() { db.Close() })
	return db, nil
}

func TestSaverOverlayLifecycle(t *testing.T) {
	db, err := openTestDB(t)
	if err != nil {
		t.Fatal(err)
	}
	s := NewSaver(db)
	rec := contracts.MetadataRecord{Provider: "p", RemoteID: "1", Title: "Show", Year: 2020, Poster: "https://x"}
	saved, err := s.Save("item-1", rec)
	if err != nil {
		t.Fatal(err)
	}
	if saved.ItemID != "item-1" || saved.FetchedAt == 0 {
		t.Fatalf("saved: %+v", saved)
	}
	got, ok := s.Get("item-1")
	if !ok || got.Title != "Show" {
		t.Fatalf("get: %+v %v", got, ok)
	}
	if _, ok := s.Get("missing"); ok {
		t.Fatal("missing overlay must report false")
	}
	if err := s.Delete("item-1"); err != nil {
		t.Fatal(err)
	}
	if _, ok := s.Get("item-1"); ok {
		t.Fatal("deleted overlay must be gone")
	}
}

func TestDeleteProviderCleansOverlays(t *testing.T) {
	db, err := openTestDB(t)
	if err != nil {
		t.Fatal(err)
	}
	s := NewSaver(db)
	if _, err := s.Save("a", contracts.MetadataRecord{Provider: "p1", RemoteID: "1", Title: "A"}); err != nil {
		t.Fatal(err)
	}
	if _, err := s.Save("b", contracts.MetadataRecord{Provider: "p2", RemoteID: "2", Title: "B"}); err != nil {
		t.Fatal(err)
	}
	n, err := s.DeleteProvider("p1")
	if err != nil || n != 1 {
		t.Fatalf("n=%d err=%v", n, err)
	}
	if _, ok := s.Get("a"); ok {
		t.Fatal("p1 overlay must be gone")
	}
	if _, ok := s.Get("b"); !ok {
		t.Fatal("p2 overlay must survive")
	}
}

func TestCacheSearchRecordTTL(t *testing.T) {
	db, err := openTestDB(t)
	if err != nil {
		t.Fatal(err)
	}
	c := NewCache(db)
	cands := []contracts.MetadataCandidate{{Provider: "p", RemoteID: "1", Title: "Show"}}
	if err := c.PutSearch("anime", "show", cands); err != nil {
		t.Fatal(err)
	}
	if got, ok := c.GetSearch("anime", "show"); !ok || len(got) != 1 {
		t.Fatalf("cache hit: %+v %v", got, ok)
	}
	if _, ok := c.GetSearch("anime", "other"); ok {
		t.Fatal("different query must miss")
	}
	rec := contracts.MetadataRecord{Provider: "p", RemoteID: "1", Title: "Show"}
	if err := c.PutRecord(rec); err != nil {
		t.Fatal(err)
	}
	if _, ok := c.GetRecord("p", "1"); !ok {
		t.Fatal("record must hit")
	}
	if _, ok := c.GetRecord("p", "2"); ok {
		t.Fatal("other id must miss")
	}
}
