package source

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-23

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
