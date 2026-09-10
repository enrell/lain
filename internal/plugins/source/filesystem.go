// Package source enumerates media roots. It discovers files; it never
// decides what they are. Identification is a separate capability so a
// community parser can replace the built-in one without touching I/O.
package source

import (
	"io/fs"
	"path/filepath"
	"strings"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

// ID is the built-in filesystem source provider id.
const ID = "lain-source-filesystem"

// Exts maps library type to recognized extensions (the inventory both clients
// implemented inventory so behavior stays comparable).
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
	return Enumerate(in)
}

// Enumerate walks the root and returns recognized files.
func Enumerate(in EnumerateInput) ([]contracts.Candidate, error) {
	allow := Exts[strings.ToLower(in.Type)]
	if allow == nil {
		allow = Exts["video"]
	}
	var out []contracts.Candidate
	err := filepath.WalkDir(in.Root, func(path string, d fs.DirEntry, err error) error {
		if err != nil {
			return nil // count walk errors at ingest, never abort the scan
		}
		if d.IsDir() {
			return nil
		}
		ext := strings.ToLower(strings.TrimPrefix(filepath.Ext(path), "."))
		if !allow[ext] {
			return nil
		}
		fi, err := d.Info()
		if err != nil {
			return nil
		}
		out = append(out, contracts.Candidate{
			Path:      path,
			Size:      fi.Size(),
			ModTime:   fi.ModTime().Unix(),
			LibraryID: in.LibraryID,
		})
		return nil
	})
	return out, err
}
