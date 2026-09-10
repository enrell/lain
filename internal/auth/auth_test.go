package auth

import (
	"testing"

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
