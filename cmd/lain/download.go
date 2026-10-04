// Offline copies: `lain download`.
//
// The CLI keeps library items on this machine so they play without the
// server (docs/slices/reading-downloads.md, P-6). Copies come from the
// same authenticated, Range-capable stream endpoint players use, so an
// interrupted download resumes where it stopped. The store is bounded
// by configurable limits; `lain watch` prefers a finished local copy,
// and `lain download play` works with the server unreachable, keeping
// the position until `lain download sync` can push it.
package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"github.com/enrell/lain/internal/downloads"
	"github.com/enrell/lain/internal/localplay"
	"github.com/enrell/lain/internal/offline"
)

const downloadUsage = `usage: lain download <command>

  add <query> [--id ID] [--rest] [--pick N] [--no-run]
                 queue an item (--rest: it and every later episode/chapter)
                 and download now; Ctrl+C pauses, nothing is lost
  run            download everything queued, resuming partial files
  list           show the offline store and its usage
  pause|resume <id>
  cancel <id>    stop a download and delete its partial bytes
  rm <id> [--force]
                 delete a local copy (--force drops unsynced progress)
  play <id|title> [--player mpv|vlc]
                 play or open a local copy without the server
  sync           push offline progress and refresh watched state
  gc [--watched] delete orphaned partials and forgotten files;
                 --watched also deletes every watched, synced copy
  config [--dir DIR] [--max SIZE|0] [--min-free SIZE|0] [--evict-watched on|off]
                 show or change where copies live and how much they may use

<id> accepts a unique id prefix or a unique part of the title.`

func cmdDownload(args []string) error {
	if len(args) == 0 {
		fmt.Fprintln(os.Stderr, downloadUsage)
		return fmt.Errorf("missing download command")
	}
	sub, rest := args[0], args[1:]
	switch sub {
	case "config":
		return downloadConfig(rest)
	case "add":
		return downloadAdd(rest)
	case "run":
		return withStore(func(s *offline.Store) error {
			cfg, client, err := loggedIn()
			if err != nil {
				return err
			}
			return runOfflineQueue(s, cfg, client)
		})
	case "list", "ls":
		return downloadList()
	case "pause", "resume", "cancel", "rm":
		return downloadEntryCommand(sub, rest)
	case "play":
		return downloadPlay(rest)
	case "sync":
		return withStore(func(s *offline.Store) error {
			cfg, client, err := loggedIn()
			if err != nil {
				return err
			}
			return syncOffline(s, cfg, client, os.Stdout)
		})
	case "gc":
		return withStore(func(s *offline.Store) error {
			if cfg, client, err := loggedIn(); err == nil {
				if err := syncOffline(s, cfg, client, io.Discard); err != nil {
					fmt.Fprintf(os.Stderr, "warning: could not refresh watched state: %v\n", err)
				}
			}
			rep, err := s.GC(hasFlag(rest, "watched"))
			if err != nil {
				return err
			}
			for _, e := range rep.Evicted {
				fmt.Printf("removed watched copy: %s\n", e.Label())
			}
			fmt.Printf("freed %s (%d orphaned partials, %d forgotten files, %d watched copies)\n",
				downloads.HumanBytes(rep.FreedBytes), rep.OrphanParts, rep.Vanished, len(rep.Evicted))
			return nil
		})
	case "help", "--help", "-h":
		fmt.Println(downloadUsage)
		return nil
	}
	fmt.Fprintln(os.Stderr, downloadUsage)
	return fmt.Errorf("unknown download command %q", sub)
}

// offlineConfigPath is ~/.config/lain/offline.json, next to the login.
func offlineConfigPath() (string, error) {
	p, err := configPath()
	if err != nil {
		return "", err
	}
	return filepath.Join(filepath.Dir(p), "offline.json"), nil
}

func dataHome() string {
	if d := os.Getenv("XDG_DATA_HOME"); d != "" && filepath.IsAbs(d) {
		return d
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return os.TempDir()
	}
	return filepath.Join(home, ".local", "share")
}

func loadOfflineConfig() (offline.Config, error) {
	cfg := offline.DefaultConfig(dataHome())
	p, err := offlineConfigPath()
	if err != nil {
		return cfg, err
	}
	raw, err := os.ReadFile(p)
	if os.IsNotExist(err) {
		return cfg, nil
	}
	if err != nil {
		return cfg, err
	}
	if err := json.Unmarshal(raw, &cfg); err != nil {
		return cfg, fmt.Errorf("%s: %w", p, err)
	}
	return cfg, nil
}

