// Package store is atomic file persistence for the trusted base:
// users, libraries, catalog, userstate, secrets and composition.
//
// Writes are temp-file + rename so a crash never leaves a half-written
// JSON document. A corrupt file is reported, never silently ignored.
package store

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
)

// ErrNotFound is returned when no document exists under the name.
var ErrNotFound = errors.New("not found")

// Dir is a rooted document directory (0600 files, 0700 dir).
type Dir struct {
	root string
}

// New creates the directory if needed.
func New(root string) (*Dir, error) {
	if err := os.MkdirAll(root, 0o700); err != nil {
		return nil, err
	}
	return &Dir{root: root}, nil
}

// Root returns the backing path (for diagnostics only).
func (d *Dir) Root() string { return d.root }

// Save marshals v as indented JSON atomically.
func (d *Dir) Save(name string, v any) error {
	raw, err := json.MarshalIndent(v, "", "  ")
	if err != nil {
		return err
	}
	tmp, err := os.CreateTemp(d.root, name+".tmp-*")
	if err != nil {
		return err
	}
	tmpName := tmp.Name()
	if _, err := tmp.Write(append(raw, '\n')); err != nil {
		tmp.Close()
		os.Remove(tmpName)
		return err
	}
	if err := tmp.Chmod(0o600); err != nil {
		tmp.Close()
		os.Remove(tmpName)
		return err
	}
	if err := tmp.Close(); err != nil {
		os.Remove(tmpName)
		return err
	}
	return os.Rename(tmpName, filepath.Join(d.root, name))
}

// Load unmarshals a document. Missing file maps to ErrNotFound.
func (d *Dir) Load(name string, v any) error {
	raw, err := os.ReadFile(filepath.Join(d.root, name))
	if err != nil {
		if os.IsNotExist(err) {
			return ErrNotFound
		}
		return err
	}
	if err := json.Unmarshal(raw, v); err != nil {
		return &CorruptError{Name: name, Err: err}
	}
	return nil
}

// CorruptError marks a present-but-unparseable document.
type CorruptError struct {
	Name string
	Err  error
}

func (e *CorruptError) Error() string { return "corrupt document " + e.Name + ": " + e.Err.Error() }
func (e *CorruptError) Unwrap() error { return e.Err }
