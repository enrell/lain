package list

import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/kv"
)

func newSvc(t *testing.T) *Service {
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

func entries(ids ...string) []contracts.ListEntry {
	out := []contracts.ListEntry{}
	for _, id := range ids {
		out = append(out, contracts.ListEntry{
			RemoteID: id, Title: "Title " + id,
			MediaType: contracts.ListMediaAnime, Status: contracts.ListStatusCurrent,
		})
	}
	return out
}

func TestPutPlatformUpsertsAndRemoves(t *testing.T) {
	s := newSvc(t)
	st, err := s.PutPlatform(PutPlatformInput{UserID: "u", Platform: "anilist", Entries: entries("1", "2", "3")})
	if err != nil {
		t.Fatal(err)
	}
	if st.Upserted != 3 || st.Removed != 0 {
		t.Fatalf("stats=%+v", st)
	}
	// Second sync drops "2": remote is authoritative.
	st, err = s.PutPlatform(PutPlatformInput{UserID: "u", Platform: "anilist", Entries: entries("1", "3", "4")})
	if err != nil {
		t.Fatal(err)
	}
	if st.Upserted != 3 || st.Removed != 1 {
		t.Fatalf("stats=%+v", st)
	}
	got := s.List(ListInput{UserID: "u"})
	ids := map[string]bool{}
	for _, e := range got {
		ids[e.RemoteID] = true
		if e.UserID != "u" || e.Platform != "anilist" || e.ID != "anilist:"+e.RemoteID {
			t.Fatalf("entry not stamped: %+v", e)
		}
	}
	for _, want := range []string{"1", "3", "4"} {
		if !ids[want] {
			t.Fatalf("missing %s in %+v", want, got)
		}
	}
}

func TestPutPlatformScopedPerUserAndPlatform(t *testing.T) {
	s := newSvc(t)
	_, _ = s.PutPlatform(PutPlatformInput{UserID: "u1", Platform: "anilist", Entries: entries("1")})
	_, _ = s.PutPlatform(PutPlatformInput{UserID: "u2", Platform: "anilist", Entries: entries("2")})
	// u1's empty sync must not touch u2's entries.
	st, err := s.PutPlatform(PutPlatformInput{UserID: "u1", Platform: "anilist", Entries: nil})
	if err != nil {
		t.Fatal(err)
	}
	if st.Removed != 1 {
		t.Fatalf("removed=%d", st.Removed)
	}
	if got := s.List(ListInput{UserID: "u2"}); len(got) != 1 {
		t.Fatalf("u2 lost entries: %+v", got)
	}
}

func TestListFilters(t *testing.T) {
	s := newSvc(t)
	es := entries("1", "2")
	es[0].MediaType = contracts.ListMediaManga
	es[1].Status = contracts.ListStatusCompleted
	_, _ = s.PutPlatform(PutPlatformInput{UserID: "u", Platform: "anilist", Entries: es})
	if got := s.List(ListInput{UserID: "u", Type: contracts.ListMediaManga}); len(got) != 1 || got[0].RemoteID != "1" {
		t.Fatalf("type filter: %+v", got)
	}
	if got := s.List(ListInput{UserID: "u", Status: contracts.ListStatusCompleted}); len(got) != 1 || got[0].RemoteID != "2" {
		t.Fatalf("status filter: %+v", got)
	}
}

func TestPutPlatformRejectsBadInput(t *testing.T) {
	s := newSvc(t)
	if _, err := s.PutPlatform(PutPlatformInput{UserID: "u", Platform: "anilist", Entries: []contracts.ListEntry{{Title: "no remote id"}}}); err == nil {
		t.Fatal("accepted entry without remote_id")
	} else if _, ok := err.(*core.Error); !ok {
		t.Fatalf("untyped error: %T", err)
	}
}

func TestDeletePlatform(t *testing.T) {
	s := newSvc(t)
	_, _ = s.PutPlatform(PutPlatformInput{UserID: "u", Platform: "anilist", Entries: entries("1", "2")})
	_, _ = s.PutPlatform(PutPlatformInput{UserID: "u", Platform: "trakt", Entries: entries("9")})
	n, err := s.DeletePlatform("u", "anilist")
	if err != nil || n != 2 {
		t.Fatalf("removed=%d err=%v", n, err)
	}
	got := s.List(ListInput{UserID: "u"})
	if len(got) != 1 || got[0].Platform != "trakt" {
		t.Fatalf("wrong entries left: %+v", got)
	}
}

func TestAccountCRUD(t *testing.T) {
	s := newSvc(t)
	if _, err := s.GetAccount("u", "anilist"); err == nil {
		t.Fatal("get missing account did not fail")
	} else if ce, ok := err.(*core.Error); !ok || ce.Code != "not-found" {
		t.Fatalf("want typed not-found, got %T %v", err, err)
	}
	a := contracts.LinkedAccount{UserID: "u", Platform: "anilist", RemoteUserID: "7", RemoteUsername: "lain", Token: "secret-token"}
	if err := s.PutAccount(a); err != nil {
		t.Fatal(err)
	}
	got, err := s.GetAccount("u", "anilist")
	if err != nil {
		t.Fatal(err)
	}
	if got.RemoteUsername != "lain" || got.Token != "secret-token" {
		t.Fatalf("account: %+v", got)
	}
	if all := s.ListAccounts(""); len(all) != 1 {
		t.Fatalf("all accounts: %+v", all)
	}
	if own := s.ListAccounts("u"); len(own) != 1 {
		t.Fatalf("user accounts: %+v", own)
	}
	if own := s.ListAccounts("other"); len(own) != 0 {
		t.Fatalf("other user accounts: %+v", own)
	}
	ok, err := s.DeleteAccount("u", "anilist")
	if err != nil || !ok {
		t.Fatalf("delete: %v %v", ok, err)
	}
	if _, err := s.GetAccount("u", "anilist"); err == nil {
		t.Fatal("account survived delete")
	}
}

func TestLinkedAccountPublicHidesToken(t *testing.T) {
	a := contracts.LinkedAccount{UserID: "u", Platform: "anilist", Token: "secret"}
	if a.Public().Token != "" {
		t.Fatal("Public() leaked token")
	}
	if a.TokenExpired(100) {
		t.Fatal("zero expiry read as expired")
	}
	a.TokenExpiresAt = 50
	if !a.TokenExpired(50) {
		t.Fatal("expiry boundary not detected")
	}
}
