package probe


import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/testutil/contract"
)

func TestProviderContract(t *testing.T) {
	// No happy-path sample: the real call execs ffprobe; ffprobe_test.go
	// covers that path through its own seams.
	contract.Run(t, Provider{}, []contract.Cap{{Name: contracts.CapMediaProbe}})
}
