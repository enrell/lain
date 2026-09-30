package comic

import (
	"archive/zip"
	"bytes"
	"encoding/binary"
	"image"
	"image/png"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/testutil/contract"
)

func pngBytes(w, h int) []byte {
	var b bytes.Buffer
	_ = png.Encode(&b, image.NewGray(image.Rect(0, 0, w, h)))
	return b.Bytes()
}

// makeZip writes name->bytes in the given order.
func makeZip(t *testing.T, name string, names []string, data map[string][]byte) string {
	t.Helper()
	p := filepath.Join(t.TempDir(), name)
	f, err := os.Create(p)
	if err != nil {
		t.Fatal(err)
	}
	zw := zip.NewWriter(f)
	for _, n := range names {
		w, _ := zw.Create(n)
		_, _ = w.Write(data[n])
	}
	if err := zw.Close(); err != nil {
		t.Fatal(err)
	}
	f.Close()
	return p
}

func TestPagesAreNaturalOrderedAndFiltered(t *testing.T) {
	names := []string{
		"page10.png", "page2.png", "page1.png", "__MACOSX/page1.png", ".hidden.png",
		"ComicInfo.xml", "ch1/", "notes.txt", "page3.PNG",
	}
	data := map[string][]byte{}
	for _, n := range names {
		data[n] = pngBytes(20, 30)
	}
	p := makeZip(t, "a.cbz", names, data)
	out, err := (&Provider{}).Invoke(contracts.CapComicPages, contracts.ComicPagesInput{Path: p})
	if err != nil {
		t.Fatal(err)
	}
	pages := out.(contracts.ComicPages)
	var got []string
	for _, pg := range pages.Pages {
		got = append(got, pg.Name)
	}
	want := []string{"page1.png", "page2.png", "page3.PNG", "page10.png"}
	if len(got) != len(want) {
		t.Fatalf("pages = %v, want %v", got, want)
	}
	for i := range want {
		if got[i] != want[i] || pages.Pages[i].Index != i {
			t.Fatalf("pages = %v, want %v", got, want)
		}
	}
	if pages.Format != "cbz" || pages.Pages[0].Width != 20 || pages.Pages[0].Height != 30 {
		t.Fatalf("format/dims = %s %dx%d", pages.Format, pages.Pages[0].Width, pages.Pages[0].Height)
	}
}

func TestNaturalLess(t *testing.T) {
	cases := []struct {
		a, b string
		less bool
	}{
		{"2.jpg", "10.jpg", true}, {"10.jpg", "2.jpg", false},
		{"ch1/2.jpg", "ch10/1.jpg", true}, {"a01.jpg", "a1.jpg", false},
		{"a1.jpg", "a01.jpg", true}, {"A1.jpg", "a2.jpg", true},
	}
	for _, c := range cases {
		if naturalLess(c.a, c.b) != c.less {
			t.Errorf("naturalLess(%q,%q) != %v", c.a, c.b, c.less)
		}
	}
}

func TestOpenPageRefusesNonImagesAndUnknownNames(t *testing.T) {
	p := makeZip(t, "a.cbz", []string{"1.png", "secret.txt"}, map[string][]byte{"1.png": pngBytes(1, 1), "secret.txt": []byte("x")})
	rc, err := OpenPage(p, "1.png")
	if err != nil {
		t.Fatal(err)
	}
	b, _ := io.ReadAll(rc)
	rc.Close()
	if len(b) == 0 {
		t.Fatal("empty page")
	}
	for _, n := range []string{"secret.txt", "missing.png", "../1.png"} {
		if _, err := OpenPage(p, n); err == nil {
			t.Errorf("OpenPage(%q) must fail", n)
		} else if _, ok := err.(*core.Error); !ok {
			t.Errorf("OpenPage(%q) untyped error %T", n, err)
		}
	}
}

func TestInvalidAndUnsupportedArchives(t *testing.T) {
	dir := t.TempDir()
	bad := filepath.Join(dir, "bad.cbz")
	_ = os.WriteFile(bad, []byte("not a zip"), 0o600)
	for _, p := range []string{bad, filepath.Join(dir, "x.pdf")} {
		_, err := (&Provider{}).Invoke(contracts.CapComicPages, contracts.ComicPagesInput{Path: p})
		if err == nil {
			t.Errorf("%s must fail", p)
		}
	}
}

