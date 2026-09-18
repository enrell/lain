package gateway

import (
	"encoding/json"
	"net/http"
	"reflect"
	"slices"
	"testing"

	"github.com/enrell/lain/internal/contracts"
)

// transcodeSettingsPayload mirrors the admin endpoint response: the
// stored policy plus the probe summary (D-045/D-031).
type transcodeSettingsPayload struct {
	Settings     contracts.TranscodeSettings `json:"settings"`
	Capabilities struct {
		FFmpeg            string          `json:"ffmpeg"`
		Encoders          []string        `json:"encoders"`
		ToneMapping       bool            `json:"tone_mapping"`
		ToneMappingBT2390 bool            `json:"tone_mapping_bt2390"`
		Hardware          map[string]bool `json:"hardware"`
	} `json:"capabilities"`
}

// TestTranscodeSettingsAdminGate pins the operator boundary: settings
// are admin-only on both read and write.
func TestTranscodeSettingsAdminGate(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)

	if rec := do(t, srv, "GET", "/api/admin/settings/transcode", nil, ""); rec.Code != http.StatusUnauthorized {
		t.Fatalf("unauthenticated GET: %d, want 401", rec.Code)
	}
	if rec := do(t, srv, "PUT", "/api/admin/settings/transcode", contracts.DefaultTranscodeSettings(), ""); rec.Code != http.StatusUnauthorized {
		t.Fatalf("unauthenticated PUT: %d, want 401", rec.Code)
	}

	rec := do(t, srv, "POST", "/api/users", map[string]string{"username": "ana", "password": "password123"}, admin)
	if rec.Code != http.StatusCreated {
		t.Fatalf("create user: %d %s", rec.Code, rec.Body.String())
	}
	user := loginAs(t, srv, "ana", "password123")
	if rec := do(t, srv, "GET", "/api/admin/settings/transcode", nil, user); rec.Code != http.StatusForbidden {
		t.Fatalf("non-admin GET: %d, want 403", rec.Code)
	}
	if rec := do(t, srv, "PUT", "/api/admin/settings/transcode", contracts.DefaultTranscodeSettings(), user); rec.Code != http.StatusForbidden {
		t.Fatalf("non-admin PUT: %d, want 403", rec.Code)
	}
}

