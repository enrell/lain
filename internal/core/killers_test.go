package core

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-22

import (
	"errors"
	"strings"
	"testing"
)

func TestSwapValidationPaths(t *testing.T) {
	r := testReg()
	if _, err := r.Swap("cap.unknown@1", []string{"good"}, 0); err == nil || !strings.Contains(err.Error(), "unknown capability cap.unknown@1") {
		t.Fatalf("unknown capability must fail with its name: %v", err)
	}
	if _, err := r.Swap("cap.a@1", []string{"good", "extra"}, 0); err == nil || !strings.Contains(err.Error(), "exactly-one needs 1") {
		t.Fatalf("exactly-one with 2 providers must fail: %v", err)
	}
	if _, err := r.Swap("cap.a@1", []string{}, 0); err == nil {
		t.Fatal("exactly-one with 0 providers must fail")
	}
	if _, err := r.Swap("cap.a@1", []string{"ghost"}, 0); err == nil || !strings.Contains(err.Error(), "unknown provider ghost") {
		t.Fatalf("unregistered provider must fail: %v", err)
	}
	// Registered but not serving the capability.
	r.Register(&fake{id: "other", caps: []string{"cap.z@1"}})
	if _, err := r.Swap("cap.a@1", []string{"other"}, 0); err == nil || !strings.Contains(err.Error(), "does not serve cap.a@1") {
		t.Fatalf("non-serving provider must fail: %v", err)
	}
	// Generation fencing: matching expected gen passes, wrong fails.
	r.Register(&fake{id: "v2", caps: []string{"cap.a@1"}, out: "v2"})
	if _, err := r.Swap("cap.a@1", []string{"v2"}, 1); err != nil {
		t.Fatalf("correct expected gen must pass: %v", err)
	}
	if _, err := r.Swap("cap.a@1", []string{"good"}, 1); err == nil {
		t.Fatal("stale gen must fail")
	} else if !strings.Contains(err.Error(), "expected generation 1, active is 2") {
		t.Fatalf("stale message must carry both gens: %v", err)
	}
	// expectedGen 0 skips fencing entirely.
	if _, err := r.Swap("cap.a@1", []string{"good"}, 0); err != nil {
		t.Fatalf("gen 0 must skip fencing: %v", err)
	}
}

func TestWithdrawUnknown(t *testing.T) {
	r := testReg()
	if err := r.Withdraw("ghost"); err == nil || !strings.Contains(err.Error(), "unknown provider ghost") {
		t.Fatalf("withdraw unknown: %v", err)
	}
	// Withdrawing the only provider of an exactly-one binding degrades it:
	// the binding keeps the provider (fail closed) and logs degraded.
	if err := r.Withdraw("good"); err != nil {
		t.Fatal(err)
	}
	provs, _, err := r.Ordered("cap.a@1")
	if err != nil || len(provs) != 1 || provs[0].ID() != "good" {
		t.Fatalf("degraded binding keeps last provider: %+v %v", provs, err)
	}
	if evs := r.Events(); len(evs) == 0 || evs[len(evs)-1].Kind != "withdraw-degraded" {
		t.Fatalf("want withdraw-degraded event: %+v", evs)
	}
}

func TestOrderedAndUnknownCaps(t *testing.T) {
	r := testReg()
	if _, _, err := r.Ordered("cap.nope@1"); err == nil || !strings.Contains(err.Error(), "unknown capability cap.nope@1") {
		t.Fatalf("ordered unknown cap: %v", err)
	}
	if _, _, err := r.CallMerge("cap.nope@1", nil, nil); err == nil {
		t.Fatal("merge on unknown cap must fail")
	}
	if _, _, _, err := r.CallFirst("cap.nope@1", nil, nil); err == nil {
		t.Fatal("callfirst on unknown cap must fail")
	}
	if _, _, err := r.CallOne("cap.nope@1", nil); err == nil {
		t.Fatal("callone on unknown cap must fail")
	}
}

func TestCallFirstOutcomes(t *testing.T) {
	r := testReg()
	// First rejects, second accepts: skip-and-continue.
	out, id, ok, err := r.CallFirst("cap.b@1", nil, func(v any) bool { return v == "second" })
	if err != nil || !ok || id != "second" || out != "second" {
		t.Fatalf("callfirst skip: %v %s %v %v", out, id, ok, err)
	}
	// All reject: not accepted, no error.
	out, id, ok, err = r.CallFirst("cap.b@1", nil, func(any) bool { return false })
	if ok || err != nil || out != nil || id != "" {
		t.Fatalf("all-reject must be (nil, '', false, nil): %v %s %v %v", out, id, ok, err)
	}
	// All fail: last error surfaces.
	r2 := NewRegistry(&Composition{Version: 1, Bindings: map[string]*Binding{
		"cap.f@1": {Mode: ModeOrderedMany, Providers: []string{"a", "b"}, Generation: 1},
	}})
	errA, errB := errors.New("err-a"), errors.New("err-b")
	r2.Register(&fake{id: "a", caps: []string{"cap.f@1"}, fail: errA})
	r2.Register(&fake{id: "b", caps: []string{"cap.f@1"}, fail: errB})
	_, _, ok, err = r2.CallFirst("cap.f@1", nil, func(any) bool { return true })
	if ok || err != errB {
		t.Fatalf("last error must surface: %v", err)
	}
}

