package ingest

import (
	"sync"
	"time"
)

// ScanTrace records per-phase durations of one Run. Phases across
// parallel roots accumulate; the runner sets it only when non-nil, so
// production scans pay nothing when unobserved.
type ScanTrace struct {
	mu        sync.Mutex
	Enumerate time.Duration
	Identify  time.Duration
	Persist   time.Duration
	Prune     time.Duration
}

func (t *ScanTrace) addEnumerate(d time.Duration) {
	if t == nil {
		return
	}
	t.mu.Lock()
	t.Enumerate += d
	t.mu.Unlock()
}

func (t *ScanTrace) addIdentify(d time.Duration) {
	if t == nil {
		return
	}
	t.mu.Lock()
	t.Identify += d
	t.mu.Unlock()
}

func (t *ScanTrace) addPersist(d time.Duration) {
	if t == nil {
		return
	}
	t.mu.Lock()
	t.Persist = d
	t.mu.Unlock()
}

func (t *ScanTrace) addPrune(d time.Duration) {
	if t == nil {
		return
	}
	t.mu.Lock()
	t.Prune = d
	t.mu.Unlock()
}

// Snapshot returns a copy of the accumulated phases.
func (t *ScanTrace) Snapshot() ScanTrace {
	if t == nil {
		return ScanTrace{}
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	return ScanTrace{
		Enumerate: t.Enumerate,
		Identify:  t.Identify,
		Persist:   t.Persist,
		Prune:     t.Prune,
	}
}
