package core

import (
	"errors"
	"testing"
)

type fake struct {
	id     string
	caps   []string
	health error
	calls  int
	fail   error
	out    any
}

func (f *fake) ID() string             { return f.id }
func (f *fake) Capabilities() []string { return f.caps }
func (f *fake) Health() error          { return f.health }
func (f *fake) Invoke(cap string, input any) (any, error) {
	f.calls++
	if f.fail != nil {
		return nil, f.fail
	}
	return f.out, nil
}

func testReg() *Registry {
	comp := &Composition{Version: 1, Bindings: map[string]*Binding{
		"cap.a@1": {Mode: ModeExactlyOne, Providers: []string{"good"}, Generation: 1},
		"cap.b@1": {Mode: ModeOrderedMany, Providers: []string{"first", "second"}, Generation: 1},
	}}
	r := NewRegistry(comp)
	r.Register(&fake{id: "good", caps: []string{"cap.a@1"}, out: "ok"})
	r.Register(&fake{id: "first", caps: []string{"cap.b@1"}, out: "first"})
	r.Register(&fake{id: "second", caps: []string{"cap.b@1"}, out: "second"})
	return r
}

func TestSwapRejectsUnhealthyKeepsServing(t *testing.T) {
	r := testReg()
	r.Register(&fake{id: "broken", caps: []string{"cap.a@1"}, health: errors.New("boom")})
	if _, err := r.Swap("cap.a@1", []string{"broken"}, 0); err == nil {
		t.Fatal("swap to unhealthy provider accepted")
	}
	out, serving, err := r.CallOne("cap.a@1", nil)
	if err != nil || out != "ok" || serving != "good" {
		t.Fatalf("serving broken after rejected swap: out=%v serving=%v err=%v", out, serving, err)
	}
	if evs := r.Events(); len(evs) == 0 || evs[len(evs)-1].Kind != "swap-rejected" {
		t.Fatalf("missing swap-rejected event: %+v", evs)
	}
}

func TestSwapFencing(t *testing.T) {
	r := testReg()
	r.Register(&fake{id: "v2", caps: []string{"cap.a@1"}, out: "v2"})
	gen, err := r.Swap("cap.a@1", []string{"v2"}, 0)
	if err != nil || gen != 2 {
		t.Fatalf("swap: gen=%d err=%v", gen, err)
	}
	if _, err := r.Swap("cap.a@1", []string{"good"}, 1); err == nil {
		t.Fatal("stale generation accepted")
	} else if ce, ok := err.(*Error); !ok || ce.Code != "stale-generation" {
		t.Fatalf("want stale-generation, got %v", err)
	}
}

func TestCallOneFallsBackToLastGood(t *testing.T) {
	r := testReg()
	good := &fake{id: "v2", caps: []string{"cap.a@1"}, out: "v2"}
	r.Register(good)
	if _, err := r.Swap("cap.a@1", []string{"v2"}, 0); err != nil {
		t.Fatal(err)
	}
	good.fail = errors.New("disk gone")
	out, serving, err := r.CallOne("cap.a@1", nil)
	if err != nil || out != "ok" || serving != "good" {
		t.Fatalf("no fallback to last-good: out=%v serving=%v err=%v", out, serving, err)
	}
}

func TestWithdrawFallsBack(t *testing.T) {
	r := testReg()
	if err := r.Withdraw("first"); err != nil {
		t.Fatal(err)
	}
	provs, gen, err := r.Ordered("cap.b@1")
	if err != nil || len(provs) != 1 || provs[0].ID() != "second" {
		t.Fatalf("ordered after withdraw: %+v gen=%d err=%v", provs, gen, err)
	}
	if gen != 2 {
		t.Fatalf("withdraw did not bump generation: %d", gen)
	}
}
