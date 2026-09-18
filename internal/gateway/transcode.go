package gateway

import (
	"errors"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"

	"github.com/enrell/lain/internal/auth"
	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

// routesTranscode exposes the prepared output behind the data gateway.
// Auth happens inside the media handlers because <video>/hls.js cannot
// attach an Authorization header; ?token= is accepted exactly like the
// stream endpoint. Progressive output is a complete MP4 with
// +faststart (Range works through ServeContent); HLS sessions serve
// the server-owned playlist and its segments.
func (s *Server) routesTranscode() {
	s.mux.HandleFunc("GET /api/items/{id}/transcode", s.handleTranscode)
	s.mux.HandleFunc("POST /api/items/{id}/transcode", s.requireAuth(s.handleTranscodeStart))
	s.mux.HandleFunc("DELETE /api/items/{id}/transcode", s.requireAuth(s.handleTranscodeCancel))
	s.mux.HandleFunc("GET /api/items/{id}/transcode/status", s.requireAuth(s.handleTranscodeStatus))
	s.mux.HandleFunc("GET /api/items/{id}/transcode/hls/{file}", s.handleTranscodeHLS)
	s.mux.HandleFunc("GET /api/items/{id}/subtitles", s.handleSubtitles)

	// Player-facing policy (quality ladder, preferred delivery).
	s.mux.HandleFunc("GET /api/playback/options", s.requireAuth(s.handlePlaybackOptions))

	// Operator surface (D-045): settings plus active sessions.
	s.mux.HandleFunc("GET /api/admin/settings/transcode", s.requireAdmin(s.handleTranscodeSettingsGet))
	s.mux.HandleFunc("PUT /api/admin/settings/transcode", s.requireAdmin(s.handleTranscodeSettingsPut))
	s.mux.HandleFunc("GET /api/admin/transcodes", s.requireAdmin(s.handleAdminTranscodes))
	s.mux.HandleFunc("DELETE /api/admin/transcodes/{session}", s.requireAdmin(s.handleAdminTranscodeCancel))
}

// publicTranscodeStatus is the client-safe projection: trusted paths
// never leave the server.
type publicTranscodeStatus struct {
	Session  string `json:"session"`
	State    string `json:"state"`
	Profile  string `json:"profile"`
	Delivery string `json:"delivery,omitempty"`

	Method   string `json:"method,omitempty"`
	Cached   bool   `json:"cached,omitempty"`
	Playable bool   `json:"playable,omitempty"`

	Encoder  string `json:"encoder,omitempty"`
	Hardware string `json:"hardware,omitempty"`
	Fallback string `json:"fallback,omitempty"`

	Reasons []string `json:"reasons,omitempty"`

	VideoCodec  string `json:"video_codec,omitempty"`
	AudioCodec  string `json:"audio_codec,omitempty"`
	Width       int    `json:"width,omitempty"`
	Height      int    `json:"height,omitempty"`
	BitrateKbps int    `json:"bitrate_kbps,omitempty"`

	// Per-stream direct flags (Jellyfin's TranscodingInfo): true means
	// that stream is stream-copied rather than re-encoded.
	VideoDirect bool `json:"video_direct,omitempty"`
	AudioDirect bool `json:"audio_direct,omitempty"`

	// Live pipeline metrics while the job runs (Jellyfin's transcoding
	// info): encoding speed and the actual output bitrate.
	FPS               float64 `json:"fps,omitempty"`
	OutputBitrateKbps int     `json:"output_bitrate_kbps,omitempty"`

	Progress    float64 `json:"progress,omitempty"`
	HasSubtitle bool    `json:"has_subtitle,omitempty"`
	ErrorCode   string  `json:"error_code,omitempty"`
	Error       string  `json:"error,omitempty"`
	QueuedAt    int64   `json:"queued_at,omitempty"`
	StartedAt   int64   `json:"started_at,omitempty"`
	FinishedAt  int64   `json:"finished_at,omitempty"`
}

func publicStatusV3(s contracts.TranscodeV3Status) publicTranscodeStatus {
	return publicTranscodeStatus{
		Session: s.Session, State: s.State, Profile: s.Profile, Delivery: s.Delivery,
		Method: s.Method, Cached: s.Cached, Playable: s.Playable,
		Encoder: s.Encoder, Hardware: s.Hardware, Fallback: s.Fallback,
		Reasons: s.Reasons, VideoCodec: s.VideoCodec, AudioCodec: s.AudioCodec,
		Width: s.Width, Height: s.Height, BitrateKbps: s.BitrateKbps,
		VideoDirect: s.VideoDirect, AudioDirect: s.AudioDirect,
		FPS: s.FPS, OutputBitrateKbps: s.OutputBitrateKbps,
		Progress: s.Progress, HasSubtitle: s.SubtitlePath != "",
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
		case "forbidden":
			status, message = http.StatusForbidden, ce.Msg
		case "queue-full":
			status, message = http.StatusTooManyRequests, ce.Msg
		case "too-many-streams":
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
	Delivery       string `json:"delivery,omitempty"`
	Quality        string `json:"quality,omitempty"`
	VideoCodec     string `json:"video_codec,omitempty"`
	AudioCodec     string `json:"audio_codec,omitempty"`
	AudioStream    *int   `json:"audio_stream,omitempty"`
	SubtitleStream *int   `json:"subtitle_stream,omitempty"`
	SubtitleMode   string `json:"subtitle_mode,omitempty"`
	// Client-side permissions, mirroring Jellyfin's PlaybackInfo flags.
	AllowVideoStreamCopy *bool `json:"allow_video_stream_copy,omitempty"`
	AllowAudioStreamCopy *bool `json:"allow_audio_stream_copy,omitempty"`
}

// transcodeRequest builds the plugin request from operator settings,
// the user's limits and the client's selection.
func (s *Server) transcodeRequest(v auth.Verified, filePath string, selection transcodeSelection) contracts.TranscodeV3Request {
	settings := s.settings.Transcode()
	policy := s.auth.PlaybackPolicy(v.UserID)
	var limits *contracts.TranscodePolicy
	if policy.Restricted() {
		limits = &contracts.TranscodePolicy{
			AllowVideoTranscode: policy.AllowsVideoTranscode(),
			AllowAudioTranscode: policy.AllowsAudioTranscode(),
			AllowRemux:          policy.AllowsRemux(),
			MaxStreams:          policy.MaxStreams,
		}
	}
	// The effective bitrate cap is the tighter of the account limit and
	// the server-wide remote client limit (0 means unlimited).
	maxBitrate := effectiveBitrateLimit(policy.MaxBitrateKbps, settings.RemoteBitrateLimitKbps)
	subtitleMode := selection.SubtitleMode
	if subtitleMode == "" {
		subtitleMode = policy.SubtitleMode
	}
	return contracts.TranscodeV3Request{
		Action:         contracts.TranscodeStartAction,
		FilePath:       filePath,
		Delivery:       selection.Delivery,
		Quality:        selection.Quality,
		MaxBitrateKbps: maxBitrate,
		VideoCodec:     selection.VideoCodec,
		AudioCodec:     selection.AudioCodec,
		AudioStream:    selection.AudioStream,
		SubtitleStream: selection.SubtitleStream,
		SubtitleMode:   subtitleMode,
		Policy:         limits,
		UserID:         v.UserID,
		Settings:       settings,

		AllowVideoStreamCopy: selection.AllowVideoStreamCopy,
		AllowAudioStreamCopy: selection.AllowAudioStreamCopy,
	}
}

// effectiveBitrateLimit returns the tighter non-zero cap, or 0 when
// neither is set.
func effectiveBitrateLimit(userKbps, serverKbps int) int {
	switch {
	case userKbps <= 0:
		return serverKbps
	case serverKbps <= 0:
		return userKbps
	case userKbps < serverKbps:
		return userKbps
	default:
		return serverKbps
	}
}

// sourceBitrateKbps sums the probed video and audio bitrates; 0 when the
// source bitrate is unknown, so a cap never forces a transcode blindly.
func sourceBitrateKbps(info *contracts.MediaInfo) int {
	if info == nil {
		return 0
	}
	total := 0
	for _, s := range info.Streams {
		if s.Type == "video" || s.Type == "audio" {
			total += s.BitRate
		}
	}
	return total / 1000
}

func (s *Server) handleTranscodeStart(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	it, ok := s.cat.Get(r.PathValue("id"))
	if !ok {
		writeErr(w, http.StatusNotFound, "unknown item")
		return
	}
	var selection transcodeSelection
	if r.ContentLength > 0 && !s.decode(w, r, &selection) {
		return
	}
	req := s.transcodeRequest(v, it.FilePath, selection)
	out, _, err := s.reg.CallOne(contracts.CapPlaybackTranscodeV3, req)
	if err != nil {
		s.logger().Warn("transcode start failed", "req", reqIDOf(r), "item", it.ID, "err", err.Error())
		writeTranscodeError(w, err)
		return
	}
	status, ok := out.(contracts.TranscodeV3Status)
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
	s.logger().Info("transcode start", "req", reqIDOf(r), "item", it.ID,
		"state", status.State, "session", status.Session, "delivery", status.Delivery, "method", status.Method)
	writeJSON(w, code, publicStatusV3(status))
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
	out, _, err := s.reg.CallOne(contracts.CapPlaybackTranscodeV3, contracts.TranscodeV3Request{
		Action: contracts.TranscodeStatusAction, FilePath: it.FilePath, Session: session,
	})
	if err != nil {
		writeTranscodeError(w, err)
		return
	}
	status, ok := out.(contracts.TranscodeV3Status)
	if !ok {
		writeErr(w, http.StatusInternalServerError, "bad transcode status")
		return
	}
	writeJSON(w, http.StatusOK, publicStatusV3(status))
}

// handleTranscodeCancel stops a session the caller owns (the player
// leaving for good, or an explicit stop).
func (s *Server) handleTranscodeCancel(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	session := r.URL.Query().Get("session")
	if session == "" {
		writeJSON(w, http.StatusBadRequest, map[string]string{"error": "session required", "code": "invalid-message"})
		return
	}
	// Bind the session to the item named in the path and to the caller:
	// a client may only cancel a session it owns, for the item it asked
	// for. An empty user id (admin cancel) skips the ownership check.
	var filePath string
	if it, ok := s.cat.Get(r.PathValue("id")); ok {
		filePath = it.FilePath
	}
	out, _, err := s.reg.CallOne(contracts.CapPlaybackTranscodeV3, contracts.TranscodeV3Request{
		Action: contracts.TranscodeCancelAction, Session: session, FilePath: filePath, UserID: v.UserID,
	})
	if err != nil {
		writeTranscodeError(w, err)
		return
	}
	status, _ := out.(contracts.TranscodeV3Status)
	s.logger().Info("transcode cancelled", "req", reqIDOf(r), "session", session)
	writeJSON(w, http.StatusOK, publicStatusV3(status))
}

// resolveV3 looks a session up for the media handlers. Paths stay
// server-side; only the trusted status is returned.
func (s *Server) resolveV3(filePath, session string) (contracts.TranscodeV3Status, error) {
	out, _, err := s.reg.CallOne(contracts.CapPlaybackTranscodeV3, contracts.TranscodeV3Request{
		Action: contracts.TranscodeResolveAction, FilePath: filePath, Session: session,
	})
	if err != nil {
		return contracts.TranscodeV3Status{}, err
	}
	status, ok := out.(contracts.TranscodeV3Status)
	if !ok {
		return contracts.TranscodeV3Status{}, errors.New("bad transcode status")
	}
	return status, nil
}

// handleTranscode serves progressive output. With a session it resolves
// the v3 artifact; without one it keeps the synchronous v1
// compatibility path. HLS sessions are served through /transcode/hls.
func (s *Server) handleTranscode(w http.ResponseWriter, r *http.Request) {
	v, ok := s.userOf(r)
	if !ok {
		writeErr(w, http.StatusUnauthorized, "unauthorized")
		return
	}
	it, ok := s.cat.Get(r.PathValue("id"))
	if !ok {
		writeErr(w, http.StatusNotFound, "unknown item")
		return
	}

	if session := r.URL.Query().Get("session"); session != "" {
		status, err := s.resolveV3(it.FilePath, session)
		if err != nil {
			writeTranscodeError(w, err)
			return
		}
		if status.Delivery == contracts.TranscodeDeliveryHLS {
			writeJSON(w, http.StatusConflict, map[string]string{
				"error": "session uses HLS delivery", "code": "invalid-message",
			})
			return
		}
		if status.State != contracts.TranscodeReady || status.Path == "" {
			w.Header().Set("Retry-After", "1")
			writeJSON(w, http.StatusAccepted, publicStatusV3(status))
			return
		}
		s.serveFile(w, r, status.Path, it.ID+".mp4", "video/mp4")
		return
	}

	// No session is the retained synchronous v1 compatibility path. It
	// carries no per-session options, so the account's playback policy is
	// enforced here: a restricted account must not get a full-rate
	// transcode/remux that ignores its limits.
	policy := s.auth.PlaybackPolicy(v.UserID)
	if !policy.AllowsVideoTranscode() && !policy.AllowsRemux() {
		writeErr(w, http.StatusForbidden, "transcoding is disabled for this user")
		return
	}
	if policy.MaxBitrateKbps > 0 {
		// The v1 path cannot cap the output bitrate; a capped account must
		// use the session API, which honors the limit.
		writeErr(w, http.StatusForbidden, "this account must use the session API for transcoding")
		return
	}
	out, _, err := s.reg.CallOne(contracts.CapPlaybackTranscode, contracts.TranscodeRequest{FilePath: it.FilePath})
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
	s.serveFile(w, r, prepared.Path, it.ID+".mp4", "video/mp4")
}

// handleTranscodeHLS serves the playlist, init segment and media
// segments of an HLS session. File names are validated against the
// session directory; nothing else is reachable.
func (s *Server) handleTranscodeHLS(w http.ResponseWriter, r *http.Request) {
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
	name := r.PathValue("file")
	if !validHLSFile(name) {
		writeErr(w, http.StatusNotFound, "unknown hls file")
		return
	}
	status, err := s.resolveV3(it.FilePath, session)
	if err != nil {
		writeTranscodeError(w, err)
		return
	}
	if status.Delivery != contracts.TranscodeDeliveryHLS || status.PlaylistPath == "" {
		writeErr(w, http.StatusNotFound, "session has no HLS output")
		return
	}
	path := filepath.Join(filepath.Dir(status.PlaylistPath), name)
	if name == "index.m3u8" {
		s.serveHLSPlaylist(w, r, path, session)
		return
	}
	if _, err := os.Stat(path); err != nil {
		writeErr(w, http.StatusNotFound, "unknown hls file")
		return
	}
	if index, ok := segmentIndex(name); ok {
		// Fetching a segment that exists is the client's progress signal:
		// it drives throttling, segment deletion and idle cleanup. A
		// request for a missing file must not move the position, or a
		// bogus index would strand ffmpeg paused forever.
		if _, _, err := s.reg.CallOne(contracts.CapPlaybackTranscodeV3, contracts.TranscodeV3Request{
			Action: contracts.TranscodePositionAction, FilePath: it.FilePath, Session: session, SegmentIndex: index,
		}); err != nil {
			s.logger().Debug("transcode position update failed", "req", reqIDOf(r), "err", err.Error())
		}
	}
	contentType := "video/iso.segment"
	if name == "init.mp4" {
		contentType = "video/mp4"
	}
	w.Header().Set("Cache-Control", "private, max-age=3600")
	s.serveFile(w, r, path, name, contentType)
}

// serveHLSPlaylist rewrites the server-owned playlist so every media URI
// carries the session and the caller's token. A relative URI inside an
// m3u8 drops the playlist's query string, so <video>/hls.js would fetch
// init.mp4 and the segments without credentials and get 401 — HLS simply
// never played. Signing the URIs keeps the existing ?token= model and also
// works for native HLS (Safari), which cannot attach headers either.
func (s *Server) serveHLSPlaylist(w http.ResponseWriter, r *http.Request, path, session string) {
	raw, err := os.ReadFile(path)
	if err != nil {
		writeErr(w, http.StatusNotFound, "unknown hls file")
		return
	}
	body := rewriteHLSPlaylist(string(raw), session, requestToken(r))
	w.Header().Set("Content-Type", "application/vnd.apple.mpegurl")
	w.Header().Set("Cache-Control", "no-store")
	w.WriteHeader(http.StatusOK)
	_, _ = w.Write([]byte(body))
}

// requestToken mirrors userOf: the Bearer header first, then ?token=.
func requestToken(r *http.Request) string {
	if h := r.Header.Get("Authorization"); strings.HasPrefix(h, "Bearer ") {
		return strings.TrimPrefix(h, "Bearer ")
	}
	return r.URL.Query().Get("token")
}

// rewriteHLSPlaylist signs the EXT-X-MAP URI and every segment line,
// leaving tags and blank lines untouched.
func rewriteHLSPlaylist(raw, session, token string) string {
	lines := strings.Split(raw, "\n")
	var b strings.Builder
	b.Grow(len(raw) + 64*len(lines))
	for i, line := range lines {
		if i > 0 {
			b.WriteByte('\n')
		}
		trimmed := strings.TrimSpace(line)
		switch {
		case trimmed == "":
			b.WriteString(line)
		case strings.HasPrefix(trimmed, "#EXT-X-MAP:"):
			b.WriteString(signMapURI(line, session, token))
		case strings.HasPrefix(trimmed, "#"):
			b.WriteString(line)
		default:
			b.WriteString(signHLSURI(trimmed, session, token))
		}
	}
	return b.String()
}

// signMapURI rewrites the URI="..." attribute of an EXT-X-MAP line.
func signMapURI(line, session, token string) string {
	const attr = `URI="`
	start := strings.Index(line, attr)
	if start < 0 {
		return line
	}
	begin := start + len(attr)
	end := strings.Index(line[begin:], `"`)
	if end < 0 {
		return line
	}
	signed := signHLSURI(line[begin:begin+end], session, token)
	return line[:begin] + signed + line[begin+end:]
}

// signHLSURI appends the session (required by the handler) and the token
// (required by auth) to one media URI.
func signHLSURI(uri, session, token string) string {
	if uri == "" {
		return uri
	}
	u, err := url.Parse(uri)
	if err != nil {
		return uri
	}
	// Never sign a URI the server does not own: an absolute or
	// protocol-relative URI would leak the caller's token to a third
	// party. Only same-directory relative names are signed.
	if u.IsAbs() || u.Host != "" || strings.HasPrefix(uri, "//") {
		return uri
	}
	q := u.Query()
	q.Set("session", session)
	if token != "" {
		q.Set("token", token)
	}
	u.RawQuery = q.Encode()
	return u.String()
}

func validHLSFile(name string) bool {
	switch name {
	case "index.m3u8", "init.mp4":
		return true
	}
	_, ok := segmentIndex(name)
	return ok
}

// segmentIndex extracts N from segNNNNN.m4s (fMP4) or segNNNNN.ts
// (mpegts).
func segmentIndex(name string) (int, bool) {
	if !strings.HasPrefix(name, "seg") {
		return 0, false
	}
	for _, ext := range []string{".m4s", ".ts"} {
		if !strings.HasSuffix(name, ext) {
			continue
		}
		digits := strings.TrimSuffix(strings.TrimPrefix(name, "seg"), ext)
		if digits == "" {
			continue
		}
		if n, err := strconv.Atoi(digits); err == nil && n >= 0 {
			return n, true
		}
	}
	return 0, false
}

// handleSubtitles serves WebVTT for one item. Two shapes:
//
//   - ?session= resolves the sidecar a transcode session produced;
//   - ?stream=N extracts a subtitle track on demand (Jellyfin's "allow
//     subtitle extraction on the fly"), so direct play gets subtitles
//     without a transcode.
//
// Auth accepts ?token= like the other media endpoints.
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
	if stream := r.URL.Query().Get("stream"); stream != "" {
		s.streamSubtitle(w, r, it, stream)
		return
	}
	session := r.URL.Query().Get("session")
	if session == "" {
		writeJSON(w, http.StatusBadRequest, map[string]string{"error": "session or stream required", "code": "invalid-message"})
		return
	}
	status, err := s.resolveV3(it.FilePath, session)
	if err != nil {
		writeTranscodeError(w, err)
		return
	}
	if status.State != contracts.TranscodeReady || status.SubtitlePath == "" {
		writeErr(w, http.StatusNotFound, "subtitles unavailable")
		return
	}
	s.serveFile(w, r, status.SubtitlePath, it.ID+".vtt", "text/vtt; charset=utf-8")
}

// streamSubtitle extracts (or serves from cache) one subtitle track.
func (s *Server) streamSubtitle(w http.ResponseWriter, r *http.Request, it contracts.CatalogItem, raw string) {
	index, err := strconv.Atoi(raw)
	if err != nil || index < 0 {
		writeJSON(w, http.StatusBadRequest, map[string]string{"error": "stream must be a non-negative index", "code": "invalid-message"})
		return
	}
	out, _, err := s.reg.CallOne(contracts.CapPlaybackTranscodeV3, contracts.TranscodeV3Request{
		Action:         contracts.TranscodeSubtitleAction,
		FilePath:       it.FilePath,
		SubtitleStream: &index,
		Settings:       s.settings.Transcode(),
	})
	if err != nil {
		s.logger().Warn("subtitle extraction failed", "req", reqIDOf(r), "item", it.ID, "stream", index, "err", err.Error())
		writeTranscodeError(w, err)
		return
	}
	status, ok := out.(contracts.TranscodeV3Status)
	if !ok || status.SubtitlePath == "" {
		writeErr(w, http.StatusInternalServerError, "bad subtitle result")
		return
	}
	s.serveFile(w, r, status.SubtitlePath, it.ID+".vtt", "text/vtt; charset=utf-8")
}

// serveFile streams one server-side artifact with Range support.
func (s *Server) serveFile(w http.ResponseWriter, r *http.Request, path, name, contentType string) {
	f, err := os.Open(path)
	if err != nil {
		writeErr(w, http.StatusNotFound, "file unavailable")
		return
	}
	defer f.Close()
	fi, err := f.Stat()
	if err != nil {
		writeErr(w, http.StatusInternalServerError, err.Error())
		return
	}
	w.Header().Set("Content-Type", contentType)
	w.Header().Set("Accept-Ranges", "bytes")
	http.ServeContent(w, r, name, fi.ModTime(), f)
}
