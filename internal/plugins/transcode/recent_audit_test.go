package transcode

// Adversarial audit of the newest transcode surface: decode-only
// hardware acceleration, hardware device resolution, the visible
// software fallback, the relocated artifact path round trip, profile-key
// identity and hardware_device validation. Each test pins one seam whose
// regression would stay silent in production.

import (
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
)

// auditForcedEncode plans a session whose video must be re-encoded even
// though the source is web-safe, so the encoder path is always reached
// (the client forbids stream copy).
func auditForcedEncode(t *testing.T, caps capabilities, mutate func(*contracts.TranscodeV3Request)) encodePlan {
	t.Helper()
	no := false
	return advRequirePlan(t, advVideoReport("h264", "yuv420p", "8"), caps, func(in *contracts.TranscodeV3Request) {
		in.AllowVideoStreamCopy = &no
		if mutate != nil {
			mutate(in)
		}
	})
}

// auditWaitV3Ready polls one session until it is ready; real ffmpeg
// preparation is asynchronous, so a deadline is the only safe wait.
func auditWaitV3Ready(t *testing.T, tr *Transcoder, src, session string) contracts.TranscodeV3Status {
	t.Helper()
	deadline := time.Now().Add(60 * time.Second)
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
		case contracts.TranscodeReady:
			return status
		case contracts.TranscodeFailed:
			t.Fatalf("session failed: %s (%s)", status.Error, status.ErrorCode)
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("session %s did not become ready", session)
	return contracts.TranscodeV3Status{}
}

// TestAuditDecodeOnlyHardwareAcceleration pins the decode-only half: a
// plan with hardware_acceleration set but hardware_encode off must still
// reach ffmpeg as -hwaccel <backend> plus the configured device instead
// of silently staying software.
func TestAuditDecodeOnlyHardwareAcceleration(t *testing.T) {
	plan := auditForcedEncode(t, advCaps(), func(in *contracts.TranscodeV3Request) {
		in.Settings.HardwareAcceleration = contracts.HWVAAPI
		in.Settings.HardwareEncode = false
	})
	if plan.hwBackend != "" {
		t.Fatalf("decode-only plan carries encode backend %q, want none", plan.hwBackend)
	}
	if !plan.hwDecode {
		t.Fatal("decode-only plan did not request hardware decoding")
	}
	if got := plan.hardwareLabel(); got != contracts.HWVAAPI {
		t.Fatalf("hardwareLabel()=%q, want %q for decode-only acceleration", got, contracts.HWVAAPI)
	}
	args := advArgs(t, plan)
	if got, ok := advFlagValue(args, "-hwaccel"); !ok || got != contracts.HWVAAPI {
		t.Fatalf("-hwaccel=%q ok=%v in %v, want vaapi", got, ok, args)
	}
	if got, ok := advFlagValue(args, "-hwaccel_device"); !ok || got != contracts.DefaultHardwareDevice {
		t.Fatalf("-hwaccel_device=%q ok=%v, want %q", got, ok, contracts.DefaultHardwareDevice)
	}
	if got, ok := advFlagValue(args, "-c:v"); !ok || got != "libx264" {
		t.Fatalf("-c:v=%q ok=%v, want the software libx264", got, ok)
	}

	// A custom render node, trimmed, is what must reach ffmpeg.
	plan = auditForcedEncode(t, advCaps(), func(in *contracts.TranscodeV3Request) {
		in.Settings.HardwareAcceleration = contracts.HWVAAPI
		in.Settings.HardwareDevice = "  /dev/dri/renderD129  "
	})
	if got, ok := advFlagValue(advArgs(t, plan), "-hwaccel_device"); !ok || got != "/dev/dri/renderD129" {
		t.Fatalf("-hwaccel_device=%q ok=%v, want the trimmed custom device", got, ok)
	}
}

