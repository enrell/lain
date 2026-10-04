package downloads

import (
	"mime"
	"net/url"
	"os"
	"path"
	"path/filepath"
	"strconv"
	"strings"
	"unicode"
)

// maxNameBytes keeps a name under common filesystem limits (255) with
// room for a " (n)" collision suffix and the ".part" extension.
const maxNameBytes = 200

// SafeName reduces a suggested file name to one base name that cannot
// escape its directory: no separators, no "..", no control characters,
// no leading dot (hidden files never show up as media). It returns
// fallback when nothing usable is left.
func SafeName(s, fallback string) string {
	s = strings.ReplaceAll(s, "\\", "/")
	s = path.Base(s)
	s = strings.Map(func(r rune) rune {
		if unicode.IsControl(r) || r == '/' || r == unicode.ReplacementChar {
			return -1
		}
		switch r {
		case '<', '>', ':', '"', '|', '?', '*':
			return '_'
		}
		return r
	}, s)
	s = strings.TrimLeft(strings.TrimSpace(s), ".")
	s = strings.TrimSpace(s)
	if len(s) > maxNameBytes {
		ext := filepath.Ext(s)
		if len(ext) > 16 {
			ext = ""
		}
		cut := maxNameBytes - len(ext)
		for cut > 0 && !utfStart(s[cut]) {
			cut--
		}
		s = s[:cut] + ext
	}
	if s == "" || s == "." || s == ".." {
		return fallback
	}
	return s
}

func utfStart(b byte) bool { return b&0xC0 != 0x80 }

// NameFromURL is the last path segment of a URL, unescaped.
func NameFromURL(raw string) string {
	u, err := url.Parse(raw)
	if err != nil {
		return ""
	}
	seg := path.Base(u.Path)
	if seg == "/" || seg == "." {
		return ""
	}
	if un, err := url.PathUnescape(seg); err == nil {
		seg = un
	}
	return seg
}

// dispositionName extracts the filename a server suggests.
func dispositionName(v string) string {
	if v == "" {
		return ""
	}
	_, params, err := mime.ParseMediaType(v)
	if err != nil {
		return ""
	}
	return params["filename"]
}

// FreePath returns dir/name, or dir/"stem (n).ext" for the first n that
// does not exist yet: a download never overwrites a file it did not
// create. reserved names (in-flight downloads) count as taken.
func FreePath(dir, name string, reserved map[string]bool) string {
	ext := filepath.Ext(name)
	stem := strings.TrimSuffix(name, ext)
	cand := name
	for n := 2; ; n++ {
		p := filepath.Join(dir, cand)
		if _, err := os.Lstat(p); os.IsNotExist(err) && !reserved[p] {
			return p
		}
		cand = stem + " (" + strconv.Itoa(n) + ")" + ext
	}
}
