package metadata


import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/testutil/contract"
)

// The remote providers get no samples: a happy-path call would dial the
// real API. Their search/resolve paths are covered by the fake-transport
// tests in this package; the contract here is protocol behaviour —
// typed errors and no panics.
func TestProviderContract(t *testing.T) {
	caps := []contract.Cap{
		{Name: contracts.CapMetadataSearch},
		{Name: contracts.CapMetadataResolve},
	}
	contract.Run(t, NewKitsu(), caps)
	contract.Run(t, NewAniList(), caps)
	contract.Run(t, NewJikan(), caps)
	contract.Run(t, NewTVMaze(), caps)

	// NFO is local-only: a search over a temp dir is a safe happy path.
	contract.Run(t, NFO{}, []contract.Cap{
		{Name: contracts.CapMetadataSearch, Sample: contracts.MetadataSearchInput{
			Query: "frieren", Dir: t.TempDir(),
		}},
		{Name: contracts.CapMetadataResolve},
	})
}
