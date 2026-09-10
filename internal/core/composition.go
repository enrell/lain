// Package core is Lain's trusted base: composition, authority over
// which provider serves each capability, generations, and recovery.
//
// It understands plugins, contracts, users, storage and recovery. It
// does not understand anime, seasons, metadata or translation: those
// are plugin policy behind the contracts in internal/contracts.
package core

import (
	"fmt"
	"sort"
)

// BindingMode declares how a capability composes providers. The mode is
// part of the contract: installing two providers never resolves by
// silent "who started first wins".
type BindingMode string

const (
	// ModeExactlyOne is a single authoritative owner (catalog, userstate).
	ModeExactlyOne BindingMode = "exactly-one"
	// ModeOrderedMany tries providers in declared order, first accepted wins.
	ModeOrderedMany BindingMode = "ordered-many"
	// ModeFirstAccepted tries in order until one accepts (planners).
	ModeFirstAccepted BindingMode = "first-accepted"
	// ModeMergeMany combines results with provenance (metadata).
	ModeMergeMany BindingMode = "merge-many"
	// ModeFanOut delivers to every provider (events, sync, webhooks).
	ModeFanOut BindingMode = "fan-out"
)

// Binding is the active choice of providers for one capability.
// Generation is a monotonic epoch: replacing a provider never
// revalidates an old reference (stale-generation errors on mismatch).
type Binding struct {
	Mode       BindingMode `json:"mode"`
	Providers  []string    `json:"providers"`
	Generation uint64      `json:"generation"`
}

// Composition is the installed choice of implementations.
type Composition struct {
	Version  int                 `json:"version"`
	Bindings map[string]*Binding `json:"bindings"`
}

// DefaultComposition is the v0.1 built-in set.
func DefaultComposition() *Composition {
	return &Composition{
		Version: 1,
		Bindings: map[string]*Binding{
			"lain.source.enumerate@1":    {Mode: ModeExactlyOne, Providers: []string{"lain-source-filesystem"}, Generation: 1},
			"lain.media.identify@1":      {Mode: ModeOrderedMany, Providers: []string{"lain-identify-anime", "lain-identify-generic"}, Generation: 1},
			"lain.catalog.read@1":        {Mode: ModeExactlyOne, Providers: []string{"lain-catalog-bolt"}, Generation: 1},
			"lain.catalog.write@1":       {Mode: ModeExactlyOne, Providers: []string{"lain-catalog-bolt"}, Generation: 1},
			"lain.userstate.progress@1":  {Mode: ModeExactlyOne, Providers: []string{"lain-userstate-bolt"}, Generation: 1},
			"lain.playback.plan@1":       {Mode: ModeFirstAccepted, Providers: []string{"lain-playback-default"}, Generation: 1},
			"lain.transform.thumbnail@1": {Mode: ModeExactlyOne, Providers: []string{"lain-thumbnail-ffmpeg"}, Generation: 1},
			"lain.search.query@1":        {Mode: ModeExactlyOne, Providers: []string{"lain-search-simple"}, Generation: 1},
			"lain.ingest.scan@1":         {Mode: ModeExactlyOne, Providers: []string{"lain-ingest-default"}, Generation: 1},
			"lain.metadata.search@1":     {Mode: ModeMergeMany, Providers: []string{"lain-metadata-nfo", "lain-metadata-kitsu", "lain-metadata-anilist", "lain-metadata-jikan"}, Generation: 1},
			"lain.metadata.resolve@1":    {Mode: ModeMergeMany, Providers: []string{"lain-metadata-nfo", "lain-metadata-kitsu", "lain-metadata-anilist", "lain-metadata-jikan"}, Generation: 1},
		},
	}
}

// Validate checks the composition against registered provider IDs.
// exactly-one bindings must name exactly one provider.
func (c *Composition) Validate(known map[string]bool) error {
	if c == nil || len(c.Bindings) == 0 {
		return fmt.Errorf("empty composition")
	}
	for cap, b := range c.Bindings {
		if b == nil || len(b.Providers) == 0 {
			return fmt.Errorf("capability %s: no providers", cap)
		}
		if b.Mode == ModeExactlyOne && len(b.Providers) != 1 {
			return fmt.Errorf("capability %s: exactly-one needs 1 provider, got %d", cap, len(b.Providers))
		}
		for _, id := range b.Providers {
			if !known[id] {
				return fmt.Errorf("capability %s: unknown provider %s", cap, id)
			}
		}
	}
	return nil
}

// LegacyProviderIDs maps retired provider ids to their replacements.
// Applied once when loading a persisted composition (v0.1 file-backed
// providers became bolt-backed without changing capabilities).
var LegacyProviderIDs = map[string]string{
	"lain-catalog-file":   "lain-catalog-bolt",
	"lain-userstate-file": "lain-userstate-bolt",
}

// MigrateProviderIDs rewrites retired ids in place, reporting what moved.
func (c *Composition) MigrateProviderIDs() []string {
	var moved []string
	for _, b := range c.Bindings {
		for i, id := range b.Providers {
			if next, ok := LegacyProviderIDs[id]; ok {
				b.Providers[i] = next
				moved = append(moved, id+"->"+next)
			}
		}
	}
	return moved
}

// Upgrade adds bindings for capabilities the saved composition does
// not know yet (new Lain versions), keeping every user override.
// Returns the added capability names.
func (c *Composition) Upgrade(fresh *Composition) []string {
	var added []string
	for cap, b := range fresh.Bindings {
		if _, ok := c.Bindings[cap]; ok {
			continue
		}
		nb := *b
		nb.Providers = append([]string(nil), b.Providers...)
		c.Bindings[cap] = &nb
		added = append(added, cap)
	}
	return added
}

// BindingSnapshot is a stable sorted view for inspection endpoints.
type BindingView struct {
	Capability string   `json:"capability"`
	Mode       string   `json:"mode"`
	Providers  []string `json:"providers"`
	Generation uint64   `json:"generation"`
}

// View returns bindings sorted by capability name.
func (c *Composition) View() []BindingView {
	out := make([]BindingView, 0, len(c.Bindings))
	for cap, b := range c.Bindings {
		provs := append([]string(nil), b.Providers...)
		out = append(out, BindingView{Capability: cap, Mode: string(b.Mode), Providers: provs, Generation: b.Generation})
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Capability < out[j].Capability })
	return out
}
