package playback

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-22

import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
)

func planFor(client, path string) contracts.Plan {
	return Plan(PlanInput{
		Request:  contracts.PlanRequest{ItemID: "item-1", Client: client},
		FilePath: path,
	})
}

func TestPlanDirectShapes(t *testing.T) {
	for _, tc := range []struct{ client, path string }{
		{"mpv", "/v/show.mkv"},
		{"lain-desktop", "/v/show.mkv"},
		{"web", "/v/movie.mp4"},
		{"web", "/v/clip.webm"},
	} {
		got := planFor(tc.client, tc.path)
		if got.Mode != "direct" || !got.Available {
			t.Fatalf("client=%s path=%s: got %+v, want direct+available", tc.client, tc.path, got)
		}
	}
}

func TestPlanBrowserMKVTranscodes(t *testing.T) {
	got := planFor("web", "/v/show.mkv")
	if got.Mode != "transcode" || !got.Available {
		t.Fatalf("got %+v, want transcode+available", got)
	}
	if got.Asset != "asset:item-1" {
		t.Fatalf("asset=%q leaks or misreferences", got.Asset)
	}
}

func TestPlanUsesProbedCodecCompatibility(t *testing.T) {
	base := PlanInput{
		Request:  contracts.PlanRequest{ItemID: "item-1", Client: "web"},
		FilePath: "/v/movie.mp4",
		MediaInfo: &contracts.MediaInfo{Format: "mov,mp4", Streams: []contracts.MediaStream{
			{Index: 0, Type: "video", Codec: "hevc", PixelFormat: "yuv420p"},
			{Index: 1, Type: "audio", Codec: "aac", Default: true},
		}},
	}
	if got := Plan(base); got.Mode != "transcode" {
		t.Fatalf("HEVC MP4 plan=%+v, want transcode", got)
	}
	base.MediaInfo.Streams[0].Codec = "h264"
	if got := Plan(base); got.Mode != "direct" {
		t.Fatalf("H.264/AAC MP4 plan=%+v, want direct", got)
	}
	base.MediaInfo.Streams[1].Codec = "mp3"
	if got := Plan(base); got.Mode != "direct" {
		t.Fatalf("H.264/MP3 MP4 plan=%+v, want direct", got)
	}
	base.MediaInfo.Streams[1].Codec = "ac3"
	if got := Plan(base); got.Mode != "transcode" {
		t.Fatalf("H.264/AC3 MP4 plan=%+v, want transcode", got)
	}
}

func TestPlanRejectsHDRForBrowserTranscode(t *testing.T) {
	got := Plan(PlanInput{
		Request:  contracts.PlanRequest{ItemID: "item-1", Client: "web"},
		FilePath: "/v/show.mkv",
		MediaInfo: &contracts.MediaInfo{Streams: []contracts.MediaStream{
			{Index: 0, Type: "video", Codec: "hevc", PixelFormat: "yuv420p10le", ColorTransfer: "smpte2084"},
		}},
	})
	if got.Mode != "transcode-required" || got.Available || got.Reason == "" {
		t.Fatalf("HDR plan=%+v, want explicit unavailable", got)
	}
}

// With no probed streams a capability claim is meaningless — an empty
// MediaInfo must never direct-play.
func TestPlanEmptyProbeNeverDirects(t *testing.T) {
	got := Plan(PlanInput{
		Request:   contracts.PlanRequest{ItemID: "i", Client: "web", Capabilities: capsOf("mkv,mkv/h264,mkv/aac")},
		FilePath:  "/v/show.mkv",
		MediaInfo: &contracts.MediaInfo{Format: "matroska", Streams: nil},
	})
	if got.Mode != "transcode" {
		t.Fatalf("empty probe + full claim = %+v, want transcode", got)
	}
	// Audio-only files still direct-play on browser-safe containers.
	for _, tc := range []struct {
		path  string
		want  string
		codec string
	}{
		{"/v/track.mp3", "direct", "mp3"},
		{"/v/track.ogg", "direct", "vorbis"},
		{"/v/track.mp4", "transcode", "aac"}, // mp4 with no video is not a video play
		{"/v/track.mkv", "transcode", "flac"},
	} {
		got := Plan(PlanInput{
			Request:  contracts.PlanRequest{ItemID: "i", Client: "web"},
			FilePath: tc.path,
			MediaInfo: &contracts.MediaInfo{Streams: []contracts.MediaStream{
				{Index: 0, Type: "audio", Codec: tc.codec, Default: true},
			}},
		})
		if got.Mode != tc.want {
			t.Fatalf("%s audio-only = %q, want %q", tc.path, got.Mode, tc.want)
		}
	}
}

