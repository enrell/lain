package search


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
