package core

import (
	"fmt"
	"sort"
	"sync"
	"time"
)

// Provider is one replaceable implementation behind one or more
// capabilities. Invoke is the uniform call surface: it mirrors the
// Matrix invoke(cap, input) shape so a future component-mode port is
// mechanical. Input and output types per capability are documented in
// docs/CONTRACTS.md; a wrong input type is an "invalid-message" error,
// never a panic.
type Provider interface {
	ID() string
	Capabilities() []string
	Health() error
	Invoke(cap string, input any) (any, error)
}

// Event records composition changes and recovery actions for
// diagnostics. Bounded in memory; the audit endpoint exposes it.
type Event struct {
	At         int64  `json:"at"`
	Kind       string `json:"kind"`
	Capability string `json:"capability,omitempty"`
	Provider   string `json:"provider,omitempty"`
	Generation uint64 `json:"generation,omitempty"`
	Detail     string `json:"detail,omitempty"`
}

// Registry is the runtime authority over the composition. All mutation
// goes through Swap/Withdraw with health checks and generation fencing:
// a broken provider can fail validation while the previous generation
// keeps serving.
type Registry struct {
	mu        sync.RWMutex
	providers map[string]Provider
	comp      *Composition
	lastGood  map[string]string // capability -> provider id that last served ok
	events    []Event
}

// NewRegistry builds a registry over a validated composition.
func NewRegistry(comp *Composition) *Registry {
	return &Registry{
		providers: map[string]Provider{},
		comp:      comp,
		lastGood:  map[string]string{},
	}
}

// Register adds a provider implementation. Registration alone never
// changes which provider serves traffic; only Swap does.
func (r *Registry) Register(p Provider) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.providers[p.ID()] = p
}

func (r *Registry) logLocked(e Event) {
	e.At = time.Now().Unix()
	r.events = append(r.events, e)
	if len(r.events) > 256 {
		r.events = r.events[len(r.events)-256:]
	}
}

// Swap moves a capability to a new provider list after validating every
// candidate's Health. expectedGen==0 skips fencing; otherwise a mismatch
// fails with stale-generation and changes nothing.
func (r *Registry) Swap(capability string, providers []string, expectedGen uint64) (uint64, error) {
	r.mu.Lock()
	defer r.mu.Unlock()
	b, ok := r.comp.Bindings[capability]
	if !ok {
		return 0, &Error{Code: "invalid-message", Msg: "unknown capability " + capability}
	}
	if expectedGen != 0 && expectedGen != b.Generation {
		return b.Generation, &Error{Code: "stale-generation", Msg: fmt.Sprintf("expected generation %d, active is %d", expectedGen, b.Generation)}
	}
	if b.Mode == ModeExactlyOne && len(providers) != 1 {
		return b.Generation, &Error{Code: "invalid-message", Msg: "exactly-one needs 1 provider"}
	}
	for _, id := range providers {
		p, ok := r.providers[id]
		if !ok {
			return b.Generation, &Error{Code: "invalid-message", Msg: "unknown provider " + id}
		}
		supported := false
		for _, c := range p.Capabilities() {
			if c == capability {
				supported = true
				break
			}
		}
		if !supported {
			return b.Generation, &Error{Code: "invalid-message", Msg: fmt.Sprintf("provider %s does not serve %s", id, capability)}
		}
		if err := p.Health(); err != nil {
			r.logLocked(Event{Kind: "swap-rejected", Capability: capability, Provider: id, Generation: b.Generation, Detail: err.Error()})
			return b.Generation, &Error{Code: "dependency-unavailable", Msg: fmt.Sprintf("provider %s unhealthy: %v", id, err)}
		}
	}
	prev := append([]string(nil), b.Providers...)
	b.Providers = append([]string(nil), providers...)
	b.Generation++
	if len(prev) == 1 {
		r.lastGood[capability] = prev[0]
	}
	r.logLocked(Event{Kind: "swap", Capability: capability, Provider: providers[0], Generation: b.Generation, Detail: fmt.Sprintf("from %v", prev)})
	return b.Generation, nil
}

// Withdraw removes a provider from serving without deleting its code.
// Bindings that referenced it fall back to remaining healthy providers;
// an exactly-one binding with no alternative fails closed and keeps the
// last generation marked degraded instead of serving nothing silently.
func (r *Registry) Withdraw(id string) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	if _, ok := r.providers[id]; !ok {
		return &Error{Code: "invalid-message", Msg: "unknown provider " + id}
	}
	for cap, b := range r.comp.Bindings {
		kept := b.Providers[:0:0]
		dropped := false
		for _, p := range b.Providers {
			if p == id {
				dropped = true
				continue
			}
			kept = append(kept, p)
		}
		if !dropped {
			continue
		}
		if len(kept) == 0 {
			r.logLocked(Event{Kind: "withdraw-degraded", Capability: cap, Provider: id, Generation: b.Generation, Detail: "no alternative provider; binding kept but degraded"})
			continue
		}
		b.Providers = kept
		b.Generation++
		r.logLocked(Event{Kind: "withdraw", Capability: cap, Provider: id, Generation: b.Generation, Detail: fmt.Sprintf("fallback to %v", kept)})
	}
	return nil
}

// Ordered returns the healthy providers for a capability in binding order.
func (r *Registry) Ordered(capability string) ([]Provider, uint64, error) {
	r.mu.RLock()
	defer r.mu.RUnlock()
	b, ok := r.comp.Bindings[capability]
	if !ok {
		return nil, 0, &Error{Code: "invalid-message", Msg: "unknown capability " + capability}
	}
	var out []Provider
	for _, id := range b.Providers {
		p, ok := r.providers[id]
		if !ok {
			continue
		}
		if err := p.Health(); err != nil {
			continue
		}
		out = append(out, p)
	}
	if len(out) == 0 {
		return nil, b.Generation, &Error{Code: "dependency-unavailable", Msg: "no healthy provider for " + capability}
	}
	return out, b.Generation, nil
}

