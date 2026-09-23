package gateway

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-23

import (
	"testing"
	"testing/synctest"
	"time"
)

// Deterministic debounce tests inside the synctest bubble: the clock is
// fake and only advances while every goroutine is durably blocked, so a
// time.Sleep in the root goroutine jumps straight to each deadline —
// instant, exact, no flakes. fsnotify itself stays outside the bubble
// (real fd I/O); the timer/state-machine half is what needs the
// deterministic clock.

func newBubbleWatcher(scans chan string) *libWatcher {
	return &libWatcher{
		debounce: 2 * time.Second,
		scan:     func(id string) { scans <- id },
		dirs:     map[string]string{},
		timers:   map[string]*time.Timer{},
	}
}

// A burst of nudges collapses into exactly one rescan, fired only after
// a full quiet period — the whole point of the debounce.
func TestSynctestDebounceCollapsesBurst(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		scans := make(chan string, 8)
		w := newBubbleWatcher(scans)
		for i := 0; i < 6; i++ {
			w.nudge("lib-1")
			time.Sleep(300 * time.Millisecond) // inside the burst window
		}
		// At t=1.8s the last nudge (t=1.5s) is still inside its 2s
		// quiet window — nothing may have fired yet.
		select {
		case id := <-scans:
			t.Fatalf("scan fired inside the debounce window for %s", id)
		default:
		}
		time.Sleep(2 * time.Second) // t=3.8s: past the 3.5s deadline
		synctest.Wait()
		select {
		case id := <-scans:
			if id != "lib-1" {
				t.Fatalf("scan fired for %s, want lib-1", id)
			}
		default:
			t.Fatal("debounced scan never fired")
		}
		select {
		case id := <-scans:
			t.Fatalf("burst produced a second scan for %s", id)
		default:
		}
	})
}

// Two libraries debounce independently: a burst on one must not delay
// or cancel the other's pending rescan.
func TestSynctestDebouncePerLibrary(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		scans := make(chan string, 8)
		w := newBubbleWatcher(scans)
		w.nudge("lib-1")
		time.Sleep(time.Second) // t=1s: half lib-1's debounce
		w.nudge("lib-2")
		w.nudge("lib-2") // re-arm lib-2 at t=1s, fires at t=3s
		time.Sleep(2100 * time.Millisecond)
		synctest.Wait()
		got := map[string]int{}
		for len(scans) > 0 {
			got[<-scans]++
		}
		if got["lib-1"] != 1 || got["lib-2"] != 1 {
			t.Fatalf("want one scan per library, got %v", got)
		}
	})
}

// dropLibrary cancels a pending rescan: nothing may fire afterwards,
// and the timer bookkeeping must be empty.
func TestSynctestDropLibraryCancelsTimer(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		scans := make(chan string, 8)
		w := newBubbleWatcher(scans)
		w.nudge("lib-1")
		time.Sleep(time.Second)
		w.dropLibrary("lib-1")
		time.Sleep(2 * time.Second) // well past the deadline
		synctest.Wait()
		select {
		case id := <-scans:
			t.Fatalf("scan fired for dropped library %s", id)
		default:
		}
		w.mu.Lock()
		defer w.mu.Unlock()
		if len(w.timers) != 0 {
			t.Fatalf("pending timer survived drop: %v", w.timers)
		}
	})
}

// A nudge landing after close must not arm a new timer — shutdown
// mid-burst fires nothing.
func TestSynctestClosedWatcherIgnoresNudge(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		scans := make(chan string, 8)
		w := newBubbleWatcher(scans)
		w.nudge("lib-1")
		w.closed = true // close() also nils the fs watcher; the flag is what gates timers
		w.mu.Lock()
		for _, tm := range w.timers {
			tm.Stop()
		}
		w.timers = map[string]*time.Timer{}
		w.mu.Unlock()
		w.nudge("lib-2") // must be ignored: closed
		time.Sleep(3 * time.Second)
		synctest.Wait()
		select {
		case id := <-scans:
			t.Fatalf("scan fired after close for %s", id)
		default:
		}
	})
}
