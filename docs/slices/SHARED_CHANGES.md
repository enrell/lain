# Shared changes — reading + downloads slice

Every edit this slice makes outside its own packages, so the parallel
SOCIAL slice can avoid or resolve conflicts. Own packages (no conflict
expected): `internal/downloads/`, `internal/plugins/comic/`,
`cmd/lain/download*.go`, `internal/gateway/downloads*.go`,
`web/src/routes/settings/downloads/`, `web/src/lib/api/downloads.ts`.

| File | Change | Conflict risk |
| --- | --- | --- |
| `internal/kv/kv.go` | New bucket constant `BDownloads` (`downloads`) added to the `Open` bucket list. | Low: one appended identifier; if social adds a bucket too, keep both in the list. |
| `internal/gateway/server.go` | `Server.downloads` field; manager built + started at the end of `NewWithOptions` (just before `s.routes()`), closed first in `Close`; `s.routesDownloads()` added after `s.routesList()`; `handleStream` honors `?download=1` (Content-Disposition attachment); `mime` import. | Medium: social will likely add a `routesX()` call and fields in the same spots — keep both lines. |
| `cmd/lain/main.go` | `case "download"` in the command switch and one usage line. | Low. |
| `cmd/lain/watch.go` | `playOneWithPlayer`: 3 lines after `streamURL` is built — use `localCopyPath(item.ID)` when an offline copy exists. | Low. |
| `cmd/lain/watch_queue.go` | `queueURL`: same 3-line local-copy check. | Low. |
| `web/src/lib/api/index.ts` | `downloads` import + key in the `api` object. | Low: alphabetical neighbor of `catalog`. |
| `web/src/lib/api/types.ts` | Appended `ComicInfo` (+ `ReaderView.info`) and the `Download*` types at the end of the file. | Low: append-only. |
| `web/src/lib/i18n/messages/en.ts` | `settings.section.downloads`, three `settings.entry.download*` keys, and a new top-level `downloads` namespace at the end. | Medium: social will add keys to the same `settings.section`/`entry` maps — keep both. |
| `web/src/lib/settings/sections.ts` | SERVER section `downloads` with chord key `d` (`g d`), three palette entries. | Medium: if social adds a section, pick a key other than `d`. |
| `web/src/routes/item/[id]/+page.svelte` | Import `ComicInfoPanel`; one line after the series branch's `{/key}` and one line after the `enrichment?.synopsis` block, both `{#if reading}`-guarded. Social's `TitleSocial` block (appended after the main `{#if}` chain) is a different spot. No new keys on `/item` (social owns `r s c f m` there). | Low. |
| `web/src/lib/i18n/messages/en.ts` | New top-level `reader.info` namespace (ComicInfo panel) and `downloads.retrying` / `downloads.limits.retries*` keys. | Low: new namespace. |

## `internal/downloads` public interface (consumed by feat/acquisition)

The acquisition slice branched from this one at `c77ae57` and reuses the
manager and limits. The list below is what changed since the first
reading-downloads summary (`d632a04`) and is **already in that branch
point**; all of it is additive, nothing was renamed, removed or changed
meaning. Changes made after `c77ae57` are logged under "After the
acquisition branch point" (none so far).

- `Error` gained `Retry bool` and `RetryAfter time.Duration`. Positional
  `&Error{code, msg}` literals no longer compile; use keyed fields
  (`&Error{Code: …, Msg: …}`), which this package now does everywhere.
- New `Retryable(err) (bool, time.Duration)`, `DefaultBackoff(attempt)`,
  `Limits.Full(dir, used)`.
- `Request` gained `Total int64` and `Ranges bool` (validator-less resume
  when the origin advertised byte ranges and the total still matches);
  `Result` gained `Ranges bool`.
- `Fetch` now marks network errors, 5xx/408/429, interrupted bodies and
  short bodies as retryable; on a mismatched range or a refused resume
  it deletes the part and returns a retryable error (the retry starts
  clean) instead of a plain `http-error`.
- `Job` gained `Ranges`, `Attempts`, `RetryAt` (JSON `ranges`,
  `attempts`, `retry_at`). A transient failure with retries left keeps
  the job `queued` with `code`/`error` set and `retry_at` in the future;
  the scheduler skips it until then.
- `Settings` gained `MaxRetries` (`max_retries`, default 5, 0–20).
  Saved settings without the field load with the default.
- `Manager` gained the exported `Backoff func(int) time.Duration` field
  (tests shorten it).
- `Pause`/`Cancel` on a running job wait (up to 5s) for the transfer to
  settle and return the settled state.

### After the acquisition branch point (`c77ae57`)

- None to `internal/downloads` or `internal/offline`.

## Browser offline (see `web-offline.md`)

| File | Change | Conflict risk |
| --- | --- | --- |
| `web/svelte.config.js` | `kit.serviceWorker.register: false` — the new `src/service-worker.ts` is registered by the app only after the user saves something offline. | Low, but any other slice adding a service worker must share this one (one worker per scope). |
| `web/src/lib/settings/sections.ts` | YOU section `offline` (chord `g o`) and palette entry `offlineStorage`, both appended at the **end** of their arrays (the rail groups by scope) to stay away from social's `privacy` lines. | Low. |
| `web/src/lib/i18n/messages/en.ts` | `settings.section.offline`, `settings.entry.offlineStorage`, new top-level `offline` namespace. | Low. |
| `web/src/routes/item/[id]/+page.svelte` | The two `ComicInfoPanel` lines now pass `{item}` instead of `itemId`. The panel binds **`o`** (save/remove offline) on `/item/{id}` for comics and manga; social's keys there are `r s c f m`. | Low. |
