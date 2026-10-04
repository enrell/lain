// Package acquire is Lain's native *arr core (docs/slices/
// acquisition.md): indexers, grabs, the download client, seeding policy
// and import into libraries. It moves bytes and owns goroutines, so it
// is a core package beside internal/downloads, not a plugin (A-2);
// parsing and indexer protocols are capabilities it calls through
// injected functions.
package acquire

import (
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"net/url"
	"path/filepath"
	"sort"
	"strings"
	"time"
	"unicode/utf8"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/kv"
)

// Error codes. Messages are written for people; codes are stable.
const (
	CodeInvalid     = "invalid-request"
	CodeNotFound    = "not-found"
	CodeState       = "invalid-state"
	CodeQuota       = "quota-exceeded"
	CodeDiskFull    = "disk-full"
	CodeUnsupported = "unsupported-protocol"
	CodeFetch       = "fetch-failed"
	CodeImport      = "import-failed"
	CodeNoLibrary   = "no-library"
	CodeStalled     = "stalled"
)

// Error is a typed acquisition error.
type Error struct {
	Code string
	Msg  string
}

func (e *Error) Error() string { return e.Code + ": " + e.Msg }

func errf(code, format string, args ...any) error {
	return &Error{Code: code, Msg: fmt.Sprintf(format, args...)}
}

// CodeOf extracts a code ("" for untyped errors).
func CodeOf(err error) string {
	var e *Error
	if errors.As(err, &e) {
		return e.Code
	}
	return ""
}

// Import modes (A-10).
const (
	ImportHardlink = "hardlink" // falls back to copy across filesystems
	ImportCopy     = "copy"
	ImportMove     = "move"
)

// Settings are the operator's acquisition policy. Every bound is a
// setting (A-5). Disk limits are the reading slice's download limits,
// shared, not duplicated (A-4).
type Settings struct {
	// Dir holds torrent data while it downloads and seeds.
	Dir string `json:"dir"`
	// ListenPort accepts inbound peers; 0 picks a free port at each start
	// (forward a fixed port for good connectivity behind NAT).
	ListenPort int `json:"listen_port"`
	// MaxActive bounds torrents downloading at once.
	MaxActive int `json:"max_active"`
	MaxPeers  int `json:"max_peers"`
	// UploadKBps and DownloadKBps cap transfer rates; 0 = unlimited.
	UploadKBps   int `json:"upload_kbps"`
	DownloadKBps int `json:"download_kbps"`
	// SeedRatio and SeedMinutes stop seeding when either is reached;
	// both 0 means "do not seed" (import, then stop).
	SeedRatio   float64 `json:"seed_ratio"`
	SeedMinutes int     `json:"seed_minutes"`
	// RemoveAfterSeeding deletes the torrent's copy once seeding stops
	// (the library copy stays).
	RemoveAfterSeeding bool   `json:"remove_after_seeding"`
	ImportMode         string `json:"import_mode"`
	// Automation (A-23) is off until enabled. RSSMinutes is the RSS
	// sync period, SearchHours the missing-search period (0 = on demand
	// only), StallHours fails a download that has not moved (0 = never).
	Automation  bool `json:"automation"`
	RSSMinutes  int  `json:"rss_minutes"`
	SearchHours int  `json:"search_hours"`
	StallHours  int  `json:"stall_hours"`
}

// DefaultSettings fit the current small disk: seed to 1.0 or a day.
func DefaultSettings(dataDir string) Settings {
	return Settings{
		Dir: filepath.Join(dataDir, "acquire"), ListenPort: 51413, MaxActive: 3, MaxPeers: 40,
		SeedRatio: 1.0, SeedMinutes: 24 * 60, RemoveAfterSeeding: true, ImportMode: ImportHardlink,
		RSSMinutes: 30, SearchHours: 12, StallHours: 6,
	}
}

