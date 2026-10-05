package acquire

import (
	"fmt"
	"log/slog"
	"net"
	"strconv"
	"sync"

	"github.com/enrell/lain/internal/torrent"
	"github.com/enrell/lain/internal/torrent/bencode"
)

// File is one file of a download on disk.
type File struct {
	Path   string `json:"path"` // absolute
	Rel    string `json:"rel"`  // inside the torrent
	Length int64  `json:"length"`
}

// ClientStatus is a download client's view of one torrent.
type ClientStatus struct {
	State      string `json:"state"` // metadata, checking, downloading, seeding, paused, error
	Size       int64  `json:"size"`
	Completed  int64  `json:"completed"`
	Downloaded int64  `json:"downloaded"`
	Uploaded   int64  `json:"uploaded"`
	Peers      int    `json:"peers"`
	DownRate   int64  `json:"down_rate"`
	UpRate     int64  `json:"up_rate"`
	Error      string `json:"error,omitempty"`
}

// AddRequest hands a torrent to a client: a .torrent body or a magnet.
type AddRequest struct {
	Torrent []byte
	Magnet  string
	Dir     string
	Paused  bool
}

// DownloadClient is what the acquisition manager drives (A-3). The
// native engine is the first implementation; qBittorrent/Transmission
// adapters can implement it later without touching import.
type DownloadClient interface {
	Name() string
	// Add returns the torrent's info hash (hex) and its size, 0 while a
	// magnet's metadata is unknown.
	Add(req AddRequest) (infoHash string, size int64, err error)
	Status(infoHash string) (ClientStatus, bool)
	Files(infoHash string) []File
	Pause(infoHash string) error
	Resume(infoHash string) error
	Remove(infoHash string, deleteData bool) error
	SetRates(uploadBps, downloadBps int64)
	// Checkpoint persists resume state (called periodically and on close).
	Checkpoint()
	Close()
}

// Events a client reports back.
type ClientEvents struct {
	// OnMetadata fires when a magnet's size becomes known.
	OnMetadata func(infoHash string, size int64)
	OnComplete func(infoHash string)
}

// Native is the standard-library engine as a DownloadClient.
type Native struct {
	st     *store
	c      *torrent.Client
	events ClientEvents
	log    *slog.Logger
	mu     sync.Mutex
}

// NativeConfig configures the engine from settings.
type NativeConfig struct {
	ListenPort   int // 0 picks a free port at each start
	ListenHost   string
	MaxPeers     int
	UploadBps    int64
	DownloadBps  int64
	Logger       *slog.Logger
	EngineConfig *torrent.Config // tests override timings
}

// NewNative starts the engine and restores persisted torrents.
func NewNative(st *store, cfg NativeConfig, ev ClientEvents) (*Native, error) {
	ec := torrent.Config{}
	if cfg.EngineConfig != nil {
		ec = *cfg.EngineConfig
	}
	// Always listen: peers must be able to reach a seeder, and trackers
	// reject a port of 0. Port 0 lets the OS pick a free one.
	ec.ListenAddr = net.JoinHostPort(cfg.ListenHost, strconv.Itoa(cfg.ListenPort))
	if cfg.MaxPeers > 0 {
		ec.MaxPeersPerTorrent = cfg.MaxPeers
	}
	ec.UploadRate, ec.DownloadRate, ec.Logger = cfg.UploadBps, cfg.DownloadBps, cfg.Logger
	c, err := torrent.NewClient(ec)
	if err != nil {
		return nil, err
	}
	n := &Native{st: st, c: c, events: ev, log: cfg.Logger}
	if n.log == nil {
		n.log = slog.New(slog.DiscardHandler)
	}
	for ih, ts := range st.torrents() {
		if err := n.restore(ih, ts); err != nil {
			n.log.Warn("torrent not restored", "torrent", ih, "err", err.Error())
		}
	}
	return n, nil
}

// Port is the bound inbound port.
func (n *Native) Port() uint16 { return n.c.Port() }

func (n *Native) Name() string { return "native" }

func (n *Native) spec(dir string, have torrent.Bitfield, paused bool) torrent.Spec {
	return torrent.Spec{
		Dir: dir, Have: have, Paused: paused,
		OnComplete: func(t *torrent.Torrent) {
			n.save(t)
			if n.events.OnComplete != nil {
				n.events.OnComplete(t.InfoHash().Hex())
			}
		},
		OnMetadata: func(t *torrent.Torrent) {
			n.save(t)
			if n.events.OnMetadata != nil {
				if info := t.Info(); info != nil {
					n.events.OnMetadata(t.InfoHash().Hex(), info.Length)
				}
			}
		},
	}
}

func (n *Native) restore(ih string, ts torrentState) error {
	h, err := torrent.ParseInfoHash(ih)
	if err != nil {
		return err
	}
	if len(ts.MetaInfo) == 0 {
		m, err := torrent.ParseMagnet(ts.Magnet)
		if err != nil || m.InfoHash != h {
			return fmt.Errorf("no metainfo or magnet saved for %s", ih)
		}
		s := n.spec(ts.Dir, nil, ts.Paused)
		s.InfoHash, s.Name, s.Trackers = m.InfoHash, m.Name, m.Trackers
		_, err = n.c.Add(s)
		return err
	}
	mi, err := torrent.ParseMetaInfo(ts.MetaInfo)
	if err != nil || mi.InfoHash != h {
		return fmt.Errorf("saved metainfo invalid")
	}
	s := n.spec(ts.Dir, ts.Have, ts.Paused)
	s.MetaInfo = mi
	_, err = n.c.Add(s)
	return err
}

