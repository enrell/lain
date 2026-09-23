package transcode

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-22

import (
	"fmt"
	"math"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
)

func hlsTestTranscoder(t *testing.T) *Transcoder {
	t.Helper()
	tr := newWithDeps(filepath.Join(t.TempDir(), "cache"), Config{}, nil, time.Now)
	t.Cleanup(func() { _ = tr.Close() })
	return tr
}

func writeTestFile(t *testing.T, path, body string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
}

func approx(a, b float64) bool { return math.Abs(a-b) < 1e-9 }

func segmentPlaylist(mediaSeq int, durations ...float64) string {
	var b strings.Builder
	b.WriteString("#EXTM3U\n#EXT-X-TARGETDURATION:4\n")
	if mediaSeq > 0 {
		fmt.Fprintf(&b, "#EXT-X-MEDIA-SEQUENCE:%d\n", mediaSeq)
	}
	b.WriteString("#EXT-X-MAP:URI=\"init.mp4\"\n")
	for i, d := range durations {
		fmt.Fprintf(&b, "#EXTINF:%.3f,\nseg%05d.m4s\n", d, mediaSeq+i)
	}
	b.WriteString("#EXT-X-ENDLIST\n")
	return b.String()
}

// TestParseHLSPlaylistReadsSegments pins the raw-playlist reader the
// whole HLS layer (playlist, throttle, deletion) depends on.
func TestParseHLSPlaylistReadsSegments(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "raw.m3u8")
	writeTestFile(t, path, strings.Join([]string{
		"#EXTM3U",
		"#EXT-X-VERSION:7",
		"#EXT-X-TARGETDURATION:4",
		"#EXT-X-MEDIA-SEQUENCE:2",
		"#EXT-X-MAP:URI=\"init.mp4\"",
		"#EXTINF:3.960,",
		"seg00002.m4s",
		"#EXTINF:4.000,",
		"seg00003.m4s",
		"#EXTINF:2.000,",
		"seg00004.m4s",
		"#EXT-X-ENDLIST",
		"",
	}, "\n"))

	segs, ended, err := parseHLSPlaylist(path)
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if !ended {
		t.Fatal("EXT-X-ENDLIST not detected")
	}
	if len(segs) != 3 {
		t.Fatalf("segments=%d, want 3", len(segs))
	}
	want := []hlsSegment{
		{URI: "seg00002.m4s", Duration: 3.96, MediaSeq: 2, StartSec: 0, EndSec: 3.96},
		{URI: "seg00003.m4s", Duration: 4.0, MediaSeq: 3, StartSec: 3.96, EndSec: 7.96},
		{URI: "seg00004.m4s", Duration: 2.0, MediaSeq: 4, StartSec: 7.96, EndSec: 9.96},
	}
	for i := range want {
		got := segs[i]
		if got.URI != want[i].URI || got.MediaSeq != want[i].MediaSeq ||
			!approx(got.Duration, want[i].Duration) ||
			!approx(got.StartSec, want[i].StartSec) || !approx(got.EndSec, want[i].EndSec) {
			t.Errorf("segment %d = %+v, want %+v", i, got, want[i])
		}
	}

	// Without ENDLIST the session is honestly unfinished.
	writeTestFile(t, path, "#EXTM3U\n#EXTINF:4.000,\nseg00000.m4s\n")
	segs, ended, err = parseHLSPlaylist(path)
	if err != nil || ended || len(segs) != 1 {
		t.Fatalf("unfinished playlist: segs=%d ended=%v err=%v", len(segs), ended, err)
	}

	if _, _, err := parseHLSPlaylist(filepath.Join(dir, "missing.m3u8")); err == nil {
		t.Fatal("a missing playlist must error")
	}
}

