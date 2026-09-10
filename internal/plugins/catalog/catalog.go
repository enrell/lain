// Package catalog is the authoritative item store (exactly-one).
// The file-backed implementation persists catalog.json atomically with
// per-field provenance. A future sqlite-catalog plugin can replace it
// behind the same caps as long as it honors export/import.
package catalog

import (
	"crypto/sha1"
	"encoding/hex"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/store"
)

// ID is the built-in file catalog provider id.
const ID = "lain-catalog-file"

// Service owns canonical media identity for its scope.
type Service struct {
	mu    sync.RWMutex
	st    *store.Dir
	items map[string]contracts.CatalogItem
}

func New(st *store.Dir) (*Service, error) {
	s := &Service{st: st, items: map[string]contracts.CatalogItem{}}
	var loaded map[string]contracts.CatalogItem
	if err := st.Load("catalog.json", &loaded); err != nil {
		if err == store.ErrNotFound {
			return s, nil
		}
		return nil, err
	}
	s.items = loaded
	return s, nil
}

func (s *Service) ID() string { return ID }
func (s *Service) Capabilities() []string {
	return []string{contracts.CapCatalogRead, contracts.CapCatalogWrite}
}
func (s *Service) Health() error { return nil }

// UpsertInput is the write call shape.
type UpsertInput struct {
	LibraryID string              `json:"library_id"`
	Proposal  contracts.Proposal  `json:"proposal"`
	Candidate contracts.Candidate `json:"candidate"`
}

// ExportDoc is the versioned portable dump (import/export contract).
type ExportDoc struct {
	Format  string                           `json:"format"`
	Version int                              `json:"version"`
	Items   map[string]contracts.CatalogItem `json:"items"`
}

func (s *Service) Invoke(cap string, input any) (any, error) {
	switch cap {
	case contracts.CapCatalogWrite:
		in, ok := input.(UpsertInput)
		if !ok {
			return nil, &core.Error{Code: "invalid-message", Msg: "UpsertInput required"}
		}
		return s.Upsert(in)
	case contracts.CapCatalogRead:
		switch in := input.(type) {
		case nil:
			return s.List(), nil
		case GetInput:
			it, ok := s.Get(in.ID)
			if !ok {
				return nil, &core.Error{Code: "invalid-message", Msg: "unknown item " + in.ID}
			}
			return it, nil
		default:
			return nil, &core.Error{Code: "invalid-message", Msg: "nil or GetInput required"}
		}
	default:
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
}

// GetInput reads one item by id.
type GetInput struct {
	ID string `json:"id"`
}

// ItemID derives a stable id from library + path.
func ItemID(libraryID, path string) string {
	h := sha1.Sum([]byte(libraryID + "\x00" + path))
	return hex.EncodeToString(h[:])[:16]
}

// NewItem builds the canonical item for a proposal without touching
// disk. Pure: same input always yields the same identity.
func NewItem(libraryID string, p contracts.Proposal, c contracts.Candidate) contracts.CatalogItem {
	return contracts.CatalogItem{
		ID: ItemID(libraryID, c.Path), LibraryID: libraryID, Kind: p.Kind,
		Title: p.Title, Season: p.Season,
		Episode: p.Episode, Year: p.Year,
		FilePath: c.Path, Size: c.Size,
		Confidence: p.Confidence, Origin: p.PluginID,
		Provenance: "identify:" + p.PluginID,
		UpdatedAt:  time.Now().Unix(),
	}
}

// Upsert inserts or refreshes the item for a candidate. Provenance
// records which plugin produced the consolidated fields.
func (s *Service) Upsert(in UpsertInput) (contracts.CatalogItem, error) {
	it := NewItem(in.LibraryID, in.Proposal, in.Candidate)
	s.mu.Lock()
	s.items[it.ID] = it
	cp := copyAll(s.items)
	s.mu.Unlock()
	if err := s.st.Save("catalog.json", cp); err != nil {
		return contracts.CatalogItem{}, err
	}
	return it, nil
}

// UpsertBatch stages many items in memory and persists exactly once.
// Scans use this: disk writes stay constant no matter the library size.
func (s *Service) UpsertBatch(items []contracts.CatalogItem) error {
	if len(items) == 0 {
		return nil
	}
	s.mu.Lock()
	for _, it := range items {
		s.items[it.ID] = it
	}
	cp := copyAll(s.items)
	s.mu.Unlock()
	return s.st.Save("catalog.json", cp)
}

// PruneMissing removes items of one library that the scan did not see.
// The caller must guarantee the root was fully walked (accessible and
// zero walk errors): a partial walk must never read as deletions.
// Persists only when something was actually removed.
func (s *Service) PruneMissing(libraryID string, present map[string]bool) (int, error) {
	s.mu.Lock()
	removed := 0
	for id, it := range s.items {
		if it.LibraryID != libraryID {
			continue
		}
		if !present[id] {
			delete(s.items, id)
			removed++
		}
	}
	if removed == 0 {
		s.mu.Unlock()
		return 0, nil
	}
	cp := copyAll(s.items)
	s.mu.Unlock()
	if err := s.st.Save("catalog.json", cp); err != nil {
		return 0, err
	}
	return removed, nil
}

// Get returns one item.
func (s *Service) Get(id string) (contracts.CatalogItem, bool) {
	s.mu.RLock()
	defer s.mu.RUnlock()
	it, ok := s.items[id]
	return it, ok
}

// List returns all items sorted by title.
func (s *Service) List() []contracts.CatalogItem {
	s.mu.RLock()
	defer s.mu.RUnlock()
	out := make([]contracts.CatalogItem, 0, len(s.items))
	for _, it := range s.items {
		out = append(out, it)
	}
	sort.Slice(out, func(i, j int) bool {
		if out[i].Title == out[j].Title {
			return out[i].ID < out[j].ID
		}
		return out[i].Title < out[j].Title
	})
	return out
}

// Search is the catalog-side substring match (kind filter optional).
func (s *Service) Search(q, kind string) []contracts.CatalogItem {
	q = strings.ToLower(strings.TrimSpace(q))
	kind = strings.ToLower(strings.TrimSpace(kind))
	var out []contracts.CatalogItem
	for _, it := range s.List() {
		if kind != "" && strings.ToLower(it.Kind) != kind {
			continue
		}
		if q != "" && !strings.Contains(strings.ToLower(it.Title), q) {
			continue
		}
		out = append(out, it)
	}
	return out
}

// Export dumps the portable document a replacement catalog must honor.
func (s *Service) Export() ExportDoc {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return ExportDoc{Format: "lain.catalog-export", Version: 1, Items: copyAll(s.items)}
}

// Import loads a portable document, replacing contents atomically.
func (s *Service) Import(doc ExportDoc) error {
	if doc.Format != "lain.catalog-export" || doc.Version != 1 {
		return &core.Error{Code: "invalid-message", Msg: "unsupported export format/version"}
	}
	if doc.Items == nil {
		doc.Items = map[string]contracts.CatalogItem{}
	}
	s.mu.Lock()
	s.items = doc.Items
	cp := copyAll(s.items)
	s.mu.Unlock()
	return s.st.Save("catalog.json", cp)
}

func copyAll(m map[string]contracts.CatalogItem) map[string]contracts.CatalogItem {
	cp := make(map[string]contracts.CatalogItem, len(m))
	for k, v := range m {
		cp[k] = v
	}
	return cp
}
