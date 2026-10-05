# Shared changes

Edits a slice made outside its own files, so parallel slices can avoid
or resolve conflicts. One section per slice; one row per touched spot.

## Reading + downloads (`feat/reading-downloads`)

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

### `internal/downloads` public interface (consumed by feat/acquisition)

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

### Browser offline (see `web-offline.md`)

| File | Change | Conflict risk |
| --- | --- | --- |
| `web/svelte.config.js` | `kit.serviceWorker.register: false` — the new `src/service-worker.ts` is registered by the app only after the user saves something offline. | Low, but any other slice adding a service worker must share this one (one worker per scope). |
| `web/src/lib/settings/sections.ts` | YOU section `offline` (chord `g o`) and palette entry `offlineStorage`, both appended at the **end** of their arrays (the rail groups by scope) to stay away from social's `privacy` lines. | Low. |
| `web/src/lib/i18n/messages/en.ts` | `settings.section.offline`, `settings.entry.offlineStorage`, new top-level `offline` namespace. | Low. |
| `web/src/routes/item/[id]/+page.svelte` | The two `ComicInfoPanel` lines now pass `{item}` instead of `itemId`. The panel binds **`o`** (save/remove offline) on `/item/{id}` for comics and manga; social's keys there are `r s c f m`. | Low. |

## Social (`feat/social`)

Everything else the slice adds is in new files: `internal/contracts/social.go`,
`internal/kv/social.go`, `internal/plugins/social/`, `internal/gateway/social.go`
(+ tests), `docs/slices/social.md`, and on the web `lib/api/social.ts`,
`lib/social/`, `lib/stores/social.svelte.ts`, `lib/components/social/`,
`routes/social/`, `routes/u/`, `routes/settings/privacy/`.

| File | Change | Conflict risk |
|---|---|---|
| `internal/core/composition.go` | +4 bindings (`lain.social.*@1` → `lain-social-bolt`) appended after `lain.settings.integrations@1`, behind a comment. **No `Version` bump** — `Upgrade` adds missing capabilities on its own, so the reading slice keeps the next version number. | Low: append-only inside the bindings map. |
| `internal/gateway/server.go` | import `plugins/social`; `social.New(db)` after `list.New`; `reg.Register(soc)` after `reg.Register(lst)`; `s.routesSocial()` after `s.routesList()`; one line in `handleProgressPut` after the scrobble block: `s.recordSocialProgress(v.UserID, prev, in)`. | Low: five one-line insertions next to list-slice lines. |
| `internal/kv` | **Not edited**: social buckets are declared in the new `internal/kv/social.go` and created by the social provider, so `kv.Open`'s bucket list is untouched. | none |
| `docs/CONTRACTS.md` | New `## Social` section inserted right before `## Component mode (wire protocol)`. | Low. |
| `docs/advisor/decisions.md` | **Not edited**: social choices are `S-*` in `docs/slices/social.md` until merge, so neither slice grabs the next `D-` number. | none |
| `web/src/lib/api/index.ts` | import + `social` entry in `api`; `export type * from './social'` and `targetOf`. Types live in the new `api/social.ts`; `types.ts` is **not** edited. | Low: one line in each list. |
| `web/src/lib/i18n/messages/en.ts` | `nav.social`, `nav.unread`; `settings.section.privacy`; six `settings.entry.*` (privacy rows); a new top-level `social` block appended before `} as const`. | Medium: the reading slice will likely add keys too — both append, so merges are mechanical. |
| `web/src/lib/settings/sections.ts` | `privacy` section (`/settings/privacy`, chord `g v`) after `security`; six palette entries after `password`. | Low. Chord `v` is now taken. |
| `web/src/lib/components/navigation/AppShell.svelte` | `/social` link after `/list`, unread badge on it, and an `$effect` that starts/stops the `socialBadge` poll. | Low. |
| `web/src/lib/components/navigation/MobileNav.svelte` | `/social` item (Users icon) after `/list`, grid `grid-cols-5` → `grid-cols-6`, unread dot. | Medium if the reading slice also adds a mobile tab: the column count must match the item count. |
| `web/src/routes/item/[id]/+page.svelte` | import `TitleSocial`; one block after the main `{#if}` chain renders `<TitleSocial target={{ item_id: item.id }} />` for every item, video or reading. | Low: appended after the page body, nothing inside existing markup changed. |

Behavior notes for the reading slice:

- Reading progress already flows through `PUT /api/items/{id}/progress`
  (D-085), so comic/manga reading shows up in the activity feed with no
  extra work. If the reader adds another progress write path, call
  `s.recordSocialProgress(userID, prev, next)` after the write.
- Social records reference works by `kind` + normalized title. New kinds
  need nothing from this slice; kind must be a lowercase token
  (`[a-z0-9-]{1,24}`).
- Title pages for comic/manga items get the social panel automatically.
  Its single-key shortcuts are `r` `s` `c` `f` `m` on `/item/{id}`; if the
  reading slice adds page-level keys there, avoid those letters (the
  reader routes `/read/*` are untouched).

## Integration (`integration/slices`)

Resolved when merging both slices onto `main`; see the summary in
`DECISIONS_TO_CONFIRM.md`'s header for the decisions themselves.

