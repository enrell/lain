# Plugin authoring (v0.1)

## Shape

A v0.1 plugin is a Go type implementing `core.Provider`:

```go
type Provider interface {
    ID() string            // e.g. "community.anime-parser"
    Capabilities() []string // e.g. []string{"lain.media.identify@1"}
    Health() error         // cheap, side-effect free
    Invoke(cap string, input any) (any, error)
}
```

Input/output Go types per capability are in `docs/CONTRACTS.md`.
Return `&core.Error{Code: "invalid-message", ...}` for wrong shapes;
codes `dependency-unavailable`, `stale-generation`, `outcome-unknown`
have registry meaning — do not invent new ones without documenting them.

## Manifest (for Matrix provisioning)

```json
{
  "id": "community.anime-parser",
  "version": "1.3.0",
  "capabilities": ["lain.media.identify@1"],
  "execution": {"kind": "process", "entrypoint": "/path/to/bin",
    "args": ["--matrix-sock", "{sock}", "--id", "{id}"]}
}
```

`internal/matrix.ExportManifests` writes these. The `plugin-run`
component mode (sdk/go `Connect` + `Serve`) is the next slice; v0.1
plugins link into the binary and register in `gateway.New`.

## Install / swap / withdraw

- Register: code change in `gateway.New` (v0.1) → restart.
- Swap binding at runtime: `POST /api/plugins/swap` with the current
  `generation`. Health is checked first; rejection changes nothing.
- Withdraw: `Registry.Withdraw` (API next slice); bindings fall back to
  remaining providers with a generation bump, or stay marked degraded.

## Rules

1. Private state in your own document (`store.Dir`); never another
   plugin's tables or files.
2. No network/file access beyond what your capability needs; declare it
   in your README.
3. Timeouts on every external process; missing binaries degrade your
   `Health`, never the boot.
4. Provenance: tag everything you produce (`origin`, `evidence`).
5. Ship a test with fixtures proving your contract behavior, including
   what you decline (for ordered-many providers, declining is a feature).
