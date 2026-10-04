package comic

import (
	"reflect"
	"strings"
	"testing"
	"unicode/utf8"

	"github.com/enrell/lain/internal/contracts"
)

const sampleInfo = `<?xml version="1.0"?>
<ComicInfo xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <Title>The Long Road</Title>
  <Series>Frieren</Series>
  <Number>12.5</Number>
  <Volume>3</Volume>
  <Summary>A mage
     walks on.</Summary>
  <Year>2021</Year>
  <Writer>Writer A</Writer>
  <Penciller>Artist B</Penciller>
  <Publisher>Publisher-A</Publisher>
  <Genre>Fantasy, Adventure, ,Drama</Genre>
  <LanguageISO>ja</LanguageISO>
  <Manga>YesAndRightToLeft</Manga>
</ComicInfo>`

func TestParseInfoFields(t *testing.T) {
	got := ParseInfo([]byte(sampleInfo))
	if got == nil {
		t.Fatal("nil info")
	}
	want := contracts.ComicInfo{
		Title: "The Long Road", Series: "Frieren", Number: "12.5", Volume: 3,
		Summary: "A mage walks on.", Year: 2021, Writer: "Writer A", Artist: "Artist B",
		Publisher: "Publisher-A", Genres: []string{"Fantasy", "Adventure", "Drama"},
		Language: "ja", Direction: "rtl",
	}
	if !reflect.DeepEqual(*got, want) {
		t.Fatalf("info = %+v\nwant %+v", *got, want)
	}
}

func TestParseInfoDirectionAndBounds(t *testing.T) {
	for doc, dir := range map[string]string{
		"<ComicInfo><Manga>No</Manga></ComicInfo>":                    "ltr",
		"<ComicInfo><Manga>Yes</Manga><Series>S</Series></ComicInfo>": "",
	} {
		if got := ParseInfo([]byte(doc)); got == nil || got.Direction != dir {
			t.Errorf("%s: %+v, want direction %q", doc, got, dir)
		}
	}
	got := ParseInfo([]byte("<ComicInfo><Year>99999</Year><Volume>-2</Volume><Series>S</Series></ComicInfo>"))
	if got == nil || got.Year != 0 || got.Volume != 0 {
		t.Fatalf("out-of-range numbers must drop: %+v", got)
	}
	long := strings.Repeat("é", 5000)
	got = ParseInfo([]byte("<ComicInfo><Summary>" + long + "</Summary><Title>" + long + "</Title></ComicInfo>"))
	if utf8.RuneCountInString(got.Summary) != maxInfoSummary || utf8.RuneCountInString(got.Title) != maxInfoField {
		t.Fatalf("caps: summary %d title %d", utf8.RuneCountInString(got.Summary), utf8.RuneCountInString(got.Title))
	}
}

func TestParseInfoNothingUseful(t *testing.T) {
	for _, doc := range []string{"", "not xml", "<ComicInfo></ComicInfo>", "<ComicInfo><Manga>Yes</Manga></ComicInfo>"} {
		if got := ParseInfo([]byte(doc)); got != nil {
			t.Errorf("%q: %+v, want nil", doc, got)
		}
	}
}

func TestReadInfoFromArchive(t *testing.T) {
	names := []string{"nested/ComicInfo.xml", "comicinfo.xml", "p1.png"}
	p := makeZip(t, "a.cbz", names, map[string][]byte{
		"nested/ComicInfo.xml": []byte("<ComicInfo><Series>Nested</Series></ComicInfo>"),
		"comicinfo.xml":        []byte("<ComicInfo><Series>Root</Series></ComicInfo>"),
		"p1.png":               pngBytes(4, 6),
	})
	out, err := (&Provider{}).Invoke(contracts.CapComicPages, contracts.ComicPagesInput{Path: p})
	if err != nil {
		t.Fatal(err)
	}
	pages := out.(contracts.ComicPages)
	if pages.Info == nil || pages.Info.Series != "Root" {
		t.Fatalf("info = %+v, want the root copy", pages.Info)
	}
	if len(pages.Pages) != 1 {
		t.Fatalf("ComicInfo.xml must not be a page: %d pages", len(pages.Pages))
	}
	bare := makeZip(t, "b.cbz", []string{"p1.png"}, map[string][]byte{"p1.png": pngBytes(4, 6)})
	if ReadInfo(bare) != nil {
		t.Fatal("archive without ComicInfo.xml must have nil info")
	}
	if ReadInfo("/nonexistent/x.cbz") != nil {
		t.Fatal("unreadable archive must have nil info")
	}
}

func FuzzParseInfo(f *testing.F) {
	f.Add([]byte(sampleInfo))
	f.Add([]byte("<ComicInfo><Number>\x00\x01</Number></ComicInfo>"))
	f.Add([]byte("<ComicInfo><Genre>,,,</Genre><Year>-1</Year></ComicInfo>"))
	f.Fuzz(func(t *testing.T, raw []byte) {
		info := ParseInfo(raw)
		if info == nil {
			return
		}
		for _, s := range []string{info.Title, info.Series, info.Number, info.Summary, info.Writer, info.Artist, info.Publisher, info.Language, info.AgeRating} {
			if !utf8.ValidString(s) {
				t.Fatalf("invalid utf-8: %q", s)
			}
			for _, r := range s {
				if r < 0x20 || r == 0x7f {
					t.Fatalf("control char in %q", s)
				}
			}
		}
		if utf8.RuneCountInString(info.Summary) > maxInfoSummary || len(info.Genres) > maxInfoGenres {
			t.Fatal("caps exceeded")
		}
		if info.Direction != "" && info.Direction != "rtl" && info.Direction != "ltr" {
			t.Fatalf("direction %q", info.Direction)
		}
	})
}
