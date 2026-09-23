// Library file watching (D-068). fsnotify watches every directory
// under each library root; events are debounced per library into a
// rescan through the existing ingest pipeline, so deletions, additions
// and D-019 move reconciliation all ride one tested path. inotify
// cannot see inside network mounts — those libraries keep reconciling
// through the manual scan, and `--watch=0`/`LAIN_WATCH=0` disables the
// watcher entirely.
package gateway

import (
	"io/fs"
	"os"
	"path/filepath"
	"sync"
	"time"

	"github.com/fsnotify/fsnotify"

	"github.com/enrell/lain/internal/contracts"
)

// watchDebounce collapses event bursts: a deleted season or a copied
// episode is never a single event, so the rescan waits for quiet.
const watchDebounceSeconds = 2

// libWatcher owns the fsnotify watcher and the per-library debounce
// timers. Watched dirs map to their library id; pending timers map a
// library id to its scheduled rescan.
type libWatcher struct {
	s        *Server
	fs       *fsnotify.Watcher
	debounce time.Duration
	// scan is the rescan entry point (s.scanLibrary in production); a
	// test seam so debounce/drop behaviour is assertable without walking
	// a real library.
	scan func(libID string)

	mu     sync.Mutex
	dirs   map[string]string      // watched dir -> library id
	timers map[string]*time.Timer // library id -> pending rescan
	closed bool
}

// StartWatcher begins filesystem reconciliation. Serve calls it unless
// disabled; tests opt in explicitly. A second call is a no-op.
func (s *Server) StartWatcher() error {
	s.scanMu.Lock()
	defer s.scanMu.Unlock()
	if s.watch != nil {
		return nil
	}
	fsw, err := fsnotify.NewWatcher()
	if err != nil {
		return err
	}
	w := &libWatcher{s: s, fs: fsw, debounce: watchDebounceSeconds * time.Second, scan: s.scanLibrary, dirs: map[string]string{}, timers: map[string]*time.Timer{}}
	if s.watchDebounce > 0 {
		w.debounce = s.watchDebounce
	}
	s.watch = w
	w.sync()
	go w.loop()
	return nil
}

// watcher returns the active watcher (nil before StartWatcher or after
// none was requested). The pointer is published under scanMu, so reads
// go through it too.
func (s *Server) watcher() *libWatcher {
	s.scanMu.Lock()
	defer s.scanMu.Unlock()
	return s.watch
}

// sync watches every current library tree and drops watches for
// libraries that are gone. Called at start and after library changes.
func (w *libWatcher) sync() {
	live := map[string]bool{}
	for _, lib := range w.s.libList() {
		live[lib.ID] = true
		w.watchTree(lib.Path, lib.ID)
	}
	w.mu.Lock()
	defer w.mu.Unlock()
	for dir, libID := range w.dirs {
		gone := !live[libID]
		if !gone {
			if fi, err := os.Stat(dir); err != nil || !fi.IsDir() {
				gone = true
			}
		}
		if gone {
			_ = w.fs.Remove(dir)
			delete(w.dirs, dir)
		}
	}
}

// watchTree adds a watch for root and every directory inside it.
// Unreadable trees stay unwatched: the scan's own unreadable-root
// reporting already names them for the operator.
func (w *libWatcher) watchTree(root, libID string) {
	fi, err := os.Stat(root)
	if err != nil || !fi.IsDir() {
		w.s.logger().Warn("library root not watchable", "library", libID, "root", root)
		return
	}
	_ = filepath.WalkDir(root, func(path string, d fs.DirEntry, err error) error {
		if err != nil || !d.IsDir() {
			return nil
		}
		w.add(path, libID)
		return nil
	})
}

func (w *libWatcher) add(dir, libID string) {
	w.mu.Lock()
	defer w.mu.Unlock()
	if w.closed {
		return
	}
	if _, ok := w.dirs[dir]; ok {
		return
	}
	if err := w.fs.Add(dir); err != nil {
		return
	}
	w.dirs[dir] = libID
}

// dropLibrary unwatches one library's dirs and cancels a pending
// rescan: the library record is gone, so nothing would consume it.
func (w *libWatcher) dropLibrary(libID string) {
	w.mu.Lock()
	defer w.mu.Unlock()
	for dir, id := range w.dirs {
		if id == libID {
			_ = w.fs.Remove(dir)
			delete(w.dirs, dir)
		}
	}
	if t, ok := w.timers[libID]; ok {
		t.Stop()
		delete(w.timers, libID)
	}
}

// libOf resolves the library owning a path by walking ancestors until a
// watched dir matches — events report children of watched dirs.
func (w *libWatcher) libOf(path string) string {
	w.mu.Lock()
	defer w.mu.Unlock()
	for p := path; ; p = filepath.Dir(p) {
		if libID, ok := w.dirs[p]; ok {
			return libID
		}
		if parent := filepath.Dir(p); parent == p {
			return ""
		}
	}
}

func (w *libWatcher) loop() {
	for {
		select {
		case ev, ok := <-w.fs.Events:
			if !ok {
				return
			}
			w.onEvent(ev)
		case err, ok := <-w.fs.Errors:
			if !ok {
				return
			}
			w.s.logger().Warn("library watcher error", "err", err.Error())
		}
	}
}

func (w *libWatcher) onEvent(ev fsnotify.Event) {
	libID := w.libOf(ev.Name)
	if libID == "" {
		return
	}
	// A created directory — including the top of a tree moved in — must
	// be watched before anything inside it can report.
	if ev.Op&fsnotify.Create != 0 {
		if fi, err := os.Stat(ev.Name); err == nil && fi.IsDir() {
			w.watchTree(ev.Name, libID)
		}
	}
	// fsnotify drops the watch of a removed/renamed dir itself; the map
	// forgets it so the stale path never mis-attributes a later event.
	if ev.Op&(fsnotify.Remove|fsnotify.Rename) != 0 {
		w.mu.Lock()
		delete(w.dirs, ev.Name)
		w.mu.Unlock()
	}
	w.nudge(libID)
}

// nudge (re)arms the library's debounce timer; the timer callback owns
// the rescan once the burst settles.
func (w *libWatcher) nudge(libID string) {
	w.mu.Lock()
	defer w.mu.Unlock()
	if w.closed {
		return
	}
	if t, ok := w.timers[libID]; ok {
		t.Stop()
	}
	w.timers[libID] = time.AfterFunc(w.debounce, func() {
		w.mu.Lock()
		delete(w.timers, libID)
		w.mu.Unlock()
		w.scan(libID)
	})
}

func (w *libWatcher) close() {
	w.mu.Lock()
	w.closed = true
	for _, t := range w.timers {
		t.Stop()
	}
	w.timers = map[string]*time.Timer{}
	w.mu.Unlock()
	_ = w.fs.Close()
}

// scanLibrary runs the ingest pipeline for one library under the same
// scan lock as manual scans. A scan already in flight re-arms the
// debounce instead of racing it.
func (s *Server) scanLibrary(id string) {
	libs := s.libList()
	var lib *contracts.Library
	for i := range libs {
		if libs[i].ID == id {
			lib = &libs[i]
			break
		}
	}
	if lib == nil {
		return // deleted between the event and the timer
	}
	s.scanMu.Lock()
	if s.scan.State == "running" {
		s.scanMu.Unlock()
		if w := s.watcher(); w != nil {
			w.nudge(id)
		}
		return
	}
	s.scan = ScanStatus{State: "running", StartedAt: time.Now().Unix(), Trigger: "watch"}
	s.scanMu.Unlock()
	go s.runScan([]contracts.Library{*lib}, "watch")
}
