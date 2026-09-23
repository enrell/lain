package auth


import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"strings"
	"testing"
	"time"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/kv"
)

func testService(t *testing.T) *Service {
	t.Helper()
	db, err := kv.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	s, err := New(db)
	if err != nil {
		t.Fatal(err)
	}
	return s
}

func TestSetupCreatesAdminOnce(t *testing.T) {
	s := testService(t)
	if s.HasUsers() {
		t.Fatal("fresh db must need setup")
	}
	u, err := s.Setup("admin", "password123")
	if err != nil {
		t.Fatal(err)
	}
	if u.Role != RoleAdmin || len(u.PassHash) != 0 {
		t.Fatalf("setup must return public admin: %+v", u)
	}
	if !s.HasUsers() {
		t.Fatal("must have users after setup")
	}
	if _, err := s.Setup("x", "password123"); err == nil {
		t.Fatal("second setup must refuse")
	}
}

func TestCreateLoginRoles(t *testing.T) {
	s := testService(t)
	if _, err := s.Setup("admin", "password123"); err != nil {
		t.Fatal(err)
	}
	u, err := s.Create("ana", "password123", RoleUser)
	if err != nil {
		t.Fatal(err)
	}
	if u.Role != RoleUser {
		t.Fatalf("role=%q, want user", u.Role)
	}
	if _, err := s.Create("ana", "password123", RoleUser); err == nil {
		t.Fatal("duplicate username must refuse")
	}
	if _, err := s.Create("bob", "short", RoleUser); err == nil {
		t.Fatal("weak password must refuse")
	}
	if _, err := s.Create("zed", "password123", "root"); err == nil {
		t.Fatal("unknown role must refuse")
	}
	tok, err := s.Login("ana", "password123")
	if err != nil {
		t.Fatal(err)
	}
	v, err := s.Verify(tok)
	if err != nil || v.UserID != u.ID || v.Role != RoleUser {
		t.Fatalf("verify=%+v err=%v", v, err)
	}
	if _, err := s.Login("ana", "wrong"); err == nil {
		t.Fatal("wrong password must refuse")
	}
}

func TestDisableLocksOutLive(t *testing.T) {
	s := testService(t)
	if _, err := s.Setup("admin", "password123"); err != nil {
		t.Fatal(err)
	}
	u, err := s.Create("ana", "password123", RoleUser)
	if err != nil {
		t.Fatal(err)
	}
	tok, err := s.Login("ana", "password123")
	if err != nil {
		t.Fatal(err)
	}
	if err := s.SetDisabled(u.ID, true); err != nil {
		t.Fatal(err)
	}
	if _, err := s.Verify(tok); err == nil {
		t.Fatal("live token must die on disable")
	}
	if _, err := s.Login("ana", "password123"); err == nil {
		t.Fatal("login must refuse disabled account")
	}
	if err := s.SetDisabled(u.ID, false); err != nil {
		t.Fatal(err)
	}
	if _, err := s.Login("ana", "password123"); err != nil {
		t.Fatalf("re-enabled login: %v", err)
	}
}

func TestPasswordRotationKillsTokens(t *testing.T) {
	s := testService(t)
	if _, err := s.Setup("admin", "password123"); err != nil {
		t.Fatal(err)
	}
	u, err := s.Create("ana", "password123", RoleUser)
	if err != nil {
		t.Fatal(err)
	}
	tok, err := s.Login("ana", "password123")
	if err != nil {
		t.Fatal(err)
	}
	if err := s.ChangePassword(u.ID, "password123", "newpassword123"); err != nil {
		t.Fatal(err)
	}
	if _, err := s.Verify(tok); err == nil {
		t.Fatal("old token must die on rotation")
	}
	if _, err := s.Login("ana", "newpassword123"); err != nil {
		t.Fatalf("login with new password: %v", err)
	}
	if err := s.ChangePassword(u.ID, "wrong", "another123"); err == nil {
		t.Fatal("rotation with wrong old password must refuse")
	}
}

