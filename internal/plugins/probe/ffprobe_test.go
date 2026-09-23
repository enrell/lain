package probe


import (
	"os"
	"path/filepath"
	"testing"

	"github.com/enrell/lain/internal/contracts"
)

// stubFFprobe installs a fake ffprobe at the front of PATH running the
// given shell body, so Invoke exercises the exec path deterministically.
func stubFFprobe(t *testing.T, body string) {
	t.Helper()
	dir := t.TempDir()
	bin := filepath.Join(dir, "ffprobe")
	if err := os.WriteFile(bin, []byte("#!/bin/sh\n"+body), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", dir+string(os.PathListSeparator)+os.Getenv("PATH"))
}

func TestInvokeRunsFFprobe(t *testing.T) {
	stubFFprobe(t, `cat <<'EOF'
{"format":{"format_name":"mov,mp4","duration":"3.0"},
 "streams":[{"index":0,"codec_type":"video","codec_name":"h264","pix_fmt":"yuv420p"}]}
EOF`)
	out, err := (Provider{}).Invoke(contracts.CapMediaProbe, contracts.MediaProbeRequest{FilePath: "/v/x.mp4"})
	if err != nil {
		t.Fatal(err)
	}
	info := out.(contracts.MediaInfo)
	if info.Format != "mov,mp4" || len(info.Streams) != 1 || info.Streams[0].Codec != "h264" {
		t.Fatalf("probe result: %+v", info)
	}
}

func TestInvokeFFprobeFailureAndGarbage(t *testing.T) {
	stubFFprobe(t, "exit 1")
	if _, err := (Provider{}).Invoke(contracts.CapMediaProbe, contracts.MediaProbeRequest{FilePath: "/v/x.mp4"}); err == nil {
		t.Fatal("ffprobe exit!=0 must fail")
	}
	stubFFprobe(t, `echo "not json"`)
	if _, err := (Provider{}).Invoke(contracts.CapMediaProbe, contracts.MediaProbeRequest{FilePath: "/v/x.mp4"}); err == nil {
		t.Fatal("garbage output must fail")
	}
	// With a stub that answers, an empty path must still be refused
	// before exec — the check is not incidental to ffprobe failing.
	if _, err := (Provider{}).Invoke(contracts.CapMediaProbe, contracts.MediaProbeRequest{}); err == nil {
		t.Fatal("empty path must refuse before exec")
	}
}

func TestHealthDependsOnPath(t *testing.T) {
	// An empty PATH hides every binary, including ffprobe.
	t.Setenv("PATH", t.TempDir())
	if err := (Provider{}).Health(); err == nil {
		t.Fatal("Health must fail without ffprobe on PATH")
	}
}

func TestStreamBitRateParsing(t *testing.T) {
	for raw, want := range map[string]int{
		"1234567": 1234567,
		"0":       0,
		"N/A":     0,
		"":        0,
		" 42 ":    42,
		"-5":      0,
	} {
		if got := streamBitRate(raw); got != want {
			t.Errorf("streamBitRate(%q)=%d, want %d", raw, got, want)
		}
	}
}

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
