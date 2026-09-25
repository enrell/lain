package settings


import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/kv"
	"github.com/enrell/lain/internal/testutil/contract"
)

func TestProviderContract(t *testing.T) {
	db, err := kv.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	contract.Run(t, Provider{DB: db}, []contract.Cap{{
		Name:   contracts.CapTranscodeSettings, NilOK: true,
		Sample: PutInput{Settings: contracts.DefaultTranscodeSettings()},
	}})
}