func TestAdminResetKillsTokens(t *testing.T) {
	s := testService(t)
	if _, err := s.Setup("admin", "password123"); err != nil {
		t.Fatal(err)
	}
	u, err := s.Create("ana", "password123", RoleUser)
	if err != nil {
		t.Fatal(err)
	}
	tok, _ := s.Login("ana", "password123")
	if err := s.AdminReset(u.ID, "resetpass123"); err != nil {
		t.Fatal(err)
	}
	if _, err := s.Verify(tok); err == nil {
		t.Fatal("token must die on admin reset")
	}
}

// Input validation boundaries: empty usernames and short passwords are
// rejected everywhere; a password of exactly 8 chars is accepted.
func TestValidationBoundaries(t *testing.T) {
	s := testService(t)
	if _, err := s.Setup("", "password123"); err == nil {
		t.Fatal("empty username must refuse")
	}
	if _, err := s.Setup("admin", "1234567"); err == nil {
		t.Fatal("7-char password must refuse")
	}
	if _, err := s.Setup("admin", "12345678"); err != nil {
		t.Fatalf("8-char password must pass: %v", err)
	}
	if _, err := s.Create("", "password123", RoleUser); err == nil {
		t.Fatal("empty username via Create must refuse")
	}
	u, err := s.Create("ana", "12345678", RoleUser)
	if err != nil {
		t.Fatalf("8-char password via Create: %v", err)
	}
	// An id longer than bbolt's key limit must fail, not panic.
	long := strings.Repeat("x", 40000)
	if _, err := s.Create(long, "password123", RoleUser); err == nil {
		t.Fatal("oversized username key must fail")
	}
	_ = u
}

// List returns users sorted by username regardless of insert order.
func TestListSortsByUsername(t *testing.T) {
	s := testService(t)
	if _, err := s.Setup("admin", "password123"); err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"zoe", "amy", "meg", "bob"} {
		if _, err := s.Create(name, "password123", RoleUser); err != nil {
			t.Fatal(err)
		}
	}
	list := s.List()
	if len(list) != 5 {
		t.Fatalf("users=%d, want 5", len(list))
	}
	var names []string
	for _, u := range list {
		names = append(names, u.Username)
		if u.PassHash != nil {
			t.Fatal("List must not leak pass hashes")
		}
	}
	for i := 1; i < len(names); i++ {
		if names[i] < names[i-1] {
			t.Fatalf("not sorted: %v", names)
		}
	}
	if names[0] != "admin" || names[1] != "amy" || names[4] != "zoe" {
		t.Fatalf("order: %v", names)
	}
}

func TestGetSetRolePlaybackPolicy(t *testing.T) {
	s := testService(t)
	if _, err := s.Setup("admin", "password123"); err != nil {
		t.Fatal(err)
	}
	u, err := s.Create("ana", "password123", RoleUser)
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := s.Get(u.ID); !ok {
		t.Fatal("Get must find the user")
	}
	if _, ok := s.Get("ghost"); ok {
		t.Fatal("Get on unknown id must report absent")
	}
	if err := s.SetRole(u.ID, "root"); err == nil {
		t.Fatal("unknown role must refuse")
	}
	if err := s.SetRole("ghost", RoleAdmin); err == nil {
		t.Fatal("SetRole on unknown user must fail")
	}
	if err := s.SetRole(u.ID, RoleAdmin); err != nil {
		t.Fatal(err)
	}
	if got, _ := s.Get(u.ID); got.Role != RoleAdmin {
		t.Fatalf("role=%q, want admin", got.Role)
	}

	// PlaybackPolicy validation.
	if err := s.SetPlayback(u.ID, PlaybackPolicy{MaxBitrateKbps: -1}); err == nil {
		t.Fatal("negative bitrate must refuse")
	}
	if err := s.SetPlayback(u.ID, PlaybackPolicy{MaxStreams: -1}); err == nil {
		t.Fatal("negative streams must refuse")
	}
	if err := s.SetPlayback(u.ID, PlaybackPolicy{MaxStreams: 33}); err == nil {
		t.Fatal("streams > 32 must refuse")
	}
	if err := s.SetPlayback(u.ID, PlaybackPolicy{SubtitleMode: "bogus"}); err == nil {
		t.Fatal("unknown subtitle mode must refuse")
	}
	if err := s.SetPlayback("ghost", PlaybackPolicy{}); err == nil {
		t.Fatal("SetPlayback on unknown user must fail")
	}
	pol := PlaybackPolicy{MaxStreams: 2, SubtitleMode: "extract"}
	if err := s.SetPlayback(u.ID, pol); err != nil {
		t.Fatal(err)
	}
	got := s.PlaybackPolicy(u.ID)
	if got.MaxStreams != 2 || got.SubtitleMode != "extract" {
		t.Fatalf("policy: %+v", got)
	}
	if empty := s.PlaybackPolicy("ghost"); empty.Restricted() {
		t.Fatalf("unknown user must yield unrestricted policy: %+v", empty)
	}
}

