package torrent

import (
	"crypto/sha1"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sync"
)

// storage maps the torrent's byte space onto its files under dir.
// Files are created sparse on first write; reads past what exists
// return zeros so a partial piece simply fails verification.
type storage struct {
	dir  string
	info *Info

	mu    sync.Mutex
	files map[int]*os.File
}

func newStorage(dir string, info *Info) *storage {
	return &storage{dir: dir, info: info, files: map[int]*os.File{}}
}

// Path is the on-disk path of file i.
func (s *storage) path(i int) string {
	return filepath.Join(append([]string{s.dir}, s.info.Files[i].Path...)...)
}

func (s *storage) open(i int, create bool) (*os.File, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if f := s.files[i]; f != nil {
		return f, nil
	}
	p := s.path(i)
	if fi, err := os.Lstat(p); err == nil && !fi.Mode().IsRegular() {
		return nil, fmt.Errorf("torrent: %s is not a regular file", p)
	}
	if !create {
		f, err := os.Open(p)
		if err != nil {
			return nil, err
		}
		return f, nil // read-only handles are not cached
	}
	if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
		return nil, err
	}
	f, err := os.OpenFile(p, os.O_RDWR|os.O_CREATE, 0o644)
	if err != nil {
		return nil, err
	}
	if fi, err := f.Stat(); err == nil && fi.Size() > s.info.Files[i].Length {
		_ = f.Truncate(s.info.Files[i].Length)
	}
	s.files[i] = f
	return f, nil
}

// span calls fn for every file segment covering [off, off+n).
func (s *storage) span(off, n int64, fn func(i int, fileOff, bufOff, length int64) error) error {
	var done int64
	for i, f := range s.info.Files {
		if n-done == 0 {
			break
		}
		end := f.Offset + f.Length
		if off+done >= end || f.Length == 0 {
			continue
		}
		start := off + done - f.Offset
		length := min(end-(off+done), n-done)
		if err := fn(i, start, done, length); err != nil {
			return err
		}
		done += length
	}
	if done != n {
		return errors.New("torrent: range outside torrent")
	}
	return nil
}

// WriteAt stores b at torrent offset off.
func (s *storage) WriteAt(b []byte, off int64) error {
	return s.span(off, int64(len(b)), func(i int, fileOff, bufOff, length int64) error {
		f, err := s.open(i, true)
		if err != nil {
			return err
		}
		_, err = f.WriteAt(b[bufOff:bufOff+length], fileOff)
		return err
	})
}

// ReadAt fills b from torrent offset off; missing data reads as zeros.
func (s *storage) ReadAt(b []byte, off int64) error {
	return s.span(off, int64(len(b)), func(i int, fileOff, bufOff, length int64) error {
		dst := b[bufOff : bufOff+length]
		f, err := s.open(i, false)
		if errors.Is(err, os.ErrNotExist) {
			clear(dst)
			return nil
		}
		if err != nil {
			return err
		}
		n, err := f.ReadAt(dst, fileOff)
		if errors.Is(err, io.EOF) {
			clear(dst[n:])
			err = nil
		}
		if !s.cached(i, f) {
			f.Close()
		}
		return err
	})
}

func (s *storage) cached(i int, f *os.File) bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.files[i] == f
}

// Verify hashes piece n against the metainfo.
func (s *storage) Verify(n int) (bool, error) {
	buf := make([]byte, s.info.PieceSize(n))
	if err := s.ReadAt(buf, int64(n)*s.info.PieceLength); err != nil {
		return false, err
	}
	return sha1.Sum(buf) == s.info.Pieces[n], nil
}

// Complete reports whether every file exists at its full size: the
// condition under which a saved resume bitfield is trusted.
func (s *storage) Complete() bool {
	for i, f := range s.info.Files {
		fi, err := os.Stat(s.path(i))
		if err != nil || fi.Size() != f.Length {
			return false
		}
	}
	return true
}

// Close releases cached handles.
func (s *storage) Close() {
	s.mu.Lock()
	defer s.mu.Unlock()
	for i, f := range s.files {
		_ = f.Close()
		delete(s.files, i)
	}
}

// Remove deletes the torrent's files and any directories left empty.
func (s *storage) Remove() error {
	s.Close()
	var first error
	for i := range s.info.Files {
		if err := os.Remove(s.path(i)); err != nil && !errors.Is(err, os.ErrNotExist) && first == nil {
			first = err
		}
	}
	if s.info.Multi {
		root := filepath.Join(s.dir, s.info.Name)
		// Remove now-empty directories, deepest first.
		var dirs []string
		_ = filepath.WalkDir(root, func(p string, d os.DirEntry, err error) error {
			if err == nil && d.IsDir() {
				dirs = append(dirs, p)
			}
			return nil
		})
		for i := len(dirs) - 1; i >= 0; i-- {
			_ = os.Remove(dirs[i])
		}
	}
	return first
}
