package downloads

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"net"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/kv"
)

// State is a job's lifecycle position.
type State string

const (
	Queued   State = "queued"
	Running  State = "running"
	Paused   State = "paused"
	Done     State = "done"
	Failed   State = "failed"
	Canceled State = "canceled"
)

// terminal states never run again without an explicit resume.
func (s State) terminal() bool { return s == Done || s == Failed || s == Canceled }

// Job is one server-side download. Path is where the finished file
// lives; until then the bytes accumulate in Part, a hidden sibling so a
// library scan never sees half a file.
type Job struct {
	ID         string `json:"id"`
	URL        string `json:"url"`
	LibraryID  string `json:"library_id,omitempty"`
	Dir        string `json:"dir"`
	Name       string `json:"name"`
	NameAuto   bool   `json:"name_auto,omitempty"` // the origin may still name it
	Path       string `json:"path,omitempty"`
	Part       string `json:"-"`
	State      State  `json:"state"`
	Bytes      int64  `json:"bytes"`
	Total      int64  `json:"total"` // -1 or 0 while unknown
	Validator  string `json:"validator,omitempty"`
	Code       string `json:"code,omitempty"`
	Error      string `json:"error,omitempty"`
	CreatedBy  string `json:"created_by,omitempty"`
	Seq        uint64 `json:"seq"` // insertion order: the queue is FIFO
	CreatedAt  int64  `json:"created_at"`
	UpdatedAt  int64  `json:"updated_at"`
	FinishedAt int64  `json:"finished_at,omitempty"`
}

// Settings are the operator's limits (P-4). Every bound is a setting.
type Settings struct {
	// Dir receives downloads that do not target a library.
	Dir string `json:"dir"`
	Limits
	// Concurrency is how many jobs transfer at once.
	Concurrency int `json:"concurrency"`
	// KeepFinishedDays drops finished, failed and canceled records
	// (never files) after this many days on cleanup; 0 keeps them.
	KeepFinishedDays int `json:"keep_finished_days"`
}

// Defaults are deliberately small for a disk-constrained host; the
// operator raises them from Settings when a bigger disk arrives.
const (
	DefaultMaxBytes     = 20 << 30
	DefaultMinFreeBytes = 5 << 30
	DefaultConcurrency  = 2
	DefaultKeepDays     = 30
	maxConcurrency      = 8
)

// DefaultSettings returns the first-boot policy for a data dir.
func DefaultSettings(dataDir string) Settings {
	return Settings{
		Dir:              filepath.Join(dataDir, "downloads"),
		Limits:           Limits{MaxBytes: DefaultMaxBytes, MinFreeBytes: DefaultMinFreeBytes},
		Concurrency:      DefaultConcurrency,
		KeepFinishedDays: DefaultKeepDays,
	}
}

// Validate normalizes s or explains why it cannot be saved.
func (s Settings) Validate() (Settings, error) {
	s.Dir = strings.TrimSpace(s.Dir)
	if s.Dir == "" || !filepath.IsAbs(s.Dir) {
		return s, &Error{CodeInvalid, "dir must be an absolute path"}
	}
	s.Dir = filepath.Clean(s.Dir)
	if s.MaxBytes < 0 || s.MinFreeBytes < 0 || s.KeepFinishedDays < 0 {
		return s, &Error{CodeInvalid, "limits cannot be negative"}
	}
	if s.Concurrency < 1 || s.Concurrency > maxConcurrency {
		return s, &Error{CodeInvalid, "concurrency must be 1-8"}
	}
	return s, nil
}

// Usage is the store's footprint against its limits.
type Usage struct {
	UsedBytes    int64 `json:"used_bytes"`
	MaxBytes     int64 `json:"max_bytes"`
	FreeBytes    int64 `json:"free_bytes"` // -1 when unknown
	MinFreeBytes int64 `json:"min_free_bytes"`
}

// CleanupReport says what a cleanup pass removed.
type CleanupReport struct {
	Parts      int   `json:"parts"`
	Records    int   `json:"records"`
	FreedBytes int64 `json:"freed_bytes"`
}

// AddInput requests a download. Dir is resolved by the caller (a
// library root, or empty for Settings.Dir); LibraryID is carried back
// in OnDone so the caller can rescan.
type AddInput struct {
	URL       string
	Name      string
	Dir       string
	LibraryID string
	CreatedBy string
}

