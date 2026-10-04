package torrent

import (
	"bufio"
	"context"
	"errors"
	"net"
	"net/netip"
	"os"
	"path/filepath"
	"sort"
	"sync"
	"time"
)

// State of a torrent.
type State string

const (
	StateMetadata    State = "metadata"
	StateChecking    State = "checking"
	StateDownloading State = "downloading"
	StateSeeding     State = "seeding"
	StatePaused      State = "paused"
	StateError       State = "error"
)

// Spec describes a torrent to add: from a .torrent (MetaInfo) or a
// magnet (InfoHash plus trackers; metadata is fetched from peers).
type Spec struct {
	MetaInfo *MetaInfo
	InfoHash InfoHash
	Name     string
	Trackers []string
	// Peers are dialed in addition to tracker results.
	Peers []netip.AddrPort
	// Dir receives the torrent's files.
	Dir string
	// Have is a saved resume bitfield, trusted only when every file is
	// already on disk at full size; otherwise present data is rechecked.
	Have   Bitfield
	Paused bool
	// OnComplete runs once, in its own goroutine, after the last piece
	// verifies.
	OnComplete func(*Torrent)
	// OnMetadata runs once when a magnet's info dictionary arrives.
	OnMetadata func(*Torrent)
}

type blockKey struct{ piece, begin int }

type pieceState struct {
	got        []bool
	gotN       int
	reqs       map[int][]*peer // block index -> requesters
	verifying  bool
	failures   int
	contribute map[*peer]bool
}

// Torrent is one swarm membership.
type Torrent struct {
	c   *Client
	ih  InfoHash
	dir string

	onComplete func(*Torrent)
	onMetadata func(*Torrent)

	mu       sync.Mutex
	name     string
	info     *Info
	st       *storage
	trackers []string
	have     Bitfield
	pieces   []pieceState
	active   map[int]bool // pieces with requested or received blocks
	avail    []int
	peers    map[*peer]bool
	// known maps a peer address to the earliest time it may be dialed;
	// failures double the wait (5 s up to 5 min), a connection resets it.
	known      map[netip.AddrPort]time.Time
	backoff    map[netip.AddrPort]time.Duration
	state      State
	err        string
	downloaded int64
	uploaded   int64
	seeders    int
	leechers   int
	completed  bool
	resume     Bitfield

	// magnet metadata assembly
	metaSize   int
	metaPieces [][]byte
	metaAsked  map[int]time.Time

	// rates sampled by the ticker
	lastDown, lastUp int64
	downRate, upRate int64

	ctx     context.Context
	cancel  context.CancelFunc
	wg      sync.WaitGroup
	running bool
	newPeer chan struct{}
	// announceNow wakes the announce loops (completion).
	announceNow chan struct{}
}

func newTorrent(c *Client, s Spec) (*Torrent, error) {
	if s.Dir == "" {
		return nil, errors.New("torrent: spec needs a directory")
	}
	t := &Torrent{
		c: c, dir: s.Dir, onComplete: s.OnComplete, onMetadata: s.OnMetadata,
		name: s.Name, peers: map[*peer]bool{}, known: map[netip.AddrPort]time.Time{}, backoff: map[netip.AddrPort]time.Duration{},
		active: map[int]bool{}, state: StatePaused, metaAsked: map[int]time.Time{},
		newPeer: make(chan struct{}, 1), announceNow: make(chan struct{}, 8), resume: s.Have,
	}
	trackers := s.Trackers
	if s.MetaInfo != nil {
		t.ih = s.MetaInfo.InfoHash
		trackers = append(append([]string(nil), s.MetaInfo.Trackers...), s.Trackers...)
		if t.name == "" {
			t.name = s.MetaInfo.Info.Name
		}
	} else {
		if s.InfoHash == (InfoHash{}) {
			return nil, errors.New("torrent: spec needs metainfo or an info hash")
		}
		t.ih = s.InfoHash
	}
	seen := map[string]bool{}
	for _, tr := range trackers {
		if !seen[tr] && validTrackerURL(tr) {
			seen[tr] = true
			t.trackers = append(t.trackers, tr)
		}
	}
	for _, p := range s.Peers {
		t.known[p] = time.Time{}
	}
	if s.MetaInfo != nil {
		t.setInfoLocked(s.MetaInfo.Info)
	}
	return t, nil
}

