package comic

import (
	"archive/zip"
	"encoding/xml"
	"io"
	"path"
	"strconv"
	"strings"
	"unicode"

	"github.com/enrell/lain/internal/contracts"
)

// maxInfoBytes bounds a ComicInfo.xml read; real files are a few KB.
const maxInfoBytes = 1 << 20

// Field length caps keep one hostile archive from bloating every page
// index response.
const (
	maxInfoField   = 256
	maxInfoSummary = 4000
	maxInfoGenres  = 16
)

// comicInfoXML is the subset of the ComicInfo schema lain shows.
type comicInfoXML struct {
	Title       string `xml:"Title"`
	Series      string `xml:"Series"`
	Number      string `xml:"Number"`
	Volume      string `xml:"Volume"`
	Count       string `xml:"Count"`
	Summary     string `xml:"Summary"`
	Year        string `xml:"Year"`
	Writer      string `xml:"Writer"`
	Penciller   string `xml:"Penciller"`
	Publisher   string `xml:"Publisher"`
	Genre       string `xml:"Genre"`
	LanguageISO string `xml:"LanguageISO"`
	AgeRating   string `xml:"AgeRating"`
	Manga       string `xml:"Manga"`
}

// ReadInfo returns the archive's ComicInfo.xml, or nil when it has none
// or it does not parse. Metadata is decoration: every failure is "no
// info", never an error that would block reading.
func ReadInfo(archive string) *contracts.ComicInfo {
	var rc io.ReadCloser
	switch Format(archive) {
	case "cbz":
		zr, err := zip.OpenReader(archive)
		if err != nil {
			return nil
		}
		defer zr.Close()
		var best *zip.File
		for _, f := range zr.File {
			if !strings.EqualFold(path.Base(f.Name), "ComicInfo.xml") {
				continue
			}
			// The root copy wins over one nested in a folder.
			if best == nil || strings.Count(f.Name, "/") < strings.Count(best.Name, "/") {
				best = f
			}
		}
		if best == nil {
			return nil
		}
		r, err := best.Open()
		if err != nil {
			return nil
		}
		rc = r
	case "cbr", "cb7":
		r, err := openExternal(archive, "ComicInfo.xml")
		if err != nil {
			return nil
		}
		rc = r
	default:
		return nil
	}
	defer rc.Close()
	raw, err := io.ReadAll(io.LimitReader(rc, maxInfoBytes+1))
	if err != nil || len(raw) > maxInfoBytes {
		return nil
	}
	return ParseInfo(raw)
}

// ParseInfo decodes a ComicInfo.xml document. It returns nil when the
// document does not parse or carries nothing lain shows.
func ParseInfo(raw []byte) *contracts.ComicInfo {
	var doc comicInfoXML
	dec := xml.NewDecoder(strings.NewReader(string(raw)))
	dec.Strict = false
	if err := dec.Decode(&doc); err != nil {
		return nil
	}
	info := contracts.ComicInfo{
		Title:     clean(doc.Title, maxInfoField),
		Series:    clean(doc.Series, maxInfoField),
		Number:    clean(doc.Number, 16),
		Volume:    bounded(doc.Volume, 0, 9999),
		Count:     bounded(doc.Count, 0, 99999),
		Summary:   clean(doc.Summary, maxInfoSummary),
		Year:      bounded(doc.Year, 1800, 2200),
		Writer:    clean(doc.Writer, maxInfoField),
		Artist:    clean(doc.Penciller, maxInfoField),
		Publisher: clean(doc.Publisher, maxInfoField),
		Language:  clean(doc.LanguageISO, 16),
		AgeRating: clean(doc.AgeRating, 64),
	}
	for _, g := range strings.Split(doc.Genre, ",") {
		if g = clean(g, 64); g != "" && len(info.Genres) < maxInfoGenres {
			info.Genres = append(info.Genres, g)
		}
	}
	switch strings.ToLower(strings.TrimSpace(doc.Manga)) {
	case "yesandrighttoleft":
		info.Direction = "rtl"
	case "no":
		info.Direction = "ltr"
	}
	if info.Title == "" && info.Series == "" && info.Number == "" && info.Volume == 0 &&
		info.Summary == "" && info.Year == 0 && info.Writer == "" && info.Artist == "" &&
		info.Publisher == "" && len(info.Genres) == 0 && info.Direction == "" &&
		info.Count == 0 && info.Language == "" && info.AgeRating == "" {
		return nil
	}
	return &info
}

// clean drops control characters, collapses whitespace runs (a summary
// becomes one paragraph) and caps
// the length in runes (never splitting a UTF-8 sequence).
func clean(s string, max int) string {
	s = strings.Map(func(r rune) rune {
		if r == unicode.ReplacementChar || unicode.IsControl(r) {
			return -1
		}
		return r
	}, s)
	s = strings.Join(strings.Fields(s), " ")
	if r := []rune(s); len(r) > max {
		s = strings.TrimSpace(string(r[:max]))
	}
	return s
}

// bounded parses an integer field, 0 when absent or out of range.
func bounded(s string, lo, hi int) int {
	n, err := strconv.Atoi(strings.TrimSpace(s))
	if err != nil || n < lo || n > hi {
		return 0
	}
	return n
}