// TestAuditHardwareEncodeNamesBackend pins the encode half: with
// hardware encode on, the VAAPI encoder is named in the argv and
// hardwareLabel() reports exactly the encode backend.
func TestAuditHardwareEncodeNamesBackend(t *testing.T) {
	plan := auditForcedEncode(t, advCaps(), func(in *contracts.TranscodeV3Request) {
		in.Settings.HardwareAcceleration = contracts.HWVAAPI
		in.Settings.HardwareEncode = true
	})
	if plan.encoder != "h264_vaapi" {
		t.Fatalf("encoder=%q, want h264_vaapi", plan.encoder)
	}
	if plan.hwBackend != contracts.HWVAAPI {
		t.Fatalf("hwBackend=%q, want %q", plan.hwBackend, contracts.HWVAAPI)
	}
	if got := plan.hardwareLabel(); got != plan.hwBackend {
		t.Fatalf("hardwareLabel()=%q, want the encode backend %q", got, plan.hwBackend)
	}
	args := advArgs(t, plan)
	if got, ok := advFlagValue(args, "-c:v"); !ok || got != "h264_vaapi" {
		t.Fatalf("-c:v=%q ok=%v, want h264_vaapi", got, ok)
	}
	if got, ok := advFlagValue(args, "-hwaccel"); !ok || got != contracts.HWVAAPI {
		t.Fatalf("-hwaccel=%q ok=%v, want vaapi", got, ok)
	}
}

// TestAuditHardwareOffHasNoLabel pins the conservative default: with
// hardware off nothing claims acceleration and no hwaccel flag appears.
func TestAuditHardwareOffHasNoLabel(t *testing.T) {
	plan := auditForcedEncode(t, advCaps(), nil)
	if got := plan.hardwareLabel(); got != "" {
		t.Fatalf("hardwareLabel()=%q with hardware off, want empty", got)
	}
	if plan.hwDecode || plan.hwBackend != "" {
		t.Fatalf("plan requests hardware with it off: hwDecode=%v hwBackend=%q", plan.hwDecode, plan.hwBackend)
	}
	for _, arg := range advArgs(t, plan) {
		if arg == "-hwaccel" || arg == "-hwaccel_device" {
			t.Fatalf("software plan carries %q", arg)
		}
	}
}

// TestAuditHardwareDeviceResolution pins the VA-API render node
// resolution: unset and whitespace-only fall back to the shipped
// default, a custom device is kept verbatim, surrounding spaces are
// trimmed.
func TestAuditHardwareDeviceResolution(t *testing.T) {
	cases := []struct {
		name   string
		device string
		want   string
	}{
		{"empty falls back to the shipped default", "", contracts.DefaultHardwareDevice},
		{"whitespace is not a device", "   \t ", contracts.DefaultHardwareDevice},
		{"custom device is kept verbatim", "/dev/dri/renderD129", "/dev/dri/renderD129"},
		{"surrounding spaces are trimmed", "  /dev/dri/renderD130  ", "/dev/dri/renderD130"},
	}
	for _, tc := range cases {
		settings := contracts.DefaultTranscodeSettings()
		settings.HardwareDevice = tc.device
		if got := hardwareDevice(settings); got != tc.want {
			t.Errorf("%s: hardwareDevice(%q)=%q, want %q", tc.name, tc.device, got, tc.want)
		}
	}
}

// TestAuditSoftwareFallbackStripsHardware pins the runtime fallback: a
// failed hardware attempt must retry with no hwDecode, no hwBackend, no
// hwupload/format=nv12 filters, and a visible note.
func TestAuditSoftwareFallbackStripsHardware(t *testing.T) {
	plan := auditForcedEncode(t, advCaps(), func(in *contracts.TranscodeV3Request) {
		in.Settings.HardwareAcceleration = contracts.HWVAAPI
		in.Settings.HardwareEncode = true
	})
	if !slices.Contains(plan.filters, "hwupload") {
		t.Fatalf("scenario is not hardware-uploading: filters=%v", plan.filters)
	}

	fb := plan.softwareFallback(advCaps())
	if fb.hwDecode {
		t.Error("software fallback kept hwDecode set")
	}
	if fb.hwBackend != "" {
		t.Errorf("software fallback kept hwBackend=%q", fb.hwBackend)
	}
	if fb.encoder != "libx264" {
		t.Errorf("software fallback encoder=%q, want libx264", fb.encoder)
	}
	for _, f := range fb.filters {
		if f == "hwupload" || f == "format=nv12" {
			t.Errorf("software fallback kept hardware filter %q in %v", f, fb.filters)
		}
	}
	if !strings.Contains(fb.fallback, "software") {
		t.Errorf("fallback note %q does not explain the software retry", fb.fallback)
	}
	for _, arg := range advArgs(t, fb) {
		if arg == "-hwaccel" || arg == "-hwaccel_device" {
			t.Errorf("fallback argv still carries %q", arg)
		}
	}
}

