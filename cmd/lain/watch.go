// Watch client: `lain login`, `lain logout`, `lain watch`.
//
// The CLI is a plain API client, like the desktop will be: login stores
// a token under the user config dir, watch resolves an item, launches
// mpv or VLC with an authenticated stream URL, then reports measured
// progress through the same endpoint every client uses. The media URL
// carries the token in the player process arguments while it runs.
package main

import (
	"bufio"
	_ "embed"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"golang.org/x/term"
)

//go:embed mpv_lain_progress.lua
var progressLua string

// clientConfig is ~/.config/lain/config.json (0600).
type clientConfig struct {
	Server            string `json:"server"`
	Token             string `json:"token"`
	Player            string `json:"player,omitempty"`
	PreferredLanguage string `json:"-"`
}

func configPath() (string, error) {
	dir, err := os.UserConfigDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, "lain", "config.json"), nil
}

func loadConfig() (clientConfig, error) {
	var c clientConfig
	p, err := configPath()
	if err != nil {
		return c, err
	}
	raw, err := os.ReadFile(p)
	if err != nil {
		return c, err
	}
	if err := json.Unmarshal(raw, &c); err != nil {
		return c, err
	}
	return c, nil
}

func saveConfig(c clientConfig) error {
	p, err := configPath()
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(p), 0o700); err != nil {
		return err
	}
	raw, err := json.MarshalIndent(c, "", "  ")
	if err != nil {
		return err
	}
	tmp, err := os.CreateTemp(filepath.Dir(p), "config-*.tmp")
	if err != nil {
		return err
	}
	name := tmp.Name()
	if _, err := tmp.Write(append(raw, '\n')); err != nil {
		tmp.Close()
		os.Remove(name)
		return err
	}
	tmp.Close()
	if err := os.Chmod(name, 0o600); err != nil {
		os.Remove(name)
		return err
	}
	return os.Rename(name, p)
}

func cmdLogin(args []string) error {
	server := flag(args, "server", "http://127.0.0.1:9360")
	username := flag(args, "username", "")
	if username == "" {
		fmt.Print("username: ")
		line, err := bufio.NewReader(os.Stdin).ReadString('\n')
		if err != nil {
			return err
		}
		username = strings.TrimSpace(line)
	}
	password := os.Getenv("LAIN_PASSWORD")
	if password == "" {
		fmt.Print("password: ")
		raw, err := term.ReadPassword(int(os.Stdin.Fd()))
		fmt.Println()
		if err != nil {
			return err
		}
		password = string(raw)
	}
	tok, err := apiLogin(server, username, password)
	if err != nil {
		return err
	}
	previous, _ := loadConfig()
	if err := saveConfig(clientConfig{Server: strings.TrimRight(server, "/"), Token: tok, Player: previous.Player}); err != nil {
		return err
	}
	fmt.Printf("logged in as %s on %s\n", username, server)
	return nil
}

func cmdLogout(args []string) error {
	p, err := configPath()
	if err != nil {
		return err
	}
	if err := os.Remove(p); err != nil && !os.IsNotExist(err) {
		return err
	}
	fmt.Println("logged out")
	return nil
}

func cmdPlayer(args []string) error {
	cfg, err := loadConfig()
	if err != nil && !os.IsNotExist(err) {
		return err
	}
	if len(args) == 0 {
		player := cfg.Player
		if player == "" {
			player = "mpv"
		}
		fmt.Println(player)
		return nil
	}
	if len(args) != 1 || (args[0] != "mpv" && args[0] != "vlc") {
		return fmt.Errorf("usage: lain player [mpv|vlc]")
	}
	cfg.Player = args[0]
	if err := saveConfig(cfg); err != nil {
		return err
	}
	fmt.Printf("default player: %s\n", cfg.Player)
	return nil
}

