package store

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-22

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestSaveLoadRoundtrip(t *testing.T) {
	d, err := New(filepath.Join(t.TempDir(), "docs"))
	if err != nil {
		t.Fatal(err)
	}
	if d.Root() == "" {
		t.Fatal("Root must report the backing path")
	}
	type doc struct {
		Name string `json:"name"`
	}
	if err := d.Save("a.json", doc{Name: "v"}); err != nil {
		t.Fatal(err)
	}
	var got doc
	if err := d.Load("a.json", &got); err != nil {
		t.Fatal(err)
	}
	if got.Name != "v" {
		t.Fatalf("roundtrip: %+v", got)
	}
	// The written file is owner-only JSON with a trailing newline.
	raw, err := os.ReadFile(filepath.Join(d.Root(), "a.json"))
	if err != nil {
		t.Fatal(err)
	}
	if raw[len(raw)-1] != '\n' {
		t.Fatal("saved document must end with a newline")
	}
	fi, _ := os.Stat(filepath.Join(d.Root(), "a.json"))
	if fi.Mode().Perm() != 0o600 {
		t.Fatalf("doc perms %o, want 600", fi.Mode().Perm())
	}
	// The temp file is renamed away — nothing partial stays behind.
	entries, _ := os.ReadDir(d.Root())
	for _, e := range entries {
		if strings.Contains(e.Name(), ".tmp") {
			t.Fatalf("stray temp file: %s", e.Name())
		}
	}
}

func TestLoadMissingIsNotFound(t *testing.T) {
	d, _ := New(t.TempDir())
	var v any
	if err := d.Load("absent.json", &v); !errors.Is(err, ErrNotFound) {
		t.Fatalf("missing doc: %v", err)
	}
}

func TestLoadCorruptIsCorruptError(t *testing.T) {
	d, _ := New(t.TempDir())
	if err := os.WriteFile(filepath.Join(d.Root(), "bad.json"), []byte("{nope"), 0o600); err != nil {
		t.Fatal(err)
	}
	var v any
	err := d.Load("bad.json", &v)
	var ce *CorruptError
	if !errors.As(err, &ce) {
		t.Fatalf("want CorruptError, got %v", err)
	}
	if ce.Name != "bad.json" || ce.Unwrap() == nil {
		t.Fatalf("CorruptError must name the doc and wrap the cause: %+v", ce)
	}
	if msg := ce.Error(); !strings.Contains(msg, "corrupt document") || !strings.Contains(msg, "bad.json") {
		t.Fatalf("CorruptError message: %q", msg)
	}
}

func TestSaveMarshalFailureLeavesNothing(t *testing.T) {
	d, _ := New(t.TempDir())
	if err := d.Save("f.json", make(chan int)); err == nil {
		t.Fatal("unmarshalable value must fail")
	}
	if _, err := os.Stat(filepath.Join(d.Root(), "f.json")); !os.IsNotExist(err) {
		t.Fatal("failed save must not leave the document behind")
	}
}

func TestNewFailsUnderAFile(t *testing.T) {
	file := filepath.Join(t.TempDir(), "f")
	if err := os.WriteFile(file, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, err := New(filepath.Join(file, "sub")); err == nil {
		t.Fatal("New under a file path must fail")
	}
}
