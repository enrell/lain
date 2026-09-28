package list

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
	s, err := New(db)
	if err != nil {
		t.Fatal(err)
	}
	contract.Run(t, s, []contract.Cap{
		{Name: contracts.CapListRead, Sample: ListInput{UserID: "user-1"}},
		{Name: contracts.CapListWrite, Sample: PutPlatformInput{UserID: "user-1", Platform: "anilist", Entries: []contracts.ListEntry{{RemoteID: "1", Title: "Frieren"}}}},
		{Name: contracts.CapListAccount, Sample: PutAccountInput{Account: contracts.LinkedAccount{UserID: "user-1", Platform: "anilist", RemoteUserID: "7", RemoteUsername: "lain"}}},
	})
}
