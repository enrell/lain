package kv

import (
	"encoding/json"
	"os"
	"path/filepath"

	bolt "go.etcd.io/bbolt"
)

// legacyUser mirrors the v0.1 users.json shape (no roles).
type legacyUser struct {
	ID        string `json:"id"`
	Username  string `json:"username"`
	PassHash  []byte `json:"pass_hash"`
	CreatedAt int64  `json:"created_at"`
}

type legacyLibrary struct {
	ID        string `json:"id"`
	Name      string `json:"name"`
	Type      string `json:"type"`
	Path      string `json:"path"`
	Source    string `json:"source"`
	CreatedAt int64  `json:"created_at"`
}

// ImportLegacy migrates v0.1 JSON documents into buckets exactly once:
// it runs only when the users bucket is empty and a users.json exists.
// Imported files are renamed to *.imported (never deleted) so the
// migration is visible and reversible by hand.
func ImportLegacy(db *bolt.DB, docs DocStore) error {
	if _, err := os.Stat(filepath.Join(docs.Root(), "users.json")); err != nil {
		return nil // fresh install, nothing to import
	}
	var users []legacyUser
	if err := docs.Load("users.json", &users); err != nil {
		return err
	}
	empty := false
	if err := db.View(func(tx *bolt.Tx) error {
		empty = tx.Bucket(BUsers).Stats().KeyN == 0
		return nil
	}); err != nil {
		return err
	}
	if !empty {
		return nil
	}
	if err := importAll(db, docs, users); err != nil {
		return err
	}
	for _, name := range []string{"users.json", "secret.json", "libraries.json", "catalog.json", "userstate.json"} {
		_ = renameIfExists(docs.Root(), name)
	}
	return nil
}

// DocStore is the minimal document surface ImportLegacy needs
// (satisfied by *store.Dir without importing it).
type DocStore interface {
	Load(name string, v any) error
	Root() string
}

func renameIfExists(root, name string) error {
	src := filepath.Join(root, name)
	if _, err := os.Stat(src); err != nil {
		return nil
	}
	return os.Rename(src, src+".imported")
}

func importAll(db *bolt.DB, docs DocStore, users []legacyUser) error {
	var secretB64 string
	secretErr := docs.Load("secret.json", &secretB64)
	var libs []legacyLibrary
	_ = docs.Load("libraries.json", &libs)
	var items map[string]json.RawMessage
	_ = docs.Load("catalog.json", &items)
	var prog map[string]json.RawMessage
	_ = docs.Load("userstate.json", &prog)

	return db.Update(func(tx *bolt.Tx) error {
		ub, un := tx.Bucket(BUsers), tx.Bucket(BUsersByName)
		for _, lu := range users {
			role := "user"
			if lu.ID == "user-admin" {
				role = "admin"
			}
			rec := map[string]any{
				"id": lu.ID, "username": lu.Username,
				"pass_hash": lu.PassHash, "role": role,
				"disabled": false, "pwd_ver": uint64(1),
				"created_at": lu.CreatedAt,
			}
			raw, err := json.Marshal(rec)
			if err != nil {
				return err
			}
			if err := ub.Put([]byte(lu.ID), raw); err != nil {
				return err
			}
			if err := un.Put([]byte(lu.Username), []byte(lu.ID)); err != nil {
				return err
			}
		}
		if secretErr == nil && secretB64 != "" {
			if err := tx.Bucket(BMeta).Put([]byte("secret"), []byte(secretB64)); err != nil {
				return err
			}
		}
		lb := tx.Bucket(BLibraries)
		for _, l := range libs {
			raw, err := json.Marshal(l)
			if err != nil {
				return err
			}
			if err := lb.Put([]byte(l.ID), raw); err != nil {
				return err
			}
		}
		ib, il := tx.Bucket(BItems), tx.Bucket(BItemsByLib)
		for id, raw := range items {
			var probe struct {
				LibraryID string `json:"library_id"`
			}
			if err := json.Unmarshal(raw, &probe); err != nil {
				continue
			}
			if err := ib.Put([]byte(id), raw); err != nil {
				return err
			}
			if err := il.Put([]byte(probe.LibraryID+"\x00"+id), []byte{}); err != nil {
				return err
			}
		}
		pb := tx.Bucket(BProgress)
		for k, raw := range prog {
			if err := pb.Put([]byte(k), raw); err != nil {
				return err
			}
		}
		return nil
	})
}
