// Package sourcewatch serves lain.source.watch@1 (D-068, D-076): the
// provider owns the fsnotify watcher, the watched-directory set and the
// per-library debounce. The gateway polls on a ticker and reconciles
// dirty libraries through the existing ingest path; a swapped provider
// can watch differently (or not at all) without touching the gateway.
// inotify cannot see inside network mounts — those libraries keep
// reconciling through the manual scan either way.
package sourcewatch

import (
	"io/fs"
	"log/slog"
	"os"
	"path/filepath"
	"sync"
	"time"

	"github.com/fsnotify/fsnotify"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

const ID = "lain-source-watch"

// DefaultDebounce collapses event bursts: a deleted season or a copied
// episode is never a single event, so the rescan waits for quiet.
const DefaultDebounce = 2 * time.Second

// Provider watches library roots and reports dirty library ids.
type Provider struct {
	// Debounce is the quiet window a dirty library must stay below
	// before poll drains it. Zero uses DefaultDebounce.
	Debounce time.Duration
	// NoFS is a test seam: bookkeeping (dirty/debounce/library diff)
	// runs without fsnotify, so deterministic clocks can drive it.
	NoFS bool

	mu      sync.Mutex
	fs      *fsnotify.Watcher
	dirs    map[string]string    // watched dir -> library id
	known   map[string]string    // library id -> root
	dirty   map[string]time.Time // library id -> last event time
	closed  bool
	started bool
	log     *slog.Logger
}

// SetLogger points the provider's diagnostics at the server logger.
func (p *Provider) SetLogger(l *slog.Logger) {
	p.mu.Lock()
	defer p.mu.Unlock()
	if l != nil {
		p.log = l
	}
}

func (p *Provider) logger() *slog.Logger {
	if p.log != nil {
		return p.log
	}
	return slog.New(slog.DiscardHandler)
}

func (*Provider) ID() string             { return ID }
func (*Provider) Capabilities() []string { return []string{contracts.CapSourceWatch} }

func (p *Provider) Health() error {
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.closed {
		return &core.Error{Code: "dependency-unavailable", Msg: "watcher closed"}
	}
	return nil
}

// WatchLibrary is one root the provider should watch.
type WatchLibrary struct {
	ID   string `json:"id"`
	Path string `json:"path"`
}

// PollInput carries the current library set: poll is the sync point,
// so library adds/removes need no separate op — the provider diffs
// its watched set against the input.
type PollInput struct {
	Libraries []WatchLibrary `json:"libraries"`
}

// PollOutput drains the libraries whose debounce window elapsed.
type PollOutput struct {
	Dirty []string `json:"dirty"`
}

// DirtyInput re-marks a library dirty with a fresh timestamp — the
// gateway's re-nudge when a scan is already running.
type DirtyInput struct {
	LibraryID string `json:"library_id"`
}

// CloseInput stops the watcher.
type CloseInput struct{}

func (p *Provider) Invoke(cap string, input any) (any, error) {
	if cap != contracts.CapSourceWatch {
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
	switch in := input.(type) {
	case PollInput:
		return p.poll(in)
	case DirtyInput:
		p.mu.Lock()
		if !p.closed {
			p.dirty[in.LibraryID] = time.Now()
		}
		p.mu.Unlock()
		return true, nil
	case CloseInput:
		return true, p.Close()
	default:
		return nil, &core.Error{Code: "invalid-message", Msg: "watch input required"}
	}
}

// poll syncs the watched set to the live libraries and drains dirty
// ids that stayed quiet for the debounce window.
func (p *Provider) poll(in PollInput) (PollOutput, error) {
	p.mu.Lock()
	if p.closed {
		p.mu.Unlock()
		return PollOutput{}, &core.Error{Code: "dependency-unavailable", Msg: "watcher closed"}
	}
	if !p.started {
		p.started = true // mark first so a bad fsnotify reports once
		p.mu.Unlock()
		if !p.NoFS {
			if err := p.start(); err != nil {
				return PollOutput{}, err
			}
		}
		p.mu.Lock()
	}
	live := map[string]bool{}
	var fresh []WatchLibrary
	for _, lib := range in.Libraries {
		live[lib.ID] = true
		if root, ok := p.known[lib.ID]; !ok || root != lib.Path {
			fresh = append(fresh, lib)
			p.known[lib.ID] = lib.Path
		}
	}
	for libID := range p.known {
		if !live[libID] {
			p.dropLocked(libID)
		}
	}
	debounce := p.Debounce
	if debounce <= 0 {
		debounce = DefaultDebounce
	}
	now := time.Now()
	var out PollOutput
	for libID, at := range p.dirty {
		if !live[libID] {
			delete(p.dirty, libID)
			continue
		}
		if now.Sub(at) >= debounce {
			delete(p.dirty, libID)
			out.Dirty = append(out.Dirty, libID)
		}
	}
	p.mu.Unlock()
	// Tree walks stay outside the lock: they do real I/O and only touch
	// dirs through add(), which takes the lock per directory.
	for _, lib := range fresh {
		p.watchTree(lib.Path, lib.ID)
	}
	return out, nil
}

// start creates the fsnotify watcher and its event loop.
func (p *Provider) start() error {
	fsw, err := fsnotify.NewWatcher()
	if err != nil {
		return &core.Error{Code: "dependency-unavailable", Msg: "watcher unavailable: " + err.Error()}
	}
	p.mu.Lock()
	p.fs = fsw
	p.mu.Unlock()
	go p.loop()
	return nil
}

// watchTree adds a watch for root and every directory inside it.
// Unreadable trees stay unwatched: the scan's own unreadable-root
// reporting already names them for the operator.
func (p *Provider) watchTree(root, libID string) {
	fi, err := os.Stat(root)
	if err != nil || !fi.IsDir() {
		p.logger().Warn("library root not watchable", "library", libID, "root", root)
		return
	}
	_ = filepath.WalkDir(root, func(path string, d fs.DirEntry, err error) error {
		if err != nil || !d.IsDir() {
			return nil
		}
		p.add(path, libID)
		return nil
	})
}

func (p *Provider) add(dir, libID string) {
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.closed || p.fs == nil {
		return
	}
	if _, ok := p.dirs[dir]; ok {
		return
	}
	if err := p.fs.Add(dir); err != nil {
		return
	}
	p.dirs[dir] = libID
}

// dropLocked unwatches one library's dirs and forgets its dirty state.
func (p *Provider) dropLocked(libID string) {
	for dir, id := range p.dirs {
		if id == libID {
			if p.fs != nil {
				_ = p.fs.Remove(dir)
			}
			delete(p.dirs, dir)
		}
	}
	delete(p.known, libID)
	delete(p.dirty, libID)
}

// libOf resolves the library owning a path by walking ancestors until a
// watched dir matches — events report children of watched dirs.
func (p *Provider) libOf(path string) string {
	p.mu.Lock()
	defer p.mu.Unlock()
	for q := path; ; q = filepath.Dir(q) {
		if libID, ok := p.dirs[q]; ok {
			return libID
		}
		if parent := filepath.Dir(q); parent == q {
			return ""
		}
	}
}

func (p *Provider) loop() {
	for {
		select {
		case ev, ok := <-p.fs.Events:
			if !ok {
				return
			}
			p.onEvent(ev)
		case err, ok := <-p.fs.Errors:
			if !ok {
				return
			}
			p.logger().Warn("library watcher error", "err", err.Error())
		}
	}
}

func (p *Provider) onEvent(ev fsnotify.Event) {
	libID := p.libOf(ev.Name)
	if libID == "" {
		return
	}
	// A created directory — including the top of a tree moved in — must
	// be watched before anything inside it can report.
	if ev.Op&fsnotify.Create != 0 {
		if fi, err := os.Stat(ev.Name); err == nil && fi.IsDir() {
			p.watchTree(ev.Name, libID)
		}
	}
	// fsnotify drops the watch of a removed/renamed dir itself; the map
	// forgets it so the stale path never mis-attributes a later event.
	if ev.Op&(fsnotify.Remove|fsnotify.Rename) != 0 {
		p.mu.Lock()
		delete(p.dirs, ev.Name)
		p.mu.Unlock()
	}
	p.mu.Lock()
	if !p.closed {
		p.dirty[libID] = time.Now()
	}
	p.mu.Unlock()
}

// Close stops the watcher and forgets all state.
func (p *Provider) Close() error {
	p.mu.Lock()
	if p.closed {
		p.mu.Unlock()
		return nil
	}
	p.closed = true
	fsw := p.fs
	p.mu.Unlock()
	if fsw != nil {
		return fsw.Close()
	}
	return nil
}

// Watching reports whether a library currently has an active watch
// tree — a diagnostics hook for the gateway's lifecycle checks.
func (p *Provider) Watching(libID string) bool {
	p.mu.Lock()
	defer p.mu.Unlock()
	_, ok := p.known[libID]
	return ok
}

// NewWatcherProvider constructs the built-in watcher provider.
func NewWatcherProvider() *Provider {
	return &Provider{
		Debounce: DefaultDebounce,
		dirs:     map[string]string{},
		known:    map[string]string{},
		dirty:    map[string]time.Time{},
	}
}
