package metadata

import (
	"log/slog"
	"path/filepath"
	"strings"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

func dirOf(path string) string {
	if path == "" {
		return ""
	}
	return filepath.Dir(path)
}

// Enricher serves lain.metadata.enrich@1 (D-076): it owns the overlay
// pipeline end to end — the search fan-out, the pick, the resolve, the
// overlay document store and the search/record caches. Composition is
// authority the whole way: metadata providers are called through the
// registry, so swapping or withdrawing them changes enrichment.
type Enricher struct {
	Reg *core.Registry
	DB  *bolt.DB
	log *slog.Logger
}

const EnricherID = "lain-metadata-enrich"

func NewEnricher(reg *core.Registry, db *bolt.DB) *Enricher {
	return &Enricher{Reg: reg, DB: db, log: slog.New(slog.DiscardHandler)}
}

// SetLogger points the enricher's diagnostics at the server logger so
// its failures keep the access-log format.
func (e *Enricher) SetLogger(l *slog.Logger) {
	if l != nil {
		e.log = l
	}
}

func (e *Enricher) logger() *slog.Logger {
	if e.log != nil {
		return e.log
	}
	return slog.New(slog.DiscardHandler)
}

func (*Enricher) ID() string             { return EnricherID }
func (*Enricher) Capabilities() []string { return []string{contracts.CapMetadataEnrich} }

func (e *Enricher) Health() error {
	if e.Reg == nil || e.DB == nil {
		return &core.Error{Code: "dependency-unavailable", Msg: "enricher missing registry or store"}
	}
	return nil
}

// EnrichInput asks for one item's overlay. Only restricts the merge to
// one named provider; empty means the bound Auto set.
type EnrichInput struct {
	Item contracts.CatalogItem `json:"item"`
	Only string                `json:"only,omitempty"`
}

// GetInput reads the stored overlay for one item.
type GetInput struct {
	ItemID string `json:"item_id"`
}

// BatchInput reads the overlays that exist for a set of items in one
// transaction — a grid's page without N+1 lookups.
type BatchInput struct {
	IDs []string `json:"ids"`
}

// DeleteInput drops one item's overlay.
type DeleteInput struct {
	ItemID string `json:"item_id"`
}

// BackfillInput asks the provider to enrich every catalog item that
// lacks an overlay. Items enrich one at a time, so provider APIs see
// a sequential trickle, never a burst.
type BackfillInput struct{}

// BackfillOutput counts what the backfill pass stored.
type BackfillOutput struct {
	Enriched int `json:"enriched"`
}

// BatchOutput lists the stored overlays for the requested items;
// missing overlays are simply absent.
type BatchOutput struct {
	Items []contracts.Enrichment `json:"items"`
}

func (e *Enricher) Invoke(cap string, input any) (any, error) {
	if cap != contracts.CapMetadataEnrich {
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
	if err := e.Health(); err != nil {
		return nil, err
	}
	saver := NewSaver(e.DB)
	switch in := input.(type) {
	case EnrichInput:
		return e.enrich(in)
	case GetInput:
		en, ok := saver.Get(in.ItemID)
		if !ok {
			return nil, &core.Error{Code: "not-found", Msg: "not enriched"}
		}
		return en, nil
	case BatchInput:
		return BatchOutput{Items: saver.GetMany(in.IDs)}, nil
	case DeleteInput:
		if err := saver.Delete(in.ItemID); err != nil {
			return nil, err
		}
		return true, nil
	case BackfillInput:
		return e.backfill(saver)
	default:
		return nil, &core.Error{Code: "invalid-message", Msg: "enrich input required"}
	}
}

// enrich searches, resolves and saves one overlay.
func (e *Enricher) enrich(in EnrichInput) (contracts.Enrichment, error) {
	item := in.Item
	kind := "anime"
	if item.Kind != "" && item.Kind != "episode" && item.Kind != "video" {
		kind = item.Kind
	}
	merged, err := e.searchMetadata(item.Title, kind, dirOf(item.FilePath), in.Only)
	if err != nil {
		e.logger().Warn("enrich search failed", "item", item.ID, "title", item.Title, "err", err.Error())
		return contracts.Enrichment{}, err
	}
	best, ok := BestPick(merged)
	if !ok {
		e.logger().Warn("enrich no match", "item", item.ID, "title", item.Title, "kind", kind)
		return contracts.Enrichment{}, &core.Error{Code: "not-found", Msg: "no metadata found"}
	}
	rec, err := e.resolveMetadata(best.Provider, best.RemoteID)
	if err != nil {
		e.logger().Warn("enrich resolve failed", "item", item.ID, "title", item.Title,
			"provider", best.Provider, "remote_id", best.RemoteID, "err", err.Error())
		return contracts.Enrichment{}, err
	}
	saved, err := NewSaver(e.DB).Save(item.ID, rec)
	if err != nil {
		e.logger().Error("enrich save failed", "item", item.ID, "title", item.Title, "err", err.Error())
		return contracts.Enrichment{}, err
	}
	e.logger().Debug("enrich ok", "item", item.ID, "title", saved.Title, "provider", saved.Provider)
	return saved, nil
}

// backfill enriches every catalog item without an overlay; per-item
// failures are skipped — a down provider or a missing match must never
// fail the scan that just succeeded.
func (e *Enricher) backfill(saver *Saver) (BackfillOutput, error) {
	out, _, err := e.Reg.CallOne(contracts.CapCatalogRead, nil)
	if err != nil {
		return BackfillOutput{}, err
	}
	items, ok := out.([]contracts.CatalogItem)
	if !ok {
		return BackfillOutput{}, &core.Error{Code: "internal", Msg: "catalog read returned a bad shape"}
	}
	done := 0
	for _, item := range items {
		if _, ok := saver.Get(item.ID); ok {
			continue
		}
		if _, err := e.enrich(EnrichInput{Item: item}); err != nil {
			continue
		}
		done++
	}
	return BackfillOutput{Enriched: done}, nil
}

// searchMetadata serves from cache, else fans out and caches the
// merge. Empty merges are never cached: a down provider must not
// poison the next attempt.
func (e *Enricher) searchMetadata(query, kind, dir, only string) ([]contracts.MetadataCandidate, error) {
	cache := NewCache(e.DB)
	if only == "" {
		if hit, ok := cache.GetSearch(kind, query); ok {
			return hit, nil
		}
	}
	in := contracts.MetadataSearchInput{Query: query, Kind: kind, Limit: 5, Dir: dir}
	out, ids, skipped, err := e.Reg.CallMergeReport(contracts.CapMetadataSearch, in, func(outputs []any, ids []string) any {
		if only != "" {
			for i, id := range ids {
				if id == only {
					return MergeCandidates(query, outputs[i:i+1], ids[i:i+1], 10)
				}
			}
			return []contracts.MetadataCandidate{}
		}
		return MergeCandidates(query, outputs, ids, 10)
	})
	// Skipped providers degrade the merge silently by contract; the
	// causes live here at debug so a dead upstream is one grep away.
	if len(skipped) > 0 {
		causes := make([]string, 0, len(skipped))
		for _, f := range skipped {
			causes = append(causes, f.Provider+": "+f.Err.Error())
		}
		e.logger().Debug("metadata providers skipped", "query", query, "kind", kind, "causes", strings.Join(causes, "; "))
	}
	if err != nil {
		return nil, err
	}
	merged, _ := out.([]contracts.MetadataCandidate)
	e.logger().Debug("metadata search", "query", query, "kind", kind, "hits", len(merged), "from", strings.Join(ids, ","))
	if only == "" && len(merged) > 0 {
		_ = cache.PutSearch(kind, query, merged)
	}
	return merged, nil
}

// resolveMetadata serves from cache, else calls the winning provider
// directly (still through registry authority).
func (e *Enricher) resolveMetadata(provider, remoteID string) (contracts.MetadataRecord, error) {
	cache := NewCache(e.DB)
	if rec, ok := cache.GetRecord(provider, remoteID); ok {
		return rec, nil
	}
	out, err := e.Reg.InvokeProvider(contracts.CapMetadataResolve, provider, contracts.MetadataResolveInput{Provider: provider, RemoteID: remoteID})
	if err != nil {
		return contracts.MetadataRecord{}, err
	}
	rec, ok := out.(contracts.MetadataRecord)
	if !ok {
		return contracts.MetadataRecord{}, &core.Error{Code: "internal", Msg: "bad record shape from " + provider}
	}
	_ = cache.PutRecord(rec)
	return rec, nil
}