// TestAuditSoftwareFallbackBurnInGraph pins the burn-in variant of the
// fallback: an image subtitle turns the filter chain into a
// complexFilter, which the software retry must clean too — otherwise
// the retry replays the VAAPI-only hwupload graph and fails again.
func TestAuditSoftwareFallbackBurnInGraph(t *testing.T) {
	plan := advRequirePlan(t, advSubtitleReport("hdmv_pgs_subtitle"), advCaps(), func(in *contracts.TranscodeV3Request) {
		in.Settings.HardwareAcceleration = contracts.HWVAAPI
		in.Settings.HardwareEncode = true
		idx := 2
		in.SubtitleStream = &idx
		in.SubtitleMode = contracts.SubtitleModeBurn
	})
	if !strings.Contains(plan.complexFilter, "hwupload") {
		t.Fatalf("scenario is not hardware-uploading: complexFilter=%q", plan.complexFilter)
	}
	fb := plan.softwareFallback(advCaps())
	// The software retry must not replay the hardware upload chain.
	if strings.Contains(fb.complexFilter, "hwupload") || strings.Contains(fb.complexFilter, "format=nv12") {
		t.Fatalf("softwareFallback left the hardware upload chain in complexFilter=%q", fb.complexFilter)
	}
}

// TestAuditRelocatedPathsRoundTrip pins the relocated layout identity:
// the paths pathsFor writes and the paths a recorded cache entry
// resolves must be the same, the lain-transcode namespace must appear
// once, and removeArtifacts must delete the file written there.
func TestAuditRelocatedPathsRoundTrip(t *testing.T) {
	tr := hlsTestTranscoder(t)
	session := "audit-relocated-session"
	settings := contracts.DefaultTranscodeSettings()
	settings.TranscodeTempPath = filepath.Join(t.TempDir(), "transcode-temp")

	paths := tr.pathsFor(session, settings)
	entry := cacheEntry{Session: session, ArtifactRoot: relocateRoot(settings)}
	resolved := tr.pathsForEntry(entry)
	if resolved != paths {
		t.Fatalf("pathsForEntry=%+v, want the pathsFor layout %+v", resolved, paths)
	}

	// Exactly one lain-transcode segment: appending the namespace again
	// would bury artifacts under .../lain-transcode/lain-transcode/...
	for _, dir := range []string{filepath.Dir(paths.media), filepath.Dir(resolved.media)} {
		count := 0
		for _, part := range strings.Split(dir, string(filepath.Separator)) {
			if part == "lain-transcode" {
				count++
			}
		}
		if count != 1 {
			t.Fatalf("resolved directory %q has %d lain-transcode segments, want 1", dir, count)
		}
	}

	// removeArtifacts must delete what pathsFor wrote, at the relocated
	// location recorded in the entry.
	if err := os.MkdirAll(filepath.Dir(paths.media), 0o700); err != nil {
		t.Fatal(err)
	}
	writeTestFile(t, paths.media, "media-bytes")
	writeTestFile(t, paths.playlist, "#EXTM3U\n")
	writeTestFile(t, paths.subtitle, "WEBVTT\n\n")
	tr.removeArtifacts(entry, resolved)
	for _, path := range []string{paths.media, paths.playlist, paths.subtitle} {
		if _, err := os.Stat(path); !os.IsNotExist(err) {
			t.Fatalf("removeArtifacts left %q behind: %v", path, err)
		}
	}
	if _, err := os.Stat(filepath.Dir(paths.media)); !os.IsNotExist(err) {
		t.Fatalf("removeArtifacts left the session directory behind: %v", err)
	}
}

