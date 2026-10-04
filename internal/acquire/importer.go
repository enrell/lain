package acquire

import (
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/plugins/catalog"
)

var videoExts = map[string]bool{".mkv": true, ".mp4": true, ".avi": true, ".m4v": true, ".webm": true, ".mov": true, ".ts": true}
var readingExts = map[string]bool{".cbz": true, ".cbr": true, ".cb7": true}

func readingKind(kind string) bool { return kind == contracts.KindManga || kind == contracts.KindComic }

// ImportItem is one file to place.
type ImportItem struct {
	Src     string            `json:"src"`
	Dst     string            `json:"dst"`
	Release contracts.Release `json:"release"`
	// Exists is true when Dst is already there (an earlier import);
	// existing files are never overwritten (A-10).
	Exists bool `json:"exists,omitempty"`
}

// Parser parses one name (the lain.release.parse@1 call).
type Parser func(name, kind string) contracts.Release

// PlanImport decides where each media file of a finished download goes
// in the library (A-10, A-11). titles are the library's existing titles,
// so a new episode lands under the folder the show already uses.
func PlanImport(lib contracts.Library, files []File, grab contracts.Release, parse Parser, titles []string) ([]ImportItem, error) {
	var media []File
	for _, f := range files {
		ext := strings.ToLower(filepath.Ext(f.Path))
		if readingKind(lib.Type) && readingExts[ext] || !readingKind(lib.Type) && videoExts[ext] {
			media = append(media, f)
		}
	}
	if len(media) > 1 {
		kept := media[:0]
		for _, f := range media {
			if !strings.Contains(strings.ToLower(filepath.Base(f.Rel)), "sample") {
				kept = append(kept, f)
			}
		}
		media = kept
	}
	if len(media) == 0 {
		return nil, errf(CodeImport, "the download has no %s files", map[bool]string{true: "comic archive", false: "video"}[readingKind(lib.Type)])
	}
	if lib.Type == "movie" {
		// A movie is its largest video; the rest are extras.
		sort.Slice(media, func(i, j int) bool { return media[i].Length > media[j].Length })
		media = media[:1]
	}
	byKey := map[string]string{}
	for _, t := range titles {
		byKey[looseKey(t)] = t
	}
	var out []ImportItem
	seen := map[string]bool{}
	for _, f := range media {
		r := parse(filepath.Base(f.Path), lib.Type)
		fill(&r, grab, len(media) == 1)
		title := r.Title
		if existing, ok := byKey[looseKey(title)]; ok {
			title = existing
		}
		rel := destination(lib.Type, title, r, filepath.Base(f.Path))
		dst := filepath.Join(lib.Path, rel)
		if !within(lib.Path, dst) {
			return nil, errf(CodeImport, "refusing a path outside the library: %s", rel)
		}
		if seen[dst] {
			return nil, errf(CodeImport, "two files would land on %s", rel)
		}
		seen[dst] = true
		item := ImportItem{Src: f.Path, Dst: dst, Release: r}
		if _, err := os.Lstat(dst); err == nil {
			item.Exists = true
		}
		out = append(out, item)
	}
	return out, nil
}

// fill completes a file's parse with what the release name said: packs
// name the show once, files often only carry the episode.
func fill(r *contracts.Release, grab contracts.Release, single bool) {
	if r.Title == "" {
		r.Title = grab.Title
	}
	if r.Year == 0 {
		r.Year = grab.Year
	}
	if r.Season == 0 && grab.Season > 0 {
		r.Season = grab.Season
		if len(r.Episodes) > 0 && r.Absolute {
			r.Absolute = false
		}
	}
	if single {
		if len(r.Episodes) == 0 {
			r.Episodes, r.Absolute = grab.Episodes, grab.Absolute
		}
		if r.Volume == 0 {
			r.Volume = grab.Volume
		}
		if r.Chapter == 0 {
			r.Chapter = grab.Chapter
		}
	}
	if r.Group == "" {
		r.Group = grab.Group
	}
	if r.Title == "" {
		r.Title = "Unknown"
	}
}

// looseKey matches titles across punctuation and case: "Show: Name" and
// "Show Name" are one title.
func looseKey(s string) string {
	s = catalog.TitleKey(s)
	return strings.Join(strings.FieldsFunc(s, func(r rune) bool {
		return !(r >= 'a' && r <= 'z' || r >= '0' && r <= '9' || r > 127)
	}), " ")
}

