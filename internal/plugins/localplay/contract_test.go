package localplay


import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
	engine "github.com/enrell/lain/internal/localplay"
	"github.com/enrell/lain/internal/testutil/contract"
)

func TestProviderContract(t *testing.T) {
	contract.Run(t, &Provider{Mgr: engine.New()}, []contract.Cap{{
		Name:   contracts.CapPlaybackLocal, NilOK: true,
		Sample: PlayInput{UserID: "u", Item: contracts.CatalogItem{ID: "i"}},
	}})
}
