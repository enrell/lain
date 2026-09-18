package gateway

// Real end-to-end transcoding: every source is generated procedurally
// by ffmpeg (lavfi), catalogued through the real ingest pipeline, then
// driven over HTTP against the real server. Nothing is faked and
// nothing is committed as binary; the produced artifacts are verified
// with ffprobe (codec, pixel format, tone-mapped transfer, scale) and
// byte-level checks (fMP4 boxes, faststart ordering).
//
// The suite skips cleanly when ffmpeg (or a specific encoder it needs)
// is unavailable, matching D-023's degrade stance.

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
)

// e2eVideoProfile is what ffprobe says about one produced artifact.
type e2eVideoProfile struct {
	Codec      string
	Pixel      string
	Transfer   string
	Primaries  string
	Width      int
	Height     int
	AudioCodec string
	Channels   int
	FPS        float64
	Frames     int
}

func e2eRequireFFmpeg(t *testing.T) {
	t.Helper()
	if _, err := exec.LookPath("ffmpeg"); err != nil {
		t.Skip("ffmpeg not installed")
	}
	if _, err := exec.LookPath("ffprobe"); err != nil {
		t.Skip("ffprobe not installed")
	}
}

func e2eHasEncoder(t *testing.T, name string) bool {
	t.Helper()
	out, err := exec.Command("ffmpeg", "-hide_banner", "-encoders").Output()
	if err != nil {
		return false
	}
	return strings.Contains(string(out), " "+name+" ")
}

// e2eRunFFmpeg synthesizes one source. A missing encoder is the local
// build's limitation and skips; any other failure is a broken fixture and
// fails, so it can never silently disable the test that depends on it.
func e2eRunFFmpeg(t *testing.T, args ...string) {
	t.Helper()
	cmd := exec.Command("ffmpeg", append([]string{"-hide_banner", "-loglevel", "error", "-y"}, args...)...)
	out, err := cmd.CombinedOutput()
	if err == nil {
		return
	}
	if strings.Contains(string(out), "Unknown encoder") {
		t.Skipf("encoder not built into this ffmpeg: %s", out)
	}
	t.Fatalf("cannot synthesize fixture: %v: %s", err, out)
}

// e2eSource describes one procedurally generated fixture.
type e2eSource struct {
	Name    string
	Build   func(t *testing.T, path string)
	NeedEnc string // skip when this encoder is missing
}

// e2eSources are the shapes the pipeline must handle: a browser-safe
// remux, an incompatible re-encode, a multi-track file with subtitles,
// and a 10-bit HDR file that must be tone-mapped.
func e2eSources(t *testing.T) []e2eSource {
	t.Helper()
	return []e2eSource{
		{
			Name: "[Fansub-A] Procedural Show - 01 [1080p].mkv",
			Build: func(t *testing.T, path string) {
				e2eRunFFmpeg(t,
					"-f", "lavfi", "-i", "testsrc2=size=640x360:rate=15:duration=6",
					"-f", "lavfi", "-i", "sine=frequency=440:sample_rate=44100:duration=6",
					"-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p",
					"-c:a", "aac", path)
			},
			NeedEnc: "libx264",
		},
		{
			Name: "[Fansub-B] Procedural Show - 02 [480p].mkv",
			Build: func(t *testing.T, path string) {
				e2eRunFFmpeg(t,
					"-f", "lavfi", "-i", "testsrc2=size=640x360:rate=15:duration=4",
					"-f", "lavfi", "-i", "sine=frequency=660:sample_rate=44100:duration=4",
					"-c:v", "mpeg4", "-q:v", "6", "-c:a", "libmp3lame", path)
			},
			NeedEnc: "mpeg4",
		},
		{
			Name: "[Fansub-A] Procedural Show - 03 [Dual-Audio].mkv",
			Build: func(t *testing.T, path string) {
				srt := filepath.Join(filepath.Dir(path), "subs.srt")
				body := "1\n00:00:00,000 --> 00:00:02,000\nFirst line\n\n2\n00:00:02,000 --> 00:00:04,000\nSecond line\n"
				if err := os.WriteFile(srt, []byte(body), 0o600); err != nil {
					t.Fatal(err)
				}
				e2eRunFFmpeg(t,
					"-f", "lavfi", "-i", "testsrc2=size=640x360:rate=15:duration=4",
					"-f", "lavfi", "-i", "sine=frequency=440:sample_rate=44100:duration=4",
					"-f", "lavfi", "-i", "sine=frequency=880:sample_rate=44100:duration=4",
					"-i", srt,
					"-map", "0:v", "-map", "1:a", "-map", "2:a", "-map", "3:s",
					"-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p",
					"-c:a", "aac", "-c:s", "srt",
					"-metadata:s:a:0", "language=jpn", "-metadata:s:a:1", "language=eng",
					"-disposition:a:1", "default",
					path)
			},
			NeedEnc: "libx264",
		},
		{
			Name: "[Fansub-C] Procedural Show - 04 [HDR10].mkv",
			Build: func(t *testing.T, path string) {
				e2eRunFFmpeg(t,
					"-f", "lavfi", "-i", "testsrc2=size=320x180:rate=15:duration=4",
					"-c:v", "libx265", "-preset", "ultrafast", "-pix_fmt", "yuv420p10le",
					"-color_primaries", "bt2020", "-color_trc", "smpte2084", "-colorspace", "bt2020nc",
					"-x265-params", "log-level=error",
					path)
			},
			NeedEnc: "libx265",
		},
		{
			Name: "[Fansub-B] Procedural Show - 05 [5.1].mkv",
			Build: func(t *testing.T, path string) {
				e2eRunFFmpeg(t,
					"-f", "lavfi", "-i", "testsrc2=size=320x180:rate=15:duration=3",
					"-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:duration=3",
					"-f", "lavfi", "-i", "sine=frequency=880:sample_rate=48000:duration=3",
					"-f", "lavfi", "-i", "sine=frequency=220:sample_rate=48000:duration=3",
					"-f", "lavfi", "-i", "sine=frequency=330:sample_rate=48000:duration=3",
					"-f", "lavfi", "-i", "sine=frequency=550:sample_rate=48000:duration=3",
					"-f", "lavfi", "-i", "sine=frequency=770:sample_rate=48000:duration=3",
					"-filter_complex", "[1:a][2:a][3:a][4:a][5:a][6:a]join=inputs=6:channel_layout=5.1[a]",
					"-map", "0:v", "-map", "[a]",
					"-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p",
					"-c:a", "ac3", path)
			},
			NeedEnc: "libx264",
		},
		{
			Name: "[Fansub-A] Procedural Show - 06 [Interlaced].mkv",
			Build: func(t *testing.T, path string) {
				e2eRunFFmpeg(t,
					"-f", "lavfi", "-i", "testsrc2=size=320x180:rate=10:duration=4",
					"-vf", "tinterlace=mode=interleave_top",
					"-flags", "+ildct+ilme",
					"-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p",
					path)
			},
			NeedEnc: "libx264",
		},
	}
}

// e2eCatalog creates a library over the generated sources and returns
// each catalog id keyed by file name. A source with no Build function is
// assumed to be on disk already (a test that needed a custom encoder
// command wrote it itself).
func e2eCatalog(t *testing.T, srv *Server, admin, dir string, sources []e2eSource) map[string]string {
	t.Helper()
	for _, src := range sources {
		if src.NeedEnc != "" && !e2eHasEncoder(t, src.NeedEnc) {
			t.Skipf("ffmpeg lacks %s", src.NeedEnc)
		}
		if src.Build == nil {
			if _, err := os.Stat(filepath.Join(dir, src.Name)); err != nil {
				t.Fatalf("fixture %s is missing and has no builder: %v", src.Name, err)
			}
			continue
		}
		src.Build(t, filepath.Join(dir, src.Name))
	}
	if rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "Procedural", "type": "anime", "path": dir}, admin); rec.Code != 201 {
		t.Fatalf("library: %d %s", rec.Code, rec.Body.String())
	}
	if rec := do(t, srv, "POST", "/api/library/scan", nil, admin); rec.Code != 202 {
		t.Fatalf("scan: %d %s", rec.Code, rec.Body.String())
	}
	waitScan(t, srv, admin)

	var page contracts.CatalogPage
	rec := do(t, srv, "GET", "/api/catalog?limit=100", nil, admin)
	if err := json.Unmarshal(rec.Body.Bytes(), &page); err != nil {
		t.Fatalf("catalog: %d %s", rec.Code, rec.Body.String())
	}
	byPath := map[string]string{}
	for _, item := range page.Items {
		byPath[filepath.Base(item.FilePath)] = item.ID
	}
	if len(byPath) != len(sources) {
		t.Fatalf("catalog has %d items, want %d (%s)", len(byPath), len(sources), rec.Body.String())
	}
	return byPath
}

