package gateway


import (
	"encoding/json"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
)

// advSettingsPayload mirrors the admin settings response shape: the
// stored policy plus the probe summary.
type advSettingsPayload struct {
	Settings contracts.TranscodeSettings `json:"settings"`
}

// advWaitReady polls one transcode session until it is ready; work is
// asynchronous, so a deadline is the only safe way to wait.
func advWaitReady(t *testing.T, srv *Server, token, id, session string) (string, map[string]any) {
	t.Helper()
	deadline := time.Now().Add(30 * time.Second)
	for {
		rec := do(t, srv, "GET", "/api/items/"+id+"/transcode/status?session="+session, nil, token)
		if rec.Code != http.StatusOK {
			t.Fatalf("status: %d %s", rec.Code, rec.Body.String())
		}
		var status map[string]any
		if err := json.Unmarshal(rec.Body.Bytes(), &status); err != nil {
			t.Fatalf("status body: %s", rec.Body.String())
		}
		switch status["state"] {
		case contracts.TranscodeReady:
			return rec.Body.String(), status
		case contracts.TranscodeFailed:
			t.Fatalf("session failed: %s", rec.Body.String())
		}
		if time.Now().After(deadline) {
			t.Fatalf("session did not finish in time: %s", rec.Body.String())
		}
		time.Sleep(20 * time.Millisecond)
	}
}

// advSynthMKV writes one synthesized Matroska fixture through ffmpeg;
// a missing ffmpeg or encoder skips the test cleanly.
func advSynthMKV(t *testing.T, path string, args ...string) {
	t.Helper()
	if _, err := exec.LookPath("ffmpeg"); err != nil {
		t.Skip("ffmpeg not installed")
	}
	cmd := exec.Command("ffmpeg", append(append([]string{"-hide_banner", "-loglevel", "error", "-y"}, args...), path)...)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Skipf("cannot synthesize fixture: %v: %s", err, out)
	}
}

// TestAdvancedPartialPutPreservesDefaults pins the merge semantics of the
// admin PUT: a partial document must not silently disable the boolean
// policies that default to true (throttle, downmix, tone mapping), while
// an explicit false must still turn one off.
func TestAdvancedPartialPutPreservesDefaults(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)

	rec := do(t, srv, "PUT", "/api/admin/settings/transcode", map[string]any{"thread_count": 3}, admin)
	if rec.Code != http.StatusOK {
		t.Fatalf("PUT: %d %s", rec.Code, rec.Body.String())
	}
	var put advSettingsPayload
	if err := json.Unmarshal(rec.Body.Bytes(), &put); err != nil {
		t.Fatal(err)
	}
	if put.Settings.ThreadCount != 3 {
		t.Fatalf("thread_count = %d, want 3", put.Settings.ThreadCount)
	}
	if !put.Settings.Throttle || !put.Settings.DownmixAudio || !put.Settings.ToneMapping {
		t.Fatalf("a partial PUT disabled default-on booleans: throttle=%v downmix=%v tone=%v",
			put.Settings.Throttle, put.Settings.DownmixAudio, put.Settings.ToneMapping)
	}

	// An explicit false still turns a policy off, and unrelated booleans
	// stay untouched.
	rec = do(t, srv, "PUT", "/api/admin/settings/transcode", map[string]any{"throttle": false}, admin)
	if rec.Code != http.StatusOK {
		t.Fatalf("PUT: %d %s", rec.Code, rec.Body.String())
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &put); err != nil {
		t.Fatal(err)
	}
	if put.Settings.Throttle {
		t.Fatal("an explicit throttle=false was ignored")
	}
	if !put.Settings.DownmixAudio || !put.Settings.ToneMapping {
		t.Fatal("an unrelated partial PUT disabled other booleans")
	}
}

