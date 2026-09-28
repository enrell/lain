package settings

import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/kv"
)

func TestIntegrationsRoundTrip(t *testing.T) {
	db, err := kv.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	p := Provider{DB: db}

	out, err := p.Invoke(contracts.CapIntegrationSettings, nil)
	if err != nil {
		t.Fatal(err)
	}
	if got := out.(contracts.IntegrationSettings); got.AniListClientID != "" {
		t.Fatalf("fresh store not empty: %+v", got)
	}

	in := contracts.IntegrationSettings{AniListClientID: "42", AniListClientSecret: "shh"}
	if _, err := p.Invoke(contracts.CapIntegrationSettings, IntPutInput{Settings: in}); err != nil {
		t.Fatal(err)
	}
	out, _ = p.Invoke(contracts.CapIntegrationSettings, nil)
	if got := out.(contracts.IntegrationSettings); got != in {
		t.Fatalf("stored %+v, want %+v", got, in)
	}

	// Ensure never overwrites an explicit save.
	other := contracts.IntegrationSettings{AniListClientID: "99"}
	res, err := p.Invoke(contracts.CapIntegrationSettings, IntEnsureInput{Base: other})
	if err != nil {
		t.Fatal(err)
	}
	if res.(HasOutput).Saved != true {
		t.Fatalf("ensure claimed a fresh write")
	}
	out, _ = p.Invoke(contracts.CapIntegrationSettings, nil)
	if got := out.(contracts.IntegrationSettings); got.AniListClientID != "42" {
		t.Fatalf("ensure clobbered save: %+v", got)
	}
}

func TestIntegrationsEnsureSeeds(t *testing.T) {
	db, err := kv.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	p := Provider{DB: db}
	res, err := p.Invoke(contracts.CapIntegrationSettings, IntEnsureInput{
		Base: contracts.IntegrationSettings{AniListClientID: "7", AniListClientSecret: "s"},
	})
	if err != nil {
		t.Fatal(err)
	}
	if res.(HasOutput).Saved != false {
		t.Fatalf("ensure should have written")
	}
	out, _ := p.Invoke(contracts.CapIntegrationSettings, nil)
	if got := out.(contracts.IntegrationSettings); got.AniListClientID != "7" {
		t.Fatalf("seeded %+v", got)
	}
}
