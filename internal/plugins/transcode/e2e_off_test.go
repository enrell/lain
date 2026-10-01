//go:build !e2e

package transcode

// e2eEnabled is false without -tags e2e: real-ffmpeg tests skip, so the
// per-commit path stays seconds. CI and `just test-e2e` run them.
const e2eEnabled = false