func apiLogin(server, username, password string) (string, error) {
	body, _ := json.Marshal(map[string]string{"username": username, "password": password})
	resp, err := http.Post(server+"/api/auth/login", "application/json", strings.NewReader(string(body)))
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()
	raw, _ := io.ReadAll(resp.Body)
	if resp.StatusCode != 200 {
		return "", fmt.Errorf("login: %s", strings.TrimSpace(string(raw)))
	}
	var out struct {
		Token string `json:"token"`
	}
	if err := json.Unmarshal(raw, &out); err != nil || out.Token == "" {
		return "", fmt.Errorf("login: bad response")
	}
	return out.Token, nil
}

// apiItem mirrors the catalog JSON the gateway serves.
type apiItem struct {
	ID       string `json:"id"`
	Title    string `json:"title"`
	Kind     string `json:"kind"`
	Season   int    `json:"season"`
	Episode  int    `json:"episode"`
	Year     int    `json:"year"`
	FilePath string `json:"file_path"`
	Missing  bool   `json:"missing"`
}

type apiProgress struct {
	ItemID      string  `json:"item_id"`
	PositionSec float64 `json:"position_sec"`
	DurationSec float64 `json:"duration_sec"`
	Completed   bool    `json:"completed"`
}

type apiClient struct {
	server string
	token  string
	http   *http.Client
}

func newAPIClient(server, token string) *apiClient {
	return &apiClient{server: strings.TrimRight(server, "/"), token: token, http: &http.Client{Timeout: 30 * time.Second}}
}

func (c *apiClient) get(path string, query map[string]string, out any) error {
	u := c.server + path
	if len(query) > 0 {
		q := url.Values{}
		for k, v := range query {
			q.Set(k, v)
		}
		u += "?" + q.Encode()
	}
	req, err := http.NewRequest("GET", u, nil)
	if err != nil {
		return err
	}
	req.Header.Set("Authorization", "Bearer "+c.token)
	resp, err := c.http.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	raw, _ := io.ReadAll(resp.Body)
	if resp.StatusCode/100 != 2 {
		return fmt.Errorf("GET %s: %d %s", path, resp.StatusCode, strings.TrimSpace(string(raw)))
	}
	if out == nil {
		return nil
	}
	return json.Unmarshal(raw, out)
}

func (c *apiClient) put(path string, body any, out any) error {
	raw, _ := json.Marshal(body)
	req, err := http.NewRequest("PUT", c.server+path, strings.NewReader(string(raw)))
	if err != nil {
		return err
	}
	req.Header.Set("Authorization", "Bearer "+c.token)
	req.Header.Set("Content-Type", "application/json")
	resp, err := c.http.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	data, _ := io.ReadAll(resp.Body)
	if resp.StatusCode/100 != 2 {
		return fmt.Errorf("PUT %s: %d %s", path, resp.StatusCode, strings.TrimSpace(string(data)))
	}
	if out == nil {
		return nil
	}
	return json.Unmarshal(data, out)
}

func (c *apiClient) patch(path string, body any, out any) error {
	raw, err := json.Marshal(body)
	if err != nil {
		return err
	}
	req, err := http.NewRequest("PATCH", c.server+path, strings.NewReader(string(raw)))
	if err != nil {
		return err
	}
	req.Header.Set("Authorization", "Bearer "+c.token)
	req.Header.Set("Content-Type", "application/json")
	resp, err := c.http.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	data, err := io.ReadAll(resp.Body)
	if err != nil {
		return err
	}
	if resp.StatusCode/100 != 2 {
		return fmt.Errorf("PATCH %s: %d %s", path, resp.StatusCode, strings.TrimSpace(string(data)))
	}
	if out == nil {
		return nil
	}
	return json.Unmarshal(data, out)
}

