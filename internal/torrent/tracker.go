package torrent

import (
	"context"
	"crypto/rand"
	"encoding/binary"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/netip"
	"net/url"
	"strconv"
	"time"

	"github.com/enrell/lain/internal/torrent/bencode"
)

// Announce events.
const (
	EventNone      = ""
	EventStarted   = "started"
	EventCompleted = "completed"
	EventStopped   = "stopped"
)

const (
	maxTrackerBody = 1 << 20
	maxPeersPerAnn = 2000
	udpProtocolID  = 0x41727101980
)

// AnnounceRequest is what a client tells a tracker.
type AnnounceRequest struct {
	InfoHash   InfoHash
	PeerID     [20]byte
	Port       uint16
	Uploaded   int64
	Downloaded int64
	Left       int64
	Event      string
	NumWant    int
}

// AnnounceResponse is the tracker's answer.
type AnnounceResponse struct {
	Interval time.Duration
	Peers    []netip.AddrPort
	Seeders  int
	Leechers int
}

// ErrTracker wraps tracker failures (including "failure reason").
var ErrTracker = errors.New("torrent: tracker")

func trackerErr(format string, args ...any) error {
	return fmt.Errorf("%w: %s", ErrTracker, fmt.Sprintf(format, args...))
}

// Announce contacts one tracker (http, https or udp).
func Announce(ctx context.Context, hc *http.Client, tracker string, req AnnounceRequest) (*AnnounceResponse, error) {
	u, err := url.Parse(tracker)
	if err != nil {
		return nil, trackerErr("bad url")
	}
	switch u.Scheme {
	case "http", "https":
		return announceHTTP(ctx, hc, u, req)
	case "udp":
		return announceUDP(ctx, u.Host, req)
	default:
		return nil, trackerErr("unsupported scheme %q", u.Scheme)
	}
}

func announceHTTP(ctx context.Context, hc *http.Client, u *url.URL, req AnnounceRequest) (*AnnounceResponse, error) {
	if hc == nil {
		hc = http.DefaultClient
	}
	q := u.Query()
	// info_hash and peer_id are raw bytes; url.Values escapes them.
	q.Set("info_hash", string(req.InfoHash[:]))
	q.Set("peer_id", string(req.PeerID[:]))
	q.Set("port", strconv.Itoa(int(req.Port)))
	q.Set("uploaded", strconv.FormatInt(req.Uploaded, 10))
	q.Set("downloaded", strconv.FormatInt(req.Downloaded, 10))
	q.Set("left", strconv.FormatInt(req.Left, 10))
	q.Set("compact", "1")
	if req.NumWant > 0 {
		q.Set("numwant", strconv.Itoa(req.NumWant))
	}
	if req.Event != "" {
		q.Set("event", req.Event)
	}
	nu := *u
	nu.RawQuery = q.Encode()
	hreq, err := http.NewRequestWithContext(ctx, http.MethodGet, nu.String(), nil)
	if err != nil {
		return nil, trackerErr("request: %v", err)
	}
	res, err := hc.Do(hreq)
	if err != nil {
		return nil, trackerErr("%v", err)
	}
	defer res.Body.Close()
	if res.StatusCode != http.StatusOK {
		return nil, trackerErr("http %d", res.StatusCode)
	}
	body, err := io.ReadAll(io.LimitReader(res.Body, maxTrackerBody+1))
	if err != nil || len(body) > maxTrackerBody {
		return nil, trackerErr("bad body")
	}
	return parseHTTPAnnounce(body)
}

func parseHTTPAnnounce(body []byte) (*AnnounceResponse, error) {
	v, err := bencode.Decode(body)
	if err != nil {
		return nil, trackerErr("%v", err)
	}
	d, ok := v.(map[string]any)
	if !ok {
		return nil, trackerErr("response is not a dictionary")
	}
	if reason, ok := bencode.Str(d, "failure reason"); ok {
		if len(reason) > 200 {
			reason = reason[:200]
		}
		return nil, trackerErr("refused: %s", reason)
	}
	out := &AnnounceResponse{Interval: 30 * time.Minute}
	if n, ok := bencode.Int(d, "interval"); ok && n > 0 && n < 86400 {
		out.Interval = time.Duration(n) * time.Second
	}
	if n, ok := bencode.Int(d, "complete"); ok {
		out.Seeders = int(n)
	}
	if n, ok := bencode.Int(d, "incomplete"); ok {
		out.Leechers = int(n)
	}
	switch p := d["peers"].(type) {
	case string:
		out.Peers = compactPeers([]byte(p), 4)
	case []any:
		for _, e := range p {
			pd, ok := e.(map[string]any)
			if !ok {
				continue
			}
			ip, _ := bencode.Str(pd, "ip")
			port, _ := bencode.Int(pd, "port")
			addr, err := netip.ParseAddr(ip)
			if err != nil || port <= 0 || port > 65535 {
				continue
			}
			out.Peers = append(out.Peers, netip.AddrPortFrom(addr, uint16(port)))
		}
	}
	if p6, ok := bencode.Str(d, "peers6"); ok {
		out.Peers = append(out.Peers, compactPeers([]byte(p6), 16)...)
	}
	if len(out.Peers) > maxPeersPerAnn {
		out.Peers = out.Peers[:maxPeersPerAnn]
	}
	return out, nil
}

