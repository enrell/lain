// Package localplay launches a media player on the server's own machine
// for loopback browser sessions (D-072). The spawned process reads the
// library files directly — no stream URL and no token leave the server —
// while a monitor goroutine reports measured progress in-process through
// a caller-supplied sink. mpv runs the shared Lua script (MPVScript);
// VLC is sampled over a private RC unix socket.
package localplay

import (
	"bytes"
	_ "embed"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"math"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/enrell/lain/internal/contracts"
)

// MPVScript is the progress-reporting Lua script the CLI and the server
// both feed to mpv. It writes one JSON state file per playlist index and
// applies the preferred-language track policy (D-071).
//
//go:embed mpv_progress.lua
var MPVScript string

var (
	// ErrNoPlayer means no supported player binary exists in PATH or no
	// graphical session is available to open a window in.
	ErrNoPlayer = errors.New("no local player available")
	// ErrBusy means another local playback is already running; one screen
	// fits one player at a time.
	ErrBusy = errors.New("a local playback session is already running")
)

// Entry is one playlist item.
type Entry struct {
	ItemID  string
	Path    string  // absolute media file path
	Resume  float64 // seconds; under 5 or non-finite starts from the top
	SubsOff bool    // VLC only: subtitles disabled after the track loads
}

// PlayRequest describes one local playback session. Save is invoked from
// the monitor goroutine and must be safe for concurrent-free sequential
// use. Done fires exactly once after the player exits; a nil Done is
// legal.
type PlayRequest struct {
	Player   string // "mpv" or "vlc"; "" picks the first available
	Entries  []Entry
	Language string
	Save     func(itemID string, positionSec, durationSec float64)
	Done     func(err error)
}

// Manager spawns and supervises local player processes.
type Manager struct {
	// LookPath and Getenv are test seams; nil uses the real calls.
	LookPath func(string) (string, error)
	Getenv   func(string) string
	Log      *slog.Logger

	mu      sync.Mutex
	running bool
	procs   map[*exec.Cmd]struct{}
	wg      sync.WaitGroup
}

func New() *Manager { return &Manager{} }

func (m *Manager) lookPath(bin string) (string, error) {
	if m.LookPath != nil {
		return m.LookPath(bin)
	}
	return exec.LookPath(bin)
}

func (m *Manager) getenv(key string) string {
	if m.Getenv != nil {
		return m.Getenv(key)
	}
	return os.Getenv(key)
}

// display reports whether a graphical session exists for the player to
// open a window in.
func (m *Manager) display() bool {
	return m.getenv("DISPLAY") != "" || m.getenv("WAYLAND_DISPLAY") != ""
}

// Players lists the supported players that can launch right now.
func (m *Manager) Players() []string {
	if !m.display() {
		return nil
	}
	var out []string
	for _, p := range []string{"mpv", "vlc"} {
		if _, err := m.lookPath(p); err == nil {
			out = append(out, p)
		}
	}
	return out
}

func (m *Manager) acquire() bool {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.running {
		return false
	}
	m.running = true
	m.procs = map[*exec.Cmd]struct{}{}
	return true
}

// reset clears the running flag after a pre-spawn failure.
func (m *Manager) reset() {
	m.mu.Lock()
	m.running = false
	m.procs = nil
	m.mu.Unlock()
}

func (m *Manager) release(cmd *exec.Cmd) {
	m.mu.Lock()
	delete(m.procs, cmd)
	m.running = len(m.procs) > 0
	m.mu.Unlock()
}

func (m *Manager) track(cmd *exec.Cmd) {
	m.mu.Lock()
	m.procs[cmd] = struct{}{}
	m.mu.Unlock()
}

// Close kills every player still running and waits for their monitors.
func (m *Manager) Close() {
	m.mu.Lock()
	procs := make([]*exec.Cmd, 0, len(m.procs))
	for c := range m.procs {
		procs = append(procs, c)
	}
	m.mu.Unlock()
	for _, c := range procs {
		if c.Process != nil {
			_ = c.Process.Kill()
		}
	}
	m.wg.Wait()
}

func (m *Manager) log() *slog.Logger {
	if m.Log != nil {
		return m.Log
	}
	return slog.New(slog.DiscardHandler)
}

