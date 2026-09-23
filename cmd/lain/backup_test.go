package main


import (
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	"github.com/enrell/lain/internal/backup"
	"github.com/enrell/lain/internal/kv"
)

func testConfigHome(t *testing.T) {
	t.Helper()
	t.Setenv("HOME", t.TempDir())
	t.Setenv("XDG_CONFIG_HOME", "")
}

// A login config pointing at a live server takes the online path: the
// snapshot streams through the API into a complete, validatable backup
// directory.
func TestCmdBackupOnline(t *testing.T) {
	testConfigHome(t)
	dbDir := t.TempDir()
	db, err := kv.Open(dbDir)
	if err != nil {
		t.Fatal(err)
	}
	if err := db.Close(); err != nil {
		t.Fatal(err)
	}
	dbBytes, err := os.ReadFile(filepath.Join(dbDir, backup.DBFile))
	if err != nil {
		t.Fatal(err)
	}
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/health":
			w.WriteHeader(200)
		case "/api/admin/backup":
			w.Write(dbBytes)
		case "/api/plugins":
			w.Header().Set("Content-Type", "application/json")
			w.Write([]byte(`{"composition":[{"capability":"cap.a"}]}`))
		default:
			w.WriteHeader(404)
		}
	}))
	defer ts.Close()
	if err := saveConfig(clientConfig{Server: ts.URL, Token: "tok"}); err != nil {
		t.Fatal(err)
	}
	out := filepath.Join(t.TempDir(), "bk")
	dir, err := cmdBackupOnline(nil, out)
	if err != nil {
		t.Fatal(err)
	}
	for _, f := range []string{backup.DBFile, backup.CompositionFile, backup.ManifestFile} {
		if _, err := os.Stat(filepath.Join(dir, f)); err != nil {
			t.Fatalf("backup missing %s: %v", f, err)
		}
	}
	if _, err := backup.Validate(dir); err != nil {
		t.Fatalf("online backup must validate: %v", err)
	}
}

// No saved login → os.ErrNotExist so cmdBackup falls back to offline.
func TestCmdBackupOnlineNeedsLogin(t *testing.T) {
	testConfigHome(t)
	if _, err := cmdBackupOnline(nil, t.TempDir()); !os.IsNotExist(err) {
		t.Fatalf("no config must report not-exist, got %v", err)
	}
}

// A saved login whose server is unreachable still reports a real error
// (not ErrNotExist) so cmdBackup can announce the fallback.
func TestCmdBackupOnlineServerDown(t *testing.T) {
	testConfigHome(t)
	if err := saveConfig(clientConfig{Server: "http://127.0.0.1:1", Token: "tok"}); err != nil {
		t.Fatal(err)
	}
	if _, err := cmdBackupOnline(nil, t.TempDir()); err == nil || os.IsNotExist(err) {
		t.Fatalf("unreachable server: want real error, got %v", err)
	}
}

// The offline path snapshots a closed data dir without any login.
func TestCmdBackupOffline(t *testing.T) {
	testConfigHome(t) // no config: online path reports not-exist, falls back
	dataDir := t.TempDir()
	db, err := kv.Open(dataDir)
	if err != nil {
		t.Fatal(err)
	}
	if err := db.Close(); err != nil {
		t.Fatal(err)
	}
	out := filepath.Join(t.TempDir(), "backups")
	if err := cmdBackup([]string{"--data-dir", dataDir, "--out", out}); err != nil {
		t.Fatal(err)
	}
	if _, err := backup.Validate(out); err != nil {
		t.Fatalf("offline backup must validate: %v", err)
	}
}

func TestCmdRestoreUsageAndHappyPath(t *testing.T) {
	if err := cmdRestore(nil); err == nil {
		t.Fatal("restore without a dir must fail usage")
	}
	// A validated backup restores into an empty data dir.
	srcData := t.TempDir()
	db, err := kv.Open(srcData)
	if err != nil {
		t.Fatal(err)
	}
	db.Close()
	bkDir, err := backup.Create(srcData, filepath.Join(t.TempDir(), "bk"))
	if err != nil {
		t.Fatal(err)
	}
	dst := filepath.Join(t.TempDir(), "data")
	if err := cmdRestore([]string{bkDir, "--data-dir", dst}); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(dst, backup.DBFile)); err != nil {
		t.Fatalf("restored db missing: %v", err)
	}
	// Restoring over a live database refuses.
	if err := cmdRestore([]string{bkDir, "--data-dir", dst}); err == nil {
		t.Fatal("restore over an existing db must refuse")
	}
}
