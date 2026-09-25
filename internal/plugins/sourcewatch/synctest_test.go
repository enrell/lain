package sourcewatch

import (
	"testing"
	"testing/synctest"
	"time"

	"github.com/enrell/lain/internal/contracts"
)

// Deterministic debounce tests inside the synctest bubble: the clock is
// fake and only advances while every goroutine is durably blocked, so a
// time.Sleep in the root goroutine jumps straight to each deadline —
// instant, exact, no flakes. fsnotify itself stays outside the bubble
// (NoFS): the dirty/debounce bookkeeping is what needs the clock.

func bubbleProvider() *Provider {
	return &Provider{
		Debounce: 2 * time.Second,
		NoFS:     true,
		dirs:     map[string]string{},
		known:    map[string]string{},
		dirty:    map[string]time.Time{},
	}
}

func bubblePoll(t *testing.T, p *Provider, libs ...WatchLibrary) PollOutput {
	t.Helper()
	out, err := p.Invoke(contracts.CapSourceWatch, PollInput{Libraries: libs})
	if err != nil {
		t.Fatalf("poll: %v", err)
	}
	return out.(PollOutput)
}

func bubbleDirty(t *testing.T, p *Provider, libID string) {
	t.Helper()
	if _, err := p.Invoke(contracts.CapSourceWatch, DirtyInput{LibraryID: libID}); err != nil {
		t.Fatalf("dirty: %v", err)
	}
}

var bubbleLib = WatchLibrary{ID: "lib-1", Path: "/nonexistent"}

// A burst of dirties collapses into exactly one drain, reported only
// after a full quiet period — the whole point of the debounce.
func TestSynctestDebounceCollapsesBurst(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		p := bubbleProvider()
		bubblePoll(t, p, bubbleLib)
		for i := 0; i < 6; i++ {
			bubbleDirty(t, p, "lib-1")
			time.Sleep(300 * time.Millisecond) // inside the burst window
		}
		// At t=1.8s the last dirty (t=1.5s) is still inside its 2s
		// quiet window — nothing may have drained yet.
		if out := bubblePoll(t, p, bubbleLib); len(out.Dirty) != 0 {
			t.Fatalf("drained inside the debounce window: %v", out.Dirty)
		}
		time.Sleep(2 * time.Second) // t=3.8s: past the 3.5s deadline
		if out := bubblePoll(t, p, bubbleLib); len(out.Dirty) != 1 || out.Dirty[0] != "lib-1" {
			t.Fatalf("want [lib-1], got %v", out.Dirty)
		}
		// Drained once means it is gone.
		if out := bubblePoll(t, p, bubbleLib); len(out.Dirty) != 0 {
			t.Fatalf("burst produced a second drain: %v", out.Dirty)
		}
	})
}

// Two libraries debounce independently: a burst on one must not delay
// the other's drain.
func TestSynctestDebouncePerLibrary(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		p := bubbleProvider()
		libs := []WatchLibrary{{ID: "lib-1", Path: "/a"}, {ID: "lib-2", Path: "/b"}}
		bubblePoll(t, p, libs...)
		bubbleDirty(t, p, "lib-1")
		time.Sleep(time.Second) // t=1s: half lib-1's debounce
		bubbleDirty(t, p, "lib-2")
		bubbleDirty(t, p, "lib-2") // re-arm lib-2 at t=1s, drains at t=3s
		time.Sleep(2100 * time.Millisecond)
		// A poll drains every due library at once; each library must
		// appear exactly once across polls.
		got := map[string]int{}
		for i := 0; i < 10 && len(got) < 2; i++ {
			for _, id := range bubblePoll(t, p, libs...).Dirty {
				got[id]++
			}
		}
		if got["lib-1"] != 1 || got["lib-2"] != 1 {
			t.Fatalf("want one drain per library, got %v", got)
		}
	})
}

// A library dropped from the poll input cancels its pending drain and
// leaves the bookkeeping empty.
func TestSynctestDropLibraryCancels(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		p := bubbleProvider()
		bubblePoll(t, p, bubbleLib)
		bubbleDirty(t, p, "lib-1")
		time.Sleep(time.Second)
		// The library vanishes from the input — the pending dirty goes
		// with it, so nothing drains afterwards.
		if out := bubblePoll(t, p); len(out.Dirty) != 0 {
			t.Fatalf("drop should cancel pending drain: %v", out.Dirty)
		}
		time.Sleep(3 * time.Second)
		if out := bubblePoll(t, p); len(out.Dirty) != 0 {
			t.Fatalf("drained for dropped library: %v", out.Dirty)
		}
		if p.Watching("lib-1") {
			t.Fatal("dropped library still watched")
		}
	})
}

// A dirty marked after Close must be ignored — shutdown mid-burst
// drains nothing.
func TestSynctestClosedProviderIgnoresDirty(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		p := bubbleProvider()
		bubblePoll(t, p, bubbleLib)
		bubbleDirty(t, p, "lib-1")
		if err := p.Close(); err != nil {
			t.Fatal(err)
		}
		bubbleDirty(t, p, "lib-1")
		time.Sleep(3 * time.Second)
		if _, err := p.Invoke(contracts.CapSourceWatch, PollInput{Libraries: []WatchLibrary{bubbleLib}}); err == nil {
			t.Fatal("closed provider must fail poll")
		}
	})
}