func saveOfflineConfig(cfg offline.Config) error {
	p, err := offlineConfigPath()
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(p), 0o700); err != nil {
		return err
	}
	raw, _ := json.MarshalIndent(cfg, "", "  ")
	tmp := p + ".tmp"
	if err := os.WriteFile(tmp, append(raw, '\n'), 0o600); err != nil {
		return err
	}
	return os.Rename(tmp, p)
}

// withStore opens the store under its lock and saves it afterwards.
func withStore(f func(*offline.Store) error) error {
	cfg, err := loadOfflineConfig()
	if err != nil {
		return err
	}
	s, err := offline.Open(cfg)
	if err != nil {
		return err
	}
	unlock, err := s.Lock()
	if err != nil {
		return err
	}
	defer unlock()
	ferr := f(s)
	if err := s.Save(); err != nil && ferr == nil {
		return err
	}
	return ferr
}

func loggedIn() (clientConfig, *apiClient, error) {
	cfg, err := loadConfig()
	if err != nil || cfg.Token == "" {
		return cfg, nil, fmt.Errorf("not logged in (run `lain login`)")
	}
	return cfg, newAPIClient(cfg.Server, cfg.Token), nil
}

// parseLimit accepts "0" (no limit) or a byte size with a binary suffix.
func parseLimit(raw string) (int64, error) {
	if strings.TrimSpace(raw) == "0" {
		return 0, nil
	}
	return parseByteSize(raw)
}

func downloadConfig(args []string) error {
	cfg, err := loadOfflineConfig()
	if err != nil {
		return err
	}
	changed := false
	if v := flag(args, "dir", ""); v != "" {
		abs, err := filepath.Abs(v)
		if err != nil {
			return err
		}
		cfg.Dir, changed = abs, true
	}
	for _, f := range []struct {
		name string
		dst  *int64
	}{{"max", &cfg.MaxBytes}, {"min-free", &cfg.MinFreeBytes}} {
		if v := flag(args, f.name, ""); v != "" {
			n, err := parseLimit(v)
			if err != nil {
				return fmt.Errorf("--%s %w", f.name, err)
			}
			*f.dst, changed = n, true
		}
	}
	switch v := flag(args, "evict-watched", ""); v {
	case "":
	case "on", "true", "yes":
		cfg.EvictWatched, changed = true, true
	case "off", "false", "no":
		cfg.EvictWatched, changed = false, true
	default:
		return fmt.Errorf("--evict-watched takes on or off")
	}
	if changed {
		if err := saveOfflineConfig(cfg); err != nil {
			return err
		}
	}
	limit := func(n int64) string {
		if n == 0 {
			return "none"
		}
		return downloads.HumanBytes(n)
	}
	fmt.Printf("dir            %s\nmax            %s\nmin free       %s\nevict watched  %v\n",
		cfg.Dir, limit(cfg.MaxBytes), limit(cfg.MinFreeBytes), cfg.EvictWatched)
	return nil
}

// catalogEntry is the catalog read a download needs (size and file name).
type catalogEntry struct {
	apiItem
	Size int64 `json:"size"`
}

func downloadAdd(args []string) error {
	cfg, client, err := loggedIn()
	if err != nil {
		return err
	}
	var start apiItem
	if id := flag(args, "id", ""); id != "" {
		if err := client.get("/api/catalog/"+url.PathEscape(id), nil, &start); err != nil {
			return err
		}
	} else {
		query := strings.TrimSpace(strings.Join(positional(args), " "))
		if query == "" {
			return fmt.Errorf("name what to download (a search query or --id)")
		}
		start, err = resolveQueryArgs(client, query, os.Stdin, os.Stdout, args)
		if err != nil {
			return err
		}
	}
	items := []apiItem{start}
	if hasFlag(args, "rest") {
		if items, err = episodeQueue(client, start); err != nil {
			return err
		}
	}
	return withStore(func(s *offline.Store) error {
		for _, it := range items {
			var full catalogEntry
			if err := client.get("/api/catalog/"+url.PathEscape(it.ID), nil, &full); err != nil {
				return err
			}
			e, err := s.Add(offline.Entry{
				ItemID: full.ID, Server: cfg.Server, Title: full.Title, Kind: full.Kind,
				Season: full.Season, Episode: full.Episode, Size: full.Size,
			}, filepath.Base(full.FilePath))
			if err != nil {
				return err
			}
			fmt.Printf("queued: %s (%s)\n", e.Label(), downloads.HumanBytes(full.Size))
		}
		if err := s.Save(); err != nil {
			return err
		}
		if hasFlag(args, "no-run") {
			return nil
		}
		return runOfflineQueue(s, cfg, client)
	})
}