// Manager runs the server download queue. Jobs and settings persist
// in the kv `downloads` bucket; a restart resumes what was running.
type Manager struct {
	db     *bolt.DB
	client *http.Client
	now    func() time.Time

	// OnDone is called (outside the lock) after a job's file is in
	// place, e.g. to rescan its library.
	OnDone func(Job)

	mu       sync.Mutex
	settings Settings
	jobs     map[string]*Job
	seq      uint64
	running  map[string]*run
	kick     chan struct{}
	closed   chan struct{}
	wg       sync.WaitGroup
	started  bool
}

// run is the cancel handle of an active transfer; reason says which
// state the job lands in when the worker unwinds.
type run struct {
	cancel context.CancelFunc
	reason State
}

var (
	keySettings = []byte("settings")
	jobPrefix   = "job/"
)

// NewManager loads persisted jobs and settings. Jobs that were running
// when the process stopped come back queued; their parts resume.
func NewManager(db *bolt.DB, defaults Settings) (*Manager, error) {
	m := &Manager{
		db: db, now: time.Now, settings: defaults,
		jobs: map[string]*Job{}, running: map[string]*run{},
		kick: make(chan struct{}, 1), closed: make(chan struct{}),
		client: &http.Client{Transport: &http.Transport{
			Proxy:                 http.ProxyFromEnvironment,
			DialContext:           (&net.Dialer{Timeout: 30 * time.Second}).DialContext,
			TLSHandshakeTimeout:   30 * time.Second,
			ResponseHeaderTimeout: 60 * time.Second,
		}},
	}
	err := db.Update(func(tx *bolt.Tx) error {
		b, err := tx.CreateBucketIfNotExists(kv.BDownloads)
		if err != nil {
			return err
		}
		if raw := b.Get(keySettings); raw != nil {
			var s Settings
			if err := json.Unmarshal(raw, &s); err == nil {
				if v, err := s.Validate(); err == nil {
					m.settings = v
				}
			}
		}
		return b.ForEach(func(k, v []byte) error {
			if !strings.HasPrefix(string(k), jobPrefix) {
				return nil
			}
			var j Job
			if err := json.Unmarshal(v, &j); err != nil {
				return nil // a corrupt record is skipped, not fatal
			}
			j.Part = partPath(j)
			if j.State == Running {
				j.State = Queued
			}
			if !j.State.terminal() {
				if fi, err := os.Stat(j.Part); err == nil {
					j.Bytes = fi.Size()
				} else {
					j.Bytes = 0
				}
			}
			m.jobs[j.ID] = &j
			m.seq = max(m.seq, j.Seq)
			return nil
		})
	})
	if err != nil {
		return nil, err
	}
	return m, nil
}

// SetClient replaces the HTTP client (tests, proxies).
func (m *Manager) SetClient(c *http.Client) { m.client = c }

func partPath(j Job) string { return filepath.Join(j.Dir, ".lain-"+j.ID+".part") }

// Start launches the scheduler. Safe to call once; Close stops it.
func (m *Manager) Start() {
	m.mu.Lock()
	if m.started {
		m.mu.Unlock()
		return
	}
	m.started = true
	m.mu.Unlock()
	m.wg.Add(1)
	go m.loop()
	m.poke()
}

// Close stops every transfer. Running jobs are persisted as queued so
// the next start resumes them from their parts.
func (m *Manager) Close() {
	m.mu.Lock()
	select {
	case <-m.closed:
		m.mu.Unlock()
		return
	default:
	}
	close(m.closed)
	for _, r := range m.running {
		r.reason = Queued
		r.cancel()
	}
	m.mu.Unlock()
	m.wg.Wait()
}

func (m *Manager) poke() {
	select {
	case m.kick <- struct{}{}:
	default:
	}
}

func (m *Manager) loop() {
	defer m.wg.Done()
	for {
		select {
		case <-m.closed:
			return
		case <-m.kick:
			m.schedule()
		}
	}
}

// schedule starts queued jobs, oldest first, up to the concurrency.
func (m *Manager) schedule() {
	m.mu.Lock()
	defer m.mu.Unlock()
	select {
	case <-m.closed:
		return
	default:
	}
	var queued []*Job
	for _, j := range m.jobs {
		if j.State == Queued {
			queued = append(queued, j)
		}
	}
	sort.Slice(queued, func(a, b int) bool { return queued[a].Seq < queued[b].Seq })
	for _, j := range queued {
		if len(m.running) >= m.settings.Concurrency {
			return
		}
		ctx, cancel := context.WithCancel(context.Background())
		r := &run{cancel: cancel, reason: Failed}
		m.running[j.ID] = r
		m.setState(j, Running, "", "")
		m.wg.Add(1)
		go m.work(ctx, j.ID, r)
	}
}

