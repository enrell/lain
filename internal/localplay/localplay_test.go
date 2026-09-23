package localplay

import (
	"errors"
	"io"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
)

func fakeEnv(keys map[string]string) func(string) string {
	return func(k string) string { return keys[k] }
}

func fakePlayerBin(t *testing.T, name, script string) string {
	t.Helper()
	dir := t.TempDir()
	path := filepath.Join(dir, name)
	if err := os.WriteFile(path, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	return path
}

func testManager(bin, display string) *Manager {
	return &Manager{
		LookPath: func(string) (string, error) { return bin, nil },
		Getenv:   fakeEnv(map[string]string{"DISPLAY": display}),
	}
}

func TestPlayersNeedsDisplayAndBinary(t *testing.T) {
	m := &Manager{
		LookPath: func(string) (string, error) { return "/bin/true", nil },
		Getenv:   fakeEnv(nil),
	}
	if got := m.Players(); len(got) != 0 {
		t.Fatalf("no display must yield no players: %v", got)
	}
	m.Getenv = fakeEnv(map[string]string{"WAYLAND_DISPLAY": "wayland-1"})
	if got := m.Players(); len(got) != 2 || got[0] != "mpv" || got[1] != "vlc" {
		t.Fatalf("players=%v", got)
	}
	m.Getenv = fakeEnv(map[string]string{"DISPLAY": ":0"})
	if got := m.Players(); len(got) != 2 {
		t.Fatalf("X11 display must also count: %v", got)
	}
}

func TestPlayMPVReportsEachEntry(t *testing.T) {
	bin := fakePlayerBin(t, "mpv", `#!/bin/sh
state=""
for arg in "$@"; do
  case "$arg" in
    --script-opt=lain-state_dir=*) state="${arg#--script-opt=lain-state_dir=}" ;;
  esac
done
printf '{"position_sec":40,"duration_sec":100}' > "$state/0.json"
printf '{"position_sec":98,"duration_sec":100}' > "$state/1.json"
`)
	var mu sync.Mutex
	saved := map[string][2]float64{}
	done := make(chan error, 1)
	err := testManager(bin, ":0").Play(PlayRequest{
		Player: "mpv",
		Entries: []Entry{
			{ItemID: "e1", Path: "/media/e1.mkv"},
			{ItemID: "e2", Path: "/media/e2.mkv", Resume: 30},
		},
		Save: func(id string, pos, dur float64) {
			mu.Lock()
			saved[id] = [2]float64{pos, dur}
			mu.Unlock()
		},
		Done: func(err error) { done <- err },
	})
	if err != nil {
		t.Fatal(err)
	}
	select {
	case err := <-done:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(15 * time.Second):
		t.Fatal("player never finished")
	}
	mu.Lock()
	defer mu.Unlock()
	if len(saved) != 2 || saved["e1"][0] != 40 || saved["e2"][0] != 98 {
		t.Fatalf("saved=%v", saved)
	}
}

func TestPlayRejectsBusyAndUnknown(t *testing.T) {
	bin := fakePlayerBin(t, "mpv", "#!/bin/sh\nsleep 30\n")
	m := testManager(bin, ":0")
	defer m.Close()
	save := func(string, float64, float64) {}
	if err := m.Play(PlayRequest{Player: "mpv", Entries: []Entry{{ItemID: "e1", Path: "/x"}}, Save: save}); err != nil {
		t.Fatal(err)
	}
	if err := m.Play(PlayRequest{Player: "mpv", Entries: []Entry{{ItemID: "e2", Path: "/y"}}, Save: save}); err != ErrBusy {
		t.Fatalf("second play must be busy: %v", err)
	}
	if err := m.Play(PlayRequest{Player: "kodi", Entries: []Entry{{ItemID: "e1", Path: "/x"}}, Save: save}); err == nil || ErrBusy == err {
		t.Fatalf("unknown player must fail without touching the session: %v", err)
	}
	if err := m.Play(PlayRequest{Player: "mpv", Save: save}); err == nil {
		t.Fatal("empty playlist must fail")
	}
	m.Close()
	if err := m.Play(PlayRequest{Player: "mpv", Entries: []Entry{{ItemID: "e3", Path: "/z"}}, Save: save}); err != nil {
		t.Fatalf("after Close the slot must free: %v", err)
	}
	m.Close()
}

func TestPlayHonoursExplicitMissingPlayer(t *testing.T) {
	m := &Manager{
		LookPath: func(name string) (string, error) { return "", exec.ErrNotFound },
		Getenv:   fakeEnv(map[string]string{"DISPLAY": ":0"}),
	}
	err := m.Play(PlayRequest{Entries: []Entry{{ItemID: "e", Path: "/x"}}, Save: func(string, float64, float64) {}})
	if !errors.Is(err, ErrNoPlayer) && err == nil {
		t.Fatalf("want a no-player failure, got %v", err)
	}
}

func TestVLCIndexAndPositionOverSocket(t *testing.T) {
	socket := filepath.Join(t.TempDir(), "vlc.sock")
	listener, err := net.Listen("unix", socket)
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	go func() {
		for {
			conn, err := listener.Accept()
			if err != nil {
				return
			}
			go func() {
				defer conn.Close()
				buffer := make([]byte, 256)
				n, _ := conn.Read(buffer)
				cmd := string(buffer[:n])
				switch {
				case strings.Contains(cmd, "get_title"):
					io.WriteString(conn, "> lain-episode-000002\n")
				case strings.Contains(cmd, "get_time"):
					io.WriteString(conn, "> 42.5\n> 1400\n")
				}
			}()
		}
	}()
	index, err := vlcQueueIndex(socket, 4)
	if err != nil || index != 2 {
		t.Fatalf("index=%d err=%v", index, err)
	}
	pos, dur, err := vlcPosition(socket)
	if err != nil || pos != 42.5 || dur != 1400 {
		t.Fatalf("pos=%v dur=%v err=%v", pos, dur, err)
	}
}

func TestSubtitleOffPolicy(t *testing.T) {
	mk := func(streams ...contracts.MediaStream) []contracts.MediaStream { return streams }
	audio := func(lang string) contracts.MediaStream {
		return contracts.MediaStream{Type: "audio", Language: lang}
	}
	sub := func(lang string) contracts.MediaStream {
		return contracts.MediaStream{Type: "subtitle", Language: lang}
	}
	if !SubtitleOff(mk(audio("por"), sub("por")), "por") {
		t.Fatal("matching audio must hide subtitles")
	}
	if SubtitleOff(mk(audio("jpn"), sub("por")), "por") {
		t.Fatal("matching subtitle must stay selectable")
	}
	if !SubtitleOff(mk(audio("jpn"), sub("eng")), "por") {
		t.Fatal("no match anywhere must hide subtitles")
	}
	if SubtitleOff(mk(contracts.MediaStream{Type: "audio"}), "por") {
		t.Fatal("untagged streams must keep player defaults")
	}
	if SubtitleOff(mk(audio("jpn")), "") {
		t.Fatal("no preference means no override")
	}
	if !SameLanguage("fre", "fra") || !SameLanguage("por-BR", "por") {
		t.Fatal("alias normalization wrong")
	}
	if SameLanguage("eng", "por") || SameLanguage("", "por") || SameLanguage("por", "") {
		t.Fatal("alias normalization must reject mismatches and empty tags")
	}
}
