package theme


import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/testutil/contract"
)

func TestProviderContract(t *testing.T) {
	contract.Run(t, Provider{}, []contract.Cap{{
		Name:   contracts.CapUITheme, NilOK: true,
		Sample: Input{ColorsPath: ""},
	}})
}
