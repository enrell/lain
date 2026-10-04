package downloads

import "fmt"

// Limits bounds what a download store may put on disk. Both are
// configuration, never constants: the machine this runs on today is
// small and the next one will not be (P-4 in docs/slices).
type Limits struct {
	// MaxBytes caps everything the store has written and still tracks
	// (finished files plus partials). 0 means no cap.
	MaxBytes int64 `json:"max_bytes"`
	// MinFreeBytes is the free space the target filesystem must keep
	// after a write. 0 disables the check.
	MinFreeBytes int64 `json:"min_free_bytes"`
}

// freeBytes is the free-space probe; tests replace it.
var freeBytes = diskFree

// Check reports whether a store that already accounts for used bytes
// may grow by grow more in dir. It returns a typed quota-exceeded or
// disk-full error naming the numbers, never a bare refusal.
func (l Limits) Check(dir string, used, grow int64) error {
	if grow <= 0 {
		return nil
	}
	if l.MaxBytes > 0 && used+grow > l.MaxBytes {
		return &Error{Code: CodeQuota, Msg: fmt.Sprintf("needs %s, %s of %s budget left",
			HumanBytes(grow), HumanBytes(max(l.MaxBytes-used, 0)), HumanBytes(l.MaxBytes))}
	}
	if l.MinFreeBytes > 0 {
		free, err := freeBytes(dir)
		if err == nil && free-grow < l.MinFreeBytes {
			return &Error{Code: CodeDiskFull, Msg: fmt.Sprintf("needs %s, %s free and %s must stay free",
				HumanBytes(grow), HumanBytes(free), HumanBytes(l.MinFreeBytes))}
		}
	}
	return nil
}

// Full reports whether a store already at used bytes has no room left
// at all — the check a queue runs before accepting new work, phrased as
// the state of the store rather than the size of one request.
func (l Limits) Full(dir string, used int64) error {
	if l.MaxBytes > 0 && used >= l.MaxBytes {
		return &Error{Code: CodeQuota, Msg: fmt.Sprintf("download budget used up: %s of %s", HumanBytes(used), HumanBytes(l.MaxBytes))}
	}
	if l.MinFreeBytes > 0 {
		if free, err := freeBytes(dir); err == nil && free <= l.MinFreeBytes {
			return &Error{Code: CodeDiskFull, Msg: fmt.Sprintf("only %s free and %s must stay free", HumanBytes(free), HumanBytes(l.MinFreeBytes))}
		}
	}
	return nil
}

// HumanBytes formats a byte count with binary units.
func HumanBytes(n int64) string {
	const unit = 1024
	if n < unit {
		return fmt.Sprintf("%d B", n)
	}
	div, exp := int64(unit), 0
	for m := n / unit; m >= unit && exp < 5; m /= unit {
		div *= unit
		exp++
	}
	return fmt.Sprintf("%.1f %ciB", float64(n)/float64(div), "KMGTPE"[exp])
}