func cmdLanguage(args []string) error {
	if len(args) > 1 {
		return fmt.Errorf("usage: lain language [three-letter-code|--clear]")
	}
	cfg, err := loadConfig()
	if err != nil || cfg.Token == "" {
		return fmt.Errorf("not logged in (run `lain login`)")
	}
	client := newAPIClient(cfg.Server, cfg.Token)
	var user struct {
		PreferredLanguage string `json:"preferred_language"`
	}
	if len(args) == 0 {
		if err := client.get("/api/me", nil, &user); err != nil {
			return err
		}
		if user.PreferredLanguage == "" {
			fmt.Println("player default")
		} else {
			fmt.Println(user.PreferredLanguage)
		}
		return nil
	}
	language := strings.ToLower(strings.TrimSpace(args[0]))
	if language == "--clear" {
		language = ""
	}
	if language != "" {
		if len(language) != 3 {
			return fmt.Errorf("language must be a three-letter ISO 639-2 code")
		}
		for _, char := range language {
			if char < 'a' || char > 'z' {
				return fmt.Errorf("language must be a three-letter ISO 639-2 code")
			}
		}
	}
	if err := client.patch("/api/me/preferences", map[string]string{"preferred_language": language}, &user); err != nil {
		return err
	}
	if user.PreferredLanguage == "" {
		fmt.Println("preferred language cleared")
	} else {
		fmt.Printf("preferred language: %s\n", user.PreferredLanguage)
	}
	return nil
}

func cmdWatch(args []string) error {
	if choice := flag(args, "player", ""); choice != "" && choice != "mpv" && choice != "vlc" {
		return fmt.Errorf("unsupported player %q (choose mpv or vlc)", choice)
	}
	cfg, err := loadConfig()
	if err != nil || cfg.Token == "" {
		return fmt.Errorf("not logged in (run `lain login`)")
	}
	player := flag(args, "player", cfg.Player)
	if player == "" {
		player = "mpv"
	}
	if player != "mpv" && player != "vlc" {
		return fmt.Errorf("unsupported player %q (choose mpv or vlc)", player)
	}
	if srv := flag(args, "server", ""); srv != "" {
		cfg.Server = strings.TrimRight(srv, "/")
	}
	client := newAPIClient(cfg.Server, cfg.Token)

	query := strings.TrimSpace(strings.Join(positional(args), " "))
	once := hasFlag(args, "once")
	dry := hasFlag(args, "dry-run")
	if !dry {
		var me struct {
			PreferredLanguage string `json:"preferred_language"`
		}
		if err := client.get("/api/me", nil, &me); err != nil {
			return err
		}
		cfg.PreferredLanguage = me.PreferredLanguage
	}

	var start apiItem
	if id := flag(args, "id", ""); id != "" {
		if err := client.get("/api/catalog/"+url.PathEscape(id), nil, &start); err != nil {
			return err
		}
	} else if hasFlag(args, "next") || query == "" {
		start, err = resolveNext(client)
		if err != nil {
			return err
		}
	} else {
		start, err = resolveQuery(client, query, os.Stdin, os.Stdout)
		if err != nil {
			return err
		}
	}
	if !once && !dry && start.Episode > 0 {
		queue, err := episodeQueue(client, start)
		if err != nil {
			return err
		}
		if len(queue) > 1 {
			return playEpisodeQueue(client, cfg, queue, player)
		}
	}
	_, err = playOneWithPlayer(client, cfg, start, player, dry)
	return err
}

// playedResult reports what one external-player session did.
type playedResult struct {
	completed bool
	position  float64
	duration  float64
}

func playOne(client *apiClient, cfg clientConfig, item apiItem, dry bool) (playedResult, error) {
	return playOneWithPlayer(client, cfg, item, "mpv", dry)
}