| File | Resolution |
|---|---|
| `internal/gateway/server.go` | Kept both: `s.routesDownloads()` then `s.routesSocial()` after `s.routesList()`. Everything else merged textually. |
| `web/src/lib/i18n/messages/en.ts` | Both slices appended top-level namespaces before `} as const`; kept `downloads`, `reader`, `offline`, then `social`. |
| `docs/slices/SHARED_CHANGES.md` | This file: one section per slice. |
| `web/src/routes/item/[id]/+page.svelte` | Web Download button (`DownloadFileButton`, key **`d`**) in the single-item action row, before the admin actions. Keys on `/item` now: social `r s c f m`, reading `o` (comic/manga), download `d`. |
| `web/src/lib/components/media/TitleView.svelte` | The same button in the (admin) episode-selection toolbar, downloading the selected files one after another; `d` while selecting. |
| `web/src/lib/i18n/messages/en.ts` | New top-level `download` namespace (button labels). |

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
| `web/src/lib/api/playback.ts` (Phase 3) | `SidecarTrack` type import; `sidecars(id)` and `sidecarUrl(id, token, n)` appended to `playback`. | Low: append-only. |
| `web/src/lib/components/player/Player.svelte` (Phase 3) | Sidecar subtitles (A-29): `sidecars` state loaded in the first `onMount`; `embeddedSubtitle`/`chosenSidecar` derived from `selectedSubtitle`; `subtitleSrc` prefers a chosen sidecar; transcode requests and the burned-in note use `embeddedSubtitle` (a sidecar never reaches a transcode); `onSubtitleChoice` rebuilds only when the embedded part changes; sidecar options appended to the Subtitles menu. Helpers live in the new `lib/player/sidecars.ts`. | Medium: the player is large and shared — the edits are local to the subtitle code. |
| `internal/core/composition.go` (Phase 3) | +1 binding after `lain.indexer@1`: `lain.subtitle@1` (ordered-many: `lain-subtitle-opensubtitles`). No `Version` bump; `Upgrade` adds it. | Low: append-only. |
| `internal/gateway/server.go` (Phase 3) | Import `internal/plugins/subtitles`; `reg.Register(subtitles.NewOpenSubtitles())` after the Torznab registration. Subtitle routes and deps are wired from `acquire.go`, not here. | Low: two added lines. |
| `web/src/lib/settings/sections.ts` (Phase 3) | +3 Ctrl+K entries: `acquisitionAutomation`, `acquisitionProfiles`, `acquisitionSubtitles` (all `/settings/acquisition`, admin). | Low: append-only. |
| `web/src/lib/i18n/messages/en.ts` (Phase 3) | `settings.sections.acquisition{Automation,Profiles,Subtitles}`; `acquire.profiles.subtitle*`/`hi*`; `acquire.settings.subtitleHours*`; new `acquire.subtitles` block. | Low: keys inside the acquisition namespace. |
| `web/src/routes/item/[id]/+page.svelte` (Phase 3 UI) | Admin-only "Subtitles" action (key `t`) in `adminActions`, a `pageKey` window handler, and a `SubtitlePanel` mounted for the item's file or the title's present files (not for comics/manga). Imports `Captions`, `SubtitlePanel`, `t`, `isTypingTarget`. | Low: additive; the rest of the page is untouched. |

## Integration (`integration/acquisition`)

`feat/acquisition` merged onto `main` after the reading, downloads and
social slices. Resolutions:

| File | Resolution |
|---|---|
| `internal/core/composition.go` | Kept both: the four `lain.social.*@1` bindings, then `lain.release.parse@1`, `lain.indexer@1`, `lain.subtitle@1`. No version bump. |
| `internal/gateway/server.go` | Kept both: `s.routesSocial()` then `s.routesAcquire()`. |
| `internal/gateway/acquire_test.go` | Dropped its `wantCode` helper; the identical one from `social_test.go` serves both. |
| `web/src/lib/api/index.ts` | Kept both re-exports (`./social` with `targetOf`, `./acquire`). |
| `web/src/lib/components/navigation/AppShell.svelte` | Kept both imports (`socialBadge`, `session`); links: Home, Library, List, Social, Search, Acquire (admin), Settings. |
| `web/src/lib/components/navigation/MobileNav.svelte`, `nav-items.ts` | The acquisition `MOBILE_NAV` table wins and gains `/social` (Users icon) after `/list`; the social unread dot is kept. Columns follow the item count: six for users, seven for admins. |
| `web/src/lib/settings/sections.ts` | `indexers` and `acquisition` sections before `offline`, which stays last as its comment asks; palette entries appended after `offlineStorage`. Chords stay unique (`n`, `a`, `o`). |
| `web/src/lib/utilities/guards.ts` | `ADMIN_ROUTES` keeps `/settings/downloads` and adds `/settings/indexers`, `/settings/acquisition`, `/acquire`. |
| `web/src/lib/i18n/messages/en.ts` | Kept both: `nav.social`/`nav.unread` and `nav.acquire`; settings sections and entries of both; the `reader`, `offline`, `social`, `download` namespaces, then `acquire`. |
| `web/src/routes/item/[id]/+page.svelte` | Kept both: the `TitleSocial` block, then the admin `SubtitlePanel`. Keys on `/item` now: social `r s c f m`, reading `o`, download `d`, subtitles `t` (admin, video). |
| `docs/advisor/decisions.md` | Sections in order: reading + downloads, social, downloads/offline integration, acquisition (D-108…D-123), Phase 2 (D-124…D-134), Phase 3 (D-135…D-152). The "next number" line moved to the end. |
| `docs/CONTRACTS.md` | Kept both: `## Social`, then `## Acquisition`. |
| `docs/slices/SHARED_CHANGES.md` | This file: one section per slice. |
