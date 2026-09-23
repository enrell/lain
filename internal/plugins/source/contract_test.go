package source


import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/testutil/contract"
)

func TestProviderContract(t *testing.T) {
	contract.Run(t, Provider{}, []contract.Cap{{
		Name:   contracts.CapSourceEnumerate,
		Sample: EnumerateInput{Root: t.TempDir(), LibraryID: "l", Type: "anime"},
	}})
}
