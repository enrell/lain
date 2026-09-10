package main

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"time"

	"github.com/enrell/lain/internal/backup"
)

// cmdBackup snapshots the database. Online first: when a saved login
// exists and the server answers, the snapshot streams from one server
// read transaction (works mid-scan, mid-stream). Otherwise it falls
// back to the offline path, which needs the server stopped (bbolt
// holds an exclusive file lock, so a second opener would hang — the
// offline open times out fast with a clear error instead).
func cmdBackup(args []string) error {
	dataDir := flag(args, "data-dir", defaultDataDir())
	out := flag(args, "out", "backups")
	if _, err := cmdBackupOnline(args, out); err == nil {
		return nil
	} else if !os.IsNotExist(err) {
		fmt.Fprintf(os.Stderr, "online backup unavailable (%v); trying offline\n", err)
	}
	dir, err := backup.Create(dataDir, out)
	if err != nil {
		return err
	}
	m, err := backup.Validate(dir)
	if err != nil {
		return err
	}
	fmt.Printf("backup (offline): %s (%d users, %d libraries, %d items)\n", dir, m.Users, m.Libraries, m.Items)
	return nil
}

func cmdBackupOnline(args []string, out string) (string, error) {
	cfg, err := loadConfig()
	if err != nil || cfg.Token == "" {
		return "", os.ErrNotExist
	}
	if srv := flag(args, "server", ""); srv != "" {
		cfg.Server = srv
	}
	client := newAPIClient(cfg.Server, cfg.Token)
	health, err := http.Get(cfg.Server + "/health")
	if err != nil {
		return "", err
	}
	health.Body.Close()
	if health.StatusCode != 200 {
		return "", fmt.Errorf("server status %d", health.StatusCode)
	}
	dir := out
	if fi, err := os.Stat(out); err == nil && fi.IsDir() {
		dir = filepath.Join(out, "lain-backup-"+time.Now().Format("20060102-150405"))
	}
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return "", err
	}
	fail := func(err error) (string, error) {
		os.RemoveAll(dir)
		return "", err
	}
	// Database snapshot stream.
	req, err := http.NewRequest("GET", cfg.Server+"/api/admin/backup", nil)
	if err != nil {
		return fail(err)
	}
	req.Header.Set("Authorization", "Bearer "+cfg.Token)
	resp, err := client.http.Do(req)
	if err != nil {
		return fail(err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != 200 {
		raw, _ := io.ReadAll(resp.Body)
		return fail(fmt.Errorf("backup stream: %d %s", resp.StatusCode, raw))
	}
	dst, err := os.OpenFile(filepath.Join(dir, backup.DBFile), os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o600)
	if err != nil {
		return fail(err)
	}
	if _, err := io.Copy(dst, resp.Body); err != nil {
		dst.Close()
		return fail(err)
	}
	dst.Close()
	// Composition rides the plugins inspect endpoint.
	var plugins struct {
		Composition any `json:"composition"`
	}
	if err := client.get("/api/plugins", nil, &plugins); err != nil {
		return fail(err)
	}
	comp := map[string]any{"version": 1, "bindings": map[string]any{}}
	if plugins.Composition != nil {
		comp["bindings"] = plugins.Composition
	}
	raw, _ := json.MarshalIndent(comp, "", "  ")
	if err := os.WriteFile(filepath.Join(dir, backup.CompositionFile), append(raw, '\n'), 0o600); err != nil {
		return fail(err)
	}
	m, err := backup.WriteManifest(filepath.Join(dir, backup.DBFile), dir)
	if err != nil {
		return fail(err)
	}
	if _, err := backup.Validate(dir); err != nil {
		return fail(err)
	}
	fmt.Printf("backup (online): %s (%d users, %d libraries, %d items)\n", dir, m.Users, m.Libraries, m.Items)
	return dir, nil
}

func cmdRestore(args []string) error {
	pos := positional(args)
	if len(pos) != 1 {
		return errUsage("usage: lain restore BACKUP-DIR [--data-dir DIR]")
	}
	dataDir := flag(args, "data-dir", defaultDataDir())
	if err := backup.Restore(pos[0], dataDir); err != nil {
		return err
	}
	fmt.Printf("restored %s into %s\n", pos[0], dataDir)
	return nil
}
