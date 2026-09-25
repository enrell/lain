package metadata


import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/testutil/contract"
)

func TestEnricherContract(t *testing.T) {
	// A nil-DB enricher still answers every protocol check typed —
	// its Health reports the missing dependency.
	e := NewEnricher(core.NewRegistry(core.DefaultComposition()), nil)
	contract.Run(t, e, []contract.Cap{{
		Name:   contracts.CapMetadataEnrich,
		Sample: EnrichInput{Item: contracts.CatalogItem{ID: "i"}},
	}})
}
