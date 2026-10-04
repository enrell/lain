package subtitle

import (
	"encoding/binary"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

// Formats Lain reads as sidecars (A-28).
var formats = map[string]bool{"srt": true, "ass": true, "ssa": true, "vtt": true}

// iso1 maps ISO 639-1 to ISO 639-2/T, the form D-071 stores.
var iso1 = map[string]string{
	"af": "afr", "ar": "ara", "az": "aze", "be": "bel", "bg": "bul", "bn": "ben", "bs": "bos", "ca": "cat",
	"cs": "ces", "cy": "cym", "da": "dan", "de": "deu", "el": "ell", "en": "eng", "eo": "epo", "es": "spa",
	"et": "est", "eu": "eus", "fa": "fas", "fi": "fin", "fr": "fra", "ga": "gle", "gl": "glg", "he": "heb",
	"hi": "hin", "hr": "hrv", "hu": "hun", "hy": "hye", "id": "ind", "is": "isl", "it": "ita", "ja": "jpn",
	"ka": "kat", "kk": "kaz", "km": "khm", "kn": "kan", "ko": "kor", "lt": "lit", "lv": "lav", "mk": "mkd",
	"ml": "mal", "mn": "mon", "mr": "mar", "ms": "msa", "my": "mya", "nb": "nob", "ne": "nep", "nl": "nld",
	"nn": "nno", "no": "nor", "pa": "pan", "pl": "pol", "pt": "por", "ro": "ron", "ru": "rus", "si": "sin",
	"sk": "slk", "sl": "slv", "sq": "sqi", "sr": "srp", "sv": "swe", "sw": "swa", "ta": "tam", "te": "tel",
	"th": "tha", "tl": "tgl", "tr": "tur", "uk": "ukr", "ur": "urd", "uz": "uzb", "vi": "vie", "zh": "zho",
}

// bibliographic maps ISO 639-2/B codes to their /T forms (the same
// table the web player's track matching uses).
var bibliographic = map[string]string{
	"fre": "fra", "ger": "deu", "chi": "zho", "dut": "nld", "gre": "ell", "rum": "ron", "cze": "ces", "slo": "slk",
	"per": "fas", "may": "msa", "alb": "sqi", "arm": "hye", "baq": "eus", "bur": "mya", "ice": "isl", "mac": "mkd",
	"geo": "kat", "wel": "cym",
}

var iso2 = func() map[string]string {
	m := map[string]string{"fil": "", "und": ""}
	for two, three := range iso1 {
		m[three] = two
	}
	return m
}()

// Language folds a language tag (ISO 639-1, 639-2/T or /B, with or
// without a region) to ISO 639-2/T; "" when it is not a known language.
func Language(tag string) string {
	t := strings.ToLower(strings.ReplaceAll(strings.TrimSpace(tag), "_", "-"))
	t, _, _ = strings.Cut(t, "-")
	switch len(t) {
	case 2:
		return iso1[t]
	case 3:
		if b, ok := bibliographic[t]; ok {
			return b
		}
		if _, ok := iso2[t]; ok && t != "und" {
			return t
		}
	}
	return ""
}

// Tag is the language tag Lain writes in a sidecar name: the region
// form when the provider gave one ("pt-BR"), else ISO 639-1 when it
// exists, else the 639-2 code (A-35).
func Tag(lang, region string) string {
	if region != "" {
		return region
	}
	if two := iso2[lang]; two != "" {
		return two
	}
	return lang
}

// Sidecar is one subtitle file next to a media file.
type Sidecar struct {
	Path     string `json:"-"`
	Name     string `json:"name"`
	Language string `json:"language"` // ISO 639-2/T, "und" when unknown
	Tag      string `json:"tag,omitempty"`
	Forced   bool   `json:"forced,omitempty"`
	HI       bool   `json:"hi,omitempty"`
	Format   string `json:"format"`
}

// stem is the media file name without its extension.
func stem(mediaPath string) string {
	base := filepath.Base(mediaPath)
	return strings.TrimSuffix(base, filepath.Ext(base))
}

// Discover lists the sidecars of a media file, by name only (A-28),
// sorted by file name.
func Discover(mediaPath string) []Sidecar {
	dir, prefix := filepath.Dir(mediaPath), stem(mediaPath)+"."
	entries, err := os.ReadDir(dir)
	if err != nil {
		return nil
	}
	var out []Sidecar
	for _, e := range entries {
		name := e.Name()
		if e.IsDir() || !strings.HasPrefix(name, prefix) {
			continue
		}
		ext := strings.ToLower(strings.TrimPrefix(filepath.Ext(name), "."))
		if !formats[ext] {
			continue
		}
		if !e.Type().IsRegular() {
			continue // a symlink could point anywhere
		}
		s := Sidecar{Path: filepath.Join(dir, name), Name: name, Format: ext, Language: "und"}
		// "<stem>.srt" has no middle; "<stem>.en.forced.srt" has "en.forced".
		middle := ""
		if rest := name[len(prefix):]; strings.Contains(rest, ".") {
			middle = strings.TrimSuffix(rest, filepath.Ext(name))
		}
		for _, tok := range strings.Split(middle, ".") {
			switch strings.ToLower(tok) {
			case "":
			case "forced":
				s.Forced = true
			case "sdh", "hi", "cc":
				s.HI = true
			case "default":
			default:
				if s.Tag == "" {
					s.Tag = tok
					if l := Language(tok); l != "" {
						s.Language = l
					}
				}
			}
		}
		out = append(out, s)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Name < out[j].Name })
	return out
}

// SidecarPath is where Lain writes a subtitle for mediaPath (A-35):
// <stem>.<tag>[.forced][.sdh].<format>.
func SidecarPath(mediaPath, lang, region string, forced, hi bool, format string) string {
	parts := []string{stem(mediaPath), Tag(lang, region)}
	if forced {
		parts = append(parts, "forced")
	}
	if hi {
		parts = append(parts, "sdh")
	}
	return filepath.Join(filepath.Dir(mediaPath), strings.Join(parts, ".")+"."+format)
}

// MovieHash is the OpenSubtitles file hash (A-31): the file size plus
// the 64-bit little-endian words of the first and last 64 KiB.
func MovieHash(path string) (string, error) {
	f, err := os.Open(path)
	if err != nil {
		return "", err
	}
	defer f.Close()
	fi, err := f.Stat()
	if err != nil {
		return "", err
	}
	const chunk = 65536
	size := fi.Size()
	if size < chunk {
		return "", errors.New("subtitle: file too small to hash")
	}
	h := uint64(size)
	buf := make([]byte, chunk)
	for _, off := range []int64{0, size - chunk} {
		if _, err := f.ReadAt(buf, off); err != nil && !errors.Is(err, io.EOF) {
			return "", err
		}
		for i := 0; i < chunk; i += 8 {
			h += binary.LittleEndian.Uint64(buf[i:])
		}
	}
	return fmt.Sprintf("%016x", h), nil
}
