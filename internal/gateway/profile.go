package gateway

import (
	"bytes"
	"encoding/binary"
	"image"
	_ "image/gif"
	_ "image/jpeg"
	_ "image/png"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"

	"github.com/enrell/lain/internal/auth"
)

// Profile and avatar routes (D-086). Uploaded pictures are stored as
// files under <data>/avatars, one per user, named by user id only, so a
// request can never address a path.
const maxAvatarBytes = 2 << 20

// maxAvatarSide bounds decoded dimensions: a tiny file that declares a
// huge canvas is refused before any client tries to render it.
const maxAvatarSide = 4096

func (s *Server) routesProfile() {
	s.mux.HandleFunc("PATCH /api/me/profile", s.requireAuth(s.handleProfilePatch))
	s.mux.HandleFunc("PUT /api/me/avatar", s.requireAuth(s.handleAvatarPut))
	s.mux.HandleFunc("DELETE /api/me/avatar", s.requireAuth(s.handleAvatarDelete))
	s.mux.HandleFunc("GET /api/users/{id}/avatar", s.handleAvatarGet)
	s.mux.HandleFunc("GET /api/profile/mascots", s.requireAuth(func(w http.ResponseWriter, _ *http.Request, _ auth.Verified) {
		writeJSON(w, http.StatusOK, map[string]any{"mascots": auth.Mascots})
	}))
}

func (s *Server) avatarPath(userID string) string {
	return filepath.Join(s.avatarDir, userID)
}

func (s *Server) handleProfilePatch(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	var in auth.ProfilePatch
	if !s.decode(w, r, &in) {
		return
	}
	u, err := s.auth.UpdateProfile(v.UserID, in)
	if err != nil {
		writeErr(w, http.StatusBadRequest, err.Error())
		return
	}
	// Picking a mascot or clearing the avatar retires an uploaded file.
	if in.Mascot != nil {
		_ = os.Remove(s.avatarPath(v.UserID))
	}
	writeJSON(w, http.StatusOK, u)
}

// sniffAvatar accepts png, jpeg, gif and webp, identified by content,
// never by the client's Content-Type header.
func sniffAvatar(b []byte) (string, bool) {
	if len(b) >= 30 && string(b[0:4]) == "RIFF" && string(b[8:12]) == "WEBP" {
		w, h := webpDims(b)
		return "image/webp", w > 0 && h > 0 && w <= maxAvatarSide && h <= maxAvatarSide
	}
	cfg, format, err := image.DecodeConfig(bytes.NewReader(b))
	if err != nil {
		return "", false
	}
	if cfg.Width <= 0 || cfg.Height <= 0 || cfg.Width > maxAvatarSide || cfg.Height > maxAvatarSide {
		return "", false
	}
	switch format {
	case "png", "jpeg", "gif":
		return "image/" + format, true
	}
	return "", false
}

func webpDims(b []byte) (int, int) {
	switch string(b[12:16]) {
	case "VP8 ":
		if b[23] == 0x9d && b[24] == 0x01 && b[25] == 0x2a {
			return int(binary.LittleEndian.Uint16(b[26:]) & 0x3fff), int(binary.LittleEndian.Uint16(b[28:]) & 0x3fff)
		}
	case "VP8L":
		if b[20] == 0x2f {
			v := binary.LittleEndian.Uint32(b[21:])
			return int(v&0x3fff) + 1, int(v>>14&0x3fff) + 1
		}
	case "VP8X":
		return (int(b[24]) | int(b[25])<<8 | int(b[26])<<16) + 1, (int(b[27]) | int(b[28])<<8 | int(b[29])<<16) + 1
	}
	return 0, 0
}

func (s *Server) handleAvatarPut(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	defer r.Body.Close()
	body, err := io.ReadAll(io.LimitReader(r.Body, maxAvatarBytes+1))
	if err != nil {
		writeErr(w, http.StatusBadRequest, "could not read the image")
		return
	}
	if len(body) > maxAvatarBytes {
		writeErr(w, http.StatusRequestEntityTooLarge, "the image must be 2 MB or smaller")
		return
	}
	if _, ok := sniffAvatar(body); !ok {
		writeErr(w, http.StatusUnsupportedMediaType, "use a PNG, JPEG, GIF or WebP image up to 4096 px per side")
		return
	}
	if err := os.MkdirAll(s.avatarDir, 0o700); err != nil {
		writeErr(w, http.StatusInternalServerError, "avatar storage unavailable")
		return
	}
	tmp := s.avatarPath(v.UserID) + ".tmp"
	if err := os.WriteFile(tmp, body, 0o600); err != nil {
		writeErr(w, http.StatusInternalServerError, "avatar storage unavailable")
		return
	}
	if err := os.Rename(tmp, s.avatarPath(v.UserID)); err != nil {
		_ = os.Remove(tmp)
		writeErr(w, http.StatusInternalServerError, "avatar storage unavailable")
		return
	}
	u, err := s.auth.SetUploadedAvatar(v.UserID)
	if err != nil {
		writeErr(w, http.StatusBadRequest, err.Error())
		return
	}
	writeJSON(w, http.StatusOK, u)
}

func (s *Server) handleAvatarDelete(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	empty := ""
	u, err := s.auth.UpdateProfile(v.UserID, auth.ProfilePatch{Mascot: &empty})
	if err != nil {
		writeErr(w, http.StatusBadRequest, err.Error())
		return
	}
	_ = os.Remove(s.avatarPath(v.UserID))
	writeJSON(w, http.StatusOK, u)
}

// handleAvatarGet serves an uploaded picture to any signed-in user.
// <img> cannot send headers, so ?token= is accepted as for streams.
func (s *Server) handleAvatarGet(w http.ResponseWriter, r *http.Request) {
	if _, ok := s.userOf(r); !ok {
		writeErr(w, http.StatusUnauthorized, "unauthorized")
		return
	}
	id := r.PathValue("id")
	u, ok := s.auth.Get(id)
	if !ok || u.Profile.Avatar.Kind != auth.AvatarUpload || strings.ContainsAny(id, `/\`) {
		writeErr(w, http.StatusNotFound, "no avatar")
		return
	}
	body, err := os.ReadFile(s.avatarPath(id))
	if err != nil {
		writeErr(w, http.StatusNotFound, "no avatar")
		return
	}
	mime, ok := sniffAvatar(body)
	if !ok {
		writeErr(w, http.StatusNotFound, "no avatar")
		return
	}
	w.Header().Set("Content-Type", mime)
	w.Header().Set("X-Content-Type-Options", "nosniff")
	// The URL carries the avatar version, so a new picture is a new URL.
	w.Header().Set("Cache-Control", "private, max-age=31536000, immutable")
	_, _ = w.Write(body)
}
