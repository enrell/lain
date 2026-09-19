package contracts

import (
	"reflect"
	"testing"
)

// TestParseCapabilitiesIsAWhitelist pins the two jobs of the parser: it
// normalizes what a client sends, and it drops everything outside this
// server's vocabulary so a claim cannot widen the set the planner
// reasons about (D-058).
func TestParseCapabilitiesIsAWhitelist(t *testing.T) {
	got := ParseCapabilities([]string{" MKV , mkv/H264 ,bogus,avi,h264, mkv/h264,"})
	want := &ClientCapabilities{Tokens: []string{"mkv", "mkv/h264"}}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("got %+v, want %+v", got, want)
	}

	// Several query values are as valid as one comma-separated value:
	// the gateway hands the query slice over as it arrives.
	got = ParseCapabilities([]string{"mkv", "mkv/vp9,mkv/opus"})
	want = &ClientCapabilities{Tokens: []string{"mkv", "mkv/opus", "mkv/vp9"}}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("multi-value got %+v, want %+v", got, want)
	}
}

// TestParseCapabilitiesDistinguishesUnknownFromEmpty is the difference
// the planner turns into behaviour: nil keeps the conservative browser
// rules, an empty set claims the client can decode nothing.
func TestParseCapabilitiesDistinguishesUnknownFromEmpty(t *testing.T) {
	if got := ParseCapabilities(nil); got != nil {
		t.Fatalf("absent caps = %+v, want nil (unknown)", got)
	}
	got := ParseCapabilities([]string{""})
	if got == nil || len(got.Tokens) != 0 {
		t.Fatalf("empty caps = %+v, want a non-nil empty claim", got)
	}
	if got.Has("mkv") {
		t.Fatal("an empty claim must not report any token")
	}
	var absent *ClientCapabilities
	if absent.Has("mkv") {
		t.Fatal("a nil capability set must not report any token")
	}
}

func TestCapabilityVocabulary(t *testing.T) {
	if container, ok := DirectContainer(".MKV"); !ok || container != "mkv" {
		t.Fatalf("DirectContainer(.MKV) = %q,%v", container, ok)
	}
	if _, ok := DirectContainer("avi"); ok {
		t.Fatal("avi has no directly playable container: the gateway cannot label it")
	}
	if family, ok := CapabilityCodec("AVC1"); !ok || family != "h264" {
		t.Fatalf("CapabilityCodec(AVC1) = %q,%v", family, ok)
	}
	if _, ok := CapabilityCodec("pcm_s16le"); ok {
		t.Fatal("pcm_s16le has no browser decoder, so it must have no token")
	}
	for _, tok := range []string{"mkv", "mkv/h264", "mp4/av1", "ogg/vorbis", "flac/flac"} {
		if !KnownCapabilityToken(tok) {
			t.Fatalf("%q must be a known token", tok)
		}
	}
	for _, tok := range []string{"", "h264", "avi", "mkv/h264/x", "mkv/", "/h264", "mkv/avc1", "mkv/mpeg4"} {
		if KnownCapabilityToken(tok) {
			t.Fatalf("%q must not be a known token", tok)
		}
	}
}