// TestWriteIndexPlaylistDropsMissingSegments proves the client-facing
// playlist never lists a file the deleter already removed.
func TestWriteIndexPlaylistDropsMissingSegments(t *testing.T) {
	tr := hlsTestTranscoder(t)
	session := "sess-index"
	dir := tr.pathsFor(session, contracts.TranscodeSettings{}).hlsDir
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	writeTestFile(t, filepath.Join(dir, hlsRawPlaylist), segmentPlaylist(3, 4, 4, 2))
	for _, name := range []string{hlsInitSegment, "seg00003.m4s", "seg00004.m4s", "seg00005.m4s"} {
		writeTestFile(t, filepath.Join(dir, name), "x")
	}
	if err := os.Remove(filepath.Join(dir, "seg00003.m4s")); err != nil {
		t.Fatal(err)
	}

	if err := writeIndexPlaylist(dir, 4); err != nil {
		t.Fatalf("writeIndexPlaylist: %v", err)
	}
	raw, err := os.ReadFile(filepath.Join(dir, hlsIndexPlaylist))
	if err != nil {
		t.Fatal(err)
	}
	index := string(raw)
	for _, want := range []string{
		"#EXTM3U",
		"#EXT-X-TARGETDURATION:4",
		"#EXT-X-MEDIA-SEQUENCE:4",
		"#EXT-X-MAP:URI=\"init.mp4\"",
		"seg00004.m4s",
		"seg00005.m4s",
		"#EXT-X-ENDLIST",
	} {
		if !strings.Contains(index, want) {
			t.Errorf("index playlist lacks %q:\n%s", want, index)
		}
	}
	if strings.Contains(index, "seg00003.m4s") {
		t.Errorf("deleted segment still listed:\n%s", index)
	}
	if !hlsPlayable(dir) {
		t.Fatal("index + init + raw segments must be playable")
	}
}

// TestHLSDiscontinuityRoundTrip proves a timeline break ffmpeg wrote is
// attributed to the segment that follows it and re-emitted in the served
// playlist: dropping it would let a client stitch across the timestamp
// jump as if the timeline were continuous.
func TestHLSDiscontinuityRoundTrip(t *testing.T) {
	tr := hlsTestTranscoder(t)
	session := "sess-disc"
	dir := tr.pathsFor(session, contracts.TranscodeSettings{}).hlsDir
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	raw := strings.Join([]string{
		"#EXTM3U",
		"#EXT-X-TARGETDURATION:4",
		"#EXT-X-MAP:URI=\"init.mp4\"",
		"#EXTINF:4.000,",
		"seg00000.m4s",
		"#EXT-X-DISCONTINUITY",
		"#EXTINF:4.000,",
		"seg00001.m4s",
		"#EXT-X-ENDLIST",
		"",
	}, "\n")
	writeTestFile(t, filepath.Join(dir, hlsRawPlaylist), raw)
	for _, name := range []string{hlsInitSegment, "seg00000.m4s", "seg00001.m4s"} {
		writeTestFile(t, filepath.Join(dir, name), "x")
	}

	segs, _, err := parseHLSPlaylist(filepath.Join(dir, hlsRawPlaylist))
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if len(segs) != 2 {
		t.Fatalf("segments=%d, want 2", len(segs))
	}
	// The tag applies to the segment that follows it, not the one before.
	if segs[0].IsDiscont || !segs[1].IsDiscont {
		t.Fatalf("discontinuity flags = [%v %v], want [false true]", segs[0].IsDiscont, segs[1].IsDiscont)
	}

	if err := writeIndexPlaylist(dir, 4); err != nil {
		t.Fatalf("writeIndexPlaylist: %v", err)
	}
	body, err := os.ReadFile(filepath.Join(dir, hlsIndexPlaylist))
	if err != nil {
		t.Fatal(err)
	}
	index := string(body)
	if strings.Count(index, "#EXT-X-DISCONTINUITY") != 1 {
		t.Fatalf("want exactly one discontinuity tag:\n%s", index)
	}
	tag := strings.Index(index, "#EXT-X-DISCONTINUITY")
	first := strings.Index(index, "seg00000.m4s")
	second := strings.Index(index, "seg00001.m4s")
	if tag < first || tag > second {
		t.Fatalf("discontinuity must sit between the two segments (first=%d tag=%d second=%d):\n%s", first, tag, second, index)
	}
}

