// Package settings serves lain.settings.transcode@1: the operator's
// transcode policy persisted in the meta bucket (D-045, D-076). The
// gateway resolves settings through the capability and passes them
// inside the transcode request — the pipeline plugins never read the
// bucket themselves.
package settings

import (
	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/kv"
)

const ID = "lain-settings-bolt"

const transcodeSettingsKey = "transcode_settings"

// Provider owns operator settings in the bbolt meta bucket.
type Provider struct {
	DB *bolt.DB
}

func (Provider) ID() string             { return ID }
func (Provider) Capabilities() []string { return []string{contracts.CapTranscodeSettings} }

// Health reports whether the underlying store is usable.
func (p Provider) Health() error {
	if p.DB == nil {
		return &core.Error{Code: "dependency-unavailable", Msg: "settings store has no db"}
	}
	return nil
}

// PutInput replaces the transcode policy.
type PutInput struct {
	Settings contracts.TranscodeSettings `json:"settings"`
}

// EnsureInput seeds a first-boot default without overwriting an
// operator's saved choice (D-063).
type EnsureInput struct {
	Base contracts.TranscodeSettings `json:"base"`
}

// HasOutput distinguishes a fresh install from an explicit save.
type HasOutput struct {
	Saved bool `json:"saved"`
}

func (p Provider) Invoke(cap string, input any) (any, error) {
	if cap != contracts.CapTranscodeSettings {
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
	if p.DB == nil {
		return nil, &core.Error{Code: "dependency-unavailable", Msg: "settings store has no db"}
	}
	switch in := input.(type) {
	case nil:
		return p.get(), nil
	case PutInput:
		saved, err := p.put(in.Settings)
		if err != nil {
			return nil, err
		}
		return saved, nil
	case EnsureInput:
		return p.ensure(in.Base)
	case HasInput:
		saved, err := p.has()
		if err != nil {
			return nil, err
		}
		return HasOutput{Saved: saved}, nil
	default:
		return nil, &core.Error{Code: "invalid-message", Msg: "settings input required"}
	}
}

// HasInput asks whether an operator save exists.
type HasInput struct{}

// has distinguishes "no save" from "corrupt save": a broken document
// is an error, never a silent reset to defaults.
func (p Provider) has() (bool, error) {
	var out contracts.TranscodeSettings
	err := p.DB.View(func(tx *bolt.Tx) error {
		return kv.GetJSON(tx, kv.BMeta, []byte(transcodeSettingsKey), &out)
	})
	if kv.IsNotFound(err) {
		return false, nil
	}
	return err == nil, err
}

// get returns the effective settings (defaults when nothing is saved
// or the read fails — a corrupted store degrades to shipped policy,
// never to a broken pipeline).
func (p Provider) get() contracts.TranscodeSettings {
	var out contracts.TranscodeSettings
	err := p.DB.View(func(tx *bolt.Tx) error {
		return kv.GetJSON(tx, kv.BMeta, []byte(transcodeSettingsKey), &out)
	})
	if err != nil {
		return contracts.DefaultTranscodeSettings()
	}
	return out.Normalize()
}

func (p Provider) put(in contracts.TranscodeSettings) (contracts.TranscodeSettings, error) {
	in = in.Normalize()
	if err := in.Validate(); err != nil {
		return contracts.TranscodeSettings{}, &core.Error{Code: "invalid-message", Msg: err.Error()}
	}
	if err := p.DB.Update(func(tx *bolt.Tx) error {
		return kv.PutJSON(tx, kv.BMeta, []byte(transcodeSettingsKey), in)
	}); err != nil {
		return contracts.TranscodeSettings{}, err
	}
	return in, nil
}

// ensure writes base only when no readable save exists; a corrupt
// document is overwritten the way the file-backed store behaved.
func (p Provider) ensure(base contracts.TranscodeSettings) (HasOutput, error) {
	saved, err := p.has()
	if err == nil && saved {
		return HasOutput{Saved: true}, nil
	}
	_, err = p.put(base)
	if err != nil {
		return HasOutput{}, err
	}
	return HasOutput{Saved: false}, nil
}