// streamClient has no overall timeout (files are large) but bounds the
// connection phases so a dead server fails fast.
func streamClient() *http.Client {
	return &http.Client{Transport: &http.Transport{
		Proxy:                 http.ProxyFromEnvironment,
		DialContext:           (&net.Dialer{Timeout: 15 * time.Second}).DialContext,
		TLSHandshakeTimeout:   15 * time.Second,
		ResponseHeaderTimeout: 60 * time.Second,
	}}
}

// runOfflineQueue downloads every queued entry in order. Ctrl+C pauses
// the current one (its bytes stay) and stops the run.
func runOfflineQueue(s *offline.Store, cfg clientConfig, client *apiClient) error {
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt)
	defer stop()
	return runOfflineQueueCtx(ctx, s, cfg, streamClient(), os.Stdout)
}

func runOfflineQueueCtx(ctx context.Context, s *offline.Store, cfg clientConfig, hc *http.Client, out io.Writer) error {
	var failed int
	for _, e := range s.List() {
		if e.State != offline.Queued {
			continue
		}
		if e.Server != "" && e.Server != cfg.Server {
			fmt.Fprintf(out, "skipped: %s belongs to %s\n", e.Label(), e.Server)
			continue
		}
		label := e.Label()
		last := time.Time{}
		got, err := s.Fetch(ctx, hc, e.ItemID, offline.FetchInput{
			URL:    cfg.Server + "/api/items/" + url.PathEscape(e.ItemID) + "/stream",
			Header: http.Header{"Authorization": {"Bearer " + cfg.Token}},
			Progress: func(done, total int64) {
				if time.Since(last) < 250*time.Millisecond && done != total {
					return
				}
				last = time.Now()
				fmt.Fprintf(out, "\r%s  %s", label, progressText(done, total))
			},
			Evicted: func(v offline.Entry) {
				fmt.Fprintf(out, "\nremoved watched copy to make room: %s\n", v.Label())
			},
		})
		fmt.Fprintln(out)
		switch {
		case errors.Is(err, context.Canceled):
			fmt.Fprintf(out, "paused: %s (%s kept; `lain download run` resumes)\n", label, downloads.HumanBytes(got.Bytes))
			return nil
		case err != nil:
			failed++
			fmt.Fprintf(out, "failed: %s: %v\n", label, err)
			if c := downloads.CodeOf(err); c == downloads.CodeQuota || c == downloads.CodeDiskFull {
				fmt.Fprintln(out, "hint: `lain download gc --watched` frees watched copies; `lain download config --max` raises the limit")
				return fmt.Errorf("offline store is full")
			}
		default:
			fmt.Fprintf(out, "done: %s\n", label)
		}
	}
	if failed > 0 {
		return fmt.Errorf("%d download(s) failed; `lain download resume <id>` retries", failed)
	}
	return nil
}

func progressText(done, total int64) string {
	if total > 0 {
		return fmt.Sprintf("%5.1f%%  %s / %s", float64(done)*100/float64(total), downloads.HumanBytes(done), downloads.HumanBytes(total))
	}
	return downloads.HumanBytes(done)
}

func downloadList() error {
	cfg, err := loadOfflineConfig()
	if err != nil {
		return err
	}
	s, err := offline.Open(cfg)
	if err != nil {
		return err
	}
	printOfflineList(s, os.Stdout)
	return nil
}

