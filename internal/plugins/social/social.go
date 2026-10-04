// Package social owns the social domain (docs/slices/social.md): privacy
// settings and favorites, relationships, activity, notifications,
// ratings, reviews, comments and collections. It is a domain of its own
// beside catalog, userstate and the list — it never writes their
// records. Works are referenced by kind plus normalized title, so any
// media kind (video or reading) gets the same social surface.
//
// The provider trusts the gateway for identity: user ids it receives
// are authenticated and exist. It owns every privacy rule, so the
// gateway asks it what a viewer may see instead of deciding itself.
package social

import (
	"crypto/rand"
	"encoding/binary"
	"fmt"
	"strings"
	"sync"
	"time"
	"unicode"
	"unicode/utf8"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/kv"
	"github.com/enrell/lain/internal/plugins/catalog"
)

// ID is the bolt social provider id.
const ID = "lain-social-bolt"

// Bounds (S-6..S-10).
const (
	maxTitle          = 200
	maxReview         = 2000
	maxComment        = 1000
	maxMessage        = 280
	maxCollectionName = 80
	maxCollectionDesc = 500
	maxCollectionSize = 500
	maxFavorites      = 12
	maxActivityRows   = 500
	maxNotifications  = 200
	maxShareTargets   = 50
	progressWindow    = 6 * time.Hour
	defaultPageLimit  = 50
	maxPageLimit      = 200
)

// Service persists the social domain in its own buckets.
type Service struct {
	db *bolt.DB
	// now is the clock; tests replace it to cross the dedupe window.
	now func() time.Time

	mu  sync.Mutex
	seq uint32
}

// New creates the social buckets if missing and returns the provider.
func New(db *bolt.DB) (*Service, error) {
	if db == nil {
		return nil, &core.Error{Code: "internal", Msg: "nil db"}
	}
	err := db.Update(func(tx *bolt.Tx) error {
		for _, b := range kv.SocialBuckets() {
			if _, err := tx.CreateBucketIfNotExists(b); err != nil {
				return fmt.Errorf("bucket %s: %w", b, err)
			}
		}
		return nil
	})
	if err != nil {
		return nil, err
	}
	return &Service{db: db, now: time.Now}, nil
}

// SetClock replaces the clock (tests).
func (s *Service) SetClock(now func() time.Time) { s.now = now }

func (s *Service) ID() string { return ID }
func (s *Service) Capabilities() []string {
	return []string{contracts.CapSocialGraph, contracts.CapSocialActivity, contracts.CapSocialReviews, contracts.CapSocialCollections}
}
func (s *Service) Health() error {
	if s.db == nil {
		return &core.Error{Code: "dependency-unavailable", Msg: "social store has no db"}
	}
	return nil
}

// newID returns a time-ordered unique id: 16 hex digits of unix nanos,
// then a process sequence and random bits so ids minted in the same
// nanosecond still sort in creation order and never collide.
func (s *Service) newID() string {
	s.mu.Lock()
	s.seq++
	seq := s.seq
	s.mu.Unlock()
	var r [4]byte
	_, _ = rand.Read(r[:])
	return fmt.Sprintf("%016x%08x%08x", uint64(s.now().UnixNano()), seq, binary.BigEndian.Uint32(r[:]))
}

func invalid(msg string) error     { return &core.Error{Code: "invalid-message", Msg: msg} }
func notFoundErr(msg string) error { return &core.Error{Code: "not-found", Msg: msg} }
func forbidden(msg string) error   { return &core.Error{Code: "forbidden", Msg: msg} }

// cleanText applies the profile text rules (D-086): trimmed, valid
// UTF-8, no control characters, single-line text has its whitespace
// collapsed, and the rune count is bounded.
func cleanText(s string, max int, field string, multiline bool) (string, error) {
	s = strings.TrimSpace(s)
	if !utf8.ValidString(s) {
		return "", invalid(field + " is not valid text")
	}
	for _, r := range s {
		if r == '\n' && multiline {
			continue
		}
		if unicode.IsControl(r) {
			return "", invalid(field + " must not contain control characters")
		}
	}
	if !multiline {
		s = strings.Join(strings.Fields(s), " ")
	}
	if utf8.RuneCountInString(s) > max {
		return "", invalid(field + " is too long")
	}
	return s, nil
}

func validKind(k string) bool {
	if k == "" || len(k) > 24 {
		return false
	}
	for _, r := range k {
		if !(r >= 'a' && r <= 'z' || r >= '0' && r <= '9' || r == '-') {
			return false
		}
	}
	return true
}

// NormalizeWork validates a work reference and cleans its title. The
// item id is kept only as a link hint and must look like a catalog id.
func NormalizeWork(w contracts.WorkRef) (contracts.WorkRef, error) {
	w.Kind = strings.ToLower(strings.TrimSpace(w.Kind))
	if !validKind(w.Kind) {
		return w, invalid("work kind must be a lowercase token")
	}
	t, err := cleanText(w.Title, maxTitle, "work title", false)
	if err != nil {
		return w, err
	}
	if t == "" {
		return w, invalid("work title required")
	}
	w.Title = t
	if len(w.ItemID) > 64 || strings.ContainsAny(w.ItemID, "\x00/\\") {
		w.ItemID = ""
	}
	return w, nil
}

// WorkKey is the identity of a work: kind plus the title page's
// grouping key (D-056), so every episode, chapter or volume of one
// title shares ratings and comments.
func WorkKey(w contracts.WorkRef) string {
	return w.Kind + "\x00" + catalog.TitleKey(w.Title)
}

func pair(a, b string) []byte { return []byte(a + "\x00" + b) }

func prefix(a string) []byte { return []byte(a + "\x00") }

// eachPrefix walks bucket keys under p in order (or reverse).
func eachPrefix(b *bolt.Bucket, p []byte, reverse bool, fn func(k, v []byte) bool) {
	c := b.Cursor()
	has := func(k []byte) bool { return k != nil && len(k) >= len(p) && string(k[:len(p)]) == string(p) }
	if !reverse {
		for k, v := c.Seek(p); has(k); k, v = c.Next() {
			if !fn(k, v) {
				return
			}
		}
		return
	}
	// Seek past the prefix range, then walk back.
	end := append(append([]byte(nil), p...), 0xff)
	k, v := c.Seek(end)
	if k == nil {
		k, v = c.Last()
	} else {
		k, v = c.Prev()
	}
	for ; has(k); k, v = c.Prev() {
		if !fn(k, v) {
			return
		}
	}
}

func clampLimit(n int) int {
	if n <= 0 {
		return defaultPageLimit
	}
	if n > maxPageLimit {
		return maxPageLimit
	}
	return n
}

func validVisibility(v string) bool {
	return v == contracts.VisibilityPublic || v == contracts.VisibilityFriends || v == contracts.VisibilityPrivate
}

// Invoke dispatches by capability and input type.
func (s *Service) Invoke(cap string, input any) (any, error) {
	if s.db == nil {
		return nil, &core.Error{Code: "dependency-unavailable", Msg: "social store has no db"}
	}
	switch cap {
	case contracts.CapSocialGraph:
		return s.invokeGraph(input)
	case contracts.CapSocialActivity:
		return s.invokeActivity(input)
	case contracts.CapSocialReviews:
		return s.invokeReviews(input)
	case contracts.CapSocialCollections:
		return s.invokeCollections(input)
	default:
		return nil, invalid("unsupported cap " + cap)
	}
}

func needUser(ids ...string) error {
	for _, id := range ids {
		if id == "" {
			return invalid("user ids required")
		}
	}
	return nil
}