// InfoHash identifies the torrent.
func (t *Torrent) InfoHash() InfoHash { return t.ih }

// Info is the metainfo once known (nil while a magnet fetches it).
func (t *Torrent) Info() *Info {
	t.mu.Lock()
	defer t.mu.Unlock()
	return t.info
}

// setInfoLocked installs the info dictionary: storage, piece tables.
func (t *Torrent) setInfoLocked(info *Info) {
	t.info = info
	if t.name == "" {
		t.name = info.Name
	}
	n := info.NumPieces()
	t.st = newStorage(t.dir, info)
	t.have = NewBitfield(n)
	t.avail = make([]int, n)
	t.pieces = make([]pieceState, n)
	for i := range t.pieces {
		blocks := int((info.PieceSize(i) + blockSize - 1) / blockSize)
		if blocks == 0 {
			blocks = 1
		}
		t.pieces[i] = pieceState{got: make([]bool, blocks), reqs: map[int][]*peer{}, contribute: map[*peer]bool{}}
	}
	for p := range t.peers {
		if p.bf != nil && !p.bf.Valid(n) {
			// Haves before the metadata may have grown the set; trim.
			bf := NewBitfield(n)
			for i := 0; i < n; i++ {
				if p.bf.Has(i) {
					bf.Set(i)
				}
			}
			p.bf = bf
		}
		if p.bf == nil {
			p.bf = NewBitfield(n)
		}
		for i := 0; i < n; i++ {
			if p.bf.Has(i) {
				t.avail[i]++
			}
		}
	}
}

// Resume starts (or restarts) the torrent.
func (t *Torrent) Resume() {
	t.mu.Lock()
	if t.running {
		t.mu.Unlock()
		return
	}
	t.running = true
	t.err = ""
	t.ctx, t.cancel = context.WithCancel(t.c.ctx)
	if t.info == nil {
		t.state = StateMetadata
	} else {
		t.state = StateChecking
	}
	needCheck := t.info != nil
	t.mu.Unlock()

	t.wg.Add(1)
	go func() {
		defer t.wg.Done()
		if needCheck {
			t.check()
		}
		if t.ctx.Err() != nil {
			return
		}
		for _, tr := range t.trackers {
			t.wg.Add(1)
			go t.announceLoop(tr)
		}
		t.wg.Add(2)
		go t.dialLoop()
		go t.tickLoop()
	}()
}

// check establishes which pieces are on disk.
func (t *Torrent) check() {
	t.mu.Lock()
	st, info, resume := t.st, t.info, t.resume
	t.mu.Unlock()
	n := info.NumPieces()
	have := NewBitfield(n)
	if resume != nil && resume.Valid(n) && st.Complete() {
		have = resume.Clone()
	} else {
		for i := 0; i < n; i++ {
			if t.ctx.Err() != nil {
				return
			}
			if !t.pieceOnDisk(i) {
				continue
			}
			if ok, err := st.Verify(i); err == nil && ok {
				have.Set(i)
			}
		}
	}
	t.mu.Lock()
	t.have = have
	t.completed = have.Count(n) == n
	if t.completed {
		t.state = StateSeeding
	} else {
		t.state = StateDownloading
	}
	done := t.completed
	t.mu.Unlock()
	if done {
		t.finalize(false)
	}
}

// pieceOnDisk is a cheap pre-check: every file the piece touches exists.
func (t *Torrent) pieceOnDisk(i int) bool {
	off := int64(i) * t.info.PieceLength
	ok := true
	_ = t.st.span(off, t.info.PieceSize(i), func(fi int, _, _, _ int64) error {
		if _, err := os.Stat(t.st.path(fi)); err != nil {
			ok = false
		}
		return nil
	})
	return ok
}

// Pause disconnects every peer and tells trackers we stopped.
func (t *Torrent) Pause() { t.stop(true) }