func printOfflineList(s *offline.Store, w io.Writer) {
	entries := s.List()
	if len(entries) == 0 {
		fmt.Fprintln(w, "offline store is empty (`lain download add <query>`)")
	}
	for _, e := range entries {
		size := e.Size
		if size <= 0 {
			size = e.Bytes
		}
		pct := "    "
		if e.State != offline.Done && size > 0 {
			pct = fmt.Sprintf("%3d%%", e.Bytes*100/size)
		}
		marks := ""
		if e.Watched {
			marks += " watched"
		}
		if e.Pending != nil {
			marks += " unsynced"
		}
		fmt.Fprintf(w, "%-8s %s %10s  %-8.8s  %s%s\n", e.State, pct, downloads.HumanBytes(size), e.ItemID, e.Label(), marks)
	}
	cfg := s.Config()
	limit := "no limit"
	if cfg.MaxBytes > 0 {
		limit = downloads.HumanBytes(cfg.MaxBytes)
	}
	fmt.Fprintf(w, "used %s of %s in %s\n", downloads.HumanBytes(s.Used()), limit, cfg.Dir)
}

// findEntry resolves an id prefix or a unique title fragment.
func findEntry(s *offline.Store, ref string) (offline.Entry, error) {
	ref = strings.TrimSpace(ref)
	if ref == "" {
		return offline.Entry{}, fmt.Errorf("name an entry (id prefix or title)")
	}
	if e, ok := s.Get(ref); ok {
		return e, nil
	}
	var hits []offline.Entry
	low := strings.ToLower(ref)
	for _, e := range s.List() {
		if strings.HasPrefix(e.ItemID, ref) || strings.Contains(strings.ToLower(e.Label()), low) {
			hits = append(hits, e)
		}
	}
	switch len(hits) {
	case 0:
		return offline.Entry{}, fmt.Errorf("nothing in the offline store matches %q", ref)
	case 1:
		return hits[0], nil
	}
	names := make([]string, 0, len(hits))
	for _, h := range hits {
		names = append(names, h.ItemID[:min(8, len(h.ItemID))]+" "+h.Label())
	}
	return offline.Entry{}, fmt.Errorf("%q matches %d entries:\n  %s", ref, len(hits), strings.Join(names, "\n  "))
}

func downloadEntryCommand(sub string, args []string) error {
	ref := strings.Join(positional(args), " ")
	return withStore(func(s *offline.Store) error {
		e, err := findEntry(s, ref)
		if err != nil {
			return err
		}
		switch sub {
		case "pause":
			if _, err := s.Pause(e.ItemID); err != nil {
				return err
			}
			fmt.Printf("paused: %s\n", e.Label())
		case "resume":
			if _, err := s.Resume(e.ItemID); err != nil {
				return err
			}
			fmt.Printf("queued: %s (`lain download run` downloads it)\n", e.Label())
		case "cancel":
			if e.State == offline.Done {
				return fmt.Errorf("%s is already downloaded; `lain download rm` deletes the copy", e.Label())
			}
			freed, err := s.Remove(e.ItemID, true)
			if err != nil {
				return err
			}
			fmt.Printf("canceled: %s (freed %s)\n", e.Label(), downloads.HumanBytes(freed))
		case "rm":
			freed, err := s.Remove(e.ItemID, hasFlag(args, "force"))
			if err != nil {
				return err
			}
			fmt.Printf("removed: %s (freed %s)\n", e.Label(), downloads.HumanBytes(freed))
		}
		return nil
	})
}

// syncOffline pushes unsynced offline positions, then refreshes the
// watched flag of every finished copy so eviction knows what is safe.
func syncOffline(s *offline.Store, cfg clientConfig, client *apiClient, out io.Writer) error {
	for _, e := range s.Pending() {
		if e.Server != "" && e.Server != cfg.Server {
			continue
		}
		p := e.Pending
		if err := client.put("/api/items/"+url.PathEscape(e.ItemID)+"/progress",
			map[string]any{"position_sec": p.PositionSec, "duration_sec": p.DurationSec, "completed": p.Completed}, nil); err != nil {
			return err
		}
		s.ClearPending(e.ItemID)
		fmt.Fprintf(out, "synced: %s at %.0fs\n", e.Label(), p.PositionSec)
	}
	for _, e := range s.List() {
		if e.State != offline.Done || (e.Server != "" && e.Server != cfg.Server) {
			continue
		}
		var p apiProgress
		if err := client.get("/api/items/"+url.PathEscape(e.ItemID)+"/progress", nil, &p); err != nil {
			return err
		}
		s.SetWatched(e.ItemID, p.Completed)
	}
	return nil
}