// work transfers one job and settles its state.
func (m *Manager) work(ctx context.Context, id string, r *run) {
	defer m.wg.Done()
	m.mu.Lock()
	j := *m.jobs[id]
	m.mu.Unlock()

	err := os.MkdirAll(j.Dir, 0o755)
	var res Result
	if err == nil {
		res, err = Fetch(ctx, m.client, Request{
			URL: j.URL, Part: j.Part, Validator: j.Validator,
			Reserve:  func(n int64) error { return m.reserve(id, n) },
			Progress: func(done, total int64) { m.progress(id, done, total) },
		})
	} else {
		err = &Error{CodeIO, err.Error()}
	}

	m.mu.Lock()
	delete(m.running, id)
	jp, ok := m.jobs[id]
	if !ok { // removed while running
		m.mu.Unlock()
		_ = os.Remove(j.Part)
		m.poke()
		return
	}
	jp.Validator = res.Validator
	if res.Total > 0 {
		jp.Total = res.Total
	}
	var done *Job
	switch {
	case ctx.Err() != nil:
		// Stopped from outside: pause, cancel or shutdown decide.
		switch r.reason {
		case Canceled:
			_ = os.Remove(jp.Part)
			jp.Bytes = 0
			m.setState(jp, Canceled, "", "")
		case Paused:
			m.setState(jp, Paused, "", "")
		default:
			m.setState(jp, Queued, "", "")
		}
	case err != nil:
		code := CodeOf(err)
		if code == "" {
			code = CodeIO
		}
		m.setState(jp, Failed, code, err.Error())
	default:
		if jp.NameAuto && res.Filename != "" {
			jp.Name = SafeName(res.Filename, jp.Name)
		}
		final := FreePath(jp.Dir, jp.Name, m.reservedPaths(jp.ID))
		if rerr := os.Rename(jp.Part, final); rerr != nil {
			m.setState(jp, Failed, CodeIO, rerr.Error())
			break
		}
		jp.Path = final
		jp.Bytes = res.Bytes
		jp.FinishedAt = m.now().Unix()
		m.setState(jp, Done, "", "")
		c := *jp
		done = &c
	}
	m.mu.Unlock()
	if done != nil && m.OnDone != nil {
		m.OnDone(*done)
	}
	m.poke()
}

// reservedPaths are final paths other jobs already own.
func (m *Manager) reservedPaths(except string) map[string]bool {
	out := map[string]bool{}
	for id, j := range m.jobs {
		if id != except && j.Path != "" {
			out[j.Path] = true
		}
	}
	return out
}

// reserve is the budget check a transfer runs before it grows by n.
func (m *Manager) reserve(id string, n int64) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	j, ok := m.jobs[id]
	if !ok {
		return &Error{CodeNotFound, "job removed"}
	}
	used := m.usedLocked(id) + j.Bytes
	return m.settings.Limits.Check(j.Dir, used, n)
}

// usedLocked sums what tracked jobs hold on disk or have committed to:
// a finished file that still exists, a partial's bytes, and the full
// size of a running transfer whose total is known (so two parallel
// jobs cannot both claim the last free gigabyte).
func (m *Manager) usedLocked(except string) int64 {
	var used int64
	for id, j := range m.jobs {
		if id == except {
			continue
		}
		switch j.State {
		case Done:
			if fi, err := os.Stat(j.Path); err == nil {
				used += fi.Size()
			}
		case Running:
			used += max(j.Bytes, j.Total)
		case Queued, Paused, Failed:
			used += j.Bytes
		}
	}
	return used
}

func (m *Manager) progress(id string, done, total int64) {
	m.mu.Lock()
	if j, ok := m.jobs[id]; ok {
		j.Bytes = done
		if total > 0 {
			j.Total = total
		}
	}
	m.mu.Unlock()
}

// setState records a transition and persists it. Caller holds mu.
func (m *Manager) setState(j *Job, s State, code, msg string) {
	j.State, j.Code, j.Error = s, code, msg
	j.UpdatedAt = m.now().Unix()
	m.persistLocked(j)
}

