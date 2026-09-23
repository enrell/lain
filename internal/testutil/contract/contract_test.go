package contract

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-23

// Self-test for the shared harness: a well-behaved fake provider must
// pass cleanly through every check. The negative direction (a bad
// provider being caught) cannot be asserted in-process — Run reports
// through t.Fatalf and subtest failures propagate to the parent — so
// coverage here comes from the happy path exercising every check
// against a conforming provider.

import (
	"testing"

	"github.com/enrell/lain/internal/core"
)

type fakeInput struct{ v int }

type fakeProvider struct{}

func (fakeProvider) ID() string             { return "lain.fake@1" }
func (fakeProvider) Capabilities() []string { return []string{"lain.fake.cap@1"} }
func (fakeProvider) Health() error          { return nil }
func (fakeProvider) Invoke(cap string, input any) (any, error) {
	if cap != "lain.fake.cap@1" {
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
	in, ok := input.(fakeInput)
	if !ok {
		return nil, &core.Error{Code: "invalid-message", Msg: "fakeInput required"}
	}
	return in.v, nil
}

type countingProvider struct {
	fakeProvider
	calls map[string]int
}

func (p countingProvider) Invoke(cap string, input any) (any, error) {
	if _, ok := input.(fakeInput); ok {
		p.calls[cap]++
	}
	return p.fakeProvider.Invoke(cap, input)
}

func TestRunAcceptsGoodProvider(t *testing.T) {
	Run(t, fakeProvider{}, []Cap{
		{Name: "lain.fake.cap@1", Sample: fakeInput{v: 7}},
	})
}

func TestRunInvokesEverySample(t *testing.T) {
	p := countingProvider{calls: map[string]int{}}
	Run(t, p, []Cap{
		{Name: "lain.fake.cap@1", Sample: fakeInput{v: 7}},
	})
	if p.calls["lain.fake.cap@1"] == 0 {
		t.Fatal("declared capability was never invoked — sample checks skipped")
	}
}
