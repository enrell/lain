// Package userstate owns per-user consumption state (exactly-one per
// scope): progress, resume, completion. Bolt-backed, keyed by
// user+item in its own bucket — catalog rewrites never touch it.
// External syncs mirror this state; they never own it.
package userstate

import (
	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/kv"
)

// ID is the bolt userstate provider id.
const ID = "lain-userstate-bolt"

// Service persists progress keyed by user+item.
type Service struct {
	db *bolt.DB
}

func New(db *bolt.DB) (*Service, error) {
	if db == nil {
		return nil, &core.Error{Code: "internal", Msg: "nil db"}
	}
	return &Service{db: db}, nil
}

func (s *Service) ID() string             { return ID }
func (s *Service) Capabilities() []string { return []string{contracts.CapUserProgress} }
func (s *Service) Health() error          { return nil }

// PutInput writes progress for the authenticated user.
type PutInput struct {
	UserID   string             `json:"user_id"`
	Progress contracts.Progress `json:"progress"`
}

// GetInput reads progress.
type GetInput struct {
	UserID string `json:"user_id"`
	ItemID string `json:"item_id"`
}

func key(userID, itemID string) []byte { return []byte(userID + "\x00" + itemID) }

func userPrefix(userID string) []byte { return []byte(userID + "\x00") }

func (s *Service) Invoke(cap string, input any) (any, error) {
	if cap != contracts.CapUserProgress {
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
	switch in := input.(type) {
	case PutInput:
		return s.Put(in)
	case GetInput:
		p, _ := s.Get(in.UserID, in.ItemID)
		return p, nil
	default:
		return nil, &core.Error{Code: "invalid-message", Msg: "PutInput or GetInput required"}
	}
}

// Put stores progress; a reindex cannot delete it because catalog
// writes never touch this bucket.
func (s *Service) Put(in PutInput) (contracts.Progress, error) {
	p := in.Progress
	p.UserID = in.UserID
	p.UpdatedAt = nowUnix()
	err := s.db.Update(func(tx *bolt.Tx) error {
		return kv.PutJSON(tx, kv.BProgress, key(p.UserID, p.ItemID), p)
	})
	if err != nil {
		return contracts.Progress{}, err
	}
	return p, nil
}

// Get returns progress or the zero value when never recorded.
func (s *Service) Get(userID, itemID string) (contracts.Progress, bool) {
	var p contracts.Progress
	err := s.db.View(func(tx *bolt.Tx) error {
		return kv.GetJSON(tx, kv.BProgress, key(userID, itemID), &p)
	})
	if err != nil {
		return contracts.Progress{}, false
	}
	return p, true
}

// List returns all of one user's progress (continue-watching feeds).
func (s *Service) List(userID string) []contracts.Progress {
	var out []contracts.Progress
	_ = s.db.View(func(tx *bolt.Tx) error {
		cur := tx.Bucket(kv.BProgress).Cursor()
		prefix := userPrefix(userID)
		for k, v := cur.Seek(prefix); k != nil && len(k) >= len(prefix) && string(k[:len(prefix)]) == string(prefix); k, v = cur.Next() {
			var p contracts.Progress
			if err := decodeProgress(v, &p); err != nil {
				continue
			}
			out = append(out, p)
		}
		return nil
	})
	if out == nil {
		out = []contracts.Progress{}
	}
	return out
}