func (m *Manager) persistLocked(j *Job) {
	raw, err := json.Marshal(j)
	if err != nil {
		return
	}
	_ = m.db.Update(func(tx *bolt.Tx) error {
		return tx.Bucket(kv.BDownloads).Put([]byte(jobPrefix+j.ID), raw)
	})
}

// Add validates and queues a download.
func (m *Manager) Add(in AddInput) (Job, error) {
	u, err := url.Parse(strings.TrimSpace(in.URL))
	if err != nil || (u.Scheme != "http" && u.Scheme != "https") || u.Host == "" {
		return Job{}, &Error{CodeInvalid, "url must be an absolute http or https URL"}
	}
	id, err := newID()
	if err != nil {
		return Job{}, &Error{CodeIO, err.Error()}
	}
	m.mu.Lock()
	dir := in.Dir
	if dir == "" {
		dir = m.settings.Dir
	}
	name := strings.TrimSpace(in.Name)
	auto := name == ""
	if auto {
		name = NameFromURL(u.String())
	}
	now := m.now()
	m.seq++
	j := &Job{
		Seq: m.seq,
		ID:  id, URL: u.String(), LibraryID: in.LibraryID, Dir: filepath.Clean(dir),
		Name: SafeName(name, "download-"+id), NameAuto: auto, State: Queued,
		CreatedBy: in.CreatedBy, CreatedAt: now.Unix(), UpdatedAt: now.Unix(), Total: -1,
	}
	j.Part = partPath(*j)
	// A known-full store refuses at the door instead of queueing a job
	// that can only fail.
	if err := m.settings.Limits.Check(j.Dir, m.usedLocked(""), 1); err != nil {
		m.mu.Unlock()
		return Job{}, err
	}
	m.jobs[id] = j
	m.persistLocked(j)
	out := *j
	m.mu.Unlock()
	m.poke()
	return out, nil
}

// List returns every job, newest first.
func (m *Manager) List() []Job {
	m.mu.Lock()
	defer m.mu.Unlock()
	out := make([]Job, 0, len(m.jobs))
	for _, j := range m.jobs {
		out = append(out, *j)
	}
	sort.Slice(out, func(a, b int) bool { return out[a].Seq > out[b].Seq })
	return out
}

// Get returns one job.
func (m *Manager) Get(id string) (Job, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	j, ok := m.jobs[id]
	if !ok {
		return Job{}, &Error{CodeNotFound, "unknown download"}
	}
	return *j, nil
}

// Pause stops a queued or running job, keeping its partial bytes.
func (m *Manager) Pause(id string) (Job, error) {
	return m.transition(id, func(j *Job) error {
		switch j.State {
		case Queued:
			m.setState(j, Paused, "", "")
		case Running:
			m.running[id].reason = Paused
			m.running[id].cancel()
		case Paused:
		default:
			return &Error{CodeState, "cannot pause a " + string(j.State) + " download"}
		}
		return nil
	})
}

// Resume requeues a paused or failed job; it continues from its part.
func (m *Manager) Resume(id string) (Job, error) {
	j, err := m.transition(id, func(j *Job) error {
		switch j.State {
		case Paused, Failed:
			m.setState(j, Queued, "", "")
		case Queued, Running:
		default:
			return &Error{CodeState, "cannot resume a " + string(j.State) + " download"}
		}
		return nil
	})
	if err == nil {
		m.poke()
	}
	return j, err
}

// Cancel stops a job for good and deletes its partial bytes. A finished
// download cannot be canceled: its file is library media now.
func (m *Manager) Cancel(id string) (Job, error) {
	return m.transition(id, func(j *Job) error {
		switch j.State {
		case Running:
			m.running[id].reason = Canceled
			m.running[id].cancel()
		case Queued, Paused, Failed:
			_ = os.Remove(j.Part)
			j.Bytes = 0
			m.setState(j, Canceled, "", "")
		case Canceled:
		default:
			return &Error{CodeState, "cannot cancel a " + string(j.State) + " download"}
		}
		return nil
	})
}

// Remove forgets a finished, failed or canceled job (its partial bytes
// go with it; a finished file stays where it is).
func (m *Manager) Remove(id string) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	j, ok := m.jobs[id]
	if !ok {
		return &Error{CodeNotFound, "unknown download"}
	}
	if !j.State.terminal() {
		return &Error{CodeState, "cancel or let the download finish first"}
	}
	if j.State != Done {
		_ = os.Remove(j.Part)
	}
	delete(m.jobs, id)
	return m.db.Update(func(tx *bolt.Tx) error {
		return tx.Bucket(kv.BDownloads).Delete([]byte(jobPrefix + id))
	})
}

