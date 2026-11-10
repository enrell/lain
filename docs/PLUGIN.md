# Plugin authoring

## Shape

An in-process plugin is a Rust type implementing `lain_core::Provider`:

```rust
pub trait Provider: Send + Sync {
    fn id(&self) -> &str;                 // e.g. "community.anime-parser"
    fn capabilities(&self) -> Vec<String>; // e.g. vec!["lain.media.identify@1".into()]
    fn health(&self) -> Result<(), Error>; // cheap, side-effect free
    fn invoke(&self, cap: &str, op: &str, input: Value) -> Result<Value, Error>;
}
```

Input/output JSON shapes per capability are in `docs/CONTRACTS.md`.
Return `Error::invalid(...)` (`invalid-message`) for wrong shapes;
codes `dependency-unavailable`, `stale-generation`, `outcome-unknown`
have registry meaning — do not invent new ones without documenting them.

## Component mode

Any provider can run out-of-process as a component (Rust components use
`lain_component::serve`): NDJSON frames on a Unix socket (a named pipe on
Windows), a `hello` handshake that declares
capabilities, `invoke`/`result`/`error` frames keyed by sequence id,
and a transport-level `health`. See "Component mode" in
`docs/CONTRACTS.md` for the protocol — third-party components can
implement it in any language.

A component manifest makes an executable provisionable:

```json
{
  "id": "community.anime-parser",
  "version": "1.3.0",
  "capabilities": ["lain.media.identify@1"],
  "execution": {"kind": "process", "entrypoint": "/path/to/bin",
    "args": ["--sock", "{sock}"]}
}
```

`{sock}` expands to the socket the host allocates. Drop the manifest
into `<data-dir>/plugins/` and the provisioner spawns, handshakes,
health-checks and registers it; delete the manifest to unregister and
kill it; rewrite it to reload.

## Install / swap / withdraw

- Install at runtime: drop a manifest into `<data-dir>/plugins/` —
  no rebuild or restart.
- In-process providers register in `gateway.NewWithOptions` (restart).
- Swap binding at runtime: `POST /api/plugins/swap` with the current
  `generation`. Health is checked first; rejection changes nothing.
- Withdraw: `POST /api/plugins/withdraw` marks a provider degraded and
  falls the binding back to remaining providers; the registration
  survives for a later swap-back.
- Unregister (provisioner path): removes the provider from every
  binding and deletes the registration entirely.

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
6. Never take the database path — the trusted core hands you snapshots
   or documents, not the live store.