// compactPeers decodes BEP 23 (ipLen 4) and BEP 7 (ipLen 16) lists.
func compactPeers(b []byte, ipLen int) []netip.AddrPort {
	step := ipLen + 2
	var out []netip.AddrPort
	for i := 0; i+step <= len(b); i += step {
		addr, ok := netip.AddrFromSlice(b[i : i+ipLen])
		port := binary.BigEndian.Uint16(b[i+ipLen:])
		if !ok || port == 0 {
			continue
		}
		out = append(out, netip.AddrPortFrom(addr.Unmap(), port))
	}
	return out
}

// CompactPeers encodes IPv4 peers (trackers in tests use it).
func CompactPeers(peers []netip.AddrPort) []byte {
	var b []byte
	for _, p := range peers {
		if !p.Addr().Unmap().Is4() {
			continue
		}
		a := p.Addr().Unmap().As4()
		b = append(b, a[:]...)
		b = binary.BigEndian.AppendUint16(b, p.Port())
	}
	return b
}

var udpEvents = map[string]uint32{EventNone: 0, EventCompleted: 1, EventStarted: 2, EventStopped: 3}

// announceUDP speaks BEP 15: connect, then announce, with a bounded
// retransmission schedule inside the caller's deadline.
func announceUDP(ctx context.Context, host string, req AnnounceRequest) (*AnnounceResponse, error) {
	var d net.Dialer
	conn, err := d.DialContext(ctx, "udp", host)
	if err != nil {
		return nil, trackerErr("%v", err)
	}
	defer conn.Close()
	exchange := func(pkt []byte, action uint32, tid uint32, minLen int) ([]byte, error) {
		buf := make([]byte, 16+6*maxPeersPerAnn)
		for attempt := 0; attempt < 3; attempt++ {
			deadline := time.Now().Add(time.Duration(2<<attempt) * time.Second)
			if dl, ok := ctx.Deadline(); ok && dl.Before(deadline) {
				deadline = dl
			}
			_ = conn.SetDeadline(deadline)
			if _, err := conn.Write(pkt); err != nil {
				return nil, trackerErr("%v", err)
			}
			for {
				n, err := conn.Read(buf)
				if err != nil {
					if ne, ok := err.(net.Error); ok && ne.Timeout() {
						break
					}
					return nil, trackerErr("%v", err)
				}
				if n < 8 || binary.BigEndian.Uint32(buf[4:]) != tid {
					continue // stale or foreign datagram
				}
				act := binary.BigEndian.Uint32(buf)
				if act == 3 {
					msg := string(buf[8:n])
					if len(msg) > 200 {
						msg = msg[:200]
					}
					return nil, trackerErr("refused: %s", msg)
				}
				if act != action || n < minLen {
					return nil, trackerErr("bad response")
				}
				return append([]byte(nil), buf[:n]...), nil
			}
			if ctx.Err() != nil {
				return nil, trackerErr("%v", ctx.Err())
			}
		}
		return nil, trackerErr("timeout")
	}

	tid := randUint32()
	pkt := binary.BigEndian.AppendUint64(nil, udpProtocolID)
	pkt = binary.BigEndian.AppendUint32(pkt, 0)
	pkt = binary.BigEndian.AppendUint32(pkt, tid)
	resp, err := exchange(pkt, 0, tid, 16)
	if err != nil {
		return nil, err
	}
	connID := binary.BigEndian.Uint64(resp[8:])

	tid = randUint32()
	numWant := int32(-1)
	if req.NumWant > 0 {
		numWant = int32(req.NumWant)
	}
	pkt = binary.BigEndian.AppendUint64(nil, connID)
	pkt = binary.BigEndian.AppendUint32(pkt, 1)
	pkt = binary.BigEndian.AppendUint32(pkt, tid)
	pkt = append(pkt, req.InfoHash[:]...)
	pkt = append(pkt, req.PeerID[:]...)
	pkt = binary.BigEndian.AppendUint64(pkt, uint64(req.Downloaded))
	pkt = binary.BigEndian.AppendUint64(pkt, uint64(req.Left))
	pkt = binary.BigEndian.AppendUint64(pkt, uint64(req.Uploaded))
	pkt = binary.BigEndian.AppendUint32(pkt, udpEvents[req.Event])
	pkt = binary.BigEndian.AppendUint32(pkt, 0) // IP: default
	pkt = binary.BigEndian.AppendUint32(pkt, randUint32())
	pkt = binary.BigEndian.AppendUint32(pkt, uint32(numWant))
	pkt = binary.BigEndian.AppendUint16(pkt, req.Port)
	resp, err = exchange(pkt, 1, tid, 20)
	if err != nil {
		return nil, err
	}
	out := &AnnounceResponse{
		Interval: time.Duration(binary.BigEndian.Uint32(resp[8:])) * time.Second,
		Leechers: int(binary.BigEndian.Uint32(resp[12:])),
		Seeders:  int(binary.BigEndian.Uint32(resp[16:])),
		Peers:    compactPeers(resp[20:], 4),
	}
	if out.Interval <= 0 || out.Interval > 24*time.Hour {
		out.Interval = 30 * time.Minute
	}
	return out, nil
}

func randUint32() uint32 {
	var b [4]byte
	_, _ = rand.Read(b[:])
	return binary.BigEndian.Uint32(b[:])
}