// destination is the naming scheme v1 (A-11).
func destination(kind, title string, r contracts.Release, original string) string {
	ext := strings.ToLower(filepath.Ext(original))
	t := safeComponent(title)
	switch {
	case kind == "movie":
		name := t
		if r.Year > 0 {
			name = fmt.Sprintf("%s (%d)", t, r.Year)
		}
		return filepath.Join(name, name+ext)
	case readingKind(kind):
		var parts []string
		if r.Volume > 0 {
			parts = append(parts, fmt.Sprintf("v%02d", r.Volume))
		}
		if r.Chapter > 0 {
			parts = append(parts, fmt.Sprintf("c%03d", r.Chapter))
		}
		if len(parts) == 0 {
			return filepath.Join(t, safeComponent(original))
		}
		return filepath.Join(t, t+" "+strings.Join(parts, " ")+ext)
	default:
		if len(r.Episodes) == 0 {
			if r.Season > 0 {
				return filepath.Join(t, fmt.Sprintf("Season %02d", r.Season), safeComponent(original))
			}
			return filepath.Join(t, safeComponent(original))
		}
		span := episodeSpan(r.Episodes)
		eps := fmt.Sprintf("E%02d", span[0])
		if len(span) > 1 {
			eps += fmt.Sprintf("-E%02d", span[len(span)-1])
		}
		if r.Season > 0 && !r.Absolute {
			return filepath.Join(t, fmt.Sprintf("Season %02d", r.Season), fmt.Sprintf("%s - S%02d%s%s", t, r.Season, eps, ext))
		}
		// Absolute numbering is season 0 in Lain's catalog: the anime
		// identifier reads "[Group] Title - 03" that way, and "S00E03"
		// when no group is known. Both forms round-trip (tested).
		if r.Group != "" && len(span) == 1 {
			width := 2
			if span[0] >= 100 {
				width = 3
			}
			if span[0] >= 1000 {
				width = 4
			}
			return filepath.Join(t, fmt.Sprintf("[%s] %s - %0*d%s", safeComponent(r.Group), t, width, span[0], ext))
		}
		return filepath.Join(t, fmt.Sprintf("%s - S00%s%s", t, eps, ext))
	}
}

// episodeSpan keeps the first and last of a run: one file holding
// several episodes is named E02-E03 (the identifier reads the first).
func episodeSpan(eps []int) []int {
	if len(eps) <= 2 {
		return eps
	}
	return []int{eps[0], eps[len(eps)-1]}
}

// safeComponent makes a title usable as one path component everywhere.
func safeComponent(s string) string {
	s = strings.Map(func(r rune) rune {
		switch {
		case r < 0x20, r == 0x7f:
			return -1
		case strings.ContainsRune(`/\:*?"<>|`, r):
			return ' '
		}
		return r
	}, s)
	s = strings.Join(strings.Fields(s), " ")
	s = strings.Trim(s, ". ")
	if len(s) > 180 {
		s = strings.TrimSpace(s[:180])
	}
	if s == "" || s == "." || s == ".." {
		s = "Unknown"
	}
	return s
}

func within(root, p string) bool {
	rel, err := filepath.Rel(root, p)
	return err == nil && rel != "." && !strings.HasPrefix(rel, "..") && !filepath.IsAbs(rel)
}

// Place puts src at dst: a hardlink when possible (seeding continues on
// the same bytes), else a copy; move renames (or copies then removes).
// It never overwrites dst.
func Place(src, dst, mode string) (string, error) {
	if _, err := os.Lstat(dst); err == nil {
		return "", errf(CodeImport, "%s already exists", dst)
	}
	if err := os.MkdirAll(filepath.Dir(dst), 0o755); err != nil {
		return "", errf(CodeImport, "create folder: %v", err)
	}
	switch mode {
	case ImportHardlink:
		if err := os.Link(src, dst); err == nil {
			return ImportHardlink, nil
		}
		// Different filesystems (or no link support): copy instead.
		return ImportCopy, copyFile(src, dst)
	case ImportMove:
		if err := os.Rename(src, dst); err == nil {
			return ImportMove, nil
		}
		if err := copyFile(src, dst); err != nil {
			return "", err
		}
		_ = os.Remove(src)
		return ImportMove, nil
	default:
		return ImportCopy, copyFile(src, dst)
	}
}

func copyFile(src, dst string) error {
	in, err := os.Open(src)
	if err != nil {
		return errf(CodeImport, "open source: %v", err)
	}
	defer in.Close()
	tmp := dst + ".lain-import"
	out, err := os.OpenFile(tmp, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o644)
	if err != nil {
		return errf(CodeImport, "create: %v", err)
	}
	_, cerr := io.Copy(out, in)
	if err := out.Close(); cerr == nil {
		cerr = err
	}
	if cerr != nil {
		_ = os.Remove(tmp)
		return errf(CodeImport, "copy: %v", cerr)
	}
	// Link the finished temp file into place: unlike rename it fails if
	// dst appeared meanwhile, so an existing file is never replaced.
	if err := os.Link(tmp, dst); err != nil {
		if errors.Is(err, os.ErrExist) {
			_ = os.Remove(tmp)
			return errf(CodeImport, "%s already exists", dst)
		}
		// No hardlinks on this filesystem: check, then rename.
		if _, serr := os.Lstat(dst); serr == nil {
			_ = os.Remove(tmp)
			return errf(CodeImport, "%s already exists", dst)
		}
		if rerr := os.Rename(tmp, dst); rerr != nil {
			_ = os.Remove(tmp)
			return errf(CodeImport, "finish copy: %v", rerr)
		}
		return nil
	}
	return os.Remove(tmp)
}
