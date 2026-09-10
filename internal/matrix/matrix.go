// Package matrix is the seam to the Matrix kernel. In v0.1 the Lain
// core runs embedded (no daemon required): this package exports the
// provider manifests in Matrix shape and diagnoses the operator
// environment, so provisioning the same composition under
// matrix-managed later is a translation, not a redesign.
package matrix

import (
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
)

// Manifest is the Matrix component manifest shape for one provider.
type Manifest struct {
	ID           string   `json:"id"`
	Version      string   `json:"version"`
	Capabilities []string `json:"capabilities"`
	Execution    struct {
		Kind       string   `json:"kind"`
		Entrypoint string   `json:"entrypoint"`
		Args       []string `json:"args"`
	} `json:"execution"`
}

// Entry describes one provisioned component.
type Entry struct {
	Manifest Manifest `json:"manifest"`
	Trusted  bool     `json:"trusted"`
}

// NewManifest builds a process-component manifest. entrypoint/args use
// the Matrix placeholders {sock} and {id} when the host fills them.
func NewManifest(id, version string, caps []string, entrypoint string, args []string) Manifest {
	m := Manifest{ID: id, Version: version, Capabilities: caps}
	m.Execution.Kind = "process"
	m.Execution.Entrypoint = entrypoint
	m.Execution.Args = args
	return m
}

// ExportManifests writes one <id>.json per provider for provisioning.
func ExportManifests(dir string, entries []Entry) error {
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return err
	}
	for _, e := range entries {
		raw, err := json.MarshalIndent(e.Manifest, "", "  ")
		if err != nil {
			return err
		}
		if err := os.WriteFile(filepath.Join(dir, e.Manifest.ID+".json"), append(raw, '\n'), 0o644); err != nil {
			return err
		}
	}
	return nil
}

// DoctorReport diagnoses the Matrix operator environment.
type DoctorReport struct {
	BinaryPresent bool   `json:"binary_present"`
	Binary        string `json:"binary"`
	HelpOK        bool   `json:"help_ok"`
	Note          string `json:"note"`
}

// Doctor checks whether matrix-managed is available. Lain never
// requires it in v0.1; the report tells the operator what is missing.
func Doctor(binary string) DoctorReport {
	r := DoctorReport{Binary: binary}
	fi, err := os.Stat(binary)
	if err != nil || fi.IsDir() {
		r.Note = "matrix-managed not found; embedded core is used"
		return r
	}
	if fi.Mode().Perm()&0o111 == 0 {
		r.Note = "matrix-managed not executable"
		return r
	}
	r.BinaryPresent = true
	cmd := exec.Command(binary, "fingerprint")
	if err := cmd.Run(); err != nil {
		// fingerprint without args fails usage-wise but proves exec;
		// any exec at all counts as present.
		r.HelpOK = true
		r.Note = "matrix-managed executes (usage error without args is expected)"
		return r
	}
	r.HelpOK = true
	r.Note = "matrix-managed available"
	_ = fmt.Sprint()
	return r
}
