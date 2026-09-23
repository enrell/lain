package catalog


import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/testutil/contract"
)

func TestProviderContract(t *testing.T) {
	s, _ := testServiceIn(t, t.TempDir())
	contract.Run(t, s, []contract.Cap{
		// nil reads list everything — nil is a valid input here.
		{Name: contracts.CapCatalogRead, NilOK: true},
		{
			Name: contracts.CapCatalogWrite,
			Sample: UpsertInput{
				LibraryID: "l",
				Proposal:  contracts.Proposal{},
				Candidate: contracts.Candidate{Path: "/media/Show - S01E02.mkv"},
			},
		},
	})
}
