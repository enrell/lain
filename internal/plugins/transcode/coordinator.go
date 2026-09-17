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
)

// Config controls bounded derivative storage and work admission.
type Config struct {
	MaxCacheBytes int64
	QueueSize     int
}

type convertFunc func(spec sourceSpec, out string, onProgress func(float64)) (string, error)

type sourceSpec struct {
	Path           string
	Size           int64
	ModTimeNS      int64
	Profile        string
	AudioStream    *int
	SubtitleStream *int
	Session        string
}

type cacheEntry struct {
	Session        string `json:"session"`
	SourcePath     string `json:"source_path"`
	SourceSize     int64  `json:"source_size"`
	SourceModTime  int64  `json:"source_mod_time_ns"`
	Profile        string `json:"profile"`
	AudioStream    *int   `json:"audio_stream,omitempty"`
	SubtitleStream *int   `json:"subtitle_stream,omitempty"`
	HasSubtitle    bool   `json:"has_subtitle,omitempty"`
	Method         string `json:"method"`
	Size           int64  `json:"size"`
	CreatedAt      int64  `json:"created_at"`
	AccessedAt     int64  `json:"accessed_at"`
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
}

// Transcoder serves the synchronous v1 contract and the asynchronous
// v2 contract over one worker and one plugin-private cache.
type Transcoder struct {
	dir      string
	config   Config
	convert  convertFunc
	now      func() time.Time
	ctx      context.Context
	cancel   context.CancelFunc
	closeOne sync.Once
	log      atomic.Pointer[slog.Logger]

	mu     sync.Mutex
	jobs   map[string]*job
	queue  chan *job
	closed bool
	wg     sync.WaitGroup

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
	if now == nil {
		now = time.Now
	}
	ctx, cancel := context.WithCancel(context.Background())
	t := &Transcoder{
		dir: dir, config: cfg, convert: convert, now: now,
		ctx: ctx, cancel: cancel, jobs: map[string]*job{},
		queue: make(chan *job, cfg.QueueSize),
	}
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.initErr = err
		return t
	}
	if err := t.cleanup(true, nil); err != nil {
		t.initErr = err
		return t
	}
	t.wg.Add(1)
	go t.worker()
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
	return []string{contracts.CapPlaybackTranscode, contracts.CapPlaybackTranscodeV2}
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

// Close cancels a running conversion and rejects queued work.
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
		status := statusFromJob(j)
		t.mu.Unlock()
		if status.State != contracts.TranscodeReady {
			return status, nil
		}
	} else {
		t.mu.Unlock()
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
	h := sha256.Sum256([]byte(fmt.Sprintf("%s|%d|%d|%s|a:%s|s:%s", s.Path, s.ModTimeNS, s.Size, s.Profile, audio, subtitle)))
	return hex.EncodeToString(h[:])
}

