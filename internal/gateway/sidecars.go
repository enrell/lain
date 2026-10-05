package gateway

import (
	"errors"
	"io"
	"net/http"
	"os"
	"strconv"

	"github.com/enrell/lain/internal/auth"
	"github.com/enrell/lain/internal/subtitle"
)

// Sidecar subtitles (docs/slices/acquisition.md, A-28/A-29): subtitle
// files next to a media file, found by name and served as WebVTT. Any
// signed-in user may read them, like the media itself; clients address
// a sidecar by index, never by path.

func (s *Server) routesSidecars() {
	s.mux.HandleFunc("GET /api/items/{id}/sidecars", s.requireAuth(s.handleSidecars))
	s.mux.HandleFunc("GET /api/items/{id}/sidecars/{n}", s.handleSidecar) // ?token= like media
}

type sidecarView struct {
	Index int `json:"index"`
	subtitle.Sidecar
}

func (s *Server) handleSidecars(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	it, ok := s.catGet(r.PathValue("id"))
	if !ok {
		writeErr(w, http.StatusNotFound, "unknown item")
		return
	}
	out := []sidecarView{}
	for i, sc := range subtitle.Discover(it.FilePath) {
		out = append(out, sidecarView{Index: i, Sidecar: sc})
	}
	writeJSON(w, http.StatusOK, map[string]any{"sidecars": out})
}

func (s *Server) handleSidecar(w http.ResponseWriter, r *http.Request) {
	if _, ok := s.userOf(r); !ok {
		writeErr(w, http.StatusUnauthorized, "unauthorized")
		return
	}
	it, ok := s.catGet(r.PathValue("id"))
	if !ok {
		writeErr(w, http.StatusNotFound, "unknown item")
		return
	}
	list := subtitle.Discover(it.FilePath)
	n, err := strconv.Atoi(r.PathValue("n"))
	if err != nil || n < 0 || n >= len(list) {
		writeErr(w, http.StatusNotFound, "no such sidecar")
		return
	}
	f, err := os.Open(list[n].Path)
	if err != nil {
		writeErr(w, http.StatusNotFound, "no such sidecar")
		return
	}
	defer f.Close()
	raw, err := io.ReadAll(io.LimitReader(f, subtitle.MaxBytes+1))
	if err != nil {
		writeErr(w, http.StatusInternalServerError, "could not read the sidecar")
		return
	}
	cues, err := subtitle.Parse(raw)
	if err != nil {
		code := http.StatusUnprocessableEntity
		if !errors.Is(err, subtitle.ErrFormat) {
			code = http.StatusInternalServerError
		}
		writeJSON(w, code, map[string]string{"error": "this subtitle file cannot be read", "code": "unsupported-media"})
		return
	}
	w.Header().Set("Content-Type", "text/vtt; charset=utf-8")
	w.Header().Set("Cache-Control", "private, no-cache")
	_, _ = w.Write(subtitle.WebVTT(cues))
}
