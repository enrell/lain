package transcode

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

const (
	// DefaultMaxCacheBytes keeps several ordinary 1080p preparations
	// without letting a derivative cache consume a data volume forever.
	DefaultMaxCacheBytes int64 = 20 << 30
	defaultQueueSize           = 8
	defaultMaxConcurrent       = 2
	// maxQueueCapacity is the channel's fixed capacity (the top of the
	// queue_size range, contracts.Validate). Admission is bounded by the
	// runtime queueLimit instead, so the channel never blocks a sender.
	maxQueueCapacity = 64
	// maxTrackedJobs bounds the in-memory session map; ready artifacts
	// stay reachable through their on-disk cache entry.
	maxTrackedJobs = 128
)

// Config controls bounded derivative storage and work admission.
type Config struct {
	MaxCacheBytes int64
	QueueSize     int
	MaxConcurrent int
}

type convertFunc func(spec sourceSpec, out string, onSample func(progressSample)) (string, error)

type sourceSpec struct {
	Path           string
	Size           int64
	ModTimeNS      int64
	Profile        string
	AudioStream    *int
	SubtitleStream *int
	// StartSec is the source position the session begins at; it is part of
	// the session identity because it changes the produced bytes.
	StartSec float64
	Session  string
	// Settings carries the operator policy for v3 sessions (binary
	// paths, subtitle extraction); v1/v2 leave it zero.
	Settings contracts.TranscodeSettings
}

type cacheEntry struct {
	Session        string   `json:"session"`
	SourcePath     string   `json:"source_path"`
	SourceSize     int64    `json:"source_size"`
	SourceModTime  int64    `json:"source_mod_time_ns"`
	Profile        string   `json:"profile"`
	Delivery       string   `json:"delivery,omitempty"`
	ArtifactRoot   string   `json:"artifact_root,omitempty"`
	AudioStream    *int     `json:"audio_stream,omitempty"`
	SubtitleStream *int     `json:"subtitle_stream,omitempty"`
	HasSubtitle    bool     `json:"has_subtitle,omitempty"`
	Method         string   `json:"method"`
	Size           int64    `json:"size"`
	CreatedAt      int64    `json:"created_at"`
	AccessedAt     int64    `json:"accessed_at"`
	Encoder        string   `json:"encoder,omitempty"`
	Hardware       string   `json:"hardware,omitempty"`
	Fallback       string   `json:"fallback,omitempty"`
	Reasons        []string `json:"reasons,omitempty"`
	VideoCodec     string   `json:"video_codec,omitempty"`
	AudioCodec     string   `json:"audio_codec,omitempty"`
	Width          int      `json:"width,omitempty"`
	Height         int      `json:"height,omitempty"`
	BitrateKbps    int      `json:"bitrate_kbps,omitempty"`
	VideoDirect    bool     `json:"video_direct,omitempty"`
	AudioDirect    bool     `json:"audio_direct,omitempty"`
}

// artifactPaths resolves one session's artifact locations. Without a
// configured temp path the historical flat layout under the data dir is
// kept byte for byte; with one, a session owns a directory under
// <temp>/lain-transcode/ while its JSON sidecar stays with the data dir
// (so the LRU index and cleanup root never move).
type artifactPaths struct {
	media    string
	hlsDir   string
	playlist string
	subtitle string
	burn     string
}

// artifactDirFor is the directory a relocated session owns under an owned
// root — the result of relocateRoot, which already carries the
// lain-transcode namespace. Appending it here too would double it.
func artifactDirFor(root, session string) string {
	return filepath.Join(root, session)
}

func (t *Transcoder) pathsFor(session string, settings contracts.TranscodeSettings) artifactPaths {
	if root := relocateRoot(settings); root != "" {
		dir := artifactDirFor(root, session)
		return artifactPaths{
			media:    filepath.Join(dir, "media.mp4"),
			hlsDir:   dir,
			playlist: filepath.Join(dir, hlsIndexPlaylist),
			subtitle: filepath.Join(dir, "subs.vtt"),
			burn:     filepath.Join(dir, "burn.ass"),
		}
	}
	return t.defaultPaths(session)
}

func (t *Transcoder) defaultPaths(session string) artifactPaths {
	hls := filepath.Join(t.dir, session+".hls")
	return artifactPaths{
		media:    filepath.Join(t.dir, session+".mp4"),
		hlsDir:   hls,
		playlist: filepath.Join(hls, hlsIndexPlaylist),
		subtitle: filepath.Join(t.dir, session+".vtt"),
		burn:     filepath.Join(t.dir, session+".burn.ass"),
	}
}

// pathsForEntry resolves the layout a finished session was written to,
// which is recorded in its sidecar.
func (t *Transcoder) pathsForEntry(e cacheEntry) artifactPaths {
	if e.ArtifactRoot != "" {
		dir := artifactDirFor(e.ArtifactRoot, e.Session)
		return artifactPaths{
			media:    filepath.Join(dir, "media.mp4"),
			hlsDir:   dir,
			playlist: filepath.Join(dir, hlsIndexPlaylist),
			subtitle: filepath.Join(dir, "subs.vtt"),
			burn:     filepath.Join(dir, "burn.ass"),
		}
	}
	return t.defaultPaths(e.Session)
}

// relocateRoot is the owned subdirectory of a configured temp path.
func relocateRoot(settings contracts.TranscodeSettings) string {
	root := strings.TrimSpace(settings.TranscodeTempPath)
	if root == "" {
		return ""
	}
	return filepath.Join(root, "lain-transcode")
}

// ensureRelocatedRoot creates and verifies the operator's temp path so a
// misconfigured volume fails at admission instead of mid-encode.
func ensureRelocatedRoot(settings contracts.TranscodeSettings) error {
	root := relocateRoot(settings)
	if root == "" {
		return nil
	}
	if err := os.MkdirAll(root, 0o700); err != nil {
		return fmt.Errorf("transcode temp path: %w", err)
	}
	probe := filepath.Join(root, ".writable")
	if err := os.WriteFile(probe, nil, 0o600); err != nil {
		return fmt.Errorf("transcode temp path: %w", err)
	}
	_ = os.Remove(probe)
	return nil
}