func (t *Torrent) stop(announceStopped bool) {
	t.mu.Lock()
	if !t.running {
		t.mu.Unlock()
		return
	}
	t.running = false
	t.cancel()
	peers := make([]*peer, 0, len(t.peers))
	for p := range t.peers {
		peers = append(peers, p)
	}
	t.mu.Unlock()
	for _, p := range peers {
		p.close()
	}
	t.wg.Wait()
	t.mu.Lock()
	if t.state != StateError {
		t.state = StatePaused
	}
	trackers := t.trackers
	req := t.announceReqLocked(EventStopped)
	t.mu.Unlock()
	if announceStopped && t.info != nil {
		ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
		for _, tr := range trackers {
			_, _ = Announce(ctx, t.c.cfg.HTTPClient, tr, req)
		}
		cancel()
	}
	if t.st != nil {
		t.st.Close()
	}
}

// Remove stops the torrent, forgets it and optionally deletes its data.
func (t *Torrent) Remove(deleteData bool) error {
	t.stop(true)
	t.c.forget(t)
	if deleteData && t.st != nil {
		return t.st.Remove()
	}
	return nil
}

// FileInfo is a torrent file on disk.
type FileInfo struct {
	Path   string `json:"path"` // absolute
	Rel    string `json:"rel"`
	Length int64  `json:"length"`
}

// Files lists the torrent's files (empty until metadata is known).
func (t *Torrent) Files() []FileInfo {
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.info == nil {
		return nil
	}
	out := make([]FileInfo, len(t.info.Files))
	for i, f := range t.info.Files {
		out[i] = FileInfo{Path: t.st.path(i), Rel: f.RelPath(), Length: f.Length}
	}
	return out
}

// Stats is a snapshot for the queue view.
type Stats struct {
	InfoHash   string `json:"info_hash"`
	Name       string `json:"name"`
	State      State  `json:"state"`
	Error      string `json:"error,omitempty"`
	Size       int64  `json:"size"`
	Completed  int64  `json:"completed"`
	Downloaded int64  `json:"downloaded"`
	Uploaded   int64  `json:"uploaded"`
	Peers      int    `json:"peers"`
	Seeders    int    `json:"seeders"`
	Leechers   int    `json:"leechers"`
	DownRate   int64  `json:"down_rate"`
	UpRate     int64  `json:"up_rate"`
	Pieces     int    `json:"pieces"`
	Have       int    `json:"have"`
}

// Stats reports progress.
func (t *Torrent) Stats() Stats {
	t.mu.Lock()
	defer t.mu.Unlock()
	s := Stats{
		InfoHash: t.ih.Hex(), Name: t.name, State: t.state, Error: t.err,
		Downloaded: t.downloaded, Uploaded: t.uploaded, Peers: len(t.peers),
		Seeders: t.seeders, Leechers: t.leechers, DownRate: t.downRate, UpRate: t.upRate,
	}
	if t.info != nil {
		s.Size = t.info.Length
		s.Pieces = t.info.NumPieces()
		for i := 0; i < s.Pieces; i++ {
			if t.have.Has(i) {
				s.Have++
				s.Completed += t.info.PieceSize(i)
			}
		}
	}
	return s
}

// Bitfield is a copy of the verified piece set (for resume).
func (t *Torrent) Bitfield() Bitfield {
	t.mu.Lock()
	defer t.mu.Unlock()
	return t.have.Clone()
}

// AddPeers dials extra peers.
func (t *Torrent) AddPeers(peers []netip.AddrPort) {
	t.mu.Lock()
	for _, p := range peers {
		if _, ok := t.known[p]; !ok {
			t.known[p] = time.Time{}
		}
	}
	t.mu.Unlock()
	t.poke()
}

func (t *Torrent) poke() {
	select {
	case t.newPeer <- struct{}{}:
	default:
	}
}

func (t *Torrent) fail(msg string) {
	t.mu.Lock()
	t.state = StateError
	t.err = msg
	t.mu.Unlock()
	t.c.cfg.Logger.Warn("torrent failed", "torrent", t.ih.Hex(), "err", msg)
	go t.stop(true)
}