// webm and mp4 each have their own web-safe track matrix; pixel format
// and audio codec both gate direct play.
func TestPlanBrowserContainerMatrix(t *testing.T) {
	vid := func(codec, pix string) contracts.MediaStream {
		return contracts.MediaStream{Index: 0, Type: "video", Codec: codec, PixelFormat: pix}
	}
	aud := func(codec string) contracts.MediaStream {
		return contracts.MediaStream{Index: 1, Type: "audio", Codec: codec, Default: true}
	}
	cases := []struct {
		name    string
		path    string
		streams []contracts.MediaStream
		want    string
	}{
		{"mp4 h264 yuvj420p", "/v/m.mp4", []contracts.MediaStream{vid("h264", "yuvj420p"), aud("aac")}, "direct"},
		{"mp4 h264 10-bit", "/v/m.mp4", []contracts.MediaStream{vid("h264", "yuv420p10le"), aud("aac")}, "transcode"},
		{"mp4 hevc", "/v/m.mp4", []contracts.MediaStream{vid("hevc", "yuv420p"), aud("aac")}, "transcode"},
		{"webm vp8 opus", "/v/m.webm", []contracts.MediaStream{vid("vp8", "yuv420p"), aud("opus")}, "direct"},
		{"webm vp9 vorbis", "/v/m.webm", []contracts.MediaStream{vid("vp9", "yuv420p"), aud("vorbis")}, "direct"},
		{"webm av1 no audio", "/v/m.webm", []contracts.MediaStream{vid("av1", "yuv420p")}, "direct"},
		{"webm vp9 ac3", "/v/m.webm", []contracts.MediaStream{vid("vp9", "yuv420p"), aud("ac3")}, "transcode"},
		{"webm h264", "/v/m.webm", []contracts.MediaStream{vid("h264", "yuv420p"), aud("opus")}, "transcode"},
		{"mp4 h264 no audio", "/v/m.mp4", []contracts.MediaStream{vid("h264", "yuv420p")}, "direct"},
	}
	for _, tc := range cases {
		got := Plan(PlanInput{
			Request:   contracts.PlanRequest{ItemID: "i", Client: "web"},
			FilePath:  tc.path,
			MediaInfo: &contracts.MediaInfo{Streams: tc.streams},
		})
		if got.Mode != tc.want {
			t.Fatalf("%s = %q, want %q", tc.name, got.Mode, tc.want)
		}
	}
}

// HDR + a working tone-map pipeline is transcode, not refusal.
func TestPlanHDRWithToneMapTranscodes(t *testing.T) {
	got := Plan(PlanInput{
		Request:  contracts.PlanRequest{ItemID: "i", Client: "web"},
		FilePath: "/v/show.mkv",
		ToneMap:  true,
		MediaInfo: &contracts.MediaInfo{Streams: []contracts.MediaStream{
			{Index: 0, Type: "video", Codec: "hevc", PixelFormat: "yuv420p10le", ColorTransfer: "arib-std-b67"},
		}},
	})
	if got.Mode != "transcode" || !got.Available {
		t.Fatalf("HDR+tonemap = %+v, want transcode", got)
	}
	// SDR flags must not trip the HDR branch.
	got = Plan(PlanInput{
		Request:  contracts.PlanRequest{ItemID: "i", Client: "web"},
		FilePath: "/v/show.mp4",
		MediaInfo: &contracts.MediaInfo{Streams: []contracts.MediaStream{
			{Index: 0, Type: "video", Codec: "h264", PixelFormat: "yuv420p", ColorTransfer: "bt709"},
		}},
	})
	if got.Mode != "direct" {
		t.Fatalf("bt709 h264 mp4 = %+v, want direct", got)
	}
}

