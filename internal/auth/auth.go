// Package auth owns accounts and session tokens for a multi-user
// server. First-run setup creates the initial admin; admins then
// create users. Login policy is local password (bcrypt); token
// checking — signature, expiry, liveness, password version — is core
// mechanism enforced on every request.
//
// Revocation without a session table: every password change, role
// change or disable bumps nothing but the password version is read
// live — tokens carry pwdv and die on mismatch. Disabling is checked
// live too, so a disabled user is locked out within one request.
package auth

import (
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"time"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/kv"
	"golang.org/x/crypto/bcrypt"
)

// Roles.
const (
	RoleAdmin = "admin"
	RoleUser  = "user"
)

// User is a local account.
type User struct {
	ID        string `json:"id"`
	Username  string `json:"username"`
	PassHash  []byte `json:"pass_hash"`
	Role      string `json:"role"`
	Disabled  bool   `json:"disabled"`
	PwdVer    uint64 `json:"pwd_ver"`
	CreatedAt int64  `json:"created_at"`
}

// Public hides the password hash.
func (u User) Public() User {
	u.PassHash = nil
	return u
}

// Service loads the secret (generated once) and serves accounts.
type Service struct {
	db     *bolt.DB
	secret []byte
}

func New(db *bolt.DB) (*Service, error) {
	if db == nil {
		return nil, errors.New("auth: nil db")
	}
	s := &Service{db: db}
	var secretB64 string
	err := db.Update(func(tx *bolt.Tx) error {
		raw := tx.Bucket(kv.BMeta).Get([]byte("secret"))
		if raw != nil {
			secretB64 = string(raw)
			return nil
		}
		gen := make([]byte, 32)
		if _, err := rand.Read(gen); err != nil {
			return err
		}
		secretB64 = base64.RawURLEncoding.EncodeToString(gen)
		return tx.Bucket(kv.BMeta).Put([]byte("secret"), []byte(secretB64))
	})
	if err != nil {
		return nil, err
	}
	raw, err := base64.RawURLEncoding.DecodeString(secretB64)
	if err != nil || len(raw) < 32 {
		return nil, errors.New("auth: corrupt secret")
	}
	s.secret = raw
	return s, nil
}

func (s *Service) get(id string) (User, error) {
	var u User
	err := s.db.View(func(tx *bolt.Tx) error {
		return kv.GetJSON(tx, kv.BUsers, []byte(id), &u)
	})
	return u, err
}

func (s *Service) put(u User) error {
	return s.db.Update(func(tx *bolt.Tx) error {
		if err := kv.PutJSON(tx, kv.BUsers, []byte(u.ID), u); err != nil {
			return err
		}
		return tx.Bucket(kv.BUsersByName).Put([]byte(u.Username), []byte(u.ID))
	})
}

// HasUsers reports whether first-run setup already happened.
func (s *Service) HasUsers() bool {
	n := 0
	_ = s.db.View(func(tx *bolt.Tx) error {
		n = tx.Bucket(kv.BUsers).Stats().KeyN
		return nil
	})
	return n > 0
}

// Setup creates the first admin account. It refuses when users exist.
func (s *Service) Setup(username, password string) (User, error) {
	if s.HasUsers() {
		return User{}, errors.New("setup already completed")
	}
	if len(username) < 1 || len(password) < 8 {
		return User{}, errors.New("username required, password >= 8 chars")
	}
	u, err := newUser("user-admin", username, password, RoleAdmin)
	if err != nil {
		return User{}, err
	}
	if err := s.put(u); err != nil {
		return User{}, err
	}
	return u.Public(), nil
}

func newUser(id, username, password, role string) (User, error) {
	if len(username) < 1 || len(password) < 8 {
		return User{}, errors.New("username required, password >= 8 chars")
	}
	if role != RoleAdmin && role != RoleUser {
		return User{}, errors.New("unknown role")
	}
	hash, err := bcrypt.GenerateFromPassword([]byte(password), bcrypt.DefaultCost)
	if err != nil {
		return User{}, err
	}
	return User{ID: id, Username: username, PassHash: hash, Role: role, PwdVer: 1, CreatedAt: time.Now().Unix()}, nil
}

// Create adds a user (admin-only at the gateway layer). Usernames are
// unique; ids derive from them deterministically.
func (s *Service) Create(username, password, role string) (User, error) {
	var out User
	err := s.db.Update(func(tx *bolt.Tx) error {
		if tx.Bucket(kv.BUsersByName).Get([]byte(username)) != nil {
			return errors.New("username taken")
		}
		u, err := newUser("user-"+username, username, password, role)
		if err != nil {
			return err
		}
		if err := kv.PutJSON(tx, kv.BUsers, []byte(u.ID), u); err != nil {
			return err
		}
		if err := tx.Bucket(kv.BUsersByName).Put([]byte(username), []byte(u.ID)); err != nil {
			return err
		}
		out = u
		return nil
	})
	if err != nil {
		return User{}, err
	}
	return out.Public(), nil
}

// List returns all users without hashes, sorted by username.
func (s *Service) List() []User {
	var out []User
	_ = s.db.View(func(tx *bolt.Tx) error {
		return tx.Bucket(kv.BUsers).ForEach(func(_, v []byte) error {
			var u User
			if err := json.Unmarshal(v, &u); err != nil {
				return nil
			}
			out = append(out, u.Public())
			return nil
		})
	})
	for i := 1; i < len(out); i++ {
		for j := i; j > 0 && out[j].Username < out[j-1].Username; j-- {
			out[j], out[j-1] = out[j-1], out[j]
		}
	}
	if out == nil {
		out = []User{}
	}
	return out
}