// TestAuditTranscodeProfileKeyHardwareIdentity pins the cache identity:
// the device is placement, not bytes, so it must not change the key,
// while every hardware switch that can change the bytes must.
func TestAuditTranscodeProfileKeyHardwareIdentity(t *testing.T) {
	base := contracts.DefaultTranscodeSettings()
	req := contracts.TranscodeV3Request{Delivery: contracts.TranscodeDeliveryHLS, Settings: base}
	key := contracts.TranscodeProfileKey(base, req)

	deviceChanged := base
	deviceChanged.HardwareDevice = "/dev/dri/renderD129"
	if got := contracts.TranscodeProfileKey(deviceChanged, req); got != key {
		t.Fatalf("profile key changed with HardwareDevice: %q != %q", got, key)
	}

	for _, tc := range []struct {
		name   string
		mutate func(*contracts.TranscodeSettings)
	}{
		{"hardware_acceleration", func(s *contracts.TranscodeSettings) { s.HardwareAcceleration = contracts.HWVAAPI }},
		{"hardware_encode", func(s *contracts.TranscodeSettings) { s.HardwareEncode = true }},
		{"hardware_decode_10bit_hevc", func(s *contracts.TranscodeSettings) { s.HardwareDecode10BitHEVC = true }},
		{"hardware_decode_10bit_vp9", func(s *contracts.TranscodeSettings) { s.HardwareDecode10BitVP9 = true }},
		{"hardware_decode_codecs", func(s *contracts.TranscodeSettings) { s.HardwareDecodeCodecs = []string{"h264"} }},
		{"ffmpeg_path", func(s *contracts.TranscodeSettings) { s.FFmpegPath = "/opt/ffmpeg/bin/ffmpeg" }},
	} {
		changed := base
		tc.mutate(&changed)
		if got := contracts.TranscodeProfileKey(changed, req); got == key {
			t.Errorf("profile key %q unchanged by %s", got, tc.name)
		}
	}
}

// TestAuditValidateHardwareDevice pins the argv-safety contract: real
// render nodes and the normalized empty value are accepted, while
// embedded control characters or an overlong value are rejected.
func TestAuditValidateHardwareDevice(t *testing.T) {
	base := contracts.DefaultTranscodeSettings()
	accepted := []struct {
		name   string
		device string
	}{
		{"shipped default", contracts.DefaultHardwareDevice},
		{"explicit render node", "/dev/dri/renderD129"},
		{"empty means the default", ""},
		{"whitespace-only is trimmed away", "   "},
		{"512 bytes is the accepted maximum", strings.Repeat("a", 512)},
	}
	for _, tc := range accepted {
		settings := base
		settings.HardwareDevice = tc.device
		if err := settings.Validate(); err != nil {
			t.Errorf("Validate(%s, %q) = %v, want nil", tc.name, tc.device, err)
		}
	}

	// Normalize must fill an empty device with the shipped default
	// before validation ever sees it.
	normalized := (contracts.TranscodeSettings{HardwareDevice: ""}).Normalize()
	if normalized.HardwareDevice != contracts.DefaultHardwareDevice {
		t.Fatalf("Normalize left HardwareDevice=%q", normalized.HardwareDevice)
	}
	if err := normalized.Validate(); err != nil {
		t.Fatalf("Validate(Normalize()) = %v, want nil", err)
	}

	rejected := []struct {
		name   string
		device string
	}{
		{"embedded newline", "/dev/dri/re\nnderD129"},
		{"embedded carriage return", "/dev/dri/re\rnderD129"},
		{"embedded NUL", "/dev/dri/re\x00nderD129"},
		{"longer than 512 bytes", strings.Repeat("a", 513)},
	}
	for _, tc := range rejected {
		settings := base
		settings.HardwareDevice = tc.device
		err := settings.Validate()
		if err == nil {
			t.Errorf("Validate(%s) accepted %q", tc.name, tc.device)
			continue
		}
		if !strings.Contains(err.Error(), "hardware_device") {
			t.Errorf("Validate(%s) error %q does not name hardware_device", tc.name, err)
		}
	}
}

