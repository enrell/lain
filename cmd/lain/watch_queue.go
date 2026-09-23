package main

import (
	"encoding/json"
	"fmt"
	"io"
	"math"
	"net"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

// episodeQueue uses the catalog's title grouping and watch order (D-056).
// Missing files stay visible in the catalog but are not put in the player.
func episodeQueue(client *apiClient, start apiItem) ([]apiItem, error) {
	if start.Episode == 0 {
		return []apiItem{start}, nil
	}
	var response struct {
		Items []apiItem `json:"items"`
	}
	if err := client.get("/api/catalog/"+url.PathEscape(start.ID)+"/episodes", nil, &response); err != nil {
		return nil, err
	}
	found := false
	queue := make([]apiItem, 0, len(response.Items))
	for _, item := range response.Items {
		if item.ID == start.ID {
			found = true
		}
		if found && !item.Missing {
			queue = append(queue, item)
		}
	}
	if !found || len(queue) == 0 {
		return nil, fmt.Errorf("selected episode is not available in its title")
	}
	return queue, nil
}

func queueResume(progress apiProgress) float64 {
	if progress.Completed || !finiteProgress(progress.PositionSec) || progress.PositionSec < 5 {
		return 0
	}
	return progress.PositionSec
}

func finiteProgress(n float64) bool { return !math.IsNaN(n) && !math.IsInf(n, 0) && n >= 0 }

type episodeMonitor struct {
	client *apiClient
	items  []apiItem
	last   map[int]apiProgress
}

type mediaStream struct {
	Type     string `json:"type"`
	Language string `json:"language"`
}

func sameLanguage(actual, preferred string) bool {
	aliases := map[string]string{
		"fre": "fra", "ger": "deu", "chi": "zho", "dut": "nld",
		"gre": "ell", "rum": "ron", "cze": "ces", "slo": "slk",
		"per": "fas", "may": "msa", "alb": "sqi", "arm": "hye",
		"baq": "eus", "bur": "mya", "ice": "isl", "mac": "mkd",
	}
	actual = strings.ToLower(strings.Split(actual, "-")[0])
	preferred = strings.ToLower(preferred)
	if mapped := aliases[actual]; mapped != "" {
		actual = mapped
	}
	if mapped := aliases[preferred]; mapped != "" {
		preferred = mapped
	}
	return actual != "" && actual == preferred
}

// A tagged audio match wins. Otherwise show only a subtitle in the
// preferred language. Unknown probe data leaves native VLC defaults.
func vlcSubtitleOff(client *apiClient, item apiItem, language string) (bool, error) {
	var plan struct {
		Streams []mediaStream `json:"streams"`
	}
	if err := client.get("/api/items/"+url.PathEscape(item.ID)+"/playback", map[string]string{"client": "vlc"}, &plan); err != nil {
		return false, err
	}
	if len(plan.Streams) == 0 {
		return false, nil
	}
	known := false
	for _, stream := range plan.Streams {
		if (stream.Type == "audio" || stream.Type == "subtitle") && stream.Language != "" {
			known = true
		}
		if stream.Type == "audio" && sameLanguage(stream.Language, language) {
			return true, nil
		}
	}
	if !known {
		return false, nil
	}
	for _, stream := range plan.Streams {
		if stream.Type == "subtitle" && sameLanguage(stream.Language, language) {
			return false, nil
		}
	}
	return true, nil
}

func (m *episodeMonitor) save(index int, pos, dur float64) error {
	if index < 0 || index >= len(m.items) || !finiteProgress(pos) || !finiteProgress(dur) || dur <= 0 {
		return nil
	}
	progress := apiProgress{ItemID: m.items[index].ID, PositionSec: pos, DurationSec: dur, Completed: pos/dur >= .95}
	if old, ok := m.last[index]; ok && old.PositionSec == pos && old.DurationSec == dur && old.Completed == progress.Completed {
		return nil
	}
	if err := m.client.put("/api/items/"+url.PathEscape(progress.ItemID)+"/progress",
		map[string]any{"position_sec": pos, "duration_sec": dur, "completed": progress.Completed}, nil); err != nil {
		return err
	}
	m.last[index] = progress
	return nil
}

func (m *episodeMonitor) flushMPV(dir string) error {
	for i := range m.items {
		pos, dur, ok := readStateFile(filepath.Join(dir, fmt.Sprintf("%d.json", i)))
		if ok {
			if err := m.save(i, pos, dur); err != nil {
				return err
			}
		}
	}
	return nil
}

func queueURL(cfg clientConfig, item apiItem) string {
	return cfg.Server + "/api/items/" + url.PathEscape(item.ID) + "/stream?token=" + url.QueryEscape(cfg.Token)
}

func playEpisodeQueue(client *apiClient, cfg clientConfig, items []apiItem, player string) error {
	if player != "mpv" && player != "vlc" {
		return fmt.Errorf("unsupported player %q", player)
	}
	if len(items) == 0 {
		return fmt.Errorf("empty episode queue")
	}
	dir, err := os.MkdirTemp("", "lain-watch-*")
	if err != nil {
		return err
	}
	defer os.RemoveAll(dir)
	var playlist strings.Builder
	playlist.WriteString("#EXTM3U\n")
	resumes := make(map[string]float64)
	subtitleOff := make([]bool, len(items))
	for i, item := range items {
		var p apiProgress
		if err := client.get("/api/items/"+url.PathEscape(item.ID)+"/progress", nil, &p); err != nil {
			return err
		}
		if at := queueResume(p); at > 0 {
			resumes[strconv.Itoa(i)] = at
		}
		if player == "vlc" && cfg.PreferredLanguage != "" {
			subtitleOff[i], err = vlcSubtitleOff(client, item, cfg.PreferredLanguage)
			if err != nil {
				return err
			}
		}
		playlist.WriteString(fmt.Sprintf("#EXTINF:-1,lain-episode-%06d\n", i))
		playlist.WriteString(queueURL(cfg, item) + "\n")
	}
	playlistFile := filepath.Join(dir, "episodes.m3u")
	if err := os.WriteFile(playlistFile, []byte(playlist.String()), 0o600); err != nil {
		return err
	}
	bin, err := exec.LookPath(player)
	if err != nil {
		return fmt.Errorf("%s not found in PATH", player)
	}
	monitor := &episodeMonitor{client: client, items: items, last: make(map[int]apiProgress)}
	var cmd *exec.Cmd
	if player == "mpv" {
		scriptFile := filepath.Join(dir, "lain-progress.lua")
		if err := os.WriteFile(scriptFile, []byte(progressLua), 0o600); err != nil {
			return err
		}
		resumeFile := filepath.Join(dir, "resume.json")
		raw, err := json.Marshal(resumes)
		if err != nil {
			return err
		}
		if err := os.WriteFile(resumeFile, raw, 0o600); err != nil {
			return err
		}
		args := []string{"--script=" + scriptFile, "--script-opt=lain-state_dir=" + dir,
			"--script-opt=lain-resume_file=" + resumeFile}
		if cfg.PreferredLanguage != "" {
			args = append(args, "--script-opt=lain-language="+cfg.PreferredLanguage)
		}
		cmd = exec.Command(bin, append(args, "--playlist="+playlistFile)...)
	} else {
		socket := filepath.Join(dir, "vlc.sock")
		args := []string{"--no-one-instance", "--play-and-exit", "--extraintf=rc", "--rc-unix=" + socket}
		if cfg.PreferredLanguage != "" {
			args = append(args, "--audio-language="+cfg.PreferredLanguage, "--sub-language="+cfg.PreferredLanguage)
		}
		if at := resumes["0"]; at > 0 {
			args = append(args, "--start-time="+strconv.FormatFloat(at, 'f', 0, 64))
		}
		cmd = exec.Command(bin, append(args, playlistFile)...)
	}
	cmd.Stdin, cmd.Stdout, cmd.Stderr = os.Stdin, os.Stdout, os.Stderr
	fmt.Printf("playing %d episodes from %s S%02dE%02d\n", len(items), items[0].Title, items[0].Season, items[0].Episode)
	if err := cmd.Start(); err != nil {
		return err
	}
	done := make(chan error, 1)
	go func() { done <- cmd.Wait() }()
	ticker := time.NewTicker(time.Second)
	defer ticker.Stop()
	current := -1
	var lastPos, lastDur float64
	var observed bool
	var saveErr error
	sample := func() {
		var err error
		if player == "mpv" {
			err = monitor.flushMPV(dir)
		} else {
			var index int
			index, err = vlcQueueIndex(filepath.Join(dir, "vlc.sock"), cfg, items)
			if err != nil {
				return
			}
			if index != current {
				current, observed = index, false
				if subtitleOff[index] {
					if _, err := vlcRC(filepath.Join(dir, "vlc.sock"), "strack -1\n"); err != nil {
						saveErr = err
						fmt.Fprintf(os.Stderr, "warning: could not disable subtitles: %v\n", err)
					}
				}
				if index > 0 && resumes[strconv.Itoa(index)] > 0 {
					if err := vlcQueueSeek(filepath.Join(dir, "vlc.sock"), resumes[strconv.Itoa(index)]); err != nil {
						saveErr = err
						fmt.Fprintf(os.Stderr, "warning: could not resume episode: %v\n", err)
					}
					return
				}
			}
			var pos, dur float64
			pos, dur, err = vlcPosition(filepath.Join(dir, "vlc.sock"))
			if err != nil || dur <= 0 {
				return
			}
			lastPos, lastDur, observed = pos, dur, true
			err = monitor.save(index, pos, dur)
		}
		if err != nil {
			saveErr = err
			fmt.Fprintf(os.Stderr, "warning: could not save episode progress: %v\n", err)
		} else {
			saveErr = nil
		}
	}
	for {
		select {
		case err := <-done:
			if player == "mpv" {
				sample()
			}
			if player == "vlc" && current >= 0 && observed {
				if e := monitor.save(current, lastPos, lastDur); e != nil {
					saveErr = e
				}
			}
			if saveErr != nil {
				return saveErr
			}
			if len(monitor.last) == 0 {
				return fmt.Errorf("%s closed without measurable progress", player)
			}
			if err != nil {
				return fmt.Errorf("%s exited: %w", player, err)
			}
			return nil
		case <-ticker.C:
			sample()
		}
	}
}

// VLC RC's status response identifies the active input. Match only a known
// item path; never print the response, because it contains the stream token.
func vlcQueueIndex(socket string, cfg clientConfig, items []apiItem) (int, error) {
	title, titleErr := vlcRC(socket, "get_title\n")
	if titleErr == nil {
		for i := range items {
			if strings.Contains(title, fmt.Sprintf("lain-episode-%06d", i)) {
				return i, nil
			}
		}
	}
	response, err := vlcRC(socket, "status\n")
	if err != nil {
		return -1, err
	}
	for i, item := range items {
		if strings.Contains(response, queueURL(cfg, item)) {
			return i, nil
		}
	}
	return -1, fmt.Errorf("VLC did not identify the active episode")
}

func vlcQueueSeek(socket string, seconds float64) error {
	_, err := vlcRC(socket, "seek "+strconv.FormatFloat(seconds, 'f', 0, 64)+"\n")
	return err
}

func vlcRC(socket, command string) (string, error) {
	conn, err := net.DialTimeout("unix", socket, 300*time.Millisecond)
	if err != nil {
		return "", err
	}
	defer conn.Close()
	_ = conn.SetDeadline(time.Now().Add(700 * time.Millisecond))
	if _, err := io.WriteString(conn, command); err != nil {
		return "", err
	}
	var output strings.Builder
	buffer := make([]byte, 4096)
	for {
		n, err := conn.Read(buffer)
		if n > 0 {
			output.Write(buffer[:n])
		}
		if err != nil {
			if timeout, ok := err.(net.Error); ok && timeout.Timeout() && output.Len() > 0 {
				return output.String(), nil
			}
			if err == io.EOF && output.Len() > 0 {
				return output.String(), nil
			}
			return "", err
		}
	}
}
