// Command lain is the Lain media server: setup, serve, diagnose and
// inspect the replaceable composition.
package main

import (
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"time"

	"github.com/enrell/lain/internal/gateway"
	"github.com/enrell/lain/internal/matrix"
	"github.com/enrell/lain/internal/plugins/userstate"
	"github.com/enrell/lain/internal/store"
)

const version = "0.1.0-dev"

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
	case "version", "--version", "-V":
		fmt.Println("lain " + version)
	default:
		usage()
		os.Exit(2)
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, "error: "+err.Error())
		os.Exit(1)
	}
}

func usage() {
	fmt.Fprintln(os.Stderr, `usage: lain <command> [flags]

  serve     run the server (flags: --data-dir, --port)
  doctor    diagnose runtime + matrix environment (flags: --data-dir, --matrix-bin)
  plugins   list registered providers + composition (flags: --data-dir)
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

func openUserstate(dataDir string) (*userstate.Service, error) {
	st, err := store.New(dataDir)
	if err != nil {
		return nil, err
	}
	return userstate.New(st)
}

func cmdServe(args []string) error {
	dataDir := flag(args, "data-dir", defaultDataDir())
	port := flag(args, "port", "9360")
	ustate, err := openUserstate(dataDir)
	if err != nil {
		return err
	}
	srv, err := gateway.New(dataDir, version, ustate)
	if err != nil {
		return err
	}
	httpSrv := &http.Server{
		Addr:         "127.0.0.1:" + port,
		Handler:      srv.Handler(),
		ReadTimeout:  15 * time.Second,
		WriteTimeout: 0, // streams are long-lived; timeouts would cut playback
	}
	fmt.Printf("lain %s on http://127.0.0.1:%s (data %s)\n", version, port, dataDir)
	return httpSrv.ListenAndServe()
}

func cmdDoctor(args []string) error {
	dataDir := flag(args, "data-dir", defaultDataDir())
	matrixBin := flag(args, "matrix-bin", "matrix-managed")
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
	ustate, err := openUserstate(dataDir)
	if err != nil {
		return err
	}
	srv, err := gateway.New(dataDir, version, ustate)
	if err != nil {
		return err
	}
	out := map[string]any{
		"composition": srv.Registry().Composition().View(),
		"providers":   srv.Registry().Providers(),
	}
	raw, _ := json.MarshalIndent(out, "", "  ")
	fmt.Println(string(raw))
	return nil
}