func (t *Torrent) announceReqLocked(event string) AnnounceRequest {
	var left int64
	if t.info != nil {
		left = t.info.Length
		for i := 0; i < t.info.NumPieces(); i++ {
			if t.have.Has(i) {
				left -= t.info.PieceSize(i)
			}
		}
	} else {
		left = 1 // unknown size: still leeching
	}
	return AnnounceRequest{
		InfoHash: t.ih, PeerID: t.c.cfg.PeerID, Port: t.c.Port(),
		Uploaded: t.uploaded, Downloaded: t.downloaded, Left: left, Event: event, NumWant: 50,
	}
}

func (t *Torrent) announceLoop(tracker string) {
	defer t.wg.Done()
	event := EventStarted
	// A torrent that starts complete never sends "completed": trackers
	// only want it for a download that finished in this swarm session.
	sentCompleted := t.completedNow()
	backoff := 15 * time.Second
	for {
		t.mu.Lock()
		if event == EventNone && t.completed && !sentCompleted {
			event = EventCompleted
		}
		req := t.announceReqLocked(event)
		t.mu.Unlock()
		ctx, cancel := context.WithTimeout(t.ctx, 30*time.Second)
		res, err := Announce(ctx, t.c.cfg.HTTPClient, tracker, req)
		cancel()
		wait := backoff
		if err != nil {
			if t.ctx.Err() != nil {
				return
			}
			t.c.cfg.Logger.Info("announce failed", "torrent", t.ih.Hex(), "err", err.Error())
			backoff = min(backoff*2, 30*time.Minute)
		} else {
			backoff = 15 * time.Second
			if event == EventCompleted {
				sentCompleted = true
			}
			event = EventNone
			wait = max(res.Interval, t.c.cfg.MinAnnounceInterval)
			t.mu.Lock()
			t.seeders, t.leechers = res.Seeders, res.Leechers
			t.mu.Unlock()
			t.AddPeers(res.Peers)
		}
		timer := time.NewTimer(wait)
		select {
		case <-t.ctx.Done():
			timer.Stop()
			return
		case <-timer.C:
		case <-t.announceNow:
			timer.Stop()
		}
	}
}

func (t *Torrent) completedNow() bool {
	t.mu.Lock()
	defer t.mu.Unlock()
	return t.completed
}

// dialLoop connects to known peers while below the peer limit.
func (t *Torrent) dialLoop() {
	defer t.wg.Done()
	tick := time.NewTicker(2 * time.Second)
	defer tick.Stop()
	for {
		t.dialSome()
		select {
		case <-t.ctx.Done():
			return
		case <-tick.C:
		case <-t.newPeer:
		}
	}
}

func (t *Torrent) dialSome() {
	t.mu.Lock()
	slots := t.c.cfg.MaxPeersPerTorrent - len(t.peers)
	connected := map[netip.AddrPort]bool{}
	for p := range t.peers {
		connected[p.addr] = true
	}
	var targets []netip.AddrPort
	now := time.Now()
	for a, next := range t.known {
		if len(targets) >= slots {
			break
		}
		if connected[a] || now.Before(next) {
			continue
		}
		wait := t.backoff[a]
		if wait == 0 {
			wait = 5 * time.Second
		}
		t.known[a] = now.Add(wait)
		t.backoff[a] = min(2*wait, 5*time.Minute)
		targets = append(targets, a)
	}
	t.mu.Unlock()
	for _, a := range targets {
		t.wg.Add(1)
		go func(a netip.AddrPort) {
			defer t.wg.Done()
			t.dial(a)
		}(a)
	}
}

