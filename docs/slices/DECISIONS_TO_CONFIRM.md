# Decisions to confirm

> **Resolved 2026-10-04:** the user accepted every row below, including
> the four points flagged at the end. They are recorded as `D-089`…`D-107`
> in `docs/advisor/decisions.md`, which is now the source of truth; this
> file stays as the review record. The acquisition slice continues from
> `D-108`.

The slices could not run the advisor protocol, so their load-bearing
choices were recorded as proposals: `P-*` (reading + downloads,
`reading-downloads.md`, `web-offline.md`) and `S-*` (social,
`social.md`). Each needs the user's approval before it becomes a `D-`
entry in `docs/advisor/decisions.md`. **The decision log has not been
edited.**

Numbers are suggestions: the log ends at `D-088`; reading merged first
on `integration/slices`, so it takes the lower numbers. If the
acquisition slice (`feat/acquisition`) lands proposals of its own
first, shift these. Rows marked **new** were made after the slice
summaries (round 2 / integration) and were never listed as `P-*`.

For each row: approve, amend, or reject. A rejection names the code to
revert in the "Implemented in" column.

## Reading + downloads

| Suggested | From | Decision | Extends / touches | Implemented in |
|---|---|---|---|---|
| `D-089` | P-1 | Reading metadata comes from the archive: `ComicInfo.xml` is parsed by the comic provider while indexing and rides additively on `ComicPages.info` and the reader view. An archive declaring right-to-left (`Manga=YesAndRightToLeft`) overrides the library default. No new capability, no composition bump, no network. Catalog identity is not rewritten from it. | D-085, D-019 | `internal/plugins/comic/comicinfo.go`, `gateway/reader.go`, `ComicInfoPanel.svelte` |
| `D-090` | P-2 | One stdlib resumable fetcher (`internal/downloads`: `.part` files, `Range` + `If-Range`, restart on a changed origin) with a byte budget, shared by the server manager and the CLI offline store. It is not a plugin: it moves bytes and owns goroutines. | D-001, D-006, D-007 | `internal/downloads/fetch.go`, `limits.go` |
| `D-091` | P-3 | Server download manager, admin-only. Jobs persist in a new bbolt bucket `downloads` and survive restart (running → queued, resumed from the part). States `queued/running/paused/done/failed/canceled`, bounded concurrency. The destination is a library root (rescanned when done) or the download directory. Only http/https URLs; safe base names; never overwrites a file. | D-004 (new bucket), D-018 | `internal/downloads/manager.go`, `gateway/downloads.go`, `/api/downloads*` |
| `D-092` | P-4 | Download limits are settings, never constants: `dir`, `max_bytes` (default 20 GiB), `min_free_bytes` (default 5 GiB), `concurrency` (2), `keep_finished_days` (30), and (**new**, round 2) `max_retries` (5). Crossing a limit fails with stable codes `quota-exceeded` / `disk-full` (HTTP 507). | D-011 | Settings › Downloads |
| `D-093` | P-5 | Download cleanup never deletes finished media. It removes partial files of failed/canceled jobs, orphaned partials and expired records only. Deleting library files stays with Q-040. | Q-040 | `Manager.Cleanup` |
| `D-094` | P-6 | Client offline copies are a CLI feature first: `lain download` keeps a store under the user data dir with its own limits, evicts watched and synced copies least-recently-used first (on by default, configurable), plays local copies from `lain watch`, and keeps offline progress until `lain download sync`. Desktop offline stays in `lain-desktop`. | D-013, D-015 | `internal/offline`, `cmd/lain/download.go` |

## Social

