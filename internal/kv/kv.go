// Package kv is the embedded database: one bbolt file (lain.db) with
// buckets per domain. Pure Go, no cgo, no SQL layer — values are JSON
// documents keyed by stable ids, secondary access via composite keys.
// One read-write transaction per mutation keeps every provider crash
// consistent without a write-ahead log of our own.
package kv

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"

	bolt "go.etcd.io/bbolt"
)

// Buckets.
var (
	BUsers       = []byte("users")
	BUsersByName = []byte("users_by_name")
	BLibraries   = []byte("libraries")
	BItems       = []byte("items")
	BItemsByLib  = []byte("items_by_library")
	BProgress    = []byte("progress")
	BMeta        = []byte("meta")
)

// Open opens (creating if needed) the database file with owner-only
// permissions. It holds the OS file lock: two servers on one data dir
// fail fast instead of corrupting each other.
func Open(dataDir string) (*bolt.DB, error) {
	if err := os.MkdirAll(dataDir, 0o700); err != nil {
		return nil, err
	}
	db, err := bolt.Open(filepath.Join(dataDir, "lain.db"), 0o600, nil)
	if err != nil {
		return nil, err
	}
	err = db.Update(func(tx *bolt.Tx) error {
		for _, b := range [][]byte{BUsers, BUsersByName, BLibraries, BItems, BItemsByLib, BProgress, BMeta} {
			if _, err := tx.CreateBucketIfNotExists(b); err != nil {
				return fmt.Errorf("bucket %s: %w", b, err)
			}
		}
		return nil
	})
	if err != nil {
		db.Close()
		return nil, err
	}
	return db, nil
}

// PutJSON marshals v under key in bucket, inside the given rw tx.
func PutJSON(tx *bolt.Tx, bucket, key []byte, v any) error {
	raw, err := json.Marshal(v)
	if err != nil {
		return err
	}
	return tx.Bucket(bucket).Put(key, raw)
}

// GetJSON unmarshals bucket/key into v. Missing key is ErrNotFound.
func GetJSON(tx *bolt.Tx, bucket, key []byte, v any) error {
	raw := tx.Bucket(bucket).Get(key)
	if raw == nil {
		return ErrNotFound
	}
	if err := json.Unmarshal(raw, v); err != nil {
		return fmt.Errorf("corrupt record %s/%s: %w", bucket, key, err)
	}
	return nil
}

// ErrNotFound marks a missing key.
type notFound string

func (e notFound) Error() string { return "not found: " + string(e) }

// ErrNotFound is returned for missing keys.
var ErrNotFound = notFound("key")

// IsNotFound reports a missing key.
func IsNotFound(err error) bool {
	_, ok := err.(notFound)
	return ok
}
