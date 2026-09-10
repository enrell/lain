// Package bench is the observability toolkit behind `lain bench`:
// controlled workloads, resource sampling, and self-contained HTML
// reports. Stdlib only: charts are hand-rolled SVG, no CDN, no JS
// framework — a report must open offline years from now.
package bench

import (
	"time"
)

// Sampler records peak resident memory during a workload by polling
// the OS at a fixed interval. Stop returns the peak in bytes.
type Sampler struct {
	stop    chan struct{}
	done    chan uint64
	initial uint64
}

// Start begins sampling every interval until Stop is called. The first
// sample is taken synchronously so Peak >= Initial always holds.
func Start(interval time.Duration) *Sampler {
	s := &Sampler{stop: make(chan struct{}), done: make(chan uint64, 1), initial: rss()}
	peak := s.initial
	go func() {
		t := time.NewTicker(interval)
		defer t.Stop()
		for {
			select {
			case <-s.stop:
				if cur := rss(); cur > peak {
					peak = cur
				}
				s.done <- peak
				return
			case <-t.C:
				if cur := rss(); cur > peak {
					peak = cur
				}
			}
		}
	}()
	return s
}

// Stop ends sampling and returns peak RSS in bytes (0 when unsupported).
func (s *Sampler) Stop() uint64 {
	close(s.stop)
	return <-s.done
}

// MemSnapshot is a point-in-time Go allocator view.
type MemSnapshot struct {
	HeapAlloc uint64 `json:"heap_alloc"`
	HeapSys   uint64 `json:"heap_sys"`
	NumGC     uint32 `json:"num_gc"`
}
