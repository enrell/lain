package userstate


import (
	"testing"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/kv"
)

func newService(t *testing.T) *Service {
	t.Helper()
	db, err := kv.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	s, err := New(db)
	if err != nil {
		t.Fatal(err)
	}
	return s
}

func TestNewRejectsNilDB(t *testing.T) {
	if _, err := New(nil); err == nil {
		t.Fatal("nil db must fail")
	} else if e, ok := err.(*core.Error); !ok || e.Code != "internal" {
		t.Fatalf("want internal error, got %v", err)
	}
}

// Put/Get roundtrip: the record gains the caller's user id and a fresh
// timestamp; a never-written key answers "absent", not an error.
func TestPutGetRoundtrip(t *testing.T) {
	s := newService(t)
	if _, ok := s.Get("u1", "i1"); ok {
		t.Fatal("Get on an empty store must report absent")
	}
	p, err := s.Put(PutInput{UserID: "u1", Progress: contracts.Progress{
		ItemID: "i1", PositionSec: 42, DurationSec: 1400,
	}})
	if err != nil {
		t.Fatal(err)
	}
	if p.UserID != "u1" || p.UpdatedAt <= 0 {
		t.Fatalf("Put must stamp user+time: %+v", p)
	}
	got, ok := s.Get("u1", "i1")
	if !ok || got.PositionSec != 42 || got.DurationSec != 1400 {
		t.Fatalf("Get: %+v ok=%v", got, ok)
	}
	// Same user+item rewrites in place; a different user is a different
	// record — the composite key must keep them apart.
	if _, err := s.Put(PutInput{UserID: "u1", Progress: contracts.Progress{ItemID: "i1", PositionSec: 99, Completed: true}}); err != nil {
		t.Fatal(err)
	}
	got, _ = s.Get("u1", "i1")
	if got.PositionSec != 99 || !got.Completed {
		t.Fatalf("overwrite: %+v", got)
	}
	if _, ok := s.Get("u2", "i1"); ok {
		t.Fatal("another user's key must stay absent")
	}
}

// List feeds continue-watching: all of one user's rows, none of
// another's, even when the user id is a strict prefix of the other.
func TestListScopesPerUser(t *testing.T) {
	s := newService(t)
	for _, in := range []PutInput{
		{UserID: "u", Progress: contracts.Progress{ItemID: "i1", PositionSec: 1}},
		{UserID: "u", Progress: contracts.Progress{ItemID: "i2", PositionSec: 2}},
		{UserID: "ux", Progress: contracts.Progress{ItemID: "i1", PositionSec: 3}},
	} {
		if _, err := s.Put(in); err != nil {
			t.Fatal(err)
		}
	}
	list := s.List("u")
	if len(list) != 2 {
		t.Fatalf("List(u)=%d rows, want 2", len(list))
	}
	seen := map[string]bool{}
	for _, p := range list {
		seen[p.ItemID] = true
		if p.UserID != "u" {
			t.Fatalf("List leaked another user's row: %+v", p)
		}
	}
	if !seen["i1"] || !seen["i2"] {
		t.Fatalf("List missing rows: %v", seen)
	}
	if got := s.List("nobody"); len(got) != 0 {
		t.Fatalf("List(nobody)=%v, want empty", got)
	}
}

// The prefix scan must include a row whose key is exactly the user
// prefix (empty item id) — the guard is >=, not >.
func TestListIncludesExactPrefixRow(t *testing.T) {
	s := newService(t)
	if _, err := s.Put(PutInput{UserID: "u", Progress: contracts.Progress{ItemID: "", PositionSec: 4}}); err != nil {
		t.Fatal(err)
	}
	list := s.List("u")
	if len(list) != 1 || list[0].PositionSec != 4 {
		t.Fatalf("exact-prefix row must be listed: %+v", list)
	}
}

// A corrupt row is skipped, not fatal: one bad record must not sink the
// feed or its neighbours.
func TestListSkipsCorruptRows(t *testing.T) {
	s := newService(t)
	if _, err := s.Put(PutInput{UserID: "u", Progress: contracts.Progress{ItemID: "good", PositionSec: 7}}); err != nil {
		t.Fatal(err)
	}
	err := s.db.Update(func(tx *bolt.Tx) error {
		return tx.Bucket(kv.BProgress).Put(key("u", "broken"), []byte("{not-json"))
	})
	if err != nil {
		t.Fatal(err)
	}
	list := s.List("u")
	if len(list) != 1 || list[0].ItemID != "good" {
		t.Fatalf("corrupt row must be skipped: %+v", list)
	}
}

func TestInvokeDispatch(t *testing.T) {
	s := newService(t)
	if s.ID() != ID {
		t.Fatalf("id=%s", s.ID())
	}
	caps := s.Capabilities()
	if len(caps) != 1 || caps[0] != contracts.CapUserProgress {
		t.Fatalf("caps=%v", caps)
	}
	if err := s.Health(); err != nil {
		t.Fatal(err)
	}
	if _, err := s.Invoke("lain.other@1", PutInput{}); err == nil {
		t.Fatal("unsupported cap must fail")
	}
	if _, err := s.Invoke(contracts.CapUserProgress, 42); err == nil {
		t.Fatal("unknown input type must fail")
	}
	out, err := s.Invoke(contracts.CapUserProgress, PutInput{
		UserID:   "u",
		Progress: contracts.Progress{ItemID: "i", PositionSec: 5},
	})
	if err != nil {
		t.Fatal(err)
	}
	if out.(contracts.Progress).PositionSec != 5 {
		t.Fatalf("put via invoke: %+v", out)
	}
	out, err = s.Invoke(contracts.CapUserProgress, GetInput{UserID: "u", ItemID: "i"})
	if err != nil {
		t.Fatal(err)
	}
	if out.(contracts.Progress).PositionSec != 5 {
		t.Fatalf("get via invoke: %+v", out)
	}
	// GetInput on an absent key yields the zero value, not an error.
	out, err = s.Invoke(contracts.CapUserProgress, GetInput{UserID: "u", ItemID: "absent"})
	if err != nil {
		t.Fatal(err)
	}
	if out.(contracts.Progress).ItemID != "" {
		t.Fatalf("absent get must be zero value: %+v", out)
	}
}