func (t *Transcoder) inspect(spec sourceSpec) contracts.TranscodeStatus {
	if entry, ok := t.readyEntry(spec); ok {
		return statusFromEntry(entry, t.mediaPath(spec.Session), true)
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if j, ok := t.jobs[spec.Session]; ok {
		return statusFromJob(j)
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
			return statusFromJob(old), nil
		}
		delete(t.jobs, spec.Session) // an explicit POST retries a failed job
	}
	if len(t.queue) >= cap(t.queue) {
		return contracts.TranscodeStatus{}, &core.Error{Code: "queue-full", Msg: "transcode queue is full"}
	}
	now := t.now().Unix()
	j := &job{spec: spec, state: contracts.TranscodeQueued, queuedAt: now, done: make(chan struct{})}
	t.jobs[spec.Session] = j
	t.queue <- j
	return statusFromJob(j), nil
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
			t.run(j)
		}
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

	out := t.mediaPath(j.spec.Session)
	method, err := t.runConvert(j.spec, out, func(frac float64) {
		t.mu.Lock()
		j.progress = frac
		t.mu.Unlock()
	})
	if err == nil && j.spec.SubtitleStream != nil {
		err = t.extractSubtitle(j.spec, t.subtitlePath(j.spec.Session))
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
		_ = os.Remove(t.subtitlePath(j.spec.Session))
		_ = os.Remove(t.subtitlePath(j.spec.Session) + ".tmp")
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

func (t *Transcoder) runConvert(spec sourceSpec, out string, onProgress func(float64)) (string, error) {
	if t.convert != nil {
		return t.convert(spec, out, onProgress)
	}
	return t.convertMedia(spec, out, onProgress)
}

func statusFromJob(j *job) contracts.TranscodeStatus {
	status := contracts.TranscodeStatus{
		Session: j.spec.Session, State: j.state, Profile: j.spec.Profile,
		Method: j.method, ErrorCode: j.errCode, Error: j.errMessage,
		QueuedAt: j.queuedAt, StartedAt: j.startedAt, FinishedAt: j.finishedAt,
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

func (t *Transcoder) readyEntry(spec sourceSpec) (cacheEntry, bool) {
	var e cacheEntry
	if err := readJSON(t.metaPath(spec.Session), &e); err != nil {
		return e, false
	}
	if e.Session != spec.Session || e.SourcePath != spec.Path || e.SourceSize != spec.Size || e.SourceModTime != spec.ModTimeNS || e.Profile != spec.Profile {
		return e, false
	}
	fi, err := os.Stat(t.mediaPath(spec.Session))
	if err != nil || fi.Size() <= 0 {
		return e, false
	}
	e.Size = fi.Size()
	if e.HasSubtitle {
		sub, err := os.Stat(t.subtitlePath(spec.Session))
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
		media := t.mediaPath(e.Session)
		mediaInfo, mediaErr := os.Stat(media)
		sourceInfo, sourceErr := os.Stat(e.SourcePath)
		var subtitleInfo os.FileInfo
		var subtitleErr error
		if e.HasSubtitle {
			subtitleInfo, subtitleErr = os.Stat(t.subtitlePath(e.Session))
		}
		stale := mediaErr != nil || mediaInfo.Size() <= 0 || sourceErr != nil ||
			sourceInfo.Size() != e.SourceSize || sourceInfo.ModTime().UnixNano() != e.SourceModTime
		if e.HasSubtitle && (subtitleErr != nil || subtitleInfo.Size() <= 0) {
			stale = true
		}
		if stale && !exclude[e.Session] {
			_ = os.Remove(media)
			_ = os.Remove(t.subtitlePath(e.Session))
			_ = os.Remove(path)
			continue
		}
		if stale {
			continue
		}
		e.Size = mediaInfo.Size()
		if e.HasSubtitle {
			e.Size += subtitleInfo.Size()
		}
		ready = append(ready, e)
		total += e.Size
	}
	// Remove media artifacts that have no metadata. A crash can leave
	// one between the media rename and sidecar write.
	for _, de := range entries {
		name := de.Name()
		if filepath.Ext(name) != ".mp4" {
			continue
		}
		id := name[:len(name)-len(".mp4")]
		if !meta[id] && !exclude[id] {
			_ = os.Remove(filepath.Join(t.dir, name))
		}
	}
	sort.Slice(ready, func(i, j int) bool {
		if ready[i].AccessedAt == ready[j].AccessedAt {
			return ready[i].CreatedAt < ready[j].CreatedAt
		}
		return ready[i].AccessedAt < ready[j].AccessedAt
	})
	for _, e := range ready {
		if total <= t.config.MaxCacheBytes {
			break
		}
		if exclude[e.Session] {
			continue
		}
		_ = os.Remove(t.mediaPath(e.Session))
		_ = os.Remove(t.subtitlePath(e.Session))
		_ = os.Remove(t.metaPath(e.Session))
		total -= e.Size
		t.logger().Debug("transcode cache evict", "session", e.Session, "bytes", e.Size)
	}
	if total > t.config.MaxCacheBytes {
		t.logger().Warn("transcode cache over quota", "bytes", total, "max_bytes", t.config.MaxCacheBytes)
	}
	return nil
}