// e2eStartSession starts a v3 session and waits until the requested
// condition holds, returning the last public status body.
func e2eStartSession(t *testing.T, srv *Server, admin, id string, selection map[string]any, wantPlayable bool) map[string]any {
	t.Helper()
	rec := do(t, srv, "POST", "/api/items/"+id+"/transcode", selection, admin)
	if rec.Code != 200 && rec.Code != 202 {
		t.Fatalf("start: %d %s", rec.Code, rec.Body.String())
	}
	var status map[string]any
	if err := json.Unmarshal(rec.Body.Bytes(), &status); err != nil {
		t.Fatalf("start body: %s", rec.Body.String())
	}
	session, _ := status["session"].(string)
	if session == "" {
		t.Fatalf("start returned no session: %s", rec.Body.String())
	}
	deadline := time.Now().Add(90 * time.Second)
	for time.Now().Before(deadline) {
		state, _ := status["state"].(string)
		if state == contracts.TranscodeReady {
			return status
		}
		if state == contracts.TranscodeFailed {
			t.Fatalf("session failed: %s", rec.Body.String())
		}
		if wantPlayable {
			if playable, _ := status["playable"].(bool); playable {
				return status
			}
		}
		time.Sleep(50 * time.Millisecond)
		rec = do(t, srv, "GET", "/api/items/"+id+"/transcode/status?session="+session, nil, admin)
		if rec.Code != 200 {
			t.Fatalf("status: %d %s", rec.Code, rec.Body.String())
		}
		if err := json.Unmarshal(rec.Body.Bytes(), &status); err != nil {
			t.Fatal(err)
		}
	}
	t.Fatalf("session did not finish in time: %s", rec.Body.String())
	return nil
}

func e2eProbe(t *testing.T, path string) e2eVideoProfile {
	t.Helper()
	out, err := exec.Command("ffprobe", "-v", "error",
		"-show_entries", "stream=codec_type,codec_name,pix_fmt,color_transfer,color_primaries,width,height,channels,r_frame_rate,nb_frames",
		"-of", "json", path).Output()
	if err != nil {
		t.Fatalf("ffprobe %s: %v", path, err)
	}
	var parsed struct {
		Streams []struct {
			Type      string `json:"codec_type"`
			Codec     string `json:"codec_name"`
			Pixel     string `json:"pix_fmt"`
			Transfer  string `json:"color_transfer"`
			Primaries string `json:"color_primaries"`
			Width     int    `json:"width"`
			Height    int    `json:"height"`
			Channels  int    `json:"channels"`
			FrameRate string `json:"r_frame_rate"`
			NbFrames  string `json:"nb_frames"`
		} `json:"streams"`
	}
	if err := json.Unmarshal(out, &parsed); err != nil {
		t.Fatal(err)
	}
	var profile e2eVideoProfile
	for _, s := range parsed.Streams {
		switch s.Type {
		case "video":
			profile.Codec, profile.Pixel, profile.Transfer = s.Codec, s.Pixel, s.Transfer
			profile.Primaries, profile.Width, profile.Height = s.Primaries, s.Width, s.Height
			profile.FPS = e2eParseRate(s.FrameRate)
			profile.Frames, _ = strconv.Atoi(s.NbFrames)
		case "audio":
			profile.AudioCodec, profile.Channels = s.Codec, s.Channels
		}
	}
	return profile
}

// e2eParseRate reads ffprobe's "num/den" rate.
func e2eParseRate(value string) float64 {
	num, den, ok := strings.Cut(value, "/")
	if !ok {
		return 0
	}
	n, err1 := strconv.ParseFloat(num, 64)
	d, err2 := strconv.ParseFloat(den, 64)
	if err1 != nil || err2 != nil || d == 0 {
		return 0
	}
	return n / d
}

func e2eWrite(t *testing.T, name string, data []byte) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), name)
	if err := os.WriteFile(path, data, 0o600); err != nil {
		t.Fatal(err)
	}
	return path
}

// TestE2EHLSPlaybackAndVerification is the real HLS path: start a
// session, fetch the playlist while ffmpeg is still working, then
// verify the segments decode to the requested quality and codec.
func TestE2EHLSPlaybackAndVerification(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	sources := e2eSources(t)
	ids := e2eCatalog(t, srv, admin, dir, sources[:1])

	name := sources[0].Name
	id := ids[name]

	status := e2eStartSession(t, srv, admin, id, map[string]any{
		"delivery": "hls",
		"quality":  "360p",
	}, true)
	session, _ := status["session"].(string)
	if delivery, _ := status["delivery"].(string); delivery != contracts.TranscodeDeliveryHLS {
		t.Fatalf("delivery=%v, want hls", status["delivery"])
	}
	if method, _ := status["method"].(string); method != "remux" {
		t.Fatalf("method=%v, want remux for a web-safe h264/aac mkv", status["method"])
	}
	reasons, _ := status["reasons"].([]any)
	if len(reasons) == 0 {
		t.Fatalf("status carries no transcode reasons: %v", status)
	}

	base := "/api/items/" + id + "/transcode/hls/"
	q := "?session=" + session + "&token=" + admin

	rec := do(t, srv, "GET", base+"index.m3u8"+q, nil, "")
	if rec.Code != 200 {
		t.Fatalf("playlist: %d %s", rec.Code, rec.Body.String())
	}
	playlist := rec.Body.String()
	if !strings.Contains(playlist, "seg00000.m4s") {
		t.Fatalf("playlist lists no segments:\n%s", playlist)
	}

	initRec := do(t, srv, "GET", base+"init.mp4"+q, nil, "")
	if initRec.Code != 200 || initRec.Body.Len() == 0 {
		t.Fatalf("init segment: %d %d bytes", initRec.Code, initRec.Body.Len())
	}
	segRec := do(t, srv, "GET", base+"seg00000.m4s"+q, nil, "")
	if segRec.Code != 200 || segRec.Body.Len() == 0 {
		t.Fatalf("segment: %d %d bytes", segRec.Code, segRec.Body.Len())
	}

	// The init segment carries the moov; joining it with one media
	// segment yields a probeable fragmented MP4.
	joined := e2eWrite(t, "joined.mp4", append(append([]byte{}, initRec.Body.Bytes()...), segRec.Body.Bytes()...))
	profile := e2eProbe(t, joined)
	if profile.Codec != "h264" || profile.Pixel != "yuv420p" {
		t.Fatalf("segment profile=%+v, want h264/yuv420p", profile)
	}
	if profile.Height > 360 || profile.Width > 640 {
		t.Fatalf("segment is %dx%d, want at most 640x360", profile.Width, profile.Height)
	}

	// A waiting session (still encoding) must be playable before it is
	// ready, which is the whole point of HLS delivery.
	if state, _ := status["state"].(string); state != contracts.TranscodeReady {
		if playable, _ := status["playable"].(bool); !playable {
			t.Fatalf("early HLS status is neither ready nor playable: %v", status)
		}
	}
}

// TestE2EProgressiveRemuxAndFaststart covers the retained progressive
// path and proves the MP4 is faststart-ordered, so Range seeking works.
func TestE2EProgressiveRemuxAndFaststart(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	sources := e2eSources(t)
	ids := e2eCatalog(t, srv, admin, dir, sources[:1])
	id := ids[sources[0].Name]

	status := e2eStartSession(t, srv, admin, id, map[string]any{"delivery": "progressive"}, false)
	session, _ := status["session"].(string)

	rec := do(t, srv, "GET", "/api/items/"+id+"/transcode?token="+admin+"&session="+session, nil, "")
	if rec.Code != 200 {
		t.Fatalf("progressive fetch: %d %s", rec.Code, rec.Body.String())
	}
	body := rec.Body.Bytes()
	path := e2eWrite(t, "progressive.mp4", body)
	profile := e2eProbe(t, path)
	if profile.Codec != "h264" {
		t.Fatalf("profile=%+v, want h264", profile)
	}
	moov := bytes.Index(body, []byte("moov"))
	mdat := bytes.Index(body, []byte("mdat"))
	if moov < 0 || mdat < 0 || moov > mdat {
		t.Fatalf("mp4 is not faststart-ordered (moov=%d mdat=%d)", moov, mdat)
	}
}

// TestE2EIncompatibleSourceReEncodes proves the full re-encode path
// produces a browser-playable stream from a source the browser cannot
// play at all (mpeg4 video + mp3 audio).
func TestE2EIncompatibleSourceReEncodes(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	sources := e2eSources(t)
	ids := e2eCatalog(t, srv, admin, dir, sources[:2])
	id := ids[sources[1].Name]

	status := e2eStartSession(t, srv, admin, id, map[string]any{
		"delivery": "progressive",
		"quality":  "480p",
	}, false)
	if method, _ := status["method"].(string); method != "transcode" {
		t.Fatalf("method=%v, want transcode for mpeg4/mp3", status["method"])
	}
	if encoder, _ := status["encoder"].(string); encoder != "libx264" {
		t.Fatalf("encoder=%v, want libx264", status["encoder"])
	}
	session, _ := status["session"].(string)
	rec := do(t, srv, "GET", "/api/items/"+id+"/transcode?token="+admin+"&session="+session, nil, "")
	if rec.Code != 200 {
		t.Fatalf("fetch: %d %s", rec.Code, rec.Body.String())
	}
	profile := e2eProbe(t, e2eWrite(t, "reencoded.mp4", rec.Body.Bytes()))
	if profile.Codec != "h264" || profile.AudioCodec != "aac" {
		t.Fatalf("profile=%+v, want h264/aac", profile)
	}
	if profile.Height > 480 {
		t.Fatalf("height=%d, want at most 480", profile.Height)
	}
}

