package backup


import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/enrell/lain/internal/auth"
	"github.com/enrell/lain/internal/kv"
)

// mkDBDir builds a data dir holding a real (closed) database.
func mkDBDir(t *testing.T) string {
	t.Helper()
	src := t.TempDir()
	db, err := kv.Open(src)
	if err != nil {
		t.Fatal(err)
	}
	if err := db.Close(); err != nil {
		t.Fatal(err)
	}
	return src
}

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

// When out does not exist it is used verbatim; when it exists as a
// directory the backup goes into a timestamped child.
func TestCreateOutDirSemantics(t *testing.T) {
	src := mkDBDir(t)
	fresh := filepath.Join(t.TempDir(), "new-backup")
	out, err := Create(src, fresh)
	if err != nil {
		t.Fatal(err)
	}
	if out != fresh {
		t.Fatalf("nonexistent out must be used verbatim: %q != %q", out, fresh)
	}
	parent := t.TempDir()
	out, err = Create(src, parent)
	if err != nil {
		t.Fatal(err)
	}
	if filepath.Dir(out) != parent || filepath.Base(out) == parent {
		t.Fatalf("existing dir must get a timestamped child: %q", out)
	}
	// An existing non-directory path can never hold a backup.
	file := filepath.Join(t.TempDir(), "afile")
	if err := os.WriteFile(file, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := Create(src, file); err == nil {
		t.Fatal("out as existing file must fail")
	}
}

// A database file that is not bbolt fails open, loudly.
func TestCreateCorruptDBFails(t *testing.T) {
	src := t.TempDir()
	if err := os.WriteFile(filepath.Join(src, DBFile), []byte("not a database"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := Create(src, t.TempDir()); err == nil {
		t.Fatal("corrupt db must fail")
	} else if !strings.Contains(err.Error(), "cannot open") {
		t.Fatalf("error must name the open failure: %v", err)
	}
}

// composition.json rides along when present, and its copy must land
// byte-identical in the restored data dir.
func TestCompositionFileCopiedThrough(t *testing.T) {
	src := mkDBDir(t)
	want := []byte(`{"providers":["a","b"]}`)
	if err := os.WriteFile(filepath.Join(src, CompositionFile), want, 0o600); err != nil {
		t.Fatal(err)
	}
	out, err := Create(src, filepath.Join(t.TempDir(), "bk"))
	if err != nil {
		t.Fatal(err)
	}
	got, err := os.ReadFile(filepath.Join(out, CompositionFile))
	if err != nil || string(got) != string(want) {
		t.Fatalf("composition.json not in backup: %v %q", err, got)
	}
	dst := t.TempDir()
	if err := Restore(out, dst); err != nil {
		t.Fatal(err)
	}
	got, err = os.ReadFile(filepath.Join(dst, CompositionFile))
	if err != nil || string(got) != string(want) {
		t.Fatalf("composition.json not restored: %v %q", err, got)
	}
}

// The timestamped backup dir may already contain a composition.json
// that is a directory — the write must fail, not pass silently.
func TestCreateCompositionWriteFailure(t *testing.T) {
	src := mkDBDir(t)
	if err := os.WriteFile(filepath.Join(src, CompositionFile), []byte("{}"), 0o600); err != nil {
		t.Fatal(err)
	}
	parent := t.TempDir()
	// Cover this second and the next so whichever one Create lands in,
	// composition.json is already there as a directory.
	now := time.Now()
	for _, ts := range []time.Time{now, now.Add(time.Second)} {
		clash := filepath.Join(parent, "lain-backup-"+ts.Format("20060102-150405"), CompositionFile)
		if err := os.MkdirAll(clash, 0o700); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := Create(src, parent); err == nil {
		t.Fatal("composition write failure must propagate")
	}
}

func TestWriteManifestFailures(t *testing.T) {
	src := mkDBDir(t)
	dbPath := filepath.Join(src, DBFile)
	// Unreadable db path fails open.
	if _, err := WriteManifest(filepath.Join(src, "nope.db"), t.TempDir()); err == nil {
		t.Fatal("unreadable db must fail")
	}
	// A manifest path that is a directory fails the write regardless of
	// the test user's privileges.
	bad := t.TempDir()
	if err := os.MkdirAll(filepath.Join(bad, ManifestFile), 0o700); err != nil {
		t.Fatal(err)
	}
	if _, err := WriteManifest(dbPath, bad); err == nil {
		t.Fatal("manifest write failure must propagate")
	}
}

// A valid manifest over an unreadable database is still not a backup.
func TestValidateUnreadableDB(t *testing.T) {
	dir := t.TempDir()
	manifest := []byte(`{"tool":"lain backup","version":1}`)
	if err := os.WriteFile(filepath.Join(dir, ManifestFile), manifest, 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, DBFile), []byte("garbage"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := Validate(dir); err == nil {
		t.Fatal("corrupt db must fail validation")
	}
	// Manifest version drift is rejected too.
	bad := filepath.Join(t.TempDir(), "v2")
	if err := os.MkdirAll(bad, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(bad, ManifestFile), []byte(`{"version":2}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := Validate(bad); err == nil {
		t.Fatal("version != 1 must fail")
	}
}

// Restore must surface copy failures: a broken symlink where the
// database would land defeats the stat check but fails O_EXCL create.
func TestRestoreCopyFailure(t *testing.T) {
	src := mkDBDir(t)
	out, err := Create(src, filepath.Join(t.TempDir(), "bk"))
	if err != nil {
		t.Fatal(err)
	}
	dst := t.TempDir()
	if err := os.Symlink("/nonexistent-target", filepath.Join(dst, DBFile)); err != nil {
		t.Fatal(err)
	}
	if err := Restore(out, dst); err == nil {
		t.Fatal("copy failure must propagate")
	}
}

// Same for the composition copy: validate passes, db copies, then the
// composition write hits an unwritable target.
func TestRestoreCompositionCopyFailure(t *testing.T) {
	src := mkDBDir(t)
	if err := os.WriteFile(filepath.Join(src, CompositionFile), []byte("{}"), 0o600); err != nil {
		t.Fatal(err)
	}
	out, err := Create(src, filepath.Join(t.TempDir(), "bk"))
	if err != nil {
		t.Fatal(err)
	}
	dst := t.TempDir()
	if err := os.Symlink("/nonexistent-target", filepath.Join(dst, CompositionFile)); err != nil {
		t.Fatal(err)
	}
	if err := Restore(out, dst); err == nil {
		t.Fatal("composition copy failure must propagate")
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
