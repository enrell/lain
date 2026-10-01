//go:build !e2e

package gateway

// e2eEnabled is false without -tags e2e: the real-ffmpeg suite skips.
const e2eEnabled = false
