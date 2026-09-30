package gateway

import (
	"errors"
	"fmt"
	"io"
	"net/http"
	"strconv"

	"github.com/enrell/lain/internal/auth"
	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/plugins/comic"
)

// routesReader exposes the reading slice (D-085). Page bytes are
// streamed from the archive by the gateway; the comic provider only
// indexes. Auth happens inside the page handler because <img> cannot
// attach an Authorization header (?token= as for streams).
func (s *Server) routesReader() {
	s.mux.HandleFunc("GET /api/items/{id}/pages", s.requireAuth(s.handlePages))
	s.mux.HandleFunc("GET /api/items/{id}/pages/{n}", s.handlePage)
}

// readerView is the page index a client renders.
type readerView struct {
	Kind      string                `json:"kind"`
	Format    string                `json:"format"`
	Direction string                `json:"direction"` // default reading direction: rtl | ltr
	Pages     []contracts.ComicPage `json:"pages"`
}

func defaultDirection(kind string) string {
	if kind == contracts.KindManga {
		return "rtl"
	}
	return "ltr"
}

// pagesFor resolves an item to its page index, replying on failure.
func (s *Server) pagesFor(w http.ResponseWriter, id string) (contracts.CatalogItem, contracts.ComicPages, bool) {
	it, ok := s.catGet(id)
	if !ok {
		writeErr(w, http.StatusNotFound, "unknown item")
		return it, contracts.ComicPages{}, false
	}
	if it.Kind != contracts.KindComic && it.Kind != contracts.KindManga {
		writeErr(w, http.StatusBadRequest, "item is not readable")
		return it, contracts.ComicPages{}, false
	}
	if it.Missing {
		writeErr(w, http.StatusNotFound, "file unavailable")
		return it, contracts.ComicPages{}, false
	}
	out, _, err := s.reg.CallOne(contracts.CapComicPages, contracts.ComicPagesInput{Path: it.FilePath})
	if err != nil {
		var ce *core.Error
		status := http.StatusServiceUnavailable
		if errors.As(err, &ce) {
			switch ce.Code {
			case "invalid-archive", "unsupported-format":
				status = http.StatusUnprocessableEntity
			}
		}
		s.logger().Warn("comic pages failed", "item", it.ID, "err", err.Error())
		writeErr(w, status, err.Error())
		return it, contracts.ComicPages{}, false
	}
	pages, ok := out.(contracts.ComicPages)
	if !ok {
		writeErr(w, http.StatusInternalServerError, "bad pages result")
		return it, contracts.ComicPages{}, false
	}
	return it, pages, true
}

func (s *Server) handlePages(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	it, pages, ok := s.pagesFor(w, r.PathValue("id"))
	if !ok {
		return
	}
	writeJSON(w, http.StatusOK, readerView{
		Kind: it.Kind, Format: pages.Format, Direction: defaultDirection(it.Kind), Pages: pages.Pages,
	})
}

func (s *Server) handlePage(w http.ResponseWriter, r *http.Request) {
	if _, ok := s.userOf(r); !ok {
		writeErr(w, http.StatusUnauthorized, "unauthorized")
		return
	}
	n, err := strconv.Atoi(r.PathValue("n"))
	if err != nil || n < 0 {
		writeErr(w, http.StatusBadRequest, "bad page")
		return
	}
	it, pages, ok := s.pagesFor(w, r.PathValue("id"))
	if !ok {
		return
	}
	if n >= len(pages.Pages) {
		writeErr(w, http.StatusNotFound, "no such page")
		return
	}
	pg := pages.Pages[n]
	etag := fmt.Sprintf(`"%d-%d-%d"`, it.Size, pg.Size, n)
	if r.Header.Get("If-None-Match") == etag {
		w.WriteHeader(http.StatusNotModified)
		return
	}
	rc, err := comic.OpenPage(it.FilePath, pg.Name)
	if err != nil {
		writeErr(w, http.StatusNotFound, "page unavailable")
		return
	}
	defer rc.Close()
	w.Header().Set("Content-Type", pg.Mime)
	w.Header().Set("Cache-Control", "private, max-age=86400")
	w.Header().Set("ETag", etag)
	if pg.Size > 0 {
		w.Header().Set("Content-Length", strconv.FormatInt(pg.Size, 10))
	}
	_, _ = io.Copy(w, rc)
}