// Add starts a torrent.
func (n *Native) Add(req AddRequest) (string, int64, error) {
	s := n.spec(req.Dir, nil, req.Paused)
	var raw []byte
	switch {
	case len(req.Torrent) > 0:
		mi, err := torrent.ParseMetaInfo(req.Torrent)
		if err != nil {
			return "", 0, errf(CodeFetch, "not a valid .torrent: %v", err)
		}
		s.MetaInfo, raw = mi, req.Torrent
	case req.Magnet != "":
		m, err := torrent.ParseMagnet(req.Magnet)
		if err != nil {
			return "", 0, errf(CodeInvalid, "bad magnet: %v", err)
		}
		if len(m.Trackers) == 0 {
			// No DHT in v1 (A-2): a magnet needs a tracker to find peers.
			return "", 0, errf(CodeUnsupported, "this magnet has no tracker; Lain's engine has no DHT yet — use the .torrent link")
		}
		s.InfoHash, s.Name, s.Trackers = m.InfoHash, m.Name, m.Trackers
	default:
		return "", 0, errf(CodeInvalid, "a .torrent or a magnet is required")
	}
	ih := s.InfoHash
	if s.MetaInfo != nil {
		ih = s.MetaInfo.InfoHash
	}
	if _, exists := n.c.Torrent(ih); exists {
		return "", 0, errf(CodeState, "this torrent is already in the queue")
	}
	ts := torrentState{MetaInfo: raw, Dir: req.Dir, Paused: req.Paused}
	if raw == nil {
		ts.Magnet = req.Magnet // until metadata arrives
	}
	if err := n.st.putTorrent(ih.Hex(), ts); err != nil {
		return "", 0, err
	}
	t, err := n.c.Add(s)
	if err != nil {
		_ = n.st.deleteTorrent(ih.Hex())
		return "", 0, errf(CodeState, "%v", err)
	}
	var size int64
	if info := t.Info(); info != nil {
		size = info.Length
	}
	return ih.Hex(), size, nil
}

// save persists metainfo (synthesized from the info dict for magnets)
// and the current bitfield.
func (n *Native) save(t *torrent.Torrent) {
	n.mu.Lock()
	defer n.mu.Unlock()
	ih := t.InfoHash().Hex()
	ts, ok := n.st.torrent(ih)
	if !ok {
		return
	}
	if len(ts.MetaInfo) == 0 {
		if info := t.Info(); info != nil {
			raw, err := bencode.Encode(map[string]any{"info": bencode.RawMessage(info.Raw)})
			if err == nil {
				ts.MetaInfo = raw
			}
		}
	}
	ts.Have = t.Bitfield()
	_ = n.st.putTorrent(ih, ts)
}

func (n *Native) get(ih string) (*torrent.Torrent, error) {
	h, err := torrent.ParseInfoHash(ih)
	if err != nil {
		return nil, errf(CodeNotFound, "unknown torrent")
	}
	t, ok := n.c.Torrent(h)
	if !ok {
		return nil, errf(CodeNotFound, "unknown torrent")
	}
	return t, nil
}

// Status reports a torrent.
func (n *Native) Status(ih string) (ClientStatus, bool) {
	t, err := n.get(ih)
	if err != nil {
		return ClientStatus{}, false
	}
	s := t.Stats()
	return ClientStatus{
		State: string(s.State), Size: s.Size, Completed: s.Completed, Downloaded: s.Downloaded,
		Uploaded: s.Uploaded, Peers: s.Peers, DownRate: s.DownRate, UpRate: s.UpRate, Error: s.Error,
	}, true
}

// Files lists a torrent's files.
func (n *Native) Files(ih string) []File {
	t, err := n.get(ih)
	if err != nil {
		return nil
	}
	var out []File
	for _, f := range t.Files() {
		out = append(out, File{Path: f.Path, Rel: f.Rel, Length: f.Length})
	}
	return out
}

func (n *Native) setPaused(ih string, paused bool) {
	n.mu.Lock()
	defer n.mu.Unlock()
	if ts, ok := n.st.torrent(ih); ok {
		ts.Paused = paused
		_ = n.st.putTorrent(ih, ts)
	}
}

// Pause stops transfers and remembers it across restarts.
func (n *Native) Pause(ih string) error {
	t, err := n.get(ih)
	if err != nil {
		return err
	}
	t.Pause()
	n.save(t)
	n.setPaused(ih, true)
	return nil
}

// Resume restarts transfers.
func (n *Native) Resume(ih string) error {
	t, err := n.get(ih)
	if err != nil {
		return err
	}
	n.setPaused(ih, false)
	t.Resume()
	return nil
}

// Remove drops a torrent and optionally its data.
func (n *Native) Remove(ih string, deleteData bool) error {
	t, err := n.get(ih)
	if err == nil {
		if rerr := t.Remove(deleteData); rerr != nil {
			return errf(CodeImport, "could not delete torrent data: %v", rerr)
		}
	}
	return n.st.deleteTorrent(ih)
}

// SetRates changes the transfer caps live.
func (n *Native) SetRates(up, down int64) { n.c.SetRates(up, down) }

// Checkpoint saves every torrent's bitfield.
func (n *Native) Checkpoint() {
	for _, t := range n.c.Torrents() {
		n.save(t)
	}
}

// Close checkpoints and stops the engine.
func (n *Native) Close() {
	n.Checkpoint()
	n.c.Close()
}
