// Package search answers lain.search.query@1 over the catalog snapshot
// handed in by the gateway. Indexing stays with the catalog; ranking
// policy lives here so a semantic-search plugin can replace this exact
// provider without touching storage.
package search

import (
	"sort"

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

// QueryInput narrows the search and pages it.
type QueryInput struct {
	Q      string `json:"q"`
	Kind   string `json:"kind"`
	Limit  int    `json:"limit"`
	Offset int    `json:"offset"`
	Sort   string `json:"sort"`
}

func (p Provider) Invoke(cap string, input any) (any, error) {
	if cap != contracts.CapSearchQuery {
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
	in, ok := input.(QueryInput)
	if !ok {
		return nil, &core.Error{Code: "invalid-message", Msg: "QueryInput required"}
	}
	return Query(p.Items(), in), nil
}

// Query filters case-insensitively by title substring (kind optional)
// and returns the requested page with the exact total.
func Query(all []contracts.CatalogItem, in QueryInput) contracts.CatalogPage {
	params := contracts.NormalizePage(in.Limit, in.Offset, in.Sort)
	var filtered []contracts.CatalogItem
	lq := lower(in.Q)
	lk := lower(in.Kind)
	for _, it := range all {
		if lk != "" && lower(it.Kind) != lk {
			continue
		}
		if lq != "" && !containsFold(it.Title, lq) {
			continue
		}
		filtered = append(filtered, it)
	}
	sortFiltered(filtered, params.Sort)
	total := len(filtered)
	if params.Offset < total {
		end := params.Offset + params.Limit
		if end > total {
			end = total
		}
		filtered = filtered[params.Offset:end]
	} else {
		filtered = []contracts.CatalogItem{}
	}
	if filtered == nil {
		filtered = []contracts.CatalogItem{}
	}
	return contracts.CatalogPage{Items: filtered, Total: total, Limit: params.Limit, Offset: params.Offset}
}

func sortFiltered(items []contracts.CatalogItem, order string) {
	if order == "recent" {
		sort.Slice(items, func(i, j int) bool {
			if items[i].UpdatedAt == items[j].UpdatedAt {
				return items[i].ID < items[j].ID
			}
			return items[i].UpdatedAt > items[j].UpdatedAt
		})
		return
	}
	sort.Slice(items, func(i, j int) bool {
		if items[i].Title == items[j].Title {
			return items[i].ID < items[j].ID
		}
		return items[i].Title < items[j].Title
	})
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
