package contracts

// CapComicPages lists the readable pages of a comic or manga archive.
// Page bytes never cross the plugin boundary: the provider returns the
// page index and the gateway streams a page from the archive on demand
// (docs/ARCHITECTURE.md), exactly like media bytes.
const CapComicPages = "lain.comic.pages@1"

// Library and item kinds of the reading slice (D-085).
const (
	KindComic = "comic"
	KindManga = "manga"
)

// ComicPagesInput names the archive. Path is supplied by the gateway
// after catalog lookup, never by a client.
type ComicPagesInput struct {
	Path string `json:"path"`
}

// ComicPage is one image of an archive in reading order. Width and
// Height are 0 when the format header could not be read; clients then
// measure after load. Name is the archive entry name and is never sent
// to clients: they address a page by Index.
type ComicPage struct {
	Index  int    `json:"index"`
	Name   string `json:"-"`
	Mime   string `json:"mime"`
	Size   int64  `json:"size"`
	Width  int    `json:"width"`
	Height int    `json:"height"`
}

// ComicPages is the page index of one archive.
type ComicPages struct {
	Format string      `json:"format"`
	Pages  []ComicPage `json:"pages"`
}
