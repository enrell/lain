package transcode

import (
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
)

// The settings page must be able to offer hardware before one backend is
// selected. The fake ffmpeg advertises every supported backend and only
// accepts a hardware encode at 128x128, matching the minimum imposed by
// several real VA-API drivers.
func TestProbeBuildDiscoversHardwareFromSoftwareMode(t *testing.T) {
	ffmpeg := filepath.Join(t.TempDir(), "ffmpeg")
	script := `#!/bin/sh
case "$*" in
  *-encoders*)
    printf '%s\n' \
      ' V..... h264_vaapi' \
      ' V..... h264_nvenc' \
      ' V..... h264_qsv' \
      ' V..... h264_amf' \
      ' V..... h264_v4l2m2m' \
      ' V..... h264_videotoolbox' \
      ' V..... h264_rkmpp'
    ;;
  *-filters*)
    printf '%s\n' ' ... hwupload' ' ... format'
    ;;
  *-hwaccels*)
    printf '%s\n' vaapi cuda qsv amf rkmpp
    ;;
  *h264_vaapi*|*h264_nvenc*|*h264_qsv*|*h264_amf*|*h264_v4l2m2m*|*h264_videotoolbox*|*h264_rkmpp*)
    case "$*" in
      *size=128x128*) exit 0 ;;
      *) exit 1 ;;
    esac
    ;;
esac
exit 0
`
	if err := os.WriteFile(ffmpeg, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}

	settings := contracts.DefaultTranscodeSettings()
	settings.FFmpegPath = ffmpeg
	settings.HardwareAcceleration = contracts.HWNone
	caps := probeBuild(settings, time.Now)

	for _, backend := range []string{
		contracts.HWVAAPI,
		contracts.HWNVENC,
		contracts.HWQSV,
		contracts.HWAMF,
		contracts.HWV4L2M2M,
		contracts.HWVideoToolbox,
		contracts.HWRKMPP,
	} {
		if !caps.Hardware[backend] {
			t.Errorf("Hardware[%q]=false, want discovered before selection", backend)
		}
	}
}
