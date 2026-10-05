# Slice: reading + downloads

Branch `feat/reading-downloads` (worktree `../lain-reading`). A parallel
session owns the SOCIAL slice; every shared-code touch is listed in
`SHARED_CHANGES.md` next to this file.

## Starting point

Reading is already on `main` (D-085): `comic`/`manga` libraries scan
`cbz`/`cbr`/`cb7`, a volume is `Season` and a chapter/issue `Episode`,
`lain.comic.pages@1` indexes pages, the gateway streams them
(`/api/items/{id}/pages[/n]`), and the web reader (`ReaderBase` with
`MangaReader` RTL/spreads/webtoon and `ComicReader` LTR/fit/zoom) resumes
from the progress record (`position_sec` = 1-based page) and walks
prev/next volume or chapter. This slice does not rebuild any of that.

Downloads did not exist. The user asked for **both** meanings
(2026-10-03): a server-side download manager that fetches content into
a library, and client offline copies of library items.

## Advisor protocol

`AGENTS.md` requires the `advisor` subagent for new slices and public
API contracts. That tool is not available in this session, so the
load-bearing choices below are written as **proposed decisions**
(`P-1`…) with their reasoning, not as `D-XXX` entries: numbering is left
to the merge so it cannot collide with the social slice. The user's own
answer ("both") is the source for scope.

All six were accepted at merge (2026-10-04) as `D-089`…`D-094` in
`docs/advisor/decisions.md`; round 2 and integration added `D-105`…`D-107`.

## Proposed decisions

- **P-1 Reading metadata comes from the archive.** `ComicInfo.xml`
  (the de-facto comic metadata file inside cbz/cbr/cb7) is parsed by the
  comic provider while it indexes pages and rides additively on
  `ComicPages.info` (omitempty) and the reader view. `Manga=YesAndRightToLeft`
  sets the default direction to `rtl` even in a `comic` library. No new
  capability, no composition bump, no network. Catalog identity is not
  rewritten from it (identity stays filename-derived, D-019).
- **P-2 One resumable fetcher, two owners.** `internal/downloads` holds
  a stdlib-only resumable HTTP fetcher (`.part` file, `Range: bytes=n-`,
  restart from zero when a server ignores ranges, `If-Range` validator so
  a changed remote never splices two files) plus a byte budget it checks
  while writing. The server manager and the CLI offline store both use
  it. It is not a plugin: like `localplay`, it moves bytes and owns
  goroutines, which plugins must not (D-006, D-007).
- **P-3 Server download manager.** Admin-only. Jobs persist in a new
  bbolt bucket `downloads` (survive restart: a job that was running comes
  back `queued` and resumes from its `.part`). States: `queued`,
  `running`, `paused`, `done`, `failed`, `canceled`. Bounded concurrency.
  Destination is a library root (then that library is rescanned) or the
  configured download directory. Only `http`/`https` URLs; filenames are
  reduced to a safe base name and never overwrite an existing file.
- **P-4 Limits are settings, never constants.** Server settings
  (`GET/PUT /api/downloads/settings`): `dir`, `max_bytes` (budget for
  everything the manager has written and still tracks, `0` = unlimited),
  `min_free_bytes` (refuse/stop when the target filesystem would drop
  below it), `concurrency`, `keep_finished_days` (record retention).
  Defaults are deliberately small for the current machine and are
  raised from the UI later. A job that would cross a limit fails with the
  stable code `quota-exceeded` / `disk-full` instead of filling the disk.
- **P-5 Cleanup never deletes finished media on the server.** Server
  cleanup removes `.part` files of failed/canceled jobs and old job
  records. Deleting library files stays an open question (Q-040).
- **P-6 Client offline copies are a CLI feature first.** `lain download`
  keeps an offline store under the user data dir (configurable `--dir`),
  fetches the original file through the existing `/api/items/{id}/stream`
  (Range already supported — no new server endpoint), with a client-side
  quota and LRU-of-watched eviction. `lain watch` plays the local copy
  when one is complete; offline progress is kept locally and pushed on
  the next online run. Web offline (service worker/OPFS) and desktop are
  later slices (the desktop lives in `lain-desktop`, D-013).

## Work plan (small commits)

1. Plan + shared-changes docs (this file).
2. ComicInfo.xml parsing in `internal/plugins/comic`, exposed in the
   reader view; web reader honors the archive's direction.
3. `internal/downloads`: resumable fetcher, budget, safe names (tests
   with `httptest`, a few KB of fixture bytes).