type job struct {
	spec       sourceSpec
	state      string
	method     string
	progress   float64
	errCode    string
	errMessage string
	queuedAt   int64
	startedAt  int64
	finishedAt int64
	done       chan struct{}

	// v3 session state.
	v3        bool
	delivery  string
	userID    string
	settings  contracts.TranscodeSettings
	plan      *encodePlan
	cmd       *exec.Cmd
	paused    bool
	cancelled bool
	// stopped marks a session the operator or the idle timeout ended: a
	// stop is terminal, so a late finish must not resurrect it as Ready.
	stopped       bool
	fps           float64
	outputKbps    int
	clientSegment int
	lastTouch     int64
	playable      bool
	subtitleFile  string
	hasSubtitle   bool
	encoder       string
	hwBackend     string
	fallback      string
	reasons       []string
	videoCodec    string
	audioCodec    string
	width         int
	height        int
	bitrateKbps   int
}

// Transcoder serves the synchronous v1 contract, the asynchronous v2
// contract and the v3 session contract over one plugin-private cache.
type Transcoder struct {
	dir      string
	config   Config
	convert  convertFunc
	now      func() time.Time
	ctx      context.Context
	cancel   context.CancelFunc
	closeOne sync.Once
	log      atomic.Pointer[slog.Logger]

	// Test seams for v3: probe/capability injection and a full run hook.
	probeFn func(ffprobePath, path string) (mediaReport, error)
	capsFn  func(settings contracts.TranscodeSettings) capabilities
	v3run   func(j *job) error

	capMu  sync.Mutex
	caps   capabilities
	capKey string

	mu            sync.Mutex
	jobs          map[string]*job
	queue         chan *job
	closed        bool
	active        int
	pending       int
	queueLimit    int
	maxConcurrent int
	wg            sync.WaitGroup

	initErr error
}

// New prepares a transcoder with production defaults.
func New(dir string) *Transcoder {
	return NewConfigured(dir, Config{})
}

// NewConfigured prepares a transcoder with explicit cache/queue bounds.
func NewConfigured(dir string, cfg Config) *Transcoder {
	return newWithDeps(dir, cfg, nil, time.Now)
}

func newWithDeps(dir string, cfg Config, convert convertFunc, now func() time.Time) *Transcoder {
	if cfg.MaxCacheBytes <= 0 {
		cfg.MaxCacheBytes = DefaultMaxCacheBytes
	}
	if cfg.QueueSize <= 0 {
		cfg.QueueSize = defaultQueueSize
	}
	// The channel is fixed at maxQueueCapacity; admission may never
	// exceed it or a send would block the caller.
	if cfg.QueueSize > maxQueueCapacity {
		cfg.QueueSize = maxQueueCapacity
	}
	if cfg.MaxConcurrent <= 0 {
		cfg.MaxConcurrent = defaultMaxConcurrent
	}
	if now == nil {
		now = time.Now
	}
	ctx, cancel := context.WithCancel(context.Background())
	t := &Transcoder{
		dir: dir, config: cfg, convert: convert, now: now,
		ctx: ctx, cancel: cancel, jobs: map[string]*job{},
		queue:         make(chan *job, maxQueueCapacity),
		queueLimit:    cfg.QueueSize,
		maxConcurrent: cfg.MaxConcurrent,
	}
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.initErr = err
		return t
	}
	if err := t.cleanup(true, nil); err != nil {
		t.initErr = err
		return t
	}
	t.wg.Add(2)
	go t.worker()
	go t.ticker()
	return t
}

func (t *Transcoder) ID() string { return ID }

// SetLogger injects the server logger without process-global state.
func (t *Transcoder) SetLogger(logger *slog.Logger) {
	if logger == nil {
		logger = slog.New(slog.DiscardHandler)
	}
	t.log.Store(logger)
}

func (t *Transcoder) logger() *slog.Logger {
	if logger := t.log.Load(); logger != nil {
		return logger
	}
	return slog.New(slog.DiscardHandler)
}

func (t *Transcoder) Capabilities() []string {
	return []string{contracts.CapPlaybackTranscode, contracts.CapPlaybackTranscodeV2, contracts.CapPlaybackTranscodeV3}
}

// Health is cheap and side-effect free. ffprobe remains opportunistic:
// its absence selects the conservative full-encode path.
func (t *Transcoder) Health() error {
	if t.initErr != nil {
		return t.initErr
	}
	if _, err := exec.LookPath("ffmpeg"); err != nil && t.convert == nil {
		return fmt.Errorf("ffmpeg not found: %w", err)
	}
	if fi, err := os.Stat(t.dir); err != nil || !fi.IsDir() {
		return fmt.Errorf("transcode cache %s unusable", t.dir)
	}
	return nil
}

// Close cancels running conversions and rejects queued work.
func (t *Transcoder) Close() error {
	t.closeOne.Do(func() {
		t.mu.Lock()
		t.closed = true
		for _, j := range t.jobs {
			if j.state == contracts.TranscodeQueued {
				j.state = contracts.TranscodeFailed
				j.errCode = "dependency-unavailable"
				j.errMessage = "transcoder stopped"
				j.finishedAt = t.now().Unix()
				close(j.done)
			}
		}
		t.mu.Unlock()
		t.cancel()
		t.wg.Wait()
	})
	return nil
}

func (t *Transcoder) Invoke(cap string, input any) (any, error) {
	switch cap {
	case contracts.CapPlaybackTranscode:
		in, ok := input.(contracts.TranscodeRequest)
		if !ok {
			return nil, invalid("TranscodeRequest required")
		}
		if in.FilePath == "" {
			return nil, invalid("file_path required")
		}
		return t.prepareSync(in.FilePath)
	case contracts.CapPlaybackTranscodeV2:
		in, ok := input.(contracts.TranscodeV2Request)
		if !ok {
			return nil, invalid("TranscodeV2Request required")
		}
		return t.invokeV2(in)
	case contracts.CapPlaybackTranscodeV3:
		in, ok := input.(contracts.TranscodeV3Request)
		if !ok {
			return nil, invalid("TranscodeV3Request required")
		}
		return t.invokeV3(in)
	default:
		return nil, invalid("unsupported cap " + cap)
	}
}

func invalid(message string) error {
	return &core.Error{Code: "invalid-message", Msg: message}
}

func unavailable(message string) error {
	return &core.Error{Code: "dependency-unavailable", Msg: message}
}

