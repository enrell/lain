// Library file watching (D-068, D-076). The watcher itself is the
// lain.source.watch@1 provider — it owns fsnotify, the watched set and
// the per-library debounce. The gateway polls it on a ticker and
// reconciles dirty libraries through the existing ingest pipeline, so
// deletions, additions and D-019 move reconciliation all ride one
// tested path. inotify cannot see inside network mounts — those
// libraries keep reconciling through the manual scan, and
// `--watch=0`/`LAIN_WATCH=0` means the capability is simply never
// invoked.
package gateway

import (
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/plugins/sourcewatch"
)

// watchPollInterval is how often the gateway drains dirty libraries.
// It only bounds detection latency; the debounce lives in the provider.
const watchPollInterval = 500 * time.Millisecond

// StartWatcher begins filesystem reconciliation. Serve calls it unless
// disabled; tests opt in explicitly. A second call is a no-op.
func (s *Server) StartWatcher() error {
	s.scanMu.Lock()
	if s.watchStarted {
		s.scanMu.Unlock()
		return nil
	}
	s.watchStarted = true
	s.scanMu.Unlock()
	if s.watchDebounce > 0 && s.watchProv != nil {
		s.watchProv.Debounce = s.watchDebounce
	}
	// The first poll runs synchronously: when StartWatcher returns, the
	// provider is already watching — events cannot slip through the
	// gap like the pre-capability watcher never allowed.
	s.pollWatch()
	go s.watchLoop()
	return nil
}

// watchLoop drains the watch capability until shutdown.
func (s *Server) watchLoop() {
	tick := time.NewTicker(watchPollInterval)
	defer tick.Stop()
	for {
		select {
		case <-s.watchDone:
			return
		case <-tick.C:
			s.pollWatch()
		}
	}
}

// pollWatch hands the provider the current library set — poll is the
// sync point, so adds and deletes need no separate signal — and
// reconciles whatever comes back dirty.
func (s *Server) pollWatch() {
	libs := s.libList()
	in := sourcewatch.PollInput{Libraries: make([]sourcewatch.WatchLibrary, 0, len(libs))}
	for _, lib := range libs {
		in.Libraries = append(in.Libraries, sourcewatch.WatchLibrary{ID: lib.ID, Path: lib.Path})
	}
	out, _, err := s.reg.CallOne(contracts.CapSourceWatch, in)
	if err != nil {
		s.logger().Debug("watch poll failed", "err", err.Error())
		return
	}
	dirty, _ := out.(sourcewatch.PollOutput)
	for _, id := range dirty.Dirty {
		s.scanLibrary(id)
	}
}

// scanLibrary runs the ingest pipeline for one library under the same
// scan lock as manual scans. A scan already in flight re-marks the
// library dirty in the provider instead of racing it.
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
		_, _, _ = s.reg.CallOne(contracts.CapSourceWatch, sourcewatch.DirtyInput{LibraryID: id})
		return
	}
	s.scan = ScanStatus{State: "running", StartedAt: time.Now().Unix(), Trigger: "watch"}
	s.scanMu.Unlock()
	go s.runScan([]contracts.Library{*lib}, "watch")
}