// TestParseSegmentIndex pins the only file names the HLS endpoint may
// treat as progress signals.
func TestParseSegmentIndex(t *testing.T) {
	cases := []struct {
		name string
		want int
		ok   bool
	}{
		{"seg00000.m4s", 0, true},
		{"seg00042.m4s", 42, true},
		{"seg.m4s", 0, false},
		{"init.mp4", 0, false},
		{"raw.m3u8", 0, false},
		{"index.m3u8", 0, false},
		{"../x.m4s", 0, false},
	}
	for _, c := range cases {
		got, ok := parseSegmentIndex(c.name)
		if got != c.want || ok != c.ok {
			t.Errorf("parseSegmentIndex(%q)=(%d,%v), want (%d,%v)", c.name, got, ok, c.want, c.ok)
		}
	}
}

// TestHLSPureHelpers covers the path helpers throttling and deletion
// rely on, without a live ffmpeg process.
func TestHLSPureHelpers(t *testing.T) {
	tr := hlsTestTranscoder(t)
	session := "sess-helpers"
	dir := tr.pathsFor(session, contracts.TranscodeSettings{}).hlsDir
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	files := map[string]string{
		hlsRawPlaylist:   "#EXTM3U\n#EXTINF:4.000,\nseg00000.m4s\n#EXTINF:4.000,\nseg00001.m4s\n#EXT-X-ENDLIST\n",
		hlsIndexPlaylist: "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:4.000,\nseg00000.m4s\n#EXTINF:4.000,\nseg00001.m4s\n#EXT-X-ENDLIST\n",
		hlsInitSegment:   "init-data",
		"seg00000.m4s":   "seg0",
		"seg00001.m4s":   "seg1",
	}
	var wantSize int64
	for name, body := range files {
		writeTestFile(t, filepath.Join(dir, name), body)
		wantSize += int64(len(body))
	}
	// Subdirectories are neither served nor counted.
	if err := os.Mkdir(filepath.Join(dir, "nested"), 0o700); err != nil {
		t.Fatal(err)
	}

	if n, total := hlsProduced(dir); n != 2 || !approx(total, 8) {
		t.Fatalf("hlsProduced=(%d,%v), want (2,8)", n, total)
	}
	if got := clientPositionSec(dir, 0); !approx(got, 4) {
		t.Fatalf("clientPositionSec(0)=%v, want 4", got)
	}
	if got := clientPositionSec(dir, 1); !approx(got, 8) {
		t.Fatalf("clientPositionSec(1)=%v, want 8", got)
	}
	if got := clientPositionSec(dir, 2); !approx(got, 8) {
		t.Fatalf("clientPositionSec(2)=%v, want 8 (caught up) past the end", got)
	}
	if got := clientPositionSec(dir, -1); got != 0 {
		t.Fatalf("clientPositionSec(-1)=%v, want 0 for a negative index", got)
	}
	if !hlsPlayable(dir) {
		t.Fatal("session with index, init and segments must be playable")
	}
	if err := os.Remove(filepath.Join(dir, hlsInitSegment)); err != nil {
		t.Fatal(err)
	}
	if hlsPlayable(dir) {
		t.Fatal("a missing init segment must not be playable")
	}
	writeTestFile(t, filepath.Join(dir, hlsInitSegment), "init-data")

	// An index playlist with no retained segments is not playable even
	// when the raw ffmpeg playlist still lists segments: the index is
	// what is actually served.
	writeTestFile(t, filepath.Join(dir, hlsIndexPlaylist), "#EXTM3U\n")
	if hlsPlayable(dir) {
		t.Fatal("an empty index playlist must not be playable")
	}
	writeTestFile(t, filepath.Join(dir, hlsIndexPlaylist), "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:4.000,\nseg00000.m4s\n#EXTINF:4.000,\nseg00001.m4s\n#EXT-X-ENDLIST\n")

	if got := hlsDirSize(dir); got != wantSize {
		t.Fatalf("hlsDirSize=%d, want %d", got, wantSize)
	}
	want := []string{hlsIndexPlaylist, hlsInitSegment, hlsRawPlaylist, "seg00000.m4s", "seg00001.m4s"}
	if got := listHLSFiles(dir); !reflect.DeepEqual(got, want) {
		t.Fatalf("listHLSFiles=%v, want %v", got, want)
	}

	missing := filepath.Join(t.TempDir(), "absent")
	if n, total := hlsProduced(filepath.Join(t.TempDir(), "absent")); n != 0 || total != 0 {
		t.Fatalf("hlsProduced on a missing session=(%d,%v)", n, total)
	}
	if hlsDirSize(missing) != 0 || listHLSFiles(missing) != nil {
		t.Fatal("missing directories must report empty, not error")
	}
}