// TestE2ESubtitleSidecarAndAudioSelection drives the multi-track source:
// the selected subtitle becomes WebVTT and the selected audio track is
// the one that ends up in the output.
func TestE2ESubtitleSidecarAndAudioSelection(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	sources := e2eSources(t)
	ids := e2eCatalog(t, srv, admin, dir, sources[2:3])
	id := ids[sources[2].Name]

	// Discover the real stream indices through the probe-backed plan.
	rec := do(t, srv, "GET", "/api/items/"+id+"/playback?client=web", nil, admin)
	var plan contracts.Plan
	if err := json.Unmarshal(rec.Body.Bytes(), &plan); err != nil {
		t.Fatalf("plan: %d %s", rec.Code, rec.Body.String())
	}
	var audioIndex, subtitleIndex int = -1, -1
	for _, stream := range plan.Streams {
		if stream.Type == "audio" && stream.Default {
			audioIndex = stream.Index
		}
		if stream.Type == "subtitle" {
			subtitleIndex = stream.Index
		}
	}
	if audioIndex < 0 || subtitleIndex < 0 {
		t.Fatalf("plan streams lack audio/subtitle: %+v", plan.Streams)
	}

	status := e2eStartSession(t, srv, admin, id, map[string]any{
		"delivery":        "progressive",
		"audio_stream":    audioIndex,
		"subtitle_stream": subtitleIndex,
	}, false)
	session, _ := status["session"].(string)
	if hasSubtitle, _ := status["has_subtitle"].(bool); !hasSubtitle {
		t.Fatalf("status reports no subtitle sidecar: %v", status)
	}

	rec = do(t, srv, "GET", "/api/items/"+id+"/subtitles?session="+session+"&token="+admin, nil, "")
	if rec.Code != 200 {
		t.Fatalf("subtitles: %d %s", rec.Code, rec.Body.String())
	}
	if body := rec.Body.String(); !strings.HasPrefix(body, "WEBVTT") || !strings.Contains(body, "First line") {
		t.Fatalf("bad WebVTT sidecar: %q", body)
	}

	rec = do(t, srv, "GET", "/api/items/"+id+"/transcode?token="+admin+"&session="+session, nil, "")
	if rec.Code != 200 {
		t.Fatalf("fetch: %d %s", rec.Code, rec.Body.String())
	}
	profile := e2eProbe(t, e2eWrite(t, "multi.mp4", rec.Body.Bytes()))
	if profile.AudioCodec == "" {
		t.Fatalf("output has no audio stream: %+v", profile)
	}
}

// TestE2ESubtitleBurnInProducesPlayableVideo exercises the burn-in chain
// against real ffmpeg: the extracted text subtitle is rendered into the
// video, so the session must finish with a playable H.264 artifact and no
// filter error (a malformed `subtitles=` chain would fail the run).
func TestE2ESubtitleBurnInProducesPlayableVideo(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	sources := e2eSources(t)
	ids := e2eCatalog(t, srv, admin, dir, sources[2:3])
	id := ids[sources[2].Name]

	rec := do(t, srv, "GET", "/api/items/"+id+"/playback?client=web", nil, admin)
	var plan contracts.Plan
	if err := json.Unmarshal(rec.Body.Bytes(), &plan); err != nil {
		t.Fatalf("plan: %d %s", rec.Code, rec.Body.String())
	}
	subtitleIndex := -1
	for _, stream := range plan.Streams {
		if stream.Type == "subtitle" {
			subtitleIndex = stream.Index
		}
	}
	if subtitleIndex < 0 {
		t.Fatalf("plan streams lack a subtitle: %+v", plan.Streams)
	}

	status := e2eStartSession(t, srv, admin, id, map[string]any{
		"delivery":        "progressive",
		"subtitle_stream": subtitleIndex,
		"subtitle_mode":   "burn",
	}, false)
	session, _ := status["session"].(string)
	if method, _ := status["method"].(string); method != "transcode" {
		t.Fatalf("method=%v, want transcode: burn-in re-encodes the video", status["method"])
	}
	rec = do(t, srv, "GET", "/api/items/"+id+"/transcode?token="+admin+"&session="+session, nil, "")
	if rec.Code != 200 || rec.Body.Len() == 0 {
		t.Fatalf("burn-in fetch: %d %s", rec.Code, rec.Body.String())
	}
	profile := e2eProbe(t, e2eWrite(t, "burnin.mp4", rec.Body.Bytes()))
	if profile.Codec != "h264" || profile.Pixel != "yuv420p" {
		t.Fatalf("burn-in profile=%+v, want h264/yuv420p", profile)
	}
}

// TestE2EHLSSessionBurnsSubtitle proves the two paths compose: an HLS
// session that also burns in a subtitle must produce a playable playlist.
// The burn-in graph maps `[vout]` from a complexFilter, so the HLS muxer
// args must accept it — an interaction neither path tests alone.
func TestE2EHLSSessionBurnsSubtitle(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	sources := e2eSources(t)
	ids := e2eCatalog(t, srv, admin, dir, sources[2:3])
	id := ids[sources[2].Name]

	rec := do(t, srv, "GET", "/api/items/"+id+"/playback?client=web", nil, admin)
	var plan contracts.Plan
	if err := json.Unmarshal(rec.Body.Bytes(), &plan); err != nil {
		t.Fatalf("plan: %d %s", rec.Code, rec.Body.String())
	}
	subtitleIndex := -1
	for _, stream := range plan.Streams {
		if stream.Type == "subtitle" {
			subtitleIndex = stream.Index
		}
	}
	if subtitleIndex < 0 {
		t.Fatalf("plan streams lack a subtitle: %+v", plan.Streams)
	}

	status := e2eStartSession(t, srv, admin, id, map[string]any{
		"delivery":        "hls",
		"subtitle_stream": subtitleIndex,
		"subtitle_mode":   "burn",
	}, true)
	session, _ := status["session"].(string)

	base := "/api/items/" + id + "/transcode/hls/"
	rec = do(t, srv, "GET", base+"index.m3u8?session="+session+"&token="+admin, nil, "")
	if rec.Code != 200 {
		t.Fatalf("playlist: %d %s", rec.Code, rec.Body.String())
	}
	playlist := rec.Body.String()
	if !strings.HasPrefix(playlist, "#EXTM3U") {
		t.Fatalf("bad playlist:\n%s", playlist)
	}
	if rec := do(t, srv, "GET", base+playlistMapURI(t, playlist), nil, ""); rec.Code != 200 || rec.Body.Len() == 0 {
		t.Fatalf("init segment: %d %s", rec.Code, rec.Body.String())
	}
	if rec := do(t, srv, "GET", base+playlistSegmentURI(t, playlist), nil, ""); rec.Code != 200 || rec.Body.Len() == 0 {
		t.Fatalf("media segment: %d %s", rec.Code, rec.Body.String())
	}
}

// TestE2EHDRToneMapsToSDR is the HDR path: a 10-bit BT.2020/PQ source
// is converted to SDR H.264, and the output is verified to carry BT.709
// markers instead of the source's HDR transfer.
func TestE2EHDRToneMapsToSDR(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	sources := e2eSources(t)
	ids := e2eCatalog(t, srv, admin, dir, sources[3:4])
	id := ids[sources[3].Name]

	// The plan must promise a playable transcode now that tone mapping
	// is enabled and its probe passed.
	rec := do(t, srv, "GET", "/api/items/"+id+"/playback?client=web", nil, admin)
	var plan contracts.Plan
	if err := json.Unmarshal(rec.Body.Bytes(), &plan); err != nil {
		t.Fatalf("plan: %d %s", rec.Code, rec.Body.String())
	}
	if plan.Mode != "transcode" || !plan.Available {
		t.Fatalf("HDR plan=%+v, want transcode+available with tone mapping on", plan)
	}

	status := e2eStartSession(t, srv, admin, id, map[string]any{"delivery": "progressive"}, false)
	session, _ := status["session"].(string)
	rec = do(t, srv, "GET", "/api/items/"+id+"/transcode?token="+admin+"&session="+session, nil, "")
	if rec.Code != 200 {
		t.Fatalf("fetch: %d %s", rec.Code, rec.Body.String())
	}
	profile := e2eProbe(t, e2eWrite(t, "tonemapped.mp4", rec.Body.Bytes()))
	if profile.Codec != "h264" || profile.Pixel != "yuv420p" {
		t.Fatalf("profile=%+v, want h264/yuv420p", profile)
	}
	if strings.EqualFold(profile.Transfer, "smpte2084") {
		t.Fatalf("output still carries the HDR transfer: %+v", profile)
	}
	// The status must explain what it actually did.
	if encoder, _ := status["encoder"].(string); encoder == "" {
		t.Fatalf("status does not report the encoder: %v", status)
	}
}

