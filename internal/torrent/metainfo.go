// Package torrent is Lain's standard-library BitTorrent engine
// (docs/slices/acquisition.md, A-1/A-2): v1 metainfo and magnets, HTTP
// and UDP trackers, the peer wire protocol with ut_metadata, piece
// verification, resume and seeding. No DHT, PEX, uTP or encryption.
package torrent

import (
	"crypto/sha1"
	"encoding/base32"
	"encoding/hex"
	"errors"
	"fmt"
	"net/url"
	"path/filepath"
	"strings"

	"github.com/enrell/lain/internal/torrent/bencode"
)

// Bounds on what a metainfo may declare.
const (
	maxPieceLength = 64 << 20
	maxTotalBytes  = 1 << 40
	maxFiles       = 100_000
	maxPathDepth   = 32
)

// ErrMeta wraps every metainfo validation failure.
var ErrMeta = errors.New("torrent: bad metainfo")

func metaErr(format string, args ...any) error {
	return fmt.Errorf("%w: %s", ErrMeta, fmt.Sprintf(format, args...))
}

// InfoHash identifies a torrent: SHA-1 of the bencoded info dictionary.
type InfoHash [20]byte

func (h InfoHash) Hex() string    { return hex.EncodeToString(h[:]) }
func (h InfoHash) String() string { return h.Hex() }

// ParseInfoHash reads 40 hex characters.
func ParseInfoHash(s string) (InfoHash, error) {
	var h InfoHash
	b, err := hex.DecodeString(s)
	if err != nil || len(b) != 20 {
		return h, metaErr("info hash must be 40 hex characters")
	}
	copy(h[:], b)
	return h, nil
}

// File is one file of a torrent, in piece order. Path is relative and
// already validated: no empty, "." or ".." components, no separators.
type File struct {
	Path   []string `json:"path"`
	Length int64    `json:"length"`
	Offset int64    `json:"offset"`
}

// RelPath joins the components with the OS separator.
func (f File) RelPath() string { return filepath.Join(f.Path...) }

// Info is the validated info dictionary.
type Info struct {
	Name        string
	PieceLength int64
	Pieces      [][20]byte
	Files       []File
	Length      int64
	Private     bool
	// Multi is true for a directory torrent (files under Name/).
	Multi bool
	// Raw is the exact bencoded info dictionary (served over ut_metadata).
	Raw []byte
}

// NumPieces is the piece count.
func (i *Info) NumPieces() int { return len(i.Pieces) }

// PieceSize is the length of piece n (the last one may be short).
func (i *Info) PieceSize(n int) int64 {
	if n == len(i.Pieces)-1 {
		if r := i.Length - int64(n)*i.PieceLength; r > 0 {
			return r
		}
	}
	return i.PieceLength
}

// MetaInfo is a parsed .torrent file.
type MetaInfo struct {
	Info     *Info
	InfoHash InfoHash
	// Trackers are announce URLs, tiers flattened in order, deduplicated.
	Trackers []string
}

// ParseMetaInfo parses and validates a .torrent file.
func ParseMetaInfo(data []byte) (*MetaInfo, error) {
	top, raw, err := bencode.DecodeDict(data)
	if err != nil {
		return nil, err
	}
	rawInfo, ok := raw["info"]
	if !ok {
		return nil, metaErr("no info dictionary")
	}
	info, err := ParseInfo(rawInfo)
	if err != nil {
		return nil, err
	}
	mi := &MetaInfo{Info: info, InfoHash: sha1.Sum(rawInfo)}
	seen := map[string]bool{}
	add := func(u string) {
		if u != "" && !seen[u] && validTrackerURL(u) {
			seen[u] = true
			mi.Trackers = append(mi.Trackers, u)
		}
	}
	if tiers, ok := bencode.List(top, "announce-list"); ok {
		for _, tier := range tiers {
			if l, ok := tier.([]any); ok {
				for _, u := range l {
					if s, ok := u.(string); ok {
						add(s)
					}
				}
			}
		}
	}
	if a, ok := bencode.Str(top, "announce"); ok {
		add(a)
	}
	return mi, nil
}

func validTrackerURL(s string) bool {
	u, err := url.Parse(s)
	if err != nil || u.Host == "" {
		return false
	}
	return u.Scheme == "http" || u.Scheme == "https" || u.Scheme == "udp"
}

// safeComponent rejects path parts that could escape the download dir.
func safeComponent(s string) bool {
	if s == "" || s == "." || s == ".." || len(s) > 255 {
		return false
	}
	return !strings.ContainsAny(s, "/\\\x00")
}

