// Watch client: `lain login`, `lain logout`, `lain watch`.
//
// The CLI is a plain API client, like the desktop will be: login stores
// a token under the user config dir, watch resolves an item, launches
// mpv with an authenticated stream URL, then reports progress from the
// state file the bundled lua script maintains. No credentials reach
// the player; progress flows through the same endpoint every client
// uses.
package main

import (
	"bufio"
	_ "embed"
	"encoding/json"
	"fmt"
	"io"
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
	Server string `json:"server"`
	Token  string `json:"token"`
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
	if err := saveConfig(clientConfig{Server: strings.TrimRight(server, "/"), Token: tok}); err != nil {
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
	ID       string  `json:"id"`
	Title    string  `json:"title"`
	Kind     string  `json:"kind"`
	Season   int     `json:"season"`
	Episode  int     `json:"episode"`
	Year     int     `json:"year"`
	FilePath string  `json:"file_path"`
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

func cmdWatch(args []string) error {
	cfg, err := loadConfig()
	if err != nil || cfg.Token == "" {
		return fmt.Errorf("not logged in (run `lain login`)")
	}
	if srv := flag(args, "server", ""); srv != "" {
		cfg.Server = strings.TrimRight(srv, "/")
	}
	client := newAPIClient(cfg.Server, cfg.Token)

	query := strings.TrimSpace(strings.Join(positional(args), " "))
	once := hasFlag(args, "once")
	dry := hasFlag(args, "dry-run")

	var start apiItem
	switch {
	case hasFlag(args, "next") || query == "":
		start, err = resolveNext(client)
		if err != nil {
			return err
		}
	default:
		start, err = resolveQuery(client, query, os.Stdin, os.Stdout)
		if err != nil {
			return err
		}
	}

	for {
		played, err := playOne(client, cfg, start, dry)
		if err != nil {
			return err
		}
		if dry || once || !played.completed || !isATTY(os.Stdout) {
			if played.completed && !once {
				reportNext(client, start)
			}
			return nil
		}
		next, ok, err := findNextEpisode(client, start)
		if err != nil {
			return err
		}
		if !ok {
			fmt.Println("no further episode found")
			return nil
		}
		fmt.Printf("next: %s S%02dE%02d\n", next.Title, next.Season, next.Episode)
		start = next
	}
}

// playedResult reports what one mpv session did.
type playedResult struct {
	completed bool
	position  float64
	duration  float64
}

func playOne(client *apiClient, cfg clientConfig, item apiItem, dry bool) (playedResult, error) {
	streamURL := cfg.Server + "/api/items/" + item.ID + "/stream?token=" + url.QueryEscape(cfg.Token)
	label := item.Title
	if item.Episode > 0 {
		label = fmt.Sprintf("%s S%02dE%02d", item.Title, item.Season, item.Episode)
	}
	stateDir, err := os.MkdirTemp("", "lain-watch-*")
	if err != nil {
		return playedResult{}, err
	}
	defer os.RemoveAll(stateDir)
	stateFile := filepath.Join(stateDir, "progress.json")
	scriptFile := filepath.Join(stateDir, "lain-progress.lua")
	if err := os.WriteFile(scriptFile, []byte(progressLua), 0o644); err != nil {
		return playedResult{}, err
	}
	mpvArgs := []string{
		"--script=" + scriptFile,
		"--script-opts=lain-state=" + stateFile,
		"--title=" + label,
		streamURL,
	}
	if dry {
		fmt.Printf("mpv %s\n", strings.Join(mpvArgs, " "))
		return playedResult{}, nil
	}
	mpv, err := exec.LookPath("mpv")
	if err != nil {
		return playedResult{}, fmt.Errorf("mpv not found in PATH")
	}
	fmt.Printf("playing: %s\n", label)
	cmd := exec.Command(mpv, mpvArgs...)
	cmd.Stdin, cmd.Stdout, cmd.Stderr = os.Stdin, os.Stdout, os.Stderr
	runErr := cmd.Run()

	// Progress comes from the state file the lua script maintains:
	// pause, 10 s ticks, shutdown and end-of-file all flush it.
	pos, dur, ok := readStateFile(stateFile)
	if !ok {
		return playedResult{}, runErrToNil(runErr, "no progress recorded (file never started?)")
	}
	completed := dur > 0 && pos/dur >= 0.95
	var saved apiProgress
	putErr := client.put("/api/items/"+item.ID+"/progress",
		map[string]any{"position_sec": pos, "duration_sec": dur, "completed": completed}, &saved)
	if putErr != nil {
		return playedResult{}, putErr
	}
	fmt.Printf("saved: %.0fs / %.0fs%s\n", pos, dur, completedMark(completed))
	return playedResult{completed: completed, position: pos, duration: dur}, runErrToNil(runErr, "")
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
	var items []apiItem
	if err := client.get("/api/search", map[string]string{"q": query}, &items); err != nil {
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

// findNextEpisode locates same-title, same-season, episode+1.
func findNextEpisode(client *apiClient, cur apiItem) (apiItem, bool, error) {
	if cur.Episode == 0 {
		return apiItem{}, false, nil
	}
	var items []apiItem
	if err := client.get("/api/search", map[string]string{"q": cur.Title}, &items); err != nil {
		return apiItem{}, false, err
	}
	wantTitle := strings.ToLower(strings.TrimSpace(cur.Title))
	for _, it := range items {
		if strings.ToLower(strings.TrimSpace(it.Title)) == wantTitle &&
			it.Season == cur.Season && it.Episode == cur.Episode+1 {
			return it, true, nil
		}
	}
	return apiItem{}, false, nil
}

func reportNext(client *apiClient, cur apiItem) {
	next, ok, err := findNextEpisode(client, cur)
	if err != nil || !ok {
		return
	}
	fmt.Printf("next: %s S%02dE%02d\n", next.Title, next.Season, next.Episode)
}

// positional returns non-flag args after the subcommand, skipping
// values consumed by known value-flags (--pick N, --server URL).
func positional(args []string) []string {
	var out []string
	skipNext := false
	for _, a := range args {
		if skipNext {
			skipNext = false
			continue
		}
		if a == "--pick" || a == "--server" {
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

func isATTY(f *os.File) bool { return term.IsTerminal(int(f.Fd())) }