func TestSetPreferredLanguage(t *testing.T) {
	s := testService(t)
	u, err := s.Setup("admin", "password123")
	if err != nil {
		t.Fatal(err)
	}
	if err := s.SetPreferredLanguage(u.ID, " POR "); err != nil {
		t.Fatal(err)
	}
	got, ok := s.Get(u.ID)
	if !ok || got.PreferredLanguage != "por" {
		t.Fatalf("normalized preference must persist: %+v", got)
	}
	for _, bad := range []string{"po", "port", "p1r", "p{r"} {
		if err := s.SetPreferredLanguage(u.ID, bad); err == nil {
			t.Fatalf("%q must refuse", bad)
		}
	}
	if err := s.SetPreferredLanguage("ghost", "por"); err == nil {
		t.Fatal("unknown user must fail")
	}
	if err := s.SetPreferredLanguage(u.ID, ""); err != nil {
		t.Fatal(err)
	}
	if got, _ := s.Get(u.ID); got.PreferredLanguage != "" {
		t.Fatalf("clear must empty the preference: %+v", got)
	}
}

// PlaybackPolicy defaults are permissive; every setter flips Restricted.
func TestPlaybackPolicySemantics(t *testing.T) {
	var p PlaybackPolicy
	if p.Restricted() || !p.AllowsVideoTranscode() || !p.AllowsAudioTranscode() || !p.AllowsRemux() {
		t.Fatalf("zero policy must be unrestricted+allow-all: %+v", p)
	}
	f := false
	for i, pol := range []PlaybackPolicy{
		{AllowVideoTranscode: &f},
		{AllowAudioTranscode: &f},
		{AllowRemux: &f},
		{MaxBitrateKbps: 8000},
		{MaxStreams: 1},
		{SubtitleMode: "off"},
	} {
		if !pol.Restricted() {
			t.Fatalf("policy %d must report restricted: %+v", i, pol)
		}
	}
	p = PlaybackPolicy{AllowVideoTranscode: &f}
	if p.AllowsVideoTranscode() || !p.AllowsAudioTranscode() {
		t.Fatalf("explicit false must win, unset defaults true: %+v", p)
	}
}

func TestSetPlaybackBoundaries(t *testing.T) {
	s := testService(t)
	u, err := s.Setup("admin", "password123")
	if err != nil {
		t.Fatal(err)
	}
	for _, streams := range []int{0, 1, 32} {
		if err := s.SetPlayback(u.ID, PlaybackPolicy{MaxStreams: streams}); err != nil {
			t.Fatalf("max_streams=%d must be accepted: %v", streams, err)
		}
	}
	for _, streams := range []int{-1, 33, 100} {
		if err := s.SetPlayback(u.ID, PlaybackPolicy{MaxStreams: streams}); err == nil {
			t.Fatalf("max_streams=%d must be rejected", streams)
		}
	}
	if err := s.SetPlayback(u.ID, PlaybackPolicy{MaxBitrateKbps: -1}); err == nil {
		t.Fatal("negative bitrate must be rejected")
	}
	if err := s.SetPlayback(u.ID, PlaybackPolicy{MaxBitrateKbps: 0}); err != nil {
		t.Fatalf("zero bitrate must be accepted: %v", err)
	}
	for _, mode := range []string{"auto", "extract", "burn", "off", "bogus"} {
		err := s.SetPlayback(u.ID, PlaybackPolicy{SubtitleMode: mode})
		if (mode == "bogus") != (err != nil) {
			t.Fatalf("subtitle_mode %q: err=%v", mode, err)
		}
	}
}

