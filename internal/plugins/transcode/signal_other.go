//go:build !unix

package transcode

import (
	"errors"
	"os"
)

// Process throttling needs POSIX job control; other platforms keep
// transcoding (the process simply never pauses).
var errThrottleUnsupported = errors.New("process throttling is not supported on this platform")

func pauseProcess(*os.Process) error  { return errThrottleUnsupported }
func resumeProcess(*os.Process) error { return errThrottleUnsupported }
