// Package userstate owns per-user consumption state (exactly-one per
// scope): progress, resume, completion. It is separate from the catalog
// so reindexing media never destroys where the user stopped watching.
// External syncs (AniList and friends) mirror this state; they never
// own it.
package userstate

import (
	"sync"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/store"
)

// ID is the built-in file userstate provider id.
const ID = "lain-userstate-file"

// Service persists progress keyed by user+item.
type Service struct {
	mu   sync.RWMutex
	st   *store.Dir
	prog map[string]contracts.Progress
}

func New(st *store.Dir) (*Service, error) {
	s := &Service{st: st, prog: map[string]contracts.Progress{}}
	var loaded map[string]contracts.Progress
	if err := st.Load("userstate.json", &loaded); err != nil {
		if err == store.ErrNotFound {
			return s, nil
		}
		return nil, err
	}
	s.prog = loaded
	return s, nil
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

func key(userID, itemID string) string { return userID + "\x00" + itemID }

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
// writes never touch this document.
func (s *Service) Put(in PutInput) (contracts.Progress, error) {
	p := in.Progress
	p.UserID = in.UserID
	p.UpdatedAt = time.Now().Unix()
	s.mu.Lock()
	s.prog[key(p.UserID, p.ItemID)] = p
	cp := copyAll(s.prog)
	s.mu.Unlock()
	if err := s.st.Save("userstate.json", cp); err != nil {
		return contracts.Progress{}, err
	}
	return p, nil
}

// Get returns progress or the zero value when never recorded.
func (s *Service) Get(userID, itemID string) (contracts.Progress, bool) {
	s.mu.RLock()
	defer s.mu.RUnlock()
	p, ok := s.prog[key(userID, itemID)]
	return p, ok
}

func copyAll(m map[string]contracts.Progress) map[string]contracts.Progress {
	cp := make(map[string]contracts.Progress, len(m))
	for k, v := range m {
		cp[k] = v
	}
	return cp
}
