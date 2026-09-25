package sourcewatch


import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/testutil/contract"
)

func TestProviderContract(t *testing.T) {
	p := NewWatcherProvider()
	p.NoFS = true
	t.Cleanup(func() { _ = p.Close() })
	contract.Run(t, p, []contract.Cap{{
		Name:   contracts.CapSourceWatch,
		Sample: PollInput{},
	}})
}
