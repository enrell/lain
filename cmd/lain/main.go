// Command lain is the Lain media server: setup, serve, diagnose and
// inspect the replaceable composition.
package main

import (
	"encoding/json"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/gateway"
	"github.com/enrell/lain/internal/matrix"
	"github.com/enrell/lain/internal/plugins/transcode"
)

// version is stamped by release builds:
// go build -ldflags "-X main.version=v0.1.0"
var version = "0.1.0-dev"

func main() {
	if len(os.Args) < 2 {
		usage()
		os.Exit(2)
	}
	var err error
	switch os.Args[1] {
	case "serve":
		err = cmdServe(os.Args[2:])
	case "doctor":
		err = cmdDoctor(os.Args[2:])
	case "plugins":
		err = cmdPlugins(os.Args[2:])
	case "bench":
		err = cmdBench(os.Args[2:])
	case "login":
		err = cmdLogin(os.Args[2:])
	case "logout":
		err = cmdLogout(os.Args[2:])
	case "watch":
		err = cmdWatch(os.Args[2:])
	case "open-url":
		err = cmdOpenURL(os.Args[2:])
	case "install-player-handler":
		err = cmdInstallPlayerHandler()
	case "backup":
		err = cmdBackup(os.Args[2:])
	case "restore":
		err = cmdRestore(os.Args[2:])
	case "version", "--version", "-V":
		fmt.Println("lain " + version)
	default:
		usage()
		os.Exit(2)
	}
	if err != nil {
		printCommandError(os.Stderr, err)
		os.Exit(1)
	}
}

func printCommandError(w io.Writer, err error) { fmt.Fprintln(w, "error: "+err.Error()) }

func usage() {
	fmt.Fprintln(os.Stderr, `usage: lain <command> [flags]

	serve     run the server (flags: --data-dir, --port, --bind, --log-level, --transcode-cache-size, --watch)
  doctor    diagnose runtime + matrix environment (flags: --data-dir, --matrix-bin)
  plugins   list registered providers + composition (flags: --data-dir)
  bench     run a workload benchmark (bench scan --path DIR [--runs N])
  login     save API credentials (flags: --server, --username; or LAIN_PASSWORD)
  logout    forget saved credentials
  watch     play in mpv or VLC ([query] [--next] [--once] [--pick N] [--player mpv|vlc] [--dry-run])
  install-player-handler  register this CLI for external-player links in the web UI
  backup    snapshot the database online (flags: --data-dir, --out)
  restore   restore a backup into an empty data dir (restore DIR [--data-dir])
  version   print version`)
}

func defaultDataDir() string {
	home, err := os.UserHomeDir()
	if err != nil {
		return "./lain-data"
	}
	return filepath.Join(home, ".local", "share", "lain")
}

func flag(args []string, name, def string) string {
	for i := 0; i < len(args); i++ {
		if args[i] == "--"+name && i+1 < len(args) {
			return args[i+1]
		}
		if len(args[i]) > len(name)+3 && args[i][:len(name)+3] == "--"+name+"=" {
			return args[i][len(name)+3:]
		}
	}
	return def
}

// flagOrEnv resolves a flag, then an env var, then a default.
func flagOrEnv(args []string, name, env, def string) string {
	if v := flag(args, name, ""); v != "" {
		return v
	}
	if v := os.Getenv(env); v != "" {
		return v
	}
	return def
}

// envOff reports the explicit-off spellings an env flag accepts.
func envOff(v string) bool { return v == "0" || v == "false" || v == "FALSE" }