// TestDeleteConsumedSegmentsDropsBehindClient pins the opt-in deletion
// window: only segments fully behind the client minus the keep window
// disappear, and the index playlist is rewritten to match.
func TestDeleteConsumedSegmentsDropsBehindClient(t *testing.T) {
	tr := hlsTestTranscoder(t)
	session := "sess-delete"
	dir := tr.pathsFor(session, contracts.TranscodeSettings{}).hlsDir
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	writeTestFile(t, filepath.Join(dir, hlsRawPlaylist), segmentPlaylist(0, 4, 4, 4))
	for _, name := range []string{hlsInitSegment, "seg00000.m4s", "seg00001.m4s", "seg00002.m4s"} {
		writeTestFile(t, filepath.Join(dir, name), "x")
	}
	j := &job{
		spec:          sourceSpec{Session: session},
		delivery:      contracts.TranscodeDeliveryHLS,
		settings:      contracts.TranscodeSettings{SegmentDeletion: true, SegmentKeepSec: 0},
		clientSegment: 1,
	}
	tr.deleteConsumedSegments(j)

	for _, gone := range []string{"seg00000.m4s", "seg00001.m4s"} {
		if _, err := os.Stat(filepath.Join(dir, gone)); !os.IsNotExist(err) {
			t.Fatalf("consumed segment %s kept: %v", gone, err)
		}
	}
	if _, err := os.Stat(filepath.Join(dir, "seg00002.m4s")); err != nil {
		t.Fatalf("segment ahead of the client removed: %v", err)
	}
	raw, err := os.ReadFile(filepath.Join(dir, hlsIndexPlaylist))
	if err != nil {
		t.Fatal(err)
	}
	index := string(raw)
	if !strings.Contains(index, "#EXT-X-MEDIA-SEQUENCE:2") ||
		!strings.Contains(index, "seg00002.m4s") || strings.Contains(index, "seg00001.m4s") {
		t.Fatalf("index after deletion:\n%s", index)
	}

	// Deletion is opt-in: the setting off keeps every segment.
	keepSession := "sess-keep"
	keepDir := tr.pathsFor(keepSession, contracts.TranscodeSettings{}).hlsDir
	if err := os.MkdirAll(keepDir, 0o700); err != nil {
		t.Fatal(err)
	}
	writeTestFile(t, filepath.Join(keepDir, hlsRawPlaylist), segmentPlaylist(0, 4, 4, 4))
	writeTestFile(t, filepath.Join(keepDir, "seg00000.m4s"), "x")
	off := &job{
		spec:          sourceSpec{Session: keepSession},
		delivery:      contracts.TranscodeDeliveryHLS,
		clientSegment: 2,
	}
	tr.deleteConsumedSegments(off)
	if _, err := os.Stat(filepath.Join(keepDir, "seg00000.m4s")); err != nil {
		t.Fatalf("segment deletion disabled but file removed: %v", err)
	}
}

