# Recovery

What survives a bad plugin, precisely.

## Swap-time (before anything changes)

`Registry.Swap` validates, in order: capability exists, generation
matches (when fenced), every candidate is registered, serves the
capability, and reports healthy. Any failure returns an error and the
active binding is untouched. Event `swap-rejected` is logged.

## Call-time (active provider fails)

`Registry.CallOne` tries the active provider; on error it tries the
last-good provider once and logs `fallback`. Independent capabilities
keep serving — a broken identifier cannot break playback or progress.

## Withdraw

`Registry.Withdraw` removes a provider from all bindings. Bindings with
remaining providers bump generation and continue; an `exactly-one`
binding with no alternative stays on its generation and reports
`dependency-unavailable` (fail closed, never silent empty results).

## What is NOT reverted

Removing a plugin reverts bindings, registrations, timers and jobs. It
does not undo: files a plugin deleted, messages already sent, external
API writes, delivered bytes or secrets, or confirmed destructive
migrations. Plugins with such powers are trusted explicitly and use
mediated services (gateway-owned writes, scoped secrets).

## Operator view

- `GET /api/plugins`: composition with generations, provider ids, and
  the recent event log (`swap`, `swap-rejected`, `fallback`,
  `withdraw`, `withdraw-degraded`).
- `lain doctor`: runtime + matrix environment report as JSON.
- `GET /api/admin/backup`: consistent snapshot stream (admin only).
- `lain backup` / `lain restore`: validated snapshots; restore never
  overwrites a live database.
- Data dir documents are atomic JSON; a corrupt file errors loudly at
  load (`store.CorruptError`), pointing at the exact document.
- Last-known composition persists in `composition.json`; a fresh
  `gateway.New` revalidates it against registered providers and refuses
  to boot on mismatch rather than serving half a composition.
