package downloads

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"testing/quick"
	"unicode/utf8"
)

func TestSafeName(t *testing.T) {
	for in, want := range map[string]string{
		"Frieren - 01.cbz":         "Frieren - 01.cbz",
		"../../etc/passwd":         "passwd",
		`..\..\boot.ini`:           "boot.ini",
		"/abs/path/x.mkv":          "x.mkv",
		".hidden.mkv":              "hidden.mkv",
		"..":                       "fallback",
		"":                         "fallback",
		"   ":                      "fallback",
		"a\x00b\nc.mkv":            "abc.mkv",
		`what?:"*<>|.mkv`:          "what_______.mkv",
		"[Fansub-A] Show - 03.mkv": "[Fansub-A] Show - 03.mkv",
	} {
		if got := SafeName(in, "fallback"); got != want {
			t.Errorf("SafeName(%q) = %q, want %q", in, got, want)
		}
	}
	long := strings.Repeat("ü", 300) + ".mkv"
	got := SafeName(long, "f")
	if len(got) > maxNameBytes || !utf8.ValidString(got) || !strings.HasSuffix(got, ".mkv") {
		t.Fatalf("long name -> %d bytes valid=%v %q", len(got), utf8.ValidString(got), got[len(got)-8:])
	}
}

// SafeName's output is always a single, non-hidden path element.
func TestSafeNameNeverEscapesProperty(t *testing.T) {
	f := func(s string) bool {
		n := SafeName(s, "x")
		return n != "" && n != "." && n != ".." && !strings.ContainsAny(n, `/\`) &&
			!strings.HasPrefix(n, ".") && filepath.Base(n) == n && len(n) <= maxNameBytes && utf8.ValidString(n)
	}
	if err := quick.Check(f, &quick.Config{MaxCount: 2000}); err != nil {
		t.Fatal(err)
	}
}

func TestNameFromURL(t *testing.T) {
	for in, want := range map[string]string{
		"https://example.test/a/Frieren%20-%2001.cbz?x=1": "Frieren - 01.cbz",
		"https://example.test/":                           "",
		"https://example.test":                            "",
		"::bad":                                           "",
	} {
		if got := NameFromURL(in); got != want {
			t.Errorf("NameFromURL(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestFreePathNeverOverwrites(t *testing.T) {
	dir := t.TempDir()
	_ = os.WriteFile(filepath.Join(dir, "a.mkv"), nil, 0o644)
	reserved := map[string]bool{filepath.Join(dir, "a (2).mkv"): true}
	if got := FreePath(dir, "a.mkv", reserved); got != filepath.Join(dir, "a (3).mkv") {
		t.Fatalf("got %s", got)
	}
	if got := FreePath(dir, "b.mkv", nil); got != filepath.Join(dir, "b.mkv") {
		t.Fatalf("got %s", got)
	}
}

func TestLimitsCheck(t *testing.T) {
	defer func(f func(string) (int64, error)) { freeBytes = f }(freeBytes)
	freeBytes = func(string) (int64, error) { return 1000, nil }

	if err := (Limits{}).Check("/x", 1<<40, 1<<40); err != nil {
		t.Fatalf("zero limits are unlimited: %v", err)
	}
	if err := (Limits{MaxBytes: 100}).Check("/x", 60, 40); err != nil {
		t.Fatalf("exactly at the cap is allowed: %v", err)
	}
	if err := (Limits{MaxBytes: 100}).Check("/x", 60, 41); CodeOf(err) != CodeQuota {
		t.Fatalf("over the cap: %v", err)
	}
	if err := (Limits{MinFreeBytes: 900}).Check("/x", 0, 101); CodeOf(err) != CodeDiskFull {
		t.Fatalf("below the floor: %v", err)
	}
	if err := (Limits{MinFreeBytes: 900}).Check("/x", 0, 100); err != nil {
		t.Fatalf("at the floor: %v", err)
	}
	freeBytes = func(string) (int64, error) { return 0, errors.New("no statfs") }
	if err := (Limits{MinFreeBytes: 900}).Check("/x", 0, 1<<30); err != nil {
		t.Fatalf("an unknown free size must not block: %v", err)
	}
}

func TestHumanBytes(t *testing.T) {
	for n, want := range map[int64]string{0: "0 B", 1023: "1023 B", 1024: "1.0 KiB", 3 << 30: "3.0 GiB"} {
		if got := HumanBytes(n); got != want {
			t.Errorf("HumanBytes(%d) = %q, want %q", n, got, want)
		}
	}
}

func TestLimitsFull(t *testing.T) {
	defer func(f func(string) (int64, error)) { freeBytes = f }(freeBytes)
	freeBytes = func(string) (int64, error) { return 1000, nil }
	if err := (Limits{MaxBytes: 100}).Full("/x", 99); err != nil {
		t.Fatalf("room left: %v", err)
	}
	if err := (Limits{MaxBytes: 100}).Full("/x", 100); CodeOf(err) != CodeQuota || !strings.Contains(err.Error(), "used up") {
		t.Fatalf("budget used up: %v", err)
	}
	if err := (Limits{MinFreeBytes: 1000}).Full("/x", 0); CodeOf(err) != CodeDiskFull {
		t.Fatalf("at the floor: %v", err)
	}
	if err := (Limits{}).Full("/x", 1<<50); err != nil {
		t.Fatalf("no limits: %v", err)
	}
}
