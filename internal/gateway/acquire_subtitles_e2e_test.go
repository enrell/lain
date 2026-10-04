package gateway

import (
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/enrell/lain/internal/acquire"
	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/plugins/catalog"
)

// TestSubtitleNeedsFromRealProbe runs the host ffprobe on synthesized
// media (A-37): stream languages decide what is missing, and the real
// duration drives the sync check.
func TestSubtitleNeedsFromRealProbe(t *testing.T) {
	requireRealFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	libDir := t.TempDir()
	rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "Anime", "type": "anime", "path": libDir}, admin)
	wantCode(t, rec, 201)
	var lib contracts.Library
	_ = json.Unmarshal(rec.Body.Bytes(), &lib)

	// 3 s of video, Japanese audio, an embedded English subtitle.
	dir := filepath.Join(libDir, "Show")
	_ = os.MkdirAll(dir, 0o755)
	srt := filepath.Join(t.TempDir(), "in.srt")
	_ = os.WriteFile(srt, []byte("1\n00:00:00,500 --> 00:00:01,500\nHi\n"), 0o644)
	media := filepath.Join(dir, "[Fansub-A] Show - 01.mkv")
	cmd := exec.Command("ffmpeg", "-hide_banner", "-loglevel", "error",
		"-f", "lavfi", "-i", "testsrc=size=160x120:rate=10:duration=3",
		"-f", "lavfi", "-i", "sine=frequency=440:duration=3",
		"-i", srt, "-map", "0", "-map", "1", "-map", "2",
		"-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", "-c:s", "srt",
		"-metadata:s:a:0", "language=jpn", "-metadata:s:s:0", "language=eng", media)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("ffmpeg: %v\n%s", err, out)
	}
	out, _, err := srv.reg.CallOne(contracts.CapCatalogWrite, catalog.UpsertInput{
		LibraryID: lib.ID,
		Proposal:  contracts.Proposal{Kind: "anime", Title: "Show", Season: 1, Episode: 1, Confidence: 0.9, PluginID: "test"},
		Candidate: contracts.Candidate{Path: media, Size: 1, ModTime: 1, LibraryID: lib.ID},
	})
	if err != nil {
		t.Fatal(err)
	}
	it := out.(contracts.CatalogItem)

	p := acquire.DefaultProfile()
	p.SubtitleLanguages = []string{"eng", "jpn", "por"}
	p, err = srv.acquire.UpdateProfile("default", p)
	if err != nil {
		t.Fatal(err)
	}
	// eng: embedded subtitle; jpn: the audio; por: missing.
	if got := srv.acquire.MissingSubtitles(media, p); strings.Join(got, ",") != "por" {
		t.Fatalf("missing = %v", got)
	}

	// A subtitle that runs to 20 minutes does not belong to a 3 s file.
	api := fakeSubtitleAPI(t, "1\n00:20:00,000 --> 00:20:01,000\nlate\n")
	body := map[string]any{"name": "OS", "kind": "opensubtitles", "base_url": api.URL + "/api/v1", "api_key": "key-1"}
	rec = do(t, srv, "POST", "/api/acquire/subtitle-providers", body, admin)
	wantCode(t, rec, 201)
	var prov struct{ ID string }
	_ = json.Unmarshal(rec.Body.Bytes(), &prov)
	rec = do(t, srv, "POST", "/api/acquire/items/"+it.ID+"/subtitles", map[string]any{"provider_id": prov.ID, "file_id": "7", "language": "por"}, admin)
	wantCode(t, rec, 422)
	if !strings.Contains(rec.Body.String(), acquire.CodeSubtitleMismatch) {
		t.Fatalf("want a mismatch: %s", rec.Body)
	}
}
