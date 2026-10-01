//go:build e2e

package gateway

// e2eEnabled turns on the real-ffmpeg suite (go test -tags e2e). It is
// off by default so the per-commit path stays seconds; CI runs it.
const e2eEnabled = true
