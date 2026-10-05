package torrent

import "testing"

func FuzzParseHTTPAnnounce(f *testing.F) {
	f.Add([]byte("d8:intervali60e5:peers6:\x7f\x00\x00\x01\x1b\x59e"))
	f.Add([]byte("d14:failure reason5:nopee"))
	f.Add([]byte("d5:peersld2:ip9:127.0.0.14:porti6881eeee"))
	f.Fuzz(func(t *testing.T, body []byte) {
		res, err := parseHTTPAnnounce(body)
		if err != nil {
			return
		}
		if len(res.Peers) > maxPeersPerAnn || res.Interval <= 0 {
			t.Fatalf("unbounded response: %d peers, interval %v", len(res.Peers), res.Interval)
		}
		for _, p := range res.Peers {
			if p.Port() == 0 {
				t.Fatal("zero port accepted")
			}
		}
	})
}
