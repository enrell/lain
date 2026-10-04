package acquire

import (
	"bytes"
	"os"
	"path/filepath"
	"syscall"
	"testing"
	"time"
)

// The library copy is sacred (D-110 condition, user approval
// 2026-10-04): whatever cleanup does to the torrent copy — after
// seeding, after a no-seed import, on failure or on removal — it never
// removes or unlinks a file in a library, hardlinked or moved in place.

func libraryFiles(t *testing.T, w *world) map[string][]byte {
	t.Helper()
	out := map[string][]byte{}
	for i, name := range []string{"[Fansub-A] SHOW - 01.mkv", "[Fansub-A] SHOW - 02.mkv"} {
		p := filepath.Join(w.lib.Path, "SHOW", name)
		b, err := os.ReadFile(p)
		if err != nil {
			t.Fatalf("library file %s: %v", p, err)
		}
		if !bytes.Equal(b, w.files[i].Data) {
			t.Fatalf("library file %s changed", p)
		}
		out[p] = b
	}
	return out
}

func nlink(t *testing.T, p string) uint64 {
	t.Helper()
	fi, err := os.Stat(p)
	if err != nil {
		t.Fatal(err)
	}
	st, ok := fi.Sys().(*syscall.Stat_t)
	if !ok {
		t.Skip("no link counts on this platform")
	}
	return uint64(st.Nlink)
}

func TestCleanupAfterSeedingKeepsHardlinkedLibraryFiles(t *testing.T) {
	w := newWorld(t)
	s := settings(0)
	s.SeedMinutes = 1 // seed, then stop on time: forced below
	m := w.manager(t, t.TempDir(), s)
	g, err := m.Grab(GrabInput{URL: w.index.URL + "/dl/show.torrent", LibraryID: w.lib.ID})
	if err != nil {
		t.Fatal(err)
	}
	waitGrab(t, m, g.ID, GrabSeeding)
	lib := libraryFiles(t, w)
	torrentCopy := filepath.Join(m.Settings().Dir, g.ID, w.mi.Info.Name, w.files[0].Path[0])
	for p := range lib {
		if nlink(t, p) != 2 {
			t.Fatalf("%s should be hardlinked to the seeding copy", p)
		}
	}
	// Pretend the seeding window is over.
	got, _ := m.st.grab(g.ID)
	got.SeedingAt = time.Now().Add(-2 * time.Minute).Unix()
	if err := m.st.putGrab(got); err != nil {
		t.Fatal(err)
	}
	done := waitGrab(t, m, g.ID, GrabDone)
	if !done.DataRemoved {
		t.Fatalf("torrent copy must be removed: %+v", done)
	}
	if _, err := os.Stat(torrentCopy); !os.IsNotExist(err) {
		t.Fatalf("torrent copy survived: %v", err)
	}
	for p := range libraryFiles(t, w) {
		if nlink(t, p) != 1 {
			t.Fatalf("%s: the library name must be the one that remains", p)
		}
	}
}

func TestCleanupAfterMoveImportKeepsLibraryFiles(t *testing.T) {
	w := newWorld(t)
	s := settings(0) // no seeding: import, then clean up
	s.ImportMode = ImportMove
	m := w.manager(t, t.TempDir(), s)
	g, err := m.Grab(GrabInput{URL: w.index.URL + "/dl/show.torrent", LibraryID: w.lib.ID})
	if err != nil {
		t.Fatal(err)
	}
	done := waitGrab(t, m, g.ID, GrabDone)
	libraryFiles(t, w)
	if !done.DataRemoved {
		t.Fatalf("the rest of the torrent copy must be cleaned: %+v", done)
	}
	if _, err := os.Stat(filepath.Join(m.Settings().Dir, g.ID)); !os.IsNotExist(err) {
		t.Fatalf("grab folder survived cleanup: %v", err)
	}
	// Removing the finished grab with data again must still leave the library alone.
	if err := m.Remove(g.ID, true); err != nil {
		t.Fatal(err)
	}
	libraryFiles(t, w)
}

func TestCleanupNeverDeletesInsideALibraryRoot(t *testing.T) {
	w := newWorld(t)
	data := t.TempDir()
	// A hostile layout: the library root contains the acquisition folder,
	// so the torrent copy itself lies inside a library.
	w.lib.Path = data
	m := w.manager(t, data, settings(0))
	g, err := m.Grab(GrabInput{URL: w.index.URL + "/dl/show.torrent", LibraryID: w.lib.ID})
	if err != nil {
		t.Fatal(err)
	}
	done := waitGrab(t, m, g.ID, GrabDone)
	libraryFiles(t, w)
	torrentCopy := filepath.Join(m.Settings().Dir, g.ID, w.mi.Info.Name, w.files[0].Path[0])
	if _, err := os.Stat(torrentCopy); err != nil {
		t.Fatalf("a file inside a library root must never be deleted: %v", err)
	}
	if done.DataRemoved {
		t.Fatal("data kept inside a library must not be reported as removed")
	}
	if err := m.Remove(g.ID, true); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(torrentCopy); err != nil {
		t.Fatalf("removing the grab deleted a file inside a library: %v", err)
	}
}

func TestSettingsRefuseADownloadFolderInsideALibrary(t *testing.T) {
	w := newWorld(t)
	m := w.manager(t, t.TempDir(), settings(0))
	for _, dir := range []string{
		filepath.Join(w.lib.Path, "downloads"), // inside a library
		filepath.Dir(w.lib.Path),               // contains a library
		w.lib.Path,                             // is a library
	} {
		s := m.Settings()
		s.Dir = dir
		if _, err := m.SetSettings(s); CodeOf(err) != CodeInvalid {
			t.Errorf("dir %s accepted: %v", dir, err)
		}
	}
}

func TestFailedDownloadCleanupSparesLibraries(t *testing.T) {
	w := newWorld(t)
	w.limits.MaxBytes = 1000
	m := w.manager(t, t.TempDir(), settings(0))
	// A library file that happens to share the torrent's relative layout.
	keep := filepath.Join(w.lib.Path, "SHOW", "keep.mkv")
	_ = os.MkdirAll(filepath.Dir(keep), 0o755)
	if err := os.WriteFile(keep, []byte("mine"), 0o644); err != nil {
		t.Fatal(err)
	}
	g, err := m.Grab(GrabInput{URL: w.index.URL + "/dl/magnet", LibraryID: w.lib.ID})
	if err != nil {
		t.Fatal(err)
	}
	waitGrabFailed(t, m, g.ID)
	if b, err := os.ReadFile(keep); err != nil || string(b) != "mine" {
		t.Fatalf("library file touched by failure cleanup: %v", err)
	}
}
