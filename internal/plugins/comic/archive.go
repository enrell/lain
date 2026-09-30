// Package comic reads comic and manga archives (lain.comic.pages@1).
// It indexes pages and opens single entries; it never decodes or
// rescales pixels for clients. CBZ is read natively (stdlib zip); CBR
// and CB7 need an external extractor (bsdtar or 7z), the same
// operator-installed-tool category as ffmpeg, and degrade to a typed
// error when none is installed.
package comic

import (
	"archive/zip"
	"bytes"
	"context"
	"fmt"
	"io"
	"os/exec"
	"path"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/enrell/lain/internal/core"
)

// maxPageBytes bounds one page read: a decompression bomb inside an
// archive must not exhaust memory.
const maxPageBytes = 128 << 20

var imageMimes = map[string]string{
	".jpg": "image/jpeg", ".jpeg": "image/jpeg", ".png": "image/png",
	".webp": "image/webp", ".gif": "image/gif", ".avif": "image/avif",
	".bmp": "image/bmp",
}

// entry is one archive member that is a page.
type entry struct {
	name string
	size int64
}

// Format returns the archive format token for a path, or "" when this
// package does not read the extension.
func Format(p string) string {
	switch strings.ToLower(filepath.Ext(p)) {
	case ".cbz", ".zip":
		return "cbz"
	case ".cbr", ".rar":
		return "cbr"
	case ".cb7", ".7z":
		return "cb7"
	}
	return ""
}

// MimeOf returns the image mime type for an entry name, "" if not an image.
func MimeOf(name string) string { return imageMimes[strings.ToLower(path.Ext(name))] }

// keepEntry filters non-page members: directories, dotfiles, macOS
// resource forks and anything that is not an image.
func keepEntry(name string) bool {
	if name == "" || strings.HasSuffix(name, "/") {
		return false
	}
	for _, seg := range strings.Split(name, "/") {
		if strings.HasPrefix(seg, ".") || seg == "__MACOSX" {
			return false
		}
	}
	return MimeOf(name) != ""
}

// sortEntries orders pages naturally: "page2" before "page10", and
// directories compare segment by segment so "ch1/2.jpg" < "ch10/1.jpg".
func sortEntries(es []entry) {
	sort.SliceStable(es, func(i, j int) bool { return naturalLess(es[i].name, es[j].name) })
}

func naturalLess(a, b string) bool {
	a, b = strings.ToLower(a), strings.ToLower(b)
	for a != "" && b != "" {
		if isDigit(a[0]) && isDigit(b[0]) {
			na, ra := digitRun(a)
			nb, rb := digitRun(b)
			ta, tb := strings.TrimLeft(na, "0"), strings.TrimLeft(nb, "0")
			if len(ta) != len(tb) {
				return len(ta) < len(tb)
			}
			if ta != tb {
				return ta < tb
			}
			if len(na) != len(nb) {
				return len(na) < len(nb)
			}
			a, b = ra, rb
			continue
		}
		if a[0] != b[0] {
			return a[0] < b[0]
		}
		a, b = a[1:], b[1:]
	}
	return len(a) < len(b)
}

func isDigit(c byte) bool { return c >= '0' && c <= '9' }

func digitRun(s string) (run, rest string) {
	i := 0
	for i < len(s) && isDigit(s[i]) {
		i++
	}
	return s[:i], s[i:]
}

// list returns the page entries of an archive in reading order.
func list(archive string) ([]entry, error) {
	switch Format(archive) {
	case "cbz":
		return listZip(archive)
	case "cbr", "cb7":
		return listExternal(archive)
	}
	return nil, &core.Error{Code: "unsupported-format", Msg: "not a comic archive: " + filepath.Ext(archive)}
}

func listZip(archive string) ([]entry, error) {
	zr, err := zip.OpenReader(archive)
	if err != nil {
		return nil, &core.Error{Code: "invalid-archive", Msg: err.Error()}
	}
	defer zr.Close()
	var es []entry
	for _, f := range zr.File {
		if keepEntry(f.Name) {
			es = append(es, entry{name: f.Name, size: int64(f.UncompressedSize64)})
		}
	}
	sortEntries(es)
	return es, nil
}

// extractor returns the external tool able to read rar and 7z archives.
func extractor() (string, error) {
	for _, tool := range []string{"bsdtar", "7z", "7zz"} {
		if p, err := exec.LookPath(tool); err == nil {
			return p, nil
		}
	}
	return "", &core.Error{Code: "dependency-unavailable", Msg: "cbr/cb7 need bsdtar or 7z installed"}
}

