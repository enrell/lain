package acquire

import (
	"context"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"math"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/downloads"
	"github.com/enrell/lain/internal/kv"
	"github.com/enrell/lain/internal/torrent"
)

const maxTorrentFile = 10 << 20

// Deps are what the manager needs from the rest of Lain. The gateway
// wires them to the registry and the library store; tests pass fakes.
type Deps struct {
	DB      *bolt.DB
	DataDir string
	Parse   Parser
	Search  func(contracts.IndexerSearchInput) ([]contracts.SearchResult, error)
	Caps    func(contracts.Indexer) (contracts.IndexerCaps, error)
	// Budget returns the shared download limits (the reading slice's
	// settings) and the bytes the HTTP download manager already uses.
	Budget  func() (downloads.Limits, int64)
	Library func(id string) (contracts.Library, bool)
	// Libraries lists every library: cleanup never deletes inside one.
	Libraries func() []contracts.Library
	// Titles lists a library's existing titles for import matching.
	Titles func(libraryID string) []string
	Rescan func(libraryID string)
	HTTP   *http.Client
	Logger *slog.Logger
	// Engine overrides engine timings (tests); ListenHost binds inbound.
	Engine     *torrent.Config
	ListenHost string
	// Tick is the monitor period (default 2 s).
	Tick time.Duration
	// Initial replaces the defaults when no settings are saved yet.
	Initial *Settings
	// Items lists a library's catalog items (what is present).
	Items func(libraryID string) []contracts.CatalogItem
	// StallAfter overrides the stall window (tests).
	StallAfter time.Duration
	// EpisodeCount asks metadata providers for a title's episode total
	// (0 = unknown). Optional; A-18.
	EpisodeCount func(title, kind string) int
	// Phase 3 (subtitles): the media probe (duration, stream languages)
	// and the lain.subtitle@1 calls. All optional.
	Probe            func(path string) (contracts.MediaInfo, error)
	SubtitleSearch   func(contracts.SubtitleSearchInput) ([]contracts.SubtitleCandidate, error)
	SubtitleDownload func(contracts.SubtitleDownloadInput) (contracts.SubtitleDownloadOutput, error)
}

// Manager runs acquisition.
type Manager struct {
	d   Deps
	st  *store
	log *slog.Logger

	mu        sync.Mutex
	settings  Settings
	client    DownloadClient
	live      map[string]ClientStatus // by grab id
	base      map[string][2]int64     // persisted downloaded/uploaded at session start
	lastCall  map[string]time.Time    // indexer id -> last request
	importing map[string]bool

	// Automation state.
	marks      map[string]mark
	lastRSS    time.Time
	autoBusy   bool
	autoPaused string

	// grabMu serializes Grab so two grabs cannot both pass the budget
	// check for the same free bytes.
	grabMu sync.Mutex

	stop chan struct{}
	wg   sync.WaitGroup
}

// New opens the store and starts the download client.
func New(d Deps) (*Manager, error) {
	if d.Logger == nil {
		d.Logger = slog.New(slog.DiscardHandler)
	}
	if d.HTTP == nil {
		d.HTTP = &http.Client{Timeout: 30 * time.Second}
	}
	if d.Tick <= 0 {
		d.Tick = 2 * time.Second
	}
	st, err := newStore(d.DB)
	if err != nil {
		return nil, err
	}
	m := &Manager{
		d: d, st: st, log: d.Logger, live: map[string]ClientStatus{}, base: map[string][2]int64{},
		lastCall: map[string]time.Time{}, importing: map[string]bool{}, stop: make(chan struct{}),
	}
	def, _ := DefaultSettings(d.DataDir).Validate()
	if d.Initial != nil {
		v, err := d.Initial.Validate()
		if err != nil {
			return nil, err
		}
		def = v
	}
	m.settings = st.settings(def)
	if err := m.startClient(); err != nil {
		return nil, err
	}
	for _, g := range st.grabs() {
		m.base[g.ID] = [2]int64{g.Downloaded, g.Uploaded}
		if g.State == GrabImporting {
			// Interrupted mid-import: run it again (placement is idempotent).
			m.queueImport(g.ID)
		}
	}
	return m, nil
}

func (m *Manager) startClient() error {
	s := m.settings
	n, err := NewNative(m.st, NativeConfig{
		ListenPort: s.ListenPort, ListenHost: m.d.ListenHost, MaxPeers: s.MaxPeers,
		UploadBps: int64(s.UploadKBps) * 1024, DownloadBps: int64(s.DownloadKBps) * 1024,
		Logger: m.log, EngineConfig: m.d.Engine,
	}, ClientEvents{OnMetadata: m.onMetadata, OnComplete: m.onComplete})
	if err != nil {
		return fmt.Errorf("torrent engine: %w", err)
	}
	m.client = n
	return nil
}

