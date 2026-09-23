package thumbnail

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-23

import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/testutil/contract"
)

func TestProviderContract(t *testing.T) {
	// No happy-path sample: the real call execs ffmpeg.
	contract.Run(t, New(t.TempDir()), []contract.Cap{{Name: contracts.CapTransformThumb}})
}