// setPassword must bump PwdVer — that is what kills outstanding tokens.
func TestPasswordChangeBumpsVersion(t *testing.T) {
	s := testService(t)
	if _, err := s.Setup("admin", "password123"); err != nil {
		t.Fatal(err)
	}
	u, _ := s.Create("ana", "password123", RoleUser)
	if got, _ := s.Get(u.ID); got.PwdVer != 1 {
		t.Fatalf("fresh user pwd_ver=%d, want 1", got.PwdVer)
	}
	if err := s.ChangePassword(u.ID, "password123", "newpassword123"); err != nil {
		t.Fatal(err)
	}
	if got, _ := s.Get(u.ID); got.PwdVer != 2 {
		t.Fatalf("after change pwd_ver=%d, want 2", got.PwdVer)
	}
	if err := s.AdminReset(u.ID, "resetpass123"); err != nil {
		t.Fatal(err)
	}
	if got, _ := s.Get(u.ID); got.PwdVer != 3 {
		t.Fatalf("after reset pwd_ver=%d, want 3", got.PwdVer)
	}
	if err := s.ChangePassword(u.ID, "newpassword123", "x"); err == nil {
		t.Fatal("short new password must refuse")
	}
	if err := s.ChangePassword("ghost", "a", "12345678"); err == nil {
		t.Fatal("unknown user must refuse")
	}
	if err := s.AdminReset("ghost", "12345678"); err == nil {
		t.Fatal("AdminReset on unknown user must fail")
	}
}

// A minted token carries a 24h expiry: decode the claims and check.
func TestMintedTokenExpiry(t *testing.T) {
	s := testService(t)
	if _, err := s.Setup("admin", "password123"); err != nil {
		t.Fatal(err)
	}
	tok, err := s.Login("admin", "password123")
	if err != nil {
		t.Fatal(err)
	}
	parts := strings.Split(tok, ".")
	if len(parts) != 3 {
		t.Fatalf("token shape: %d parts", len(parts))
	}
	raw, err := base64.RawURLEncoding.DecodeString(parts[1])
	if err != nil {
		t.Fatal(err)
	}
	var c struct {
		Iat int64 `json:"iat"`
		Exp int64 `json:"exp"`
	}
	if err := json.Unmarshal(raw, &c); err != nil {
		t.Fatal(err)
	}
	if c.Exp-c.Iat != 24*3600 {
		t.Fatalf("token ttl=%d, want 86400", c.Exp-c.Iat)
	}
	if c.Exp <= time.Now().Unix() {
		t.Fatal("token must not be minted expired")
	}
	if _, err := s.Verify(tok); err != nil {
		t.Fatalf("fresh token must verify: %v", err)
	}
}

// Verify rejects malformed and tampered tokens, and tokens whose
// subject no longer exists.
func TestVerifyRejectsBadTokens(t *testing.T) {
	s := testService(t)
	if _, err := s.Setup("admin", "password123"); err != nil {
		t.Fatal(err)
	}
	for _, tok := range []string{"", "a.b", "a.b.c.d", "aaa.bbb.ccc"} {
		if _, err := s.Verify(tok); err == nil {
			t.Fatalf("token %q must refuse", tok)
		}
	}
	tok, err := s.Login("admin", "password123")
	if err != nil {
		t.Fatal(err)
	}
	parts := strings.Split(tok, ".")
	// Valid signature over garbage claims still decodes+unmarshals bad.
	garbage := base64.RawURLEncoding.EncodeToString([]byte("{nope"))
	if _, err := s.Verify(parts[0] + "." + garbage + "." + parts[2]); err == nil {
		t.Fatal("resigned claim substitution must refuse")
	}
	// Forge a correctly-signed token for a subject that does not exist.
	// s is same-package, so the test can reuse the real secret.
	forged := s.mint(User{ID: "ghost", Role: RoleUser, PwdVer: 1})
	if _, err := s.Verify(forged); err == nil {
		t.Fatal("token for deleted subject must refuse")
	}
}

