package transcode


// Model-based test of the v2 job lifecycle. A seeded random walk mixes
// start / status / resolve / finish / fail / retry / stop / queue-full
// ops against a real Transcoder whose convert function is gated per
// session, so the model controls exactly when each job settles.
//
// Invariants asserted after every op:
//   - admission never exceeds MaxConcurrent (active) or QueueSize (pending)
//   - a job's done channel is closed iff its state is terminal, and
//     closes exactly once (a second close would panic the suite)
//   - state transitions only follow the legal edges:
//     idle -> queued -> running -> ready|failed, failed -> queued
//     (retry), terminal -> gone (prune)
//   - a stopped job must never come back to ready even if its convert
//     finishes successfully afterwards
//   - progress stays inside [0,1]
//
// Deterministic under fixed seeds; bump the table to explore other walks.

import (
	"fmt"
	"math/rand"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
)

func TestModelJobLifecycle(t *testing.T) {
	for _, seed := range []int64{1, 7, 20260923, 555} {
		t.Run(fmt.Sprintf("seed-%d", seed), func(t *testing.T) {
			jobLifecycleWalk(t, seed)
		})
	}
}

// jobCtl is the model's handle on one session: how its convert should
// behave once the worker reaches it, and which states the model allows.
type jobCtl struct {
	src      string
	session  string
	release  chan struct{}
	outcome  string // "pending" (gated), "ok", "fail"
	released bool
	lastSeen string // last model-observed state, for transition checks
	pruned   bool
}