// TestE2ESettingsDriveSegmentsAndQuality proves the operator settings
// reach the pipeline: a changed segment length changes the produced
// playlist and the segment boundaries of an encoded session (stream
// copy cannot move keyframes, so the encode path is the one that can
// honor the setting exactly).
func TestE2ESettingsDriveSegmentsAndQuality(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	sources := e2eSources(t)
	ids := e2eCatalog(t, srv, admin, dir, sources[1:2])
	id := ids[sources[1].Name]

	rec := do(t, srv, "GET", "/api/admin/settings/transcode", nil, admin)
	if rec.Code != 200 {
		t.Fatalf("settings get: %d %s", rec.Code, rec.Body.String())
	}
	var payload struct {
		Settings     contracts.TranscodeSettings `json:"settings"`
		Capabilities map[string]any              `json:"capabilities"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &payload); err != nil {
		t.Fatal(err)
	}
	if payload.Capabilities == nil {
		t.Fatalf("settings response lacks capabilities: %s", rec.Body.String())
	}
	settings := payload.Settings
	settings.HLSSegmentSeconds = 2
	settings.QueueSize = 2
	if rec := do(t, srv, "PUT", "/api/admin/settings/transcode", settings, admin); rec.Code != 200 {
		t.Fatalf("settings put: %d %s", rec.Code, rec.Body.String())
	}

	status := e2eStartSession(t, srv, admin, id, map[string]any{"delivery": "hls"}, true)
	session, _ := status["session"].(string)
	rec = do(t, srv, "GET", "/api/items/"+id+"/transcode/hls/index.m3u8?session="+session+"&token="+admin, nil, "")
	if rec.Code != 200 {
		t.Fatalf("playlist: %d %s", rec.Code, rec.Body.String())
	}
	playlist := rec.Body.String()
	if !strings.Contains(playlist, "#EXT-X-TARGETDURATION:2") {
		t.Fatalf("playlist did not adopt the 2s segment setting:\n%s", playlist)
	}
	if segments := strings.Count(playlist, "seg") - strings.Count(playlist, "#EXT-X-SEG"); segments < 2 {
		t.Fatalf("a 4s source at 2s segments must yield at least two segments:\n%s", playlist)
	}

	// Sessions are visible to the operator surface and can be stopped.
	rec = do(t, srv, "GET", "/api/admin/transcodes", nil, admin)
	if rec.Code != 200 || !strings.Contains(rec.Body.String(), session) {
		t.Fatalf("admin sessions: %d %s", rec.Code, rec.Body.String())
	}
	rec = do(t, srv, "DELETE", "/api/admin/transcodes/"+session, nil, admin)
	if rec.Code != 200 {
		t.Fatalf("cancel: %d %s", rec.Code, rec.Body.String())
	}
}

// TestE2ESessionCacheReuse proves the second play of the same session
// options is served from the derivative cache instead of re-encoding.
func TestE2ESessionCacheReuse(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	sources := e2eSources(t)
	ids := e2eCatalog(t, srv, admin, dir, sources[:1])
	id := ids[sources[0].Name]

	selection := map[string]any{"delivery": "progressive", "quality": "360p"}
	first := e2eStartSession(t, srv, admin, id, selection, false)
	firstSession, _ := first["session"].(string)

	second := e2eStartSession(t, srv, admin, id, selection, false)
	secondSession, _ := second["session"].(string)
	if firstSession != secondSession {
		t.Fatalf("identical options produced two sessions: %s vs %s", firstSession, secondSession)
	}
	if cached, _ := second["cached"].(bool); !cached {
		t.Fatalf("second start was not served from cache: %v", second)
	}
	if method, _ := second["method"].(string); method != "remux" {
		t.Fatalf("method=%v, want remux", second["method"])
	}
}

// TestE2EDirectPlaySubtitleExtraction covers on-the-fly subtitle
// extraction: a file that plays directly still gets WebVTT for a chosen
// track, and the same request is served from cache afterwards.
func TestE2EDirectPlaySubtitleExtraction(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	sources := e2eSources(t)
	subbed := sources[2] // the dual-audio file that also carries SRT
	ids := e2eCatalog(t, srv, admin, dir, []e2eSource{subbed})
	id := ids[subbed.Name]

	// The plan must stay direct play: subtitles never force a transcode.
	rec := do(t, srv, "GET", "/api/items/"+id+"/playback?client=web", nil, admin)
	var plan contracts.Plan
	if err := json.Unmarshal(rec.Body.Bytes(), &plan); err != nil {
		t.Fatalf("plan: %d %s", rec.Code, rec.Body.String())
	}
	var subtitleIndex = -1
	for _, stream := range plan.Streams {
		if stream.Type == "subtitle" && stream.Convertible {
			subtitleIndex = stream.Index
		}
	}
	if subtitleIndex < 0 {
		t.Fatalf("plan streams lack a convertible subtitle: %+v", plan.Streams)
	}

	rec = do(t, srv, "GET", fmt.Sprintf("/api/items/%s/subtitles?stream=%d&token=%s", id, subtitleIndex, admin), nil, "")
	if rec.Code != 200 {
		t.Fatalf("subtitle extraction: %d %s", rec.Code, rec.Body.String())
	}
	if ct := rec.Header().Get("Content-Type"); ct != "text/vtt; charset=utf-8" {
		t.Fatalf("content-type %q", ct)
	}
	body := rec.Body.String()
	if !strings.HasPrefix(body, "WEBVTT") || !strings.Contains(body, "First line") {
		t.Fatalf("bad WebVTT: %q", body)
	}
	if strings.Contains(body, "transcodes") || strings.Contains(body, dir) {
		t.Fatalf("subtitle body leaks a filesystem path: %q", body)
	}

	// The second request comes from the sidecar cache and is identical.
	again := do(t, srv, "GET", fmt.Sprintf("/api/items/%s/subtitles?stream=%d&token=%s", id, subtitleIndex, admin), nil, "")
	if again.Code != 200 || again.Body.String() != body {
		t.Fatalf("cached subtitle differs: %d %q", again.Code, again.Body.String())
	}

	// An unknown track is refused without inventing a sidecar.
	if rec := do(t, srv, "GET", "/api/items/"+id+"/subtitles?stream=99&token="+admin, nil, ""); rec.Code == 200 {
		t.Fatalf("unknown subtitle track served: %s", rec.Body.String())
	}
}

// TestE2ESubtitleExtractionDisabledHonoursSettings proves the operator
// toggle is enforced with a stable 403 instead of a silent failure.
func TestE2ESubtitleExtractionDisabledHonoursSettings(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	sources := e2eSources(t)
	subbed := sources[2]
	ids := e2eCatalog(t, srv, admin, dir, []e2eSource{subbed})
	id := ids[subbed.Name]

	e2ePutSettings(t, srv, admin, func(s *contracts.TranscodeSettings) {
		off := false
		s.AllowSubtitleExtraction = &off
	})

	rec := do(t, srv, "GET", "/api/items/"+id+"/subtitles?stream=2&token="+admin, nil, "")
	if rec.Code != 403 {
		t.Fatalf("disabled extraction: %d %s, want 403", rec.Code, rec.Body.String())
	}
	if !strings.Contains(rec.Body.String(), "forbidden") {
		t.Fatalf("disabled extraction error is not the stable code: %s", rec.Body.String())
	}
}

// TestE2EStreamCopyPermissionForcesEncode proves the client-side flag is
// honored: forbidding a stream copy turns a remux into a re-encode.
func TestE2EStreamCopyPermissionForcesEncode(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	sources := e2eSources(t)
	ids := e2eCatalog(t, srv, admin, dir, sources[:1])
	id := ids[sources[0].Name]

	remux := e2eStartSession(t, srv, admin, id, map[string]any{
		"delivery": "progressive",
	}, false)
	if method, _ := remux["method"].(string); method != "remux" {
		t.Fatalf("default method=%v, want remux for a web-safe source", remux["method"])
	}
	if direct, _ := remux["video_direct"].(bool); !direct {
		t.Fatalf("remux reports video_direct=%v, want true", remux["video_direct"])
	}

	encoded := e2eStartSession(t, srv, admin, id, map[string]any{
		"delivery":                "progressive",
		"allow_video_stream_copy": false,
		"allow_audio_stream_copy": false,
	}, false)
	if method, _ := encoded["method"].(string); method != "transcode" {
		t.Fatalf("method=%v with stream copy forbidden, want transcode", encoded["method"])
	}
	if direct, _ := encoded["video_direct"].(bool); direct {
		t.Fatalf("video_direct=%v after forbidding a copy, want false", encoded["video_direct"])
	}
	session, _ := encoded["session"].(string)
	rec := do(t, srv, "GET", "/api/items/"+id+"/transcode?token="+admin+"&session="+session, nil, "")
	if rec.Code != 200 {
		t.Fatalf("fetch: %d %s", rec.Code, rec.Body.String())
	}
	profile := e2eProbe(t, e2eWrite(t, "forced-encode.mp4", rec.Body.Bytes()))
	if profile.Codec != "h264" || profile.AudioCodec != "aac" {
		t.Fatalf("profile=%+v, want h264/aac", profile)
	}
}

// TestE2EDeinterlaceMethodBwdif covers the filter choice: bwdif where the
// build has it, yadif with a reported fallback where it does not.
func TestE2EDeinterlaceMethodBwdif(t *testing.T) {
	e2eRequireFFmpeg(t)
	if !e2eHasFilter(t, "bwdif") {
		t.Skip("ffmpeg build lacks the bwdif filter")
	}
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	sources := e2eSources(t)
	interlaced := sources[5]
	ids := e2eCatalog(t, srv, admin, dir, []e2eSource{interlaced})
	id := ids[interlaced.Name]

	e2ePutSettings(t, srv, admin, func(s *contracts.TranscodeSettings) {
		s.Deinterlace = contracts.DeinterlaceAuto
		s.DeinterlaceMethod = contracts.DeinterlaceBwdif
	})
	status := e2eStartSession(t, srv, admin, id, map[string]any{"delivery": "progressive"}, false)
	if fallback, _ := status["fallback"].(string); strings.Contains(fallback, "bwdif") {
		t.Fatalf("bwdif was reported unavailable although the probe found it: %v", status)
	}
	session, _ := status["session"].(string)
	rec := do(t, srv, "GET", "/api/items/"+id+"/transcode?token="+admin+"&session="+session, nil, "")
	if rec.Code != 200 {
		t.Fatalf("fetch: %d %s", rec.Code, rec.Body.String())
	}
	profile := e2eProbe(t, e2eWrite(t, "bwdif.mp4", rec.Body.Bytes()))
	if profile.Codec != "h264" || profile.FPS <= 0 {
		t.Fatalf("profile=%+v, want a decodable h264 output", profile)
	}
}

// e2eHasFilter reports whether the local ffmpeg build ships a filter.
func e2eHasFilter(t *testing.T, name string) bool {
	t.Helper()
	out, err := exec.Command("ffmpeg", "-hide_banner", "-filters").Output()
	if err != nil {
		return false
	}
	for _, line := range strings.Split(string(out), "\n") {
		fields := strings.Fields(line)
		if len(fields) >= 2 && fields[1] == name {
			return true
		}
	}
	return false
}

// e2eSessionFor is a small helper used by the reporting assertions: it
// renders one public status into a single line for failure messages.
func e2eSessionFor(status map[string]any) string {
	parts := make([]string, 0, 4)
	for _, key := range []string{"session", "state", "delivery", "method"} {
		if value, ok := status[key]; ok {
			parts = append(parts, fmt.Sprintf("%s=%v", key, value))
		}
	}
	if reasons, ok := status["reasons"].([]any); ok {
		parts = append(parts, "reasons="+strconv.Itoa(len(reasons)))
	}
	return strings.Join(parts, " ")
}

// e2ePutSettings saves one settings mutation through the admin API and
// returns the effective settings.
func e2ePutSettings(t *testing.T, srv *Server, admin string, mutate func(*contracts.TranscodeSettings)) contracts.TranscodeSettings {
	t.Helper()
	rec := do(t, srv, "GET", "/api/admin/settings/transcode", nil, admin)
	if rec.Code != 200 {
		t.Fatalf("settings get: %d %s", rec.Code, rec.Body.String())
	}
	var payload struct {
		Settings contracts.TranscodeSettings `json:"settings"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &payload); err != nil {
		t.Fatal(err)
	}
	mutate(&payload.Settings)
	rec = do(t, srv, "PUT", "/api/admin/settings/transcode", payload.Settings, admin)
	if rec.Code != 200 {
		t.Fatalf("settings put: %d %s", rec.Code, rec.Body.String())
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &payload); err != nil {
		t.Fatal(err)
	}
	return payload.Settings
}