4. `internal/downloads` manager: queue, pause/resume/cancel, persistence,
   quota, cleanup (tests with a gated test server).
5. Gateway routes `/api/downloads*` + `downloads` bucket (tests).
6. CLI `lain download` offline store + `lain watch` local-copy playback.
7. Web: SERVER › Downloads settings page (queue + limits), Ctrl+K entry.

## Out of scope / left for later

- Web offline library, desktop offline (different repo).
- Torrent or indexer sources (would need dependencies and touch D-010).
- PDF reading (D-085 excludes it).
- Automatic deletion of finished library files (Q-040).

## Server download API (admin only)

| Route | Body / result |
| --- | --- |
| `GET /api/downloads` | `{jobs, usage, settings}` — jobs newest first |
| `POST /api/downloads` | `{url, name?, library_id?}` → `201` job (`queued`) |
| `GET /api/downloads/{id}` | job |
| `POST /api/downloads/{id}/pause\|resume\|cancel` | job |
| `DELETE /api/downloads/{id}` | forget a done/failed/canceled job (file stays) |
| `GET/PUT /api/downloads/settings` | `{dir, max_bytes, min_free_bytes, concurrency, keep_finished_days}` |
| `POST /api/downloads/cleanup` | `{parts, records, freed_bytes}` |
| `GET /api/items/{id}/stream?download=1` | original file as an attachment (browser offline copy) |

Errors carry `{error, code}`; codes: `invalid-request` 400, `not-found`
404, `invalid-state` 409, `quota-exceeded`/`disk-full` 507. A job with
`library_id` lands in that library's root and triggers a rescan of that
library when it finishes. Defaults: 20 GiB budget, 5 GiB free-space
floor, 2 concurrent, records kept 30 days.

## Status (2026-10-03)

Built on `feat/reading-downloads`:

- ComicInfo.xml parsed from cbz (stdlib) and cbr/cb7 (bsdtar/7z), capped
  and sanitized, fuzzed (`FuzzParseInfo`); rides on `ComicPages.info` and
  the reader view; an archive declaring right-to-left wins over the
  library default.
- `internal/downloads`: resumable fetcher (Range + If-Range, restart on a
  changed origin or ignored Range, 416 = already complete), `Limits`
  (byte budget + free-space floor), safe names, and the server manager
  (FIFO queue, bounded concurrency, pause/resume/cancel that answer with
  the settled state, restart resumes from `.part`, quota accounting that
  counts a running job's full size, cleanup that never deletes finished
  files).
- Gateway admin routes (table above) and `?download=1` on streams.
- `internal/offline` + `lain download …`: client store with its own
  limits, LRU eviction of watched and synced copies, single-writer lock,
  offline progress kept and synced; `lain watch` plays a local copy when
  one exists.
- Web: SERVER › Downloads (`g d`, three Ctrl+K entries) — queue with row
  keys (`/`, `p`, `c`, `Delete`, `j`/`k`), storage usage, cleanup, and
  limits staged behind Ctrl+S.

Smoke-tested against a real `lain serve`: a 738-byte cbz downloaded into
a manga library was rescanned into a readable `rtl` item with its
ComicInfo, then copied offline with the CLI and removed again.

## Round 2 (2026-10-03)

- Server downloads retry transient failures (network, 5xx, 408/429,
  dropped transfers) with exponential backoff and jitter, honoring
  `Retry-After`; retries resume by byte range, also from origins with no
  validator when they advertise ranges and the total still matches.
  `max_retries` is a setting (default 5; Settings › Downloads).
- `lain watch --dry-run` names the offline copy it would play.
- The title page shows ComicInfo (series, number, volume, writer,
  artist, publisher, year, language, genres, summary, reading direction
  and where it comes from) in a self-contained panel.
- Browser offline reading: plan and first cut in `web-offline.md`.

## Left for later

- Browser offline beyond comics and manga: see `web-offline.md`.
- Desktop offline copies (`lain-desktop`, D-013).

Done at integration (2026-10-04): the web "Download file" action
(`/stream?download=1`, D-107) and registering P-1…P-6 as `D-089`…`D-094`.

Done on 2026-10-05: `lain download run` retries transient failures with
the server's backoff policy (D-105), resuming from the partial bytes.
The count is `lain download config --retries N` (default 5, 0 turns
retries off); progress resets it. Ctrl+C during a wait leaves the copy
queued for the next run.