// Play validates the request, writes the playlist plus resume state into
// a private temp dir, spawns the player, and returns. Progress flows
// through req.Save until the process exits; req.Done then receives the
// outcome — nil when at least one position was measured, an error
// otherwise.
func (m *Manager) Play(req PlayRequest) error {
	if len(req.Entries) == 0 {
		return errors.New("empty playlist")
	}
	if req.Save == nil {
		return errors.New("progress sink required")
	}
	player := req.Player
	if player == "" {
		if players := m.Players(); len(players) > 0 {
			player = players[0]
		}
	}
	if player != "mpv" && player != "vlc" {
		return fmt.Errorf("unsupported player %q", player)
	}
	bin, err := m.lookPath(player)
	if err != nil {
		return fmt.Errorf("%s not found in PATH", player)
	}
	if !m.display() {
		return ErrNoPlayer
	}
	if !m.acquire() {
		return ErrBusy
	}
	dir, err := os.MkdirTemp("", "lain-play-*")
	if err != nil {
		m.reset()
		return err
	}
	var playlist strings.Builder
	playlist.WriteString("#EXTM3U\n")
	resumes := map[string]float64{}
	for i, e := range req.Entries {
		if e.Resume >= 5 && finite(e.Resume) {
			resumes[strconv.Itoa(i)] = e.Resume
		}
		playlist.WriteString(fmt.Sprintf("#EXTINF:-1,lain-episode-%06d\n%s\n", i, e.Path))
	}
	if err := os.WriteFile(filepath.Join(dir, "episodes.m3u"), []byte(playlist.String()), 0o600); err != nil {
		os.RemoveAll(dir)
		m.reset()
		return err
	}
	raw, err := json.Marshal(resumes)
	if err != nil {
		os.RemoveAll(dir)
		m.reset()
		return err
	}
	if err := os.WriteFile(filepath.Join(dir, "resume.json"), raw, 0o600); err != nil {
		os.RemoveAll(dir)
		m.reset()
		return err
	}
	var cmd *exec.Cmd
	if player == "mpv" {
		script := filepath.Join(dir, "lain-progress.lua")
		if err := os.WriteFile(script, []byte(MPVScript), 0o600); err != nil {
			os.RemoveAll(dir)
			m.reset()
			return err
		}
		args := []string{"--script=" + script,
			"--script-opt=lain-state_dir=" + dir,
			"--script-opt=lain-resume_file=" + filepath.Join(dir, "resume.json")}
		if req.Language != "" {
			args = append(args, "--script-opt=lain-language="+req.Language)
		}
		cmd = exec.Command(bin, append(args, "--playlist="+filepath.Join(dir, "episodes.m3u"))...)
	} else {
		socket := filepath.Join(dir, "vlc.sock")
		args := []string{"--no-one-instance", "--play-and-exit",
			"--extraintf=rc", "--rc-unix=" + socket}
		if req.Language != "" {
			args = append(args, "--audio-language="+req.Language, "--sub-language="+req.Language)
		}
		if at := resumes["0"]; at > 0 {
			args = append(args, "--start-time="+strconv.FormatFloat(at, 'f', 0, 64))
		}
		cmd = exec.Command(bin, append(args, filepath.Join(dir, "episodes.m3u"))...)
	}
	var stderr cappedBuffer
	cmd.Stdout, cmd.Stderr = &stderr, &stderr
	if err := cmd.Start(); err != nil {
		os.RemoveAll(dir)
		m.reset()
		return err
	}
	m.track(cmd)
	m.wg.Add(1)
	go m.monitor(cmd, player, dir, req, &stderr)
	return nil
}

// cappedBuffer keeps only the head of player output for diagnostics.
type cappedBuffer struct{ b bytes.Buffer }

func (w *cappedBuffer) Write(p []byte) (int, error) {
	if room := 8192 - w.b.Len(); room > 0 {
		w.b.Write(p[:min(len(p), room)])
	}
	return len(p), nil
}

func (w *cappedBuffer) String() string { return w.b.String() }

func finite(n float64) bool { return !math.IsNaN(n) && !math.IsInf(n, 0) && n >= 0 }

