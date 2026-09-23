package userstate

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-23

// Fault-injection: a closed database must surface as an error or a
// miss — never a panic. Progress rows are precious; a store failure is
// an operator problem, not a crash.

import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/kv"
)

func TestClosedDBDegrades(t *testing.T) {
	db, err := kv.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	s, err := New(db)
	if err != nil {
		t.Fatal(err)
	}
	db.Close()

	if _, err := s.Put(PutInput{UserID: "u", Progress: contracts.Progress{ItemID: "i", PositionSec: 10}}); err == nil {
		t.Fatal("Put on closed db returned nil error")
	}
	if _, ok := s.Get("u", "i"); ok {
		t.Fatal("Get on closed db returned hit")
	}
	if got := s.List("u"); len(got) != 0 {
		t.Fatalf("List on closed db = %v", got)
	}
}
