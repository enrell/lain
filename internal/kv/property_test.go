package kv

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-23

import (
	"reflect"
	"testing"
	"testing/quick"

	bolt "go.etcd.io/bbolt"
)

// Property: PutJSON/GetJSON round-trips arbitrary JSON values through
// bolt untouched — the composite codecs above this layer depend on it.
func TestPropertyPutGetRoundTrip(t *testing.T) {
	db, err := Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })

	roundTrip := func(v any, into func() any) bool {
		key := []byte("k")
		if err := db.Update(func(tx *bolt.Tx) error {
			return PutJSON(tx, BCache, key, v)
		}); err != nil {
			return false
		}
		out := into()
		if err := db.View(func(tx *bolt.Tx) error {
			return GetJSON(tx, BCache, key, out)
		}); err != nil {
			return false
		}
		return reflect.DeepEqual(reflect.ValueOf(out).Elem().Interface(), v)
	}

	cfg := &quick.Config{MaxCount: 200}
	if err := quick.Check(func(v map[string]string) bool {
		return roundTrip(v, func() any { return &map[string]string{} })
	}, cfg); err != nil {
		t.Error(err)
	}
	if err := quick.Check(func(v []int64) bool {
		return roundTrip(v, func() any { return &[]int64{} })
	}, cfg); err != nil {
		t.Error(err)
	}
	if err := quick.Check(func(v string) bool {
		return roundTrip(v, func() any { return new(string) })
	}, cfg); err != nil {
		t.Error(err)
	}
}

// Property: a key written under one name never reads back under
// another — keys are verbatim byte strings, no escaping tricks.
func TestPropertyKeyIsolation(t *testing.T) {
	db, err := Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })

	err = quick.Check(func(a, b string) bool {
		ka, kb := []byte("a\x00"+a), []byte("b\x00"+b)
		if err := db.Update(func(tx *bolt.Tx) error {
			if err := PutJSON(tx, BCache, ka, "A"); err != nil {
				return err
			}
			return PutJSON(tx, BCache, kb, "B")
		}); err != nil {
			return false
		}
		var got string
		if err := db.View(func(tx *bolt.Tx) error {
			return GetJSON(tx, BCache, ka, &got)
		}); err != nil {
			return false
		}
		return got == "A"
	}, &quick.Config{MaxCount: 100})
	if err != nil {
		t.Error(err)
	}
}
