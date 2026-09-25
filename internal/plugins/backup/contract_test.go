package backup


import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/testutil/contract"
)

func TestProviderContract(t *testing.T) {
	contract.Run(t, Provider{}, []contract.Cap{{
		Name: contracts.CapBackupCreate,
		// A missing snapshot must fail typed, not panic.
		Sample: CreateInput{SnapshotPath: "/nonexistent/lain.db"},
	}})
}