// Port is the engine's inbound port (tests dial it).
func (m *Manager) Port() uint16 {
	m.mu.Lock()
	defer m.mu.Unlock()
	if n, ok := m.client.(*Native); ok {
		return n.Port()
	}
	return 0
}

// Start runs the monitor loop.
func (m *Manager) Start() {
	m.wg.Add(1)
	go m.loop()
}

// Close stops the loop and the client.
func (m *Manager) Close() {
	select {
	case <-m.stop:
	default:
		close(m.stop)
	}
	m.wg.Wait()
	m.mu.Lock()
	c := m.client
	m.mu.Unlock()
	m.persistLive()
	c.Close()
}

func (m *Manager) loop() {
	defer m.wg.Done()
	t := time.NewTicker(m.d.Tick)
	defer t.Stop()
	n := 0
	for {
		select {
		case <-m.stop:
			return
		case <-t.C:
		}
		n++
		m.monitor()
		m.schedule(time.Now())
		if n%15 == 0 {
			m.mu.Lock()
			c := m.client
			m.mu.Unlock()
			c.Checkpoint()
			m.persistLive()
		}
	}
}

// ---- settings ----

// Settings returns the current policy.
func (m *Manager) Settings() Settings {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.settings
}

// SetSettings validates and applies a policy. Rates apply live; a new
// port or peer limit restarts the engine (torrents resume from disk).
func (m *Manager) SetSettings(s Settings) (Settings, error) {
	s, err := s.Validate()
	if err != nil {
		return s, err
	}
	if m.d.Libraries != nil {
		if name, bad := overlapsLibrary(s.Dir, m.d.Libraries()); bad {
			return s, errf(CodeInvalid, "the download folder must not be, contain or lie inside a library (%s)", name)
		}
	}
	if err := m.st.saveSettings(s); err != nil {
		return s, err
	}
	m.mu.Lock()
	old := m.settings
	m.settings = s
	c := m.client
	m.mu.Unlock()
	c.SetRates(int64(s.UploadKBps)*1024, int64(s.DownloadKBps)*1024)
	if old.ListenPort != s.ListenPort || old.MaxPeers != s.MaxPeers {
		m.persistLive()
		c.Close()
		m.mu.Lock()
		err := m.startClient()
		m.mu.Unlock()
		if err != nil {
			return s, err
		}
	}
	return s, nil
}

// Usage is acquisition's footprint against the shared limits.
type Usage struct {
	UsedBytes      int64 `json:"used_bytes"`      // torrent data on disk
	DownloadsBytes int64 `json:"downloads_bytes"` // the HTTP download manager's share
	MaxBytes       int64 `json:"max_bytes"`
	MinFreeBytes   int64 `json:"min_free_bytes"`
}

func (m *Manager) usedBytes(except string) int64 {
	var used int64
	for _, g := range m.st.grabs() {
		if g.ID != except && g.OnDisk() {
			used += g.Size
		}
	}
	return used
}

// Usage reports the shared budget.
func (m *Manager) Usage() Usage {
	lim, other := m.budget()
	return Usage{UsedBytes: m.usedBytes(""), DownloadsBytes: other, MaxBytes: lim.MaxBytes, MinFreeBytes: lim.MinFreeBytes}
}

func (m *Manager) budget() (downloads.Limits, int64) {
	if m.d.Budget == nil {
		return downloads.Limits{}, 0
	}
	return m.d.Budget()
}

// reserve checks that size more bytes fit the shared budget (A-4).
func (m *Manager) reserve(except string, size int64) error {
	lim, other := m.budget()
	dir := m.Settings().Dir
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return errf(CodeDiskFull, "cannot create %s: %v", dir, err)
	}
	if err := lim.Check(dir, other+m.usedBytes(except), size); err != nil {
		code := CodeQuota
		if downloads.CodeOf(err) == downloads.CodeDiskFull {
			code = CodeDiskFull
		}
		var de *downloads.Error
		msg := err.Error()
		if errors.As(err, &de) {
			msg = de.Msg
		}
		return &Error{Code: code, Msg: msg}
	}
	return nil
}

// ---- indexers ----

// Indexers lists indexers without credentials.
func (m *Manager) Indexers() []contracts.Indexer {
	out := m.st.indexers()
	for i := range out {
		out[i] = out[i].Public()
	}
	return out
}

// HasAPIKey reports whether an indexer has a stored key.
func (m *Manager) HasAPIKey(id string) bool {
	ix, err := m.st.indexer(id)
	return err == nil && ix.APIKey != ""
}

