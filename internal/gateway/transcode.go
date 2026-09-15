package gateway

import (
	"errors"
	"net/http"
	"os"

	"github.com/enrell/lain/internal/auth"
	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

// routesTranscode exposes the prepared MP4 behind the data gateway.
// Auth happens inside the handler because <video> cannot attach an
// Authorization header; ?token= is accepted exactly like the stream
// endpoint. Output is a complete MP4 with +faststart, so Range
// seeking works through ServeContent like a direct stream.
func (s *Server) routesTranscode() {
	s.mux.HandleFunc("GET /api/items/{id}/transcode", s.handleTranscode)
	s.mux.HandleFunc("POST /api/items/{id}/transcode", s.requireAuth(s.handleTranscodeStart))
	s.mux.HandleFunc("GET /api/items/{id}/transcode/status", s.requireAuth(s.handleTranscodeStatus))
	s.mux.HandleFunc("GET /api/items/{id}/subtitles", s.handleSubtitles)
}

type publicTranscodeStatus struct {
	Session     string `json:"session"`
	State       string `json:"state"`
	Profile     string `json:"profile"`
	Method      string `json:"method,omitempty"`
	Cached      bool   `json:"cached,omitempty"`
	HasSubtitle bool   `json:"has_subtitle,omitempty"`
	ErrorCode   string `json:"error_code,omitempty"`
	Error       string `json:"error,omitempty"`
	QueuedAt    int64  `json:"queued_at,omitempty"`
	StartedAt   int64  `json:"started_at,omitempty"`
	FinishedAt  int64  `json:"finished_at,omitempty"`
}

func publicStatus(s contracts.TranscodeStatus) publicTranscodeStatus {
	return publicTranscodeStatus{
		Session: s.Session, State: s.State, Profile: s.Profile,
		Method: s.Method, Cached: s.Cached, HasSubtitle: s.SubtitlePath != "",
		ErrorCode: s.ErrorCode, Error: s.Error,
		QueuedAt: s.QueuedAt, StartedAt: s.StartedAt, FinishedAt: s.FinishedAt,
	}
}

func writeTranscodeError(w http.ResponseWriter, err error) {
	code, status, message := "dependency-unavailable", http.StatusServiceUnavailable, "transcode unavailable"
	var ce *core.Error
	if errors.As(err, &ce) {
		code = ce.Code
		switch ce.Code {
		case "invalid-message":
			status, message = http.StatusBadRequest, ce.Msg
		case "queue-full":
			status, message = http.StatusTooManyRequests, ce.Msg
		case "not-found":
			status, message = http.StatusNotFound, ce.Msg
		case "unsupported-media":
			status, message = http.StatusUnprocessableEntity, ce.Msg
		}
	}
	writeJSON(w, status, map[string]string{"error": message, "code": code})
}

type transcodeSelection struct {
	Profile        string `json:"profile,omitempty"`
	AudioStream    *int   `json:"audio_stream,omitempty"`
	SubtitleStream *int   `json:"subtitle_stream,omitempty"`
}

func (s *Server) handleTranscodeStart(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	it, ok := s.cat.Get(r.PathValue("id"))
	if !ok {
		writeErr(w, http.StatusNotFound, "unknown item")
		return
	}
	var selection transcodeSelection
	if r.ContentLength > 0 && !s.decode(w, r, &selection) {
		return
	}
	out, _, err := s.reg.CallOne(contracts.CapPlaybackTranscodeV2, contracts.TranscodeV2Request{
		Action: contracts.TranscodeStartAction, FilePath: it.FilePath,
		Profile: selection.Profile, AudioStream: selection.AudioStream, SubtitleStream: selection.SubtitleStream,
	})
	if err != nil {
		s.logger().Warn("transcode start failed", "req", reqIDOf(r), "item", it.ID, "err", err.Error())
		writeTranscodeError(w, err)
		return
	}
	status, ok := out.(contracts.TranscodeStatus)
	if !ok {
		writeErr(w, http.StatusInternalServerError, "bad transcode status")
		return
	}
	code := http.StatusAccepted
	if status.State == contracts.TranscodeReady {
		code = http.StatusOK
	} else {
		w.Header().Set("Retry-After", "1")
	}
	s.logger().Info("transcode start", "req", reqIDOf(r), "item", it.ID, "state", status.State, "session", status.Session)
	writeJSON(w, code, publicStatus(status))
}

func (s *Server) handleTranscodeStatus(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	it, ok := s.cat.Get(r.PathValue("id"))
	if !ok {
		writeErr(w, http.StatusNotFound, "unknown item")
		return
	}
	session := r.URL.Query().Get("session")
	if session == "" {
		writeJSON(w, http.StatusBadRequest, map[string]string{"error": "session required", "code": "invalid-message"})
		return
	}
	out, _, err := s.reg.CallOne(contracts.CapPlaybackTranscodeV2, contracts.TranscodeV2Request{
		Action: contracts.TranscodeStatusAction, FilePath: it.FilePath, Session: session,
	})
	if err != nil {
		writeTranscodeError(w, err)
		return
	}
	status, ok := out.(contracts.TranscodeStatus)
	if !ok {
		writeErr(w, http.StatusInternalServerError, "bad transcode status")
		return
	}
	writeJSON(w, http.StatusOK, publicStatus(status))
}

// handleTranscode resolves identity, asks the transcode provider for
// the cached MP4, then streams the file: the provider never returns
// bytes. First play of an item prepares the file inline, so the first
// response takes as long as the remux/re-encode; later plays are
// instant cache hits. Absent ffmpeg degrades to 503, never a fake
// stream.
func (s *Server) handleTranscode(w http.ResponseWriter, r *http.Request) {
	if _, ok := s.userOf(r); !ok {
		writeErr(w, http.StatusUnauthorized, "unauthorized")
		return
	}
	it, ok := s.cat.Get(r.PathValue("id"))
	if !ok {
		writeErr(w, http.StatusNotFound, "unknown item")
		return
	}

	var out any
	var err error
	if session := r.URL.Query().Get("session"); session != "" {
		out, _, err = s.reg.CallOne(contracts.CapPlaybackTranscodeV2, contracts.TranscodeV2Request{
			Action: contracts.TranscodeResolveAction, FilePath: it.FilePath, Session: session,
		})
		if err == nil {
			status, ok := out.(contracts.TranscodeStatus)
			if !ok {
				writeErr(w, http.StatusInternalServerError, "bad transcode status")
				return
			}
			if status.State != contracts.TranscodeReady {
				w.Header().Set("Retry-After", "1")
				writeJSON(w, http.StatusAccepted, publicStatus(status))
				return
			}
			out = contracts.Transcode{Path: status.Path, Method: status.Method, Cached: status.Cached}
		}
	} else {
		// No session is the retained synchronous v1 compatibility path.
		out, _, err = s.reg.CallOne(contracts.CapPlaybackTranscode, contracts.TranscodeRequest{FilePath: it.FilePath})
	}
	if err != nil {
		s.logger().Warn("transcode failed", "req", reqIDOf(r), "item", it.ID, "err", err.Error())
		writeTranscodeError(w, err)
		return
	}
	prepared, ok := out.(contracts.Transcode)
	if !ok || prepared.Path == "" {
		s.logger().Error("transcode bad result", "req", reqIDOf(r), "item", it.ID)
		writeErr(w, http.StatusInternalServerError, "bad transcode result")
		return
	}
	f, err := os.Open(prepared.Path)
	if err != nil {
		writeErr(w, http.StatusNotFound, "transcode unavailable")
		return
	}
	defer f.Close()
	fi, err := f.Stat()
	if err != nil {
		writeErr(w, http.StatusInternalServerError, err.Error())
		return
	}
	w.Header().Set("Content-Type", "video/mp4")
	w.Header().Set("Accept-Ranges", "bytes")
	http.ServeContent(w, r, it.ID+".mp4", fi.ModTime(), f)
}

// handleSubtitles serves the WebVTT sidecar for a ready transcode
// session. Auth accepts ?token= like other media endpoints; the
// provider path never leaves the server.
func (s *Server) handleSubtitles(w http.ResponseWriter, r *http.Request) {
	if _, ok := s.userOf(r); !ok {
		writeErr(w, http.StatusUnauthorized, "unauthorized")
		return
	}
	it, ok := s.cat.Get(r.PathValue("id"))
	if !ok {
		writeErr(w, http.StatusNotFound, "unknown item")
		return
	}
	session := r.URL.Query().Get("session")
	if session == "" {
		writeJSON(w, http.StatusBadRequest, map[string]string{"error": "session required", "code": "invalid-message"})
		return
	}
	out, _, err := s.reg.CallOne(contracts.CapPlaybackTranscodeV2, contracts.TranscodeV2Request{
		Action: contracts.TranscodeResolveAction, FilePath: it.FilePath, Session: session,
	})
	if err != nil {
		writeTranscodeError(w, err)
		return
	}
	status, ok := out.(contracts.TranscodeStatus)
	if !ok || status.State != contracts.TranscodeReady || status.SubtitlePath == "" {
		writeErr(w, http.StatusNotFound, "subtitles unavailable")
		return
	}
	f, err := os.Open(status.SubtitlePath)
	if err != nil {
		writeErr(w, http.StatusNotFound, "subtitles unavailable")
		return
	}
	defer f.Close()
	fi, err := f.Stat()
	if err != nil {
		writeErr(w, http.StatusInternalServerError, err.Error())
		return
	}
	w.Header().Set("Content-Type", "text/vtt; charset=utf-8")
	http.ServeContent(w, r, it.ID+".vtt", fi.ModTime(), f)
}
