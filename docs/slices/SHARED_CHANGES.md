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

## Acquisition (`feat/acquisition`, branched from `feat/reading-downloads`)

Own packages (no conflict expected): `internal/torrent/`,
`internal/acquire/`, `internal/plugins/release/`, `internal/plugins/indexer/`,
`internal/kv/acquire.go`, `internal/gateway/acquire*.go`, and the web
files under `routes/acquire/`, `routes/settings/indexers/`,
`routes/settings/acquisition/`, `lib/api/acquire.ts`,
`lib/components/acquire/`. Rows are added below as shared files are touched.

| File | Change | Conflict risk |
| --- | --- | --- |
| `internal/core/composition.go` | +2 bindings after `lain.settings.integrations@1`, behind a blank line and a comment so gofmt does not realign the map: `lain.release.parse@1` (ordered-many: `lain-release-model`, `lain-release-tokenizer`), `lain.indexer@1` (ordered-many: `lain-indexer-torznab`). **No `Version` bump**: `Upgrade` adds missing capabilities. | Low: append-only. |
| `internal/gateway/server.go` | imports `acquire`, `plugins/indexer`, `plugins/release`; `Server.acquire` field after `downloads`; `s.acquire.Close()` before `s.downloads.Close()` in `Close`; three `reg.Register` lines after `listlink.NewAniList()`; `s.startAcquire(dataDir)` after `s.downloads = dl`; `s.routesAcquire()` after `s.routesDownloads()`. | Low: one-line insertions next to the reading slice's lines. |
| `internal/plugins/release` (new) reuses `catalog.TitleKey`; `internal/acquire` reads `downloads.Limits`/`Usage` and calls `downloads.Error` with keyed fields only. | — | none |
| `web/src/lib/api/index.ts` | `acquire` import + key; `export type * from './acquire'` (types live in `api/acquire.ts`, `types.ts` untouched). | Low. |
| `web/src/lib/i18n/messages/en.ts` | `nav.acquire`; `settings.section.indexers`/`acquisition`; four `settings.entry.*`; a new top-level `acquire` namespace appended at the end. | Medium: append-only, same spots other slices append to. |
| `web/src/lib/settings/sections.ts` | SERVER sections `indexers` (`g n`) and `acquisition` (`g a`) after `downloads`; four palette entries after `downloadLimits`. | Low. Chords `n` and `a` are now taken. |
| `web/src/lib/utilities/guards.ts` | `ADMIN_ROUTES` += `/settings/indexers`, `/settings/acquisition`, `/acquire`. (Note: `/settings/downloads` is not in this list on the reading branch.) | Low. |
| `web/src/lib/components/navigation/AppShell.svelte` | `session` import; an admin-only `/acquire` link between Search and Settings. Mobile nav unchanged. | Low. |
| `web/src/lib/components/navigation/MobileNav.svelte` | Items come from the new `nav-items.ts` (`MOBILE_NAV` + `visibleNav(isAdmin)`); admins get `/acquire`; the grid's column count follows the item count instead of `grid-cols-5`. Main's social slice also edits this file (Social tab, 6 columns): on merge, add `{ href: '/social', label: 'nav.social', exact: false }` to `MOBILE_NAV` and its icon to the map — the derived columns then fit both. | Medium: same file as social on main. |
| `internal/gateway/server.go` (Phase 2) | one more field after `acquire`: `episodeCountSeam func(title, kind string) int` (test seam so acquisition tests never reach metadata providers, D-120). | Low. |