func playOneWithPlayer(client *apiClient, cfg clientConfig, item apiItem, player string, dry bool) (playedResult, error) {
	streamURL := cfg.Server + "/api/items/" + url.PathEscape(item.ID) + "/stream?token=" + url.QueryEscape(cfg.Token)
	label := item.Title
	if item.Episode > 0 {
		label = fmt.Sprintf("%s S%02dE%02d", item.Title, item.Season, item.Episode)
	}
	if player != "mpv" && player != "vlc" {
		return playedResult{}, fmt.Errorf("unsupported player %q", player)
	}
	stateDir, err := os.MkdirTemp("", "lain-watch-*")
	if err != nil {
		return playedResult{}, err
	}
	defer os.RemoveAll(stateDir)
	var previous apiProgress
	if !dry {
		if err := client.get("/api/items/"+url.PathEscape(item.ID)+"/progress", nil, &previous); err != nil {
			return playedResult{}, err
		}
	}
	resume := previous.PositionSec
	if previous.Completed || resume < 5 {
		resume = 0
	}
	args := []string{}
	stateFile := filepath.Join(stateDir, "progress.json")
	socketFile := filepath.Join(stateDir, "vlc.sock")
	if player == "mpv" {
		scriptFile := filepath.Join(stateDir, "lain-progress.lua")
		if err := os.WriteFile(scriptFile, []byte(progressLua), 0o600); err != nil {
			return playedResult{}, err
		}
		args = append(args, "--script="+scriptFile, "--script-opts=lain-state="+stateFile, "--title="+label)
		if cfg.PreferredLanguage != "" {
			args = append(args, "--script-opt=lain-language="+cfg.PreferredLanguage)
		}
		if resume > 0 {
			args = append(args, "--start="+strconv.FormatFloat(resume, 'f', 0, 64))
		}
	} else {
		args = append(args, "--no-one-instance", "--play-and-exit", "--extraintf=rc", "--rc-unix="+socketFile, "--meta-title="+label)
		if cfg.PreferredLanguage != "" {
			args = append(args, "--audio-language="+cfg.PreferredLanguage, "--sub-language="+cfg.PreferredLanguage)
		}
		if resume > 0 {
			args = append(args, "--start-time="+strconv.FormatFloat(resume, 'f', 0, 64))
		}
	}
	args = append(args, streamURL)
	if dry {
		fmt.Printf("%s %s [authenticated stream URL]\n", player, strings.Join(args[:len(args)-1], " "))
		return playedResult{}, nil
	}
	bin, err := exec.LookPath(player)
	if err != nil {
		return playedResult{}, fmt.Errorf("%s not found in PATH", player)
	}
	fmt.Printf("playing: %s\n", label)
	cmd := exec.Command(bin, args...)
	cmd.Stdin, cmd.Stdout, cmd.Stderr = os.Stdin, os.Stdout, os.Stderr
	var runErr error
	var pos, dur float64
	var ok bool
	if player == "vlc" {
		subtitleOff := false
		if cfg.PreferredLanguage != "" {
			subtitleOff, err = vlcSubtitleOff(client, item, cfg.PreferredLanguage)
			if err != nil {
				return playedResult{}, err
			}
		}
		pos, dur, ok, runErr = runVLCWithSubtitlePolicy(cmd, socketFile, subtitleOff)
	} else {
		runErr = cmd.Run()
		pos, dur, ok = readStateFile(stateFile)
	}

	// mpv writes a state file; VLC is sampled through its local RC socket.
	if !ok {
		if runErr != nil {
			return playedResult{}, fmt.Errorf("%s exited before progress was recorded: %w", player, runErr)
		}
		fmt.Fprintf(os.Stderr, "warning: %s closed without measurable progress; watch position was not changed\n", player)
		return playedResult{}, nil
	}
	completed := dur > 0 && pos/dur >= 0.95
	var saved apiProgress
	putErr := client.put("/api/items/"+url.PathEscape(item.ID)+"/progress",
		map[string]any{"position_sec": pos, "duration_sec": dur, "completed": completed}, &saved)
	if putErr != nil {
		return playedResult{}, putErr
	}
	fmt.Printf("saved: %.0fs / %.0fs%s\n", pos, dur, completedMark(completed))
	result := playedResult{completed: completed, position: pos, duration: dur}
	return result, playerExitErr(player, runErr)
}

