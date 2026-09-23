package matrix

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-22

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestNewManifestShape(t *testing.T) {
	m := NewManifest("lain-catalog", "1", []string{"cap.a", "cap.b"}, "bin/lain-catalog", []string{"--sock", "{sock}"})
	if m.ID != "lain-catalog" || m.Version != "1" {
		t.Fatalf("manifest: %+v", m)
	}
	if len(m.Capabilities) != 2 || m.Capabilities[0] != "cap.a" {
		t.Fatalf("caps: %v", m.Capabilities)
	}
	if m.Execution.Kind != "process" || m.Execution.Entrypoint != "bin/lain-catalog" {
		t.Fatalf("execution: %+v", m.Execution)
	}
	if len(m.Execution.Args) != 2 || m.Execution.Args[1] != "{sock}" {
		t.Fatalf("args: %v", m.Execution.Args)
	}
}

func TestExportManifestsWritesOneJSONPerEntry(t *testing.T) {
	dir := t.TempDir()
	entries := []Entry{
		{Manifest: NewManifest("a", "1", nil, "bin/a", nil), Trusted: true},
		{Manifest: NewManifest("b", "2", []string{"cap"}, "bin/b", []string{"--x"})},
	}
	if err := ExportManifests(filepath.Join(dir, "out"), entries); err != nil {
		t.Fatal(err)
	}
	for _, e := range entries {
		raw, err := os.ReadFile(filepath.Join(dir, "out", e.Manifest.ID+".json"))
		if err != nil {
			t.Fatal(err)
		}
		if raw[len(raw)-1] != '\n' {
			t.Fatal("manifest must end with a newline")
		}
		var m Manifest
		if err := json.Unmarshal(raw, &m); err != nil || m.ID != e.Manifest.ID {
			t.Fatalf("manifest %s did not round-trip: %v %+v", e.Manifest.ID, err, m)
		}
	}
	if err := ExportManifests(filepath.Join(dir, "x", "y"), nil); err != nil {
		t.Fatalf("empty export must still create the dir: %v", err)
	}
}

func TestDoctorAbsentBinary(t *testing.T) {
	r := Doctor(filepath.Join(t.TempDir(), "no-such-bin"))
	if r.BinaryPresent || r.HelpOK {
		t.Fatalf("absent binary: %+v", r)
	}
	if r.Note == "" {
		t.Fatal("absent binary must carry a note")
	}
	// A directory is not a binary.
	r = Doctor(t.TempDir())
	if r.BinaryPresent {
		t.Fatalf("directory must not count as a binary: %+v", r)
	}
}

func TestDoctorNotExecutable(t *testing.T) {
	f := filepath.Join(t.TempDir(), "bin")
	if err := os.WriteFile(f, []byte("#!/bin/sh\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	r := Doctor(f)
	if r.BinaryPresent || r.HelpOK {
		t.Fatalf("non-executable: %+v", r)
	}
}

// Any exec at all counts as present: a usage-error exit still proves
// the binary runs.
func TestDoctorExecutable(t *testing.T) {
	dir := t.TempDir()
	ok := filepath.Join(dir, "matrix-ok")
	if err := os.WriteFile(ok, []byte("#!/bin/sh\nexit 0\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	r := Doctor(ok)
	if !r.BinaryPresent || !r.HelpOK {
		t.Fatalf("working binary: %+v", r)
	}
	if r.Note != "matrix-managed available" {
		t.Fatalf("clean fingerprint note: %q", r.Note)
	}
	fail := filepath.Join(dir, "matrix-fail")
	if err := os.WriteFile(fail, []byte("#!/bin/sh\nexit 1\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	r = Doctor(fail)
	if !r.BinaryPresent || !r.HelpOK {
		t.Fatalf("usage-error binary still counts as present: %+v", r)
	}
	if r.Note == "matrix-managed available" {
		t.Fatalf("usage-error path must carry its own note: %q", r.Note)
	}
}