func TestPlanRejectsBadInput(t *testing.T) {
	if _, err := (Planner{}).Invoke(contracts.CapPlaybackPlan, "nope"); err == nil {
		t.Fatal("wrong input type must fail")
	}
	if _, err := (Planner{}).Invoke("lain.playback.other@1", PlanInput{}); err == nil {
		t.Fatal("wrong capability must fail")
	}
}

// capsOf builds a capability set the way the gateway does, so the planner
// tests and the wire format cannot drift apart.
func capsOf(tokens string) *contracts.ClientCapabilities {
	return contracts.ParseCapabilities([]string{tokens})
}

func mkvStreams() []contracts.MediaStream {
	return []contracts.MediaStream{
		{Index: 0, Type: "video", Codec: "h264", PixelFormat: "yuv420p"},
		{Index: 1, Type: "audio", Codec: "aac", Default: true},
	}
}

// TestPlanDirectPlaysReportedCapabilities is the D-058 core: when a
// client reports what it can decode, the container and every track the
// player would use must be covered, and anything less transcodes.
func TestPlanDirectPlaysReportedCapabilities(t *testing.T) {
	cases := []struct {
		name  string
		path  string
		caps  string
		want  string
		apply func(*contracts.MediaInfo)
	}{
		{
			name: "matroska covered", path: "/v/show.mkv",
			caps: "mkv,mkv/h264,mkv/aac", want: "direct",
		},
		{
			name: "audio family missing", path: "/v/show.mkv",
			caps: "mkv,mkv/h264", want: "transcode",
		},
		{
			name: "video family missing", path: "/v/show.mkv",
			caps: "mkv,mkv/aac", want: "transcode",
		},
		{
			name: "container missing", path: "/v/show.mkv",
			caps: "mp4,mp4/h264,mp4/aac", want: "transcode",
		},
		{
			name: "nothing claimed", path: "/v/show.mkv",
			caps: "", want: "transcode",
		},
		{
			name: "hevc claimed for matroska", path: "/v/show.mkv",
			caps: "mkv,mkv/hevc,mkv/aac", want: "direct",
			apply: func(info *contracts.MediaInfo) { info.Streams[0].Codec = "hevc" },
		},
		{
			name: "h264 high 10 stays out", path: "/v/show.mkv",
			caps: "mkv,mkv/h264,mkv/aac", want: "transcode",
			apply: func(info *contracts.MediaInfo) { info.Streams[0].PixelFormat = "yuv420p10le" },
		},
		{
			name: "codec without a token", path: "/v/show.mkv",
			caps: "mkv,mkv/h264,mkv/aac", want: "transcode",
			apply: func(info *contracts.MediaInfo) { info.Streams[1].Codec = "pcm_s16le" },
		},
		{
			// Only the tracks a player selects are consulted: the
			// second, non-default track cannot veto a file whose
			// default track decodes.
			name: "unused track does not veto", path: "/v/show.mkv",
			caps: "mkv,mkv/h264,mkv/aac", want: "direct",
			apply: func(info *contracts.MediaInfo) {
				info.Streams = append(info.Streams, contracts.MediaStream{Index: 2, Type: "audio", Codec: "ac3"})
			},
		},
		{
			name: "default track does decide", path: "/v/show.mkv",
			caps: "mkv,mkv/h264,mkv/aac", want: "transcode",
			apply: func(info *contracts.MediaInfo) {
				info.Streams[1].Default = false
				info.Streams = append(info.Streams, contracts.MediaStream{Index: 2, Type: "audio", Codec: "ac3", Default: true})
			},
		},
		{
			name: "hevc mp4 claimed", path: "/v/movie.mp4",
			caps: "mp4,mp4/hevc,mp4/aac", want: "direct",
			apply: func(info *contracts.MediaInfo) { info.Streams[0].Codec = "hevc" },
		},
		{
			// The container must be one the gateway can label honestly.
			name: "container outside the vocabulary", path: "/v/old.avi",
			caps: "avi,avi/h264,avi/ac3", want: "transcode",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			info := &contracts.MediaInfo{Format: "matroska,webm", Streams: mkvStreams()}
			if tc.apply != nil {
				tc.apply(info)
			}
			got := Plan(PlanInput{
				Request:   contracts.PlanRequest{ItemID: "item-1", Client: "web", Capabilities: capsOf(tc.caps)},
				FilePath:  tc.path,
				MediaInfo: info,
			})
			if got.Mode != tc.want {
				t.Fatalf("plan=%+v, want %s", got, tc.want)
			}
		})
	}
}