// TestTranscodeSettingsReadWrite covers the round trip: defaults plus a
// probe summary, invalid values refused, valid values persisted.
func TestTranscodeSettingsReadWrite(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)

	rec := do(t, srv, "GET", "/api/admin/settings/transcode", nil, admin)
	if rec.Code != http.StatusOK {
		t.Fatalf("GET: %d %s", rec.Code, rec.Body.String())
	}
	var got transcodeSettingsPayload
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(got.Settings, contracts.DefaultTranscodeSettings()) {
		t.Fatalf("defaults = %+v", got.Settings)
	}
	if got.Capabilities.FFmpeg == "" {
		t.Fatal("capabilities must name the probed ffmpeg binary")
	}
	if got.Capabilities.Hardware == nil {
		t.Fatal("capabilities must carry a hardware map")
	}
	if got.Capabilities.ToneMappingBT2390 && !got.Capabilities.ToneMapping {
		t.Fatal("BT.2390 cannot be available without the base tone-map chain")
	}

	bad := contracts.DefaultTranscodeSettings()
	bad.CRF = 99
	rec = do(t, srv, "PUT", "/api/admin/settings/transcode", bad, admin)
	if rec.Code != http.StatusBadRequest {
		t.Fatalf("invalid PUT: %d %s", rec.Code, rec.Body.String())
	}
	var apiErr struct {
		Error string `json:"error"`
		Code  string `json:"code"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &apiErr); err != nil || apiErr.Code != "invalid-message" {
		t.Fatalf("invalid PUT body=%s", rec.Body.String())
	}

	want := contracts.DefaultTranscodeSettings()
	want.DefaultDelivery = contracts.TranscodeDeliveryProgressive
	want.HLSSegmentSeconds = 4
	want.CRF = 20
	want.EncoderPreset = "fast"
	want.QueueSize = 4
	want.MaxConcurrent = 1
	want.SubtitleMode = contracts.SubtitleModeOff
	want.Deinterlace = contracts.DeinterlaceOff
	want.ToneMappingPeakNits = 250
	rec = do(t, srv, "PUT", "/api/admin/settings/transcode", want, admin)
	if rec.Code != http.StatusOK {
		t.Fatalf("valid PUT: %d %s", rec.Code, rec.Body.String())
	}
	var put transcodeSettingsPayload
	if err := json.Unmarshal(rec.Body.Bytes(), &put); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(put.Settings, want.Normalize()) {
		t.Fatalf("PUT echoed %+v, want %+v", put.Settings, want.Normalize())
	}

	rec = do(t, srv, "GET", "/api/admin/settings/transcode", nil, admin)
	if rec.Code != http.StatusOK {
		t.Fatalf("GET after PUT: %d", rec.Code)
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(got.Settings, want.Normalize()) {
		t.Fatalf("persisted settings = %+v, want %+v", got.Settings, want.Normalize())
	}
}

// TestTranscodeSettingsSurviveRestart proves D-045 persistence: the
// operator choice lives in the meta bucket, not in process memory.
func TestTranscodeSettingsSurviveRestart(t *testing.T) {
	dir := t.TempDir()
	srv, err := NewWithOptions(dir, "test", Options{})
	if err != nil {
		t.Fatal(err)
	}
	closed := false
	t.Cleanup(func() {
		if !closed {
			_ = srv.Close()
		}
	})
	admin := setupAdmin(t, srv)

	want := contracts.DefaultTranscodeSettings()
	want.DefaultDelivery = contracts.TranscodeDeliveryProgressive
	want.CRF = 27
	want.EncoderPreset = "slow"
	if rec := do(t, srv, "PUT", "/api/admin/settings/transcode", want, admin); rec.Code != http.StatusOK {
		t.Fatalf("PUT: %d %s", rec.Code, rec.Body.String())
	}
	if err := srv.Close(); err != nil {
		t.Fatal(err)
	}
	closed = true

	srv2, err := NewWithOptions(dir, "test", Options{})
	if err != nil {
		t.Fatalf("reopen: %v", err)
	}
	t.Cleanup(func() { _ = srv2.Close() })
	admin2 := loginAs(t, srv2, "admin", "password123")

	rec := do(t, srv2, "GET", "/api/admin/settings/transcode", nil, admin2)
	if rec.Code != http.StatusOK {
		t.Fatalf("GET after restart: %d %s", rec.Code, rec.Body.String())
	}
	var got transcodeSettingsPayload
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	if got.Settings.CRF != 27 || got.Settings.EncoderPreset != "slow" ||
		got.Settings.DefaultDelivery != contracts.TranscodeDeliveryProgressive {
		t.Fatalf("settings lost across restart: %+v", got.Settings)
	}
}

// TestPlaybackOptionsListsLadder pins the player-facing subset: both
// deliveries and the configured quality names.
func TestPlaybackOptionsListsLadder(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)

	if rec := do(t, srv, "GET", "/api/playback/options", nil, ""); rec.Code != http.StatusUnauthorized {
		t.Fatalf("unauthenticated options: %d, want 401", rec.Code)
	}
	rec := do(t, srv, "GET", "/api/playback/options", nil, admin)
	if rec.Code != http.StatusOK {
		t.Fatalf("options: %d %s", rec.Code, rec.Body.String())
	}
	var opts struct {
		DefaultDelivery string                       `json:"default_delivery"`
		Deliveries      []string                     `json:"deliveries"`
		Qualities       []contracts.TranscodeQuality `json:"qualities"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &opts); err != nil {
		t.Fatal(err)
	}
	if opts.DefaultDelivery != contracts.TranscodeDeliveryHLS {
		t.Fatalf("default_delivery=%q, want hls", opts.DefaultDelivery)
	}
	wantDeliveries := []string{contracts.TranscodeDeliveryHLS, contracts.TranscodeDeliveryProgressive}
	if !reflect.DeepEqual(opts.Deliveries, wantDeliveries) {
		t.Fatalf("deliveries=%v, want %v", opts.Deliveries, wantDeliveries)
	}
	names := make([]string, 0, len(opts.Qualities))
	for _, q := range opts.Qualities {
		names = append(names, q.Name)
	}
	for _, want := range []string{"2160p", "1080p", "720p", "480p", "360p"} {
		if !slices.Contains(names, want) {
			t.Errorf("qualities=%v, want %q", names, want)
		}
	}
}

