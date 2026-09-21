// Operator settings for the transcode pipeline (D-045): JSON in the
// bbolt meta bucket, resolved by the gateway and passed inside the
// contract input. The plugin never reads the bucket.
package gateway

import (
	"net/http"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/auth"
	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/kv"
)

const transcodeSettingsKey = "transcode_settings"

// SettingsStore persists operator policy in the meta bucket.
type SettingsStore struct {
	db *bolt.DB
}

// HasTranscode distinguishes a fresh install from an operator who explicitly
// saved the software backend. That distinction is load-bearing for D-063:
// detection chooses a first-boot default but never rewrites a saved choice.
func (st *SettingsStore) HasTranscode() (bool, error) {
	var out contracts.TranscodeSettings
	err := st.db.View(func(tx *bolt.Tx) error {
		return kv.GetJSON(tx, kv.BMeta, []byte(transcodeSettingsKey), &out)
	})
	if kv.IsNotFound(err) {
		return false, nil
	}
	return err == nil, err
}

// Transcode returns the effective transcode settings (defaults when
// nothing was saved yet).
func (st *SettingsStore) Transcode() contracts.TranscodeSettings {
	var out contracts.TranscodeSettings
	err := st.db.View(func(tx *bolt.Tx) error {
		return kv.GetJSON(tx, kv.BMeta, []byte(transcodeSettingsKey), &out)
	})
	if err != nil {
		return contracts.DefaultTranscodeSettings()
	}
	return out.Normalize()
}

// SaveTranscode validates and stores the settings.
func (st *SettingsStore) SaveTranscode(in contracts.TranscodeSettings) error {
	in = in.Normalize()
	if err := in.Validate(); err != nil {
		return err
	}
	return st.db.Update(func(tx *bolt.Tx) error {
		return kv.PutJSON(tx, kv.BMeta, []byte(transcodeSettingsKey), in)
	})
}

// Ensure writes the given settings when none exist yet (first boot
// adopts CLI defaults without overwriting an operator choice).
func (st *SettingsStore) Ensure(base contracts.TranscodeSettings) error {
	var existing contracts.TranscodeSettings
	err := st.db.View(func(tx *bolt.Tx) error {
		return kv.GetJSON(tx, kv.BMeta, []byte(transcodeSettingsKey), &existing)
	})
	if err == nil {
		return nil
	}
	return st.SaveTranscode(base)
}

// automaticHardwarePreference is deterministic when a machine exposes more
// than one API for the same GPU: vendor-native encoders win over generic
// Linux APIs, and dedicated SoC media paths win over V4L2's fallback wrapper.
var automaticHardwarePreference = []string{
	contracts.HWNVENC,
	contracts.HWQSV,
	contracts.HWVAAPI,
	contracts.HWVideoToolbox,
	contracts.HWAMF,
	contracts.HWRKMPP,
	contracts.HWV4L2M2M,
}

func withAutomaticHardware(settings contracts.TranscodeSettings, available map[string]bool) contracts.TranscodeSettings {
	settings.HardwareAcceleration = contracts.HWNone
	settings.HardwareEncode = false
	for _, backend := range automaticHardwarePreference {
		if available[backend] {
			settings.HardwareAcceleration = backend
			settings.HardwareEncode = true
			break
		}
	}
	return settings
}

func (s *Server) handleTranscodeSettingsGet(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	settings := s.settings.Transcode()
	writeJSON(w, http.StatusOK, map[string]any{
		"settings":     settings,
		"capabilities": s.probeTranscode(settings),
	})
}

func (s *Server) handleTranscodeSettingsPut(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	// Decode over the current settings so a partial document keeps every
	// omitted field. A plain bool cannot tell "false" from "unset", so a
	// zero value would silently disable throttle, downmix and tone
	// mapping, which all default to true.
	in := s.settings.Transcode()
	if !s.decode(w, r, &in) {
		return
	}
	if err := s.settings.SaveTranscode(in); err != nil {
		writeJSON(w, http.StatusBadRequest, map[string]string{"error": err.Error(), "code": "invalid-message"})
		return
	}
	settings := s.settings.Transcode()
	s.logger().Info("transcode settings updated", "req", reqIDOf(r))
	writeJSON(w, http.StatusOK, map[string]any{
		"settings":     settings,
		"capabilities": s.probeTranscode(settings),
	})
}

// playbackOptions is the player-facing subset of the policy: what the
// quality menu may offer and which delivery the server prefers.
type playbackOptions struct {
	DefaultDelivery string                       `json:"default_delivery"`
	Deliveries      []string                     `json:"deliveries"`
	Qualities       []contracts.TranscodeQuality `json:"qualities"`
}

func (s *Server) handlePlaybackOptions(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	settings := s.settings.Transcode()
	writeJSON(w, http.StatusOK, playbackOptions{
		DefaultDelivery: settings.DefaultDelivery,
		Deliveries:      []string{contracts.TranscodeDeliveryHLS, contracts.TranscodeDeliveryProgressive},
		Qualities:       settings.Qualities,
	})
}

// adminTranscodeStatus is the operator view: the public projection plus
// the owning account, which the player never needs.
type adminTranscodeStatus struct {
	publicTranscodeStatus
	UserID string `json:"user_id,omitempty"`
}

// handleAdminTranscodes lists active sessions without filesystem paths.
func (s *Server) handleAdminTranscodes(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	out, _, err := s.reg.CallOne(contracts.CapPlaybackTranscodeV3, contracts.TranscodeV3Request{
		Action: contracts.TranscodeListAction,
	})
	if err != nil {
		writeTranscodeError(w, err)
		return
	}
	statuses, _ := out.([]contracts.TranscodeV3Status)
	sessions := make([]adminTranscodeStatus, 0, len(statuses))
	for _, st := range statuses {
		sessions = append(sessions, adminTranscodeStatus{publicTranscodeStatus: publicStatusV3(st), UserID: st.UserID})
	}
	writeJSON(w, http.StatusOK, map[string]any{"sessions": sessions})
}

// handleAdminTranscodeCancel stops one session by id.
func (s *Server) handleAdminTranscodeCancel(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	session := r.PathValue("session")
	if session == "" {
		writeErr(w, http.StatusBadRequest, "session required")
		return
	}
	out, _, err := s.reg.CallOne(contracts.CapPlaybackTranscodeV3, contracts.TranscodeV3Request{
		Action: contracts.TranscodeCancelAction, Session: session,
	})
	if err != nil {
		writeTranscodeError(w, err)
		return
	}
	status, _ := out.(contracts.TranscodeV3Status)
	s.logger().Info("transcode cancelled by admin", "req", reqIDOf(r), "session", session)
	writeJSON(w, http.StatusOK, publicStatusV3(status))
}