// Validate normalizes s or explains why it cannot be saved.
func (s Settings) Validate() (Settings, error) {
	s.Dir = strings.TrimSpace(s.Dir)
	if s.Dir == "" || !filepath.IsAbs(s.Dir) {
		return s, errf(CodeInvalid, "dir must be an absolute path")
	}
	s.Dir = filepath.Clean(s.Dir)
	switch {
	case s.ListenPort < 0 || s.ListenPort > 65535:
		return s, errf(CodeInvalid, "listen_port must be 0-65535")
	case s.MaxActive < 1 || s.MaxActive > 20:
		return s, errf(CodeInvalid, "max_active must be 1-20")
	case s.MaxPeers < 1 || s.MaxPeers > 200:
		return s, errf(CodeInvalid, "max_peers must be 1-200")
	case s.UploadKBps < 0 || s.DownloadKBps < 0:
		return s, errf(CodeInvalid, "rates cannot be negative")
	case s.SeedRatio < 0 || s.SeedRatio > 100 || s.SeedMinutes < 0 || s.SeedMinutes > 525600:
		return s, errf(CodeInvalid, "seed limits out of range")
	}
	if s.RSSMinutes == 0 {
		s.RSSMinutes = 30
	}
	switch {
	case s.RSSMinutes < 10 || s.RSSMinutes > 1440:
		return s, errf(CodeInvalid, "rss_minutes must be 10-1440")
	case s.SearchHours < 0 || s.SearchHours > 168:
		return s, errf(CodeInvalid, "search_hours must be 0-168")
	case s.StallHours < 0 || s.StallHours > 168:
		return s, errf(CodeInvalid, "stall_hours must be 0-168")
	}
	switch s.ImportMode {
	case ImportHardlink, ImportCopy, ImportMove:
	case "":
		s.ImportMode = ImportHardlink
	default:
		return s, errf(CodeInvalid, "import_mode must be hardlink, copy or move")
	}
	return s, nil
}

// Seeds reports whether the policy seeds at all.
func (s Settings) Seeds() bool { return s.SeedRatio > 0 || s.SeedMinutes > 0 }

// Grab states.
const (
	GrabQueued      = "queued"      // waiting for a download slot
	GrabMetadata    = "metadata"    // magnet: fetching the info dictionary
	GrabDownloading = "downloading" //
	GrabImporting   = "importing"   //
	GrabSeeding     = "seeding"     // imported, seeding under the policy
	GrabDone        = "done"        // imported, no longer seeding
	GrabPaused      = "paused"      //
	GrabFailed      = "failed"      //
)

// Grab is one release taken from an indexer (or a pasted magnet).
type Grab struct {
	ID        string            `json:"id"`
	Title     string            `json:"title"`
	IndexerID string            `json:"indexer_id,omitempty"`
	Source    string            `json:"source"` // "torrent-url", "magnet"
	Magnet    string            `json:"magnet,omitempty"`
	InfoHash  string            `json:"info_hash,omitempty"`
	Release   contracts.Release `json:"release"`
	LibraryID string            `json:"library_id"`
	// Dir is the grab's own folder for torrent data.
	Dir        string `json:"dir,omitempty"`
	Kind       string `json:"kind"` // the library type
	State      string `json:"state"`
	Size       int64  `json:"size"`
	Downloaded int64  `json:"downloaded"`
	Uploaded   int64  `json:"uploaded"`
	Completed  int64  `json:"completed"`
	Peers      int    `json:"peers"`
	DownRate   int64  `json:"down_rate"`
	UpRate     int64  `json:"up_rate"`
	// Imported lists library paths the import created.
	Imported   []string `json:"imported,omitempty"`
	Code       string   `json:"code,omitempty"`
	Error      string   `json:"error,omitempty"`
	CreatedBy  string   `json:"created_by,omitempty"`
	CreatedAt  int64    `json:"created_at"`
	UpdatedAt  int64    `json:"updated_at"`
	FinishedAt int64    `json:"finished_at,omitempty"` // download complete
	SeedingAt  int64    `json:"seeding_at,omitempty"`
	// DataRemoved is true once the torrent copy is gone from Dir.
	DataRemoved bool `json:"data_removed,omitempty"`
	// Automation bookkeeping (Phase 2): the monitored title this grab
	// serves and the units it covers. An upgrade lists the library files
	// it replaces (ReplaceFrom) and where they were held (Replaced).
	MonitoredID string   `json:"monitored_id,omitempty"`
	Units       []Unit   `json:"units,omitempty"`
	Upgrade     bool     `json:"upgrade,omitempty"`
	ReplaceFrom []string `json:"replace_from,omitempty"`
	Replaced    []string `json:"replaced,omitempty"`
}

// OnDisk reports whether the grab's torrent data counts against the
// budget (A-4).
func (g Grab) OnDisk() bool {
	return !g.DataRemoved && g.State != GrabFailed
}