// A corrupt stored secret must fail New loudly, not mint weak tokens.
func TestNewRejectsCorruptSecret(t *testing.T) {
	for _, secret := range []string{"!!!not-base64", base64.RawURLEncoding.EncodeToString([]byte("short"))} {
		db, err := kv.Open(t.TempDir())
		if err != nil {
			t.Fatal(err)
		}
		if err := db.Update(func(tx *bolt.Tx) error {
			return tx.Bucket(kv.BMeta).Put([]byte("secret"), []byte(secret))
		}); err != nil {
			t.Fatal(err)
		}
		if _, err := New(db); err == nil {
			t.Fatalf("secret %q must refuse", secret)
		}
		db.Close()
	}
	// A closed db cannot even read meta.
	db, err := kv.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	db.Close()
	if _, err := New(db); err == nil {
		t.Fatal("New on closed db must fail")
	}
	if _, err := New(nil); err == nil {
		t.Fatal("nil db must fail")
	}
}

func TestUsersSurviveReopen(t *testing.T) {
	dir := t.TempDir()
	db, err := kv.Open(dir)
	if err != nil {
		t.Fatal(err)
	}
	s, _ := New(db)
	if _, err := s.Setup("admin", "password123"); err != nil {
		t.Fatal(err)
	}
	if _, err := s.Create("ana", "password123", RoleUser); err != nil {
		t.Fatal(err)
	}
	db.Close()
	db2, err := kv.Open(dir)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db2.Close() })
	s2, err := New(db2)
	if err != nil {
		t.Fatal(err)
	}
	if got := len(s2.List()); got != 2 {
		t.Fatalf("users=%d, want 2 after reopen", got)
	}
	if _, err := s2.Login("ana", "password123"); err != nil {
		t.Fatalf("login after reopen: %v", err)
	}
}

// Boundary precision for validation: 1-char usernames pass, MaxStreams
// 32 is the inclusive bound, 8-char passwords pass on change, and a
// token whose Exp equals now is still valid (strict > comparison).
func TestValidationEdgesExact(t *testing.T) {
	s := testService(t)
	if _, err := s.Setup("a", "password123"); err != nil {
		t.Fatalf("1-char username must pass: %v", err)
	}
	u, err := s.Create("x", "12345678", RoleUser)
	if err != nil {
		t.Fatalf("1-char create must pass: %v", err)
	}
	if err := s.SetPlayback(u.ID, PlaybackPolicy{MaxStreams: 32}); err != nil {
		t.Fatalf("max_streams 32 must pass: %v", err)
	}
	if err := s.SetPlayback(u.ID, PlaybackPolicy{MaxStreams: 33}); err == nil {
		t.Fatal("max_streams 33 must refuse")
	}
	if err := s.ChangePassword(u.ID, "12345678", "abcdefgh"); err != nil {
		t.Fatalf("8-char new password must pass: %v", err)
	}
	if err := s.ChangePassword(u.ID, "abcdefgh", "1234567"); err == nil {
		t.Fatal("7-char new password must refuse")
	}
}

// A token valid at exactly its Exp second is still good: expiry uses >.
func TestVerifyExpiryBoundary(t *testing.T) {
	s := testService(t)
	u, err := s.Setup("admin", "password123")
	if err != nil {
		t.Fatal(err)
	}
	stored, err := s.get(u.ID)
	if err != nil {
		t.Fatal(err)
	}
	mintWith := func(exp int64) string {
		head, _ := json.Marshal(map[string]string{"alg": "HS256", "typ": "JWT"})
		body, _ := json.Marshal(claims{Sub: u.ID, Role: u.Role, PwdV: stored.PwdVer, Iat: time.Now().Unix(), Exp: exp})
		he := base64.RawURLEncoding.EncodeToString(head)
		be := base64.RawURLEncoding.EncodeToString(body)
		mac := hmac.New(sha256.New, s.secret)
		mac.Write([]byte(he + "." + be))
		return he + "." + be + "." + base64.RawURLEncoding.EncodeToString(mac.Sum(nil))
	}
	if _, err := s.Verify(mintWith(time.Now().Unix())); err != nil {
		t.Fatalf("exp==now must still verify: %v", err)
	}
	if _, err := s.Verify(mintWith(time.Now().Unix() - 1)); err == nil {
		t.Fatal("exp<now must be expired")
	}
}
