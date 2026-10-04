package gateway

import (
	"errors"
	"net/http"

	"github.com/enrell/lain/internal/auth"
	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/downloads"
)

// routesDownloads exposes the server download manager (docs/slices/
// reading-downloads.md, P-3/P-4). Every route is admin-only: a download
// makes the server fetch an arbitrary URL and write to its disk.
func (s *Server) routesDownloads() {
	m := s.mux
	m.HandleFunc("GET /api/downloads", s.requireAdmin(s.handleDownloadsList))
	m.HandleFunc("POST /api/downloads", s.requireAdmin(s.handleDownloadAdd))
	m.HandleFunc("GET /api/downloads/settings", s.requireAdmin(s.handleDownloadSettingsGet))
	m.HandleFunc("PUT /api/downloads/settings", s.requireAdmin(s.handleDownloadSettingsPut))
	m.HandleFunc("POST /api/downloads/cleanup", s.requireAdmin(s.handleDownloadCleanup))
	m.HandleFunc("GET /api/downloads/{id}", s.requireAdmin(s.handleDownloadGet))
	m.HandleFunc("DELETE /api/downloads/{id}", s.requireAdmin(s.handleDownloadRemove))
	m.HandleFunc("POST /api/downloads/{id}/{action}", s.requireAdmin(s.handleDownloadAction))
}

// downloadsView is the queue page in one read.
type downloadsView struct {
	Jobs     []downloads.Job    `json:"jobs"`
	Usage    downloads.Usage    `json:"usage"`
	Settings downloads.Settings `json:"settings"`
}

// writeDownloadErr maps typed manager errors to HTTP statuses and keeps
// the stable code in the body next to the message.
func writeDownloadErr(w http.ResponseWriter, err error) {
	status := http.StatusInternalServerError
	switch downloads.CodeOf(err) {
	case downloads.CodeInvalid:
		status = http.StatusBadRequest
	case downloads.CodeNotFound:
		status = http.StatusNotFound
	case downloads.CodeState:
		status = http.StatusConflict
	case downloads.CodeQuota, downloads.CodeDiskFull:
		status = http.StatusInsufficientStorage
	}
	body := map[string]string{"error": err.Error()}
	var de *downloads.Error
	if errors.As(err, &de) {
		body["error"], body["code"] = de.Msg, de.Code
	}
	writeJSON(w, status, body)
}

func (s *Server) handleDownloadsList(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	writeJSON(w, http.StatusOK, downloadsView{Jobs: s.downloads.List(), Usage: s.downloads.Usage(), Settings: s.downloads.Settings()})
}

func (s *Server) handleDownloadAdd(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	var in struct {
		URL       string `json:"url"`
		Name      string `json:"name"`
		LibraryID string `json:"library_id"`
	}
	if !s.decode(w, r, &in) {
		return
	}
	add := downloads.AddInput{URL: in.URL, Name: in.Name, CreatedBy: v.UserID}
	if in.LibraryID != "" {
		lib, ok := s.libByID(in.LibraryID)
		if !ok {
			writeDownloadErr(w, &downloads.Error{Code: downloads.CodeNotFound, Msg: "unknown library"})
			return
		}
		add.Dir, add.LibraryID = lib.Path, lib.ID
	}
	j, err := s.downloads.Add(add)
	if err != nil {
		writeDownloadErr(w, err)
		return
	}
	s.logger().Info("download queued", "req", reqIDOf(r), "download", j.ID, "library", j.LibraryID)
	writeJSON(w, http.StatusCreated, j)
}

func (s *Server) handleDownloadGet(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	j, err := s.downloads.Get(r.PathValue("id"))
	if err != nil {
		writeDownloadErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, j)
}

func (s *Server) handleDownloadAction(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	id := r.PathValue("id")
	var (
		j   downloads.Job
		err error
	)
	switch r.PathValue("action") {
	case "pause":
		j, err = s.downloads.Pause(id)
	case "resume":
		j, err = s.downloads.Resume(id)
	case "cancel":
		j, err = s.downloads.Cancel(id)
	default:
		writeErr(w, http.StatusNotFound, "unknown action")
		return
	}
	if err != nil {
		writeDownloadErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, j)
}

func (s *Server) handleDownloadRemove(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	if err := s.downloads.Remove(r.PathValue("id")); err != nil {
		writeDownloadErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]string{"status": "ok"})
}

func (s *Server) handleDownloadSettingsGet(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	writeJSON(w, http.StatusOK, s.downloads.Settings())
}

func (s *Server) handleDownloadSettingsPut(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	var in downloads.Settings
	if !s.decode(w, r, &in) {
		return
	}
	out, err := s.downloads.SetSettings(in)
	if err != nil {
		writeDownloadErr(w, err)
		return
	}
	s.logger().Info("download settings saved", "req", reqIDOf(r), "max_bytes", out.MaxBytes, "min_free_bytes", out.MinFreeBytes, "concurrency", out.Concurrency)
	writeJSON(w, http.StatusOK, out)
}

func (s *Server) handleDownloadCleanup(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	rep, err := s.downloads.Cleanup()
	if err != nil {
		// Partial cleanups still report what they freed.
		s.logger().Warn("download cleanup incomplete", "req", reqIDOf(r), "err", err.Error())
	}
	writeJSON(w, http.StatusOK, rep)
}

// libByID finds a library by id.
func (s *Server) libByID(id string) (contracts.Library, bool) {
	for _, l := range s.libList() {
		if l.ID == id {
			return l, true
		}
	}
	return contracts.Library{}, false
}

// downloadDone rescans the library a finished download landed in, so
// it appears without waiting for the watcher or a manual scan.
func (s *Server) downloadDone(j downloads.Job) {
	s.logger().Info("download finished", "download", j.ID, "library", j.LibraryID, "bytes", j.Bytes)
	if j.LibraryID != "" {
		s.scanLibrary(j.LibraryID)
	}
}
