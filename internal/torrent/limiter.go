package torrent

import (
	"context"
	"sync"
	"time"
)

// Limiter is a token bucket in bytes per second; a rate of 0 is
// unlimited. Upload and download caps are settings (A-5), so the rate
// can change while transfers run.
type Limiter struct {
	mu     sync.Mutex
	rate   int64
	tokens float64
	last   time.Time
}

// NewLimiter returns a limiter at rate bytes/second (0 = unlimited).
func NewLimiter(rate int64) *Limiter { return &Limiter{rate: rate, last: time.Now()} }

// SetRate changes the rate.
func (l *Limiter) SetRate(rate int64) {
	l.mu.Lock()
	l.rate = rate
	l.mu.Unlock()
}

// Wait blocks until n bytes may pass or ctx ends.
func (l *Limiter) Wait(ctx context.Context, n int) error {
	for {
		l.mu.Lock()
		if l.rate <= 0 {
			l.mu.Unlock()
			return nil
		}
		now := time.Now()
		l.tokens += now.Sub(l.last).Seconds() * float64(l.rate)
		l.last = now
		// Allow at most one second of burst.
		if l.tokens > float64(l.rate) {
			l.tokens = float64(l.rate)
		}
		need := float64(n)
		if need > float64(l.rate) {
			need = float64(l.rate) // a block larger than a second's worth
		}
		if l.tokens >= need {
			l.tokens -= float64(n)
			l.mu.Unlock()
			return nil
		}
		wait := time.Duration((need - l.tokens) / float64(l.rate) * float64(time.Second))
		l.mu.Unlock()
		t := time.NewTimer(wait)
		select {
		case <-ctx.Done():
			t.Stop()
			return ctx.Err()
		case <-t.C:
		}
	}
}
