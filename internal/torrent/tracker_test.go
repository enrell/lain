package torrent_test

import (
	"context"
	"errors"
	"net/netip"
	"testing"
	"time"

	"github.com/enrell/lain/internal/torrent"
	"github.com/enrell/lain/internal/torrent/trackertest"
)

func TestAnnounceHTTPAndUDP(t *testing.T) {
	tr, err := trackertest.New()
	if err != nil {
		t.Fatal(err)
	}
	defer tr.Close()
	ih := torrent.InfoHash{1, 2, 3}
	for _, url := range []string{tr.HTTPURL(), tr.UDPURL()} {
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		a := torrent.AnnounceRequest{InfoHash: ih, PeerID: [20]byte{'a', byte(len(url))}, Port: 7001, Left: 0, Event: torrent.EventStarted}
		if _, err := torrent.Announce(ctx, nil, url, a); err != nil {
			t.Fatalf("%s seeder: %v", url, err)
		}
		b := torrent.AnnounceRequest{InfoHash: ih, PeerID: [20]byte{'b', byte(len(url))}, Port: 7002, Left: 100, Event: torrent.EventStarted}
		res, err := torrent.Announce(ctx, nil, url, b)
		cancel()
		if err != nil {
			t.Fatalf("%s leecher: %v", url, err)
		}
		found := false
		for _, p := range res.Peers {
			if p == netip.MustParseAddrPort("127.0.0.1:7001") {
				found = true
			}
		}
		if !found || res.Seeders < 1 || res.Interval != 5*time.Second {
			t.Fatalf("%s response: %+v", url, res)
		}
	}
	if tr.Count(torrent.EventStarted) != 4 {
		t.Fatalf("started announces = %d", tr.Count(torrent.EventStarted))
	}
}

func TestAnnounceRefusalAndBadSchemes(t *testing.T) {
	tr, err := trackertest.New()
	if err != nil {
		t.Fatal(err)
	}
	defer tr.Close()
	tr.Refuse("unregistered torrent")
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	for _, url := range []string{tr.HTTPURL(), tr.UDPURL(), "ftp://x/announce"} {
		if _, err := torrent.Announce(ctx, nil, url, torrent.AnnounceRequest{}); !errors.Is(err, torrent.ErrTracker) {
			t.Errorf("%s: %v", url, err)
		}
	}
}

func TestUDPAnnounceTimesOut(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 300*time.Millisecond)
	defer cancel()
	// A loopback port with nothing listening: reads time out or fail.
	if _, err := torrent.Announce(ctx, nil, "udp://127.0.0.1:9", torrent.AnnounceRequest{}); !errors.Is(err, torrent.ErrTracker) {
		t.Fatalf("expected a tracker error, got %v", err)
	}
}
