package main

import (
	"fmt"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

var playerExecutable = os.Executable
var playerEvalSymlinks = filepath.EvalSymlinks
var playerUserHome = os.UserHomeDir

// A browser cannot start a local process. Its lain: link carries only an
// origin, item ID and player name; the local CLI supplies its own token.
func parsePlayerURL(raw string, cfg clientConfig) (string, string, error) {
	u, err := url.Parse(raw)
	if err != nil || u.Scheme != "lain" || u.Host != "play" || u.Path != "" || u.Fragment != "" || u.User != nil {
		return "", "", fmt.Errorf("invalid Lain player link")
	}
	q := u.Query()
	if len(q) != 3 || len(q["server"]) != 1 || len(q["id"]) != 1 || len(q["player"]) != 1 {
		return "", "", fmt.Errorf("invalid Lain player link parameters")
	}
	server, err := url.Parse(q.Get("server"))
	if err != nil || (server.Scheme != "http" && server.Scheme != "https") || server.Host == "" || server.User != nil || server.RawQuery != "" || server.Fragment != "" || (server.Path != "" && server.Path != "/") {
		return "", "", fmt.Errorf("invalid server in Lain player link")
	}
	if strings.TrimRight(q.Get("server"), "/") != strings.TrimRight(cfg.Server, "/") {
		return "", "", fmt.Errorf("this browser's server differs from the local login; run `lain login --server <web address>` on this computer first")
	}
	id, player := q.Get("id"), q.Get("player")
	if id == "" || strings.ContainsAny(id, "/\\\x00") || (player != "mpv" && player != "vlc") {
		return "", "", fmt.Errorf("invalid item or player in Lain player link")
	}
	return id, player, nil
}

func cmdOpenURL(args []string) error {
	if len(args) != 1 {
		return fmt.Errorf("usage: lain open-url 'lain://play?...'")
	}
	cfg, err := loadConfig()
	if err != nil || cfg.Token == "" {
		return fmt.Errorf("not logged in (run `lain login` on this computer)")
	}
	id, player, err := parsePlayerURL(args[0], cfg)
	if err != nil {
		return err
	}
	return cmdWatch([]string{"--id", id, "--player", player, "--once"})
}

func cmdInstallPlayerHandler() error {
	bin, err := playerExecutable()
	if err != nil {
		return err
	}
	bin, err = playerEvalSymlinks(bin)
	if err != nil {
		return err
	}
	dataHome := os.Getenv("XDG_DATA_HOME")
	if dataHome == "" {
		home, err := playerUserHome()
		if err != nil {
			return err
		}
		dataHome = filepath.Join(home, ".local", "share")
	}
	dir := filepath.Join(dataHome, "applications")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return err
	}
	// Desktop Exec quoting is not shell quoting. Percent signs inside the
	// executable path must be doubled to avoid field-code expansion.
	quoted := strings.NewReplacer("\\", "\\\\", "\"", "\\\"", "%", "%%").Replace(bin)
	entry := "[Desktop Entry]\nType=Application\nName=Lain external player\nNoDisplay=true\nTerminal=true\nExec=\"" + quoted + "\" open-url %u\nMimeType=x-scheme-handler/lain;\n"
	path := filepath.Join(dir, "lain-player.desktop")
	if err := os.WriteFile(path, []byte(entry), 0o644); err != nil {
		return err
	}
	if out, err := exec.Command("xdg-mime", "default", "lain-player.desktop", "x-scheme-handler/lain").CombinedOutput(); err != nil {
		return fmt.Errorf("register Lain player handler: %w: %s", err, strings.TrimSpace(string(out)))
	}
	fmt.Println("Lain player links registered for this user")
	return nil
}