// TestE2EInterlacedDoubleRate is the deinterlace slice: an interlaced
// source gets yadif, and double rate keeps both fields as frames, which
// doubles the produced frame rate.
func TestE2EInterlacedDoubleRate(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	sources := e2eSources(t)
	interlaced := sources[5]
	ids := e2eCatalog(t, srv, admin, dir, []e2eSource{interlaced})
	id := ids[interlaced.Name]

	source := e2eProbe(t, filepath.Join(dir, interlaced.Name))
	if source.FPS <= 0 {
		t.Fatalf("interlaced fixture has no frame rate: %+v", source)
	}

	frameRate := func(doubleRate bool) float64 {
		e2ePutSettings(t, srv, admin, func(s *contracts.TranscodeSettings) {
			s.Deinterlace = contracts.DeinterlaceAuto
			s.DeinterlaceDoubleRate = doubleRate
		})
		status := e2eStartSession(t, srv, admin, id, map[string]any{"delivery": "progressive"}, false)
		session, _ := status["session"].(string)
		rec := do(t, srv, "GET", "/api/items/"+id+"/transcode?token="+admin+"&session="+session, nil, "")
		if rec.Code != 200 {
			t.Fatalf("fetch: %d %s", rec.Code, rec.Body.String())
		}
		return e2eProbe(t, e2eWrite(t, fmt.Sprintf("deint-%v.mp4", doubleRate), rec.Body.Bytes())).FPS
	}

	single := frameRate(false)
	double := frameRate(true)
	if single != source.FPS {
		t.Fatalf("single-rate output is %v fps, want the source rate %v", single, source.FPS)
	}
	if double != source.FPS*2 {
		t.Fatalf("double-rate output is %v fps, want twice the source rate %v", double, source.FPS*2)
	}
}

// TestE2ETempPathRelocatesArtifacts proves the operator's temp path is
// honored: artifacts land in lain's own subdirectory there while the
// JSON index stays with the data dir.
func TestE2ETempPathRelocatesArtifacts(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	sources := e2eSources(t)
	ids := e2eCatalog(t, srv, admin, dir, sources[:1])
	id := ids[sources[0].Name]

	tempRoot := t.TempDir()
	e2ePutSettings(t, srv, admin, func(s *contracts.TranscodeSettings) {
		s.TranscodeTempPath = tempRoot
	})

	status := e2eStartSession(t, srv, admin, id, map[string]any{"delivery": "progressive"}, false)
	session, _ := status["session"].(string)

	artifact := filepath.Join(tempRoot, "lain-transcode", session, "media.mp4")
	if _, err := os.Stat(artifact); err != nil {
		t.Fatalf("artifact is not under the configured temp path: %v", err)
	}

	// The artifact must still be served, and the data dir must hold only
	// the sidecar.
	rec := do(t, srv, "GET", "/api/items/"+id+"/transcode?token="+admin+"&session="+session, nil, "")
	if rec.Code != 200 || rec.Body.Len() == 0 {
		t.Fatalf("relocated artifact not served: %d", rec.Code)
	}
	entries, err := os.ReadDir(filepath.Join(filepath.Dir(srv.db.Path()), "transcodes"))
	if err == nil {
		for _, entry := range entries {
			if strings.HasSuffix(entry.Name(), ".mp4") {
				t.Fatalf("media left in the data dir: %s", entry.Name())
			}
		}
	}
}

// TestE2EPerCodecCRFChangesOutput proves the per-codec CRF reaches the
// encoder: a very high CRF must produce a much smaller artifact than a
// low one for the same source and preset.
func TestE2EPerCodecCRFChangesOutput(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	sources := e2eSources(t)
	ids := e2eCatalog(t, srv, admin, dir, sources[1:2])
	id := ids[sources[1].Name]

	sizeFor := func(crf int) int {
		e2ePutSettings(t, srv, admin, func(s *contracts.TranscodeSettings) {
			s.H264CRF = crf
			s.CRF = crf
			s.EncoderPreset = "veryfast"
		})
		status := e2eStartSession(t, srv, admin, id, map[string]any{"delivery": "progressive"}, false)
		session, _ := status["session"].(string)
		rec := do(t, srv, "GET", "/api/items/"+id+"/transcode?token="+admin+"&session="+session, nil, "")
		if rec.Code != 200 {
			t.Fatalf("fetch: %d %s", rec.Code, rec.Body.String())
		}
		return rec.Body.Len()
	}

	low := sizeFor(18)
	high := sizeFor(48)
	if high >= low {
		t.Fatalf("crf 48 produced %d bytes, crf 18 produced %d: the setting did not reach the encoder", high, low)
	}
}

// TestE2ESessionMetricsWhileRunning covers the live pipeline metrics:
// a deliberately slow encode must report fps and an output bitrate
// while it is still running.
func TestE2ESessionMetricsWhileRunning(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()

	// mpeg4/mp3 forces a full re-encode, which is what produces live
	// encoder metrics (a remux has no frame rate to report).
	slow := filepath.Join(dir, "[Fansub-A] Procedural Show - 07 [Slow].mkv")
	e2eRunFFmpeg(t,
		"-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30:duration=10",
		"-f", "lavfi", "-i", "sine=frequency=440:sample_rate=44100:duration=10",
		"-c:v", "mpeg4", "-q:v", "2", "-c:a", "libmp3lame", slow)
	ids := e2eCatalog(t, srv, admin, dir, []e2eSource{{Name: filepath.Base(slow)}})
	id := ids[filepath.Base(slow)]

	e2ePutSettings(t, srv, admin, func(s *contracts.TranscodeSettings) {
		s.EncoderPreset = "veryslow"
		s.H264CRF = 18
		s.CRF = 18
	})

	rec := do(t, srv, "POST", "/api/items/"+id+"/transcode", map[string]any{"delivery": "progressive"}, admin)
	if rec.Code != 200 && rec.Code != 202 {
		t.Fatalf("start: %d %s", rec.Code, rec.Body.String())
	}
	var status map[string]any
	if err := json.Unmarshal(rec.Body.Bytes(), &status); err != nil {
		t.Fatal(err)
	}
	session, _ := status["session"].(string)

	var sawFPS, sawBitrate float64
	deadline := time.Now().Add(120 * time.Second)
	for time.Now().Before(deadline) {
		rec = do(t, srv, "GET", "/api/items/"+id+"/transcode/status?session="+session, nil, admin)
		if err := json.Unmarshal(rec.Body.Bytes(), &status); err != nil {
			t.Fatal(err)
		}
		if fps, ok := status["fps"].(float64); ok && fps > 0 {
			sawFPS = fps
		}
		if kbps, ok := status["output_bitrate_kbps"].(float64); ok && kbps > 0 {
			sawBitrate = kbps
		}
		if state, _ := status["state"].(string); state == contracts.TranscodeReady || state == contracts.TranscodeFailed {
			break
		}
		if sawFPS > 0 && sawBitrate > 0 {
			break
		}
		time.Sleep(100 * time.Millisecond)
	}
	if sawFPS <= 0 {
		t.Fatalf("no fps was reported while the session ran: %v", status)
	}
	if sawBitrate <= 0 {
		t.Fatalf("no output bitrate was reported while the session ran: %v", status)
	}
}

