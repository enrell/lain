package backup

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/enrell/lain/internal/auth"
	"github.com/enrell/lain/internal/kv"
)

func TestBackupRestoreRoundtrip(t *testing.T) {
	src := t.TempDir()
	db, err := kv.Open(src)
	if err != nil {
		t.Fatal(err)
	}
	a, err := auth.New(db)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := a.Setup("admin", "password123"); err != nil {
		t.Fatal(err)
	}
	if _, err := a.Create("ana", "password123", auth.RoleUser); err != nil {
		t.Fatal(err)
	}
	db.Close()

	out, err := Create(src, t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	m, err := Validate(out)
	if err != nil {
		t.Fatal(err)
	}
	if m.Users != 2 {
		t.Fatalf("manifest users=%d, want 2", m.Users)
	}

	// Restore refuses over a live database...
	if err := Restore(out, src); err == nil {
		t.Fatal("restore over live db must refuse")
	}
	// ...and works into an empty dir, bootable with the same credentials.
	dst := t.TempDir()
	if err := Restore(out, dst); err != nil {
		t.Fatal(err)
	}
	db2, err := kv.Open(dst)
	if err != nil {
		t.Fatal(err)
	}
	defer db2.Close()
	a2, err := auth.New(db2)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := a2.Login("ana", "password123"); err != nil {
		t.Fatalf("login on restored db: %v", err)
	}
}

func TestBackupEmptyDirFails(t *testing.T) {
	if _, err := Create(t.TempDir(), t.TempDir()); err == nil {
		t.Fatal("backup of nothing must fail")
	}
}

func TestValidateRejectsGarbage(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "manifest.json"), []byte("{nope"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := Validate(dir); err == nil {
		t.Fatal("corrupt manifest must fail validation")
	}
	if err := Restore(dir, t.TempDir()); err == nil {
		t.Fatal("restore of invalid backup must fail")
	}
}