func jobLifecycleWalk(t *testing.T, seed int64) {
	rng := rand.New(rand.NewSource(seed))
	root := t.TempDir()

	var mu sync.Mutex
	gates := map[string]chan struct{}{}
	outcomes := map[string]string{}
	var jobs []*jobCtl

	sources := make([]string, 4)
	for i := range sources {
		sources[i] = filepath.Join(root, fmt.Sprintf("src%d.mp4", i))
		if err := os.WriteFile(sources[i], []byte("src"), 0o600); err != nil {
			t.Fatal(err)
		}
	}

	// Every session can hold a slot at once: the model drives outcomes
	// through per-session gates, and a gated convert holding the last
	// slot would starve later ops. Queue-bound behaviour is covered by
	// TestAsyncQueueIsBounded already.
	tr := newWithDeps(filepath.Join(root, "cache"),
		Config{MaxCacheBytes: 1 << 20, QueueSize: len(sources), MaxConcurrent: len(sources)},
		func(spec sourceSpec, out string, _ func(progressSample)) (string, error) {
			mu.Lock()
			gate := gates[spec.Session]
			mu.Unlock()
			if gate != nil {
				<-gate
			}
			mu.Lock()
			outcome := outcomes[spec.Session]
			mu.Unlock()
			switch outcome {
			case "fail":
				return "", &transcodeModelError{msg: "convert failed by model"}
			default:
				return "transcode", os.WriteFile(out, []byte("mp4"), 0o600)
			}
		}, time.Now)
	t.Cleanup(func() { _ = tr.Close() })

	// Reachability-closed legal edges: observation samples the state, so
	// intermediate hops (queued -> running -> ready) may be skipped.
	// "gone" means pruned from t.jobs.
	legal := map[string]map[string]bool{
		"":                         {contracts.TranscodeQueued: true, contracts.TranscodeRunning: true, contracts.TranscodeReady: true, contracts.TranscodeIdle: true, contracts.TranscodeFailed: true},
		contracts.TranscodeIdle:    {contracts.TranscodeQueued: true, contracts.TranscodeRunning: true, contracts.TranscodeReady: true, contracts.TranscodeFailed: true},
		contracts.TranscodeQueued:  {contracts.TranscodeQueued: true, contracts.TranscodeRunning: true, contracts.TranscodeReady: true, contracts.TranscodeFailed: true},
		contracts.TranscodeRunning: {contracts.TranscodeRunning: true, contracts.TranscodeReady: true, contracts.TranscodeFailed: true},
		contracts.TranscodeReady:   {contracts.TranscodeReady: true, "gone": true},
		contracts.TranscodeFailed:  {contracts.TranscodeFailed: true, contracts.TranscodeQueued: true, contracts.TranscodeRunning: true, contracts.TranscodeReady: true, "gone": true},
		"gone":                     {contracts.TranscodeQueued: true, contracts.TranscodeRunning: true, contracts.TranscodeReady: true, contracts.TranscodeFailed: true},
	}

	checkInvariants := func(step int) {
		tr.mu.Lock()
		if tr.active > tr.maxConcurrent {
			t.Fatalf("step %d: active=%d exceeds MaxConcurrent=%d", step, tr.active, tr.maxConcurrent)
		}
		if tr.pending > tr.queueLimit {
			t.Fatalf("step %d: pending=%d exceeds QueueSize=%d", step, tr.pending, tr.queueLimit)
		}
		if tr.active < 0 || tr.pending < 0 {
			t.Fatalf("step %d: negative counters active=%d pending=%d", step, tr.active, tr.pending)
		}
		for session, j := range tr.jobs {
			terminal := j.state == contracts.TranscodeReady || j.state == contracts.TranscodeFailed
			select {
			case <-j.done:
				if !terminal {
					t.Fatalf("step %d: job %s done closed while state=%s", step, session, j.state)
				}
			default:
				if terminal {
					t.Fatalf("step %d: job %s done open at terminal state=%s", step, session, j.state)
				}
			}
			if j.progress < 0 || j.progress > 1 {
				t.Fatalf("step %d: job %s progress %f out of range", step, session, j.progress)
			}
			if j.stopped && j.state == contracts.TranscodeReady {
				t.Fatalf("step %d: stopped job %s resurrected to ready", step, session)
			}
		}
		tr.mu.Unlock()
	}

	observe := func(step int, c *jobCtl, state string) {
		if c.pruned {
			state = "gone"
		}
		if !legal[c.lastSeen][state] {
			t.Fatalf("step %d: session %s illegal transition %s -> %s", step, c.session, c.lastSeen, state)
		}
		c.lastSeen = state
	}

	statusOf := func(c *jobCtl) contracts.TranscodeStatus {
		out, err := tr.Invoke(contracts.CapPlaybackTranscodeV2, contracts.TranscodeV2Request{
			Action: contracts.TranscodeStatusAction, FilePath: c.src, Session: c.session,
		})
		if err != nil {
			t.Fatalf("status for %s: %v", c.session, err)
		}
		return out.(contracts.TranscodeStatus)
	}

	for step := 0; step < 150; step++ {
		op := rng.Intn(100)
		switch {
		case op < 30 && len(jobs) < len(sources):
			// Start a fresh session: first inspect to learn its id, then
			// admit. The model expects queued-or-running.
			c := &jobCtl{src: sources[len(jobs)], release: make(chan struct{}), outcome: "pending"}
			inspect, err := tr.Invoke(contracts.CapPlaybackTranscodeV2, contracts.TranscodeV2Request{
				Action: contracts.TranscodeInspectAction, FilePath: c.src,
			})
			if err != nil {
				t.Fatalf("step %d: inspect: %v", step, err)
			}
			c.session = inspect.(contracts.TranscodeStatus).Session
			if c.session == "" {
				t.Fatalf("step %d: inspect returned empty session", step)
			}
			mu.Lock()
			gates[c.session] = c.release
			outcomes[c.session] = "pending"
			mu.Unlock()
			out, err := tr.Invoke(contracts.CapPlaybackTranscodeV2, contracts.TranscodeV2Request{
				Action: contracts.TranscodeStartAction, FilePath: c.src, Session: c.session,
			})
			if err != nil {
				t.Fatalf("step %d: start: %v", step, err)
			}
			st := out.(contracts.TranscodeStatus).State
			observe(step, c, st)
			if st != contracts.TranscodeQueued && st != contracts.TranscodeRunning {
				t.Fatalf("step %d: fresh start state=%s", step, st)
			}
			jobs = append(jobs, c)

		case op < 55 && len(jobs) > 0:
			// Status must agree with a legal transition from last seen.
			c := jobs[rng.Intn(len(jobs))]
			observe(step, c, statusOf(c).State)

		case op < 70 && len(jobs) > 0:
			// Release a gated convert with a success outcome — skip
			// sessions that already went terminal (a stopped job never
			// reaches its convert).
			c := pickUnreleased(t, tr, jobs)
			if c == nil {
				continue
			}
			mu.Lock()
			outcomes[c.session] = "ok"
			mu.Unlock()
			c.released = true
			close(c.release)
			got := waitState(t, tr, c.src, c.session, contracts.TranscodeReady)
			observe(step, c, got.State)

		case op < 82 && len(jobs) > 0:
			// Release a gated convert with a failure outcome.
			c := pickUnreleased(t, tr, jobs)
			if c == nil {
				continue
			}
			mu.Lock()
			outcomes[c.session] = "fail"
			mu.Unlock()
			c.released = true
			close(c.release)
			got := waitState(t, tr, c.src, c.session, contracts.TranscodeFailed)
			observe(step, c, got.State)

		case op < 90 && len(jobs) > 0:
			// Retry: an explicit start on a failed session re-queues it.
			var c *jobCtl
			for _, j := range jobs {
				mu.Lock()
				failed := outcomes[j.session] == "fail"
				mu.Unlock()
				if j.released && failed && j.lastSeen == contracts.TranscodeFailed {
					c = j
					break
				}
			}
			if c == nil {
				continue
			}
			c.release = make(chan struct{})
			c.released = false
			mu.Lock()
			gates[c.session] = c.release
			outcomes[c.session] = "pending"
			mu.Unlock()
			out, err := tr.Invoke(contracts.CapPlaybackTranscodeV2, contracts.TranscodeV2Request{
				Action: contracts.TranscodeStartAction, FilePath: c.src, Session: c.session,
			})
			if err != nil {
				t.Fatalf("step %d: retry: %v", step, err)
			}
			st := out.(contracts.TranscodeStatus).State
			observe(step, c, st)
			if st != contracts.TranscodeQueued && st != contracts.TranscodeRunning {
				t.Fatalf("step %d: retry state=%s", step, st)
			}

		case op < 95 && len(jobs) > 0:
			// Resolve: touches a ready artifact, harmless elsewhere.
			c := jobs[rng.Intn(len(jobs))]
			out, err := tr.Invoke(contracts.CapPlaybackTranscodeV2, contracts.TranscodeV2Request{
				Action: contracts.TranscodeResolveAction, FilePath: c.src, Session: c.session,
			})
			if err != nil {
				// A non-ready job may legitimately miss artifacts — but
				// a missing-session error must not appear for a live job.
				if c.lastSeen != "gone" {
					t.Fatalf("step %d: resolve on live session %s: %v", step, c.session, err)
				}
				continue
			}
			observe(step, c, out.(contracts.TranscodeStatus).State)

		default:
			// Stop a still-queued job. Running-stop is v3-only in
			// production (tick/cancel never see v2 jobs), so the model
			// only exercises the reachable path: queued -> stopped.
			var c *jobCtl
			tr.mu.Lock()
			for _, j := range jobs {
				if jj, ok := tr.jobs[j.session]; ok && jj.state == contracts.TranscodeQueued {
					c = j
					break
				}
			}
			if c == nil {
				tr.mu.Unlock()
				tr.pruneJobs()
				continue
			}
			j := tr.jobs[c.session]
			tr.mu.Unlock()
			tr.stopJob(j, "model-stop", "stopped by model")
			// The worker must observe the terminal state and close
			// done — a queued-stopped job leaking an open done channel
			// is exactly the hang this invariant exists to catch.
			select {
			case <-j.done:
			case <-time.After(2 * time.Second):
				t.Fatalf("step %d: stopped queued job %s never closed done", step, c.session)
			}
			tr.mu.Lock()
			st := tr.jobs[c.session].state
			tr.mu.Unlock()
			observe(step, c, st)
		}
		checkInvariants(step)
	}

	// Final sweep: release every gate, let jobs settle, then Close must
	// leave only terminal states and no leaked counters.
	for _, c := range jobs {
		if !c.released {
			mu.Lock()
			outcomes[c.session] = "ok"
			mu.Unlock()
			c.released = true
			close(c.release)
		}
	}
	for _, c := range jobs {
		st := statusOf(c).State
		if st != contracts.TranscodeReady && st != contracts.TranscodeFailed {
			t.Fatalf("final: session %s stuck at %s", c.session, st)
		}
	}
	checkInvariants(999)
	if err := tr.Close(); err != nil {
		t.Fatal(err)
	}
	tr.mu.Lock()
	if tr.active != 0 || tr.pending != 0 {
		t.Fatalf("after close: active=%d pending=%d", tr.active, tr.pending)
	}
	tr.mu.Unlock()
}

type transcodeModelError struct{ msg string }

func (e *transcodeModelError) Error() string { return e.msg }

// pickUnreleased returns a job whose convert has not been released yet
// and whose live state is not already terminal.
func pickUnreleased(t *testing.T, tr *Transcoder, jobs []*jobCtl) *jobCtl {
	t.Helper()
	tr.mu.Lock()
	defer tr.mu.Unlock()
	for _, c := range jobs {
		if c.released {
			continue
		}
		j, ok := tr.jobs[c.session]
		if !ok {
			continue
		}
		if j.state == contracts.TranscodeReady || j.state == contracts.TranscodeFailed {
			continue
		}
		return c
	}
	return nil
}