func TestCallMergeReportAllFailMessage(t *testing.T) {
	r := NewRegistry(&Composition{Version: 1, Bindings: map[string]*Binding{
		"cap.m@1": {Mode: ModeMergeMany, Providers: []string{"x"}, Generation: 1},
	}})
	r.Register(&fake{id: "x", caps: []string{"cap.m@1"}, fail: errTestFail})
	_, _, failed, err := r.CallMergeReport("cap.m@1", nil, nil)
	if err == nil || !strings.Contains(err.Error(), "all providers failed for cap.m@1") {
		t.Fatalf("all-fail message: %v", err)
	}
	if len(failed) != 1 || failed[0].Provider != "x" {
		t.Fatalf("failed list: %+v", failed)
	}
}

func TestInvokeProviderGuards(t *testing.T) {
	r := testReg()
	if _, err := r.InvokeProvider("cap.nope@1", "good", nil); err == nil || !strings.Contains(err.Error(), "unknown capability cap.nope@1") {
		t.Fatalf("unknown cap: %v", err)
	}
	// Registered but not bound to this capability.
	r.Register(&fake{id: "free", caps: []string{"cap.a@1"}})
	if _, err := r.InvokeProvider("cap.a@1", "free", nil); err == nil || !strings.Contains(err.Error(), "not bound to cap.a@1") {
		t.Fatalf("unbound-but-registered: %v", err)
	}
	// Bound but unhealthy.
	r2 := NewRegistry(&Composition{Version: 1, Bindings: map[string]*Binding{
		"cap.a@1": {Mode: ModeExactlyOne, Providers: []string{"sick"}, Generation: 1},
	}})
	r2.Register(&fake{id: "sick", caps: []string{"cap.a@1"}, health: errTestFail})
	if _, err := r2.InvokeProvider("cap.a@1", "sick", nil); err == nil || !strings.Contains(err.Error(), "unhealthy") {
		t.Fatalf("unhealthy bound provider: %v", err)
	}
	// Bound and healthy, but the provider does not declare the cap.
	r3 := NewRegistry(&Composition{Version: 1, Bindings: map[string]*Binding{
		"cap.a@1": {Mode: ModeExactlyOne, Providers: []string{"misc"}, Generation: 1},
	}})
	r3.Register(&fake{id: "misc", caps: []string{"cap.b@1"}})
	if _, err := r3.InvokeProvider("cap.a@1", "misc", nil); err == nil || !strings.Contains(err.Error(), "does not serve cap.a@1") {
		t.Fatalf("bound provider not serving cap: %v", err)
	}
}

func TestProvidersSortedAndHealthy(t *testing.T) {
	r := testReg()
	r.Register(&fake{id: "zeta", caps: []string{"cap.a@1"}, health: errTestFail})
	ids := r.Providers()
	for i, id := range ids {
		if i > 0 && id <= ids[i-1] {
			t.Fatalf("providers not sorted: %+v", ids)
		}
	}
	if len(ids) != 4 {
		t.Fatalf("want 4 providers, got %+v", ids)
	}
	infos := r.ProviderInfos()
	for i, p := range infos {
		if i > 0 && p.ID <= infos[i-1].ID {
			t.Fatalf("provider infos not sorted: %+v", infos)
		}
		if p.ID == "zeta" && p.Healthy {
			t.Fatal("unhealthy provider must report Healthy=false")
		}
	}
}

func TestEventLogRingBuffer(t *testing.T) {
	r := testReg()
	for i := 0; i < 260; i++ {
		r.logLocked(Event{Kind: "tick", Detail: string(rune('a' + i%26))})
	}
	evs := r.Events()
	if len(evs) != 256 {
		t.Fatalf("log must cap at 256, got %d", len(evs))
	}
	if evs[255].Detail != string(rune('a'+259%26)) {
		t.Fatalf("newest event must be last: %+v", evs[255])
	}
}

func TestErrorFormat(t *testing.T) {
	e := &Error{Code: "stale-generation", Msg: "nope"}
	if e.Error() != "stale-generation: nope" {
		t.Fatalf("Error() = %q", e.Error())
	}
}

