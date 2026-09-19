package playback

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

func TestPlanRejectsBadInput(t *testing.T) {
	if _, err := (Planner{}).Invoke(contracts.CapPlaybackPlan, "nope"); err == nil {
		t.Fatal("wrong input type must fail")
	}
	if _, err := (Planner{}).Invoke("lain.playback.other@1", PlanInput{}); err == nil {
		t.Fatal("wrong capability must fail")
	}
}