func playerExitErr(player string, runErr error) error {
	if player == "vlc" && runErr != nil {
		return fmt.Errorf("vlc exited after saving progress: %w", runErr)
	}
	return runErrToNil(runErr, "")
}

// VLC's local RC socket reports the same position and duration that mpv's
// bundled Lua script writes. The socket lives in a private temporary dir.
func runVLC(cmd *exec.Cmd, socket string) (pos, dur float64, ok bool, runErr error) {
	return runVLCWithSubtitlePolicy(cmd, socket, false)
}

func runVLCWithSubtitlePolicy(cmd *exec.Cmd, socket string, subtitleOff bool) (pos, dur float64, ok bool, runErr error) {
	if err := cmd.Start(); err != nil {
		return 0, 0, false, err
	}
	done := make(chan error, 1)
	go func() { done <- cmd.Wait() }()
	ticker := time.NewTicker(time.Second)
	defer ticker.Stop()
	trackApplied := !subtitleOff
	var trackErr error
	for {
		select {
		case err := <-done:
			if trackErr != nil {
				return pos, dur, ok, trackErr
			}
			return pos, dur, ok, err
		case <-ticker.C:
			p, d, err := vlcPosition(socket)
			if err == nil && d > 0 && p >= 0 {
				if !trackApplied {
					_, trackErr = vlcRC(socket, "strack -1\n")
					trackApplied = trackErr == nil
				}
				pos, dur, ok = p, d, true
			}
		}
	}
}

func vlcPosition(socket string) (float64, float64, error) {
	conn, err := net.DialTimeout("unix", socket, 300*time.Millisecond)
	if err != nil {
		return 0, 0, err
	}
	defer conn.Close()
	_ = conn.SetDeadline(time.Now().Add(time.Second))
	if _, err := io.WriteString(conn, "get_time\nget_length\n"); err != nil {
		return 0, 0, err
	}
	var values []float64
	scanner := bufio.NewScanner(conn)
	for scanner.Scan() {
		line := strings.TrimSpace(strings.TrimLeft(scanner.Text(), "> "))
		if n, err := strconv.ParseFloat(line, 64); err == nil && n >= 0 {
			values = append(values, n)
			if len(values) == 2 {
				return values[0], values[1], nil
			}
		}
	}
	if err := scanner.Err(); err != nil {
		return 0, 0, err
	}
	return 0, 0, fmt.Errorf("VLC did not report time and duration")
}

func runErrToNil(runErr error, note string) error {
	if runErr == nil {
		return nil
	}
	// mpv exits non-zero on plain quit (EOF/q); progress is already
	// saved, so report the note instead of failing the command.
	if note != "" {
		return fmt.Errorf("%s", note)
	}
	return nil
}

func completedMark(completed bool) string {
	if completed {
		return " (completed)"
	}
	return ""
}

func readStateFile(path string) (pos, dur float64, ok bool) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return 0, 0, false
	}
	var st struct {
		PositionSec float64 `json:"position_sec"`
		DurationSec float64 `json:"duration_sec"`
	}
	if err := json.Unmarshal(raw, &st); err != nil {
		return 0, 0, false
	}
	return st.PositionSec, st.DurationSec, true
}

// resolveNext picks the first unfinished continue-watching entry.
func resolveNext(client *apiClient) (apiItem, error) {
	var feed []struct {
		apiProgress
	}
	if err := client.get("/api/me/continue", nil, &feed); err != nil {
		return apiItem{}, err
	}
	for _, p := range feed {
		if p.Completed {
			continue
		}
		// Freshest entry the service lists is last-written; feed is
		// unordered, so take the first unfinished one and move on.
		var item apiItem
		if err := client.get("/api/catalog/"+p.ItemID, nil, &item); err != nil {
			continue
		}
		return item, nil
	}
	return apiItem{}, fmt.Errorf("nothing to continue (watch something first, or pass a search)")
}

