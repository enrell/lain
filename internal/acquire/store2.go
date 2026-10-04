package acquire

import (
	"encoding/json"
	"sort"
	"strings"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/kv"
)

// Phase 2 persistence: generic JSON records keyed by id.

func listJSON[T any](s *store, bucket []byte) []T {
	out := []T{}
	_ = s.db.View(func(tx *bolt.Tx) error {
		return tx.Bucket(bucket).ForEach(func(_, v []byte) error {
			var x T
			if json.Unmarshal(v, &x) == nil {
				out = append(out, x)
			}
			return nil
		})
	})
	return out
}

func getJSON[T any](s *store, bucket []byte, id, what string) (T, error) {
	var x T
	err := s.db.View(func(tx *bolt.Tx) error { return kv.GetJSON(tx, bucket, []byte(id), &x) })
	if err != nil {
		return x, errf(CodeNotFound, "unknown %s", what)
	}
	return x, nil
}

func putJSON(s *store, bucket []byte, id string, v any) error {
	return s.db.Update(func(tx *bolt.Tx) error { return kv.PutJSON(tx, bucket, []byte(id), v) })
}

func deleteKey(s *store, bucket []byte, id, what string) error {
	return s.db.Update(func(tx *bolt.Tx) error {
		b := tx.Bucket(bucket)
		if b.Get([]byte(id)) == nil {
			return errf(CodeNotFound, "unknown %s", what)
		}
		return b.Delete([]byte(id))
	})
}

func (s *store) profiles() []Profile {
	out := listJSON[Profile](s, kv.BAcqProfiles)
	sort.SliceStable(out, func(i, j int) bool { return out[i].Name < out[j].Name })
	return out
}

func (s *store) monitored() []Monitored {
	out := listJSON[Monitored](s, kv.BAcqMonitored)
	sort.SliceStable(out, func(i, j int) bool { return strings.ToLower(out[i].Title) < strings.ToLower(out[j].Title) })
	return out
}

// BlockEntry is a release automation must not grab again (A-22).
type BlockEntry struct {
	ID          string `json:"id"`
	InfoHash    string `json:"info_hash,omitempty"`
	Title       string `json:"title"`
	IndexerID   string `json:"indexer_id,omitempty"`
	MonitoredID string `json:"monitored_id,omitempty"`
	Reason      string `json:"reason"`
	At          int64  `json:"at"`
}

func (s *store) blocklist() []BlockEntry {
	out := listJSON[BlockEntry](s, kv.BAcqBlocklist)
	sort.SliceStable(out, func(i, j int) bool { return out[i].At > out[j].At })
	return out
}
