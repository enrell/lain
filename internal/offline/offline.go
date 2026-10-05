// Package offline is the client-side store of offline copies: library
// items fetched from a lain server onto this machine so they play or
// read without the network (docs/slices/reading-downloads.md, P-6).
//
// The store is a directory with the finished files, a hidden .parts/
// folder for partial bytes, and index.json. Transfers resume through
// the server's Range-capable stream endpoint (internal/downloads.Fetch);
// the directory is bounded by configurable limits and, when room is
// needed, may evict copies the user already watched, oldest first.
// One process at a time mutates a store (Lock).
package offline

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/enrell/lain/internal/downloads"
)

// State is an entry's position in the offline queue.
type State string

const (
	Queued State = "queued"
	Paused State = "paused"
	Done   State = "done"
	Failed State = "failed"
)

// Progress is a playback or reading position recorded while offline,
// pushed to the server on the next sync.
type Progress struct {
	PositionSec float64 `json:"position_sec"`
	DurationSec float64 `json:"duration_sec"`
	Completed   bool    `json:"completed"`
	At          int64   `json:"at"`
}

// Entry is one offline copy (or a copy in progress).
type Entry struct {
	ItemID    string    `json:"item_id"`
	Server    string    `json:"server"`
	Title     string    `json:"title"`
	Kind      string    `json:"kind,omitempty"`
	Season    int       `json:"season,omitempty"`
	Episode   int       `json:"episode,omitempty"`
	File      string    `json:"file"` // base name inside the store
	Size      int64     `json:"size"` // expected bytes (catalog), 0 unknown
	Bytes     int64     `json:"bytes"`
	Validator string    `json:"validator,omitempty"`
	State     State     `json:"state"`
	Code      string    `json:"code,omitempty"`
	Error     string    `json:"error,omitempty"`
	Seq       uint64    `json:"seq"`
	AddedAt   int64     `json:"added_at"`
	DoneAt    int64     `json:"done_at,omitempty"`
	UsedAt    int64     `json:"used_at,omitempty"` // last played/read locally
	Watched   bool      `json:"watched,omitempty"`
	Pending   *Progress `json:"pending,omitempty"`
}

// Label is the human name of an entry.
func (e Entry) Label() string {
	switch {
	case e.Season > 0 && e.Episode > 0:
		return fmt.Sprintf("%s S%02dE%02d", e.Title, e.Season, e.Episode)
	case e.Episode > 0:
		return fmt.Sprintf("%s %d", e.Title, e.Episode)
	case e.Season > 0:
		return fmt.Sprintf("%s Vol %d", e.Title, e.Season)
	}
	return e.Title
}

// Config is the user's policy for the store.
type Config struct {
	Dir string `json:"dir"`
	downloads.Limits
	// EvictWatched lets the store delete watched copies, least recently
	// used first, when a new copy needs room.
	EvictWatched bool `json:"evict_watched"`
	// MaxRetries is how many consecutive transient failures `lain
	// download run` retries per copy before marking it failed; progress
	// resets the count.
	MaxRetries int `json:"max_retries"`
}

// Defaults for a small disk; every one is a setting (`lain download config`).
const (
	DefaultMaxBytes     = 10 << 30
	DefaultMinFreeBytes = 5 << 30
	DefaultMaxRetries   = 5
)

// DefaultConfig places the store under the user data dir.
func DefaultConfig(dataHome string) Config {
	return Config{
		Dir:          filepath.Join(dataHome, "lain", "offline"),
		Limits:       downloads.Limits{MaxBytes: DefaultMaxBytes, MinFreeBytes: DefaultMinFreeBytes},
		EvictWatched: true,
		MaxRetries:   DefaultMaxRetries,
	}
}

type index struct {
	Version int      `json:"version"`
	Seq     uint64   `json:"seq"`
	Entries []*Entry `json:"entries"`
}

// Store is an opened offline directory.
type Store struct {
	cfg Config
	idx index
	now func() time.Time
}

// Open loads (creating if needed) the store described by cfg.
func Open(cfg Config) (*Store, error) {
	if cfg.Dir == "" || !filepath.IsAbs(cfg.Dir) {
		return nil, &downloads.Error{Code: downloads.CodeInvalid, Msg: "offline dir must be an absolute path"}
	}
	if err := os.MkdirAll(filepath.Join(cfg.Dir, ".parts"), 0o755); err != nil {
		return nil, &downloads.Error{Code: downloads.CodeIO, Msg: err.Error()}
	}
	s := &Store{cfg: cfg, idx: index{Version: 1}, now: time.Now}
	raw, err := os.ReadFile(s.indexPath())
	switch {
	case err == nil:
		if err := json.Unmarshal(raw, &s.idx); err != nil {
			return nil, &downloads.Error{Code: downloads.CodeIO, Msg: "corrupt offline index: " + err.Error()}
		}
	case !os.IsNotExist(err):
		return nil, &downloads.Error{Code: downloads.CodeIO, Msg: err.Error()}
	}
	return s, nil
}