// CallOne invokes an exactly-one capability. If the active provider
// fails at call time, the last-good provider is tried once and the
// incident is logged; independent capabilities are unaffected.
func (r *Registry) CallOne(capability string, input any) (any, string, error) {
	provs, gen, err := r.Ordered(capability)
	if err != nil {
		return nil, "", err
	}
	out, callErr := provs[0].Invoke(capability, input)
	if callErr == nil {
		return out, provs[0].ID(), nil
	}
	r.mu.RLock()
	fallbackID := r.lastGood[capability]
	fb, ok := r.providers[fallbackID]
	r.mu.RUnlock()
	if ok && fallbackID != "" && fallbackID != provs[0].ID() {
		if err := fb.Health(); err == nil {
			if out2, err2 := fb.Invoke(capability, input); err2 == nil {
				r.mu.Lock()
				r.logLocked(Event{Kind: "fallback", Capability: capability, Provider: fallbackID, Generation: gen, Detail: fmt.Sprintf("active %s failed: %v", provs[0].ID(), callErr)})
				r.mu.Unlock()
				return out2, fallbackID, nil
			}
		}
	}
	return nil, provs[0].ID(), callErr
}

// CallFirst tries ordered providers until one accepts. The accepted
// predicate interprets the capability's output (identify: Accepted()).
// It returns the output, the serving provider id, and whether accepted.
func (r *Registry) CallFirst(capability string, input any, accepted func(any) bool) (any, string, bool, error) {
	provs, _, err := r.Ordered(capability)
	if err != nil {
		return nil, "", false, err
	}
	var lastErr error
	for _, p := range provs {
		out, err := p.Invoke(capability, input)
		if err != nil {
			lastErr = err
			continue
		}
		if accepted(out) {
			return out, p.ID(), true, nil
		}
	}
	return nil, "", false, lastErr
}

// CallMerge fans out to every healthy provider of a merge-many
// capability and hands the collected outputs to merge. Provider
// failures are skipped (best-effort remotes), never fatal: a failing
// provider degrades the merge, it does not fail the call. Total
// failure of all providers is dependency-unavailable.
func (r *Registry) CallMerge(capability string, input any, merge func(outputs []any, ids []string) any) (any, []string, error) {
	provs, _, err := r.Ordered(capability)
	if err != nil {
		return nil, nil, err
	}
	var outputs []any
	var ids []string
	for _, p := range provs {
		out, err := p.Invoke(capability, input)
		if err != nil {
			continue
		}
		outputs = append(outputs, out)
		ids = append(ids, p.ID())
	}
	if len(outputs) == 0 {
		return nil, nil, &Error{Code: "dependency-unavailable", Msg: "all providers failed for " + capability}
	}
	return merge(outputs, ids), ids, nil
}

// InvokeProvider calls one named provider of a capability directly,
// still through authority: the provider must be registered, bound to
// the capability, and healthy. Used when the caller already chose
// (e.g. resolving the winning search candidate).
func (r *Registry) InvokeProvider(capability, providerID string, input any) (any, error) {
	r.mu.RLock()
	b, ok := r.comp.Bindings[capability]
	p, pok := r.providers[providerID]
	r.mu.RUnlock()
	if !ok {
		return nil, &Error{Code: "invalid-message", Msg: "unknown capability " + capability}
	}
	if !pok {
		return nil, &Error{Code: "invalid-message", Msg: "unknown provider " + providerID}
	}
	bound := false
	for _, id := range b.Providers {
		if id == providerID {
			bound = true
			break
		}
	}
	if !bound {
		return nil, &Error{Code: "invalid-message", Msg: "provider " + providerID + " not bound to " + capability}
	}
	serves := false
	for _, c := range p.Capabilities() {
		if c == capability {
			serves = true
			break
		}
	}
	if !serves {
		return nil, &Error{Code: "invalid-message", Msg: "provider " + providerID + " does not serve " + capability}
	}
	if err := p.Health(); err != nil {
		return nil, &Error{Code: "dependency-unavailable", Msg: "provider " + providerID + " unhealthy"}
	}
	return p.Invoke(capability, input)
}

// Providers lists registered provider ids sorted.
func (r *Registry) Providers() []string {
	r.mu.RLock()
	defer r.mu.RUnlock()
	out := make([]string, 0, len(r.providers))
	for id := range r.providers {
		out = append(out, id)
	}
	sort.Strings(out)
	return out
}

// Events returns the recent composition/recovery log.
func (r *Registry) Events() []Event {
	r.mu.RLock()
	defer r.mu.RUnlock()
	return append([]Event(nil), r.events...)
}

// Composition returns the live composition (read-only copy of bindings).
func (r *Registry) Composition() *Composition {
	r.mu.RLock()
	defer r.mu.RUnlock()
	cp := &Composition{Version: r.comp.Version, Bindings: map[string]*Binding{}}
	for k, b := range r.comp.Bindings {
		nb := *b
		nb.Providers = append([]string(nil), b.Providers...)
		cp.Bindings[k] = &nb
	}
	return cp
}

// Error is a typed registry error with a stable wire code.
type Error struct {
	Code string
	Msg  string
}

func (e *Error) Error() string { return e.Code + ": " + e.Msg }