func listExternal(archive string) ([]entry, error) {
	tool, err := extractor()
	if err != nil {
		return nil, err
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	var cmd *exec.Cmd
	if filepath.Base(tool) == "bsdtar" {
		cmd = exec.CommandContext(ctx, tool, "-tf", archive)
	} else {
		cmd = exec.CommandContext(ctx, tool, "l", "-ba", "-slt", archive)
	}
	out, err := cmd.Output()
	if err != nil {
		return nil, &core.Error{Code: "invalid-archive", Msg: fmt.Sprintf("%s: %v", filepath.Base(tool), err)}
	}
	var es []entry
	for _, line := range strings.Split(string(out), "\n") {
		line = strings.TrimRight(line, "\r")
		if strings.HasPrefix(line, "Path = ") {
			line = strings.TrimPrefix(line, "Path = ")
		} else if filepath.Base(tool) != "bsdtar" {
			continue
		}
		if keepEntry(line) {
			es = append(es, entry{name: line})
		}
	}
	sortEntries(es)
	return es, nil
}

// OpenPage returns the bytes of one entry. Entries are addressed by the
// names List produced; anything else is refused, so a client index can
// never reach an arbitrary archive member.
func OpenPage(archive, name string) (io.ReadCloser, error) {
	if MimeOf(name) == "" {
		return nil, &core.Error{Code: "not-found", Msg: "not a page"}
	}
	switch Format(archive) {
	case "cbz":
		zr, err := zip.OpenReader(archive)
		if err != nil {
			return nil, &core.Error{Code: "invalid-archive", Msg: err.Error()}
		}
		for _, f := range zr.File {
			if f.Name != name {
				continue
			}
			rc, err := f.Open()
			if err != nil {
				zr.Close()
				return nil, &core.Error{Code: "invalid-archive", Msg: err.Error()}
			}
			return &zipPage{ReadCloser: rc, zr: zr}, nil
		}
		zr.Close()
		return nil, &core.Error{Code: "not-found", Msg: "no such page"}
	case "cbr", "cb7":
		return openExternal(archive, name)
	}
	return nil, &core.Error{Code: "unsupported-format", Msg: "not a comic archive"}
}

type zipPage struct {
	io.ReadCloser
	zr *zip.ReadCloser
}

func (z *zipPage) Close() error {
	err := z.ReadCloser.Close()
	z.zr.Close()
	return err
}

func openExternal(archive, name string) (io.ReadCloser, error) {
	tool, err := extractor()
	if err != nil {
		return nil, err
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	var cmd *exec.Cmd
	if filepath.Base(tool) == "bsdtar" {
		cmd = exec.CommandContext(ctx, tool, "-xOf", archive, "--", name)
	} else {
		cmd = exec.CommandContext(ctx, tool, "e", "-so", "-y", archive, name)
	}
	var buf bytes.Buffer
	cmd.Stdout = &limitedWriter{w: &buf, n: maxPageBytes}
	if err := cmd.Run(); err != nil || buf.Len() == 0 {
		return nil, &core.Error{Code: "invalid-archive", Msg: "could not extract page"}
	}
	return io.NopCloser(&buf), nil
}

type limitedWriter struct {
	w io.Writer
	n int64
}

func (l *limitedWriter) Write(p []byte) (int, error) {
	if int64(len(p)) > l.n {
		return 0, fmt.Errorf("page exceeds %d bytes", maxPageBytes)
	}
	l.n -= int64(len(p))
	return l.w.Write(p)
}

// coverProbe bounds how many leading pages are inspected for a cover.
const coverProbe = 4

// FirstPage opens the page that best represents the archive: the first
// of the leading pages that is portrait (a title banner or a wide
// spread makes a poor cover), else simply the first page.
func FirstPage(archive string) (io.ReadCloser, error) {
	es, err := list(archive)
	if err != nil {
		return nil, err
	}
	if len(es) == 0 {
		return nil, &core.Error{Code: "not-found", Msg: "archive has no pages"}
	}
	pick := es[0].name
	if Format(archive) == "cbz" {
		for i := 0; i < len(es) && i < coverProbe; i++ {
			rc, err := OpenPage(archive, es[i].name)
			if err != nil {
				continue
			}
			w, h := dimensions(rc, MimeOf(es[i].name))
			rc.Close()
			if h > w && w > 0 {
				pick = es[i].name
				break
			}
		}
	}
	return OpenPage(archive, pick)
}
