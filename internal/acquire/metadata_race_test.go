package acquire

import (
	"testing"
	"time"
)

// earlyClient reports a magnet's metadata while Add is still running,
// before the grab is stored: a fast local swarm does this on CI.
type earlyClient struct {
	DownloadClient
	m *Manager
}

const earlyHash = "a1134d724672bffc94d1c361edb4a04ff3fdf61a"

func (c earlyClient) Add(AddRequest) (string, int64, error) {
	done := make(chan struct{})
	go func() {
		c.m.onMetadata(earlyHash, 1<<30)
		close(done)
	}()
	// Unfixed, the event finds no grab and returns at once. Fixed, it
	// waits for Grab to store the record; don't wait for it here.
	select {
	case <-done:
	case <-time.After(200 * time.Millisecond):
	}
	return earlyHash, 0, nil
}

func (earlyClient) Status(string) (ClientStatus, bool) { return ClientStatus{}, false }
func (earlyClient) Remove(string, bool) error          { return nil }

func TestMetadataBeforeTheGrabIsStoredStillChecksTheBudget(t *testing.T) {
	w := newWorld(t)
	w.limits.MaxBytes = 1000
	m := w.manager(t, t.TempDir(), settings(0))
	m.mu.Lock()
	m.client = earlyClient{DownloadClient: m.client, m: m}
	m.mu.Unlock()

	g, err := m.Grab(GrabInput{Magnet: "magnet:?xt=urn:btih:" + earlyHash + "&tr=http://127.0.0.1:1/announce", LibraryID: w.lib.ID})
	if err != nil {
		t.Fatal(err)
	}
	if got := waitGrabFailed(t, m, g.ID); got.Code != CodeQuota {
		t.Fatalf("a 1 GiB magnet must fail the 1000-byte budget: %+v", got)
	}
}
