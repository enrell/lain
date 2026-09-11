// Package catalog is the authoritative item store (exactly-one).
// Bolt-backed: one read-write transaction per mutation, prefix scans
// for library scoping. The portable ExportDoc stays the replacement
// contract for any future catalog provider.
package catalog

import (
	"crypto/sha1"
	"encoding/hex"
	"sort"
	"strconv"
	"strings"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/kv"
)

// ID is the bolt catalog provider id.
const ID = "lain-catalog-bolt"

// libSep separates library and item in composite keys.
const libSep = "\x00"

// Service owns canonical media identity for its scope.
type Service struct {
	db *bolt.DB
}

func New(db *bolt.DB) (*Service, error) {
	if db == nil {
		return nil, &core.Error{Code: "internal", Msg: "nil db"}
	}
	return &Service{db: db}, nil
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

// ItemID derives a stable id from library + path. Fresh items use it as
// their identity; ReconcileMoves later re-homes the ID onto a new path
// when a move or rename is detected, so IDs stay stable (D-019).
func ItemID(libraryID, path string) string {
	h := sha1.Sum([]byte(libraryID + "\x00" + path))
	return hex.EncodeToString(h[:])[:16]
}

// Fingerprint is the move/rename-proof logical key of an item: kind plus
// normalized title, season, episode, and year. Years match when either
// side is unknown (0), because identifiers often fill the year in later.
func Fingerprint(libraryID, kind, title string, season, episode, year int) string {
	norm := strings.ToLower(strings.Join(strings.Fields(title), " "))
	return libraryID + "\x00" + kind + "\x00" + norm +
		"\x00" + strconv.Itoa(season) + "\x00" + strconv.Itoa(episode) +
		"\x00" + strconv.Itoa(year)
}

func fingerprintOf(it contracts.CatalogItem) string {
	return Fingerprint(it.LibraryID, it.Kind, it.Title, it.Season, it.Episode, it.Year)
}

func sameLogical(a, b contracts.CatalogItem) bool {
	if a.LibraryID != b.LibraryID || a.Kind != b.Kind {
		return false
	}
	na := strings.ToLower(strings.Join(strings.Fields(a.Title), " "))
	nb := strings.ToLower(strings.Join(strings.Fields(b.Title), " "))
	if na != nb || a.Season != b.Season || a.Episode != b.Episode {
		return false
	}
	return a.Year == 0 || b.Year == 0 || a.Year == b.Year
}

// ReconcileMoves re-homes stable IDs onto moved or renamed files (D-019).
// For every scanned item whose path-derived ID is unknown, it looks for an
// existing item in the same library with the same logical fingerprint
// whose ID is absent from present (the old path is gone from this scan).
// Matches adopt the existing ID and record the previous path in Aliases;
// present is updated so pruning keeps the adopted record. Genuine
// duplicates (both paths present) and cross-library titles never merge.
// It returns the rewritten items and the migration count.
func (s *Service) ReconcileMoves(libraryID string, items []contracts.CatalogItem, present map[string]bool) ([]contracts.CatalogItem, int) {
	if len(items) == 0 {
		return items, 0
	}
	existing := s.ListByLibrary(libraryID)
	byID := make(map[string]contracts.CatalogItem, len(existing))
	for _, it := range existing {
		byID[it.ID] = it
	}
	migrated := 0
	for i, it := range items {
		if old, ok := byID[it.ID]; ok && old.FilePath == it.FilePath {
			continue // refresh of a known path: identity already stable
		}
		var match *contracts.CatalogItem
		for _, cand := range existing {
			if cand.ID == it.ID || present[cand.ID] {
				continue
			}
			if !sameLogical(cand, it) {
				continue
			}
			c := cand
			if match == nil || c.UpdatedAt < match.UpdatedAt {
				match = &c
			}
		}
		if match == nil {
			continue
		}
		if match.FilePath != it.FilePath && !hasAlias(it.Aliases, match.FilePath) {
			it.Aliases = append(it.Aliases, match.FilePath)
			if len(it.Aliases) > 8 {
				it.Aliases = it.Aliases[len(it.Aliases)-8:]
			}
		}
		for _, a := range match.Aliases {
			if !hasAlias(it.Aliases, a) {
				it.Aliases = append(it.Aliases, a)
			}
		}
		delete(present, it.ID)
		it.ID = match.ID
		present[it.ID] = true
		items[i] = it
		migrated++
	}
	return items, migrated
}

func hasAlias(aliases []string, path string) bool {
	for _, a := range aliases {
		if a == path {
			return true
		}
	}
	return false
}

// NewItem builds the canonical item for a proposal without touching
// disk. Pure: same input always yields the same identity.
func NewItem(libraryID string, p contracts.Proposal, c contracts.Candidate) contracts.CatalogItem {
	return contracts.CatalogItem{
		ID: idFor(libraryID, c.Path), LibraryID: libraryID, Kind: p.Kind,
		Title: p.Title, Season: p.Season,
		Episode: p.Episode, Year: p.Year,
		FilePath: c.Path, Size: c.Size,
		Confidence: p.Confidence, Origin: p.PluginID,
		Provenance: "identify:" + p.PluginID,
		UpdatedAt:  nowUnix(),
	}
}

func idFor(libraryID, path string) string { return ItemID(libraryID, path) }

func libKey(libraryID, itemID string) []byte {
	return []byte(libraryID + libSep + itemID)
}

func libPrefix(libraryID string) []byte {
	return []byte(libraryID + libSep)
}

// Upsert inserts or refreshes one item.
func (s *Service) Upsert(in UpsertInput) (contracts.CatalogItem, error) {
	it := NewItem(in.LibraryID, in.Proposal, in.Candidate)
	err := s.db.Update(func(tx *bolt.Tx) error {
		if err := kv.PutJSON(tx, kv.BItems, []byte(it.ID), it); err != nil {
			return err
		}
		return tx.Bucket(kv.BItemsByLib).Put(libKey(it.LibraryID, it.ID), []byte{})
	})
	if err != nil {
		return contracts.CatalogItem{}, err
	}
	return it, nil
}

// UpsertBatch stages many items in one transaction. Scans use this:
// disk writes stay constant no matter the library size.
func (s *Service) UpsertBatch(items []contracts.CatalogItem) error {
	if len(items) == 0 {
		return nil
	}
	return s.db.Update(func(tx *bolt.Tx) error {
		ib, lb := tx.Bucket(kv.BItems), tx.Bucket(kv.BItemsByLib)
		for _, it := range items {
			raw, err := marshalItem(it)
			if err != nil {
				return err
			}
			if err := ib.Put([]byte(it.ID), raw); err != nil {
				return err
			}
			if err := lb.Put(libKey(it.LibraryID, it.ID), []byte{}); err != nil {
				return err
			}
		}
		return nil
	})
}

// PruneMissing removes items of one library that the scan did not see.
// The caller must guarantee the root was fully walked: a partial walk
// must never read as deletions.
func (s *Service) PruneMissing(libraryID string, present map[string]bool) (int, error) {
	removed := 0
	err := s.db.Update(func(tx *bolt.Tx) error {
		ib, lb := tx.Bucket(kv.BItems), tx.Bucket(kv.BItemsByLib)
		cur := lb.Cursor()
		prefix := libPrefix(libraryID)
		for k, _ := cur.Seek(prefix); k != nil && hasPrefix(k, prefix); k, _ = cur.Next() {
			id := string(k[len(prefix):])
			if present[id] {
				continue
			}
			if err := ib.Delete([]byte(id)); err != nil {
				return err
			}
			if err := cur.Delete(); err != nil {
				return err
			}
			removed++
		}
		return nil
	})
	return removed, err
}

// Get returns one item.
func (s *Service) Get(id string) (contracts.CatalogItem, bool) {
	var it contracts.CatalogItem
	err := s.db.View(func(tx *bolt.Tx) error {
		return kv.GetJSON(tx, kv.BItems, []byte(id), &it)
	})
	if err != nil {
		return contracts.CatalogItem{}, false
	}
	return it, true
}

// List returns all items sorted by title.
func (s *Service) List() []contracts.CatalogItem {
	items, _ := s.Query("", "", contracts.PageParams{Limit: -1, Sort: "title"})
	return items
}

// Page returns one title- or recency-sorted slice with the total.
// Limit < 0 means unbounded (internal callers: export, search index).
func (s *Service) Page(p contracts.PageParams) (contracts.CatalogPage, error) {
	items, total := s.Query("", "", p)
	return contracts.CatalogPage{Items: items, Total: total, Limit: p.Limit, Offset: p.Offset}, nil
}

// Search is the catalog-side substring match (kind filter optional).
// Kept for internal callers; the gateway serves Query with paging.
func (s *Service) Search(q, kind string) []contracts.CatalogItem {
	items, _ := s.Query(q, kind, contracts.PageParams{Limit: -1, Sort: "title"})
	return items
}

// Query filters by library/substring/kind then pages. The filter is a
// single cursor walk (no FTS index yet); paging slices after filtering
// so the total stays exact.
func (s *Service) Query(q, kind string, p contracts.PageParams) ([]contracts.CatalogItem, int) {
	ql := strings.ToLower(strings.TrimSpace(q))
	kl := strings.ToLower(strings.TrimSpace(kind))
	ll := strings.TrimSpace(p.LibraryID)
	var out []contracts.CatalogItem
	_ = s.db.View(func(tx *bolt.Tx) error {
		return tx.Bucket(kv.BItems).ForEach(func(_, v []byte) error {
			var it contracts.CatalogItem
			if err := unmarshalItem(v, &it); err != nil {
				return nil
			}
			if ll != "" && it.LibraryID != ll {
				return nil
			}
			if kl != "" && strings.ToLower(it.Kind) != kl {
				return nil
			}
			if ql != "" && !strings.Contains(strings.ToLower(it.Title), ql) {
				return nil
			}
			out = append(out, it)
			return nil
		})
	})
	sortItems(out, p.Sort)
	total := len(out)
	if p.Limit >= 0 {
		if p.Offset >= total {
			return []contracts.CatalogItem{}, total
		}
		end := p.Offset + p.Limit
		if end > total {
			end = total
		}
		out = out[p.Offset:end]
	}
	if out == nil {
		out = []contracts.CatalogItem{}
	}
	return out, total
}

func sortItems(out []contracts.CatalogItem, order string) {
	if order == "recent" {
		sort.Slice(out, func(i, j int) bool {
			if out[i].UpdatedAt == out[j].UpdatedAt {
				return out[i].ID < out[j].ID
			}
			return out[i].UpdatedAt > out[j].UpdatedAt
		})
		return
	}
	sort.Slice(out, func(i, j int) bool {
		if out[i].Title == out[j].Title {
			return out[i].ID < out[j].ID
		}
		return out[i].Title < out[j].Title
	})
}

// ListByLibrary returns one library's items sorted by title.
func (s *Service) ListByLibrary(libraryID string) []contracts.CatalogItem {
	var out []contracts.CatalogItem
	_ = s.db.View(func(tx *bolt.Tx) error {
		ib, lb := tx.Bucket(kv.BItems), tx.Bucket(kv.BItemsByLib)
		cur := lb.Cursor()
		prefix := libPrefix(libraryID)
		for k, _ := cur.Seek(prefix); k != nil && hasPrefix(k, prefix); k, _ = cur.Next() {
			var it contracts.CatalogItem
			if err := kv.GetJSON(tx, kv.BItems, []byte(string(k[len(prefix):])), &it); err != nil {
				continue
			}
			_ = ib
			out = append(out, it)
		}
		return nil
	})
	sort.Slice(out, func(i, j int) bool {
		if out[i].Title == out[j].Title {
			return out[i].ID < out[j].ID
		}
		return out[i].Title < out[j].Title
	})
	if out == nil {
		out = []contracts.CatalogItem{}
	}
	return out
}

// Count returns the total item count without materializing records.
func (s *Service) Count() int {
	n := 0
	_ = s.db.View(func(tx *bolt.Tx) error {
		n = tx.Bucket(kv.BItems).Stats().KeyN
		return nil
	})
	return n
}

// Export dumps the portable document a replacement catalog must honor.
// Version 2 adds per-item aliases (identity v2, D-019).
func (s *Service) Export() ExportDoc {
	doc := ExportDoc{Format: "lain.catalog-export", Version: 2, Items: map[string]contracts.CatalogItem{}}
	for _, it := range s.List() {
		doc.Items[it.ID] = it
	}
	return doc
}

// Import loads a portable document, replacing contents atomically in
// one transaction (index rebuilt with the items). Versions 1 (no
// aliases) and 2 (identity v2 aliases) are accepted.
func (s *Service) Import(doc ExportDoc) error {
	if doc.Format != "lain.catalog-export" || (doc.Version != 1 && doc.Version != 2) {
		return &core.Error{Code: "invalid-message", Msg: "unsupported export format/version"}
	}
	if doc.Items == nil {
		doc.Items = map[string]contracts.CatalogItem{}
	}
	return s.db.Update(func(tx *bolt.Tx) error {
		ib, lb := tx.Bucket(kv.BItems), tx.Bucket(kv.BItemsByLib)
		cur := lb.Cursor()
		for k, _ := cur.First(); k != nil; k, _ = cur.Next() {
			if err := cur.Delete(); err != nil {
				return err
			}
		}
		cur2 := ib.Cursor()
		for k, _ := cur2.First(); k != nil; k, _ = cur2.Next() {
			if err := cur2.Delete(); err != nil {
				return err
			}
		}
		for id, it := range doc.Items {
			it.ID = id
			raw, err := marshalItem(it)
			if err != nil {
				return err
			}
			if err := ib.Put([]byte(id), raw); err != nil {
				return err
			}
			if err := lb.Put(libKey(it.LibraryID, id), []byte{}); err != nil {
				return err
			}
		}
		return nil
	})
}
