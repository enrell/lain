// Package backup owns disaster recovery: consistent snapshots and
// validated restores. The database file supports online snapshots via
// a read transaction, so backup never stops the server. Restore never
// overwrites a live database — like the Matrix rule, removal is an
// explicit operator act first.
package backup

import (
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"time"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/kv"
)

// Manifest describes one backup directory.
type Manifest struct {
	Tool      string    `json:"tool"`
	Version   int       `json:"version"`
	CreatedAt time.Time `json:"created_at"`
	Users     int       `json:"users"`
	Libraries int       `json:"libraries"`
	Items     int       `json:"items"`
}

// Files in a backup directory.
const (
	DBFile          = "lain.db"
	CompositionFile = "composition.json"
	ManifestFile    = "manifest.json"
)

// Create snapshots dataDir into a new timestamped directory under out
// root (or out itself when it does not exist yet). The server may keep
// running: the snapshot comes from one read transaction.
func Create(dataDir, out string) (string, error) {
	if _, err := os.Stat(filepath.Join(dataDir, DBFile)); err != nil {
		return "", fmt.Errorf("nothing to back up in %s", dataDir)
	}
	dir := out
	if fi, err := os.Stat(out); err == nil && fi.IsDir() {
		dir = filepath.Join(out, "lain-backup-"+time.Now().Format("20060102-150405"))
	}
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return "", err
	}
	db, err := bolt.Open(filepath.Join(dataDir, DBFile), 0o600, &bolt.Options{ReadOnly: true, Timeout: 2 * time.Second})
	if err != nil {
		return "", fmt.Errorf("cannot open %s (server running? stop it or back up online): %w", filepath.Join(dataDir, DBFile), err)
	}
	defer db.Close()
	if err := Snapshot(db, filepath.Join(dir, DBFile)); err != nil {
		os.RemoveAll(dir)
		return "", err
	}
	if raw, err := os.ReadFile(filepath.Join(dataDir, CompositionFile)); err == nil {
		if err := os.WriteFile(filepath.Join(dir, CompositionFile), raw, 0o600); err != nil {
			os.RemoveAll(dir)
			return "", err
		}
	}
	if _, err := WriteManifest(filepath.Join(dir, DBFile), dir); err != nil {
		os.RemoveAll(dir)
		return "", err
	}
	return dir, nil
}

// Snapshot copies the live database bytes from an open handle into
// dst (0600). Used by Create for the offline path.
func Snapshot(db *bolt.DB, dst string) error {
	return db.View(func(tx *bolt.Tx) error {
		out, err := os.OpenFile(dst, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o600)
		if err != nil {
			return err
		}
		defer out.Close()
		_, err = tx.WriteTo(out)
		return err
	})
}

// WriteManifest counts the records in dbPath (opened read-only) and
// writes manifest.json into dir.
func WriteManifest(dbPath, dir string) (Manifest, error) {
	var m Manifest
	db, err := bolt.Open(dbPath, 0o600, &bolt.Options{ReadOnly: true, Timeout: 2 * time.Second})
	if err != nil {
		return m, err
	}
	defer db.Close()
	err = db.View(func(tx *bolt.Tx) error {
		m.Users = tx.Bucket(kv.BUsers).Stats().KeyN
		m.Libraries = tx.Bucket(kv.BLibraries).Stats().KeyN
		m.Items = tx.Bucket(kv.BItems).Stats().KeyN
		return nil
	})
	if err != nil {
		return m, err
	}
	m.Tool = "lain backup"
	m.Version = 1
	m.CreatedAt = time.Now()
	raw, _ := json.MarshalIndent(m, "", "  ")
	if err := os.WriteFile(filepath.Join(dir, ManifestFile), append(raw, '\n'), 0o600); err != nil {
		return m, err
	}
	return m, nil
}

// Validate opens the backup read-only and checks structure. A backup
// that fails here must never be restored over anything.
func Validate(dir string) (Manifest, error) {
	var m Manifest
	raw, err := os.ReadFile(filepath.Join(dir, ManifestFile))
	if err != nil {
		return m, fmt.Errorf("not a lain backup: %w", err)
	}
	if err := json.Unmarshal(raw, &m); err != nil {
		return m, fmt.Errorf("corrupt manifest: %w", err)
	}
	if m.Version != 1 {
		return m, fmt.Errorf("unsupported backup version %d", m.Version)
	}
	db, err := bolt.Open(filepath.Join(dir, DBFile), 0o600, &bolt.Options{ReadOnly: true, Timeout: 2 * time.Second})
	if err != nil {
		return m, fmt.Errorf("backup db unreadable: %w", err)
	}
	defer db.Close()
	return m, db.View(func(tx *bolt.Tx) error {
		for _, b := range [][]byte{kv.BUsers, kv.BLibraries, kv.BItems, kv.BProgress, kv.BMeta} {
			if tx.Bucket(b) == nil {
				return fmt.Errorf("backup db missing bucket %s", b)
			}
		}
		return nil
	})
}

// Restore copies a validated backup into dataDir. It refuses when a
// live database exists: move it aside first, explicitly.
func Restore(backupDir, dataDir string) error {
	if _, err := Validate(backupDir); err != nil {
		return err
	}
	if _, err := os.Stat(filepath.Join(dataDir, DBFile)); err == nil {
		return fmt.Errorf("refusing: %s already exists (move it aside first)", filepath.Join(dataDir, DBFile))
	}
	if err := os.MkdirAll(dataDir, 0o700); err != nil {
		return err
	}
	if err := copyFile(filepath.Join(backupDir, DBFile), filepath.Join(dataDir, DBFile), 0o600); err != nil {
		return err
	}
	if _, err := os.Stat(filepath.Join(backupDir, CompositionFile)); err == nil {
		if err := copyFile(filepath.Join(backupDir, CompositionFile), filepath.Join(dataDir, CompositionFile), 0o600); err != nil {
			return err
		}
	}
	return nil
}

func copyFile(src, dst string, perm os.FileMode) error {
	in, err := os.Open(src)
	if err != nil {
		return err
	}
	defer in.Close()
	out, err := os.OpenFile(dst, os.O_CREATE|os.O_EXCL|os.O_WRONLY, perm)
	if err != nil {
		return err
	}
	defer out.Close()
	_, err = io.Copy(out, in)
	return err
}