func (t *Transcoder) invokeV2(in contracts.TranscodeV2Request) (contracts.TranscodeStatus, error) {
	if in.FilePath == "" {
		return contracts.TranscodeStatus{}, invalid("file_path required")
	}
	if (in.Action == contracts.TranscodeStatusAction || in.Action == contracts.TranscodeResolveAction) && in.Session != "" {
		return t.bySession(in.FilePath, in.Session, in.Action == contracts.TranscodeResolveAction)
	}
	spec, err := t.spec(in.FilePath, in.Profile, in.AudioStream, in.SubtitleStream)
	if err != nil {
		return contracts.TranscodeStatus{}, err
	}
	if in.Session != "" && in.Session != spec.Session {
		return contracts.TranscodeStatus{}, invalid("session does not match source and profile")
	}
	switch in.Action {
	case contracts.TranscodeInspectAction, contracts.TranscodeStatusAction:
		return t.inspect(spec), nil
	case contracts.TranscodeStartAction:
		return t.start(spec)
	default:
		return contracts.TranscodeStatus{}, invalid("action must be inspect, start, status or resolve")
	}
}

func (t *Transcoder) bySession(path, session string, touch bool) (contracts.TranscodeStatus, error) {
	fi, err := os.Stat(path)
	if err != nil || !fi.Mode().IsRegular() {
		return contracts.TranscodeStatus{}, unavailable("source file unavailable")
	}
	t.mu.Lock()
	if j, ok := t.jobs[session]; ok {
		if j.spec.Path != path || j.spec.Size != fi.Size() || j.spec.ModTimeNS != fi.ModTime().UnixNano() {
			t.mu.Unlock()
			return contracts.TranscodeStatus{}, invalid("session does not match source")
		}
		status := t.statusFromJob(j)
		t.mu.Unlock()
		if status.State != contracts.TranscodeReady {
			return status, nil
		}
	} else {
		t.mu.Unlock()
	}
	if !validSessionID(session) {
		return contracts.TranscodeStatus{}, &core.Error{Code: "not-found", Msg: "unknown transcode session"}
	}
	var entry cacheEntry
	if err := readJSON(t.metaPath(session), &entry); err != nil {
		return contracts.TranscodeStatus{}, &core.Error{Code: "not-found", Msg: "unknown transcode session"}
	}
	if entry.SourcePath != path || entry.SourceSize != fi.Size() || entry.SourceModTime != fi.ModTime().UnixNano() {
		return contracts.TranscodeStatus{}, invalid("session does not match source")
	}
	media, err := os.Stat(t.mediaPath(session))
	if err != nil || media.Size() <= 0 {
		return contracts.TranscodeStatus{}, &core.Error{Code: "not-found", Msg: "transcode artifact unavailable"}
	}
	if touch {
		_ = t.touch(entry)
	}
	return statusFromEntry(entry, t.mediaPath(session), true), nil
}

func (t *Transcoder) spec(path, requestedProfile string, audio, subtitle *int) (sourceSpec, error) {
	fi, err := os.Stat(path)
	if err != nil || !fi.Mode().IsRegular() {
		return sourceSpec{}, unavailable("source file unavailable")
	}
	p := requestedProfile
	if p == "" {
		p = profile
	}
	if p != profile && p != legacyProfile {
		return sourceSpec{}, invalid("unsupported transcode profile")
	}
	s := sourceSpec{Path: path, Size: fi.Size(), ModTimeNS: fi.ModTime().UnixNano(), Profile: p, AudioStream: cloneInt(audio), SubtitleStream: cloneInt(subtitle)}
	s.Session = sessionKey(s)
	return s, nil
}

func cloneInt(v *int) *int {
	if v == nil {
		return nil
	}
	n := *v
	return &n
}

func sessionKey(s sourceSpec) string {
	audio, subtitle := "default", "off"
	if s.AudioStream != nil {
		audio = fmt.Sprint(*s.AudioStream)
	}
	if s.SubtitleStream != nil {
		subtitle = fmt.Sprint(*s.SubtitleStream)
	}
	h := sha256.Sum256([]byte(fmt.Sprintf("%s|%d|%d|%.3f|%s|a:%s|s:%s", s.Path, s.ModTimeNS, s.Size, s.StartSec, s.Profile, audio, subtitle)))
	return hex.EncodeToString(h[:])
}

// validSessionID accepts exactly the 64-hex ids sessionKey produces.
// Session ids arrive from clients (cancel/status/resolve), so they are
// checked before ever being joined into a filesystem path.
func validSessionID(session string) bool {
	if len(session) != 64 {
		return false
	}
	for i := 0; i < len(session); i++ {
		c := session[i]
		if (c < '0' || c > '9') && (c < 'a' || c > 'f') {
			return false
		}
	}
	return true
}

