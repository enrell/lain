//go:build unix

package transcode

import (
	"os"
	"syscall"
)

// pauseProcess suspends a running ffmpeg (throttling); resumeProcess
// continues it. SIGSTOP/SIGCONT keep the process state intact.
func pauseProcess(p *os.Process) error  { return p.Signal(syscall.SIGSTOP) }
func resumeProcess(p *os.Process) error { return p.Signal(syscall.SIGCONT) }
