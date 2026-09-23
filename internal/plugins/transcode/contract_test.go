package transcode


import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/testutil/contract"
)

func TestProviderContract(t *testing.T) {
	tr := NewConfigured(t.TempDir(), Config{})
	t.Cleanup(func() { _ = tr.Close() })
	// No happy-path samples: real transcode calls spawn ffmpeg. The
	// protocol checks (typed errors, honest caps, no panics) are the
	// contract boundary.
	contract.Run(t, tr, []contract.Cap{
		{Name: contracts.CapPlaybackTranscode},
		{Name: contracts.CapPlaybackTranscodeV2},
		{Name: contracts.CapPlaybackTranscodeV3},
	})
}