// advScanCatalog ingests a directory through the real scan pipeline and
// returns the catalog ids keyed by file name. catalogOneMKV covers the
// single-item case; the stream-limit test needs two items.
func advScanCatalog(t *testing.T, srv *Server, admin, dir string) map[string]string {
	t.Helper()
	if rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "ADV", "type": "movie", "path": dir}, admin); rec.Code != http.StatusCreated {
		t.Fatalf("library: %d %s", rec.Code, rec.Body.String())
	}
	if rec := do(t, srv, "POST", "/api/library/scan", nil, admin); rec.Code != http.StatusAccepted {
		t.Fatalf("scan: %d %s", rec.Code, rec.Body.String())
	}
	waitScan(t, srv, admin)

	var page contracts.CatalogPage
	rec := do(t, srv, "GET", "/api/catalog?limit=100", nil, admin)
	if err := json.Unmarshal(rec.Body.Bytes(), &page); err != nil {
		t.Fatalf("catalog: %d %s", rec.Code, rec.Body.String())
	}
	out := map[string]string{}
	for _, item := range page.Items {
		out[filepath.Base(item.FilePath)] = item.ID
	}
	return out
}

// TestAdvancedSettingsDefaults pins the shipped defaults of the new
// advanced fields as the admin API reports them.
func TestAdvancedSettingsDefaults(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)

	rec := do(t, srv, "GET", "/api/admin/settings/transcode", nil, admin)
	if rec.Code != http.StatusOK {
		t.Fatalf("GET: %d %s", rec.Code, rec.Body.String())
	}
	var got advSettingsPayload
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	s := got.Settings
	checks := []struct {
		name string
		got  any
		want any
	}{
		{"thread_count", s.ThreadCount, 0},
		{"max_muxing_queue_size", s.MuxingQueueSize, 2048},
		{"downmix_audio_boost", s.DownmixAudioBoost, 2.0},
		{"downmix_stereo_algorithm", s.DownmixStereoAlgorithm, contracts.DownmixNone},
		{"h264_crf", s.H264CRF, 23},
		{"h265_crf", s.H265CRF, 28},
		{"av1_crf", s.AV1CRF, 32},
		{"h264_preset", s.H264Preset, ""},
		{"h265_preset", s.H265Preset, ""},
		{"av1_preset", s.AV1Preset, ""},
		{"transcode_temp_path", s.TranscodeTempPath, ""},
		{"remote_bitrate_limit_kbps", s.RemoteBitrateLimitKbps, 0},
		{"audio_vbr", s.AudioVBR, false},
		{"deinterlace_double_rate", s.DeinterlaceDoubleRate, false},
		{"hardware_decode_10bit_hevc", s.HardwareDecode10BitHEVC, false},
		{"hardware_decode_10bit_vp9", s.HardwareDecode10BitVP9, false},
	}
	for _, c := range checks {
		if c.got != c.want {
			t.Errorf("default %s = %v, want %v", c.name, c.got, c.want)
		}
	}
}

// TestAdvancedSettingsRoundTrip pins the admin PUT: a partial document
// is normalized, accepted, and read back through a following GET.
func TestAdvancedSettingsRoundTrip(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	temp := t.TempDir()

	rec := do(t, srv, "PUT", "/api/admin/settings/transcode", map[string]any{
		"thread_count":        4,
		"transcode_temp_path": temp,
		"h264_crf":            30,
	}, admin)
	if rec.Code != http.StatusOK {
		t.Fatalf("PUT: %d %s", rec.Code, rec.Body.String())
	}
	var put advSettingsPayload
	if err := json.Unmarshal(rec.Body.Bytes(), &put); err != nil {
		t.Fatal(err)
	}
	if put.Settings.ThreadCount != 4 || put.Settings.TranscodeTempPath != temp || put.Settings.H264CRF != 30 {
		t.Fatalf("PUT echoed %+v", put.Settings)
	}

	rec = do(t, srv, "GET", "/api/admin/settings/transcode", nil, admin)
	if rec.Code != http.StatusOK {
		t.Fatalf("GET after PUT: %d %s", rec.Code, rec.Body.String())
	}
	var got advSettingsPayload
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	if got.Settings.ThreadCount != 4 || got.Settings.TranscodeTempPath != temp || got.Settings.H264CRF != 30 {
		t.Fatalf("persisted settings = %+v", got.Settings)
	}
}

