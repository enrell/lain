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
	Info   *ComicInfo  `json:"info,omitempty"`
}

// ComicInfo is the metadata an archive carries about itself in a
// ComicInfo.xml member (the de-facto format of comic tools). Every
// field is optional; Info is nil when the archive has none or it does
// not parse. It never changes catalog identity, it only decorates.
type ComicInfo struct {
	Title     string   `json:"title,omitempty"`
	Series    string   `json:"series,omitempty"`
	Number    string   `json:"number,omitempty"` // issue/chapter, may be "12.5"
	Volume    int      `json:"volume,omitempty"`
	Count     int      `json:"count,omitempty"` // issues in the series
	Summary   string   `json:"summary,omitempty"`
	Year      int      `json:"year,omitempty"`
	Writer    string   `json:"writer,omitempty"`
	Artist    string   `json:"artist,omitempty"`
	Publisher string   `json:"publisher,omitempty"`
	Genres    []string `json:"genres,omitempty"`
	Language  string   `json:"language,omitempty"`
	AgeRating string   `json:"age_rating,omitempty"`
	// Direction is "rtl" when the file declares right-to-left manga,
	// "ltr" when it declares a western comic, "" when it says nothing.
	Direction string `json:"direction,omitempty"`
}