// localCopyPath is the hook `lain watch` uses to prefer an offline copy
// over the network stream. Failures mean "no local copy".
var localCopyPath = func(itemID string) string {
	cfg, err := loadOfflineConfig()
	if err != nil {
		return ""
	}
	if _, err := os.Stat(filepath.Join(cfg.Dir, "index.json")); err != nil {
		return ""
	}
	s, err := offline.Open(cfg)
	if err != nil {
		return ""
	}
	return s.LocalPath(itemID)
}

// downloadPlay opens a local copy with no server involved. Video goes to
// the configured player with the offline position; the new position is
// kept and pushed immediately when the server answers, else on sync.
func downloadPlay(args []string) error {
	return withStore(func(s *offline.Store) error {
		e, err := findEntry(s, strings.Join(positional(args), " "))
		if err != nil {
			return err
		}
		path := s.LocalPath(e.ItemID)
		if path == "" {
			return fmt.Errorf("%s has no finished local copy", e.Label())
		}
		ccfg, _ := loadConfig()
		player := flag(args, "player", ccfg.Player)
		if player == "" {
			player = "mpv"
		}
		if player != "mpv" && player != "vlc" {
			return fmt.Errorf("unsupported player %q (choose mpv or vlc)", player)
		}
		if e.Kind == "comic" || e.Kind == "manga" {
			return fmt.Errorf("%s is an archive; open %s in a comic reader (the web reader needs the server)", e.Label(), path)
		}
		resume := 0.0
		if e.Pending != nil && !e.Pending.Completed {
			resume = e.Pending.PositionSec
		}
		s.Touch(e.ItemID)
		pos, dur, ok, runErr := playLocalFile(player, e.Label(), path, resume)
		if !ok {
			if runErr != nil {
				return fmt.Errorf("%s exited before progress was recorded: %w", player, runErr)
			}
			return nil
		}
		s.RecordOffline(e.ItemID, offline.Progress{PositionSec: pos, DurationSec: dur, Completed: dur > 0 && pos/dur >= 0.95})
		fmt.Printf("kept offline: %.0fs / %.0fs\n", pos, dur)
		if ccfg.Token != "" {
			if err := syncOffline(s, ccfg, newAPIClient(ccfg.Server, ccfg.Token), os.Stdout); err != nil {
				fmt.Println("server unreachable; `lain download sync` pushes it later")
			}
		}
		return playerExitErr(player, runErr)
	})
}

// playLocalFile runs mpv or VLC on a file and measures the position the
// same way `lain watch` does.
func playLocalFile(player, label, path string, resume float64) (pos, dur float64, ok bool, runErr error) {
	bin, err := exec.LookPath(player)
	if err != nil {
		return 0, 0, false, fmt.Errorf("%s not found in PATH", player)
	}
	dir, err := os.MkdirTemp("", "lain-offline-*")
	if err != nil {
		return 0, 0, false, err
	}
	defer os.RemoveAll(dir)
	stateFile := filepath.Join(dir, "progress.json")
	socket := filepath.Join(dir, "vlc.sock")
	var args []string
	if player == "mpv" {
		script := filepath.Join(dir, "lain-progress.lua")
		if err := os.WriteFile(script, []byte(localplay.MPVScript), 0o600); err != nil {
			return 0, 0, false, err
		}
		args = []string{"--script=" + script, "--script-opts=lain-state=" + stateFile, "--title=" + label}
		if resume > 0 {
			args = append(args, "--start="+strconv.FormatFloat(resume, 'f', 0, 64))
		}
	} else {
		args = []string{"--no-one-instance", "--play-and-exit", "--extraintf=rc", "--rc-unix=" + socket, "--meta-title=" + label}
		if resume > 0 {
			args = append(args, "--start-time="+strconv.FormatFloat(resume, 'f', 0, 64))
		}
	}
	cmd := exec.Command(bin, append(args, path)...)
	cmd.Stdin, cmd.Stdout, cmd.Stderr = os.Stdin, os.Stdout, os.Stderr
	fmt.Printf("playing offline: %s\n", label)
	if player == "vlc" {
		return runVLC(cmd, socket)
	}
	runErr = cmd.Run()
	pos, dur, ok = readStateFile(stateFile)
	return pos, dur, ok, runErr
}
