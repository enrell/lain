package metadata

import (
	"encoding/json"
	"strings"
	"time"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/kv"
)

// Saver persists enrichment overlays: one per catalog item, keyed by
// item id. Identity lives in the catalog; this bucket only decorates.
type Saver struct {
	db *bolt.DB
}

func NewSaver(db *bolt.DB) *Saver { return &Saver{db: db} }

// Save stores the overlay built from a resolved record.
func (s *Saver) Save(itemID string, rec contracts.MetadataRecord) (contracts.Enrichment, error) {
	e := contracts.Enrichment{
		ItemID: itemID, Provider: rec.Provider, RemoteID: rec.RemoteID,
		Title: rec.Title, Year: rec.Year, Genres: rec.Genres,
		Synopsis: rec.Synopsis, Poster: rec.Poster, Cover: rec.Cover,
		FetchedAt: time.Now().Unix(),
	}
	err := s.db.Update(func(tx *bolt.Tx) error {
		return kv.PutJSON(tx, kv.BEnrich, []byte(itemID), e)
	})
	if err != nil {
		return contracts.Enrichment{}, err
	}
	return e, nil
}

// Get returns the overlay or false.
func (s *Saver) Get(itemID string) (contracts.Enrichment, bool) {
	var e contracts.Enrichment
	err := s.db.View(func(tx *bolt.Tx) error {
		return kv.GetJSON(tx, kv.BEnrich, []byte(itemID), &e)
	})
	if err != nil {
		return contracts.Enrichment{}, false
	}
	return e, true
}

// Delete removes one item's overlay (provider removal path).
func (s *Saver) Delete(itemID string) error {
	return s.db.Update(func(tx *bolt.Tx) error {
		return tx.Bucket(kv.BEnrich).Delete([]byte(itemID))
	})
}

// DeleteProvider removes every overlay served by a provider: uninstall
// leaves no orphaned decorations behind.
func (s *Saver) DeleteProvider(provider string) (int, error) {
	n := 0
	err := s.db.Update(func(tx *bolt.Tx) error {
		b := tx.Bucket(kv.BEnrich)
		cur := b.Cursor()
		for k, v := cur.First(); k != nil; k, v = cur.Next() {
			var e contracts.Enrichment
			if err := json.Unmarshal(v, &e); err != nil {
				continue
			}
			if e.Provider == provider {
				if err := cur.Delete(); err != nil {
					return err
				}
				n++
			}
		}
		return nil
	})
	return n, err
}

// Cache lifetimes: search answers go stale fast (new seasons), records
// are stable for weeks.
const (
	SearchTTL = 7 * 24 * time.Hour
	RecordTTL = 30 * 24 * time.Hour
)

type cacheEntry struct {
	At   int64           `json:"at"`
	Data json.RawMessage `json:"data"`
}

// Cache is a TTL memo in front of remotes: repeat enrichments and
// rescans do not touch the network. Mechanism, not provider.
type Cache struct {
	db *bolt.DB
}

func NewCache(db *bolt.DB) *Cache { return &Cache{db: db} }

func searchKey(kind, query string) []byte {
	return []byte("q\x00" + strings.ToLower(kind) + "\x00" + strings.ToLower(strings.TrimSpace(query)))
}

func recordKey(provider, id string) []byte {
	return []byte("r\x00" + provider + "\x00" + id)
}

// GetSearch returns cached merged candidates when fresh.
func (c *Cache) GetSearch(kind, query string) ([]contracts.MetadataCandidate, bool) {
	var e cacheEntry
	err := c.db.View(func(tx *bolt.Tx) error {
		return kv.GetJSON(tx, kv.BCache, searchKey(kind, query), &e)
	})
	if err != nil || time.Since(time.Unix(e.At, 0)) > SearchTTL {
		return nil, false
	}
	var out []contracts.MetadataCandidate
	if err := json.Unmarshal(e.Data, &out); err != nil {
		return nil, false
	}
	return out, true
}

// PutSearch stores merged candidates.
func (c *Cache) PutSearch(kind, query string, out []contracts.MetadataCandidate) error {
	raw, err := json.Marshal(out)
	if err != nil {
		return err
	}
	return c.db.Update(func(tx *bolt.Tx) error {
		return kv.PutJSON(tx, kv.BCache, searchKey(kind, query), cacheEntry{At: time.Now().Unix(), Data: raw})
	})
}

// GetRecord returns a cached resolved record when fresh.
func (c *Cache) GetRecord(provider, id string) (contracts.MetadataRecord, bool) {
	var e cacheEntry
	err := c.db.View(func(tx *bolt.Tx) error {
		return kv.GetJSON(tx, kv.BCache, recordKey(provider, id), &e)
	})
	if err != nil || time.Since(time.Unix(e.At, 0)) > RecordTTL {
		return contracts.MetadataRecord{}, false
	}
	var rec contracts.MetadataRecord
	if err := json.Unmarshal(e.Data, &rec); err != nil {
		return contracts.MetadataRecord{}, false
	}
	return rec, true
}

// PutRecord stores a resolved record.
func (c *Cache) PutRecord(rec contracts.MetadataRecord) error {
	raw, err := json.Marshal(rec)
	if err != nil {
		return err
	}
	return c.db.Update(func(tx *bolt.Tx) error {
		return kv.PutJSON(tx, kv.BCache, recordKey(rec.Provider, rec.RemoteID), cacheEntry{At: time.Now().Unix(), Data: raw})
	})
}