func cmdServe(args []string) error {
	dataDir := flag(args, "data-dir", defaultDataDir())
	port := flag(args, "port", "9360")
	bind := flag(args, "bind", "127.0.0.1")
	logLevel := flagOrEnv(args, "log-level", "LAIN_LOG_LEVEL", "info")
	cacheSize := flagOrEnv(args, "transcode-cache-size", "LAIN_TRANSCODE_CACHE_SIZE", "20GiB")
	cacheBytes, err := parseByteSize(cacheSize)
	if err != nil {
		return fmt.Errorf("transcode cache size: %w", err)
	}
	level, err := gateway.ParseLogLevel(logLevel)
	if err != nil {
		return err
	}
	srv, err := gateway.NewWithOptions(dataDir, version, gateway.Options{TranscodeCacheBytes: cacheBytes})
	if err != nil {
		return err
	}
	defer srv.Close()
	logger := gateway.NewAgentLogger(os.Stdout, level, "lain", version)
	srv.SetLogger(logger)
	logger.Info("log level: " + logLevel)
	if envOff(os.Getenv("LAIN_AUTO_ENRICH")) {
		srv.SetAutoEnrich(false)
	}
	// The filesystem watcher reconciles the catalog on delete/add/move
	// events (D-068). inotify cannot see network mounts, so the operator
	// can turn it off and keep the manual scan.
	if !envOff(flagOrEnv(args, "watch", "LAIN_WATCH", "")) {
		startLibraryWatcher(srv.StartWatcher, logger)
	}
	httpSrv := newHTTPServer(bind, port, srv.Handler())
	fmt.Printf("lain %s on http://%s:%s (data %s)\n", version, bind, port, dataDir)
	return httpSrv.ListenAndServe()
}

func startLibraryWatcher(start func() error, logger *slog.Logger) {
	if err := start(); err != nil {
		logger.Warn("library watcher disabled", "err", err.Error())
	}
}

// newHTTPServer builds the listener config. Streams are long-lived, so
// only reads carry a timeout; writes must never cut playback.
func newHTTPServer(bind, port string, h http.Handler) *http.Server {
	return &http.Server{
		Addr:         bind + ":" + port,
		Handler:      h,
		ReadTimeout:  15 * time.Second,
		WriteTimeout: 0,
	}
}

func parseByteSize(raw string) (int64, error) {
	s := strings.TrimSpace(raw)
	multiplier := int64(1)
	for suffix, value := range map[string]int64{
		"KiB": 1 << 10,
		"MiB": 1 << 20,
		"GiB": 1 << 30,
		"TiB": 1 << 40,
	} {
		if strings.HasSuffix(s, suffix) {
			multiplier = value
			s = strings.TrimSpace(strings.TrimSuffix(s, suffix))
			break
		}
	}
	n, err := strconv.ParseInt(s, 10, 64)
	if err != nil || n <= 0 || n > (1<<63-1)/multiplier {
		return 0, fmt.Errorf("must be a positive byte count with optional KiB, MiB, GiB or TiB suffix")
	}
	return n * multiplier, nil
}

func cmdDoctor(args []string) error {
	dataDir := flag(args, "data-dir", defaultDataDir())
	matrixBin := flag(args, "matrix-bin", os.Getenv("LAIN_MATRIX_BIN"))
	rep := map[string]any{
		"version":  version,
		"data_dir": dataDir,
		"matrix":   matrix.Doctor(matrixBin),
	}
	if _, err := os.Stat(dataDir); err == nil {
		rep["data_dir_present"] = true
	}
	for _, bin := range []string{"ffprobe", "ffmpeg"} {
		rep[bin] = binPresent(bin)
	}
	// The transcode report names what the local ffmpeg can actually do:
	// hardware is opt-in and its probe result is never hidden (D-031).
	if binPresent("ffmpeg") {
		rep["transcode"] = transcode.Probe(contracts.DefaultTranscodeSettings())
	}
	raw, _ := json.MarshalIndent(rep, "", "  ")
	fmt.Println(string(raw))
	return nil
}

func binPresent(name string) bool {
	for _, dir := range filepath.SplitList(os.Getenv("PATH")) {
		fi, err := os.Stat(filepath.Join(dir, name))
		if err == nil && !fi.IsDir() && fi.Mode().Perm()&0o111 != 0 {
			return true
		}
	}
	return false
}

func cmdPlugins(args []string) error {
	dataDir := flag(args, "data-dir", defaultDataDir())
	srv, err := gateway.New(dataDir, version)
	if err != nil {
		return err
	}
	defer srv.Close()
	out := map[string]any{
		"composition": srv.Registry().Composition().View(),
		"providers":   srv.Registry().Providers(),
	}
	raw, _ := json.MarshalIndent(out, "", "  ")
	fmt.Println(string(raw))
	return nil
}