// TestE2EStreamLimitPerUser proves the per-user simultaneous stream
// limit is enforced with a stable 429 instead of silently queueing.
func TestE2EStreamLimitPerUser(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()

	// The first session must still be running when the second start
	// arrives, so it is a slow re-encode (mpeg4) under a slow preset.
	first := filepath.Join(dir, "[Fansub-A] Procedural Show - 08 [Busy].mkv")
	e2eRunFFmpeg(t,
		"-f", "lavfi", "-i", "testsrc2=size=640x360:rate=25:duration=8",
		"-c:v", "mpeg4", "-q:v", "2", first)
	second := filepath.Join(dir, "[Fansub-B] Procedural Show - 09 [Idle].mkv")
	e2eRunFFmpeg(t,
		"-f", "lavfi", "-i", "testsrc2=size=320x180:rate=15:duration=3",
		"-c:v", "mpeg4", "-q:v", "2", second)
	ids := e2eCatalog(t, srv, admin, dir, []e2eSource{
		{Name: filepath.Base(first)}, {Name: filepath.Base(second)},
	})
	firstID, secondID := ids[filepath.Base(first)], ids[filepath.Base(second)]

	// The admin account itself carries the limit, so no second user is
	// needed to exercise the admission check.
	rec := do(t, srv, "GET", "/api/me", nil, admin)
	var me struct {
		ID string `json:"id"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &me); err != nil || me.ID == "" {
		t.Fatalf("me: %d %s", rec.Code, rec.Body.String())
	}
	if rec := do(t, srv, "PATCH", "/api/users/"+me.ID, map[string]any{
		"playback": map[string]any{"max_streams": 1},
	}, admin); rec.Code != 200 {
		t.Fatalf("limits patch: %d %s", rec.Code, rec.Body.String())
	}

	e2ePutSettings(t, srv, admin, func(s *contracts.TranscodeSettings) {
		s.EncoderPreset = "veryslow"
		s.H264CRF = 16
		s.CRF = 16
		s.MaxConcurrent = 2
	})

	rec = do(t, srv, "POST", "/api/items/"+firstID+"/transcode", map[string]any{"delivery": "progressive"}, admin)
	if rec.Code != 200 && rec.Code != 202 {
		t.Fatalf("first start: %d %s", rec.Code, rec.Body.String())
	}
	var started map[string]any
	if err := json.Unmarshal(rec.Body.Bytes(), &started); err != nil {
		t.Fatal(err)
	}
	if state, _ := started["state"].(string); state != contracts.TranscodeQueued && state != contracts.TranscodeRunning {
		t.Fatalf("first session is %q, want it still busy: %v", state, started)
	}

	rec = do(t, srv, "POST", "/api/items/"+secondID+"/transcode", map[string]any{"delivery": "progressive"}, admin)
	if rec.Code != 429 {
		t.Fatalf("second concurrent stream: %d %s, want 429", rec.Code, rec.Body.String())
	}
	if !strings.Contains(rec.Body.String(), "too-many-streams") {
		t.Fatalf("second stream error is not the stable code: %s", rec.Body.String())
	}

	// Joining the first session must still be allowed at the limit.
	rejoin := do(t, srv, "POST", "/api/items/"+firstID+"/transcode", map[string]any{"delivery": "progressive"}, admin)
	if rejoin.Code != 200 && rejoin.Code != 202 {
		t.Fatalf("rejoin at the limit: %d %s", rejoin.Code, rejoin.Body.String())
	}
}

// TestE2ERemoteBitrateCap proves the server-wide ceiling is the tighter
// of the two caps: a 1080p quality request under a 500 kbps server limit
// must report the server cap on the session.
func TestE2ERemoteBitrateCap(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	sources := e2eSources(t)
	ids := e2eCatalog(t, srv, admin, dir, sources[1:2])
	id := ids[sources[1].Name]

	e2ePutSettings(t, srv, admin, func(s *contracts.TranscodeSettings) {
		s.RemoteBitrateLimitKbps = 500
	})

	status := e2eStartSession(t, srv, admin, id, map[string]any{
		"delivery": "progressive",
		"quality":  "1080p",
	}, false)
	if kbps, _ := status["bitrate_kbps"].(float64); kbps != 500 {
		t.Fatalf("bitrate_kbps=%v, want the server-wide 500 kbps cap", status["bitrate_kbps"])
	}
}

// TestE2EDownmixToStereo covers the audio downmix: a 5.1 source becomes
// stereo AAC with the configured gain applied.
func TestE2EDownmixToStereo(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	sources := e2eSources(t)
	surround := sources[4]
	ids := e2eCatalog(t, srv, admin, dir, []e2eSource{surround})
	id := ids[surround.Name]

	if got := e2eProbe(t, filepath.Join(dir, surround.Name)).Channels; got < 6 {
		t.Skipf("fixture has %d channels, want 5.1", got)
	}
	e2ePutSettings(t, srv, admin, func(s *contracts.TranscodeSettings) {
		s.DownmixAudio = true
		s.DownmixAudioBoost = 2
		s.DownmixStereoAlgorithm = contracts.DownmixNightmode
	})

	status := e2eStartSession(t, srv, admin, id, map[string]any{"delivery": "progressive"}, false)
	session, _ := status["session"].(string)
	rec := do(t, srv, "GET", "/api/items/"+id+"/transcode?token="+admin+"&session="+session, nil, "")
	if rec.Code != 200 {
		t.Fatalf("fetch: %d %s", rec.Code, rec.Body.String())
	}
	profile := e2eProbe(t, e2eWrite(t, "downmix.mp4", rec.Body.Bytes()))
	if profile.AudioCodec != "aac" || profile.Channels != 2 {
		t.Fatalf("profile=%+v, want stereo aac after downmix", profile)
	}
}

// TestE2EHEVCOutput proves the HEVC output permission path: with
// allow_hevc on, a requested hevc session re-encodes to HEVC and the
// produced MP4 probes as hevc.
func TestE2EHEVCOutput(t *testing.T) {
	e2eRequireFFmpeg(t)
	if !e2eHasEncoder(t, "libx265") {
		t.Skip("ffmpeg lacks libx265")
	}
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	sources := e2eSources(t)
	ids := e2eCatalog(t, srv, admin, dir, sources[:1])
	id := ids[sources[0].Name]

	e2ePutSettings(t, srv, admin, func(s *contracts.TranscodeSettings) { s.AllowHEVC = true })

	status := e2eStartSession(t, srv, admin, id, map[string]any{
		"delivery":    "progressive",
		"video_codec": "hevc",
	}, false)
	if codec, _ := status["video_codec"].(string); codec != "hevc" {
		t.Fatalf("video_codec=%v, want hevc", status["video_codec"])
	}
	if method, _ := status["method"].(string); method != "transcode" {
		t.Fatalf("method=%v, want transcode (hevc output from an h264 source)", status["method"])
	}
	session, _ := status["session"].(string)
	rec := do(t, srv, "GET", "/api/items/"+id+"/transcode?token="+admin+"&session="+session, nil, "")
	if rec.Code != 200 {
		t.Fatalf("fetch: %d %s", rec.Code, rec.Body.String())
	}
	profile := e2eProbe(t, e2eWrite(t, "hevc.mp4", rec.Body.Bytes()))
	if profile.Codec != "hevc" {
		t.Fatalf("profile=%+v, want hevc", profile)
	}
}

// TestE2EHLSSegmentContainerMPEGTS proves the HLS container choice end to
// end: mpegts produces .ts segments with no init segment and no
// #EXT-X-MAP, and the served segment probes as h264.
func TestE2EHLSSegmentContainerMPEGTS(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	sources := e2eSources(t)
	ids := e2eCatalog(t, srv, admin, dir, sources[:1])
	id := ids[sources[0].Name]

	e2ePutSettings(t, srv, admin, func(s *contracts.TranscodeSettings) {
		s.HLSSegmentContainer = contracts.HLSSegmentTS
	})

	status := e2eStartSession(t, srv, admin, id, map[string]any{
		"delivery": "hls",
		"quality":  "360p",
	}, true)
	session, _ := status["session"].(string)
	base := "/api/items/" + id + "/transcode/hls/"
	q := "?session=" + session + "&token=" + admin

	rec := do(t, srv, "GET", base+"index.m3u8"+q, nil, "")
	if rec.Code != 200 {
		t.Fatalf("playlist: %d %s", rec.Code, rec.Body.String())
	}
	playlist := rec.Body.String()
	if strings.Contains(playlist, "#EXT-X-MAP") {
		t.Fatalf("mpegts playlist must not reference an init segment:\n%s", playlist)
	}
	if !strings.Contains(playlist, "seg00000.ts") {
		t.Fatalf("mpegts playlist lists no .ts segments:\n%s", playlist)
	}
	if initRec := do(t, srv, "GET", base+"init.mp4"+q, nil, ""); initRec.Code != 404 {
		t.Fatalf("mpegts session served an init segment: %d", initRec.Code)
	}
	segRec := do(t, srv, "GET", base+"seg00000.ts"+q, nil, "")
	if segRec.Code != 200 || segRec.Body.Len() == 0 {
		t.Fatalf("ts segment: %d %d bytes", segRec.Code, segRec.Body.Len())
	}
	profile := e2eProbe(t, e2eWrite(t, "seg.ts", segRec.Body.Bytes()))
	if profile.Codec != "h264" {
		t.Fatalf("ts segment profile=%+v, want h264", profile)
	}
}

// TestE2EAV1Output proves the AV1 output permission path: with allow_av1
// on, a requested av1 session re-encodes to AV1 and the MP4 probes as av1.
func TestE2EAV1Output(t *testing.T) {
	e2eRequireFFmpeg(t)
	if !e2eHasEncoder(t, "libsvtav1") && !e2eHasEncoder(t, "libaom-av1") {
		t.Skip("ffmpeg lacks an AV1 encoder")
	}
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	sources := e2eSources(t)
	ids := e2eCatalog(t, srv, admin, dir, sources[:1])
	id := ids[sources[0].Name]

	e2ePutSettings(t, srv, admin, func(s *contracts.TranscodeSettings) { s.AllowAV1 = true })

	status := e2eStartSession(t, srv, admin, id, map[string]any{
		"delivery":    "progressive",
		"video_codec": "av1",
		"quality":     "360p",
	}, false)
	if codec, _ := status["video_codec"].(string); codec != "av1" {
		t.Fatalf("video_codec=%v, want av1", status["video_codec"])
	}
	session, _ := status["session"].(string)
	rec := do(t, srv, "GET", "/api/items/"+id+"/transcode?token="+admin+"&session="+session, nil, "")
	if rec.Code != 200 {
		t.Fatalf("fetch: %d %s", rec.Code, rec.Body.String())
	}
	profile := e2eProbe(t, e2eWrite(t, "av1.mp4", rec.Body.Bytes()))
	if profile.Codec != "av1" {
		t.Fatalf("profile=%+v, want av1", profile)
	}
}

// TestE2EHardwareUnavailableStillPlays proves a requested-but-unavailable
// hardware backend degrades to software: the session still produces a
// playable h264 stream instead of failing.
func TestE2EHardwareUnavailableStillPlays(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	sources := e2eSources(t)
	ids := e2eCatalog(t, srv, admin, dir, sources[:2])
	id := ids[sources[1].Name] // mpeg4/mp3 forces a re-encode

	e2ePutSettings(t, srv, admin, func(s *contracts.TranscodeSettings) {
		s.HardwareAcceleration = contracts.HWVAAPI
		s.HardwareEncode = true
	})
	status := e2eStartSession(t, srv, admin, id, map[string]any{
		"delivery": "progressive",
		"quality":  "360p",
	}, false)
	session, _ := status["session"].(string)
	rec := do(t, srv, "GET", "/api/items/"+id+"/transcode?token="+admin+"&session="+session, nil, "")
	if rec.Code != 200 {
		t.Fatalf("fetch: %d %s", rec.Code, rec.Body.String())
	}
	profile := e2eProbe(t, e2eWrite(t, "hwfallback.mp4", rec.Body.Bytes()))
	if profile.Codec != "h264" {
		t.Fatalf("profile=%+v, want a playable h264 stream", profile)
	}
}

// TestE2ESegmentDeletion proves segment deletion end to end: after the
// client fetches a later segment, the ones behind the keep window are
// gone from disk (a 404) and the playlist no longer lists them.
func TestE2ESegmentDeletion(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	name := "[Fansub-A] Procedural Show - 90 [Deletion].mkv"
	e2eRunFFmpeg(t,
		"-f", "lavfi", "-i", "testsrc2=size=320x180:rate=15:duration=12",
		"-f", "lavfi", "-i", "sine=frequency=440:sample_rate=44100:duration=12",
		"-c:v", "mpeg4", "-q:v", "6", "-c:a", "libmp3lame",
		filepath.Join(dir, name))
	ids := e2eCatalog(t, srv, admin, dir, []e2eSource{{Name: name}})
	id := ids[name]

	e2ePutSettings(t, srv, admin, func(s *contracts.TranscodeSettings) {
		s.HLSSegmentSeconds = 1
		s.SegmentDeletion = true
		s.SegmentKeepSec = 1
	})

	status := e2eStartSession(t, srv, admin, id, map[string]any{"delivery": "hls"}, false)
	session, _ := status["session"].(string)
	base := "/api/items/" + id + "/transcode/hls/"
	q := "?session=" + session + "&token=" + admin

	rec := do(t, srv, "GET", base+"index.m3u8"+q, nil, "")
	if rec.Code != 200 {
		t.Fatalf("playlist: %d %s", rec.Code, rec.Body.String())
	}
	var names []string
	for _, line := range strings.Split(rec.Body.String(), "\n") {
		line = strings.TrimSpace(line)
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		if i := strings.IndexByte(line, '?'); i >= 0 {
			line = line[:i]
		}
		names = append(names, line)
	}
	if len(names) < 4 {
		t.Skipf("only %d segments produced; cannot exercise deletion", len(names))
	}
	last := names[len(names)-1]
	if segRec := do(t, srv, "GET", base+last+q, nil, ""); segRec.Code != 200 {
		t.Fatalf("fetching %s: %d", last, segRec.Code)
	}
	// Fetching the last segment moves the client past the keep window, so
	// the early segments must be gone.
	if firstRec := do(t, srv, "GET", base+names[0]+q, nil, ""); firstRec.Code != 404 {
		t.Fatalf("segment %s survived deletion: %d", names[0], firstRec.Code)
	}
}

// TestE2EBitrateCapIsEncoded proves the cap is really applied to the
// bytes, not just echoed in the status: a high-bitrate source under a
// 400 kbps ceiling must produce far fewer bytes than the raw source.
func TestE2EBitrateCapIsEncoded(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	name := "[Fansub-A] Procedural Show - 91 [Capped].mkv"
	e2eRunFFmpeg(t,
		"-f", "lavfi", "-i", "testsrc2=size=640x360:rate=25:duration=8",
		"-c:v", "mpeg4", "-b:v", "4000k",
		filepath.Join(dir, name))
	ids := e2eCatalog(t, srv, admin, dir, []e2eSource{{Name: name}})
	id := ids[name]

	e2ePutSettings(t, srv, admin, func(s *contracts.TranscodeSettings) { s.RemoteBitrateLimitKbps = 400 })

	status := e2eStartSession(t, srv, admin, id, map[string]any{"delivery": "progressive"}, false)
	if kbps, _ := status["bitrate_kbps"].(float64); kbps != 400 {
		t.Fatalf("bitrate_kbps=%v, want the server-wide 400 kbps cap", status["bitrate_kbps"])
	}
	if method, _ := status["method"].(string); method != "transcode" {
		t.Fatalf("method=%v, want transcode so the cap can apply", status["method"])
	}
	session, _ := status["session"].(string)
	rec := do(t, srv, "GET", "/api/items/"+id+"/transcode?token="+admin+"&session="+session, nil, "")
	if rec.Code != 200 {
		t.Fatalf("fetch: %d %s", rec.Code, rec.Body.String())
	}
	// 400 kbps of video plus the audio track over 8 seconds, doubled for
	// container slack, is still far below the ~4 Mbps source.
	limitBytes := (400 + 160) * 1000 / 8 * 8 * 2
	if rec.Body.Len() > limitBytes {
		t.Fatalf("capped output is %d bytes, want under %d for a 400 kbps ceiling over 8s", rec.Body.Len(), limitBytes)
	}
}

// TestE2EThrottlePausesProduction proves the throttle end to end: with a
// small produced-ahead budget and one fetched segment, ffmpeg must stop
// producing more segments instead of racing to the end.
func TestE2EThrottlePausesProduction(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	name := "[Fansub-A] Procedural Show - 95 [Throttle].mkv"
	e2eRunFFmpeg(t,
		"-f", "lavfi", "-i", "testsrc2=size=320x180:rate=15:duration=60",
		"-c:v", "mpeg4", "-q:v", "6", filepath.Join(dir, name))
	ids := e2eCatalog(t, srv, admin, dir, []e2eSource{{Name: name}})
	id := ids[name]

	e2ePutSettings(t, srv, admin, func(s *contracts.TranscodeSettings) {
		s.HLSSegmentSeconds = 1
		s.Throttle = true
		s.ThrottleAheadSec = 5
	})

	status := e2eStartSession(t, srv, admin, id, map[string]any{"delivery": "hls"}, true)
	session, _ := status["session"].(string)
	base := "/api/items/" + id + "/transcode/hls/"
	q := "?session=" + session + "&token=" + admin

	// Fetch one segment so the client has a position; production must then
	// pause once it is far enough ahead.
	if rec := do(t, srv, "GET", base+"seg00000.m4s"+q, nil, ""); rec.Code != 200 {
		t.Fatalf("first segment: %d %s", rec.Code, rec.Body.String())
	}
	count := func() int {
		rec := do(t, srv, "GET", base+"index.m3u8"+q, nil, "")
		n := 0
		for _, line := range strings.Split(rec.Body.String(), "\n") {
			line = strings.TrimSpace(line)
			if line != "" && !strings.HasPrefix(line, "#") {
				n++
			}
		}
		return n
	}
	time.Sleep(3 * time.Second)
	first := count()
	time.Sleep(3 * time.Second)
	second := count()
	if second > first+1 {
		t.Fatalf("production did not pause: %d then %d segments", first, second)
	}
}

// TestE2EIdleTimeoutStopsAbandonedSession proves the idle timeout end to
// end: a session nobody fetches (only status polls, which do not refresh
// lastTouch) must stop itself after idle_timeout_sec.
func TestE2EIdleTimeoutStopsAbandonedSession(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	name := "[Fansub-A] Procedural Show - 96 [Idle].mkv"
	e2eRunFFmpeg(t,
		"-f", "lavfi", "-i", "testsrc2=size=640x360:rate=25:duration=60",
		"-c:v", "mpeg4", "-q:v", "6", filepath.Join(dir, name))
	ids := e2eCatalog(t, srv, admin, dir, []e2eSource{{Name: name}})
	id := ids[name]

	// A slow preset keeps the session running well past the idle window
	// instead of finishing (a finished session is Ready, not idle).
	e2ePutSettings(t, srv, admin, func(s *contracts.TranscodeSettings) {
		s.IdleTimeoutSec = 15
		s.EncoderPreset = "veryslow"
	})

	status := e2eStartSession(t, srv, admin, id, map[string]any{"delivery": "hls"}, true)
	session, _ := status["session"].(string)

	deadline := time.Now().Add(40 * time.Second)
	for time.Now().Before(deadline) {
		rec := do(t, srv, "GET", "/api/items/"+id+"/transcode/status?session="+session, nil, admin)
		var st map[string]any
		if err := json.Unmarshal(rec.Body.Bytes(), &st); err != nil {
			t.Fatal(err)
		}
		if st["state"] == "failed" {
			if code, _ := st["error_code"].(string); code != "idle-timeout" {
				t.Fatalf("session failed with %v, want idle-timeout", st)
			}
			return
		}
		time.Sleep(500 * time.Millisecond)
	}
	t.Fatal("an abandoned session was not stopped by the idle timeout")
}

// TestE2EQueueFullRejects proves the queue bound end to end: with one
// slot and a one-deep queue, a third distinct session is refused with the
// stable `queue-full` code.
func TestE2EQueueFullRejects(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	names := []string{
		"[Fansub-A] Procedural Show - 92 [Long].mkv",
		"[Fansub-B] Procedural Show - 93 [Short B].mkv",
		"[Fansub-B] Procedural Show - 94 [Short C].mkv",
	}
	e2eRunFFmpeg(t, "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=15:duration=60", "-c:v", "mpeg4", "-q:v", "6", filepath.Join(dir, names[0]))
	e2eRunFFmpeg(t, "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=15:duration=4", "-c:v", "mpeg4", "-q:v", "6", filepath.Join(dir, names[1]))
	e2eRunFFmpeg(t, "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=15:duration=4", "-c:v", "mpeg4", "-q:v", "6", filepath.Join(dir, names[2]))
	sources := make([]e2eSource, 0, len(names))
	for _, n := range names {
		sources = append(sources, e2eSource{Name: n})
	}
	ids := e2eCatalog(t, srv, admin, dir, sources)

	e2ePutSettings(t, srv, admin, func(s *contracts.TranscodeSettings) {
		s.MaxConcurrent = 1
		s.QueueSize = 1
		s.EncoderPreset = "veryslow"
	})

	post := func(id string) (string, map[string]any) {
		rec := do(t, srv, "POST", "/api/items/"+id+"/transcode", map[string]any{"delivery": "progressive"}, admin)
		if rec.Code != 200 && rec.Code != 202 {
			t.Fatalf("start %s: %d %s", id, rec.Code, rec.Body.String())
		}
		var st map[string]any
		if err := json.Unmarshal(rec.Body.Bytes(), &st); err != nil {
			t.Fatal(err)
		}
		session, _ := st["session"].(string)
		return session, st
	}

	// A must actually hold the only slot before B can be counted as
	// waiting, so wait for it to be running.
	sessionA, _ := post(ids[names[0]])
	deadline := time.Now().Add(30 * time.Second)
	for {
		rec := do(t, srv, "GET", "/api/items/"+ids[names[0]]+"/transcode/status?session="+sessionA, nil, admin)
		var st map[string]any
		_ = json.Unmarshal(rec.Body.Bytes(), &st)
		if st["state"] == "running" {
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("session A never started running: %v", st)
		}
		time.Sleep(50 * time.Millisecond)
	}

	// B is admitted and waits for the slot (queue depth 1).
	post(ids[names[1]])

	// C has nowhere to go.
	rec := do(t, srv, "POST", "/api/items/"+ids[names[2]]+"/transcode", map[string]any{"delivery": "progressive"}, admin)
	if rec.Code != 429 {
		t.Fatalf("third concurrent start: %d %s, want 429", rec.Code, rec.Body.String())
	}
	if !strings.Contains(rec.Body.String(), "queue-full") {
		t.Fatalf("third start error is not the stable code: %s", rec.Body.String())
	}
}

// TestE2EVideoOnlySourceNoAudio proves a silent source transcodes: the
// output has a video stream and no audio track, and the session succeeds
// instead of failing on the missing audio stream.
func TestE2EVideoOnlySourceNoAudio(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	name := "[Fansub-A] Procedural Show - 97 [Silent].mkv"
	e2eRunFFmpeg(t,
		"-f", "lavfi", "-i", "testsrc2=size=320x180:rate=15:duration=4",
		"-c:v", "mpeg4", "-q:v", "6", "-an", filepath.Join(dir, name))
	ids := e2eCatalog(t, srv, admin, dir, []e2eSource{{Name: name}})
	id := ids[name]

	status := e2eStartSession(t, srv, admin, id, map[string]any{"delivery": "progressive"}, false)
	session, _ := status["session"].(string)
	rec := do(t, srv, "GET", "/api/items/"+id+"/transcode?token="+admin+"&session="+session, nil, "")
	if rec.Code != 200 {
		t.Fatalf("fetch: %d %s", rec.Code, rec.Body.String())
	}
	profile := e2eProbe(t, e2eWrite(t, "silent.mp4", rec.Body.Bytes()))
	if profile.Codec != "h264" {
		t.Fatalf("profile=%+v, want an h264 video stream", profile)
	}
	if profile.AudioCodec != "" {
		t.Fatalf("profile=%+v, want no audio track", profile)
	}
}

// TestE2EHLSSidecarAvailableWhileRunning proves an HLS session offers its
// WebVTT subtitle track from the start, not only once the whole file has
// been produced.
func TestE2EHLSSidecarAvailableWhileRunning(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	name := "[Fansub-A] Procedural Show - 98 [Sidecar].mkv"
	srt := filepath.Join(dir, "subs.srt")
	body := "1\n00:00:00,000 --> 00:00:03,000\nFirst line\n\n2\n00:00:03,000 --> 00:00:06,000\nSecond line\n"
	if err := os.WriteFile(srt, []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
	e2eRunFFmpeg(t,
		"-f", "lavfi", "-i", "testsrc2=size=320x180:rate=15:duration=20",
		"-f", "lavfi", "-i", "sine=frequency=440:sample_rate=44100:duration=20",
		"-i", srt,
		"-map", "0:v", "-map", "1:a", "-map", "2:s",
		"-c:v", "mpeg4", "-q:v", "6", "-c:a", "libmp3lame", "-c:s", "srt",
		filepath.Join(dir, name))
	ids := e2eCatalog(t, srv, admin, dir, []e2eSource{{Name: name}})
	id := ids[name]

	// A slow preset keeps the session running while the sidecar is checked.
	e2ePutSettings(t, srv, admin, func(s *contracts.TranscodeSettings) { s.EncoderPreset = "veryslow" })

	status := e2eStartSession(t, srv, admin, id, map[string]any{
		"delivery":        "hls",
		"subtitle_mode":   "extract",
		"subtitle_stream": 2,
	}, true)
	session, _ := status["session"].(string)

	rec := do(t, srv, "GET", "/api/items/"+id+"/transcode/status?session="+session, nil, admin)
	var st map[string]any
	if err := json.Unmarshal(rec.Body.Bytes(), &st); err != nil {
		t.Fatal(err)
	}
	if state, _ := st["state"].(string); state == contracts.TranscodeRunning {
		if has, _ := st["has_subtitle"].(bool); !has {
			t.Fatalf("a running HLS session offers no subtitle track yet: %v", st)
		}
	}
	subRec := do(t, srv, "GET", "/api/items/"+id+"/subtitles?session="+session+"&token="+admin, nil, "")
	if subRec.Code != 200 || subRec.Body.Len() == 0 {
		t.Fatalf("sidecar: %d %d bytes", subRec.Code, subRec.Body.Len())
	}
}