// ParseInfo validates a raw bencoded info dictionary.
func ParseInfo(raw []byte) (*Info, error) {
	v, err := bencode.Decode(raw)
	if err != nil {
		return nil, err
	}
	d, ok := v.(map[string]any)
	if !ok {
		return nil, metaErr("info is not a dictionary")
	}
	info := &Info{Raw: append([]byte(nil), raw...)}
	if info.Name, ok = bencode.Str(d, "name"); !ok || !safeComponent(info.Name) {
		return nil, metaErr("unsafe or missing name")
	}
	if info.PieceLength, ok = bencode.Int(d, "piece length"); !ok || info.PieceLength <= 0 || info.PieceLength > maxPieceLength {
		return nil, metaErr("bad piece length")
	}
	pieces, ok := bencode.Str(d, "pieces")
	if !ok || len(pieces) == 0 || len(pieces)%20 != 0 {
		return nil, metaErr("bad pieces")
	}
	if p, ok := bencode.Int(d, "private"); ok && p == 1 {
		info.Private = true
	}
	if length, ok := bencode.Int(d, "length"); ok {
		if length < 0 {
			return nil, metaErr("negative length")
		}
		info.Files = []File{{Path: []string{info.Name}, Length: length}}
		info.Length = length
	} else {
		files, ok := bencode.List(d, "files")
		if !ok || len(files) == 0 || len(files) > maxFiles {
			return nil, metaErr("no length and no files")
		}
		info.Multi = true
		seen := map[string]bool{}
		for _, fv := range files {
			fd, ok := fv.(map[string]any)
			if !ok {
				return nil, metaErr("file entry is not a dictionary")
			}
			length, ok := bencode.Int(fd, "length")
			if !ok || length < 0 {
				return nil, metaErr("bad file length")
			}
			parts, ok := bencode.List(fd, "path")
			if !ok || len(parts) == 0 || len(parts) > maxPathDepth {
				return nil, metaErr("bad file path")
			}
			f := File{Length: length, Offset: info.Length}
			f.Path = append(f.Path, info.Name)
			for _, p := range parts {
				s, ok := p.(string)
				if !ok || !safeComponent(s) {
					return nil, metaErr("unsafe file path")
				}
				f.Path = append(f.Path, s)
			}
			key := strings.ToLower(strings.Join(f.Path, "/"))
			if seen[key] {
				return nil, metaErr("duplicate file path")
			}
			seen[key] = true
			info.Files = append(info.Files, f)
			info.Length += length
			if info.Length > maxTotalBytes {
				return nil, metaErr("torrent too large")
			}
		}
	}
	if info.Length > maxTotalBytes {
		return nil, metaErr("torrent too large")
	}
	n := len(pieces) / 20
	want := (info.Length + info.PieceLength - 1) / info.PieceLength
	if info.Length == 0 {
		want = 1
	}
	if int64(n) != want {
		return nil, metaErr("piece count %d does not match length (want %d)", n, want)
	}
	info.Pieces = make([][20]byte, n)
	for i := range info.Pieces {
		copy(info.Pieces[i][:], pieces[i*20:])
	}
	return info, nil
}

// Magnet is a parsed magnet link.
type Magnet struct {
	InfoHash InfoHash
	Name     string
	Trackers []string
}

// ParseMagnet reads magnet:?xt=urn:btih:<hex|base32>&dn=..&tr=..
func ParseMagnet(s string) (*Magnet, error) {
	u, err := url.Parse(s)
	if err != nil || u.Scheme != "magnet" {
		return nil, metaErr("not a magnet link")
	}
	q := u.Query()
	m := &Magnet{Name: q.Get("dn")}
	found := false
	for _, xt := range q["xt"] {
		const p = "urn:btih:"
		if !strings.HasPrefix(strings.ToLower(xt), p) {
			continue
		}
		v := xt[len(p):]
		switch len(v) {
		case 40:
			h, err := ParseInfoHash(v)
			if err != nil {
				return nil, err
			}
			m.InfoHash = h
		case 32:
			b, err := base32.StdEncoding.DecodeString(strings.ToUpper(v))
			if err != nil || len(b) != 20 {
				return nil, metaErr("bad base32 info hash")
			}
			copy(m.InfoHash[:], b)
		default:
			return nil, metaErr("bad info hash length")
		}
		found = true
		break
	}
	if !found {
		return nil, metaErr("magnet has no btih")
	}
	seen := map[string]bool{}
	for _, tr := range q["tr"] {
		if validTrackerURL(tr) && !seen[tr] {
			seen[tr] = true
			m.Trackers = append(m.Trackers, tr)
		}
	}
	return m, nil
}

// String renders the magnet back to a link.
func (m *Magnet) String() string {
	q := url.Values{}
	q.Set("xt", "urn:btih:"+m.InfoHash.Hex())
	if m.Name != "" {
		q.Set("dn", m.Name)
	}
	for _, tr := range m.Trackers {
		q.Add("tr", tr)
	}
	return "magnet:?" + q.Encode()
}

// SourceFile is content for Build.
type SourceFile struct {
	Path []string // relative to the torrent name; one element for single-file
	Data []byte
}

// Build creates a .torrent from in-memory content. Lain never authors
// torrents for publishing; tests and fixtures use it to make swarms.
func Build(name string, files []SourceFile, pieceLength int64, trackers []string) ([]byte, *MetaInfo, error) {
	var all []byte
	info := map[string]any{"name": name, "piece length": pieceLength}
	if len(files) == 1 && len(files[0].Path) <= 1 {
		info["length"] = int64(len(files[0].Data))
		all = files[0].Data
	} else {
		var list []any
		for _, f := range files {
			p := make([]any, len(f.Path))
			for i, s := range f.Path {
				p[i] = s
			}
			list = append(list, map[string]any{"length": int64(len(f.Data)), "path": p})
			all = append(all, f.Data...)
		}
		info["files"] = list
	}
	var pieces []byte
	for off := 0; off < len(all) || off == 0; off += int(pieceLength) {
		end := off + int(pieceLength)
		if end > len(all) {
			end = len(all)
		}
		h := sha1.Sum(all[off:end])
		pieces = append(pieces, h[:]...)
		if end == len(all) {
			break
		}
	}
	info["pieces"] = string(pieces)
	top := map[string]any{"info": info}
	if len(trackers) > 0 {
		top["announce"] = trackers[0]
		tier := make([]any, len(trackers))
		for i, t := range trackers {
			tier[i] = t
		}
		top["announce-list"] = []any{tier}
	}
	raw, err := bencode.Encode(top)
	if err != nil {
		return nil, nil, err
	}
	mi, err := ParseMetaInfo(raw)
	return raw, mi, err
}