func (s *Store) indexPath() string { return filepath.Join(s.cfg.Dir, "index.json") }

// Config returns the policy the store was opened with.
func (s *Store) Config() Config { return s.cfg }

// Save writes the index atomically.
func (s *Store) Save() error {
	raw, err := json.MarshalIndent(s.idx, "", "  ")
	if err != nil {
		return err
	}
	tmp, err := os.CreateTemp(s.cfg.Dir, ".index-*.tmp")
	if err != nil {
		return &downloads.Error{Code: downloads.CodeIO, Msg: err.Error()}
	}
	if _, err := tmp.Write(append(raw, '\n')); err != nil {
		tmp.Close()
		os.Remove(tmp.Name())
		return &downloads.Error{Code: downloads.CodeIO, Msg: err.Error()}
	}
	tmp.Close()
	if err := os.Rename(tmp.Name(), s.indexPath()); err != nil {
		os.Remove(tmp.Name())
		return &downloads.Error{Code: downloads.CodeIO, Msg: err.Error()}
	}
	return nil
}

// Lock takes the store's single-writer lock. A lock left by a process
// that no longer exists is taken over.
func (s *Store) Lock() (unlock func(), err error) {
	p := filepath.Join(s.cfg.Dir, ".lock")
	for attempt := 0; attempt < 2; attempt++ {
		f, err := os.OpenFile(p, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
		if err == nil {
			fmt.Fprintf(f, "%d\n", os.Getpid())
			f.Close()
			return func() { os.Remove(p) }, nil
		}
		if !os.IsExist(err) {
			return nil, &downloads.Error{Code: downloads.CodeIO, Msg: err.Error()}
		}
		raw, _ := os.ReadFile(p)
		pid, _ := strconv.Atoi(strings.TrimSpace(string(raw)))
		if pid > 0 && pid != os.Getpid() && processAlive(pid) {
			return nil, &downloads.Error{Code: downloads.CodeState, Msg: fmt.Sprintf("another lain process (pid %d) is using the offline store", pid)}
		}
		os.Remove(p) // stale
	}
	return nil, &downloads.Error{Code: downloads.CodeState, Msg: "could not lock the offline store"}
}

// List returns entries in queue order.
func (s *Store) List() []Entry {
	out := make([]Entry, 0, len(s.idx.Entries))
	for _, e := range s.idx.Entries {
		out = append(out, *e)
	}
	sort.Slice(out, func(a, b int) bool { return out[a].Seq < out[b].Seq })
	return out
}

func (s *Store) find(id string) *Entry {
	for _, e := range s.idx.Entries {
		if e.ItemID == id {
			return e
		}
	}
	return nil
}

// Get returns one entry.
func (s *Store) Get(id string) (Entry, bool) {
	if e := s.find(id); e != nil {
		return *e, true
	}
	return Entry{}, false
}

// Path is where a finished entry's file lives.
func (s *Store) Path(e Entry) string { return filepath.Join(s.cfg.Dir, e.File) }

func (s *Store) partPath(e Entry) string {
	return filepath.Join(s.cfg.Dir, ".parts", downloads.SafeName(e.ItemID, "item")+".part")
}

// LocalPath returns the finished copy of an item, "" when there is none
// on disk (a vanished file does not count).
func (s *Store) LocalPath(itemID string) string {
	e := s.find(itemID)
	if e == nil || e.State != Done {
		return ""
	}
	p := s.Path(*e)
	if _, err := os.Stat(p); err != nil {
		return ""
	}
	return p
}

// Add queues an item. Adding an item already in the store requeues it
// when it was paused or failed and otherwise returns it unchanged.
// fileName is the server file's base name; it is made safe and unique.
func (s *Store) Add(e Entry, fileName string) (Entry, error) {
	if e.ItemID == "" {
		return Entry{}, &downloads.Error{Code: downloads.CodeInvalid, Msg: "item id required"}
	}
	if cur := s.find(e.ItemID); cur != nil {
		if cur.State == Paused || cur.State == Failed {
			cur.State, cur.Code, cur.Error = Queued, "", ""
		}
		return *cur, nil
	}
	taken := map[string]bool{}
	for _, o := range s.idx.Entries {
		taken[filepath.Join(s.cfg.Dir, o.File)] = true
	}
	name := downloads.SafeName(fileName, downloads.SafeName(e.Title, "item-"+e.ItemID))
	e.File = filepath.Base(downloads.FreePath(s.cfg.Dir, name, taken))
	s.idx.Seq++
	e.Seq = s.idx.Seq
	e.State = Queued
	e.Bytes = 0
	e.AddedAt = s.now().Unix()
	ne := e
	s.idx.Entries = append(s.idx.Entries, &ne)
	return ne, nil
}

// Pause marks a queued entry paused: `Run` skips it until resumed.
func (s *Store) Pause(id string) (Entry, error) {
	e := s.find(id)
	if e == nil {
		return Entry{}, &downloads.Error{Code: downloads.CodeNotFound, Msg: "not in the offline store"}
	}
	if e.State == Done {
		return *e, &downloads.Error{Code: downloads.CodeState, Msg: "already downloaded"}
	}
	e.State = Paused
	return *e, nil
}

// Resume requeues a paused or failed entry.
func (s *Store) Resume(id string) (Entry, error) {
	e := s.find(id)
	if e == nil {
		return Entry{}, &downloads.Error{Code: downloads.CodeNotFound, Msg: "not in the offline store"}
	}
	if e.State == Paused || e.State == Failed {
		e.State, e.Code, e.Error = Queued, "", ""
	}
	return *e, nil
}

// Remove deletes an entry and every byte it holds: its finished file or
// its partial download. Unsynced offline progress is refused unless
// force is set, so a position is never silently lost.
func (s *Store) Remove(id string, force bool) (freed int64, err error) {
	e := s.find(id)
	if e == nil {
		return 0, &downloads.Error{Code: downloads.CodeNotFound, Msg: "not in the offline store"}
	}
	if e.Pending != nil && !force {
		return 0, &downloads.Error{Code: downloads.CodeState, Msg: "offline progress not synced yet (run `lain download sync`, or pass --force)"}
	}
	for _, p := range []string{s.Path(*e), s.partPath(*e)} {
		if fi, err := os.Stat(p); err == nil {
			if err := os.Remove(p); err != nil {
				return freed, &downloads.Error{Code: downloads.CodeIO, Msg: err.Error()}
			}
			freed += fi.Size()
		}
	}
	s.drop(id)
	return freed, nil
}

func (s *Store) drop(id string) {
	out := s.idx.Entries[:0]
	for _, e := range s.idx.Entries {
		if e.ItemID != id {
			out = append(out, e)
		}
	}
	s.idx.Entries = out
}

// Used is what the store holds on disk: finished files and partials.
func (s *Store) Used() int64 { return s.usedExcept("") }

func (s *Store) usedExcept(id string) int64 {
	var used int64
	for _, e := range s.idx.Entries {
		if e.ItemID == id {
			continue
		}
		if e.State == Done {
			if fi, err := os.Stat(s.Path(*e)); err == nil {
				used += fi.Size()
			}
		} else {
			used += e.Bytes
		}
	}
	return used
}

// makeRoom checks that entry id may grow by n bytes, evicting watched,
// synced copies (least recently used first) when the policy allows.
func (s *Store) makeRoom(id string, have, n int64) ([]Entry, error) {
	var evicted []Entry
	for {
		err := s.cfg.Limits.Check(s.cfg.Dir, s.usedExcept(id)+have, n)
		if err == nil || !s.cfg.EvictWatched {
			return evicted, err
		}
		victim := s.evictionCandidate(id)
		if victim == nil {
			return evicted, err
		}
		v := *victim
		if _, rerr := s.Remove(v.ItemID, false); rerr != nil {
			return evicted, err
		}
		evicted = append(evicted, v)
	}
}

// evictionCandidate is the least recently used watched copy with no
// unsynced progress.
func (s *Store) evictionCandidate(except string) *Entry {
	var best *Entry
	for _, e := range s.idx.Entries {
		if e.ItemID == except || e.State != Done || !e.Watched || e.Pending != nil {
			continue
		}
		if best == nil || lastUse(e) < lastUse(best) {
			best = e
		}
	}
	return best
}

func lastUse(e *Entry) int64 { return max(e.UsedAt, e.DoneAt) }

// FetchInput names the stream to pull an entry from.
type FetchInput struct {
	URL      string
	Header   http.Header
	Progress func(done, total int64)
	// Evicted is told about each copy removed to make room.
	Evicted func(Entry)
}

// Fetch transfers one entry, resuming from its partial bytes. On
// success the file moves into place and the entry is Done; a canceled
// context leaves it Paused with its part kept; any other failure marks
// it Failed with a stable code. The index is saved either way.
func (s *Store) Fetch(ctx context.Context, client *http.Client, id string, in FetchInput) (Entry, error) {
	e := s.find(id)
	if e == nil {
		return Entry{}, &downloads.Error{Code: downloads.CodeNotFound, Msg: "not in the offline store"}
	}
	if e.State == Done {
		return *e, nil
	}
	part := s.partPath(*e)
	if fi, err := os.Stat(part); err == nil {
		e.Bytes = fi.Size()
	} else {
		e.Bytes = 0
	}
	reserve := func(n int64) error {
		ev, err := s.makeRoom(id, e.Bytes, n)
		for _, v := range ev {
			if in.Evicted != nil {
				in.Evicted(v)
			}
		}
		return err
	}
	res, err := downloads.Fetch(ctx, client, downloads.Request{
		URL: in.URL, Header: in.Header, Part: part, Validator: e.Validator,
		Reserve: reserve,
		Progress: func(done, total int64) {
			e.Bytes = done
			if in.Progress != nil {
				in.Progress(done, total)
			}
		},
	})
	e.Validator = res.Validator
	e.Bytes = res.Bytes
	if res.Total > 0 {
		e.Size = res.Total
	}
	switch {
	case err != nil && errors.Is(err, context.Canceled):
		e.State = Paused
	case err != nil:
		e.State = Failed
		e.Code, e.Error = downloads.CodeOf(err), err.Error()
		if e.Code == "" {
			e.Code = downloads.CodeIO
		}
	default:
		if rerr := os.Rename(part, s.Path(*e)); rerr != nil {
			e.State, e.Code, e.Error = Failed, downloads.CodeIO, rerr.Error()
			err = &downloads.Error{Code: downloads.CodeIO, Msg: rerr.Error()}
			break
		}
		e.State, e.Code, e.Error = Done, "", ""
		e.DoneAt = s.now().Unix()
	}
	if serr := s.Save(); serr != nil && err == nil {
		err = serr
	}
	return *e, err
}

// Touch records local use (playback) of an entry for LRU eviction.
func (s *Store) Touch(id string) {
	if e := s.find(id); e != nil {
		e.UsedAt = s.now().Unix()
	}
}

// SetWatched records the server's completed flag for an entry.
func (s *Store) SetWatched(id string, watched bool) {
	if e := s.find(id); e != nil {
		e.Watched = watched
	}
}

// RecordOffline keeps a position measured without the server. A
// completed position also marks the copy watched.
func (s *Store) RecordOffline(id string, p Progress) {
	if e := s.find(id); e != nil {
		p.At = s.now().Unix()
		e.Pending = &p
		e.UsedAt = p.At
		if p.Completed {
			e.Watched = true
		}
	}
}

// Pending lists entries with unsynced offline progress.
func (s *Store) Pending() []Entry {
	var out []Entry
	for _, e := range s.idx.Entries {
		if e.Pending != nil {
			out = append(out, *e)
		}
	}
	return out
}

// ClearPending forgets a synced offline position.
func (s *Store) ClearPending(id string) {
	if e := s.find(id); e != nil {
		e.Pending = nil
	}
}

// GCReport says what a cleanup pass did.
type GCReport struct {
	OrphanParts int     `json:"orphan_parts"`
	Vanished    int     `json:"vanished"`
	Evicted     []Entry `json:"evicted,omitempty"`
	FreedBytes  int64   `json:"freed_bytes"`
}

// GC removes partial files no entry owns, forgets finished entries
// whose file was deleted by hand, and — when watched is set — removes
// every watched copy without unsynced progress.
func (s *Store) GC(watched bool) (GCReport, error) {
	var rep GCReport
	owned := map[string]bool{}
	for _, e := range s.idx.Entries {
		owned[s.partPath(*e)] = true
	}
	ents, err := os.ReadDir(filepath.Join(s.cfg.Dir, ".parts"))
	if err != nil && !os.IsNotExist(err) {
		return rep, &downloads.Error{Code: downloads.CodeIO, Msg: err.Error()}
	}
	for _, d := range ents {
		p := filepath.Join(s.cfg.Dir, ".parts", d.Name())
		if owned[p] || !d.Type().IsRegular() {
			continue
		}
		if fi, err := d.Info(); err == nil && os.Remove(p) == nil {
			rep.OrphanParts++
			rep.FreedBytes += fi.Size()
		}
	}
	for _, e := range append([]*Entry(nil), s.idx.Entries...) {
		if e.State != Done {
			continue
		}
		if _, err := os.Stat(s.Path(*e)); os.IsNotExist(err) && e.Pending == nil {
			s.drop(e.ItemID)
			rep.Vanished++
			continue
		}
		if watched && e.Watched && e.Pending == nil {
			v := *e
			freed, err := s.Remove(e.ItemID, false)
			if err != nil {
				return rep, err
			}
			rep.FreedBytes += freed
			rep.Evicted = append(rep.Evicted, v)
		}
	}
	return rep, s.Save()
}
