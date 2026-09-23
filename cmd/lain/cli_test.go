package main


import (
	"errors"
	"math"
	"os"
	"path/filepath"
	"testing"
)

func TestFlagParsesBothForms(t *testing.T) {
	args := []string{"--data-dir", "/x", "--port=1234", "pos"}
	if got := flag(args, "data-dir", ""); got != "/x" {
		t.Fatalf("spaced flag: %q", got)
	}
	if got := flag(args, "port", ""); got != "1234" {
		t.Fatalf("equals flag: %q", got)
	}
	if got := flag(args, "absent", "dflt"); got != "dflt" {
		t.Fatalf("default: %q", got)
	}
	// A flag name alone at the end is not a value.
	if got := flag([]string{"--port"}, "port", "9"); got != "9" {
		t.Fatalf("valueless flag: %q", got)
	}
	// "--p" must not match "--port".
	if got := flag([]string{"--p", "x"}, "port", "9"); got != "9" {
		t.Fatalf("prefix collision: %q", got)
	}
	// "--port=" with an empty value falls back to the default: the flag
	// parser requires a non-empty value after the equals sign.
	if got := flag([]string{"--port="}, "port", "9"); got != "9" {
		t.Fatalf("empty equals: %q", got)
	}
}

func TestPositionalSkipsFlagsAndTheirValues(t *testing.T) {
	got := positional([]string{"query", "--data-dir", "/x", "--pick", "3", "--dry-run", "other", "-v"})
	if len(got) != 2 || got[0] != "query" || got[1] != "other" {
		t.Fatalf("positional: %v", got)
	}
	if len(positional(nil)) != 0 {
		t.Fatal("no args must yield no positionals")
	}
}

func TestHasFlag(t *testing.T) {
	if !hasFlag([]string{"--dry-run"}, "dry-run") {
		t.Fatal("present flag not found")
	}
	if hasFlag([]string{"--dry-run=x"}, "dry-run") || hasFlag([]string{"--dry"}, "dry-run") {
		t.Fatal("hasFlag must match the exact flag only")
	}
}

func TestBenchInt(t *testing.T) {
	if got := benchInt([]string{"--runs", "5"}, "runs", 3); got != 5 {
		t.Fatalf("runs: %d", got)
	}
	for _, args := range [][]string{
		{"--runs", "abc"},
		{"--runs", "0"},
		{"--runs", "-2"},
		{"--runs"}, // valueless
		{},
	} {
		if got := benchInt(args, "runs", 3); got != 3 {
			t.Fatalf("benchInt(%v)=%d, want default 3", args, got)
		}
	}
}

func TestHumanBytesCLI(t *testing.T) {
	for in, want := range map[uint64]string{
		0:              "0 B",
		512:            "512 B",
		1023:           "1023 B",
		1024:           "1.0 KiB",
		5 << 20:        "5.0 MiB",
		3 << 30:        "3.0 GiB",
		1 << 40:        "1024.0 GiB", // beyond GiB stays in GiB
		math.MaxUint64: "17179869184.0 GiB",
	} {
		if got := humanBytesCLI(in); got != want {
			t.Fatalf("humanBytesCLI(%d)=%q, want %q", in, got, want)
		}
	}
}

func TestUsageErrorCarriesMessage(t *testing.T) {
	err := errUsage("usage: x")
	if err.Error() != "usage: x" {
		t.Fatalf("usage error: %q", err.Error())
	}
	var ue *usageError
	if !errors.As(err, &ue) {
		t.Fatal("errUsage must return *usageError")
	}
}

func TestCmdBenchRejectsBadInput(t *testing.T) {
	if err := cmdBench(nil); err == nil {
		t.Fatal("no args must fail")
	}
	if err := cmdBench([]string{"bogus"}); err == nil {
		t.Fatal("unknown target must fail")
	}
	if err := cmdBench([]string{"scan"}); err == nil {
		t.Fatal("scan without --path must fail")
	}
	if err := cmdBench([]string{"scan", "--path", filepath.Join(t.TempDir(), "missing")}); err == nil {
		t.Fatal("scan on a missing dir must fail")
	}
	// A file is not a scannable directory.
	f := filepath.Join(t.TempDir(), "file")
	if err := os.WriteFile(f, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := cmdBench([]string{"scan", "--path", f}); err == nil {
		t.Fatal("scan on a file must fail")
	}
}

// One real scan through the benchmark wiring: a temp media file in,
// a populated RunResult out.
func TestBenchOneScan(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "[Fansub-A] Frieren - 01 [1080p].mkv"), []byte("fake"), 0o644); err != nil {
		t.Fatal(err)
	}
	res, err := benchOneScan(root, "anime", 1)
	if err != nil {
		t.Fatal(err)
	}
	if res.Candidates != 1 || res.Identified != 1 || res.Run != 1 {
		t.Fatalf("run result: %+v", res)
	}
	if res.TotalMs <= 0 {
		t.Fatalf("total time must be positive: %+v", res)
	}
}

func TestParseByteSizeOverflow(t *testing.T) {
	// The biggest multiplier overflows on a large mantissa.
	if _, err := parseByteSize("9000000000000000000TiB"); err == nil {
		t.Fatal("overflow must fail")
	}
	// Boundary: exactly maxint64 in bytes is fine.
	if got, err := parseByteSize("9223372036854775807"); err != nil || got != math.MaxInt64 {
		t.Fatalf("maxint64 bytes: %d %v", got, err)
	}
}

func TestBinPresent(t *testing.T) {
	dir := t.TempDir()
	exe := filepath.Join(dir, "some-tool")
	if err := os.WriteFile(exe, []byte("#!/bin/sh\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	plain := filepath.Join(dir, "not-exe")
	if err := os.WriteFile(plain, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", dir)
	if !binPresent("some-tool") {
		t.Fatal("executable on PATH not found")
	}
	if binPresent("not-exe") {
		t.Fatal("non-executable must not count")
	}
	if binPresent("missing-tool") {
		t.Fatal("absent binary must not count")
	}
}