func (m *Manager) transition(id string, f func(*Job) error) (Job, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	j, ok := m.jobs[id]
	if !ok {
		return Job{}, &Error{CodeNotFound, "unknown download"}
	}
	if err := f(j); err != nil {
		return *j, err
	}
	return *j, nil
}

// Settings returns the current policy.
func (m *Manager) Settings() Settings {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.settings
}

// SetSettings validates and saves a new policy. Lowering concurrency
// lets running jobs finish; raising it starts queued ones.
func (m *Manager) SetSettings(s Settings) (Settings, error) {
	s, err := s.Validate()
	if err != nil {
		return s, err
	}
	raw, _ := json.Marshal(s)
	m.mu.Lock()
	err = m.db.Update(func(tx *bolt.Tx) error {
		return tx.Bucket(kv.BDownloads).Put(keySettings, raw)
	})
	if err == nil {
		m.settings = s
	}
	m.mu.Unlock()
	if err != nil {
		return s, &Error{CodeIO, err.Error()}
	}
	m.poke()
	return s, nil
}

// Usage reports the footprint against the limits.
func (m *Manager) Usage() Usage {
	m.mu.Lock()
	used := m.usedLocked("")
	s := m.settings
	m.mu.Unlock()
	free := int64(-1)
	if f, err := freeBytes(existingParent(s.Dir)); err == nil {
		free = f
	}
	return Usage{UsedBytes: used, MaxBytes: s.MaxBytes, FreeBytes: free, MinFreeBytes: s.MinFreeBytes}
}

// existingParent walks up to a directory that exists, so free space can
// be reported before the download dir is created.
func existingParent(p string) string {
	for {
		if _, err := os.Stat(p); err == nil {
			return p
		}
		parent := filepath.Dir(p)
		if parent == p {
			return p
		}
		p = parent
	}
}

// Cleanup removes partial bytes of failed and canceled jobs and, past
// the retention window, the records of terminal jobs. It never deletes
// a finished file: those are library media (P-5).
func (m *Manager) Cleanup() (CleanupReport, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	var rep CleanupReport
	cutoff := int64(0)
	if m.settings.KeepFinishedDays > 0 {
		cutoff = m.now().Add(-time.Duration(m.settings.KeepFinishedDays) * 24 * time.Hour).Unix()
	}
	var errs []error
	for id, j := range m.jobs {
		if j.State == Failed || j.State == Canceled {
			if fi, err := os.Stat(j.Part); err == nil {
				if err := os.Remove(j.Part); err != nil {
					errs = append(errs, err)
				} else {
					rep.Parts++
					rep.FreedBytes += fi.Size()
				}
			}
			if j.State == Failed && j.Bytes > 0 {
				j.Bytes = 0
				j.Validator = ""
				m.persistLocked(j)
			}
		}
		if j.State.terminal() && cutoff > 0 && j.UpdatedAt < cutoff {
			if err := m.db.Update(func(tx *bolt.Tx) error {
				return tx.Bucket(kv.BDownloads).Delete([]byte(jobPrefix + id))
			}); err != nil {
				errs = append(errs, err)
				continue
			}
			delete(m.jobs, id)
			rep.Records++
		}
	}
	// Orphaned parts: a .lain-*.part in the download dir no job owns
	// (a record removed while the process was down).
	owned := map[string]bool{}
	for _, j := range m.jobs {
		owned[j.Part] = true
	}
	if ents, err := os.ReadDir(m.settings.Dir); err == nil {
		for _, e := range ents {
			n := e.Name()
			p := filepath.Join(m.settings.Dir, n)
			if e.Type().IsRegular() && strings.HasPrefix(n, ".lain-") && strings.HasSuffix(n, ".part") && !owned[p] {
				if fi, err := e.Info(); err == nil && os.Remove(p) == nil {
					rep.Parts++
					rep.FreedBytes += fi.Size()
				}
			}
		}
	}
	return rep, errors.Join(errs...)
}

func newID() (string, error) {
	b := make([]byte, 8)
	if _, err := rand.Read(b); err != nil {
		return "", err
	}
	return hex.EncodeToString(b), nil
}
