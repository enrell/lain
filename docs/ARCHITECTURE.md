# Architecture

## Principle

Matrix administers components. Lain defines media contracts. Plugins
implement policy. The trusted base preserves authority, data,
communication and recovery.

"Everything is a plugin" means every **functionally relevant decision**
is replaceable without forking the server — not that every function is
a process.

## The fixed core

`internal/core` + `internal/store` + `internal/auth` + `internal/gateway`
+ `internal/matrix` (seam). It owns:

- composition load/validate/persist (`composition.json`)
- provider registry, health-gated swap, generation fencing, last-good
  fallback, withdraw with degraded marking
- first-run setup, bcrypt passwords, HS256 tokens, per-request auth
- atomic JSON documents (temp + rename; corrupt files reported, never
  silently ignored)
- the HTTP boundary; media bytes via `ServeContent` (Range native)
- Matrix manifest export + environment doctor

It does not parse filenames, understand seasons, call metadata APIs, or
transcode. Those are plugins.

## Data plane vs control plane

Plugin calls carry references and small JSON (candidates, proposals,
plans). Media bytes never enter a plugin call: the gateway authorizes,
resolves the catalog entry, and streams from disk. This is the rule that
lets a future Matrix-daemon mode work unchanged — Matrix streams are
bounded and text-oriented; gigabytes of video must not cross them.

```text
plugin -> plan/reference -> gateway validates -> URL -> client
```

## Plugin boundary rules

1. A plugin implements `core.Provider`: `ID`, `Capabilities`, `Health`,
   `Invoke(cap, input) (any, error)`.
2. Wrong input types are `invalid-message` errors, never panics.
3. `Health` must be cheap and side-effect free; swap calls it before
   mutating anything.
4. Private state lives in the plugin's own document; shared facts
   (catalog item, progress) pass through capabilities with provenance.
5. No plugin touches `net/http`, process-wide state, or another
   plugin's files.
6. Heavy binaries (ffmpeg/ffprobe) are spawned by the plugin that needs
   them, with timeouts; absence degrades the capability, never the boot.

## Composition modes

Declared per capability in the composition (`exactly-one`,
`ordered-many`, `first-accepted`, `merge-many`, `fan-out`). v0.1 wires
the default set in `core.DefaultComposition`; `merge-many` and `fan-out`
are declared for metadata providers and events (next slice).

## Storage (v0.1)

JSON documents under the data dir: `users`, `secret`, `libraries`,
`catalog`, `userstate`, `composition`. The catalog exposes a versioned
`ExportDoc` (`lain.catalog-export@1`); any replacement catalog
(sqlite-catalog) must implement export/import of that document.

## Matrix seam

v0.1 runs embedded: no daemon, no PKI, no socket. `internal/matrix`
exports provider manifests in Matrix component shape and diagnoses the
operator environment (`lain doctor --matrix-bin ...`). The `Invoke`
surface already mirrors `invoke(cap, input)`, so component-mode
(`lain plugin-run` + sdk/go `Connect`) is a transport change, not a
redesign. Provisioning under `matrix-managed` comes after the PKI flow,
not before the product works.