func (t *Transcoder) inspect(spec sourceSpec) contracts.TranscodeStatus {
	if entry, ok := t.readyEntry(spec); ok {
		return statusFromEntry(entry, t.mediaPath(spec.Session), true)
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if j, ok := t.jobs[spec.Session]; ok {
		return t.statusFromJob(j)
	}
	return contracts.TranscodeStatus{Session: spec.Session, State: contracts.TranscodeIdle, Profile: spec.Profile}
}

func (t *Transcoder) start(spec sourceSpec) (contracts.TranscodeStatus, error) {
	if entry, ok := t.readyEntry(spec); ok {
		_ = t.touch(entry)
		return statusFromEntry(entry, t.mediaPath(spec.Session), true), nil
	}
	if err := t.cleanup(false, t.activeSessions()); err != nil {
		return contracts.TranscodeStatus{}, unavailable("transcode cache unavailable")
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.closed {
		return contracts.TranscodeStatus{}, unavailable("transcoder stopped")
	}
	if old, ok := t.jobs[spec.Session]; ok {
		if old.state != contracts.TranscodeFailed {
			return t.statusFromJob(old), nil
		}
		delete(t.jobs, spec.Session) // an explicit POST retries a failed job
	}
	if t.pending >= t.queueLimit {
		return contracts.TranscodeStatus{}, &core.Error{Code: "queue-full", Msg: "transcode queue is full"}
	}
	now := t.now().Unix()
	j := &job{spec: spec, state: contracts.TranscodeQueued, queuedAt: now, done: make(chan struct{})}
	t.jobs[spec.Session] = j
	t.pending++
	t.queue <- j
	return t.statusFromJob(j), nil
}

func (t *Transcoder) prepareSync(path string) (contracts.Transcode, error) {
	spec, err := t.spec(path, legacyProfile, nil, nil)
	if err != nil {
		return contracts.Transcode{}, err
	}
	status, err := t.start(spec)
	if err != nil {
		return contracts.Transcode{}, err
	}
	if status.State == contracts.TranscodeReady {
		return contracts.Transcode{Path: status.Path, Method: status.Method, Cached: true}, nil
	}
	t.mu.Lock()
	j := t.jobs[spec.Session]
	t.mu.Unlock()
	if j == nil {
		return contracts.Transcode{}, unavailable("transcode job unavailable")
	}
	select {
	case <-j.done:
	case <-t.ctx.Done():
		return contracts.Transcode{}, unavailable("transcoder stopped")
	}
	status = t.inspect(spec)
	if status.State != contracts.TranscodeReady {
		return contracts.Transcode{}, unavailable(status.Error)
	}
	return contracts.Transcode{Path: status.Path, Method: status.Method}, nil
}

// worker dispatches queued jobs into bounded concurrent runs.
func (t *Transcoder) worker() {
	defer t.wg.Done()
	for {
		select {
		case <-t.ctx.Done():
			return
		case j := <-t.queue:
			if j == nil {
				continue
			}
			t.wg.Add(1)
			go func(j *job) {
				defer t.wg.Done()
				if !t.acquireSlot() {
					t.finishPending(j)
					return
				}
				t.finishPending(j)
				defer t.releaseSlot()
				if j.v3 {
					t.runV3(j)
					return
				}
				t.run(j)
			}(j)
		}
	}
}

func (t *Transcoder) acquireSlot() bool {
	for {
		t.mu.Lock()
		if t.closed {
			t.mu.Unlock()
			return false
		}
		if t.active < t.maxConcurrent {
			t.active++
			t.mu.Unlock()
			return true
		}
		t.mu.Unlock()
		select {
		case <-t.ctx.Done():
			return false
		case <-time.After(50 * time.Millisecond):
		}
	}
}

func (t *Transcoder) releaseSlot() {
	t.mu.Lock()
	if t.active > 0 {
		t.active--
	}
	t.mu.Unlock()
}

// finishPending marks one admitted job as no longer waiting for a slot.
func (t *Transcoder) finishPending(j *job) {
	t.mu.Lock()
	if t.pending > 0 {
		t.pending--
	}
	t.mu.Unlock()
}

// ticker keeps HLS playlists fresh, applies throttling and stops
// abandoned sessions.
func (t *Transcoder) ticker() {
	defer t.wg.Done()
	tick := time.NewTicker(time.Second)
	defer tick.Stop()
	for {
		select {
		case <-t.ctx.Done():
			return
		case <-tick.C:
			t.tick()
		}
	}
}

func (t *Transcoder) tick() {
	now := t.now().Unix()
	t.pruneJobs()
	t.mu.Lock()
	var hls []*job
	for _, j := range t.jobs {
		if j.v3 && j.delivery == contracts.TranscodeDeliveryHLS {
			hls = append(hls, j)
		}
	}
	t.mu.Unlock()
	for _, j := range hls {
		t.mu.Lock()
		state, lastTouch := j.state, j.lastTouch
		idle := j.settings.IdleTimeoutSec
		t.mu.Unlock()
		switch state {
		case contracts.TranscodeRunning:
			dir := t.pathsFor(j.spec.Session, j.settings).hlsDir
			if err := writeIndexPlaylist(dir, j.settings.HLSSegmentSeconds); err != nil {
				continue
			}
			t.mu.Lock()
			j.playable = hlsPlayable(dir)
			t.mu.Unlock()
			t.applyThrottle(j)
			t.deleteConsumedSegments(j)
			if idle > 0 && lastTouch > 0 && now-lastTouch > int64(idle) {
				t.stopJob(j, "idle-timeout", "session stopped after inactivity")
			}
		case contracts.TranscodeReady:
			dir := t.pathsFor(j.spec.Session, j.settings).hlsDir
			_ = writeIndexPlaylist(dir, j.settings.HLSSegmentSeconds)
		}
	}
}

// pruneJobs bounds the in-memory session map: finished sessions are
// kept long enough for a client to read its result, then dropped (the
// cache entry on disk is what makes a later play instant).
func (t *Transcoder) pruneJobs() {
	t.mu.Lock()
	defer t.mu.Unlock()
	if len(t.jobs) <= maxTrackedJobs {
		return
	}
	type finished struct {
		session string
		at      int64
	}
	var done []finished
	for session, j := range t.jobs {
		switch j.state {
		case contracts.TranscodeReady, contracts.TranscodeFailed:
			done = append(done, finished{session: session, at: j.finishedAt})
		}
	}
	sort.Slice(done, func(i, j int) bool { return done[i].at < done[j].at })
	for _, entry := range done {
		if len(t.jobs) <= maxTrackedJobs {
			break
		}
		delete(t.jobs, entry.session)
	}
}

// stopJob kills a running session and marks it failed with the reason.
func (t *Transcoder) stopJob(j *job, code, message string) {
	t.mu.Lock()
	if j.state != contracts.TranscodeRunning && j.state != contracts.TranscodeQueued {
		t.mu.Unlock()
		return
	}
	cmd := j.cmd
	j.stopped = true
	j.state = contracts.TranscodeFailed
	j.errCode = code
	j.errMessage = message
	j.finishedAt = t.now().Unix()
	if j.paused {
		j.paused = false
	}
	t.mu.Unlock()
	if cmd != nil && cmd.Process != nil {
		_ = cmd.Process.Kill()
	}
}

func (t *Transcoder) run(j *job) {
	t.mu.Lock()
	if j.state != contracts.TranscodeQueued || t.closed {
		t.mu.Unlock()
		return
	}
	j.state = contracts.TranscodeRunning
	j.startedAt = t.now().Unix()
	t.mu.Unlock()
	t.logger().Info("transcode running", "session", j.spec.Session, "profile", j.spec.Profile)

	paths := t.pathsFor(j.spec.Session, j.settings)
	out := paths.media
	method, err := t.runConvert(j.spec, out, func(sample progressSample) {
		t.mu.Lock()
		if sample.HasFraction {
			j.progress = sample.Fraction
		}
		if sample.FPS > 0 {
			j.fps = sample.FPS
		}
		if sample.BitrateKbps > 0 {
			j.outputKbps = sample.BitrateKbps
		}
		t.mu.Unlock()
	})
	if err == nil && j.spec.SubtitleStream != nil {
		err = t.extractSubtitle(j.spec, paths.subtitle)
	}
	if err == nil {
		err = t.record(j.spec, method)
	}
	if err == nil {
		// Enforce the ready-cache budget before waking synchronous v1
		// waiters or reporting v2 ready.
		err = t.cleanup(false, t.activeSessionsWith(j.spec.Session))
	}

	t.mu.Lock()
	j.finishedAt = t.now().Unix()
	if err != nil {
		_ = os.Remove(out)
		_ = os.Remove(out + ".tmp")
		_ = os.Remove(paths.subtitle)
		_ = os.Remove(paths.subtitle + ".tmp")
		j.state = contracts.TranscodeFailed
		j.errCode = "dependency-unavailable"
		j.errMessage = "transcode failed"
		var ce *core.Error
		if errors.As(err, &ce) {
			j.errCode = ce.Code
			j.errMessage = ce.Msg
		}
	} else {
		j.state = contracts.TranscodeReady
		j.method = method
	}
	close(j.done)
	t.mu.Unlock()
	if err != nil {
		detail := strings.ReplaceAll(err.Error(), j.spec.Path, "<media>")
		t.logger().Warn("transcode failed", "session", j.spec.Session, "profile", j.spec.Profile, "err", detail)
	} else {
		t.logger().Info("transcode ready", "session", j.spec.Session, "profile", j.spec.Profile, "method", method, "dur_sec", j.finishedAt-j.startedAt)
	}
}

func (t *Transcoder) runConvert(spec sourceSpec, out string, onSample func(progressSample)) (string, error) {
	if t.convert != nil {
		return t.convert(spec, out, onSample)
	}
	return t.convertMedia(spec, out, onSample)
}

// runV3 executes a v3 session: progressive writes one MP4 atomically;
// HLS writes segments plus the server-owned playlist.
func (t *Transcoder) runV3(j *job) {
	t.mu.Lock()
	if j.state != contracts.TranscodeQueued || t.closed {
		t.mu.Unlock()
		return
	}
	j.state = contracts.TranscodeRunning
	j.startedAt = t.now().Unix()
	j.lastTouch = j.startedAt
	t.mu.Unlock()
	t.logger().Info("transcode running", "session", j.spec.Session, "profile", j.spec.Profile, "delivery", j.delivery)

	var err error
	if t.v3run != nil {
		err = t.v3run(j)
	} else {
		err = t.runV3Plan(j)
	}
	if err == nil {
		err = t.finalizeV3(j)
	}

	t.mu.Lock()
	j.finishedAt = t.now().Unix()
	if err == nil && j.stopped {
		// A stop that landed mid-run is terminal: never resurrect it as
		// Ready or cache it.
		err = errors.New("session stopped")
	}
	if err != nil {
		// A cancelled job's artifacts were already dropped by cancelV3;
		// removing them again could delete the artifacts of a restarted
		// job that now owns the same session paths.
		if !j.cancelled {
			t.removeV3Artifacts(j)
		}
		j.state = contracts.TranscodeFailed
		if j.errCode == "" {
			j.errCode = "dependency-unavailable"
			j.errMessage = "transcode failed"
		}
		var ce *core.Error
		if errors.As(err, &ce) {
			j.errCode = ce.Code
			j.errMessage = ce.Msg
		}
	} else {
		j.state = contracts.TranscodeReady
		j.playable = true
	}
	close(j.done)
	t.mu.Unlock()
	if err == nil {
		_ = t.cleanup(false, t.activeSessionsWith(j.spec.Session))
	}
	if err != nil {
		detail := strings.ReplaceAll(err.Error(), j.spec.Path, "<media>")
		t.logger().Warn("transcode failed", "session", j.spec.Session, "profile", j.spec.Profile, "err", detail)
	} else {
		t.logger().Info("transcode ready", "session", j.spec.Session, "profile", j.spec.Profile, "method", j.method, "dur_sec", j.finishedAt-j.startedAt)
	}
}

// runContext bounds one bounded preparation. An HLS session streams for
// as long as the viewer watches (throttling paces ffmpeg), so it gets no
// total-time deadline; it ends by finishing, cancellation or the idle
// timeout. Only progressive preparations are bounded by `timeout`.
func (t *Transcoder) runContext(delivery string) (context.Context, context.CancelFunc) {
	if delivery == contracts.TranscodeDeliveryHLS {
		return t.ctx, func() {}
	}
	return context.WithTimeout(t.ctx, timeout)
}

// runV3Plan performs the ffmpeg work for one planned session.
func (t *Transcoder) runV3Plan(j *job) error {
	t.mu.Lock()
	stopped := j.stopped
	t.mu.Unlock()
	if stopped {
		return errors.New("session stopped")
	}
	plan := *j.plan
	paths := t.pathsFor(j.spec.Session, j.settings)
	ctx, cancel := t.runContext(plan.delivery)
	defer cancel()

	var subtitleFile string
	if plan.burnText {
		subtitleFile = paths.burn
		if err := t.extractSubtitleAs(j.spec, *plan.subtitle, subtitleFile, "ass"); err != nil {
			return err
		}
	}
	// Extract the WebVTT sidecar before the encode so an HLS session can
	// offer its subtitle track from the first segment, instead of only once
	// the whole file has been produced (which for HLS is the very end).
	if plan.subtitleMode == contracts.SubtitleModeExtract && plan.subtitle != nil {
		if err := t.extractSubtitleAs(j.spec, *plan.subtitle, paths.subtitle, "webvtt"); err != nil {
			return err
		}
		t.mu.Lock()
		j.hasSubtitle = true
		t.mu.Unlock()
	}

	onProgress := func(sample progressSample) {
		t.mu.Lock()
		if sample.HasFraction {
			j.progress = sample.Fraction
		}
		if sample.FPS > 0 {
			j.fps = sample.FPS
		}
		if sample.BitrateKbps > 0 {
			j.outputKbps = sample.BitrateKbps
		}
		t.mu.Unlock()
	}
	onCmd := func(cmd *exec.Cmd) {
		t.mu.Lock()
		j.cmd = cmd
		t.mu.Unlock()
	}

	if plan.isHLS() {
		dir := paths.hlsDir
		if err := os.MkdirAll(dir, 0o700); err != nil {
			return err
		}
		raw := filepath.Join(dir, hlsRawPlaylist)
		err := t.execPlan(ctx, plan, raw, subtitleFile, onProgress, onCmd)
		if err != nil && (plan.hwBackend != "" || plan.hwDecode) {
			// Visible fallback: retry in software and report why.
			t.logger().Warn("hardware transcode failed; retrying in software", "session", j.spec.Session, "err", err.Error())
			fallback := plan.softwareFallback(t.capabilitiesFor(plan.settings))
			t.mu.Lock()
			j.fallback = fallback.fallback
			j.encoder = fallback.encoder
			j.hwBackend = ""
			t.mu.Unlock()
			err = t.execPlan(ctx, fallback, raw, subtitleFile, onProgress, onCmd)
		}
		if err != nil {
			return err
		}
		if err := writeIndexPlaylist(dir, plan.settings.HLSSegmentSeconds); err != nil {
			return err
		}
		t.mu.Lock()
		j.playable = hlsPlayable(dir)
		t.mu.Unlock()
		return nil
	}

	out := paths.media
	tmp := out + ".tmp"
	if err := os.MkdirAll(filepath.Dir(out), 0o700); err != nil {
		return err
	}
	err := t.execPlan(ctx, plan, tmp, subtitleFile, onProgress, onCmd)
	if err != nil && (plan.hwBackend != "" || plan.hwDecode) {
		t.logger().Warn("hardware transcode failed; retrying in software", "session", j.spec.Session, "err", err.Error())
		fallback := plan.softwareFallback(t.capabilitiesFor(plan.settings))
		t.mu.Lock()
		j.fallback = fallback.fallback
		j.encoder = fallback.encoder
		j.hwBackend = ""
		t.mu.Unlock()
		err = t.execPlan(ctx, fallback, tmp, subtitleFile, onProgress, onCmd)
	}
	if err != nil {
		_ = os.Remove(tmp)
		return err
	}
	if err := os.Rename(tmp, out); err != nil {
		_ = os.Remove(tmp)
		return err
	}
	return nil
}

// finalizeV3 extracts the sidecar subtitle and writes the cache entry.
func (t *Transcoder) finalizeV3(j *job) error {
	plan := *j.plan
	paths := t.pathsFor(j.spec.Session, j.settings)
	if plan.subtitleMode == contracts.SubtitleModeExtract && plan.subtitle != nil {
		t.mu.Lock()
		already := j.hasSubtitle
		t.mu.Unlock()
		if !already {
			if err := t.extractSubtitleAs(j.spec, *plan.subtitle, paths.subtitle, "webvtt"); err != nil {
				return err
			}
			t.mu.Lock()
			j.hasSubtitle = true
			t.mu.Unlock()
		}
	}
	if plan.burnText {
		_ = os.Remove(paths.burn)
	}
	return t.recordV3(j)
}

func (t *Transcoder) removeV3Artifacts(j *job) {
	paths := t.pathsFor(j.spec.Session, j.settings)
	if j.delivery == contracts.TranscodeDeliveryHLS {
		_ = os.RemoveAll(paths.hlsDir)
		return
	}
	_ = os.Remove(paths.media)
	_ = os.Remove(paths.media + ".tmp")
	_ = os.Remove(paths.subtitle)
	_ = os.Remove(paths.burn)
}

func (t *Transcoder) statusFromJob(j *job) contracts.TranscodeStatus {
	status := contracts.TranscodeStatus{
		Session: j.spec.Session, State: j.state, Profile: j.spec.Profile,
		Method: j.method, ErrorCode: j.errCode, Error: j.errMessage,
		QueuedAt: j.queuedAt, StartedAt: j.startedAt, FinishedAt: j.finishedAt,
	}
	// A finished in-memory job still has an artifact on disk; without this
	// path the v1/v2 caller would see an empty one (a 500 at the gateway)
	// when the cache entry was evicted while the job was still tracked.
	if j.state == contracts.TranscodeReady {
		status.Path = t.mediaPath(j.spec.Session)
	}
	// Progress is only meaningful while work is pending (D-039).
	if j.state == contracts.TranscodeQueued || j.state == contracts.TranscodeRunning {
		status.Progress = j.progress
	}
	return status
}

func statusFromEntry(e cacheEntry, path string, cached bool) contracts.TranscodeStatus {
	status := contracts.TranscodeStatus{
		Session: e.Session, State: contracts.TranscodeReady, Profile: e.Profile,
		Path: path, Method: e.Method, Cached: cached, FinishedAt: e.CreatedAt,
	}
	if e.HasSubtitle {
		status.SubtitlePath = filepath.Join(filepath.Dir(path), e.Session+".vtt")
	}
	return status
}

func (t *Transcoder) mediaPath(session string) string { return filepath.Join(t.dir, session+".mp4") }
func (t *Transcoder) metaPath(session string) string  { return filepath.Join(t.dir, session+".json") }
func (t *Transcoder) subtitlePath(session string) string {
	return filepath.Join(t.dir, session+".vtt")
}
func (t *Transcoder) burnSubtitlePath(session string) string {
	return filepath.Join(t.dir, session+".burn.ass")
}
func (t *Transcoder) hlsDir(session string) string {
	return filepath.Join(t.dir, session+".hls")
}

func (t *Transcoder) record(spec sourceSpec, method string) error {
	fi, err := os.Stat(t.mediaPath(spec.Session))
	if err != nil || fi.Size() == 0 {
		return errors.New("no mp4 produced")
	}
	size := fi.Size()
	if spec.SubtitleStream != nil {
		sub, subErr := os.Stat(t.subtitlePath(spec.Session))
		if subErr != nil || sub.Size() == 0 {
			return errors.New("no WebVTT produced")
		}
		size += sub.Size()
	}
	now := t.now().Unix()
	e := cacheEntry{
		Session: spec.Session, SourcePath: spec.Path, SourceSize: spec.Size,
		SourceModTime: spec.ModTimeNS, Profile: spec.Profile,
		AudioStream: cloneInt(spec.AudioStream), SubtitleStream: cloneInt(spec.SubtitleStream),
		Method: method, Size: size, CreatedAt: now, AccessedAt: now,
		HasSubtitle: spec.SubtitleStream != nil,
	}
	return writeJSONAtomic(t.metaPath(spec.Session), e)
}

// recordV3 writes the cache entry for a finished v3 session. A session
// the client cancelled never becomes a cache entry, even when ffmpeg
// happened to finish in the same instant.
func (t *Transcoder) recordV3(j *job) error {
	t.mu.Lock()
	plan := j.plan
	hasSubtitle := j.hasSubtitle
	cancelled := j.cancelled
	encoder, hw, fallback := j.encoder, j.hwBackend, j.fallback
	t.mu.Unlock()
	if cancelled {
		return errors.New("session cancelled")
	}
	paths := t.pathsFor(j.spec.Session, j.settings)
	artifactRoot := relocateRoot(j.settings)
	size := int64(0)
	if plan.isHLS() {
		size = hlsDirSize(paths.hlsDir)
		if !hlsPlayable(paths.hlsDir) {
			return errors.New("no playable HLS output produced")
		}
	} else {
		fi, err := os.Stat(paths.media)
		if err != nil || fi.Size() == 0 {
			return errors.New("no mp4 produced")
		}
		size = fi.Size()
	}
	if hasSubtitle {
		sub, err := os.Stat(paths.subtitle)
		if err != nil || sub.Size() == 0 {
			return errors.New("no WebVTT produced")
		}
		size += sub.Size()
	}
	now := t.now().Unix()
	e := cacheEntry{
		Session: j.spec.Session, SourcePath: j.spec.Path, SourceSize: j.spec.Size,
		SourceModTime: j.spec.ModTimeNS, Profile: j.spec.Profile, Delivery: plan.delivery,
		ArtifactRoot: artifactRoot,
		AudioStream:  cloneInt(j.spec.AudioStream), SubtitleStream: cloneInt(j.spec.SubtitleStream),
		Method: plan.method, Size: size, CreatedAt: now, AccessedAt: now,
		HasSubtitle: hasSubtitle, Encoder: encoder, Hardware: hw, Fallback: fallback,
		Reasons: plan.reasons, VideoCodec: plan.videoCodec, AudioCodec: plan.audioCodec,
		Width: plan.width, Height: plan.height, BitrateKbps: plan.maxBitrateKbps,
		VideoDirect: plan.copyVideo, AudioDirect: plan.copyAudio,
	}
	return writeJSONAtomic(t.metaPath(j.spec.Session), e)
}

func (t *Transcoder) readyEntry(spec sourceSpec) (cacheEntry, bool) {
	var e cacheEntry
	if err := readJSON(t.metaPath(spec.Session), &e); err != nil {
		return e, false
	}
	if e.Session != spec.Session || e.SourcePath != spec.Path || e.SourceSize != spec.Size || e.SourceModTime != spec.ModTimeNS || e.Profile != spec.Profile {
		return e, false
	}
	paths := t.pathsForEntry(e)
	if e.Delivery == contracts.TranscodeDeliverySubtitle {
		// A sidecar-only entry has exactly one artifact.
		fi, err := os.Stat(paths.subtitle)
		if err != nil || fi.Size() <= 0 {
			return e, false
		}
		e.Size = fi.Size()
		return e, true
	}
	if e.Delivery == contracts.TranscodeDeliveryHLS {
		if !hlsPlayable(paths.hlsDir) {
			return e, false
		}
		e.Size = hlsDirSize(paths.hlsDir)
		if e.HasSubtitle {
			sub, err := os.Stat(paths.subtitle)
			if err != nil || sub.Size() <= 0 {
				return e, false
			}
			e.Size += sub.Size()
		}
		return e, true
	}
	fi, err := os.Stat(paths.media)
	if err != nil || fi.Size() <= 0 {
		return e, false
	}
	e.Size = fi.Size()
	if e.HasSubtitle {
		sub, err := os.Stat(paths.subtitle)
		if err != nil || sub.Size() <= 0 {
			return e, false
		}
		e.Size += sub.Size()
	}
	return e, true
}

func (t *Transcoder) touch(e cacheEntry) error {
	e.AccessedAt = t.now().Unix()
	return writeJSONAtomic(t.metaPath(e.Session), e)
}

func writeJSONAtomic(path string, v any) error {
	raw, err := json.Marshal(v)
	if err != nil {
		return err
	}
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, raw, 0o600); err != nil {
		return err
	}
	if err := os.Rename(tmp, path); err != nil {
		_ = os.Remove(tmp)
		return err
	}
	return nil
}

func readJSON(path string, v any) error {
	raw, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	return json.Unmarshal(raw, v)
}

// cacheBudget reads the ready-cache quota under the lock: admission may
// adopt a new value from request settings while cleanup is running.
func (t *Transcoder) cacheBudget() int64 {
	t.mu.Lock()
	defer t.mu.Unlock()
	return t.config.MaxCacheBytes
}

func (t *Transcoder) activeSessions() map[string]bool {
	return t.activeSessionsWith("")
}

func (t *Transcoder) activeSessionsWith(extra string) map[string]bool {
	t.mu.Lock()
	defer t.mu.Unlock()
	out := map[string]bool{}
	if extra != "" {
		out[extra] = true
	}
	for id, j := range t.jobs {
		if j.state == contracts.TranscodeQueued || j.state == contracts.TranscodeRunning {
			out[id] = true
		}
	}
	return out
}

// recentAccessGrace protects a recently-fetched session from eviction: a
// ready HLS session is still playable, so its segments must survive while
// a client is watching.
const recentAccessGrace = 60 * time.Second

// cleanup removes abandoned/stale derivatives and then evicts the
// least-recently-used ready entries until the configured ready-cache
// budget is met. Active temporary output is intentionally not counted.
func (t *Transcoder) cleanup(startup bool, exclude map[string]bool) error {
	entries, err := os.ReadDir(t.dir)
	if err != nil {
		return err
	}
	if exclude == nil {
		exclude = map[string]bool{}
	}
	budget := t.cacheBudget()
	meta := map[string]bool{}
	var ready []cacheEntry
	var total int64
	for _, de := range entries {
		name := de.Name()
		path := filepath.Join(t.dir, name)
		if filepath.Ext(name) != ".json" {
			if startup && filepath.Ext(name) == ".tmp" {
				_ = os.Remove(path)
			}
			continue
		}
		var e cacheEntry
		if err := readJSON(path, &e); err != nil || e.Session == "" {
			_ = os.Remove(path)
			continue
		}
		meta[e.Session] = true
		paths := t.pathsForEntry(e)
		var mediaInfo os.FileInfo
		var mediaErr error
		if e.Delivery != contracts.TranscodeDeliveryHLS && e.Delivery != contracts.TranscodeDeliverySubtitle {
			mediaInfo, mediaErr = os.Stat(paths.media)
		}
		sourceInfo, sourceErr := os.Stat(e.SourcePath)
		var subtitleInfo os.FileInfo
		var subtitleErr error
		if e.HasSubtitle || e.Delivery == contracts.TranscodeDeliverySubtitle {
			subtitleInfo, subtitleErr = os.Stat(paths.subtitle)
		}
		sidecarOK := subtitleInfo != nil && subtitleErr == nil && subtitleInfo.Size() > 0
		stale := false
		switch e.Delivery {
		case contracts.TranscodeDeliverySubtitle:
			stale = !sidecarOK
		case contracts.TranscodeDeliveryHLS:
			stale = !hlsPlayable(paths.hlsDir)
		default:
			stale = mediaErr != nil || mediaInfo.Size() <= 0
		}
		stale = stale || sourceErr != nil ||
			sourceInfo.Size() != e.SourceSize || sourceInfo.ModTime().UnixNano() != e.SourceModTime
		if e.HasSubtitle && e.Delivery != contracts.TranscodeDeliverySubtitle && !sidecarOK {
			stale = true
		}
		if stale && !exclude[e.Session] {
			t.removeArtifacts(e, paths)
			continue
		}
		if stale {
			continue
		}
		switch e.Delivery {
		case contracts.TranscodeDeliverySubtitle:
			e.Size = subtitleInfo.Size()
		case contracts.TranscodeDeliveryHLS:
			e.Size = hlsDirSize(paths.hlsDir)
		default:
			e.Size = mediaInfo.Size()
		}
		if e.HasSubtitle && e.Delivery != contracts.TranscodeDeliverySubtitle {
			e.Size += subtitleInfo.Size()
		}
		ready = append(ready, e)
		total += e.Size
	}
	// Remove artifacts that have no metadata. A crash can leave one
	// between the media rename and sidecar write.
	for _, de := range entries {
		name := de.Name()
		switch {
		case filepath.Ext(name) == ".mp4":
			id := name[:len(name)-len(".mp4")]
			if !meta[id] && !exclude[id] {
				_ = os.Remove(filepath.Join(t.dir, name))
			}
		case strings.HasSuffix(name, ".hls") && de.IsDir():
			id := strings.TrimSuffix(name, ".hls")
			if !meta[id] && !exclude[id] {
				_ = os.RemoveAll(filepath.Join(t.dir, name))
			}
		}
	}
	sort.Slice(ready, func(i, j int) bool {
		if ready[i].AccessedAt == ready[j].AccessedAt {
			return ready[i].CreatedAt < ready[j].CreatedAt
		}
		return ready[i].AccessedAt < ready[j].AccessedAt
	})
	grace := t.now().Add(-recentAccessGrace).Unix()
	for _, e := range ready {
		if total <= budget {
			break
		}
		// A ready HLS session can still be watched: deleting its segments
		// mid-playback would break the client, so a session touched within
		// the grace window is never the eviction victim. The cache may then
		// exceed its budget while a viewer holds a session, which the
		// warning below reports rather than pulling the file away silently.
		if exclude[e.Session] || e.AccessedAt >= grace {
			continue
		}
		t.removeArtifacts(e, t.pathsForEntry(e))
		total -= e.Size
		t.logger().Debug("transcode cache evict", "session", e.Session, "bytes", e.Size)
	}
	if total > budget {
		t.logger().Warn("transcode cache over quota", "bytes", total, "max_bytes", budget)
	}
	return nil
}

// removeArtifacts deletes every file of one session, wherever it lives.
func (t *Transcoder) removeArtifacts(e cacheEntry, paths artifactPaths) {
	_ = os.Remove(paths.media)
	_ = os.Remove(paths.media + ".tmp")
	_ = os.Remove(paths.subtitle)
	_ = os.Remove(paths.burn)
	// The HLS directory holds the playlist and segments; in the relocated
	// layout it is the session directory itself.
	_ = os.RemoveAll(paths.hlsDir)
	if e.ArtifactRoot != "" {
		// Drop the now-empty session directory too: it is lain's own
		// namespace under the operator's temp path.
		_ = os.Remove(artifactDirFor(e.ArtifactRoot, e.Session))
	}
	_ = os.Remove(t.metaPath(e.Session))
}

// sweepRelocatedOrphans removes session directories under a configured
// temp path that no sidecar claims (a crash between mkdir and record),
// scoped to lain's own subdirectory so operator files are never touched.
// Directories of active sessions are excluded: they have no sidecar yet
// but must never be deleted out from under a viewer.
func (t *Transcoder) sweepRelocatedOrphans(root string, exclude map[string]bool) {
	if root == "" {
		return
	}
	// The temp root can be shared by more than one lain instance (same
	// transcode_temp_path, different data dirs). Only the instance that
	// owns the root may sweep it: one that did not create these session
	// directories cannot tell a live one from an orphan. The marker is
	// claimed atomically, and a marker left by a data dir that no longer
	// exists is taken over.
	owner := filepath.Join(root, ".owner")
	if raw, err := os.ReadFile(owner); err == nil {
		got := strings.TrimSpace(string(raw))
		if got != t.dir {
			if _, statErr := os.Stat(got); statErr == nil {
				t.logger().Warn("transcode temp path is owned by another data dir; skipping orphan sweep",
					"root", root, "owner", got, "data_dir", t.dir)
				return
			}
			_ = os.WriteFile(owner, []byte(t.dir), 0o600)
		}
	} else if f, err := os.OpenFile(owner, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o600); err == nil {
		_, _ = f.WriteString(t.dir)
		_ = f.Close()
	} else if raw, err := os.ReadFile(owner); err != nil || strings.TrimSpace(string(raw)) != t.dir {
		// Lost the atomic claim to another instance.
		return
	}
	entries, err := os.ReadDir(root)
	if err != nil {
		return
	}
	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}
		session := entry.Name()
		if exclude[session] {
			continue
		}
		if _, err := os.Stat(t.metaPath(session)); err == nil {
			continue
		}
		_ = os.RemoveAll(filepath.Join(root, session))
		t.logger().Debug("removed orphaned transcode directory", "session", session)
	}
}
