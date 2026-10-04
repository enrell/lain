//go:build !unix

package downloads

import "errors"

// diskFree is unknown off unix; the free-space floor is then skipped
// (Check ignores probe errors) and only the byte budget applies.
func diskFree(string) (int64, error) { return 0, errors.ErrUnsupported }
