package comic

import (
	"archive/zip"
	"bytes"
	"encoding/binary"
	"image"
	_ "image/gif"
	_ "image/jpeg"
	_ "image/png"
	"io"

	"github.com/enrell/lain/internal/contracts"
)

// dimensions reads only the image header. The stdlib decodes jpeg, png
// and gif headers; webp is parsed here so the common manga format does
// not force clients to measure after load. Unknown formats return 0,0.
func dimensions(r io.Reader, mime string) (int, int) {
	head, _ := io.ReadAll(io.LimitReader(r, 1<<20))
	if mime == "image/webp" {
		return webpSize(head)
	}
	cfg, _, err := image.DecodeConfig(bytes.NewReader(head))
	if err != nil {
		return 0, 0
	}
	return cfg.Width, cfg.Height
}

func webpSize(b []byte) (int, int) {
	if len(b) < 30 || string(b[0:4]) != "RIFF" || string(b[8:12]) != "WEBP" {
		return 0, 0
	}
	switch string(b[12:16]) {
	case "VP8 ":
		if len(b) >= 30 && b[23] == 0x9d && b[24] == 0x01 && b[25] == 0x2a {
			return int(binary.LittleEndian.Uint16(b[26:]) & 0x3fff), int(binary.LittleEndian.Uint16(b[28:]) & 0x3fff)
		}
	case "VP8L":
		if b[20] == 0x2f {
			v := binary.LittleEndian.Uint32(b[21:])
			return int(v&0x3fff) + 1, int(v>>14&0x3fff) + 1
		}
	case "VP8X":
		w := int(b[24]) | int(b[25])<<8 | int(b[26])<<16
		h := int(b[27]) | int(b[28])<<8 | int(b[29])<<16
		return w + 1, h + 1
	}
	return 0, 0
}

// probeZip fills page dimensions with one archive open.
func probeZip(archive string, pages []contracts.ComicPage) {
	zr, err := zip.OpenReader(archive)
	if err != nil {
		return
	}
	defer zr.Close()
	byName := make(map[string]*zip.File, len(zr.File))
	for _, f := range zr.File {
		byName[f.Name] = f
	}
	for i := range pages {
		f := byName[pages[i].Name]
		if f == nil {
			continue
		}
		if rc, err := f.Open(); err == nil {
			pages[i].Width, pages[i].Height = dimensions(rc, pages[i].Mime)
			rc.Close()
		}
	}
}
