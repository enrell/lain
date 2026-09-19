// V3 session API: inspect/start/status/resolve plus the v3-only
// cancel/position/list actions. Work admission, cache identity and
// status shapes live here; execution lives in coordinator/encode.
package transcode

import (
	"os"
	"path/filepath"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

// capabilitiesFor returns probed capabilities through the test seam
// when one is installed.
func (t *Transcoder) capabilitiesFor(settings contracts.TranscodeSettings) capabilities {
	if t.capsFn != nil {
		return t.capsFn(settings)
	}
	return t.probeCapabilities(settings)
}

func (t *Transcoder) probeReport(settings contracts.TranscodeSettings, path string) (mediaReport, error) {
	if t.probeFn != nil {
		return t.probeFn(ffprobeBinary(settings), path)
	}
	return probeMediaWith(ffprobeBinary(settings), path)
}

// specV3 derives the session identity for one v3 request.
func (t *Transcoder) specV3(path string, in contracts.TranscodeV3Request) (sourceSpec, contracts.TranscodeSettings, error) {
	fi, err := os.Stat(path)
	if err != nil || !fi.Mode().IsRegular() {
		return sourceSpec{}, contracts.TranscodeSettings{}, unavailable("source file unavailable")
	}
	settings := in.Settings.Normalize()
	if err := settings.Validate(); err != nil {
		return sourceSpec{}, settings, invalid(err.Error())
	}
	// A configured temp path must exist and be writable before work is
	// admitted, not halfway through an encode.
	if err := ensureRelocatedRoot(settings); err != nil {
		return sourceSpec{}, settings, unavailable(err.Error())
	}
	if in.StartSec < 0 {
		return sourceSpec{}, settings, invalid("start_sec must not be negative")
	}
	s := sourceSpec{
		Path: path, Size: fi.Size(), ModTimeNS: fi.ModTime().UnixNano(),
		Profile:     contracts.TranscodeProfileKey(settings, in),
		AudioStream: cloneInt(in.AudioStream), SubtitleStream: cloneInt(in.SubtitleStream),
		StartSec: in.StartSec,
		Settings: settings,
	}
	s.Session = sessionKey(s)
	return s, settings, nil
}

// invokeV3 routes one v3 request. Session-scoped actions resolve by
// session id alone: the gateway already authorized the item and the id
// is opaque and unguessable.
func (t *Transcoder) invokeV3(in contracts.TranscodeV3Request) (any, error) {
	switch in.Action {
	case contracts.TranscodeListAction:
		return t.listV3(), nil
	case contracts.TranscodeSubtitleAction:
		return t.subtitlesV3(in)
	case contracts.TranscodeCancelAction:
		if in.Session == "" {
			return contracts.TranscodeV3Status{}, invalid("session required")
		}
		return t.cancelV3(in.Session, in.FilePath, in.UserID), nil
	case contracts.TranscodePositionAction:
		if in.Session == "" {
			return contracts.TranscodeV3Status{}, invalid("session required")
		}
		return t.positionV3(in.Session, in.FilePath, in.SegmentIndex)
	case contracts.TranscodeStatusAction, contracts.TranscodeResolveAction:
		if in.Session == "" {
			return contracts.TranscodeV3Status{}, invalid("session required")
		}
		return t.bySessionV3(in.FilePath, in.Session, in.Action == contracts.TranscodeResolveAction)
	}
	if in.FilePath == "" {
		return contracts.TranscodeV3Status{}, invalid("file_path required")
	}
	spec, settings, err := t.specV3(in.FilePath, in)
	if err != nil {
		return contracts.TranscodeV3Status{}, err
	}
	if in.Session != "" && in.Session != spec.Session {
		return contracts.TranscodeV3Status{}, invalid("session does not match source, profile and options")
	}
	switch in.Action {
	case contracts.TranscodeInspectAction:
		return t.inspectV3(spec, settings), nil
	case contracts.TranscodeStartAction:
		return t.startV3(spec, in, settings)
	default:
		return contracts.TranscodeV3Status{}, invalid("action must be inspect, start, status, resolve, position, cancel or list")
	}
}

// inspectV3 is side-effect free: ready cache, then live job, else idle.
func (t *Transcoder) inspectV3(spec sourceSpec, settings contracts.TranscodeSettings) contracts.TranscodeV3Status {
	if entry, ok := t.readyEntry(spec); ok {
		return t.statusV3FromEntry(entry)
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if j, ok := t.jobs[spec.Session]; ok {
		return t.statusV3FromJob(j)
	}
	return contracts.TranscodeV3Status{
		Session: spec.Session, State: contracts.TranscodeIdle,
		Profile: spec.Profile, Delivery: settings.DefaultDelivery,
	}
}

// bySessionV3 resolves a session by id, verifying the source path when
// one is supplied. touch marks recency for the LRU cache.
func (t *Transcoder) bySessionV3(path, session string, touch bool) (contracts.TranscodeV3Status, error) {
	t.mu.Lock()
	j, ok := t.jobs[session]
	if ok {
		if path != "" && j.spec.Path != path {
			t.mu.Unlock()
			return contracts.TranscodeV3Status{}, invalid("session does not match source")
		}
		// Only a media fetch (resolve) counts as "someone is watching"; a
		// status poll must not keep an abandoned session alive, or the idle
		// timeout could never fire while anything polls the status.
		if touch {
			j.lastTouch = t.now().Unix()
		}
		status := t.statusV3FromJob(j)
		t.mu.Unlock()
		return status, nil
	}
	t.mu.Unlock()
	if !validSessionID(session) {
		return contracts.TranscodeV3Status{}, &core.Error{Code: "not-found", Msg: "unknown transcode session"}
	}
	var entry cacheEntry
	if err := readJSON(t.metaPath(session), &entry); err != nil {
		return contracts.TranscodeV3Status{}, &core.Error{Code: "not-found", Msg: "unknown transcode session"}
	}
	if path != "" && entry.SourcePath != path {
		return contracts.TranscodeV3Status{}, invalid("session does not match source")
	}
	if touch {
		_ = t.touch(entry)
	}
	return t.statusV3FromEntry(entry), nil
}

// positionV3 records the segment the client last fetched, driving
// throttling, segment deletion and idle cleanup. The latest report wins,
// not the highest: after a rewind the client is genuinely behind, so
// ffmpeg must pause again and the deleter must stop removing segments
// the client is about to re-fetch. An out-of-order report can only make
// the position older, which errs toward keeping more segments and
// pausing sooner — never toward deleting or racing ahead of the viewer.
func (t *Transcoder) positionV3(session, path string, segmentIndex int) (contracts.TranscodeV3Status, error) {
	t.mu.Lock()
	j, ok := t.jobs[session]
	if !ok {
		t.mu.Unlock()
		return contracts.TranscodeV3Status{}, &core.Error{Code: "not-found", Msg: "unknown transcode session"}
	}
	if path != "" && j.spec.Path != path {
		t.mu.Unlock()
		return contracts.TranscodeV3Status{}, invalid("session does not match source")
	}
	if segmentIndex >= 0 {
		j.clientSegment = segmentIndex
	}
	j.lastTouch = t.now().Unix()
	status := t.statusV3FromJob(j)
	t.mu.Unlock()
	t.applyThrottle(j)
	t.deleteConsumedSegments(j)
	return status, nil
}

// cancelV3 stops a session and drops its artifacts. The job stays
// flagged as cancelled so a race with a finishing ffmpeg cannot write
// a cache entry for a session nobody asked for anymore.
func (t *Transcoder) cancelV3(session, path, userID string) contracts.TranscodeV3Status {
	t.mu.Lock()
	j, ok := t.jobs[session]
	if !ok {
		t.mu.Unlock()
		return t.cancelFinishedV3(session, path)
	}
	if path != "" && j.spec.Path != path {
		t.mu.Unlock()
		return contracts.TranscodeV3Status{Session: session, State: contracts.TranscodeIdle}
	}
	// Only the account that started a session (or an admin cancel, which
	// sends no user id) may stop it.
	if userID != "" && j.userID != "" && j.userID != userID {
		t.mu.Unlock()
		return contracts.TranscodeV3Status{Session: session, State: contracts.TranscodeIdle}
	}
	if !j.v3 {
		// A v3 cancel only governs v3 sessions. A legacy (v1/v2) job is
		// left to finish: nothing waits on a cancelled legacy job, so
		// flipping its state would strand a synchronous waiter.
		t.mu.Unlock()
		return contracts.TranscodeV3Status{Session: session, State: contracts.TranscodeIdle}
	}
	cmd := j.cmd
	profile := j.spec.Profile
	j.cancelled = true
	// A job that is still queued must not start after all: runV3 only
	// runs a job marked queued, so flipping the state stops it before
	// ffmpeg is spawned (killing cmd only helps a running job).
	j.state = contracts.TranscodeIdle
	paths := t.pathsFor(session, j.settings)
	delete(t.jobs, session)
	t.mu.Unlock()
	if cmd != nil && cmd.Process != nil {
		_ = cmd.Process.Kill()
	}
	_ = os.Remove(paths.media)
	_ = os.RemoveAll(paths.hlsDir)
	_ = os.Remove(paths.subtitle)
	_ = os.Remove(paths.burn)
	_ = os.Remove(t.metaPath(session))
	t.logger().Info("transcode cancelled", "session", session)
	return contracts.TranscodeV3Status{Session: session, State: contracts.TranscodeIdle, Profile: profile}
}

// cancelFinishedV3 drops a session that is only a cache entry: a client
// that leaves for good should not leave an artifact behind.
func (t *Transcoder) cancelFinishedV3(session, path string) contracts.TranscodeV3Status {
	if !validSessionID(session) {
		return contracts.TranscodeV3Status{Session: session, State: contracts.TranscodeIdle}
	}
	var e cacheEntry
	if err := readJSON(t.metaPath(session), &e); err != nil {
		return contracts.TranscodeV3Status{Session: session, State: contracts.TranscodeIdle}
	}
	if path != "" && e.SourcePath != path {
		return contracts.TranscodeV3Status{Session: session, State: contracts.TranscodeIdle}
	}
	t.removeArtifacts(e, t.pathsForEntry(e))
	t.logger().Info("transcode cancelled", "session", session)
	return contracts.TranscodeV3Status{Session: session, State: contracts.TranscodeIdle, Profile: e.Profile}
}

// listV3 reports every known session (active or freshly finished).
func (t *Transcoder) listV3() []contracts.TranscodeV3Status {
	t.mu.Lock()
	defer t.mu.Unlock()
	out := make([]contracts.TranscodeV3Status, 0, len(t.jobs))
	for _, j := range t.jobs {
		out = append(out, t.statusV3FromJob(j))
	}
	return out
}

// subtitlesV3 extracts one subtitle stream to a cached WebVTT sidecar
// without starting a transcode: the direct-play path (Jellyfin's "allow
// subtitle extraction on the fly").
func (t *Transcoder) subtitlesV3(in contracts.TranscodeV3Request) (contracts.TranscodeV3Status, error) {
	if in.FilePath == "" {
		return contracts.TranscodeV3Status{}, invalid("file_path required")
	}
	if in.SubtitleStream == nil {
		return contracts.TranscodeV3Status{}, invalid("subtitle_stream required")
	}
	settings := in.Settings.Normalize()
	if err := settings.Validate(); err != nil {
		return contracts.TranscodeV3Status{}, invalid(err.Error())
	}
	if !settings.AllowsSubtitleExtraction() {
		return contracts.TranscodeV3Status{}, &core.Error{
			Code: "forbidden",
			Msg:  "on-the-fly subtitle extraction is disabled by server settings",
		}
	}
	if err := ensureRelocatedRoot(settings); err != nil {
		return contracts.TranscodeV3Status{}, unavailable(err.Error())
	}
	// The sidecar is its own cache entry: same identity rules as a
	// session, keyed by the source and the selected stream.
	spec, _, err := t.specV3(in.FilePath, in)
	if err != nil {
		return contracts.TranscodeV3Status{}, err
	}
	spec.Profile = subtitleProfile
	spec.SubtitleStream = cloneInt(in.SubtitleStream)
	spec.Session = sessionKey(spec)
	if entry, ok := t.readyEntry(spec); ok {
		_ = t.touch(entry)
		return t.statusV3FromEntry(entry), nil
	}
	report, err := t.probeReport(settings, spec.Path)
	if err != nil {
		return contracts.TranscodeV3Status{}, unavailable("ffprobe could not read the source")
	}
	var selected *stream
	for i := range report.Streams {
		s := &report.Streams[i]
		if s.Index == *in.SubtitleStream && s.CodecType == "subtitle" {
			selected = s
			break
		}
	}
	if selected == nil {
		return contracts.TranscodeV3Status{}, invalid("subtitle_stream is not a subtitle track")
	}
	if !textSubtitleCodec(selected.CodecName) {
		return contracts.TranscodeV3Status{}, &core.Error{
			Code: "unsupported-media",
			Msg:  "selected subtitle cannot be converted to WebVTT",
		}
	}
	paths := t.pathsFor(spec.Session, settings)
	if err := os.MkdirAll(filepath.Dir(paths.subtitle), 0o700); err != nil {
		return contracts.TranscodeV3Status{}, unavailable("transcode cache unavailable")
	}
	if err := t.extractSubtitleAs(spec, *selected, paths.subtitle, "webvtt"); err != nil {
		t.logger().Warn("subtitle extraction failed", "session", spec.Session, "err", err.Error())
		return contracts.TranscodeV3Status{}, &core.Error{Code: "dependency-unavailable", Msg: "subtitle extraction failed"}
	}
	fi, err := os.Stat(paths.subtitle)
	if err != nil || fi.Size() == 0 {
		return contracts.TranscodeV3Status{}, unavailable("no WebVTT produced")
	}
	now := t.now().Unix()
	entry := cacheEntry{
		Session: spec.Session, SourcePath: spec.Path, SourceSize: spec.Size,
		SourceModTime: spec.ModTimeNS, Profile: spec.Profile, Delivery: contracts.TranscodeDeliverySubtitle,
		ArtifactRoot: relocateRoot(settings), SubtitleStream: cloneInt(in.SubtitleStream),
		HasSubtitle: true, Method: "extract", Size: fi.Size(), CreatedAt: now, AccessedAt: now,
	}
	if err := writeJSONAtomic(t.metaPath(spec.Session), entry); err != nil {
		return contracts.TranscodeV3Status{}, unavailable("transcode cache unavailable")
	}
	_ = t.cleanup(false, t.activeSessionsWith(spec.Session))
	t.logger().Info("subtitle extracted", "session", spec.Session, "codec", selected.CodecName)
	return t.statusV3FromEntry(entry), nil
}

// activeStreamsFor counts the running or queued sessions of one opaque
// user id, excluding one session (the caller's own, when joining it).
func (t *Transcoder) activeStreamsFor(userID, except string) int {
	if userID == "" {
		return 0
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	return t.activeStreamsForLocked(userID, except)
}

// activeStreamsForLocked is the same count for callers that already hold
// the transcoder mutex (Go mutexes are not reentrant).
func (t *Transcoder) activeStreamsForLocked(userID, except string) int {
	if userID == "" {
		return 0
	}
	n := 0
	for session, j := range t.jobs {
		if session == except || j.userID != userID {
			continue
		}
		if j.state == contracts.TranscodeQueued || j.state == contracts.TranscodeRunning {
			n++
		}
	}
	return n
}

// policyAllowsMethod reports whether a playback policy permits serving an
// already-produced derivative of the given method. A nil policy is
// unrestricted; a subtitle sidecar is never a video transcode.
func policyAllowsMethod(p *contracts.TranscodePolicy, method string) error {
	if p == nil || method == "extract" {
		return nil
	}
	if method == "remux" {
		if !p.AllowRemux {
			return &core.Error{Code: "forbidden", Msg: "remuxing is disabled for this user"}
		}
		return nil
	}
	if !p.AllowVideoTranscode {
		return &core.Error{Code: "forbidden", Msg: "video transcoding is disabled for this user"}
	}
	return nil
}

func (t *Transcoder) startV3(spec sourceSpec, in contracts.TranscodeV3Request, settings contracts.TranscodeSettings) (contracts.TranscodeV3Status, error) {
	if entry, ok := t.readyEntry(spec); ok {
		// A cached derivative must satisfy the caller's policy too: the
		// cache is keyed by source and options, not by account, so a
		// restricted user could otherwise be served another account's
		// remux/transcode.
		if err := policyAllowsMethod(in.Policy, entry.Method); err != nil {
			return contracts.TranscodeV3Status{}, err
		}
		_ = t.touch(entry)
		return t.statusV3FromEntry(entry), nil
	}
	if err := t.cleanup(false, t.activeSessions()); err != nil {
		return contracts.TranscodeV3Status{}, unavailable("transcode cache unavailable")
	}
	// A relocated root is lain's own namespace: drop directories no
	// sidecar claims (a crash between mkdir and record). Active sessions
	// (including this one) are excluded so a live stream is never wiped.
	t.sweepRelocatedOrphans(relocateRoot(settings), t.activeSessionsWith(spec.Session))
	plan, err := t.planV3(spec, in, t.capabilitiesFor(settings))
	if err != nil {
		return contracts.TranscodeV3Status{}, err
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return contracts.TranscodeV3Status{}, unavailable("transcoder stopped")
	}
	if old, ok := t.jobs[spec.Session]; ok {
		if old.state != contracts.TranscodeFailed {
			// Joining a live session must satisfy the caller's policy as
			// well (session ids are shared across accounts).
			if err := policyAllowsMethod(in.Policy, old.method); err != nil {
				return contracts.TranscodeV3Status{}, err
			}
			return t.statusV3FromJob(old), nil
		}
		delete(t.jobs, spec.Session) // an explicit POST retries a failed job
	}
	// Adopt the operator bounds carried by this request so cache, queue
	// and concurrency changes apply without a restart.
	if settings.CacheBytes > 0 {
		t.config.MaxCacheBytes = settings.CacheBytes
	}
	if settings.MaxConcurrent > 0 {
		t.maxConcurrent = settings.MaxConcurrent
	}
	if settings.QueueSize > 0 {
		limit := settings.QueueSize
		if limit > maxQueueCapacity {
			limit = maxQueueCapacity
		}
		t.queueLimit = limit
	}
	if t.pending >= t.queueLimit {
		return contracts.TranscodeV3Status{}, &core.Error{Code: "queue-full", Msg: "transcode queue is full"}
	}
	// A per-user simultaneous-stream limit is checked here so joining an
	// already-running session of the same user never counts twice.
	if in.Policy != nil && in.Policy.MaxStreams > 0 {
		if t.activeStreamsForLocked(in.UserID, spec.Session) >= in.Policy.MaxStreams {
			return contracts.TranscodeV3Status{}, &core.Error{
				Code: "too-many-streams",
				Msg:  "simultaneous stream limit reached for this account",
			}
		}
	}
	now := t.now().Unix()
	j := &job{
		spec: spec, state: contracts.TranscodeQueued, queuedAt: now, done: make(chan struct{}),
		v3: true, delivery: plan.delivery, settings: settings, plan: &plan,
		userID:    in.UserID,
		lastTouch: now, encoder: plan.encoder, hwBackend: plan.hardwareLabel(),
		fallback: plan.fallback, reasons: plan.reasons,
		videoCodec: plan.videoCodec, audioCodec: plan.audioCodec,
		width: plan.width, height: plan.height, bitrateKbps: plan.maxBitrateKbps,
		method: plan.method,
	}
	t.jobs[spec.Session] = j
	t.pending++
	t.queue <- j
	return t.statusV3FromJob(j), nil
}

// statusV3FromJob renders a live session; PlaylistPath is trusted-plane.
func (t *Transcoder) statusV3FromJob(j *job) contracts.TranscodeV3Status {
	status := contracts.TranscodeV3Status{
		Session: j.spec.Session, State: j.state, Profile: j.spec.Profile, Delivery: j.delivery,
		Method: j.method, Encoder: j.encoder, Hardware: j.hwBackend, Fallback: j.fallback,
		Reasons: j.reasons, VideoCodec: j.videoCodec, AudioCodec: j.audioCodec,
		Width: j.width, Height: j.height, BitrateKbps: j.bitrateKbps,
		VideoDirect: j.plan != nil && j.plan.copyVideo,
		AudioDirect: j.plan != nil && j.plan.copyAudio,
		ErrorCode:   j.errCode, Error: j.errMessage, UserID: j.userID,
		QueuedAt: j.queuedAt, StartedAt: j.startedAt, FinishedAt: j.finishedAt,
	}
	if j.state == contracts.TranscodeQueued || j.state == contracts.TranscodeRunning {
		status.Progress = j.progress
		status.FPS = j.fps
		status.OutputBitrateKbps = j.outputKbps
	}
	if j.hasSubtitle {
		status.SubtitlePath = t.subtitlePath(j.spec.Session)
	}
	switch j.delivery {
	case contracts.TranscodeDeliveryHLS:
		if j.state == contracts.TranscodeRunning || j.state == contracts.TranscodeReady {
			status.PlaylistPath = t.pathsFor(j.spec.Session, j.settings).playlist
			status.Playable = j.playable || j.state == contracts.TranscodeReady
		}
	default:
		if j.state == contracts.TranscodeReady {
			status.Path = t.pathsFor(j.spec.Session, j.settings).media
			status.Playable = true
		}
	}
	return status
}

func (t *Transcoder) statusV3FromEntry(e cacheEntry) contracts.TranscodeV3Status {
	paths := t.pathsForEntry(e)
	status := contracts.TranscodeV3Status{
		Session: e.Session, State: contracts.TranscodeReady, Profile: e.Profile,
		Delivery: e.Delivery, Method: e.Method, Cached: true, Playable: true,
		Encoder: e.Encoder, Hardware: e.Hardware, Fallback: e.Fallback,
		Reasons: e.Reasons, VideoCodec: e.VideoCodec, AudioCodec: e.AudioCodec,
		Width: e.Width, Height: e.Height, BitrateKbps: e.BitrateKbps,
		VideoDirect: e.VideoDirect, AudioDirect: e.AudioDirect,
		FinishedAt: e.CreatedAt,
	}
	if e.HasSubtitle {
		status.SubtitlePath = paths.subtitle
	}
	switch e.Delivery {
	case contracts.TranscodeDeliverySubtitle:
		// Sidecar-only: no media path and nothing to play.
		status.Playable = false
		status.Method = "extract"
	case contracts.TranscodeDeliveryHLS:
		status.PlaylistPath = paths.playlist
	default:
		status.Path = paths.media
	}
	return status
}

func (t *Transcoder) playlistPath(session string) string {
	return t.pathsFor(session, contracts.TranscodeSettings{}).playlist
}

// CapabilitiesReport is the public, path-free probe summary for the
// admin settings UI (D-031: probing is visible, never silent).
type CapabilitiesReport struct {
	FFmpeg            string          `json:"ffmpeg"`
	Encoders          []string        `json:"encoders"`
	ToneMapping       bool            `json:"tone_mapping"`
	ToneMappingBT2390 bool            `json:"tone_mapping_bt2390"`
	Hardware          map[string]bool `json:"hardware"`
}

// Probe runs (or reuses) the capability probe for one settings shape.
func (t *Transcoder) Probe(settings contracts.TranscodeSettings) CapabilitiesReport {
	return reportFromCaps(t.capabilitiesFor(settings.Normalize()))
}

// reportFromCaps projects the probed surface into the public report.
func reportFromCaps(caps capabilities) CapabilitiesReport {
	report := CapabilitiesReport{
		FFmpeg:            caps.FFmpeg,
		ToneMapping:       caps.ToneMap,
		ToneMappingBT2390: caps.ToneMap2390,
		Hardware:          map[string]bool{},
	}
	for _, name := range []string{
		"libx264", "libx265", "libsvtav1", "libaom-av1",
		"h264_vaapi", "hevc_vaapi", "av1_vaapi",
		"h264_nvenc", "hevc_nvenc", "av1_nvenc",
		"h264_qsv", "hevc_qsv", "av1_qsv",
		"h264_amf", "hevc_amf", "av1_amf",
		"h264_v4l2m2m", "hevc_v4l2m2m",
		"h264_videotoolbox", "hevc_videotoolbox",
	} {
		if caps.hasEncoder(name) {
			report.Encoders = append(report.Encoders, name)
		}
	}
	for backend, ok := range caps.Hardware {
		report.Hardware[backend] = ok
	}
	return report
}
