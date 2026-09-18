package transcode

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
)

// advTempFixture builds a transcoder over a real (tiny) source file with
// injected probe/capability seams: the relocated artifact layout must be
// verifiable without ffmpeg.
func advTempFixture(t *testing.T) (*Transcoder, string) {
	t.Helper()
	root := t.TempDir()
	src := filepath.Join(root, "[Fansub-A] Temp Show.mkv")
	if err := os.WriteFile(src, []byte("source"), 0o600); err != nil {
		t.Fatal(err)
	}
	tr := newWithDeps(filepath.Join(root, "data"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	tr.probeFn = func(string, string) (mediaReport, error) {
		return mediaReport{Streams: []stream{
			{Index: 0, CodecType: "video", CodecName: "h264", PixelFormat: "yuv420p",
				Width: 1920, Height: 1080, FrameRate: "25/1"},
			{Index: 1, CodecType: "audio", CodecName: "aac", Channels: 2, Default: 1},
		}}, nil
	}
	tr.capsFn = func(contracts.TranscodeSettings) capabilities {
		return capabilities{
			Encoders: map[string]bool{"libx264": true},
			Filters:  map[string]bool{},
			Hardware: map[string]bool{},
		}
	}
	return tr, src
}

// advWaitV3 polls one session until it reaches want; transcoding is
// asynchronous, so a deadline is the only safe way to wait.
func advWaitV3(t *testing.T, tr *Transcoder, src, session, want string) contracts.TranscodeV3Status {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		out, err := tr.Invoke(contracts.CapPlaybackTranscodeV3, contracts.TranscodeV3Request{
			Action: contracts.TranscodeStatusAction, FilePath: src, Session: session,
		})
		if err != nil {
			t.Fatalf("status: %v", err)
		}
		status, ok := out.(contracts.TranscodeV3Status)
		if !ok {
			t.Fatalf("status returned %T", out)
		}
		switch status.State {
		case want:
			return status
		case contracts.TranscodeFailed:
			t.Fatalf("session failed: %s (%s)", status.Error, status.ErrorCode)
		}
		time.Sleep(5 * time.Millisecond)
	}
	t.Fatalf("session %s did not reach %s", session, want)
	return contracts.TranscodeV3Status{}
}

// TestAdvancedPathsForFlatLayout pins the historical flat layout: an
// empty (or whitespace-only) temp path must keep every artifact under
// the data dir byte for byte.
func TestAdvancedPathsForFlatLayout(t *testing.T) {
	tr, _ := advTempFixture(t)
	session := "adv-flat"

	paths := tr.pathsFor(session, contracts.TranscodeSettings{})
	want := artifactPaths{
		media:    filepath.Join(tr.dir, session+".mp4"),
		hlsDir:   filepath.Join(tr.dir, session+".hls"),
		playlist: filepath.Join(tr.dir, session+".hls", "index.m3u8"),
		subtitle: filepath.Join(tr.dir, session+".vtt"),
		burn:     filepath.Join(tr.dir, session+".burn.ass"),
	}
	if paths != want {
		t.Fatalf("flat paths=%+v, want %+v", paths, want)
	}

	spaced := tr.pathsFor(session, contracts.TranscodeSettings{TranscodeTempPath: "   "})
	if spaced != want {
		t.Fatalf("whitespace temp path=%+v, want the flat layout %+v", spaced, want)
	}
}

// TestAdvancedPathsForRelocatedLayout pins the relocated layout: a
// session owns a directory under <temp>/lain-transcode while the
// playlist and subtitle stay in the same directory.
func TestAdvancedPathsForRelocatedLayout(t *testing.T) {
	tr, _ := advTempFixture(t)
	session := "adv-moved"
	settings := contracts.TranscodeSettings{TranscodeTempPath: "/tmp/x"}

	paths := tr.pathsFor(session, settings)
	dir := filepath.Join("/tmp/x", "lain-transcode", session)
	want := artifactPaths{
		media:    filepath.Join(dir, "media.mp4"),
		hlsDir:   dir,
		playlist: filepath.Join(dir, "index.m3u8"),
		subtitle: filepath.Join(dir, "subs.vtt"),
		burn:     filepath.Join(dir, "burn.ass"),
	}
	if paths != want {
		t.Fatalf("relocated paths=%+v, want %+v", paths, want)
	}

	// An entry without a recorded root keeps the historical flat layout.
	flat := tr.pathsForEntry(cacheEntry{Session: session})
	if flat.media != filepath.Join(tr.dir, session+".mp4") {
		t.Fatalf("entry without artifact_root=%+v, want the flat layout", flat)
	}

	// A sidecar-backed entry must reproduce the layout it was written to:
	// recordV3 stores relocateRoot(settings) in artifact_root, so
	// pathsForEntry must resolve the same directory pathsFor produced.
	recorded := cacheEntry{Session: session, ArtifactRoot: relocateRoot(settings)}
	if got := tr.pathsForEntry(recorded); got != want {
		t.Fatalf("pathsForEntry(%q)=%+v, want %+v", recorded.ArtifactRoot, got, want)
	}
}

// TestAdvancedEnsureRelocatedRoot pins admission-time verification: the
// owned directory is created, no writability probe is left behind, and
// a temp root that is not a directory fails immediately.
func TestAdvancedEnsureRelocatedRoot(t *testing.T) {
	root := t.TempDir()
	settings := contracts.DefaultTranscodeSettings()
	settings.TranscodeTempPath = root
	if err := ensureRelocatedRoot(settings); err != nil {
		t.Fatalf("ensureRelocatedRoot: %v", err)
	}
	relocated := filepath.Join(root, "lain-transcode")
	fi, err := os.Stat(relocated)
	if err != nil || !fi.IsDir() {
		t.Fatalf("relocated root missing: %v", err)
	}
	if _, err := os.Stat(filepath.Join(relocated, ".writable")); !os.IsNotExist(err) {
		t.Fatalf("writability probe left behind: %v", err)
	}

	// A temp root that is an existing file can never hold artifacts.
	fileRoot := filepath.Join(t.TempDir(), "not-a-dir")
	if err := os.WriteFile(fileRoot, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	settings.TranscodeTempPath = fileRoot
	if err := ensureRelocatedRoot(settings); err == nil {
		t.Fatal("ensureRelocatedRoot accepted a file as the temp root")
	}
}

// TestAdvancedRelocateRoot pins the owned-subdirectory derivation:
// whitespace is not a configuration, and surrounding spaces are trimmed.
func TestAdvancedRelocateRoot(t *testing.T) {
	if got := relocateRoot(contracts.TranscodeSettings{TranscodeTempPath: "   "}); got != "" {
		t.Fatalf("relocateRoot(whitespace)=%q, want empty", got)
	}
	got := relocateRoot(contracts.TranscodeSettings{TranscodeTempPath: "  /tmp/x  "})
	if want := filepath.Join("/tmp/x", "lain-transcode"); got != want {
		t.Fatalf("relocateRoot(spaced)=%q, want %q", got, want)
	}
}

// TestAdvancedRelocatedSessionLifecycle is the real-filesystem flow: a
// session with a temp path writes its artifact under the temp root, its
// sidecar records that root, and cancelling the finished session removes
// both together.
func TestAdvancedRelocatedSessionLifecycle(t *testing.T) {
	tr, src := advTempFixture(t)
	tempRoot := filepath.Join(t.TempDir(), "temp")
	settings := contracts.DefaultTranscodeSettings()
	settings.TranscodeTempPath = tempRoot

	// The injected run hook plays the role of ffmpeg; it has to write
	// exactly where the relocated layout says the artifact belongs.
	tr.v3run = func(j *job) error {
		paths := tr.pathsFor(j.spec.Session, j.settings)
		if err := os.MkdirAll(filepath.Dir(paths.media), 0o700); err != nil {
			return err
		}
		return os.WriteFile(paths.media, []byte("\x00\x00\x00\x18ftypmp42"), 0o600)
	}

	out, err := tr.Invoke(contracts.CapPlaybackTranscodeV3, contracts.TranscodeV3Request{
		Action:   contracts.TranscodeStartAction,
		FilePath: src,
		Delivery: contracts.TranscodeDeliveryProgressive,
		Settings: settings,
	})
	if err != nil {
		t.Fatalf("start: %v", err)
	}
	started, ok := out.(contracts.TranscodeV3Status)
	if !ok {
		t.Fatalf("start returned %T", out)
	}
	ready := advWaitV3(t, tr, src, started.Session, contracts.TranscodeReady)

	paths := tr.pathsFor(started.Session, settings)
	if !strings.HasPrefix(paths.media, filepath.Join(tempRoot, "lain-transcode")) {
		t.Fatalf("media path %q is not under the configured temp root", paths.media)
	}
	if ready.Path != paths.media {
		t.Fatalf("ready path=%q, want the relocated media %q", ready.Path, paths.media)
	}
	if _, err := os.Stat(paths.media); err != nil {
		t.Fatalf("artifact missing under the temp root: %v", err)
	}
	if _, err := os.Stat(filepath.Join(tr.dir, started.Session+".mp4")); !os.IsNotExist(err) {
		t.Fatalf("a flat artifact appeared next to the sidecar: %v", err)
	}

	sidecar := filepath.Join(tr.dir, started.Session+".json")
	raw, err := os.ReadFile(sidecar)
	if err != nil {
		t.Fatalf("sidecar missing in the data dir: %v", err)
	}
	// The sidecar must record where the artifacts live. recordV3
	// currently writes relocateRoot(settings) (the owned namespace), so
	// only "set" is asserted here; the pathsForEntry mismatch is a
	// separate finding.
	var entry cacheEntry
	if err := json.Unmarshal(raw, &entry); err != nil {
		t.Fatalf("sidecar: %v", err)
	}
	if entry.ArtifactRoot == "" {
		t.Fatal("sidecar has no artifact_root for a relocated session")
	}

	// Cancelling a finished session must drop the artifact directory
	// and its sidecar together.
	out, err = tr.Invoke(contracts.CapPlaybackTranscodeV3, contracts.TranscodeV3Request{
		Action: contracts.TranscodeCancelAction, Session: started.Session,
	})
	if err != nil {
		t.Fatalf("cancel: %v", err)
	}
	if status := out.(contracts.TranscodeV3Status); status.State != contracts.TranscodeIdle {
		t.Fatalf("cancel=%+v, want idle", status)
	}
	if _, err := os.Stat(filepath.Dir(paths.media)); !os.IsNotExist(err) {
		t.Fatalf("artifact directory survived the cancel: %v", err)
	}
	if _, err := os.Stat(sidecar); !os.IsNotExist(err) {
		t.Fatalf("sidecar survived the cancel: %v", err)
	}
}

// TestAdvancedSweepRelocatedOrphans pins the crash recovery scope: only
// unclaimed session directories under lain's own subdirectory are
// removed, sidecar-backed ones and operator files are never touched.
func TestAdvancedSweepRelocatedOrphans(t *testing.T) {
	tr, _ := advTempFixture(t)
	settings := contracts.DefaultTranscodeSettings()
	settings.TranscodeTempPath = filepath.Join(t.TempDir(), "temp")
	if err := ensureRelocatedRoot(settings); err != nil {
		t.Fatal(err)
	}
	root := relocateRoot(settings)

	orphan := filepath.Join(root, "orphan-session")
	if err := os.MkdirAll(orphan, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(orphan, "media.mp4"), []byte("stale"), 0o600); err != nil {
		t.Fatal(err)
	}

	kept := filepath.Join(root, "kept-session")
	if err := os.MkdirAll(kept, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(tr.metaPath("kept-session"), []byte("{}"), 0o600); err != nil {
		t.Fatal(err)
	}

	// A live session has a directory but no sidecar yet; it must be
	// excluded from the sweep so a second start cannot wipe it.
	live := filepath.Join(root, "live-session")
	if err := os.MkdirAll(live, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(live, "seg00000.m4s"), []byte("streaming"), 0o600); err != nil {
		t.Fatal(err)
	}

	operatorFile := filepath.Join(root, "operator-notes.txt")
	if err := os.WriteFile(operatorFile, []byte("not ours"), 0o600); err != nil {
		t.Fatal(err)
	}

	tr.sweepRelocatedOrphans(root, map[string]bool{"live-session": true})

	if _, err := os.Stat(orphan); !os.IsNotExist(err) {
		t.Fatalf("orphan directory survived the sweep: %v", err)
	}
	if _, err := os.Stat(kept); err != nil {
		t.Fatalf("claimed session directory was removed: %v", err)
	}
	if _, err := os.Stat(filepath.Join(live, "seg00000.m4s")); err != nil {
		t.Fatalf("live session artifact was removed by the sweep: %v", err)
	}
	if _, err := os.Stat(operatorFile); err != nil {
		t.Fatalf("operator file was touched: %v", err)
	}
}
