// Package contract is the shared harness that verifies the
// core.Provider boundary against every plugin: an honest capability
// list, typed *core.Error failures and no panics — whatever the input.
// docs/CONTRACTS.md is the prose form; Run is the executable form.
package contract

import (
	"strings"
	"testing"

	"github.com/enrell/lain/internal/core"
)

// Cap pairs one declared capability with one valid Sample input for the
// happy-path check. Leave Sample nil to skip it — providers whose real
// work needs the network or ffmpeg keep those calls in their own tests.
// NilOK marks the rare capability where nil is itself a valid input
// (e.g. catalog read lists on nil).
type Cap struct {
	Name   string
	Sample any
	NilOK  bool
}

// Run checks the core.Provider contract against p.
func Run(t *testing.T, p core.Provider, caps []Cap) {
	t.Helper()
	checkIdentity(t, p, caps)
	checkUnknownCapability(t, p)
	checkGarbageInputs(t, p, caps)
	checkSamples(t, p, caps)
	checkHealth(t, p)
}

// invoke calls Invoke and turns a panic into a test failure instead of
// a suite crash, so one bad provider cannot hide the others' verdicts.
func invoke(t *testing.T, p core.Provider, cap string, input any) (out any, err error) {
	t.Helper()
	defer func() {
		if r := recover(); r != nil {
			t.Fatalf("%s: Invoke(%q, %T) panicked: %v", p.ID(), cap, input, r)
		}
	}()
	return p.Invoke(cap, input)
}

func assertTyped(t *testing.T, p core.Provider, cap string, err error) {
	t.Helper()
	ce, ok := err.(*core.Error)
	if !ok {
		t.Fatalf("%s: Invoke(%q) returned untyped error %T: %v", p.ID(), cap, err, err)
	}
	if ce.Code == "" {
		t.Fatalf("%s: Invoke(%q) returned *core.Error with empty code", p.ID(), cap)
	}
}

func checkIdentity(t *testing.T, p core.Provider, caps []Cap) {
	t.Helper()
	if strings.TrimSpace(p.ID()) != p.ID() || p.ID() == "" {
		t.Fatalf("provider ID %q is empty or padded", p.ID())
	}
	declared := p.Capabilities()
	if len(declared) == 0 {
		t.Fatalf("%s declares no capabilities", p.ID())
	}
	seen := map[string]bool{}
	for _, c := range declared {
		if strings.TrimSpace(c) != c || c == "" {
			t.Fatalf("%s declares an empty or padded capability %q", p.ID(), c)
		}
		if seen[c] {
			t.Fatalf("%s declares %q twice", p.ID(), c)
		}
		seen[c] = true
	}
	exercised := map[string]bool{}
	for _, c := range caps {
		exercised[c.Name] = true
	}
	for _, c := range declared {
		if !exercised[c] {
			t.Fatalf("%s declares %q but the contract test does not exercise it", p.ID(), c)
		}
	}
	for _, c := range caps {
		if !seen[c.Name] {
			t.Fatalf("%s: contract test exercises %q which Capabilities() does not declare", p.ID(), c.Name)
		}
	}
}

func checkUnknownCapability(t *testing.T, p core.Provider) {
	t.Helper()
	_, err := invoke(t, p, "lain.no.such-capability@99", nil)
	if err == nil {
		t.Fatalf("%s accepted an unknown capability", p.ID())
	}
	assertTyped(t, p, "lain.no.such-capability@99", err)
}

// garbage is deliberately wrong-typed input. None of these types is a
// valid Invoke payload for any capability in the tree.
var garbage = []any{
	"just a string",
	42,
	3.14,
	true,
	[]byte("bytes"),
	[]string{"a", "b"},
	map[string]any{"x": 1},
	map[string]string{"k": "v"},
	struct{ X int }{X: 1},
	[]any{nil, "x"},
	func() {},
}

func checkGarbageInputs(t *testing.T, p core.Provider, caps []Cap) {
	t.Helper()
	for _, c := range caps {
		inputs := garbage
		if !c.NilOK {
			inputs = append([]any{nil}, inputs...)
		}
		for _, g := range inputs {
			_, err := invoke(t, p, c.Name, g)
			if err == nil {
				t.Fatalf("%s: Invoke(%q, %T) accepted garbage input", p.ID(), c.Name, g)
			}
			if strings.Contains(err.Error(), "unsupported cap") {
				t.Fatalf("%s: declared capability %q reaches the unsupported branch", p.ID(), c.Name)
			}
			assertTyped(t, p, c.Name, err)
		}
	}
}

func checkSamples(t *testing.T, p core.Provider, caps []Cap) {
	t.Helper()
	for _, c := range caps {
		if c.Sample == nil {
			continue
		}
		_, err := invoke(t, p, c.Name, c.Sample)
		// Operational failures (db, io, network) may surface untyped —
		// the typed-error contract covers protocol violations only.
		if err != nil && strings.Contains(err.Error(), "unsupported cap") {
			t.Fatalf("%s: valid input on %q reaches the unsupported branch", p.ID(), c.Name)
		}
	}
}

func checkHealth(t *testing.T, p core.Provider) {
	t.Helper()
	defer func() {
		if r := recover(); r != nil {
			t.Fatalf("%s: Health() panicked: %v", p.ID(), r)
		}
	}()
	_ = p.Health()
}