// store persists indexers, settings and grabs.
type store struct {
	db *bolt.DB
}

func newStore(db *bolt.DB) (*store, error) {
	err := db.Update(func(tx *bolt.Tx) error {
		for _, b := range kv.AcquireBuckets() {
			if _, err := tx.CreateBucketIfNotExists(b); err != nil {
				return fmt.Errorf("bucket %s: %w", b, err)
			}
		}
		return nil
	})
	return &store{db: db}, err
}

func newID() string {
	var b [8]byte
	_, _ = rand.Read(b[:])
	return hex.EncodeToString(b[:])
}

func (s *store) settings(def Settings) Settings {
	out := def
	_ = s.db.View(func(tx *bolt.Tx) error {
		raw := tx.Bucket(kv.BAcqSettings).Get([]byte("settings"))
		if raw != nil {
			_ = json.Unmarshal(raw, &out)
		}
		return nil
	})
	return out
}

func (s *store) saveSettings(v Settings) error {
	return s.db.Update(func(tx *bolt.Tx) error {
		return kv.PutJSON(tx, kv.BAcqSettings, []byte("settings"), v)
	})
}

// ---- indexers ----

// IndexerInput creates or patches an indexer; nil fields stay.
type IndexerInput struct {
	Name          *string `json:"name"`
	Protocol      *string `json:"protocol"`
	URL           *string `json:"url"`
	APIKey        *string `json:"api_key"`
	Categories    *[]int  `json:"categories"`
	Enabled       *bool   `json:"enabled"`
	Priority      *int    `json:"priority"`
	MinIntervalMs *int    `json:"min_interval_ms"`
}

func cleanName(s string, max int) (string, error) {
	s = strings.Join(strings.Fields(s), " ")
	if !utf8.ValidString(s) || utf8.RuneCountInString(s) > max {
		return "", errf(CodeInvalid, "name must be text up to %d characters", max)
	}
	for _, r := range s {
		if r < 0x20 {
			return "", errf(CodeInvalid, "name must not contain control characters")
		}
	}
	return s, nil
}

func (in IndexerInput) apply(ix *contracts.Indexer) error {
	if in.Name != nil {
		n, err := cleanName(*in.Name, 60)
		if err != nil {
			return err
		}
		ix.Name = n
	}
	if in.Protocol != nil {
		ix.Protocol = *in.Protocol
	}
	if in.URL != nil {
		ix.URL = strings.TrimSpace(*in.URL)
	}
	if in.APIKey != nil {
		// An empty key in a patch keeps the stored one; "-" clears it.
		switch k := strings.TrimSpace(*in.APIKey); k {
		case "":
		case "-":
			ix.APIKey = ""
		default:
			if len(k) > 512 {
				return errf(CodeInvalid, "api key too long")
			}
			ix.APIKey = k
		}
	}
	if in.Categories != nil {
		if len(*in.Categories) > 50 {
			return errf(CodeInvalid, "at most 50 categories")
		}
		ix.Categories = append([]int{}, *in.Categories...)
	}
	if in.Enabled != nil {
		ix.Enabled = *in.Enabled
	}
	if in.Priority != nil {
		ix.Priority = *in.Priority
	}
	if in.MinIntervalMs != nil {
		ix.MinIntervalMs = *in.MinIntervalMs
	}
	if ix.Name == "" {
		return errf(CodeInvalid, "name required")
	}
	if ix.Protocol != contracts.IndexerTorznab && ix.Protocol != contracts.IndexerNewznab {
		return errf(CodeInvalid, "protocol must be torznab or newznab")
	}
	u, err := url.Parse(ix.URL)
	if err != nil || (u.Scheme != "http" && u.Scheme != "https") || u.Host == "" || u.User != nil {
		return errf(CodeInvalid, "url must be http(s) without credentials (use the API key field)")
	}
	if ix.MinIntervalMs < 0 || ix.MinIntervalMs > 600_000 {
		return errf(CodeInvalid, "min_interval_ms must be 0-600000")
	}
	if ix.Categories == nil {
		ix.Categories = []int{}
	}
	return nil
}

