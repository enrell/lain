//go:build !linux

package bench

// rss reports 0 where /proc is unavailable; MemSnapshot still covers
// the Go allocator there.
func rss() uint64 { return 0 }
