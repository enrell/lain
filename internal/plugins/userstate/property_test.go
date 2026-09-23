package userstate


import (
	"strings"
	"testing"
	"testing/quick"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/kv"
)

// Property: the composite progress key is injective over NUL-free ids —
// a collision would leak one user's progress onto another item.
func TestPropertyKeyInjective(t *testing.T) {
	err := quick.Check(func(a, b, c, d string) bool {
		if strings.ContainsRune(a+b+c+d, '\x00') {
			return true
		}
		if string(key(a, b)) != string(key(c, d)) {
			return true
		}
		return a == c && b == d
	}, &quick.Config{MaxCount: 500})
	if err != nil {
		t.Error(err)
	}
}

// Property: Put then Get returns the stored progress for arbitrary
// positions — progress must survive the byte codec exactly.
func TestPropertyPutGetRoundTrip(t *testing.T) {
	db, err := kv.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	s, err := New(db)
	if err != nil {
		t.Fatal(err)
	}

	err = quick.Check(func(pos float64, itemTail uint8) bool {
		if pos != pos { // NaN survives nowhere
			return true
		}
		in := PutInput{
			UserID: "u1",
			Progress: contracts.Progress{
				ItemID:      "item",
				PositionSec: pos,
			},
		}
		if _, err := s.Put(in); err != nil {
			return false
		}
		got, _ := s.Get("u1", "item")
		return got.PositionSec == pos && got.ItemID == "item" && got.UserID == "u1"
	}, &quick.Config{MaxCount: 200})
	if err != nil {
		t.Error(err)
	}
}
