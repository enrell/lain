package ingest


import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/testutil/contract"
)

func TestProviderContract(t *testing.T) {
	r, _ := testRunner(t)
	contract.Run(t, r, []contract.Cap{{
		Name:   contracts.CapIngestScan,
		Sample: ScanInput{Libraries: nil},
	}})
}
