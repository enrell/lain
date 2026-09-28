// Package list owns the per-user media tracking list (D-078..D-081):
// standalone entries that exist with or without library files, plus the
// linked external accounts that import them. Bolt-backed, keyed by
// user+platform in its own buckets — catalog and userstate never see
// this domain, and remote platforms own their imported entries: a sync
// replaces the user's platform set wholesale.
package list

import (
	"strings"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/kv"
)

// ID is the bolt list provider id.
const ID = "lain-list-bolt"

// Service persists list entries and linked accounts keyed by user.
type Service struct {
	db *bolt.DB
}

func New(db *bolt.DB) (*Service, error) {
	if db == nil {
		return nil, &core.Error{Code: "internal", Msg: "nil db"}
	}
	return &Service{db: db}, nil
}

func (s *Service) ID() string { return ID }
func (s *Service) Capabilities() []string {
	return []string{contracts.CapListRead, contracts.CapListWrite, contracts.CapListAccount}
}
func (s *Service) Health() error {
	if s.db == nil {
		return &core.Error{Code: "dependency-unavailable", Msg: "list store has no db"}
	}
	return nil
}

// ListInput reads a user's entries, optionally narrowed by media type
// and status. Empty filters read everything.
type ListInput struct {
	UserID string `json:"user_id"`
	Type   string `json:"type,omitempty"`
	Status string `json:"status,omitempty"`
}

// PutPlatformInput replaces a user's entry set for one platform: the
// remote answer is authoritative (D-081), so stored platform entries
// absent from Entries are removed.
type PutPlatformInput struct {
	UserID   string                `json:"user_id"`
	Platform string                `json:"platform"`
	Entries  []contracts.ListEntry `json:"entries"`
}

// DeletePlatformInput removes every entry a platform owns for a user
// (the unlink path, D-079).
type DeletePlatformInput struct {
	UserID   string `json:"user_id"`
	Platform string `json:"platform"`
}

// GetAccountInput reads one linked account.
type GetAccountInput struct {
	UserID   string `json:"user_id"`
	Platform string `json:"platform"`
}

// PutAccountInput stores or updates a linked account.
type PutAccountInput struct {
	Account contracts.LinkedAccount `json:"account"`
}

// DeleteAccountInput removes a linked account.
type DeleteAccountInput struct {
	UserID   string `json:"user_id"`
	Platform string `json:"platform"`
}

// ListAccountsInput reads linked accounts: one user's, or every
// account in the store when UserID is empty (the scheduler's view).
type ListAccountsInput struct {
	UserID string `json:"user_id,omitempty"`
}

func entryKey(userID, platform, remoteID string) []byte {
	return []byte(userID + "\x00" + platform + "\x00" + remoteID)
}

func userPrefix(userID string) []byte { return []byte(userID + "\x00") }

func userPlatformPrefix(userID, platform string) []byte {
	return []byte(userID + "\x00" + platform + "\x00")
}

func accountKey(userID, platform string) []byte {
	return []byte(userID + "\x00" + platform)
}

func validInput(in PutPlatformInput) error {
	if in.UserID == "" || in.Platform == "" {
		return &core.Error{Code: "invalid-message", Msg: "user_id and platform required"}
	}
	for _, e := range in.Entries {
		if strings.TrimSpace(e.RemoteID) == "" || strings.TrimSpace(e.Title) == "" {
			return &core.Error{Code: "invalid-message", Msg: "entries need remote_id and title"}
		}
	}
	return nil
}

