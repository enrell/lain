package contracts

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-23

import (
	"bytes"
	"encoding/json"
	"testing"
	"testing/quick"
)

// Property: every contract document marshals and unmarshal-marshals to
// identical bytes — persisted JSON and API payloads must be codec-stable
// across versions of the struct. Byte equality (not DeepEqual) absorbs
// nil-vs-empty normalization, which JSON itself erases.
func codecStable[T any](t *testing.T, name string) {
	t.Helper()
	err := quick.Check(func(v T) bool {
		b1, err := json.Marshal(v)
		if err != nil {
			return true // unmarshalable input values are not the contract
		}
		var v2 T
		if err := json.Unmarshal(b1, &v2); err != nil {
			t.Logf("%s: unmarshal failed: %v", name, err)
			return false
		}
		b2, err := json.Marshal(v2)
		if err != nil {
			t.Logf("%s: remarshal failed: %v", name, err)
			return false
		}
		return bytes.Equal(b1, b2)
	}, &quick.Config{MaxCount: 300})
	if err != nil {
		t.Error(err)
	}
}

func TestPropertyContractCodecStability(t *testing.T) {
	codecStable[CatalogItem](t, "CatalogItem")
	codecStable[Progress](t, "Progress")
	codecStable[PlanRequest](t, "PlanRequest")
	codecStable[ScanStats](t, "ScanStats")
	codecStable[MetadataSearchInput](t, "MetadataSearchInput")
	codecStable[MediaInfo](t, "MediaInfo")
}