// TestAdvancedTempPathArtifactsAndNoLeak pins the operator temp path end
// to end: the artifact lands under <temp>/lain-transcode and the client
// status never reveals where it went.
func TestAdvancedTempPathArtifactsAndNoLeak(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	temp := t.TempDir()

	if rec := do(t, srv, "PUT", "/api/admin/settings/transcode", map[string]any{"transcode_temp_path": temp}, admin); rec.Code != http.StatusOK {
		t.Fatalf("settings PUT: %d %s", rec.Code, rec.Body.String())
	}
	id := catalogOneMKV(t, srv, admin, t.TempDir(), "[Fansub-A] Show.mkv")

	rec := do(t, srv, "POST", "/api/items/"+id+"/transcode", map[string]any{"delivery": "progressive"}, admin)
	if rec.Code != http.StatusOK && rec.Code != http.StatusAccepted {
		t.Fatalf("start: %d %s", rec.Code, rec.Body.String())
	}
	var started struct {
		Session string `json:"session"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &started); err != nil || started.Session == "" {
		t.Fatalf("start body: %d %s", rec.Code, rec.Body.String())
	}

	body, _ := advWaitReady(t, srv, admin, id, started.Session)
	artifact := filepath.Join(temp, "lain-transcode", started.Session, "media.mp4")
	if _, err := os.Stat(artifact); err != nil {
		t.Fatalf("artifact not under the configured temp path: %v", err)
	}
	if strings.Contains(body, temp) || strings.Contains(body, "lain-transcode") {
		t.Fatalf("status leaked the temp path: %s", body)
	}
}

// TestAdvancedPerUserStreamLimit pins D-042's max_streams admission.
// The two admin sessions deliberately occupy both concurrency slots, so
// the account's first session stays queued while the second start is
// refused with 429 "too-many-streams". Once that first session is ready
// it no longer counts, which is the documented behavior: only
// queued/running sessions are simultaneous.
func TestAdvancedPerUserStreamLimit(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	dir := t.TempDir()
	// The slow source keeps the account session pending; mpeg4 forces a
	// real re-encode instead of an instant remux.
	slow := "[Fansub-A] Slow Show.mkv"
	fast := "[Fansub-A] Fast Show.mkv"
	advSynthMKV(t, filepath.Join(dir, slow),
		"-f", "lavfi", "-i", "testsrc2=size=320x240:rate=25:duration=20", "-c:v", "mpeg4", "-q:v", "6")
	advSynthMKV(t, filepath.Join(dir, fast),
		"-f", "lavfi", "-i", "testsrc=size=160x120:rate=10:duration=2", "-c:v", "libx264", "-pix_fmt", "yuv420p")
	ids := advScanCatalog(t, srv, admin, dir)
	slowID, fastID := ids[slow], ids[fast]
	if slowID == "" || fastID == "" {
		t.Fatalf("catalog ids missing: %v", ids)
	}

	rec := do(t, srv, "POST", "/api/users", map[string]string{"username": "ana", "password": "password123"}, admin)
	if rec.Code != http.StatusCreated {
		t.Fatalf("create user: %d %s", rec.Code, rec.Body.String())
	}
	var created struct {
		ID string `json:"id"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &created); err != nil || created.ID == "" {
		t.Fatalf("created user: %s", rec.Body.String())
	}
	rec = do(t, srv, "PATCH", "/api/users/"+created.ID, map[string]any{
		"playback": map[string]any{"max_streams": 1},
	}, admin)
	if rec.Code != http.StatusOK {
		t.Fatalf("patch max_streams: %d %s", rec.Code, rec.Body.String())
	}
	ana := loginAs(t, srv, "ana", "password123")

	// Two distinct sessions on the slow source hold both slots; the
	// quality names make the profile keys different.
	for _, quality := range []string{"1080p", "720p"} {
		rec := do(t, srv, "POST", "/api/items/"+slowID+"/transcode",
			map[string]any{"delivery": "progressive", "quality": quality}, admin)
		if rec.Code != http.StatusOK && rec.Code != http.StatusAccepted {
			t.Fatalf("slot holder %s: %d %s", quality, rec.Code, rec.Body.String())
		}
	}

	recA := do(t, srv, "POST", "/api/items/"+slowID+"/transcode", map[string]any{"delivery": "progressive"}, ana)
	if recA.Code != http.StatusAccepted {
		t.Fatalf("first account start: %d %s", recA.Code, recA.Body.String())
	}
	var first struct {
		Session string `json:"session"`
		State   string `json:"state"`
	}
	if err := json.Unmarshal(recA.Body.Bytes(), &first); err != nil || first.Session == "" {
		t.Fatalf("first start body: %d %s", recA.Code, recA.Body.String())
	}
	if first.State != contracts.TranscodeQueued && first.State != contracts.TranscodeRunning {
		t.Fatalf("first session state=%q, want queued or running", first.State)
	}

	recB := do(t, srv, "POST", "/api/items/"+fastID+"/transcode", map[string]any{"delivery": "progressive"}, ana)
	if recB.Code != http.StatusTooManyRequests {
		t.Fatalf("second start: %d %s, want 429", recB.Code, recB.Body.String())
	}
	var apiErr struct {
		Error string `json:"error"`
		Code  string `json:"code"`
	}
	if err := json.Unmarshal(recB.Body.Bytes(), &apiErr); err != nil || apiErr.Code != "too-many-streams" {
		t.Fatalf("second start body=%s, want code too-many-streams", recB.Body.String())
	}

	// A finished session is not a simultaneous stream: once the first
	// session is ready the same second start must be admitted.
	advWaitReady(t, srv, ana, slowID, first.Session)
	recB = do(t, srv, "POST", "/api/items/"+fastID+"/transcode", map[string]any{"delivery": "progressive"}, ana)
	if recB.Code != http.StatusOK && recB.Code != http.StatusAccepted {
		t.Fatalf("start after the first session finished: %d %s", recB.Code, recB.Body.String())
	}
}

