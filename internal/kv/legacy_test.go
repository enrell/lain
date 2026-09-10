package kv

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	bolt "go.etcd.io/bbolt"
)

// TestLegacyImport migrates a v0.1 JSON data dir into buckets once,
// then renames the sources so a second boot is a no-op.
func TestLegacyImport(t *testing.T) {
	dir := t.TempDir()
	write := func(name, body string) {
		t.Helper()
		if err := os.WriteFile(filepath.Join(dir, name), []byte(body), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	write("secret.json", `"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="`)
	write("users.json", `[{"id":"user-admin","username":"admin","pass_hash":"eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4","created_at":1}]`)
	write("libraries.json", `[{"id":"lib-1","name":"Anime","type":"anime","path":"/media","source":"lain-source-filesystem","created_at":1}]`)
	write("catalog.json", `{"abc123":{"id":"abc123","library_id":"lib-1","kind":"episode","title":"Show","episode":2,"file_path":"/media/e2.mkv","origin":"x","provenance":"y","updated_at":1}}`)
	write("userstate.json", `{"user-admin\u0000123":{"item_id":"123","user_id":"user-admin","position_sec":5,"updated_at":1}}`)

	docs := dirDocs{root: dir}
	db, err := Open(dir)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	if err := ImportLegacy(db, docs); err != nil {
		t.Fatal(err)
	}
	if err := db.View(func(tx *bolt.Tx) error {
		if tx.Bucket(BUsers).Stats().KeyN != 1 {
			t.Errorf("users=%d, want 1", tx.Bucket(BUsers).Stats().KeyN)
		}
		if tx.Bucket(BLibraries).Stats().KeyN != 1 {
			t.Errorf("libraries wrong")
		}
		if tx.Bucket(BItems).Stats().KeyN != 1 || tx.Bucket(BItemsByLib).Stats().KeyN != 1 {
			t.Errorf("items/index wrong")
		}
		if tx.Bucket(BProgress).Stats().KeyN != 1 {
			t.Errorf("progress wrong")
		}
		var u struct {
			Role   string `json:"role"`
			PwdVer uint64 `json:"pwd_ver"`
		}
		if err := GetJSON(tx, BUsers, []byte("user-admin"), &u); err != nil {
			t.Errorf("admin missing: %v", err)
		} else if u.Role != "admin" || u.PwdVer != 1 {
			t.Errorf("admin record wrong: %+v", u)
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"users.json", "secret.json", "libraries.json", "catalog.json", "userstate.json"} {
		if _, err := os.Stat(filepath.Join(dir, name)); !os.IsNotExist(err) {
			t.Errorf("%s must be renamed after import", name)
		}
		if _, err := os.Stat(filepath.Join(dir, name+".imported")); err != nil {
			t.Errorf("%s.imported missing", name)
		}
	}
	// Second run is a no-op (idempotent, no duplicates).
	if err := ImportLegacy(db, docs); err != nil {
		t.Fatal(err)
	}
}

// dirDocs is a DocStore over a plain directory.
type dirDocs struct{ root string }

func (d dirDocs) Root() string { return d.root }

func (d dirDocs) Load(name string, v any) error {
	raw, err := os.ReadFile(filepath.Join(d.root, name))
	if err != nil {
		if os.IsNotExist(err) {
			return ErrNotFound
		}
		return err
	}
	return json.Unmarshal(raw, v)
}
