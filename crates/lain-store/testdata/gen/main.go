// Command gen writes generated.bbolt, the fixture that exercises the
// Rust bbolt reader on branch pages, overflow pages, inline buckets and
// nested buckets. Run from the repository root:
//
//	go run ./crates/lain-store/testdata/gen crates/lain-store/testdata/generated.bbolt
//
// It lives under testdata/, so `go build ./...` and `go vet ./...`
// ignore it.
package main

import (
	"fmt"
	"os"
	"strings"

	bolt "go.etcd.io/bbolt"
)

func main() {
	out := os.Args[1]
	_ = os.Remove(out)
	db, err := bolt.Open(out, 0o600, nil)
	if err != nil {
		panic(err)
	}
	defer db.Close()
	err = db.Update(func(tx *bolt.Tx) error {
		big, _ := tx.CreateBucket([]byte("big"))
		for i := 0; i < 5000; i++ {
			v := fmt.Sprintf("v%05d-%s", i, strings.Repeat("x", i%50))
			if err := big.Put([]byte(fmt.Sprintf("k%05d", i)), []byte(v)); err != nil {
				return err
			}
		}
		huge, _ := tx.CreateBucket([]byte("huge"))
		val := make([]byte, 20000)
		for i := range val {
			val[i] = byte('a' + i%26)
		}
		if err := huge.Put([]byte("blob"), val); err != nil {
			return err
		}
		small, _ := tx.CreateBucket([]byte("small"))
		_ = small.Put([]byte("a"), []byte("1"))
		_ = small.Put([]byte("b"), []byte("2"))
		nested, _ := tx.CreateBucket([]byte("nested"))
		_ = nested.Put([]byte("top"), []byte("t"))
		child, _ := nested.CreateBucket([]byte("child"))
		return child.Put([]byte("c"), []byte("3"))
	})
	if err != nil {
		panic(err)
	}
}