// Get returns the public user record.
func (s *Service) Get(id string) (User, bool) {
	u, err := s.get(id)
	if err != nil {
		return User{}, false
	}
	return u.Public(), true
}

// SetDisabled locks/unlocks an account effective immediately:
// Verify reads liveness on every request.
func (s *Service) SetDisabled(id string, disabled bool) error {
	u, err := s.get(id)
	if err != nil {
		return errors.New("unknown user")
	}
	u.Disabled = disabled
	return s.put(u)
}

// SetRole changes admin/user effective on next Verify (role rides the
// token, so existing tokens keep the old role until expiry — callers
// that need instant demotion also bump via ChangePassword).
func (s *Service) SetRole(id, role string) error {
	if role != RoleAdmin && role != RoleUser {
		return errors.New("unknown role")
	}
	u, err := s.get(id)
	if err != nil {
		return errors.New("unknown user")
	}
	u.Role = role
	return s.put(u)
}

// ChangePassword verifies the old password, sets the new one and bumps
// the password version: every previously minted token dies on next use.
func (s *Service) ChangePassword(id, old, new string) error {
	u, err := s.get(id)
	if err != nil {
		return errors.New("unknown user")
	}
	if err := bcrypt.CompareHashAndPassword(u.PassHash, []byte(old)); err != nil {
		return errors.New("invalid credentials")
	}
	return s.setPassword(u, new)
}

// AdminReset sets any user's password without the old one (admin-only
// at the gateway layer) and kills their tokens via version bump.
func (s *Service) AdminReset(id, new string) error {
	u, err := s.get(id)
	if err != nil {
		return errors.New("unknown user")
	}
	return s.setPassword(u, new)
}

func (s *Service) setPassword(u User, new string) error {
	if len(new) < 8 {
		return errors.New("password >= 8 chars")
	}
	hash, err := bcrypt.GenerateFromPassword([]byte(new), bcrypt.DefaultCost)
	if err != nil {
		return err
	}
	u.PassHash = hash
	u.PwdVer++
	return s.put(u)
}

// Login verifies credentials (and liveness) and mints a 24h token.
func (s *Service) Login(username, password string) (string, error) {
	var u User
	err := s.db.View(func(tx *bolt.Tx) error {
		id := tx.Bucket(kv.BUsersByName).Get([]byte(username))
		if id == nil {
			return errors.New("invalid credentials")
		}
		return kv.GetJSON(tx, kv.BUsers, id, &u)
	})
	if err != nil {
		return "", errors.New("invalid credentials")
	}
	if u.Disabled {
		return "", errors.New("account disabled")
	}
	if err := bcrypt.CompareHashAndPassword(u.PassHash, []byte(password)); err != nil {
		return "", errors.New("invalid credentials")
	}
	return s.mint(u), nil
}

type claims struct {
	Sub  string `json:"sub"`
	Role string `json:"role"`
	PwdV uint64 `json:"pwdv"`
	Iat  int64  `json:"iat"`
	Exp  int64  `json:"exp"`
}

func (s *Service) mint(u User) string {
	now := time.Now().Unix()
	c := claims{Sub: u.ID, Role: u.Role, PwdV: u.PwdVer, Iat: now, Exp: now + 24*3600}
	head, _ := json.Marshal(map[string]string{"alg": "HS256", "typ": "JWT"})
	body, _ := json.Marshal(c)
	he := base64.RawURLEncoding.EncodeToString(head)
	be := base64.RawURLEncoding.EncodeToString(body)
	mac := hmac.New(sha256.New, s.secret)
	mac.Write([]byte(he + "." + be))
	sig := base64.RawURLEncoding.EncodeToString(mac.Sum(nil))
	return he + "." + be + "." + sig
}

// Verified is an authenticated identity.
type Verified struct {
	UserID string
	Role   string
}

// Verify checks signature, expiry, account liveness and password
// version against live state. Disabled accounts and rotated passwords
// fail here, on every request.
func (s *Service) Verify(token string) (Verified, error) {
	parts := strings.Split(token, ".")
	if len(parts) != 3 {
		return Verified{}, errors.New("bad token")
	}
	mac := hmac.New(sha256.New, s.secret)
	mac.Write([]byte(parts[0] + "." + parts[1]))
	want := base64.RawURLEncoding.EncodeToString(mac.Sum(nil))
	if !hmac.Equal([]byte(want), []byte(parts[2])) {
		return Verified{}, errors.New("bad token")
	}
	raw, err := base64.RawURLEncoding.DecodeString(parts[1])
	if err != nil {
		return Verified{}, errors.New("bad token")
	}
	var c claims
	if err := json.Unmarshal(raw, &c); err != nil {
		return Verified{}, errors.New("bad token")
	}
	if time.Now().Unix() > c.Exp {
		return Verified{}, errors.New("token expired")
	}
	u, err := s.get(c.Sub)
	if err != nil {
		return Verified{}, fmt.Errorf("unknown subject")
	}
	if u.Disabled {
		return Verified{}, errors.New("account disabled")
	}
	if u.PwdVer != c.PwdV {
		return Verified{}, errors.New("password rotated")
	}
	return Verified{UserID: u.ID, Role: u.Role}, nil
}
