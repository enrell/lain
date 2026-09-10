// Package search answers lain.search.query@1 over the catalog snapshot
// handed in by the gateway. Indexing stays with the catalog; ranking
// policy lives here so a semantic-search plugin can replace this exact
// provider without touching storage.
package search

import (
	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

// ID is the built-in simple-search provider id.
const ID = "lain-search-simple"

// Provider serves substring search.
type Provider struct {
	// Items is refreshed by the gateway per call in v0.1 (snapshot in,
	// ranked list out). A future index-backed provider keeps its own.
	Items func() []contracts.CatalogItem
}

func (p Provider) ID() string             { return ID }
func (p Provider) Capabilities() []string { return []string{contracts.CapSearchQuery} }
func (p Provider) Health() error          { return nil }

// QueryInput narrows the search.
type QueryInput struct {
	Q    string `json:"q"`
	Kind string `json:"kind"`
}

func (p Provider) Invoke(cap string, input any) (any, error) {
	if cap != contracts.CapSearchQuery {
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
	in, ok := input.(QueryInput)
	if !ok {
		return nil, &core.Error{Code: "invalid-message", Msg: "QueryInput required"}
	}
	return Query(p.Items(), in.Q, in.Kind), nil
}

// Query filters case-insensitively by title substring (kind optional).
func Query(items []contracts.CatalogItem, q, kind string) []contracts.CatalogItem {
	var filtered []contracts.CatalogItem
	lq := lower(q)
	lk := lower(kind)
	for _, it := range items {
		if lk != "" && lower(it.Kind) != lk {
			continue
		}
		if lq != "" && !containsFold(it.Title, lq) {
			continue
		}
		filtered = append(filtered, it)
	}
	if filtered == nil {
		filtered = []contracts.CatalogItem{}
	}
	return filtered
}

func lower(s string) string {
	b := []byte(s)
	for i, c := range b {
		if c >= 'A' && c <= 'Z' {
			b[i] = c + 32
		}
	}
	return string(b)
}

func containsFold(title, lq string) bool {
	lt := lower(title)
	if len(lq) > len(lt) {
		return false
	}
	for i := 0; i+len(lq) <= len(lt); i++ {
		if lt[i:i+len(lq)] == lq {
			return true
		}
	}
	return false
}
