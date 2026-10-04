// Package trackertest is a minimal in-process BitTorrent tracker (HTTP
// and UDP) for tests. Lain's tests never contact a real tracker
// (docs/slices/acquisition.md, A-13); swarms run on loopback against
// this one.
package trackertest

import (
	"encoding/binary"
	"net"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"strconv"
	"sync"

	"github.com/enrell/lain/internal/torrent"
	"github.com/enrell/lain/internal/torrent/bencode"
)

type peer struct {
	addr netip.AddrPort
	left int64
}

// Tracker keeps one swarm per info hash.
type Tracker struct {
	mu     sync.Mutex
	swarms map[torrent.InfoHash]map[[20]byte]peer
	// Announces counts every announce, by event.
	Announces map[string]int
	http      *httptest.Server
	udp       net.PacketConn
	// Refuse makes every announce fail with this reason.
	Refuse string
}

// New starts HTTP and UDP listeners on loopback.
func New() (*Tracker, error) {
	t := &Tracker{swarms: map[torrent.InfoHash]map[[20]byte]peer{}, Announces: map[string]int{}}
	t.http = httptest.NewServer(http.HandlerFunc(t.serveHTTP))
	pc, err := net.ListenPacket("udp", "127.0.0.1:0")
	if err != nil {
		t.http.Close()
		return nil, err
	}
	t.udp = pc
	go t.serveUDP()
	return t, nil
}

// HTTPURL is the announce URL of the HTTP tracker.
func (t *Tracker) HTTPURL() string { return t.http.URL + "/announce" }

// UDPURL is the announce URL of the UDP tracker.
func (t *Tracker) UDPURL() string { return "udp://" + t.udp.LocalAddr().String() }

// Close stops both listeners.
func (t *Tracker) Close() {
	t.http.Close()
	_ = t.udp.Close()
}

// Count is the number of announces seen with the event.
func (t *Tracker) Count(event string) int {
	t.mu.Lock()
	defer t.mu.Unlock()
	return t.Announces[event]
}

func (t *Tracker) announce(ih torrent.InfoHash, id [20]byte, addr netip.AddrPort, left int64, event string) (peers []netip.AddrPort, seeders, leechers int) {
	t.mu.Lock()
	defer t.mu.Unlock()
	t.Announces[event]++
	sw := t.swarms[ih]
	if sw == nil {
		sw = map[[20]byte]peer{}
		t.swarms[ih] = sw
	}
	if event == torrent.EventStopped {
		delete(sw, id)
	} else {
		sw[id] = peer{addr: addr, left: left}
	}
	for pid, p := range sw {
		if p.left == 0 {
			seeders++
		} else {
			leechers++
		}
		if pid != id {
			peers = append(peers, p.addr)
		}
	}
	return peers, seeders, leechers
}

func (t *Tracker) serveHTTP(w http.ResponseWriter, r *http.Request) {
	q := r.URL.Query()
	fail := func(reason string) {
		b, _ := bencode.Encode(map[string]any{"failure reason": reason})
		_, _ = w.Write(b)
	}
	if t.Refuse != "" {
		fail(t.Refuse)
		return
	}
	var ih torrent.InfoHash
	var id [20]byte
	if len(q.Get("info_hash")) != 20 || len(q.Get("peer_id")) != 20 {
		fail("bad info_hash or peer_id")
		return
	}
	copy(ih[:], q.Get("info_hash"))
	copy(id[:], q.Get("peer_id"))
	port, _ := strconv.Atoi(q.Get("port"))
	left, _ := strconv.ParseInt(q.Get("left"), 10, 64)
	host, _, _ := net.SplitHostPort(r.RemoteAddr)
	ip, err := netip.ParseAddr(host)
	if err != nil || port <= 0 || port > 65535 {
		fail("bad address")
		return
	}
	peers, seeders, leechers := t.announce(ih, id, netip.AddrPortFrom(ip.Unmap(), uint16(port)), left, q.Get("event"))
	b, _ := bencode.Encode(map[string]any{
		"interval": 5, "complete": seeders, "incomplete": leechers,
		"peers": string(torrent.CompactPeers(peers)),
	})
	_, _ = w.Write(b)
}

func (t *Tracker) serveUDP() {
	buf := make([]byte, 2048)
	conns := map[uint64]bool{}
	for {
		n, from, err := t.udp.ReadFrom(buf)
		if err != nil {
			return
		}
		if n < 16 {
			continue
		}
		action := binary.BigEndian.Uint32(buf[8:])
		tid := binary.BigEndian.Uint32(buf[12:])
		reply := func(b []byte) { _, _ = t.udp.WriteTo(b, from) }
		if t.Refuse != "" {
			out := binary.BigEndian.AppendUint32(nil, 3)
			out = binary.BigEndian.AppendUint32(out, tid)
			reply(append(out, t.Refuse...))
			continue
		}
		switch action {
		case 0:
			if binary.BigEndian.Uint64(buf) != 0x41727101980 {
				continue
			}
			cid := uint64(len(conns)+1) * 7919
			conns[cid] = true
			out := binary.BigEndian.AppendUint32(nil, 0)
			out = binary.BigEndian.AppendUint32(out, tid)
			reply(binary.BigEndian.AppendUint64(out, cid))
		case 1:
			if n < 98 || !conns[binary.BigEndian.Uint64(buf)] {
				continue
			}
			var ih torrent.InfoHash
			var id [20]byte
			copy(ih[:], buf[16:36])
			copy(id[:], buf[36:56])
			left := int64(binary.BigEndian.Uint64(buf[64:]))
			event := map[uint32]string{0: "", 1: torrent.EventCompleted, 2: torrent.EventStarted, 3: torrent.EventStopped}[binary.BigEndian.Uint32(buf[80:])]
			port := binary.BigEndian.Uint16(buf[96:])
			fromAddr, _ := netip.ParseAddrPort(from.String())
			peers, seeders, leechers := t.announce(ih, id, netip.AddrPortFrom(fromAddr.Addr().Unmap(), port), left, event)
			out := binary.BigEndian.AppendUint32(nil, 1)
			out = binary.BigEndian.AppendUint32(out, tid)
			out = binary.BigEndian.AppendUint32(out, 5)
			out = binary.BigEndian.AppendUint32(out, uint32(leechers))
			out = binary.BigEndian.AppendUint32(out, uint32(seeders))
			reply(append(out, torrent.CompactPeers(peers)...))
		}
	}
}