// TestPlanKeepsLegacyRulesWhenNothingIsReported: an old client, curl or
// any other consumer that sends no capability list must see exactly the
// behaviour it saw before D-058 — including Matroska transcoding, and
// including an MP4 whose audio the conservative rules reject.
func TestPlanKeepsLegacyRulesWhenNothingIsReported(t *testing.T) {
	mkv := Plan(PlanInput{
		Request:   contracts.PlanRequest{ItemID: "item-1", Client: "web"},
		FilePath:  "/v/show.mkv",
		MediaInfo: &contracts.MediaInfo{Streams: mkvStreams()},
	})
	if mkv.Mode != "transcode" {
		t.Fatalf("mkv without caps=%+v, want the conservative transcode", mkv)
	}
	ac3 := Plan(PlanInput{
		Request:  contracts.PlanRequest{ItemID: "item-1", Client: "web"},
		FilePath: "/v/movie.mp4",
		MediaInfo: &contracts.MediaInfo{Streams: []contracts.MediaStream{
			{Index: 0, Type: "video", Codec: "h264", PixelFormat: "yuv420p"},
			{Index: 1, Type: "audio", Codec: "ac3", Default: true},
		}},
	})
	if ac3.Mode != "transcode" {
		t.Fatalf("ac3 without caps=%+v, want the conservative transcode", ac3)
	}
	// A claim is what changes the answer, and only a claim.
	reported := Plan(PlanInput{
		Request:  contracts.PlanRequest{ItemID: "item-1", Client: "web", Capabilities: capsOf("mkv,mkv/h264,mkv/ac3")},
		FilePath: "/v/show.mkv",
		MediaInfo: &contracts.MediaInfo{Streams: []contracts.MediaStream{
			{Index: 0, Type: "video", Codec: "h264", PixelFormat: "yuv420p"},
			{Index: 1, Type: "audio", Codec: "ac3", Default: true},
		}},
	})
	if reported.Mode != "direct" {
		t.Fatalf("reported ac3=%+v, want direct", reported)
	}
}

// TestPlanDirectsDesktopRegardlessOfCapabilities: mpv never probes and
// never consults the capability list — the desktop plays its own files.
func TestPlanDirectsDesktopRegardlessOfCapabilities(t *testing.T) {
	got := Plan(PlanInput{
		Request:   contracts.PlanRequest{ItemID: "item-1", Client: "mpv", Capabilities: capsOf("")},
		FilePath:  "/v/show.mkv",
		MediaInfo: &contracts.MediaInfo{Streams: mkvStreams()},
	})
	if got.Mode != "direct" || !got.Available {
		t.Fatalf("desktop plan=%+v, want direct+available", got)
	}
}