// resolveQuery searches and picks one item: single hit plays directly,
// several hits take --pick N or ask interactively.
func resolveQuery(client *apiClient, query string, in *os.File, out *os.File) (apiItem, error) {
	return resolveQueryArgs(client, query, in, out, os.Args)
}

func resolveQueryArgs(client *apiClient, query string, in *os.File, out *os.File, argv []string) (apiItem, error) {
	items, err := searchItems(client, query)
	if err != nil {
		return apiItem{}, err
	}
	if len(items) == 0 {
		return apiItem{}, fmt.Errorf("no results for %q", query)
	}
	if len(items) == 1 {
		return items[0], nil
	}
	if n := pickFlag(argv); n >= 1 && n <= len(items) {
		return items[n-1], nil
	}
	return pickItem(items, in, out)
}

// pickFlag reads --pick N from argv.
func pickFlag(argv []string) int {
	for i := 0; i < len(argv); i++ {
		if argv[i] == "--pick" && i+1 < len(argv) {
			n, _ := strconv.Atoi(argv[i+1])
			return n
		}
		if strings.HasPrefix(argv[i], "--pick=") {
			n, _ := strconv.Atoi(strings.TrimPrefix(argv[i], "--pick="))
			return n
		}
	}
	return 0
}

// pickItem lists candidates and reads a choice. Pure over slices +
// streams so tests drive it without a server.
func pickItem(items []apiItem, in *os.File, out *os.File) (apiItem, error) {
	n := len(items)
	if n > 20 {
		items = items[:20]
		n = 20
	}
	for i, it := range items {
		fmt.Fprintf(out, "%2d  %s", i+1, it.Title)
		if it.Episode > 0 {
			fmt.Fprintf(out, " S%02dE%02d", it.Season, it.Episode)
		}
		if it.Year > 0 {
			fmt.Fprintf(out, " (%d)", it.Year)
		}
		fmt.Fprintln(out)
	}
	fmt.Fprintf(out, "pick [1-%d]: ", n)
	line, err := bufio.NewReader(in).ReadString('\n')
	if err != nil {
		return apiItem{}, err
	}
	idx, err := strconv.Atoi(strings.TrimSpace(line))
	if err != nil || idx < 1 || idx > n {
		return apiItem{}, fmt.Errorf("invalid pick")
	}
	return items[idx-1], nil
}

// apiPage mirrors the paged envelope the gateway serves.
type apiPage struct {
	Items []apiItem `json:"items"`
	Total int       `json:"total"`
}

// searchItems fetches the full match set (watch is interactive; paging
// the pick list comes with the desktop client).
func searchItems(client *apiClient, query string) ([]apiItem, error) {
	var page apiPage
	if err := client.get("/api/search", map[string]string{"q": query, "limit": "500"}, &page); err != nil {
		return nil, err
	}
	return page.Items, nil
}

// valueFlags consume the following arg.
var valueFlags = map[string]bool{
	"--pick": true, "--player": true, "--id": true, "--server": true, "--data-dir": true,
	"--out": true, "--username": true, "--password": true,
	"--type": true, "--runs": true, "--port": true, "--matrix-bin": true,
}

// positional returns non-flag args after the subcommand, skipping
// values consumed by known value-flags (--pick N, --data-dir D...).
func positional(args []string) []string {
	var out []string
	skipNext := false
	for _, a := range args {
		if skipNext {
			skipNext = false
			continue
		}
		if valueFlags[a] {
			skipNext = true
			continue
		}
		if strings.HasPrefix(a, "-") {
			continue
		}
		out = append(out, a)
	}
	return out
}

func hasFlag(args []string, name string) bool {
	for _, a := range args {
		if a == "--"+name {
			return true
		}
	}
	return false
}