func (s *Service) Invoke(cap string, input any) (any, error) {
	if s.db == nil {
		return nil, &core.Error{Code: "dependency-unavailable", Msg: "list store has no db"}
	}
	switch cap {
	case contracts.CapListRead:
		in, ok := input.(ListInput)
		if !ok {
			return nil, &core.Error{Code: "invalid-message", Msg: "ListInput required"}
		}
		if in.UserID == "" {
			return nil, &core.Error{Code: "invalid-message", Msg: "user_id required"}
		}
		return s.List(in), nil
	case contracts.CapListWrite:
		switch in := input.(type) {
		case PutPlatformInput:
			return s.PutPlatform(in)
		case DeletePlatformInput:
			if in.UserID == "" || in.Platform == "" {
				return nil, &core.Error{Code: "invalid-message", Msg: "user_id and platform required"}
			}
			return s.DeletePlatform(in.UserID, in.Platform)
		default:
			return nil, &core.Error{Code: "invalid-message", Msg: "PutPlatformInput or DeletePlatformInput required"}
		}
	case contracts.CapListAccount:
		switch in := input.(type) {
		case GetAccountInput:
			if in.UserID == "" || in.Platform == "" {
				return nil, &core.Error{Code: "invalid-message", Msg: "user_id and platform required"}
			}
			return s.GetAccount(in.UserID, in.Platform)
		case PutAccountInput:
			a := in.Account
			if a.UserID == "" || a.Platform == "" {
				return nil, &core.Error{Code: "invalid-message", Msg: "account needs user_id and platform"}
			}
			if err := s.PutAccount(a); err != nil {
				return nil, err
			}
			return a, nil
		case DeleteAccountInput:
			if in.UserID == "" || in.Platform == "" {
				return nil, &core.Error{Code: "invalid-message", Msg: "user_id and platform required"}
			}
			return s.DeleteAccount(in.UserID, in.Platform)
		case ListAccountsInput:
			return s.ListAccounts(in.UserID), nil
		default:
			return nil, &core.Error{Code: "invalid-message", Msg: "GetAccountInput, PutAccountInput, DeleteAccountInput or ListAccountsInput required"}
		}
	default:
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
}

// List returns the user's entries, filtered, sorted by title then
// platform so the read is deterministic.
func (s *Service) List(in ListInput) []contracts.ListEntry {
	var out []contracts.ListEntry
	_ = s.db.View(func(tx *bolt.Tx) error {
		cur := tx.Bucket(kv.BList).Cursor()
		prefix := userPrefix(in.UserID)
		for k, v := cur.Seek(prefix); k != nil && len(k) >= len(prefix) && string(k[:len(prefix)]) == string(prefix); k, v = cur.Next() {
			var e contracts.ListEntry
			if err := decodeEntry(v, &e); err != nil {
				continue
			}
			if in.Type != "" && e.MediaType != in.Type {
				continue
			}
			if in.Status != "" && e.Status != in.Status {
				continue
			}
			out = append(out, e)
		}
		return nil
	})
	sortEntries(out)
	if out == nil {
		out = []contracts.ListEntry{}
	}
	return out
}

func sortEntries(es []contracts.ListEntry) {
	for i := 1; i < len(es); i++ {
		for j := i; j > 0; j-- {
			a, b := es[j-1], es[j]
			if a.Title > b.Title || (a.Title == b.Title && a.Platform > b.Platform) {
				es[j-1], es[j] = es[j], es[j-1]
				continue
			}
			break
		}
	}
}

// PutPlatform replaces the user's platform set with the remote answer:
// upserts incoming entries and removes stored ones the remote dropped.
func (s *Service) PutPlatform(in PutPlatformInput) (contracts.ListSyncStats, error) {
	if err := validInput(in); err != nil {
		return contracts.ListSyncStats{}, err
	}
	var stats contracts.ListSyncStats
	err := s.db.Update(func(tx *bolt.Tx) error {
		b := tx.Bucket(kv.BList)
		seen := map[string]bool{}
		for _, e := range in.Entries {
			rid := strings.TrimSpace(e.RemoteID)
			if seen[rid] {
				continue
			}
			seen[rid] = true
			e.UserID = in.UserID
			e.Platform = in.Platform
			e.RemoteID = rid
			e.ID = in.Platform + ":" + rid
			e.UpdatedAt = nowUnix()
			if err := kv.PutJSON(tx, kv.BList, entryKey(in.UserID, in.Platform, rid), e); err != nil {
				return err
			}
			stats.Upserted++
		}
		prefix := userPlatformPrefix(in.UserID, in.Platform)
		var stale [][]byte
		cur := b.Cursor()
		for k, _ := cur.Seek(prefix); k != nil && len(k) >= len(prefix) && string(k[:len(prefix)]) == string(prefix); k, _ = cur.Next() {
			kk := append([]byte(nil), k...)
			rid := string(kk[len(prefix):])
			if !seen[rid] {
				stale = append(stale, kk)
			}
		}
		for _, k := range stale {
			if err := b.Delete(k); err != nil {
				return err
			}
			stats.Removed++
		}
		return nil
	})
	return stats, err
}

// DeletePlatform removes every entry one platform owns for a user.
func (s *Service) DeletePlatform(userID, platform string) (int, error) {
	removed := 0
	err := s.db.Update(func(tx *bolt.Tx) error {
		b := tx.Bucket(kv.BList)
		prefix := userPlatformPrefix(userID, platform)
		var keys [][]byte
		cur := b.Cursor()
		for k, _ := cur.Seek(prefix); k != nil && len(k) >= len(prefix) && string(k[:len(prefix)]) == string(prefix); k, _ = cur.Next() {
			keys = append(keys, append([]byte(nil), k...))
		}
		for _, k := range keys {
			if err := b.Delete(k); err != nil {
				return err
			}
			removed++
		}
		return nil
	})
	return removed, err
}

// GetAccount reads one linked account; missing is a typed not-found.
func (s *Service) GetAccount(userID, platform string) (contracts.LinkedAccount, error) {
	var a contracts.LinkedAccount
	err := s.db.View(func(tx *bolt.Tx) error {
		return kv.GetJSON(tx, kv.BListAccounts, accountKey(userID, platform), &a)
	})
	if err != nil {
		if kv.IsNotFound(err) {
			return a, &core.Error{Code: "not-found", Msg: "no " + platform + " account linked"}
		}
		return a, err
	}
	return a, nil
}

// PutAccount stores a linked account.
func (s *Service) PutAccount(a contracts.LinkedAccount) error {
	return s.db.Update(func(tx *bolt.Tx) error {
		return kv.PutJSON(tx, kv.BListAccounts, accountKey(a.UserID, a.Platform), a)
	})
}

// DeleteAccount removes a linked account, reporting whether it existed.
func (s *Service) DeleteAccount(userID, platform string) (bool, error) {
	found := false
	err := s.db.Update(func(tx *bolt.Tx) error {
		b := tx.Bucket(kv.BListAccounts)
		k := accountKey(userID, platform)
		if b.Get(k) == nil {
			return nil
		}
		found = true
		return b.Delete(k)
	})
	return found, err
}

// ListAccounts reads one user's accounts, or every account when
// userID is empty.
func (s *Service) ListAccounts(userID string) []contracts.LinkedAccount {
	var out []contracts.LinkedAccount
	_ = s.db.View(func(tx *bolt.Tx) error {
		cur := tx.Bucket(kv.BListAccounts).Cursor()
		prefix := ""
		if userID != "" {
			prefix = userID + "\x00"
		}
		for k, v := cur.Seek([]byte(prefix)); k != nil && strings.HasPrefix(string(k), prefix); k, v = cur.Next() {
			var a contracts.LinkedAccount
			if err := decodeAccount(v, &a); err != nil {
				continue
			}
			out = append(out, a)
		}
		return nil
	})
	if out == nil {
		out = []contracts.LinkedAccount{}
	}
	return out
}