// TestUserPlaybackPolicyRoundTrip pins the per-user limits (D-042): the
// PATCH response and the user list must both carry them.
func TestUserPlaybackPolicyRoundTrip(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)

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
		"playback": map[string]any{"allow_video_transcode": false, "max_bitrate_kbps": 2000},
	}, admin)
	if rec.Code != http.StatusOK {
		t.Fatalf("patch: %d %s", rec.Code, rec.Body.String())
	}
	var patched struct {
		Playback struct {
			AllowVideoTranscode *bool `json:"allow_video_transcode"`
			MaxBitrateKbps      int   `json:"max_bitrate_kbps"`
		} `json:"playback"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &patched); err != nil {
		t.Fatal(err)
	}
	if patched.Playback.AllowVideoTranscode == nil || *patched.Playback.AllowVideoTranscode {
		t.Fatalf("patched playback=%+v, want allow_video_transcode=false", patched.Playback)
	}
	if patched.Playback.MaxBitrateKbps != 2000 {
		t.Fatalf("patched max_bitrate_kbps=%d, want 2000", patched.Playback.MaxBitrateKbps)
	}

	rec = do(t, srv, "GET", "/api/users", nil, admin)
	if rec.Code != http.StatusOK {
		t.Fatalf("list users: %d", rec.Code)
	}
	var users []struct {
		ID       string `json:"id"`
		Playback struct {
			AllowVideoTranscode *bool `json:"allow_video_transcode"`
			MaxBitrateKbps      int   `json:"max_bitrate_kbps"`
		} `json:"playback"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &users); err != nil {
		t.Fatal(err)
	}
	found := false
	for _, u := range users {
		if u.ID != created.ID {
			continue
		}
		found = true
		if u.Playback.AllowVideoTranscode == nil || *u.Playback.AllowVideoTranscode || u.Playback.MaxBitrateKbps != 2000 {
			t.Fatalf("listed playback=%+v", u.Playback)
		}
	}
	if !found {
		t.Fatalf("user %s missing from /api/users", created.ID)
	}
}

// TestRestrictedUserGetsUnavailablePlan is the honest-degradation
// slice: the admin sees the normal transcode plan, a user forbidden
// from both remux and video transcode sees an unavailable plan instead
// of an endpoint that would refuse the work.
func TestRestrictedUserGetsUnavailablePlan(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	id := catalogOneMKV(t, srv, admin, t.TempDir(), "[Fansub-A] Show.mkv")

	var adminPlan contracts.Plan
	rec := do(t, srv, "GET", "/api/items/"+id+"/playback?client=web", nil, admin)
	if err := json.Unmarshal(rec.Body.Bytes(), &adminPlan); err != nil {
		t.Fatalf("admin playback: %d %s", rec.Code, rec.Body.String())
	}
	if adminPlan.Mode != "transcode" || !adminPlan.Available {
		t.Fatalf("admin plan=%+v, want an available transcode", adminPlan)
	}

	rec = do(t, srv, "POST", "/api/users", map[string]string{"username": "ana", "password": "password123"}, admin)
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
		"playback": map[string]any{"allow_video_transcode": false, "allow_remux": false},
	}, admin)
	if rec.Code != http.StatusOK {
		t.Fatalf("patch: %d %s", rec.Code, rec.Body.String())
	}
	user := loginAs(t, srv, "ana", "password123")

	var userPlan contracts.Plan
	rec = do(t, srv, "GET", "/api/items/"+id+"/playback?client=web", nil, user)
	if err := json.Unmarshal(rec.Body.Bytes(), &userPlan); err != nil {
		t.Fatalf("user playback: %d %s", rec.Code, rec.Body.String())
	}
	if userPlan.Mode != "transcode-required" || userPlan.Available {
		t.Fatalf("restricted plan=%+v, want transcode-required and unavailable", userPlan)
	}

	// The v1 compatibility path carries no per-session options, so it must
	// enforce the same policy: a denied account cannot bypass it.
	rec = do(t, srv, "GET", "/api/items/"+id+"/transcode", nil, user)
	if rec.Code != http.StatusForbidden {
		t.Fatalf("v1 transcode for a denied account: %d %s", rec.Code, rec.Body.String())
	}
}