func (s *store) indexers() []contracts.Indexer {
	out := []contracts.Indexer{}
	_ = s.db.View(func(tx *bolt.Tx) error {
		return tx.Bucket(kv.BAcqIndexers).ForEach(func(_, v []byte) error {
			var ix contracts.Indexer
			if json.Unmarshal(v, &ix) == nil {
				out = append(out, ix)
			}
			return nil
		})
	})
	sort.SliceStable(out, func(i, j int) bool {
		if out[i].Priority != out[j].Priority {
			return out[i].Priority < out[j].Priority
		}
		return out[i].Name < out[j].Name
	})
	return out
}

func (s *store) indexer(id string) (contracts.Indexer, error) {
	var ix contracts.Indexer
	err := s.db.View(func(tx *bolt.Tx) error {
		return kv.GetJSON(tx, kv.BAcqIndexers, []byte(id), &ix)
	})
	if err != nil {
		return ix, errf(CodeNotFound, "unknown indexer")
	}
	return ix, nil
}

func (s *store) putIndexer(ix contracts.Indexer) error {
	return s.db.Update(func(tx *bolt.Tx) error {
		return kv.PutJSON(tx, kv.BAcqIndexers, []byte(ix.ID), ix)
	})
}

func (s *store) deleteIndexer(id string) error {
	return s.db.Update(func(tx *bolt.Tx) error {
		b := tx.Bucket(kv.BAcqIndexers)
		if b.Get([]byte(id)) == nil {
			return errf(CodeNotFound, "unknown indexer")
		}
		return b.Delete([]byte(id))
	})
}

// ---- grabs ----

func (s *store) grabs() []Grab {
	out := []Grab{}
	_ = s.db.View(func(tx *bolt.Tx) error {
		return tx.Bucket(kv.BAcqGrabs).ForEach(func(_, v []byte) error {
			var g Grab
			if json.Unmarshal(v, &g) == nil {
				out = append(out, g)
			}
			return nil
		})
	})
	sort.SliceStable(out, func(i, j int) bool { return out[i].CreatedAt > out[j].CreatedAt })
	return out
}

func (s *store) grab(id string) (Grab, error) {
	var g Grab
	err := s.db.View(func(tx *bolt.Tx) error {
		return kv.GetJSON(tx, kv.BAcqGrabs, []byte(id), &g)
	})
	if err != nil {
		return g, errf(CodeNotFound, "unknown grab")
	}
	return g, nil
}

func (s *store) putGrab(g Grab) error {
	g.UpdatedAt = time.Now().Unix()
	return s.db.Update(func(tx *bolt.Tx) error {
		return kv.PutJSON(tx, kv.BAcqGrabs, []byte(g.ID), g)
	})
}

func (s *store) deleteGrab(id string) error {
	return s.db.Update(func(tx *bolt.Tx) error {
		return tx.Bucket(kv.BAcqGrabs).Delete([]byte(id))
	})
}

// torrentState is the engine's resume record.
type torrentState struct {
	MetaInfo []byte `json:"metainfo,omitempty"` // raw .torrent, or a synthesized one for magnets
	// Magnet restarts a magnet whose metadata had not arrived yet.
	Magnet string `json:"magnet,omitempty"`
	Have   []byte `json:"have,omitempty"`
	Dir    string `json:"dir"`
	Paused bool   `json:"paused,omitempty"`
}

func (s *store) torrent(ih string) (torrentState, bool) {
	var st torrentState
	err := s.db.View(func(tx *bolt.Tx) error {
		return kv.GetJSON(tx, kv.BAcqTorrents, []byte(ih), &st)
	})
	return st, err == nil
}

func (s *store) putTorrent(ih string, st torrentState) error {
	return s.db.Update(func(tx *bolt.Tx) error {
		return kv.PutJSON(tx, kv.BAcqTorrents, []byte(ih), st)
	})
}

func (s *store) deleteTorrent(ih string) error {
	return s.db.Update(func(tx *bolt.Tx) error {
		return tx.Bucket(kv.BAcqTorrents).Delete([]byte(ih))
	})
}

func (s *store) torrents() map[string]torrentState {
	out := map[string]torrentState{}
	_ = s.db.View(func(tx *bolt.Tx) error {
		return tx.Bucket(kv.BAcqTorrents).ForEach(func(k, v []byte) error {
			var st torrentState
			if json.Unmarshal(v, &st) == nil {
				out[string(k)] = st
			}
			return nil
		})
	})
	return out
}
