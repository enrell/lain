package sourcewatch

import (
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/fsnotify/fsnotify"

	"github.com/enrell/lain/internal/contracts"
)

func poll(t *testing.T, p *Provider, libs ...WatchLibrary) PollOutput {
	t.Helper()
	out, err := p.Invoke(contracts.CapSourceWatch, PollInput{Libraries: libs})
	if err != nil {
		t.Fatalf("poll: %v", err)
	}
	res, ok := out.(PollOutput)
	if !ok {
		t.Fatalf("poll returned %T", out)
	}
	return res
}

func dirty(t *testing.T, p *Provider, libID string) {
	t.Helper()
	if _, err := p.Invoke(contracts.CapSourceWatch, DirtyInput{LibraryID: libID}); err != nil {
		t.Fatalf("dirty: %v", err)
	}
}

func newTestProvider(t *testing.T, debounce time.Duration) *Provider {
	t.Helper()
	p := NewWatcherProvider()
	p.Debounce = debounce
	t.Cleanup(func() { _ = p.Close() })
	return p
}

// A burst of dirties collapses into a single drain: every event resets
// the quiet window while the tree is still settling.
func TestDebounceCollapsesBurst(t *testing.T) {
	p := newTestProvider(t, 40*time.Millisecond)
	p.NoFS = true
	lib := WatchLibrary{ID: "lib-1", Path: t.TempDir()}
	poll(t, p, lib) // register the watch set
	for i := 0; i < 6; i++ {
		dirty(t, p, "lib-1")
		time.Sleep(5 * time.Millisecond)
	}
	deadline := time.Now().Add(2 * time.Second)
	for {
		if out := poll(t, p, lib); len(out.Dirty) == 1 && out.Dirty[0] == "lib-1" {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("dirty library never drained")
		}
		time.Sleep(10 * time.Millisecond)
	}
	// Drained once means it is gone: a second poll reports nothing.
	if out := poll(t, p, lib); len(out.Dirty) != 0 {
		t.Fatalf("burst produced a second drain: %v", out.Dirty)
	}
}

// A library dropped from the poll input cancels its pending rescan:
// nothing may drain afterwards.
func TestDropLibraryCancelsPending(t *testing.T) {
	p := newTestProvider(t, 40*time.Millisecond)
	p.NoFS = true
	lib := WatchLibrary{ID: "lib-1", Path: t.TempDir()}
	poll(t, p, lib)
	dirty(t, p, "lib-1")
	time.Sleep(3 * 40 * time.Millisecond)
	// The library vanished: the input no longer carries it.
	if out := poll(t, p); len(out.Dirty) != 0 {
		t.Fatalf("drop should cancel pending drain, got %v", out.Dirty)
	}
	if p.Watching("lib-1") {
		t.Fatal("dropped library still watched")
	}
}

// A dirty marked after Close is ignored; poll on a closed provider
// reports dependency-unavailable rather than serving stale state.
func TestClosedWatcherIgnoresDirty(t *testing.T) {
	p := newTestProvider(t, time.Millisecond)
	p.NoFS = true
	lib := WatchLibrary{ID: "lib-1", Path: t.TempDir()}
	poll(t, p, lib)
	if err := p.Close(); err != nil {
		t.Fatal(err)
	}
	dirty(t, p, "lib-1")
	if _, err := p.Invoke(contracts.CapSourceWatch, PollInput{Libraries: []WatchLibrary{lib}}); err == nil {
		t.Fatal("closed provider must fail poll")
	} else if ce, ok := err.(interface{ Error() string }); !ok || ce.Error() == "" {
		t.Fatalf("closed poll: %v", err)
	}
	if err := p.Health(); err == nil {
		t.Fatal("closed provider must report unhealthy")
	}
}

// A directory create under a watched parent registers the new dir; a
// remove event drops it again so a recycled path cannot mis-attribute
// later events to the library.
func TestEventDirTracking(t *testing.T) {
	p := newTestProvider(t, time.Hour)
	defer p.Close()
	// The tracking half of the event loop needs a real fs watcher:
	// add() registers dirs through it.
	fsw, err := fsnotify.NewWatcher()
	if err != nil {
		t.Fatal(err)
	}
	defer fsw.Close()
	p.fs = fsw
	parent := t.TempDir()
	dir := filepath.Join(parent, "newdir")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	p.mu.Lock()
	p.dirs[parent] = "lib-1"
	p.mu.Unlock()
	p.onEvent(fsnotify.Event{Name: dir, Op: fsnotify.Create})
	p.mu.Lock()
	got := p.dirs[dir]
	p.mu.Unlock()
	if got != "lib-1" {
		t.Fatalf("created dir not tracked: dirs[%q]=%q", dir, got)
	}
	p.onEvent(fsnotify.Event{Name: dir, Op: fsnotify.Remove})
	p.mu.Lock()
	_, ok := p.dirs[dir]
	p.mu.Unlock()
	if ok {
		t.Fatal("removed dir still tracked")
	}
}

// A dirty library whose file events arrive through the real fsnotify
// path drains through poll — the fs watcher is not decoration.
func TestFSEventMarksDirty(t *testing.T) {
	p := newTestProvider(t, 20*time.Millisecond)
	defer p.Close()
	root := t.TempDir()
	lib := WatchLibrary{ID: "lib-1", Path: root}
	poll(t, p, lib) // starts fsnotify + watches the tree
	if err := os.WriteFile(filepath.Join(root, "new.mkv"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	deadline := time.Now().Add(5 * time.Second)
	for {
		if out := poll(t, p, lib); len(out.Dirty) == 1 && out.Dirty[0] == "lib-1" {
			return
		}
		if time.Now().After(deadline) {
			t.Fatal("filesystem event never marked the library dirty")
		}
		time.Sleep(20 * time.Millisecond)
	}
}
