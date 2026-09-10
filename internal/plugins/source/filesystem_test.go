package source

import (
	"errors"
	"os"
	"path/filepath"
	"testing"
)

func TestMissingRootIsRootError(t *testing.T) {
	_, stats, err := Enumerate(EnumerateInput{Root: "/nonexistent-lain-lib", LibraryID: "l", Type: "anime"})
	if err == nil {
		t.Fatal("missing root must error")
	}
	var re *RootError
	if !errors.As(err, &re) {
		t.Fatalf("want *RootError, got %T", err)
	}
	if stats.Accessible {
		t.Fatal("missing root must not be accessible")
	}
}

func TestFileAsRootIsRootError(t *testing.T) {
	f := filepath.Join(t.TempDir(), "x.mkv")
	if err := os.WriteFile(f, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, _, err := Enumerate(EnumerateInput{Root: f, LibraryID: "l", Type: "anime"}); err == nil {
		t.Fatal("file root must error")
	}
}

func TestWalkCountsAndSkipsJunk(t *testing.T) {
	root := t.TempDir()
	if err := os.MkdirAll(filepath.Join(root, "sub", "deep"), 0o755); err != nil {
		t.Fatal(err)
	}
	write := func(p string) {
		t.Helper()
		if err := os.WriteFile(filepath.Join(root, p), []byte("x"), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	write("a.mkv")
	write("sub/b.mp4")
	write("sub/deep/c.avi")
	write("spam.url")      // junk: never a candidate
	write("notes.txt")     // junk
	write("sub/cover.jpg") // photo type, not anime

	cands, stats, err := Enumerate(EnumerateInput{Root: root, LibraryID: "l", Type: "anime"})
	if err != nil {
		t.Fatal(err)
	}
	if !stats.Accessible {
		t.Fatal("root must be accessible")
	}
	if len(cands) != 3 {
		t.Fatalf("candidates=%d, want 3", len(cands))
	}
	if stats.Dirs != 3 { // root, sub, sub/deep
		t.Fatalf("dirs=%d, want 3", stats.Dirs)
	}
	if stats.WalkErrors != 0 {
		t.Fatalf("walk errors=%d, want 0", stats.WalkErrors)
	}
	// Deterministic lexical order.
	if cands[0].Path > cands[1].Path || cands[1].Path > cands[2].Path {
		t.Fatalf("non-deterministic order: %v", cands)
	}
	for _, c := range cands {
		if c.LibraryID != "l" || c.Size != 1 {
			t.Fatalf("bad candidate meta: %+v", c)
		}
	}
}

func TestUnreadableSubdirCountsKeepsWalking(t *testing.T) {
	root := t.TempDir()
	good := filepath.Join(root, "good")
	bad := filepath.Join(root, "bad")
	if err := os.MkdirAll(good, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(bad, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(good, "ok.mkv"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(bad, "hidden.mkv"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(bad, 0o000); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.Chmod(bad, 0o755) })

	cands, stats, err := Enumerate(EnumerateInput{Root: root, LibraryID: "l", Type: "anime"})
	if err != nil {
		t.Fatal(err)
	}
	if !stats.Accessible {
		t.Fatal("root stays accessible despite bad child")
	}
	if stats.WalkErrors == 0 {
		t.Fatal("unreadable dir must be counted, not silent")
	}
	if len(cands) != 1 {
		t.Fatalf("candidates=%d, want 1 (good dir only)", len(cands))
	}
}

func TestSymlinkLoopTerminates(t *testing.T) {
	root := t.TempDir()
	sub := filepath.Join(root, "sub")
	if err := os.MkdirAll(sub, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(sub, "ep.mkv"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	// Loop: sub/loop -> root. WalkDir never follows symlinks.
	if err := os.Symlink(root, filepath.Join(sub, "loop")); err != nil {
		t.Fatal(err)
	}
	cands, _, err := Enumerate(EnumerateInput{Root: root, LibraryID: "l", Type: "anime"})
	if err != nil {
		t.Fatal(err)
	}
	if len(cands) != 1 {
		t.Fatalf("candidates=%d, want 1 (symlink never followed)", len(cands))
	}
}

func TestUnknownTypeFallsBackToVideo(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "x.mkv"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	cands, _, err := Enumerate(EnumerateInput{Root: root, LibraryID: "l", Type: "whatever"})
	if err != nil {
		t.Fatal(err)
	}
	if len(cands) != 1 {
		t.Fatalf("unknown type must fall back to video set, got %d", len(cands))
	}
}
