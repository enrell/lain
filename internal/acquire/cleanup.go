package acquire

import (
	"io/fs"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/enrell/lain/internal/contracts"
)

// Cleanup rule (D-110, user condition 2026-10-04): deleting a torrent
// copy never removes or unlinks a file in a library — whether it was
// hardlinked (the library keeps its own name for the same bytes),
// moved in place, or sits inside a library root because of an unusual
// folder layout. Every deletion of torrent data goes through discard.

// within reports whether p is root or below it.
func withinOrEqual(root, p string) bool {
	root, p = filepath.Clean(root), filepath.Clean(p)
	if root == p {
		return true
	}
	rel, err := filepath.Rel(root, p)
	return err == nil && rel != ".." && !strings.HasPrefix(rel, ".."+string(filepath.Separator)) && !filepath.IsAbs(rel)
}

func (m *Manager) libraryRoots() []string {
	if m.d.Libraries == nil {
		return nil
	}
	var roots []string
	for _, l := range m.d.Libraries() {
		if l.Path != "" {
			roots = append(roots, l.Path)
		}
	}
	return roots
}

// protected reports a path cleanup must never delete.
func (m *Manager) protected(p string, g Grab, roots []string) bool {
	for _, root := range roots {
		if withinOrEqual(root, p) {
			return true
		}
	}
	for _, imp := range g.Imported {
		if filepath.Clean(imp) == filepath.Clean(p) {
			return true
		}
	}
	return false
}

// grabDir is where a grab's torrent data lives.
func (m *Manager) grabDir(g Grab) string {
	if g.Dir != "" {
		return g.Dir
	}
	return filepath.Join(m.Settings().Dir, g.ID)
}

// discard drops the torrent from the client and deletes its copy under
// the grab's own folder, file by file, sparing protected paths. It
// reports whether nothing was left behind. Symlinks are removed as
// links and never followed.
func (m *Manager) discard(g Grab) bool {
	m.mu.Lock()
	c := m.client
	m.mu.Unlock()
	if g.InfoHash != "" {
		if err := c.Remove(g.InfoHash, false); err != nil && CodeOf(err) != CodeNotFound {
			m.log.Warn("could not drop torrent", "grab", g.ID, "err", err.Error())
		}
	}
	dir := m.grabDir(g)
	roots := m.libraryRoots()
	if m.protected(dir, g, roots) {
		m.log.Warn("torrent copy lies inside a library; kept", "grab", g.ID, "dir", dir)
		return false
	}
	clean := true
	var dirs []string
	err := filepath.WalkDir(dir, func(p string, d fs.DirEntry, err error) error {
		if err != nil {
			if os.IsNotExist(err) {
				return nil
			}
			clean = false
			return nil
		}
		if d.IsDir() {
			dirs = append(dirs, p)
			return nil
		}
		if m.protected(p, g, roots) {
			clean = false
			m.log.Warn("kept a protected file during cleanup", "grab", g.ID, "path", p)
			return nil
		}
		if err := os.Remove(p); err != nil && !os.IsNotExist(err) {
			clean = false
		}
		return nil
	})
	if err != nil {
		clean = false
	}
	// Deepest folders first; a non-empty one simply stays.
	sort.Slice(dirs, func(i, j int) bool { return len(dirs[i]) > len(dirs[j]) })
	for _, d := range dirs {
		_ = os.Remove(d)
	}
	if _, err := os.Lstat(dir); err == nil {
		clean = false
	}
	return clean
}

// overlapsLibrary refuses a download folder that is, contains or lies
// inside a library root: the torrent copy and the library must never
// share a tree.
func overlapsLibrary(dir string, libs []contracts.Library) (string, bool) {
	for _, l := range libs {
		if l.Path == "" {
			continue
		}
		if withinOrEqual(l.Path, dir) || withinOrEqual(dir, l.Path) {
			return l.Name, true
		}
	}
	return "", false
}
