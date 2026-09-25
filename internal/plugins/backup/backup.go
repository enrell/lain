// Package backup serves lain.backup.create@1 (D-076): the trusted core
// hands a consistent snapshot plus data-directory documents, and the
// provider owns the artifact format. The built-in is a passthrough —
// the raw database image, same as the endpoint always served — while a
// replacement can package, encrypt or ship it elsewhere.
package backup

import (
	"encoding/json"
	"os"
	"path/filepath"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

const ID = "lain-backup-bundle"

// Provider packages backup artifacts.
type Provider struct{}

func (Provider) ID() string             { return ID }
func (Provider) Capabilities() []string { return []string{contracts.CapBackupCreate} }
func (Provider) Health() error          { return nil }

// CreateInput is the trusted-core handoff (D-076): a completed,
// consistent snapshot of the live database — never the locked db file
// itself — plus the data-directory JSON documents a bundle may want
// (composition and friends).
type CreateInput struct {
	SnapshotPath string                     `json:"snapshot_path"`
	OutDir       string                     `json:"out_dir"`
	Docs         map[string]json.RawMessage `json:"docs,omitempty"`
}

// CreateOutput names the produced artifact. Path is what the gateway
// streams; Filename is what the download should be called.
type CreateOutput struct {
	Path     string `json:"path"`
	Filename string `json:"filename"`
}

func (Provider) Invoke(cap string, input any) (any, error) {
	if cap != contracts.CapBackupCreate {
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
	in, ok := input.(CreateInput)
	if !ok {
		return nil, &core.Error{Code: "invalid-message", Msg: "backup.CreateInput required"}
	}
	fi, err := os.Stat(in.SnapshotPath)
	if err != nil || fi.IsDir() || fi.Size() == 0 {
		return nil, &core.Error{Code: "dependency-unavailable", Msg: "snapshot missing or empty"}
	}
	return CreateOutput{Path: in.SnapshotPath, Filename: filepath.Base(in.SnapshotPath)}, nil
}
