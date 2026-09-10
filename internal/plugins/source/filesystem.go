// Package source enumerates media roots. It discovers files; it never
// decides what they are. Identification is a separate capability so a
// community parser can replace the built-in one without touching I/O.
package source

import (
	"errors"
	"io/fs"
	"os"
	"path/filepath"
	"strings"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

// ID is the built-in filesystem source provider id.
const ID = "lain-source-filesystem"

// Exts maps library type to recognized extensions (the inventory both
// clients recognize so behavior stays comparable).
var Exts = map[string]map[string]bool{
	"anime": field("mkv", "mp4", "avi", "mov", "m4v", "webm"),
	"show":  field("mkv", "mp4", "avi", "mov", "m4v", "webm"),
	"movie": field("mkv", "mp4", "avi", "mov", "m4v", "webm"),
	"video": field("mkv", "mp4", "avi", "mov", "m4v", "webm"),
	"music": field("mp3", "flac", "m4a", "aac", "ogg", "opus", "wav"),
	"book":  field("pdf", "epub", "mobi", "azw3"),
	"comic": field("cbz", "cbr", "cb7", "pdf"),
	"photo": field("jpg", "jpeg", "png", "webp", "gif", "heic", "avif"),
}

func field(exts ...string) map[string]bool {
	m := map[string]bool{}
	for _, e := range exts {
		m[e] = true
	}
	return m
}

// Provider serves lain.source.enumerate@1.
type Provider struct{}

func (Provider) ID() string             { return ID }
func (Provider) Capabilities() []string { return []string{contracts.CapSourceEnumerate} }
func (Provider) Health() error          { return nil }

// EnumerateInput selects a root and its library type filter.
type EnumerateInput struct {
	Root      string `json:"root"`
	LibraryID string `json:"library_id"`
	Type      string `json:"type"`
}

func (Provider) Invoke(cap string, input any) (any, error) {
	if cap != contracts.CapSourceEnumerate {
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
	in, ok := input.(EnumerateInput)
	if !ok {
		return nil, &core.Error{Code: "invalid-message", Msg: "EnumerateInput required"}
	}
	cands, stats, err := Enumerate(in)
	if err != nil {
		return nil, err
	}
	return EnumerateOutput{Candidates: cands, Stats: stats}, nil
}

// EnumerateOutput carries files plus walk observations.
type EnumerateOutput struct {
	Candidates []contracts.Candidate `json:"candidates"`
	Stats      EnumStats             `json:"stats"`
}

// EnumStats observes a walk: counts for progress UI, errors for the
// prune guard. Accessible==false means the root itself could not be
// read (unmounted drive, deleted path): the caller must not prune.
type EnumStats struct {
	Root       string `json:"root"`
	Accessible bool   `json:"accessible"`
	Dirs       int    `json:"dirs"`
	Entries    int    `json:"entries"`
	WalkErrors int    `json:"walk_errors"`
}

// RootError marks an unreadable library root. It is a distinct type so
// the pipeline can tell "root gone" (skip prune) from "file failed"
// (count and continue).
type RootError struct {
	Root string
	Err  error
}

func (e *RootError) Error() string { return "inaccessible root " + e.Root + ": " + e.Err.Error() }
func (e *RootError) Unwrap() error { return e.Err }

// Enumerate walks the root and returns recognized files with walk stats.
// Unreadable entries are counted in WalkErrors and skipped; only an
// unreadable root itself is a hard error. Order is deterministic
// (lexical walk). Symlinks are never followed, so loops terminate.
func Enumerate(in EnumerateInput) ([]contracts.Candidate, EnumStats, error) {
	stats := EnumStats{Root: in.Root}
	fi, err := os.Stat(in.Root)
	if err != nil || !fi.IsDir() {
		return nil, stats, &RootError{Root: in.Root, Err: errOrNotDir(err)}
	}
	stats.Accessible = true
	allow := Exts[strings.ToLower(in.Type)]
	if allow == nil {
		allow = Exts["video"]
	}
	var out []contracts.Candidate
	err = filepath.WalkDir(in.Root, func(path string, d fs.DirEntry, err error) error {
		stats.Entries++
		if err != nil {
			stats.WalkErrors++
			return nil // skip, keep walking
		}
		if d.IsDir() {
			stats.Dirs++
			return nil
		}
		ext := strings.ToLower(strings.TrimPrefix(filepath.Ext(path), "."))
		if !allow[ext] {
			return nil
		}
		info, err := d.Info()
		if err != nil {
			stats.WalkErrors++
			return nil
		}
		out = append(out, contracts.Candidate{
			Path:      path,
			Size:      info.Size(),
			ModTime:   info.ModTime().Unix(),
			LibraryID: in.LibraryID,
		})
		return nil
	})
	return out, stats, err
}

func errOrNotDir(err error) error {
	if err == nil {
		return errors.New("not a directory")
	}
	return err
}
