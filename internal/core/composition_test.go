package core

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-22

import "testing"

// Saved v1 compositions gain the TVMaze provider on boot without
// losing user overrides (e.g. a deliberately withdrawn AniList).
func TestUpgradeV1ToV2AppendsTVMaze(t *testing.T) {
	saved := &Composition{
		Version: 1,
		Bindings: map[string]*Binding{
			"lain.metadata.search@1":  {Mode: ModeMergeMany, Providers: []string{"lain-metadata-nfo", "lain-metadata-kitsu", "lain-metadata-jikan"}, Generation: 1},
			"lain.metadata.resolve@1": {Mode: ModeMergeMany, Providers: []string{"lain-metadata-nfo", "lain-metadata-kitsu", "lain-metadata-jikan"}, Generation: 1},
		},
	}
	saved.Upgrade(DefaultComposition())
	if saved.Version != DefaultComposition().Version {
		t.Fatalf("version=%d, want current %d", saved.Version, DefaultComposition().Version)
	}
	for _, cap := range []string{"lain.metadata.search@1", "lain.metadata.resolve@1"} {
		provs := saved.Bindings[cap].Providers
		if len(provs) != 4 || provs[3] != "lain-metadata-tvmaze" {
			t.Fatalf("%s providers=%v, want tvmaze appended last", cap, provs)
		}
		for _, id := range provs {
			if id == "lain-metadata-anilist" {
				t.Fatalf("%s must not resurrect a withdrawn provider: %v", cap, provs)
			}
		}
	}
	// Second boot is a no-op: no duplicates, generation stable.
	gen := saved.Bindings["lain.metadata.search@1"].Generation
	saved.Upgrade(DefaultComposition())
	if provs := saved.Bindings["lain.metadata.search@1"].Providers; len(provs) != 4 {
		t.Fatalf("re-upgrade must not duplicate: %v", provs)
	}
	if got := saved.Bindings["lain.metadata.search@1"].Generation; got != gen {
		t.Fatalf("re-upgrade bumped generation %d->%d", gen, got)
	}
}

func TestUpgradeV2AddsAsyncTranscodeWithoutReplacingV1(t *testing.T) {
	saved := &Composition{
		Version: 2,
		Bindings: map[string]*Binding{
			"lain.playback.transcode@1": {Mode: ModeExactlyOne, Providers: []string{"custom-transcoder"}, Generation: 7},
		},
	}
	added := saved.Upgrade(DefaultComposition())
	if saved.Version != DefaultComposition().Version {
		t.Fatalf("version=%d, want current %d", saved.Version, DefaultComposition().Version)
	}
	if got := saved.Bindings["lain.playback.transcode@1"]; got.Providers[0] != "custom-transcoder" || got.Generation != 7 {
		t.Fatalf("v1 override replaced: %+v", got)
	}
	if got := saved.Bindings["lain.playback.transcode@2"]; got == nil || got.Providers[0] != "lain-transcode-ffmpeg" {
		t.Fatalf("v2 binding missing: %+v (added %v)", got, added)
	}
	if got := saved.Bindings["lain.playback.transcode@3"]; got == nil || got.Providers[0] != "lain-transcode-ffmpeg" {
		t.Fatalf("v3 binding missing: %+v (added %v)", got, added)
	}
}

func TestDefaultCompositionValidates(t *testing.T) {
	fresh := DefaultComposition()
	known := map[string]bool{}
	for _, b := range fresh.Bindings {
		for _, id := range b.Providers {
			known[id] = true
		}
	}
	// Every bound id is known by construction; the registry test
	// covers unknown-provider rejection.
	if err := fresh.Validate(known); err != nil {
		t.Fatalf("defaults must validate: %v", err)
	}
}
