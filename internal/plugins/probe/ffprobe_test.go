package probe

import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
)

func TestParseNormalizesTechnicalStreams(t *testing.T) {
	raw := []byte(`{
  "format":{"format_name":"matroska,webm","duration":"42.5"},
  "streams":[
    {"index":0,"codec_type":"video","codec_name":"h264","profile":"High","pix_fmt":"yuv420p","width":1920,"height":1080,"color_transfer":"bt709","disposition":{"default":1}},
    {"index":2,"codec_type":"audio","codec_name":"aac","channels":2,"tags":{"language":"JPN","title":"Main"},"disposition":{"default":1}},
    {"index":4,"codec_type":"subtitle","codec_name":"ass","tags":{"language":"eng"},"disposition":{"forced":1}}
  ]}`)
	got, err := parse(raw)
	if err != nil {
		t.Fatal(err)
	}
	if got.Format != "matroska,webm" || got.Duration != 42.5 || len(got.Streams) != 3 {
		t.Fatalf("info=%+v", got)
	}
	if a := got.Streams[1]; a.Index != 2 || a.Language != "jpn" || !a.Default {
		t.Fatalf("audio=%+v", a)
	}
	if sub := got.Streams[2]; !sub.Convertible || !sub.Forced {
		t.Fatalf("subtitle=%+v", sub)
	}
}

func TestProviderRejectsBadMessages(t *testing.T) {
	p := Provider{}
	if _, err := p.Invoke(contracts.CapMediaProbe, "bad"); err == nil {
		t.Fatal("wrong input must fail")
	}
	if _, err := p.Invoke(contracts.CapMediaProbe, contracts.MediaProbeRequest{}); err == nil {
		t.Fatal("empty path must fail")
	}
	if _, err := p.Invoke("lain.media.other@1", contracts.MediaProbeRequest{}); err == nil {
		t.Fatal("wrong capability must fail")
	}
}
