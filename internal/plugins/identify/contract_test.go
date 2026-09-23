package identify


import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/testutil/contract"
)

func TestProviderContract(t *testing.T) {
	caps := []contract.Cap{{
		Name:   contracts.CapMediaIdentify,
		Sample: contracts.Candidate{Path: "Show - S01E02 [1080p].mkv", LibraryID: "l"},
	}}
	contract.Run(t, Anime{}, caps)
	contract.Run(t, Generic{}, caps)
}
