// Package auth owns first-run setup, password verification and HS256
// session tokens. Token checking is a core mechanism; the login policy
// (local password here) is the replaceable part in later versions.
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

	"github.com/enrell/lain/internal/store"
	"golang.org/x/crypto/bcrypt"
)

// User is a local account with a bcrypt password hash.
type User struct {
	ID        string `json:"id"`
	Username  string `json:"username"`
	PassHash  []byte `json:"pass_hash"`
	CreatedAt int64  `json:"created_at"`
}

// Service loads the secret (generated once) and the user list.
type Service struct {
	st     *store.Dir
	secret []byte
	users  map[string]User // by id
}

func New(st *store.Dir) (*Service, error) {
	s := &Service{st: st, users: map[string]User{}}
	var secretB64 string
	if err := st.Load("secret.json", &secretB64); err != nil {
		if err != store.ErrNotFound {
			return nil, err
		}
		raw := make([]byte, 32)
		if _, err := rand.Read(raw); err != nil {
			return nil, err
		}
		secretB64 = base64.RawURLEncoding.EncodeToString(raw)
		if err := st.Save("secret.json", secretB64); err != nil {
			return nil, err
		}
	}
	raw, err := base64.RawURLEncoding.DecodeString(secretB64)
	if err != nil || len(raw) < 32 {
		return nil, errors.New("auth: corrupt secret")
	}
	s.secret = raw
	var users []User
	if err := st.Load("users.json", &users); err != nil {
		if err != store.ErrNotFound {
			return nil, err
		}
		return s, nil
	}
	for _, u := range users {
		s.users[u.ID] = u
	}
	return s, nil
}

func (s *Service) persist() error {
	list := make([]User, 0, len(s.users))
	for _, u := range s.users {
		list = append(list, u)
	}
	return s.st.Save("users.json", list)
}

// HasUsers reports whether first-run setup already happened.
func (s *Service) HasUsers() bool { return len(s.users) > 0 }

// Setup creates the first admin account. It refuses when users exist.
func (s *Service) Setup(username, password string) (User, error) {
	if s.HasUsers() {
		return User{}, errors.New("setup already completed")
	}
	if len(username) < 1 || len(password) < 8 {
		return User{}, errors.New("username required, password >= 8 chars")
	}
	hash, err := bcrypt.GenerateFromPassword([]byte(password), bcrypt.DefaultCost)
	if err != nil {
		return User{}, err
	}
	u := User{ID: "user-admin", Username: username, PassHash: hash, CreatedAt: time.Now().Unix()}
	s.users[u.ID] = u
	if err := s.persist(); err != nil {
		return User{}, err
	}
	u.PassHash = nil
	return u, nil
}

// Login verifies credentials and mints a 24h token.
func (s *Service) Login(username, password string) (string, error) {
	for _, u := range s.users {
		if u.Username != username {
			continue
		}
		if err := bcrypt.CompareHashAndPassword(u.PassHash, []byte(password)); err != nil {
			return "", errors.New("invalid credentials")
		}
		return s.mint(u.ID), nil
	}
	return "", errors.New("invalid credentials")
}

// Get returns the public user record.
func (s *Service) Get(id string) (User, bool) {
	u, ok := s.users[id]
	if !ok {
		return User{}, false
	}
	u.PassHash = nil
	return u, true
}

type claims struct {
	Sub string `json:"sub"`
	Iat int64  `json:"iat"`
	Exp int64  `json:"exp"`
}

func (s *Service) mint(userID string) string {
	now := time.Now().Unix()
	c := claims{Sub: userID, Iat: now, Exp: now + 24*3600}
	head, _ := json.Marshal(map[string]string{"alg": "HS256", "typ": "JWT"})
	body, _ := json.Marshal(c)
	he := base64.RawURLEncoding.EncodeToString(head)
	be := base64.RawURLEncoding.EncodeToString(body)
	mac := hmac.New(sha256.New, s.secret)
	mac.Write([]byte(he + "." + be))
	sig := base64.RawURLEncoding.EncodeToString(mac.Sum(nil))
	return he + "." + be + "." + sig
}

// Verify checks signature and expiry, returning the subject user id.
func (s *Service) Verify(token string) (string, error) {
	parts := strings.Split(token, ".")
	if len(parts) != 3 {
		return "", errors.New("bad token")
	}
	mac := hmac.New(sha256.New, s.secret)
	mac.Write([]byte(parts[0] + "." + parts[1]))
	want := base64.RawURLEncoding.EncodeToString(mac.Sum(nil))
	if !hmac.Equal([]byte(want), []byte(parts[2])) {
		return "", errors.New("bad token")
	}
	raw, err := base64.RawURLEncoding.DecodeString(parts[1])
	if err != nil {
		return "", errors.New("bad token")
	}
	var c claims
	if err := json.Unmarshal(raw, &c); err != nil {
		return "", errors.New("bad token")
	}
	if time.Now().Unix() > c.Exp {
		return "", errors.New("token expired")
	}
	if _, ok := s.users[c.Sub]; !ok {
		return "", fmt.Errorf("unknown subject")
	}
	return c.Sub, nil
}