func (m *Manager) monitor(cmd *exec.Cmd, player, dir string, req PlayRequest, stderr *cappedBuffer) {
	defer m.wg.Done()
	defer m.release(cmd)
	defer os.RemoveAll(dir)
	done := make(chan error, 1)
	go func() { done <- cmd.Wait() }()
	ticker := time.NewTicker(time.Second)
	defer ticker.Stop()
	saved := map[int][2]float64{}
	observed := false
	current := -1
	var lastPos, lastDur float64
	sample := func() {
		if player == "mpv" {
			for i := range req.Entries {
				pos, dur, ok := readState(filepath.Join(dir, strconv.Itoa(i)+".json"))
				if !ok {
					continue
				}
				observed = true
				m.save(req, saved, i, pos, dur)
			}
			return
		}
		index, err := vlcQueueIndex(filepath.Join(dir, "vlc.sock"), len(req.Entries))
		if err != nil || index < 0 {
			return
		}
		if index != current {
			current = index
			if req.Entries[index].SubsOff {
				if _, err := vlcRC(filepath.Join(dir, "vlc.sock"), "strack -1\n"); err != nil {
					m.log().Debug("localplay: could not disable subtitles", "err", err)
				}
			}
			if index > 0 && req.Entries[index].Resume >= 5 {
				if err := vlcSeek(filepath.Join(dir, "vlc.sock"), req.Entries[index].Resume); err != nil {
					m.log().Debug("localplay: could not resume episode", "err", err)
				}
				return
			}
		}
		pos, dur, err := vlcPosition(filepath.Join(dir, "vlc.sock"))
		if err != nil || dur <= 0 {
			return
		}
		lastPos, lastDur, observed = pos, dur, true
		m.save(req, saved, index, pos, dur)
	}
	for {
		select {
		case err := <-done:
			sample()
			if player == "vlc" && current >= 0 && observed {
				m.save(req, saved, current, lastPos, lastDur)
			}
			if req.Done == nil {
				return
			}
			if !observed {
				detail := strings.TrimSpace(stderr.String())
				if detail == "" {
					req.Done(fmt.Errorf("%s closed without measurable progress", player))
				} else {
					req.Done(fmt.Errorf("%s closed without measurable progress: %s", player, detail))
				}
				return
			}
			if err != nil {
				req.Done(fmt.Errorf("%s exited: %w", player, err))
				return
			}
			req.Done(nil)
			return
		case <-ticker.C:
			sample()
		}
	}
}

func (m *Manager) save(req PlayRequest, saved map[int][2]float64, index int, pos, dur float64) {
	if index < 0 || index >= len(req.Entries) || !finite(pos) || !finite(dur) || dur <= 0 {
		return
	}
	if old, ok := saved[index]; ok && old[0] == pos && old[1] == dur {
		return
	}
	saved[index] = [2]float64{pos, dur}
	req.Save(req.Entries[index].ItemID, pos, dur)
}

type stateFile struct {
	PositionSec float64 `json:"position_sec"`
	DurationSec float64 `json:"duration_sec"`
}

func readState(path string) (float64, float64, bool) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return 0, 0, false
	}
	var s stateFile
	if err := json.Unmarshal(raw, &s); err != nil {
		return 0, 0, false
	}
	return s.PositionSec, s.DurationSec, true
}

var languageAliases = map[string]string{
	"fre": "fra", "ger": "deu", "chi": "zho", "dut": "nld",
	"gre": "ell", "rum": "ron", "cze": "ces", "slo": "slk",
	"per": "fas", "may": "msa", "alb": "sqi", "arm": "hye",
	"baq": "eus", "bur": "mya", "ice": "isl", "mac": "mkd",
}

// SubtitleOff applies the preferred-language policy (D-071) to a probed
// stream list: a matching audio track wins and subtitles hide; without
// matching audio a matching subtitle stays selectable; with tagged
// streams but no match at all, subs hide rather than show an unwanted
// language. Untagged metadata returns false so the player's own defaults
// stay in charge.
func SubtitleOff(streams []contracts.MediaStream, language string) bool {
	if language == "" {
		return false
	}
	known := false
	for _, s := range streams {
		if (s.Type == "audio" || s.Type == "subtitle") && s.Language != "" {
			known = true
		}
		if s.Type == "audio" && SameLanguage(s.Language, language) {
			return true
		}
	}
	if !known {
		return false
	}
	for _, s := range streams {
		if s.Type == "subtitle" && SameLanguage(s.Language, language) {
			return false
		}
	}
	return true
}

// SameLanguage reports whether a stream's language tag matches the
// account preference, normalizing ISO 639-2 bibliographic codes.
func SameLanguage(actual, preferred string) bool {
	actual = strings.ToLower(strings.Split(actual, "-")[0])
	preferred = strings.ToLower(preferred)
	if mapped := languageAliases[actual]; mapped != "" {
		actual = mapped
	}
	if mapped := languageAliases[preferred]; mapped != "" {
		preferred = mapped
	}
	return actual != "" && actual == preferred
}