| Suggested | From | Decision | Extends / touches | Implemented in |
|---|---|---|---|---|
| `D-095` | S-1 | Social is a fourth domain beside catalog, userstate and the list, with its own buckets and provider `lain-social-bolt`. It never writes catalog, progress or list records; it only observes progress writes through a gateway hook. | D-008, D-078 | `internal/plugins/social` |
| `D-096` | S-2 | Four exactly-one capabilities (`lain.social.graph@1`, `.activity@1`, `.reviews@1`, `.collections@1`). The composition gains the bindings through `Upgrade` without a version bump. | D-075, D-076 | `core/composition.go` |
| `D-097` | S-3 | Media-agnostic work identity: `WorkRef{kind, title, item_id?}`, keyed by kind + `TitleKey(title)`. A manga and an anime of the same name stay separate. Any lowercase kind token works. | D-056, D-078, D-085 | `contracts/social.go` |
| `D-098` | S-4 | The audience is one server: "public" means every signed-in account on this server. Nothing is federated or anonymous, and every social route requires auth. | D-086 | `gateway/social.go` |
| `D-099` | S-5 | Privacy model: `public/friends/private` for profile details, activity and ratings, plus `discoverable`, `allow_requests` and `hide_progress`. Defaults are profile public, activity friends, ratings friends. Blocking is symmetric and silent. Admins get no privacy override in the API. | D-086 | Settings › Privacy |
| `D-100` | S-6 | Activity comes from progress writes, deduplicated: progress at most once per work per 6 h, completion only on the transition (D-084 rule). Each account keeps its newest 500 rows. | D-084 | `recordSocialProgress` |
| `D-101` | S-7 | Comments are explicit public acts on a work, visible to every signed-in account except across a block. Authors delete their own and admins may moderate. Length caps: comments 1000, reviews 2000, names 80, notes 280. | D-086 | reviews capability |
| `D-102` | S-8 | Shares target friends only and land as notifications (`friend_request`, `friend_accepted`, `share`, `collection_shared`, `reply`). Each account keeps its newest 200. | — | activity capability |
| `D-103` | S-9 | Collections are named, ordered lists of works (≤ 500), `private/friends/public`, plus read-only sharing with chosen friends. They are independent of the lain list (the list tracks, a collection curates). | D-078 | collections capability |
| `D-104` | S-10 | Favorites are up to 12 works on the social profile record, not the account record (auth keeps identity). | D-005, D-086 | graph capability |

## New (round 2 and integration — not yet listed anywhere as P-/S-)

| Suggested | Decision | Extends / touches | Implemented in |
|---|---|---|---|
| `D-105` | Transient download failures (network errors, 5xx, 408/429, dropped or short bodies) retry with exponential backoff (5 s doubling to 10 min, ±20% jitter), honoring `Retry-After` up to 1 h, up to `max_retries`. Retries resume by byte range, including from origins without a validator when they advertised ranges and the total still matches; any other mismatch restarts clean. Progress resets the retry count. | `internal/downloads` |
| `D-106` | Browser offline is opt-in and network-first. The service worker is not auto-registered; it installs on the first "Save offline". Online behavior is unchanged, and cached copies answer only when the network fails, for the app shell, the two session reads and saved items' reads. The first cut covers comics and manga in Cache Storage, within a per-browser cap (default 1 GiB) bounded by 80% of the browser quota. Video offline is planned via OPFS + Range (`web-offline.md`). | `web/src/service-worker.ts`, `lib/offline/` |
| `D-107` | The web "Download file" action is a plain attachment of the original file (`GET /api/items/{id}/stream?download=1`, token in the query string like every media URL). It is available to every signed-in user on single-item title pages, and to admins for selected episodes on series pages (key `d`). | `DownloadFileButton.svelte`, `handleStream` |

## Points that need a conscious yes

- **D-091 / D-092:** the server now fetches arbitrary http(s) URLs for
  admins, including LAN addresses. That is intended for self-hosters, but
  it is a server-side request capability. Restricting private ranges
  would be a one-line policy change if wanted.
- **D-094:** watched-copy eviction is **on** by default in the CLI. It
  only deletes local copies whose progress is synced, but it does delete
  files without asking.
- **D-096** and the reading slice both avoided a composition version
  bump, so the next slice that needs one takes the next number freely.
- **D-106** adds the app's first service worker. Any later slice that
  wants one must share it (one worker per scope).

## Draft — approved, recorded as `D-153`

Recorded in `docs/advisor/decisions.md` on 2026-10-05, after the
acquisition slice's `D-108`…`D-152` landed. The text below is the
original draft.

Approved by the user on 2026-10-04. It takes the **first number after the
acquisition slice's entries** (acquisition keeps `D-108` onward and is
recording them in its own worktree). Add it to
`docs/advisor/decisions.md` only after those entries land on `main`.
Nothing gets renumbered.

- `D-???` — Docker media mounts are read-write by default, so deleting an
  episode or series from Lain removes the files on the host. The
  installer-generated compose file, the shipped `docker-compose.yml` /
  `docker-compose.dev.yml` and the README drop `:ro`. The installer runs
  the container as the invoking user's uid, and `docker run` users pass
  `--user $(id -u):$(id -g)` so the container can write. Appending `:ro`
  to a mount forbids deletes again. This is the deployment half of
  "delete from disk" (Q-040). Library files are deleted only by explicit
  user action, never by download cleanup (D-093). Source: user approval
  2026-10-04; commit `c5150ad`.
