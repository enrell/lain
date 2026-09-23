package search

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-23

import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/testutil/contract"
)

func TestProviderContract(t *testing.T) {
	p := Provider{Items: func() []contracts.CatalogItem { return nil }}
	contract.Run(t, p, []contract.Cap{{
		Name:   contracts.CapSearchQuery,
		Sample: QueryInput{Q: "frieren"},
	}})
}