func (t *Torrent) dial(a netip.AddrPort) {
	d := net.Dialer{Timeout: t.c.cfg.DialTimeout}
	conn, err := d.DialContext(t.ctx, "tcp", a.String())
	if err != nil {
		return
	}
	_ = conn.SetDeadline(time.Now().Add(t.c.cfg.DialTimeout))
	if err := writeHandshake(conn, t.ih, t.c.cfg.PeerID); err != nil {
		conn.Close()
		return
	}
	r := bufio.NewReaderSize(conn, 64<<10)
	hs, err := readHandshake(r)
	if err != nil || hs.InfoHash != t.ih || hs.PeerID == t.c.cfg.PeerID {
		conn.Close()
		return
	}
	_ = conn.SetDeadline(time.Time{})
	t.attach(conn, r, hs, a)
}

// tickLoop runs timeouts, choking and rate sampling.
func (t *Torrent) tickLoop() {
	defer t.wg.Done()
	tick := time.NewTicker(time.Second)
	defer tick.Stop()
	n := 0
	for {
		select {
		case <-t.ctx.Done():
			return
		case <-tick.C:
		}
		n++
		t.mu.Lock()
		t.downRate, t.lastDown = t.downloaded-t.lastDown, t.downloaded
		t.upRate, t.lastUp = t.uploaded-t.lastUp, t.uploaded
		t.expireRequestsLocked()
		if n%10 == 0 {
			t.rechokeLocked(true)
		}
		t.expireMetaLocked()
		for p := range t.peers {
			t.fillLocked(p)
		}
		t.mu.Unlock()
	}
}

func (t *Torrent) expireRequestsLocked() {
	deadline := time.Now().Add(-t.c.cfg.RequestTimeout)
	for p := range t.peers {
		for k, at := range p.inflight {
			if at.Before(deadline) {
				t.dropRequestLocked(p, k)
			}
		}
	}
}

func (t *Torrent) dropRequestLocked(p *peer, k blockKey) {
	delete(p.inflight, k)
	if t.info == nil || k.piece >= len(t.pieces) {
		return
	}
	ps := &t.pieces[k.piece]
	b := k.begin / blockSize
	reqs := ps.reqs[b]
	for i, q := range reqs {
		if q == p {
			reqs = append(reqs[:i], reqs[i+1:]...)
			break
		}
	}
	if len(reqs) == 0 {
		delete(ps.reqs, b)
	} else {
		ps.reqs[b] = reqs
	}
}

// rechokeLocked unchokes up to UploadSlots interested peers, rotating
// one slot every period so newcomers get a turn.
func (t *Torrent) rechokeLocked(rotate bool) {
	var unchoked, waiting []*peer
	for p := range t.peers {
		if !p.peerInterested {
			if !p.amChoking {
				p.setChoking(true)
			}
			continue
		}
		if p.amChoking {
			waiting = append(waiting, p)
		} else {
			unchoked = append(unchoked, p)
		}
	}
	slots := t.c.cfg.UploadSlots
	if rotate && len(waiting) > 0 && len(unchoked) >= slots && len(unchoked) > 0 {
		sort.Slice(unchoked, func(i, j int) bool { return unchoked[i].unchokedAt.Before(unchoked[j].unchokedAt) })
		unchoked[0].setChoking(true)
		unchoked = unchoked[1:]
	}
	sort.Slice(waiting, func(i, j int) bool { return waiting[i].interestedAt.Before(waiting[j].interestedAt) })
	for _, p := range waiting {
		if len(unchoked) >= slots {
			break
		}
		p.setChoking(false)
		unchoked = append(unchoked, p)
	}
}

// finalize runs after the last piece: extends never-written files to
// their size and fires OnComplete once.
func (t *Torrent) finalize(fire bool) {
	t.mu.Lock()
	info, st := t.info, t.st
	t.mu.Unlock()
	for i, f := range info.Files {
		if f.Length != 0 {
			continue
		}
		p := st.path(i)
		if err := os.MkdirAll(filepath.Dir(p), 0o755); err == nil {
			if fh, err := os.OpenFile(p, os.O_CREATE|os.O_WRONLY, 0o644); err == nil {
				fh.Close()
			}
		}
	}
	if fire {
		for range t.trackers {
			select {
			case t.announceNow <- struct{}{}:
			default:
			}
		}
	}
	if fire && t.onComplete != nil {
		go t.onComplete(t)
	}
}