// CreateIndexer adds an indexer (enabled, 2 s minimum interval).
func (m *Manager) CreateIndexer(in IndexerInput) (contracts.Indexer, error) {
	ix := contracts.Indexer{ID: newID(), Enabled: true, MinIntervalMs: 2000, CreatedAt: time.Now().Unix()}
	if err := in.apply(&ix); err != nil {
		return contracts.Indexer{}, err
	}
	if err := m.st.putIndexer(ix); err != nil {
		return contracts.Indexer{}, err
	}
	return ix.Public(), nil
}

// UpdateIndexer patches an indexer; an empty api_key keeps the old one.
func (m *Manager) UpdateIndexer(id string, in IndexerInput) (contracts.Indexer, error) {
	ix, err := m.st.indexer(id)
	if err != nil {
		return ix, err
	}
	if err := in.apply(&ix); err != nil {
		return contracts.Indexer{}, err
	}
	if err := m.st.putIndexer(ix); err != nil {
		return contracts.Indexer{}, err
	}
	return ix.Public(), nil
}

// DeleteIndexer removes an indexer.
func (m *Manager) DeleteIndexer(id string) error { return m.st.deleteIndexer(id) }

// wait enforces the indexer's minimum interval (A-8).
func (m *Manager) wait(ctx context.Context, ix contracts.Indexer) error {
	for {
		m.mu.Lock()
		next := m.lastCall[ix.ID].Add(time.Duration(ix.MinIntervalMs) * time.Millisecond)
		now := time.Now()
		if !now.Before(next) {
			m.lastCall[ix.ID] = now
			m.mu.Unlock()
			return nil
		}
		m.mu.Unlock()
		select {
		case <-ctx.Done():
			return errf(CodeState, "rate limit wait exceeded the request time")
		case <-time.After(next.Sub(now)):
		}
	}
}