func TestIndexCacheFollowsFileIdentity(t *testing.T) {
	p := makeZip(t, "a.cbz", []string{"1.png"}, map[string][]byte{"1.png": pngBytes(1, 1)})
	prov := &Provider{}
	first, _ := prov.Invoke(contracts.CapComicPages, contracts.ComicPagesInput{Path: p})
	// Replace the archive with a longer one at the same path.
	q := makeZip(t, "b.cbz", []string{"1.png", "2.png"}, map[string][]byte{"1.png": pngBytes(1, 1), "2.png": pngBytes(1, 1)})
	data, _ := os.ReadFile(q)
	_ = os.WriteFile(p, data, 0o600)
	second, _ := prov.Invoke(contracts.CapComicPages, contracts.ComicPagesInput{Path: p})
	if len(first.(contracts.ComicPages).Pages) != 1 || len(second.(contracts.ComicPages).Pages) != 2 {
		t.Fatal("cache served a stale index for a replaced archive")
	}
}

func TestWebpSize(t *testing.T) {
	b := make([]byte, 40)
	copy(b, "RIFF")
	copy(b[8:], "WEBPVP8X")
	// VP8X: canvas size is stored minus one, 24 bits little-endian.
	b[24], b[25], b[26] = 99, 0, 0
	b[27], b[28], b[29] = 199, 0, 0
	if w, h := webpSize(b); w != 100 || h != 200 {
		t.Fatalf("VP8X = %dx%d", w, h)
	}
	l := make([]byte, 40)
	copy(l, "RIFF")
	copy(l[8:], "WEBPVP8L")
	l[20] = 0x2f
	binary.LittleEndian.PutUint32(l[21:], uint32(49)|uint32(79)<<14)
	if w, h := webpSize(l); w != 50 || h != 80 {
		t.Fatalf("VP8L = %dx%d", w, h)
	}
	if w, h := webpSize([]byte("junk")); w != 0 || h != 0 {
		t.Fatal("junk must give 0x0")
	}
}

// bsdtar reads zip content regardless of extension, so a zip named .cbr
// exercises the external-extractor path without a rar encoder.
func TestExternalExtractorPath(t *testing.T) {
	if _, err := exec.LookPath("bsdtar"); err != nil {
		t.Skip("bsdtar not installed")
	}
	p := makeZip(t, "a.cbr", []string{"2.png", "1.png"}, map[string][]byte{"1.png": pngBytes(2, 2), "2.png": pngBytes(2, 2)})
	out, err := (&Provider{}).Invoke(contracts.CapComicPages, contracts.ComicPagesInput{Path: p})
	if err != nil {
		t.Fatal(err)
	}
	pages := out.(contracts.ComicPages)
	if len(pages.Pages) != 2 || pages.Pages[0].Name != "1.png" {
		t.Fatalf("pages = %+v", pages.Pages)
	}
	rc, err := OpenPage(p, "2.png")
	if err != nil {
		t.Fatal(err)
	}
	b, _ := io.ReadAll(rc)
	if !bytes.Equal(b, pngBytes(2, 2)) {
		t.Fatal("extracted bytes differ")
	}
}

func TestProviderContract(t *testing.T) {
	p := makeZip(t, "a.cbz", []string{"1.png"}, map[string][]byte{"1.png": pngBytes(1, 1)})
	contract.Run(t, &Provider{}, []contract.Cap{{
		Name: contracts.CapComicPages, Sample: contracts.ComicPagesInput{Path: p},
	}})
}

func TestFirstPagePrefersPortrait(t *testing.T) {
	banner, portrait := pngBytes(40, 10), pngBytes(10, 20)
	p := makeZip(t, "a.cbz", []string{"0.png", "1.png"}, map[string][]byte{"0.png": banner, "1.png": portrait})
	rc, err := FirstPage(p)
	if err != nil {
		t.Fatal(err)
	}
	defer rc.Close()
	b, _ := io.ReadAll(rc)
	if !bytes.Equal(b, portrait) {
		t.Fatal("cover must skip the wide banner")
	}
	q := makeZip(t, "b.cbz", []string{"0.png"}, map[string][]byte{"0.png": banner})
	rc2, err := FirstPage(q)
	if err != nil {
		t.Fatal(err)
	}
	defer rc2.Close()
	b, _ = io.ReadAll(rc2)
	if !bytes.Equal(b, banner) {
		t.Fatal("fallback must be the first page")
	}
}