// TestAdvancedRemoteBitrateLimit pins the server-wide remote cap: an
// invalid value is refused, and a valid one tightens the effective
// bitrate the gateway resolves into the session.
func TestAdvancedRemoteBitrateLimit(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)

	// A negative cap must never reach a session.
	rec := do(t, srv, "PUT", "/api/admin/settings/transcode", map[string]any{"remote_bitrate_limit_kbps": -1}, admin)
	if rec.Code != http.StatusBadRequest {
		t.Fatalf("negative cap: %d %s, want 400", rec.Code, rec.Body.String())
	}

	if rec := do(t, srv, "PUT", "/api/admin/settings/transcode", map[string]any{"remote_bitrate_limit_kbps": 2000}, admin); rec.Code != http.StatusOK {
		t.Fatalf("cap PUT: %d %s", rec.Code, rec.Body.String())
	}
	id := catalogOneMKV(t, srv, admin, t.TempDir(), "[Fansub-A] Show.mkv")

	// The 1080p preset asks for 8000 kbps, but the effective cap is the
	// tighter server-wide 2000 the gateway resolved.
	rec = do(t, srv, "POST", "/api/items/"+id+"/transcode",
		map[string]any{"delivery": "progressive", "quality": "1080p"}, admin)
	if rec.Code != http.StatusOK && rec.Code != http.StatusAccepted {
		t.Fatalf("start: %d %s", rec.Code, rec.Body.String())
	}
	var status struct {
		Session     string `json:"session"`
		BitrateKbps int    `json:"bitrate_kbps"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &status); err != nil || status.Session == "" {
		t.Fatalf("start body: %d %s", rec.Code, rec.Body.String())
	}
	if status.BitrateKbps != 2000 {
		t.Fatalf("bitrate_kbps=%d, want the effective 2000 cap", status.BitrateKbps)
	}
}
