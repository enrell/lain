package gateway

import (
	"net/http"
	"os"
	"strconv"
	"strings"

	"github.com/enrell/lain/internal/contracts"
)

const (
	defaultThumbWidth = 480
	minThumbWidth     = 32
	maxThumbWidth     = 1280
	defaultThumbTime  = 10.0
)

// routesThumbnail exposes the image transform behind the data gateway.
// Auth happens inside the handler because <img> cannot attach an
// Authorization header; ?token= is accepted exactly like the stream
// endpoint.
func (s *Server) routesThumbnail() {
	s.mux.HandleFunc("GET /api/items/{id}/thumbnail", s.handleThumbnail)
}

// handleThumbnail resolves identity, asks the transform provider for a
// cached still, then streams the file: the provider never returns bytes.
func (s *Server) handleThumbnail(w http.ResponseWriter, r *http.Request) {
	if _, ok := s.userOf(r); !ok {
		writeErr(w, http.StatusUnauthorized, "unauthorized")
		return
	}
	it, ok := s.cat.Get(r.PathValue("id"))
	if !ok {
		writeErr(w, http.StatusNotFound, "unknown item")
		return
	}

	at := defaultThumbTime
	if raw := strings.TrimSpace(r.URL.Query().Get("t")); raw != "" {
		if v, err := strconv.ParseFloat(raw, 64); err == nil && v >= 0 {
			at = v
		}
	}
	width := defaultThumbWidth
	if raw := strings.TrimSpace(r.URL.Query().Get("w")); raw != "" {
		if v, err := strconv.Atoi(raw); err == nil && v > 0 {
			width = v
		}
	}
	if width < minThumbWidth {
		width = minThumbWidth
	}
	if width > maxThumbWidth {
		width = maxThumbWidth
	}

	out, _, err := s.reg.CallOne(contracts.CapTransformThumb, contracts.ThumbnailRequest{
		FilePath: it.FilePath,
		TimeSec:  at,
		Width:    width,
	})
	if err != nil {
		writeErr(w, http.StatusServiceUnavailable, err.Error())
		return
	}
	thumb, ok := out.(contracts.Thumbnail)
	if !ok || thumb.Path == "" {
		writeErr(w, http.StatusInternalServerError, "bad thumbnail result")
		return
	}
	f, err := os.Open(thumb.Path)
	if err != nil {
		writeErr(w, http.StatusNotFound, "thumbnail unavailable")
		return
	}
	defer f.Close()
	fi, err := f.Stat()
	if err != nil {
		writeErr(w, http.StatusInternalServerError, err.Error())
		return
	}
	w.Header().Set("Cache-Control", "private, max-age=86400")
	w.Header().Set("Content-Type", "image/jpeg")
	http.ServeContent(w, r, "", fi.ModTime(), f)
}
