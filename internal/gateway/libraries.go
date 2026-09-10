package gateway

import (
	"errors"
	"os"
	"sort"
	"time"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/kv"
	"github.com/enrell/lain/internal/plugins/source"
)

// LibraryStore persists configured media roots in the libraries bucket.
type LibraryStore struct {
	db *bolt.DB
}

// List returns all libraries sorted by name.
func (l *LibraryStore) List() ([]contracts.Library, error) {
	var out []contracts.Library
	err := l.db.View(func(tx *bolt.Tx) error {
		return tx.Bucket(kv.BLibraries).ForEach(func(_, v []byte) error {
			var lib contracts.Library
			if err := decodeLibrary(v, &lib); err != nil {
				return nil
			}
			out = append(out, lib)
			return nil
		})
	})
	if err != nil {
		return nil, err
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Name < out[j].Name })
	if out == nil {
		out = []contracts.Library{}
	}
	return out, nil
}

// Create validates and stores a library. Id is stable over the path so
// re-creating the same root never orphans catalog entries.
func (l *LibraryStore) Create(name, typ, path string) (contracts.Library, error) {
	if name == "" || path == "" {
		return contracts.Library{}, errors.New("name and path required")
	}
	fi, err := os.Stat(path)
	if err != nil || !fi.IsDir() {
		return contracts.Library{}, errors.New("path is not a readable directory")
	}
	if typ == "" {
		typ = "anime"
	}
	lib := contracts.Library{
		ID: "lib-" + shortID(path), Name: name, Type: typ,
		Path: path, Source: source.ID, CreatedAt: time.Now().Unix(),
	}
	err = l.db.Update(func(tx *bolt.Tx) error {
		return kv.PutJSON(tx, kv.BLibraries, []byte(lib.ID), lib)
	})
	if err != nil {
		return contracts.Library{}, err
	}
	return lib, nil
}

// Delete removes a library. Catalog entries survive (prune happens on
// the next scan of remaining libraries); userstate is never touched.
func (l *LibraryStore) Delete(id string) error {
	return l.db.Update(func(tx *bolt.Tx) error {
		b := tx.Bucket(kv.BLibraries)
		if b.Get([]byte(id)) == nil {
			return errors.New("unknown library")
		}
		return b.Delete([]byte(id))
	})
}
