package gateway

import (
	"errors"
	"net/http"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/auth"
	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/kv"
	"github.com/enrell/lain/internal/plugins/source"
)

// LibraryStore persists configured media roots in the libraries bucket.
type LibraryStore struct {
	db *bolt.DB
}

// List returns all libraries sorted by name.
func (l *LibraryStore) List() ([]contracts.Library, error) {
	var out []contracts.Library
	err := l.db.View(func(tx *bolt.Tx) error {
		return tx.Bucket(kv.BLibraries).ForEach(func(_, v []byte) error {
			var lib contracts.Library
			if err := decodeLibrary(v, &lib); err != nil {
				return nil
			}
			out = append(out, lib)
			return nil
		})
	})
	if err != nil {
		return nil, err
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Name < out[j].Name })
	if out == nil {
		out = []contracts.Library{}
	}
	return out, nil
}

// Create validates and stores a library. Id is stable over the path so
// re-creating the same root never orphans catalog entries.
func (l *LibraryStore) Create(name, typ, path string) (contracts.Library, error) {
	if name == "" || path == "" {
		return contracts.Library{}, errors.New("name and path required")
	}
	fi, err := os.Stat(path)
	if err != nil || !fi.IsDir() {
		return contracts.Library{}, errors.New("path is not a readable directory")
	}
	if typ == "" {
		typ = "anime"
	}
	lib := contracts.Library{
		ID: "lib-" + shortID(path), Name: name, Type: typ,
		Path: path, Source: source.ID, CreatedAt: time.Now().Unix(),
	}
	err = l.db.Update(func(tx *bolt.Tx) error {
		return kv.PutJSON(tx, kv.BLibraries, []byte(lib.ID), lib)
	})
	if err != nil {
		return contracts.Library{}, err
	}
	return lib, nil
}

// Delete removes a library. Catalog entries survive (prune happens on
// the next scan of remaining libraries); userstate is never touched.
func (l *LibraryStore) Delete(id string) error {
	return l.db.Update(func(tx *bolt.Tx) error {
		b := tx.Bucket(kv.BLibraries)
		if b.Get([]byte(id)) == nil {
			return errors.New("unknown library")
		}
		return b.Delete([]byte(id))
	})
}

// browseCandidates are the auto-detect roots for the library folder
// picker, most specific first. Detection returns the first candidate
// that exists and reads as a directory, so the dialog opens on folders
// that can actually hold a library instead of the whole filesystem.
var browseCandidates = []string{
	"/media/videos",
	"/media",
	"/videos",
	"/mnt/media",
	"/srv/media",
	"/data/media",
	"/mnt",
	"/srv",
	"/data",
	"/home",
	"/",
}

// browseDir is one listed subdirectory.
type browseDir struct {
	Name string `json:"name"`
	Path string `json:"path"`
}

// browseResult is the folder-picker payload.
type browseResult struct {
	Path     string      `json:"path"`
	Parent   string      `json:"parent"`
	Detected bool        `json:"detected"`
	Dirs     []browseDir `json:"dirs"`
}

// handleBrowse lists server-side directories for the library folder
// picker. Admin-only: directory names are filesystem information.
// An empty path auto-detects the media root; otherwise path must be an
// absolute readable directory. Only subdirectories are listed (no files,
// no dotfiles), sorted by name, capped so huge trees stay responsive.
func (s *Server) handleBrowse(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	raw := strings.TrimSpace(r.URL.Query().Get("path"))
	detected := false
	if raw == "" {
		root, ok := detectMediaRoot()
		if !ok {
			writeErr(w, 400, "no browsable directory found")
			return
		}
		raw, detected = root, true
	}
	clean := filepath.Clean(raw)
	if !filepath.IsAbs(clean) {
		writeErr(w, 400, "path must be absolute")
		return
	}
	entries, err := os.ReadDir(clean)
	if err != nil {
		writeErr(w, 400, "path is not a readable directory")
		return
	}
	out := []browseDir{}
	for _, e := range entries {
		name := e.Name()
		if strings.HasPrefix(name, ".") {
			continue
		}
		info, err := e.Info()
		if err != nil || !info.IsDir() {
			continue
		}
		out = append(out, browseDir{Name: name, Path: filepath.Join(clean, name)})
		if len(out) >= 500 {
			break
		}
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Name < out[j].Name })
	parent := ""
	if clean != "/" {
		parent = filepath.Dir(clean)
	}
	writeJSON(w, 200, browseResult{Path: clean, Parent: parent, Detected: detected, Dirs: out})
}

func detectMediaRoot() (string, bool) {
	for _, c := range browseCandidates {
		fi, err := os.Stat(c)
		if err == nil && fi.IsDir() {
			return c, true
		}
	}
	return "", false
}
