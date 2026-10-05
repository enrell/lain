package torrent

import (
	"bufio"
	"context"
	"crypto/rand"
	"errors"
	"io"
	"log/slog"
	"net"
	"net/http"
	"net/netip"
	"sync"
	"time"
)

// Config tunes the engine. Every bound is a setting (A-5); zero values
// take the defaults below.
type Config struct {
	// ListenAddr accepts inbound peers ("" disables inbound, tests use
	// "127.0.0.1:0").
	ListenAddr string
	// PeerID is our identity; zero picks a random "-LN0001-" id.
	PeerID             [20]byte
	MaxPeersPerTorrent int
	UploadSlots        int
	// UploadRate and DownloadRate are bytes per second, 0 = unlimited.
	UploadRate   int64
	DownloadRate int64
	HTTPClient   *http.Client
	Logger       *slog.Logger
	DialTimeout  time.Duration
	// MinAnnounceInterval floors the tracker's interval (default 60 s).
	MinAnnounceInterval time.Duration
	// RequestTimeout returns a block request to the pool (default 45 s).
	RequestTimeout time.Duration
}

func (c Config) withDefaults() Config {
	if c.MaxPeersPerTorrent <= 0 {
		c.MaxPeersPerTorrent = 40
	}
	if c.UploadSlots <= 0 {
		c.UploadSlots = 4
	}
	if c.HTTPClient == nil {
		c.HTTPClient = &http.Client{Timeout: 30 * time.Second}
	}
	if c.Logger == nil {
		c.Logger = slog.New(slog.NewTextHandler(io.Discard, nil))
	}
	if c.DialTimeout <= 0 {
		c.DialTimeout = 10 * time.Second
	}
	if c.MinAnnounceInterval <= 0 {
		c.MinAnnounceInterval = 60 * time.Second
	}
	if c.RequestTimeout <= 0 {
		c.RequestTimeout = 45 * time.Second
	}
	if c.PeerID == ([20]byte{}) {
		copy(c.PeerID[:], "-LN0001-")
		_, _ = rand.Read(c.PeerID[8:])
	}
	return c
}

// Client runs torrents and the inbound listener.
type Client struct {
	cfg      Config
	ln       net.Listener
	up, down *Limiter

	mu       sync.Mutex
	torrents map[InfoHash]*Torrent
	closed   bool

	ctx    context.Context
	cancel context.CancelFunc
	wg     sync.WaitGroup
}

// NewClient starts the engine.
func NewClient(cfg Config) (*Client, error) {
	cfg = cfg.withDefaults()
	ctx, cancel := context.WithCancel(context.Background())
	c := &Client{
		cfg: cfg, torrents: map[InfoHash]*Torrent{},
		up: NewLimiter(cfg.UploadRate), down: NewLimiter(cfg.DownloadRate),
		ctx: ctx, cancel: cancel,
	}
	if cfg.ListenAddr != "" {
		ln, err := net.Listen("tcp", cfg.ListenAddr)
		if err != nil {
			cancel()
			return nil, err
		}
		c.ln = ln
		c.wg.Add(1)
		go c.acceptLoop()
	}
	return c, nil
}

// Port is the inbound TCP port (0 without a listener).
func (c *Client) Port() uint16 {
	if c.ln == nil {
		return 0
	}
	ap, err := netip.ParseAddrPort(c.ln.Addr().String())
	if err != nil {
		return 0
	}
	return ap.Port()
}

// SetRates changes the transfer caps (bytes/second, 0 = unlimited).
func (c *Client) SetRates(upload, download int64) {
	c.up.SetRate(upload)
	c.down.SetRate(download)
}

// Torrent returns a running torrent by info hash.
func (c *Client) Torrent(ih InfoHash) (*Torrent, bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	t, ok := c.torrents[ih]
	return t, ok
}

// Torrents lists every torrent.
func (c *Client) Torrents() []*Torrent {
	c.mu.Lock()
	defer c.mu.Unlock()
	out := make([]*Torrent, 0, len(c.torrents))
	for _, t := range c.torrents {
		out = append(out, t)
	}
	return out
}

// Close stops every torrent and the listener.
func (c *Client) Close() {
	c.mu.Lock()
	if c.closed {
		c.mu.Unlock()
		return
	}
	c.closed = true
	ts := make([]*Torrent, 0, len(c.torrents))
	for _, t := range c.torrents {
		ts = append(ts, t)
	}
	c.mu.Unlock()
	for _, t := range ts {
		t.stop(false)
	}
	c.cancel()
	if c.ln != nil {
		_ = c.ln.Close()
	}
	c.wg.Wait()
}

// ErrExists is returned when a torrent is added twice.
var ErrExists = errors.New("torrent: already added")

// Add registers and (unless spec.Paused) starts a torrent.
func (c *Client) Add(spec Spec) (*Torrent, error) {
	t, err := newTorrent(c, spec)
	if err != nil {
		return nil, err
	}
	c.mu.Lock()
	if c.closed {
		c.mu.Unlock()
		return nil, errors.New("torrent: client closed")
	}
	if _, dup := c.torrents[t.ih]; dup {
		c.mu.Unlock()
		return nil, ErrExists
	}
	c.torrents[t.ih] = t
	c.mu.Unlock()
	if !spec.Paused {
		t.Resume()
	}
	return t, nil
}

func (c *Client) forget(t *Torrent) {
	c.mu.Lock()
	if c.torrents[t.ih] == t {
		delete(c.torrents, t.ih)
	}
	c.mu.Unlock()
}

func (c *Client) acceptLoop() {
	defer c.wg.Done()
	for {
		conn, err := c.ln.Accept()
		if err != nil {
			if c.ctx.Err() != nil {
				return
			}
			c.cfg.Logger.Warn("torrent accept failed", "err", err.Error())
			time.Sleep(100 * time.Millisecond)
			continue
		}
		go c.handleInbound(conn)
	}
}

func (c *Client) handleInbound(conn net.Conn) {
	_ = conn.SetDeadline(time.Now().Add(c.cfg.DialTimeout))
	r := bufio.NewReaderSize(conn, 64<<10)
	hs, err := readHandshake(r)
	if err != nil {
		conn.Close()
		return
	}
	t, ok := c.Torrent(hs.InfoHash)
	if !ok || hs.PeerID == c.cfg.PeerID {
		conn.Close()
		return
	}
	if err := writeHandshake(conn, hs.InfoHash, c.cfg.PeerID); err != nil {
		conn.Close()
		return
	}
	_ = conn.SetDeadline(time.Time{})
	addr, _ := netip.ParseAddrPort(conn.RemoteAddr().String())
	t.attach(conn, r, hs, addr)
}