// TestAuditRealHLSSessionProducesParsablePlaylist is the real end-to-end
// slice: a synthesized MKV drives a full HLS session through the plugin
// and the produced index.m3u8 must parse back with at least one segment.
func TestAuditRealHLSSessionProducesParsablePlaylist(t *testing.T) {
	requireFFmpeg(t)
	if _, err := exec.LookPath("ffprobe"); err != nil {
		t.Skip("ffprobe not installed")
	}
	root := t.TempDir()
	src := makeMKV(t, root, "[Fansub-A] Audit HLS.mkv", "libx264", "aac")

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
	started, ok := out.(contracts.TranscodeV3Status)
	if !ok || started.Session == "" {
		t.Fatalf("start returned %T (%+v)", out, out)
	}
	status := auditWaitV3Ready(t, tr, src, started.Session)
	if status.PlaylistPath == "" {
		t.Fatalf("ready status has no playlist path: %+v", status)
	}

	segments, ended, err := parseHLSPlaylist(status.PlaylistPath)
	if err != nil {
		t.Fatalf("parse %s: %v", status.PlaylistPath, err)
	}
	if len(segments) == 0 {
		t.Fatalf("index.m3u8 at %s reports no segments", status.PlaylistPath)
	}
	if !ended {
		t.Fatalf("finished session playlist lacks ENDLIST: %s", status.PlaylistPath)
	}
	if !hlsPlayable(filepath.Dir(status.PlaylistPath)) {
		t.Fatalf("hlsPlayable says %s is not playable", status.PlaylistPath)
	}
	// The gateway signs these URI lines; they must be bare file names so
	// the advertised URIs stay relative to the session directory.
	for _, seg := range segments {
		if filepath.Base(seg.URI) != seg.URI {
			t.Fatalf("segment URI %q is not a bare file name", seg.URI)
		}
	}
}

// TestAuditRealMPEGTSHLSSession pins the HLS container choice end to end:
// an mpegts session produces .ts segments, no init segment, and a
// playable index with no #EXT-X-MAP.
func TestAuditRealMPEGTSHLSSession(t *testing.T) {
	requireFFmpeg(t)
	if _, err := exec.LookPath("ffprobe"); err != nil {
		t.Skip("ffprobe not installed")
	}
	root := t.TempDir()
	src := makeMKV(t, root, "[Fansub-A] Audit TS.mkv", "libx264", "aac")

	tr := New(filepath.Join(root, "cache"))
	t.Cleanup(func() { _ = tr.Close() })

	settings := contracts.DefaultTranscodeSettings()
	settings.HLSSegmentContainer = contracts.HLSSegmentTS
	out, err := tr.Invoke(contracts.CapPlaybackTranscodeV3, contracts.TranscodeV3Request{
		Action:   contracts.TranscodeStartAction,
		FilePath: src,
		Delivery: contracts.TranscodeDeliveryHLS,
		Settings: settings,
	})
	if err != nil {
		t.Fatalf("start: %v", err)
	}
	started, ok := out.(contracts.TranscodeV3Status)
	if !ok || started.Session == "" {
		t.Fatalf("start returned %T (%+v)", out, out)
	}
	status := auditWaitV3Ready(t, tr, src, started.Session)
	if status.PlaylistPath == "" {
		t.Fatalf("ready status has no playlist path: %+v", status)
	}
	dir := filepath.Dir(status.PlaylistPath)
	segments, _, err := parseHLSPlaylist(status.PlaylistPath)
	if err != nil {
		t.Fatalf("parse %s: %v", status.PlaylistPath, err)
	}
	if len(segments) == 0 {
		t.Fatalf("mpegts index has no segments: %s", status.PlaylistPath)
	}
	for _, seg := range segments {
		if !strings.HasSuffix(seg.URI, ".ts") {
			t.Fatalf("mpegts segment URI %q is not a .ts file", seg.URI)
		}
	}
	if hlsMapURI(status.PlaylistPath) != "" {
		t.Fatalf("mpegts index must not reference an init segment: %s", status.PlaylistPath)
	}
	if _, err := os.Stat(filepath.Join(dir, hlsInitSegment)); err == nil {
		t.Fatalf("mpegts session produced an init segment")
	}
	if !hlsPlayable(dir) {
		t.Fatalf("hlsPlayable says the mpegts session is not playable")
	}
}