func TestMigrateProviderIDs(t *testing.T) {
	c := &Composition{Version: 1, Bindings: map[string]*Binding{
		"cap.x@1": {Mode: ModeExactlyOne, Providers: []string{"lain-catalog-file", "lain-userstate-file", "other"}, Generation: 1},
	}}
	moved := c.MigrateProviderIDs()
	if len(moved) != 2 || moved[0] != "lain-catalog-file->lain-catalog-bolt" {
		t.Fatalf("moved=%v", moved)
	}
	provs := c.Bindings["cap.x@1"].Providers
	if provs[0] != "lain-catalog-bolt" || provs[1] != "lain-userstate-bolt" || provs[2] != "other" {
		t.Fatalf("rewrite: %v", provs)
	}
	if len(c.MigrateProviderIDs()) != 0 {
		t.Fatal("second migrate must be a no-op")
	}
}

func TestUpgradeVersionGates(t *testing.T) {
	// v2 -> v3 must NOT re-add tvmaze (user may have withdrawn it).
	saved := &Composition{Version: 2, Bindings: map[string]*Binding{
		"lain.metadata.search@1": {Mode: ModeMergeMany, Providers: []string{"lain-metadata-nfo"}, Generation: 5},
	}}
	saved.Upgrade(&Composition{Version: 3, Bindings: map[string]*Binding{
		"lain.metadata.search@1": {Mode: ModeMergeMany, Providers: []string{"lain-metadata-nfo", "lain-metadata-tvmaze"}, Generation: 1},
	}})
	if provs := saved.Bindings["lain.metadata.search@1"].Providers; len(provs) != 1 {
		t.Fatalf("v2->v3 must not append tvmaze: %v", provs)
	}
	// v1 -> fresh v2 (exactly 2): tvmaze does get appended, gen bumps once.
	saved = &Composition{Version: 1, Bindings: map[string]*Binding{
		"lain.metadata.search@1":  {Mode: ModeMergeMany, Providers: []string{"lain-metadata-nfo"}, Generation: 5},
		"lain.metadata.resolve@1": {Mode: ModeMergeMany, Providers: []string{"lain-metadata-nfo"}, Generation: 7},
	}}
	saved.Upgrade(&Composition{Version: 2, Bindings: map[string]*Binding{
		"lain.metadata.search@1":  {Mode: ModeMergeMany, Providers: []string{"a"}, Generation: 1},
		"lain.metadata.resolve@1": {Mode: ModeMergeMany, Providers: []string{"a"}, Generation: 1},
	}})
	if provs := saved.Bindings["lain.metadata.search@1"].Providers; len(provs) != 2 || provs[1] != "lain-metadata-tvmaze" {
		t.Fatalf("v1->v2 must append tvmaze: %v", provs)
	}
	if g := saved.Bindings["lain.metadata.search@1"].Generation; g != 6 {
		t.Fatalf("generation must bump by 1, got %d", g)
	}
	if saved.Version != 2 {
		t.Fatalf("version must adopt fresh: %d", saved.Version)
	}
	// tvmaze already present: no duplicate, no bump.
	saved2 := &Composition{Version: 1, Bindings: map[string]*Binding{
		"lain.metadata.search@1": {Mode: ModeMergeMany, Providers: []string{"lain-metadata-tvmaze"}, Generation: 9},
	}}
	saved2.Upgrade(&Composition{Version: 2, Bindings: map[string]*Binding{
		"lain.metadata.search@1": {Mode: ModeMergeMany, Providers: []string{"a"}, Generation: 1},
	}})
	if provs := saved2.Bindings["lain.metadata.search@1"].Providers; len(provs) != 1 {
		t.Fatalf("existing tvmaze must not duplicate: %v", provs)
	}
	if g := saved2.Bindings["lain.metadata.search@1"].Generation; g != 9 {
		t.Fatalf("no-op must not bump generation: %d", g)
	}
}

func TestCompositionViewSorted(t *testing.T) {
	c := &Composition{Version: 1, Bindings: map[string]*Binding{
		"cap.b@1": {Mode: ModeExactlyOne, Providers: []string{"b"}, Generation: 2},
		"cap.a@1": {Mode: ModeMergeMany, Providers: []string{"a"}, Generation: 1},
	}}
	v := c.View()
	if len(v) != 2 || v[0].Capability != "cap.a@1" || v[1].Capability != "cap.b@1" {
		t.Fatalf("view must sort by capability: %+v", v)
	}
	if v[0].Mode != "merge-many" || v[0].Generation != 1 || v[0].Providers[0] != "a" {
		t.Fatalf("view fields: %+v", v[0])
	}
}
