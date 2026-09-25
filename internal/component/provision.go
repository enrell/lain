package component

import (
	"encoding/json"
	"log/slog"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/fsnotify/fsnotify"

	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/matrix"
)

// Provisioner installs components from a watched directory (D-077):
// the operator drops an <id>.json manifest plus its executable into
// <data-dir>/plugins/; boot discovers and registers, and a runtime
// watch installs, reloads and removes components live through the
// registry's own authority (Register/Swap/Unregister). Sockets live in
// a hidden .run subdirectory so they never match the manifest glob.
type Provisioner struct {
	dir string
	reg *core.Registry
	log *slog.Logger

	mu        sync.Mutex
	procs     map[string]*Process // manifest id -> running component
	mtimes    map[string]int64    // manifest path -> last seen modtime
	done      chan struct{}
	wg        sync.WaitGroup
	watcher   *fsnotify.Watcher
	closeOnce sync.Once
}

// NewProvisioner watches dir for component manifests.
func NewProvisioner(dir string, reg *core.Registry, log *slog.Logger) *Provisioner {
	if log == nil {
		log = slog.Default()
	}
	return &Provisioner{dir: dir, reg: reg, log: log, procs: map[string]*Process{}, mtimes: map[string]int64{}, done: make(chan struct{})}
}

// SetLogger swaps the provisioner's logger (the server's SetLogger
// propagates it so component lines share the access-log format).
func (p *Provisioner) SetLogger(l *slog.Logger) {
	p.mu.Lock()
	defer p.mu.Unlock()
	if l != nil {
		p.log = l
	}
}

// Start provisions every manifest present and begins the watch. An
// unwatchable directory is not fatal: boot-time discovery already ran
// and the error is reported so the operator learns why live installs
// do not land.
func (p *Provisioner) Start() error {
	if err := os.MkdirAll(p.dir, 0o755); err != nil {
		return err
	}
	p.sync()
	w, err := fsnotify.NewWatcher()
	if err != nil {
		return err
	}
	if err := w.Add(p.dir); err != nil {
		w.Close()
		return err
	}
	p.watcher = w
	p.wg.Add(1)
	go p.watch()
	return nil
}

// Close stops the watch and every spawned component. Idempotent —
// Server.Close may run more than once (test cleanup).
func (p *Provisioner) Close() {
	p.closeOnce.Do(func() {
		close(p.done)
		p.wg.Wait()
		p.mu.Lock()
		defer p.mu.Unlock()
		for id, proc := range p.procs {
			_ = proc.Close()
			delete(p.procs, id)
		}
		if p.watcher != nil {
			_ = p.watcher.Close()
		}
	})
}

// watch debounces fsnotify events into a reconcile pass: manifest
// writes arrive as create+write bursts, so sync diffs the directory
// rather than trusting event names.
func (p *Provisioner) watch() {
	defer p.wg.Done()
	var debounce *time.Timer
	var debounceC <-chan time.Time
	for {
		select {
		case <-p.done:
			if debounce != nil {
				debounce.Stop()
			}
			return
		case _, ok := <-p.watcher.Events:
			if !ok {
				return
			}
			if debounce == nil {
				debounce = time.NewTimer(300 * time.Millisecond)
				debounceC = debounce.C
			} else {
				debounce.Reset(300 * time.Millisecond)
			}
		case err, ok := <-p.watcher.Errors:
			if !ok {
				return
			}
			p.log.Warn("plugin watch error", "err", err.Error())
		case <-debounceC:
			debounceC = nil
			debounce = nil
			p.sync()
		}
	}
}

// sync reconciles the directory with the running set: new or rewritten
// manifests spawn (respawn) their component; vanished manifests get
// withdrawn from bindings, unregistered and killed.
func (p *Provisioner) sync() {
	entries, err := os.ReadDir(p.dir)
	if err != nil {
		p.log.Warn("plugin dir unreadable", "dir", p.dir, "err", err.Error())
		return
	}
	seen := map[string]string{} // manifest path -> provider id
	for _, e := range entries {
		if e.IsDir() || !strings.HasSuffix(e.Name(), ".json") {
			continue
		}
		path := filepath.Join(p.dir, e.Name())
		info, err := e.Info()
		if err != nil {
			continue
		}
		var m matrix.Manifest
		raw, err := os.ReadFile(path)
		if err != nil || json.Unmarshal(raw, &m) != nil || m.ID == "" {
			p.log.Warn("invalid plugin manifest", "file", e.Name())
			continue
		}
		seen[path] = m.ID
		p.mu.Lock()
		_, running := p.procs[m.ID]
		changed := p.mtimes[path] != info.ModTime().UnixNano()
		p.mtimes[path] = info.ModTime().UnixNano()
		p.mu.Unlock()
		if running && !changed {
			continue
		}
		p.reinstall(m)
	}
	// Remove what vanished.
	p.mu.Lock()
	var gone []string
	kept := map[string]int64{}
	for path, mt := range p.mtimes {
		if _, ok := seen[path]; ok {
			kept[path] = mt
		} else {
			gone = append(gone, path)
		}
	}
	p.mtimes = kept
	p.mu.Unlock()
	for _, path := range gone {
		p.remove(path)
	}
}

func (p *Provisioner) reinstall(m matrix.Manifest) {
	proc, err := Spawn(m, filepath.Join(p.dir, ".run"))
	if err != nil {
		p.log.Warn("plugin spawn failed", "id", m.ID, "err", err.Error())
		return
	}
	p.mu.Lock()
	old := p.procs[m.ID]
	p.procs[m.ID] = proc
	p.mu.Unlock()
	if old != nil {
		_ = old.Close()
	}
	// Register under the manifest id: a component naming an existing
	// provider id replaces it — bindings keep working, now served by
	// the component (D-074).
	p.reg.Register(proc)
	p.log.Info("plugin installed", "id", m.ID, "capabilities", strings.Join(m.Capabilities, ","))
}

func (p *Provisioner) remove(path string) {
	p.mu.Lock()
	defer p.mu.Unlock()
	for id, proc := range p.procs {
		if filepath.Join(p.dir, id+".json") != path {
			continue
		}
		delete(p.procs, id)
		_ = proc.Close()
		if err := p.reg.Unregister(id); err != nil {
			p.log.Warn("plugin unregister failed", "id", id, "err", err.Error())
		}
		p.log.Info("plugin removed", "id", id)
	}
}

// Installed lists the running component ids, sorted (diagnostics).
func (p *Provisioner) Installed() []string {
	p.mu.Lock()
	defer p.mu.Unlock()
	out := make([]string, 0, len(p.procs))
	for id := range p.procs {
		out = append(out, id)
	}
	sort.Strings(out)
	return out
}
