package kv

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-22

import (
	"errors"
	"os"
	"path/filepath"
	"testing"

	bolt "go.etcd.io/bbolt"
)

func TestOpenCreatesBucketsAndFilePerms(t *testing.T) {
	dir := t.TempDir()
	db, err := Open(dir)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	if err := db.View(func(tx *bolt.Tx) error {
		for _, b := range [][]byte{BUsers, BUsersByName, BLibraries, BItems, BItemsByLib, BProgress, BMeta, BEnrich, BCache} {
			if tx.Bucket(b) == nil {
				t.Fatalf("missing bucket %s", b)
			}
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	fi, err := os.Stat(filepath.Join(dir, "lain.db"))
	if err != nil {
		t.Fatal(err)
	}
	if fi.Mode().Perm() != 0o600 {
		t.Fatalf("db perms %o, want 600", fi.Mode().Perm())
	}
}

func TestOpenFailsOnBadDir(t *testing.T) {
	// A file where the directory should be makes MkdirAll fail.
	file := filepath.Join(t.TempDir(), "not-a-dir")
	if err := os.WriteFile(file, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, err := Open(filepath.Join(file, "data")); err == nil {
		t.Fatal("Open under a file path must fail")
	}
}

func TestPutGetJSONRoundtrip(t *testing.T) {
	db, err := Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	type rec struct {
		Name string `json:"name"`
		N    int    `json:"n"`
	}
	if err := db.Update(func(tx *bolt.Tx) error {
		return PutJSON(tx, BMeta, []byte("k"), rec{Name: "v", N: 7})
	}); err != nil {
		t.Fatal(err)
	}
	var got rec
	if err := db.View(func(tx *bolt.Tx) error {
		return GetJSON(tx, BMeta, []byte("k"), &got)
	}); err != nil {
		t.Fatal(err)
	}
	if got.Name != "v" || got.N != 7 {
		t.Fatalf("roundtrip: %+v", got)
	}
}

func TestGetJSONMissingIsNotFound(t *testing.T) {
	db, err := Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	err = db.View(func(tx *bolt.Tx) error {
		var v any
		return GetJSON(tx, BMeta, []byte("absent"), &v)
	})
	if !IsNotFound(err) {
		t.Fatalf("missing key: want IsNotFound, got %v", err)
	}
	if IsNotFound(errors.New("other")) {
		t.Fatal("unrelated error must not be IsNotFound")
	}
	if IsNotFound(nil) {
		t.Fatal("nil must not be IsNotFound")
	}
	if got := ErrNotFound.Error(); got != "not found: key" {
		t.Fatalf("ErrNotFound text: %q", got)
	}
}

// A corrupt record is reported with bucket+key context — never a bare
// unmarshal error and never mistaken for absence.
func TestGetJSONCorruptIsNamed(t *testing.T) {
	db, err := Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	if err := db.Update(func(tx *bolt.Tx) error {
		return tx.Bucket(BMeta).Put([]byte("bad"), []byte("{broken"))
	}); err != nil {
		t.Fatal(err)
	}
	err = db.View(func(tx *bolt.Tx) error {
		var v any
		return GetJSON(tx, BMeta, []byte("bad"), &v)
	})
	if err == nil || IsNotFound(err) {
		t.Fatalf("corrupt record: %v", err)
	}
	if got := err.Error(); !contains(got, "meta") || !contains(got, "bad") {
		t.Fatalf("corrupt error must name bucket+key: %q", got)
	}
}

func TestPutJSONMarshalFailure(t *testing.T) {
	db, err := Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	err = db.Update(func(tx *bolt.Tx) error {
		return PutJSON(tx, BMeta, []byte("k"), make(chan int))
	})
	if err == nil {
		t.Fatal("unmarshalable value must fail")
	}
}

func contains(s, sub string) bool {
	for i := 0; i+len(sub) <= len(s); i++ {
		if s[i:i+len(sub)] == sub {
			return true
		}
	}
	return len(sub) == 0
}
