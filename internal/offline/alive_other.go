//go:build !unix

package offline

import "os"

// processAlive is conservative off unix: FindProcess succeeds only for
// a live process on Windows.
func processAlive(pid int) bool {
	p, err := os.FindProcess(pid)
	if err != nil {
		return false
	}
	_ = p.Release()
	return true
}
