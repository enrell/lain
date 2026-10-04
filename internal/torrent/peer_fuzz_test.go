package torrent

import (
	"bufio"
	"net"
	"net/netip"
	"os"
	"sync"
	"testing"
	"time"
)

// FuzzPeerMessages feeds arbitrary bytes after a valid handshake to a
// running torrent: any input may drop the peer, none may panic or wedge.
func FuzzPeerMessages(f *testing.F) {
	data := make([]byte, 70000)
	_, mi, err := Build("x.bin", []SourceFile{{Data: data}}, 32<<10, nil)
	if err != nil {
		f.Fatal(err)
	}
	dir, err := os.MkdirTemp("", "lain-fuzz-peer")
	if err != nil {
		f.Fatal(err)
	}
	defer os.RemoveAll(dir)
	c, err := NewClient(Config{})
	if err != nil {
		f.Fatal(err)
	}
	defer c.Close()
	tor, err := c.Add(Spec{MetaInfo: mi, Dir: dir})
	if err != nil {
		f.Fatal(err)
	}
	for tor.Stats().State == StateChecking {
		time.Sleep(time.Millisecond)
	}
	f.Add([]byte{0, 0, 0, 5, msgBitfield, 0xe0})
	f.Add([]byte{0, 0, 0, 1, msgUnchoke, 0, 0, 0, 13, msgRequest, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x40, 0})
	f.Add([]byte{0, 0, 0, 3, msgExtended, 0, 'd', 0, 0, 0, 5, msgHave, 0xff, 0xff, 0xff, 0xff})
	f.Add([]byte{0, 0, 0, 20, msgExtended, 1, 'd', '8', ':', 'm', 's', 'g', '_', 't', 'y', 'p', 'e', 'i', '1', 'e', 'e', 'x', 'x', 'x'})
	var mu sync.Mutex
	n := 0
	f.Fuzz(func(t *testing.T, payload []byte) {
		mu.Lock()
		n++
		id := [20]byte{byte(n), byte(n >> 8), byte(n >> 16), 'f'}
		mu.Unlock()
		a, b := net.Pipe()
		done := make(chan struct{})
		go func() {
			defer close(done)
			// Drain whatever the engine sends so its writer never blocks.
			buf := make([]byte, 4096)
			for {
				if _, err := b.Read(buf); err != nil {
					return
				}
			}
		}()
		tor.attach(a, bufio.NewReader(a), handshake{InfoHash: mi.InfoHash, PeerID: id, Reserved: [8]byte{5: 0x10}}, netip.AddrPort{})
		_ = b.SetWriteDeadline(time.Now().Add(time.Second))
		_, _ = b.Write(payload)
		b.Close()
		<-done
	})
}