// TestIndexer probes caps and records health.
func (m *Manager) TestIndexer(id string) (contracts.Indexer, error) {
	ix, err := m.st.indexer(id)
	if err != nil {
		return ix, err
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	if err := m.wait(ctx, ix); err != nil {
		return ix.Public(), err
	}
	caps, cerr := m.d.Caps(ix)
	ix.LastCheckAt = time.Now().Unix()
	if cerr != nil {
		ix.LastError = cerr.Error()
	} else {
		ix.LastError, ix.Caps = "", &caps
	}
	if err := m.st.putIndexer(ix); err != nil {
		return ix.Public(), err
	}
	return ix.Public(), cerr
}

// SearchQuery asks every enabled indexer (or the listed ones).
type SearchQuery struct {
	Query      string   `json:"q"`
	Kind       string   `json:"kind"` // library type: anime, series, movie, manga, comic
	Season     int      `json:"season,omitempty"`
	Episode    int      `json:"episode,omitempty"`
	Year       int      `json:"year,omitempty"`
	IndexerIDs []string `json:"indexer_ids,omitempty"`
}

// Candidate is a result with its parse and rank.
type Candidate struct {
	contracts.SearchResult
	Release contracts.Release `json:"release"`
	Score   int               `json:"score"`
	// Rejections explain why a result does not fit the query; rejected
	// results stay visible for manual grabs.
	Rejections []string `json:"rejections,omitempty"`
}

// IndexerFailure reports one indexer that did not answer.
type IndexerFailure struct {
	IndexerID string `json:"indexer_id"`
	Name      string `json:"name"`
	Error     string `json:"error"`
}

// SearchResults is a merged answer.
type SearchResults struct {
	Candidates []Candidate      `json:"candidates"`
	Failures   []IndexerFailure `json:"failures"`
}

func searchKind(kind string) string {
	switch kind {
	case "anime", "series":
		return contracts.SearchTV
	case "movie":
		return contracts.SearchMovie
	case contracts.KindManga, contracts.KindComic:
		return contracts.SearchBook
	}
	return contracts.SearchGeneric
}

// Search queries indexers concurrently, parses and ranks the results.
func (m *Manager) Search(q SearchQuery) (SearchResults, error) {
	q.Query = strings.TrimSpace(q.Query)
	if q.Query == "" || len(q.Query) > 200 {
		return SearchResults{}, errf(CodeInvalid, "a query of 1-200 characters is required")
	}
	want := map[string]bool{}
	for _, id := range q.IndexerIDs {
		want[id] = true
	}
	var targets []contracts.Indexer
	for _, ix := range m.st.indexers() {
		if ix.Enabled && (len(want) == 0 || want[ix.ID]) {
			targets = append(targets, ix)
		}
	}
	if len(targets) == 0 {
		return SearchResults{}, errf(CodeNotFound, "no enabled indexer; add one under Settings › Indexers")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 45*time.Second)
	defer cancel()
	out := SearchResults{Candidates: []Candidate{}, Failures: []IndexerFailure{}}
	var mu sync.Mutex
	var wg sync.WaitGroup
	for _, ix := range targets {
		wg.Add(1)
		go func(ix contracts.Indexer) {
			defer wg.Done()
			err := m.wait(ctx, ix)
			var res []contracts.SearchResult
			if err == nil {
				res, err = m.d.Search(contracts.IndexerSearchInput{
					Indexer: ix, Query: q.Query, Kind: searchKind(q.Kind), Season: q.Season, Episode: q.Episode, Year: q.Year, Limit: 100,
				})
			}
			mu.Lock()
			defer mu.Unlock()
			if err != nil {
				out.Failures = append(out.Failures, IndexerFailure{IndexerID: ix.ID, Name: ix.Name, Error: err.Error()})
				return
			}
			for _, r := range res {
				out.Candidates = append(out.Candidates, m.rank(q, r))
			}
		}(ix)
	}
	wg.Wait()
	sort.SliceStable(out.Candidates, func(i, j int) bool {
		a, b := out.Candidates[i], out.Candidates[j]
		if (len(a.Rejections) == 0) != (len(b.Rejections) == 0) {
			return len(a.Rejections) == 0
		}
		if a.Score != b.Score {
			return a.Score > b.Score
		}
		return a.Seeders > b.Seeders
	})
	return out, nil
}

var resolutionScore = map[string]int{"2160p": 40, "1080p": 30, "720p": 20, "576p": 10, "480p": 5}

// rank is the Phase 1 score: fit to the query first, then swarm health
// and quality. Phase 2 replaces the quality part with profiles.
func (m *Manager) rank(q SearchQuery, r contracts.SearchResult) Candidate {
	c := Candidate{SearchResult: r, Release: m.d.Parse(r.Title, q.Kind)}
	rel := c.Release
	if looseKey(rel.Title) != looseKey(q.Query) {
		c.Rejections = append(c.Rejections, "title does not match")
	}
	if q.Season > 0 && rel.Season > 0 && rel.Season != q.Season {
		c.Rejections = append(c.Rejections, fmt.Sprintf("season %d, wanted %d", rel.Season, q.Season))
	}
	if q.Episode > 0 && len(rel.Episodes) > 0 && !contains(rel.Episodes, q.Episode) {
		c.Rejections = append(c.Rejections, fmt.Sprintf("episode not in release, wanted %d", q.Episode))
	}
	if r.Protocol == contracts.ProtocolUsenet {
		c.Rejections = append(c.Rejections, "usenet: no usenet client yet")
	}
	if r.Protocol == contracts.ProtocolTorrent && r.Seeders == 0 {
		c.Rejections = append(c.Rejections, "no seeders")
	}
	c.Score = resolutionScore[rel.Resolution] + int(10*math.Log2(float64(r.Seeders+1)))
	if rel.Proper || rel.Repack {
		c.Score += 5
	}
	if r.Freeleech {
		c.Score += 2
	}
	return c
}

func contains(xs []int, v int) bool {
	for _, x := range xs {
		if x == v {
			return true
		}
	}
	return false
}

// ---- grabs ----

// GrabInput takes a search result, a .torrent URL or a magnet.
type GrabInput struct {
	Result    *contracts.SearchResult `json:"result,omitempty"`
	URL       string                  `json:"url,omitempty"`
	Magnet    string                  `json:"magnet,omitempty"`
	Title     string                  `json:"title,omitempty"`
	LibraryID string                  `json:"library_id"`
	CreatedBy string                  `json:"-"`
	// Set by automation only.
	MonitoredID string   `json:"-"`
	Units       []Unit   `json:"-"`
	ReplaceFrom []string `json:"-"`
}

// errMagnet carries a redirect to a magnet link out of the HTTP client.
type errMagnet struct{ link string }

func (e errMagnet) Error() string { return "redirected to a magnet link" }

// fetchTorrent downloads a .torrent; indexers (and Jackett/Prowlarr)
// may answer with a redirect to a magnet instead.
func (m *Manager) fetchTorrent(link string) ([]byte, string, error) {
	u, err := url.Parse(link)
	if err != nil || (u.Scheme != "http" && u.Scheme != "https") {
		return nil, "", errf(CodeInvalid, "the link must be http(s)")
	}
	hc := *m.d.HTTP
	hc.CheckRedirect = func(req *http.Request, via []*http.Request) error {
		if req.URL.Scheme == "magnet" {
			return errMagnet{link: req.URL.String()}
		}
		if len(via) >= 5 {
			return errors.New("too many redirects")
		}
		return nil
	}
	res, err := hc.Get(link)
	if err != nil {
		var em errMagnet
		if errors.As(err, &em) {
			return nil, em.link, nil
		}
		// The URL may carry an API key: report the host only.
		return nil, "", errf(CodeFetch, "%s did not answer", u.Host)
	}
	defer res.Body.Close()
	if res.StatusCode != http.StatusOK {
		return nil, "", errf(CodeFetch, "%s answered %d", u.Host, res.StatusCode)
	}
	body, err := io.ReadAll(io.LimitReader(res.Body, maxTorrentFile+1))
	if err != nil || len(body) > maxTorrentFile {
		return nil, "", errf(CodeFetch, "the .torrent from %s is unreadable or too large", u.Host)
	}
	if mag := strings.TrimSpace(string(body)); strings.HasPrefix(mag, "magnet:") {
		return nil, mag, nil
	}
	return body, "", nil
}

// Grab starts acquiring a release into a library.
func (m *Manager) Grab(in GrabInput) (Grab, error) {
	lib, ok := m.d.Library(in.LibraryID)
	if !ok {
		return Grab{}, errf(CodeNoLibrary, "choose the library to import into")
	}
	g := Grab{ID: newID(), LibraryID: lib.ID, Kind: lib.Type, CreatedBy: in.CreatedBy, CreatedAt: time.Now().Unix(), Title: strings.TrimSpace(in.Title),
		MonitoredID: in.MonitoredID, Units: in.Units, ReplaceFrom: in.ReplaceFrom, Upgrade: len(in.ReplaceFrom) > 0}
	link, magnet := strings.TrimSpace(in.URL), strings.TrimSpace(in.Magnet)
	if r := in.Result; r != nil {
		if r.Protocol == contracts.ProtocolUsenet {
			return Grab{}, errf(CodeUnsupported, "usenet releases need a usenet client, which Lain does not have yet")
		}
		g.IndexerID, g.Title = r.IndexerID, r.Title
		link, magnet = r.Link, r.Magnet
	}
	var body []byte
	if link != "" && magnet == "" {
		var err error
		body, magnet, err = m.fetchTorrent(link)
		if err != nil {
			return Grab{}, err
		}
	}
	if body == nil && magnet == "" {
		return Grab{}, errf(CodeInvalid, "a search result, a .torrent URL or a magnet is required")
	}
	if body != nil {
		mi, err := torrent.ParseMetaInfo(body)
		if err != nil {
			return Grab{}, errf(CodeFetch, "not a valid .torrent")
		}
		g.Source, g.Size = "torrent-url", mi.Info.Length
		if g.Title == "" {
			g.Title = mi.Info.Name
		}
		if err := m.reserve("", g.Size); err != nil {
			return Grab{}, err
		}
	} else {
		mg, err := torrent.ParseMagnet(magnet)
		if err != nil {
			return Grab{}, errf(CodeInvalid, "bad magnet link")
		}
		g.Source, g.Magnet = "magnet", magnet
		if g.Title == "" {
			g.Title = mg.Name
		}
		// Size unknown until metadata: refuse only a store already full.
		lim, other := m.budget()
		if err := lim.Full(m.Settings().Dir, other+m.usedBytes("")); err != nil {
			return Grab{}, &Error{Code: CodeQuota, Msg: err.Error()}
		}
	}
	m.grabMu.Lock()
	defer m.grabMu.Unlock()
	if body != nil {
		// Re-check under the lock: another grab may have taken the room.
		if err := m.reserve("", g.Size); err != nil {
			return Grab{}, err
		}
	}
	if g.Title == "" {
		g.Title = "Untitled"
	}
	g.Title = clipText(g.Title, 300)
	g.Release = m.d.Parse(g.Title, lib.Type)

	m.mu.Lock()
	active := m.activeLocked()
	dir := filepath.Join(m.settings.Dir, g.ID)
	c := m.client
	maxActive := m.settings.MaxActive
	m.mu.Unlock()
	paused := active >= maxActive
	ih, size, err := c.Add(AddRequest{Torrent: body, Magnet: magnet, Dir: dir, Paused: paused})
	if err != nil {
		return Grab{}, err
	}
	g.InfoHash, g.Dir = ih, dir
	if size > 0 {
		g.Size = size
	}
	switch {
	case paused:
		g.State = GrabQueued
	case body == nil:
		g.State = GrabMetadata
	default:
		g.State = GrabDownloading
	}
	if err := m.st.putGrab(g); err != nil {
		m.discard(g)
		return Grab{}, err
	}
	m.log.Info("grab added", "grab", g.ID, "library", g.LibraryID, "state", g.State, "bytes", g.Size)
	return g, nil
}

func (m *Manager) activeLocked() int {
	n := 0
	for _, g := range m.st.grabs() {
		if g.State == GrabDownloading || g.State == GrabMetadata {
			n++
		}
	}
	return n
}

func (m *Manager) byInfoHash(ih string) (Grab, bool) {
	for _, g := range m.st.grabs() {
		if g.InfoHash == ih && !g.DataRemoved {
			return g, true
		}
	}
	return Grab{}, false
}

func (m *Manager) onMetadata(ih string, size int64) {
	g, ok := m.byInfoHash(ih)
	if !ok {
		return
	}
	g.Size = size
	if err := m.reserve(g.ID, size); err != nil {
		m.fail(g, err)
		return
	}
	if g.State == GrabMetadata {
		g.State = GrabDownloading
	}
	_ = m.st.putGrab(g)
}

func (m *Manager) onComplete(ih string) {
	g, ok := m.byInfoHash(ih)
	if !ok || g.State == GrabSeeding || g.State == GrabDone {
		return
	}
	g.State, g.FinishedAt = GrabImporting, time.Now().Unix()
	_ = m.st.putGrab(g)
	m.queueImport(g.ID)
}

// fail marks a grab failed and frees its disk share: a failed download
// is removed with its data (it never reached the library).
func (m *Manager) fail(g Grab, err error) {
	g.State, g.Code, g.Error = GrabFailed, CodeOf(err), err.Error()
	if g.Code == "" {
		g.Code = CodeImport
	}
	var e *Error
	if errors.As(err, &e) {
		g.Error = e.Msg
	}
	if g.FinishedAt == 0 && g.InfoHash != "" {
		g.DataRemoved = m.discard(g)
	}
	// A release that failed is not grabbed again by automation (A-22).
	m.block(g, g.Error)
	_ = m.st.putGrab(g)
	m.log.Warn("grab failed", "grab", g.ID, "code", g.Code, "err", g.Error)
}

func (m *Manager) queueImport(id string) {
	m.mu.Lock()
	if m.importing[id] {
		m.mu.Unlock()
		return
	}
	m.importing[id] = true
	m.mu.Unlock()
	m.wg.Add(1)
	go func() {
		defer m.wg.Done()
		defer func() {
			m.mu.Lock()
			delete(m.importing, id)
			m.mu.Unlock()
		}()
		m.runImport(id)
	}()
}

// runImport places the files, rescans, then applies the seeding policy.
func (m *Manager) runImport(id string) {
	g, err := m.st.grab(id)
	if err != nil {
		return
	}
	lib, ok := m.d.Library(g.LibraryID)
	if !ok {
		m.importFailed(g, errf(CodeNoLibrary, "the target library no longer exists"))
		return
	}
	m.mu.Lock()
	c, s := m.client, m.settings
	m.mu.Unlock()
	files := c.Files(g.InfoHash)
	// An upgrade first moves the files it replaces out of the library
	// into the holding folder, so the new names never collide with them
	// and a failed import can put them back.
	held, err := m.holdReplaced(g, lib)
	if err != nil {
		m.importFailed(g, err)
		return
	}
	restore := func() { m.restoreHeld(held) }
	var titles []string
	if m.d.Titles != nil {
		titles = m.d.Titles(lib.ID)
	}
	plan, err := PlanImport(lib, files, g.Release, m.d.Parse, titles)
	if err != nil {
		restore()
		m.importFailed(g, err)
		return
	}
	mode := s.ImportMode
	if mode == ImportMove && s.Seeds() {
		mode = ImportCopy // seeding needs the torrent's own copy
	}
	g.Imported = nil
	for _, it := range plan {
		if it.Exists {
			g.Imported = append(g.Imported, it.Dst)
			continue
		}
		used, err := Place(it.Src, it.Dst, mode)
		if err != nil {
			// Files already placed stay; the held ones return wherever
			// their path is still free (restoreHeld never overwrites).
			restore()
			m.importFailed(g, err)
			return
		}
		if used == ImportCopy && mode == ImportHardlink {
			// A cross-filesystem copy doubles the bytes on disk.
			m.log.Info("import copied instead of linking", "grab", g.ID, "dst", it.Dst)
		}
		g.Imported = append(g.Imported, it.Dst)
	}
	// The quality ledger (path → resolution) outlives the grab: Lain's
	// names carry no quality (D-118), so this is how upgrades know a
	// file already meets the cutoff.
	for _, it := range plan {
		q := it.Release.Resolution
		if q == "" {
			q = g.Release.Resolution
		}
		if q != "" {
			_ = putJSON(m.st, kv.BAcqQuality, it.Dst, q)
		}
	}
	g.Code, g.Error = "", ""
	for _, h := range held {
		g.Replaced = append(g.Replaced, h.to)
	}
	if m.d.Rescan != nil {
		m.d.Rescan(lib.ID)
	}
	if s.Seeds() && mode != ImportMove {
		g.State, g.SeedingAt = GrabSeeding, time.Now().Unix()
	} else {
		g.State = GrabDone
		g.DataRemoved = m.discard(g)
	}
	_ = m.st.putGrab(g)
	m.log.Info("grab imported", "grab", g.ID, "files", len(g.Imported), "state", g.State)
	// Subtitles for a monitored title follow its import (A-36).
	if g.MonitoredID != "" && m.d.SubtitleSearch != nil {
		m.wg.Add(1)
		go func(g Grab) {
			defer m.wg.Done()
			m.subtitlesAfterImport(g)
		}(g)
	}
}

// importFailed keeps the data so the import can be retried by hand.
func (m *Manager) importFailed(g Grab, err error) {
	g.State, g.Code, g.Error = GrabFailed, CodeImport, err.Error()
	var e *Error
	if errors.As(err, &e) {
		g.Code, g.Error = e.Code, e.Msg
	}
	_ = m.st.putGrab(g)
	// Automation must not grab it again; the data stays for a manual
	// retry (RetryImport), which is not affected by the blocklist.
	m.block(g, g.Error)
	m.log.Warn("import failed", "grab", g.ID, "err", g.Error)
}

// monitor refreshes live stats, enforces seeding limits and fills slots.
func (m *Manager) monitor() {
	m.mu.Lock()
	c, s := m.client, m.settings
	m.mu.Unlock()
	grabs := m.st.grabs()
	active := 0
	var queued []Grab
	now := time.Now()
	for _, g := range grabs {
		if g.InfoHash == "" || g.DataRemoved {
			continue
		}
		st, ok := c.Status(g.InfoHash)
		if ok {
			m.mu.Lock()
			m.live[g.ID] = st
			m.mu.Unlock()
		}
		switch g.State {
		case GrabDownloading, GrabMetadata:
			active++
			if ok && st.State == string(torrent.StateError) {
				m.fail(g, errf(CodeImport, "%s", st.Error))
				continue
			}
			if ok && m.stalled(g, st, s, now) {
				m.fail(g, errf(CodeStalled, "no progress for %s", m.stallWindow(s)))
				continue
			}
			if ok && st.State == string(torrent.StateSeeding) && g.FinishedAt == 0 {
				m.onComplete(g.InfoHash) // completed while the event was missed (restart)
			}
		case GrabQueued:
			queued = append(queued, g)
		case GrabSeeding:
			up := m.total(g, st, ok)[1]
			ratioDone := s.SeedRatio > 0 && g.Size > 0 && float64(up)/float64(g.Size) >= s.SeedRatio
			timeDone := s.SeedMinutes > 0 && now.Sub(time.Unix(g.SeedingAt, 0)) >= time.Duration(s.SeedMinutes)*time.Minute
			if ratioDone || timeDone || !s.Seeds() {
				m.stopSeeding(g, s)
			}
		}
	}
	sort.Slice(queued, func(i, j int) bool { return queued[i].CreatedAt < queued[j].CreatedAt })
	for _, g := range queued {
		if active >= s.MaxActive {
			break
		}
		if err := c.Resume(g.InfoHash); err == nil {
			g.State = GrabDownloading
			if g.Source == "magnet" && g.Size == 0 {
				g.State = GrabMetadata
			}
			_ = m.st.putGrab(g)
			active++
		}
	}
}

func (m *Manager) stopSeeding(g Grab, s Settings) {
	m.mu.Lock()
	c := m.client
	m.mu.Unlock()
	t := m.total(g, ClientStatus{}, false)
	g.Downloaded, g.Uploaded = t[0], t[1]
	if s.RemoveAfterSeeding {
		g.DataRemoved = m.discard(g)
	} else if err := c.Remove(g.InfoHash, false); err != nil && CodeOf(err) != CodeNotFound {
		m.log.Warn("could not stop seeding", "grab", g.ID, "err", err.Error())
	}
	g.State = GrabDone
	_ = m.st.putGrab(g)
	m.log.Info("seeding stopped", "grab", g.ID, "uploaded", g.Uploaded, "size", g.Size)
}

// total is cumulative downloaded/uploaded: the persisted base plus this
// engine session's counters (the engine restarts from zero).
func (m *Manager) total(g Grab, st ClientStatus, ok bool) [2]int64 {
	m.mu.Lock()
	defer m.mu.Unlock()
	if !ok {
		st, ok = m.live[g.ID]
	}
	b := m.base[g.ID]
	if !ok {
		return [2]int64{max(b[0], g.Downloaded), max(b[1], g.Uploaded)}
	}
	return [2]int64{b[0] + st.Downloaded, b[1] + st.Uploaded}
}

// persistLive writes live counters into the grab records.
func (m *Manager) persistLive() {
	for _, g := range m.st.grabs() {
		m.mu.Lock()
		st, ok := m.live[g.ID]
		m.mu.Unlock()
		if !ok || g.DataRemoved {
			continue
		}
		t := m.total(g, st, true)
		if t[0] != g.Downloaded || t[1] != g.Uploaded {
			g.Downloaded, g.Uploaded = t[0], t[1]
			_ = m.st.putGrab(g)
		}
	}
}

// Grabs lists grabs with live stats, newest first.
func (m *Manager) Grabs() []Grab {
	out := m.st.grabs()
	for i := range out {
		out[i] = m.withLive(out[i])
	}
	return out
}

func (m *Manager) withLive(g Grab) Grab {
	m.mu.Lock()
	st, ok := m.live[g.ID]
	m.mu.Unlock()
	if ok && !g.DataRemoved {
		t := m.total(g, st, true)
		g.Downloaded, g.Uploaded = t[0], t[1]
		g.Completed, g.Peers, g.DownRate, g.UpRate = st.Completed, st.Peers, st.DownRate, st.UpRate
		if st.Size > 0 {
			g.Size = st.Size
		}
	}
	if g.State == GrabDone || g.State == GrabSeeding {
		g.Completed = g.Size
	}
	return g
}

// Get returns one grab.
func (m *Manager) Get(id string) (Grab, error) {
	g, err := m.st.grab(id)
	if err != nil {
		return g, err
	}
	return m.withLive(g), nil
}

// Pause stops a downloading or seeding grab.
func (m *Manager) Pause(id string) (Grab, error) {
	g, err := m.st.grab(id)
	if err != nil {
		return g, err
	}
	switch g.State {
	case GrabDownloading, GrabMetadata, GrabQueued, GrabSeeding:
	default:
		return g, errf(CodeState, "a %s grab cannot be paused", g.State)
	}
	m.mu.Lock()
	c := m.client
	m.mu.Unlock()
	if err := c.Pause(g.InfoHash); err != nil {
		return g, err
	}
	g.State = GrabPaused
	return g, m.st.putGrab(g)
}

// Resume restarts a paused grab (queued when no slot is free).
func (m *Manager) Resume(id string) (Grab, error) {
	g, err := m.st.grab(id)
	if err != nil {
		return g, err
	}
	if g.State != GrabPaused {
		return g, errf(CodeState, "only a paused grab can be resumed")
	}
	if g.SeedingAt > 0 {
		m.mu.Lock()
		c := m.client
		m.mu.Unlock()
		if err := c.Resume(g.InfoHash); err != nil {
			return g, err
		}
		g.State = GrabSeeding
	} else {
		g.State = GrabQueued // the monitor starts it when a slot is free
	}
	return g, m.st.putGrab(g)
}

// RetryImport runs the import again after a failure.
func (m *Manager) RetryImport(id string) (Grab, error) {
	g, err := m.st.grab(id)
	if err != nil {
		return g, err
	}
	if g.State != GrabFailed || g.FinishedAt == 0 || g.DataRemoved {
		return g, errf(CodeState, "only a finished download whose import failed can be retried")
	}
	g.State, g.Code, g.Error = GrabImporting, "", ""
	if err := m.st.putGrab(g); err != nil {
		return g, err
	}
	m.queueImport(g.ID)
	return g, nil
}

// Remove forgets a grab; deleteData also deletes the torrent's copy
// (never the files imported into the library).
func (m *Manager) Remove(id string, deleteData bool) error {
	g, err := m.st.grab(id)
	if err != nil {
		return err
	}
	if g.State == GrabImporting {
		return errf(CodeState, "wait for the import to finish")
	}
	switch {
	case deleteData:
		m.discard(g) // spares library files and anything imported
	case g.InfoHash != "" && !g.DataRemoved:
		m.mu.Lock()
		c := m.client
		m.mu.Unlock()
		if err := c.Remove(g.InfoHash, false); err != nil && CodeOf(err) != CodeNotFound {
			return err
		}
	}
	m.mu.Lock()
	delete(m.live, id)
	delete(m.base, id)
	m.mu.Unlock()
	return m.st.deleteGrab(id)
}

// Parse exposes the release parser (diagnostics).
func (m *Manager) Parse(name, kind string) contracts.Release { return m.d.Parse(name, kind) }

func clipText(s string, n int) string {
	if len(s) > n {
		return s[:n]
	}
	return s
}