// makeAVMKV synthesizes a small Matroska with H.264 video and AAC
// audio. It differs from makeMKV because the shared helper puts codec
// flags between its two inputs, which ffmpeg reads as input options.
func makeAVMKV(t *testing.T, dir, name string) string {
	t.Helper()
	requireFFmpeg(t)
	out := filepath.Join(dir, name)
	args := []string{"-hide_banner", "-loglevel", "error",
		"-f", "lavfi", "-i", "testsrc=size=160x120:rate=10:duration=2",
		"-f", "lavfi", "-i", "sine=frequency=440:sample_rate=22050:duration=2",
		"-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac",
		"-y", out}
	if combo, err := exec.Command("ffmpeg", args...).CombinedOutput(); err != nil {
		t.Skipf("cannot synthesize mkv: %v: %s", err, combo)
	}
	return out
}

// TestV3HLSSessionProducesPlayableOutput is the real packaging slice: a
// synthesized MKV runs through the public v3 API and must yield a
// finished fMP4 playlist with an init segment and media segments.
func TestV3HLSSessionProducesPlayableOutput(t *testing.T) {
	requireFFmpeg(t)
	if _, err := exec.LookPath("ffprobe"); err != nil {
		t.Skip("ffprobe not installed")
	}
	root := t.TempDir()
	src := makeAVMKV(t, root, "[Fansub-A] Show.mkv")
	tr := New(filepath.Join(root, "cache"))
	t.Cleanup(func() { _ = tr.Close() })

	out, err := tr.Invoke(contracts.CapPlaybackTranscodeV3, contracts.TranscodeV3Request{
		Action:   contracts.TranscodeStartAction,
		FilePath: src,
		Delivery: contracts.TranscodeDeliveryHLS,
		Settings: contracts.DefaultTranscodeSettings(),
	})
	if err != nil {
		t.Fatalf("start: %v", err)
	}
	status := out.(contracts.TranscodeV3Status)
	if status.Session == "" {
		t.Fatal("start returned no session")
	}

	deadline := time.Now().Add(60 * time.Second)
	for status.State != contracts.TranscodeReady && time.Now().Before(deadline) {
		time.Sleep(25 * time.Millisecond)
		out, err := tr.Invoke(contracts.CapPlaybackTranscodeV3, contracts.TranscodeV3Request{
			Action:   contracts.TranscodeStatusAction,
			FilePath: src,
			Session:  status.Session,
		})
		if err != nil {
			t.Fatalf("status: %v", err)
		}
		status = out.(contracts.TranscodeV3Status)
	}
	if status.State != contracts.TranscodeReady {
		t.Fatalf("session did not become ready: %+v", status)
	}
	if !status.Playable || status.PlaylistPath == "" {
		t.Fatalf("ready status=%+v, want a playable playlist", status)
	}

	dir := filepath.Dir(status.PlaylistPath)
	index, err := os.ReadFile(status.PlaylistPath)
	if err != nil {
		t.Fatalf("read playlist: %v", err)
	}
	if !strings.HasPrefix(string(index), "#EXTM3U") || !strings.Contains(string(index), "#EXT-X-ENDLIST") {
		t.Fatalf("finished playlist:\n%s", index)
	}
	initRaw, err := os.ReadFile(filepath.Join(dir, hlsInitSegment))
	if err != nil {
		t.Fatalf("read init segment: %v", err)
	}
	if len(initRaw) < 8 || string(initRaw[4:8]) != "ftyp" {
		t.Fatalf("init segment is not fMP4: % x", initRaw[:min(8, len(initRaw))])
	}
	segments := 0
	for _, name := range listHLSFiles(dir) {
		if _, ok := parseSegmentIndex(name); ok {
			segments++
		}
	}
	if segments == 0 {
		t.Fatalf("no media segments in %v", listHLSFiles(dir))
	}
}
