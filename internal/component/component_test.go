package component

import (
	"os"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/matrix"
)

// stubProvider is the component the test helper process serves.
type stubProvider struct{ healthy bool }

func (s stubProvider) ID() string             { return "test-stub" }
func (s stubProvider) Capabilities() []string { return []string{contracts.CapMediaProbe} }
func (s stubProvider) Health() error {
	if !s.healthy {
		return &core.Error{Code: "dependency-unavailable", Msg: "stub down"}
	}
	return nil
}
func (s stubProvider) Invoke(cap string, input any) (any, error) {
	in, ok := input.(contracts.MediaProbeRequest)
	if cap != contracts.CapMediaProbe || !ok {
		return nil, &core.Error{Code: "invalid-message", Msg: "bad call"}
	}
	if in.FilePath == "/fail" {
		return nil, &core.Error{Code: "not-found", Msg: "no such file"}
	}
	return contracts.MediaInfo{Duration: 42}, nil
}

// TestHelperProcess re-execs the test binary as a component child: the
// manifest's entrypoint is os.Args[0] and this "test" becomes the
// serving side when the component-serve positional arg is in argv.
// Positional args (no dashes) end flag parsing, so the test binary
// accepts them where a --flag would fail "not defined".
func TestHelperProcess(t *testing.T) {
	t.Helper()
	for i, a := range os.Args {
		if a == "component-serve" && i+1 < len(os.Args) {
			if err := Serve(os.Args[i+1], stubProvider{healthy: true}); err != nil {
				os.Exit(1)
			}
			os.Exit(0)
		}
	}
}

func stubManifest(t *testing.T) matrix.Manifest {
	t.Helper()
	var m matrix.Manifest
	m.ID = "test-stub"
	m.Capabilities = []string{contracts.CapMediaProbe}
	m.Execution.Kind = "process"
	m.Execution.Entrypoint = os.Args[0]
	// Positional args after the flags: the test binary's flag parser
	// stops at the first non-flag, so component-serve survives as argv.
	m.Execution.Args = []string{"-test.run=^TestHelperProcess$", "component-serve", "{sock}"}
	return m
}

func waitHealthy(t *testing.T, p *Process) {
	t.Helper()
	deadline := time.Now().Add(15 * time.Second)
	for time.Now().Before(deadline) {
		if p.Health() == nil {
			return
		}
		time.Sleep(50 * time.Millisecond)
	}
	t.Fatalf("component never became healthy: %v", p.Health())
}

func TestProcessInvokeRoundTrip(t *testing.T) {
	p, err := Spawn(stubManifest(t), t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer p.Close()
	waitHealthy(t, p)

	out, err := p.Invoke(contracts.CapMediaProbe, contracts.MediaProbeRequest{FilePath: "/media/x.mkv"})
	if err != nil {
		t.Fatalf("invoke: %v", err)
	}
	info, ok := out.(contracts.MediaInfo)
	if !ok || info.Duration != 42 {
		t.Fatalf("bad output: %#v", out)
	}

	// Typed errors keep their code across the wire.
	if _, err := p.Invoke(contracts.CapMediaProbe, contracts.MediaProbeRequest{FilePath: "/fail"}); err == nil {
		t.Fatal("expected error")
	} else if ce, ok := err.(*core.Error); !ok || ce.Code != "not-found" {
		t.Fatalf("want not-found, got %v", err)
	}

	// Unknown capability/op is invalid-message before any spawn work.
	if _, err := p.Invoke("lain.nope@1", nil); err == nil {
		t.Fatal("unknown cap must fail")
	}
}

func TestProcessHealthAfterRespawn(t *testing.T) {
	p, err := Spawn(stubManifest(t), t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer p.Close()
	waitHealthy(t, p)

	// Kill the child; the supervisor must respawn it on the backoff
	// ladder and Health recovers without re-registration.
	p.mu.Lock()
	p.killLocked()
	p.mu.Unlock()
	waitHealthy(t, p)
	out, err := p.Invoke(contracts.CapMediaProbe, contracts.MediaProbeRequest{FilePath: "/x"})
	if err != nil || out.(contracts.MediaInfo).Duration != 42 {
		t.Fatalf("invoke after respawn: %v %#v", err, out)
	}
}

func TestProcessUnspawnable(t *testing.T) {
	m := stubManifest(t)
	m.Execution.Entrypoint = "/nonexistent/lain-plugin-binary"
	p, err := Spawn(m, t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer p.Close()
	// Never healthy, never crashes the host — Health reports down.
	time.Sleep(300 * time.Millisecond)
	if err := p.Health(); err == nil {
		t.Fatal("unspawnable component must report unhealthy")
	} else if ce, ok := err.(*core.Error); !ok || ce.Code != "dependency-unavailable" {
		t.Fatalf("want dependency-unavailable, got %v", err)
	}
	if _, err := p.Invoke(contracts.CapMediaProbe, contracts.MediaProbeRequest{}); err == nil {
		t.Fatal("invoke on down component must fail")
	}
}

func TestProvisionerInstallRemove(t *testing.T) {
	reg := core.NewRegistry(core.DefaultComposition())
	dir := t.TempDir()
	prov := NewProvisioner(dir, reg, nil)
	if err := prov.Start(); err != nil {
		t.Fatal(err)
	}
	defer prov.Close()

	manifest := []byte(`{"id":"test-stub","version":"1","capabilities":["lain.media.probe@1"],` +
		`"execution":{"kind":"process","entrypoint":"` + os.Args[0] +
		`","args":["-test.run=^TestHelperProcess$","component-serve","{sock}"]}}`)
	if err := os.WriteFile(dir+"/test-stub.json", manifest, 0o644); err != nil {
		t.Fatal(err)
	}
	deadline := time.Now().Add(10 * time.Second)
	for time.Now().Before(deadline) {
		for _, id := range reg.Providers() {
			if id == "test-stub" {
				goto installed
			}
		}
		time.Sleep(100 * time.Millisecond)
	}
	t.Fatal("manifest not provisioned")
installed:
	// Removing the manifest unregisters the provider and kills the child.
	if err := os.Remove(dir + "/test-stub.json"); err != nil {
		t.Fatal(err)
	}
	deadline = time.Now().Add(10 * time.Second)
	for time.Now().Before(deadline) {
		gone := true
		for _, id := range reg.Providers() {
			if id == "test-stub" {
				gone = false
			}
		}
		if gone {
			return
		}
		time.Sleep(100 * time.Millisecond)
	}
	t.Fatal("provider still registered after manifest removal")
}
