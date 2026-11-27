# Decisions

Reset on 2026-11-13. The earlier log is in git history
(`git log -p -- docs/advisor/decisions.md`).

## D-301 — deleting a file leaves nothing behind

2026-11-20, asked for by the owner: deleting from the title page "is buggy,
gives no useful feedback, sometimes does not work and leaves the content on
disk, and what is deleted stays as a ghost in the interface". Three causes,
none of them an error the route reported (every delete answered 200):

- An import hardlinks or copies, so the torrent copy kept the bytes while
  the grab seeded or kept its data. `DELETE /api/items/{id}` now tells the
  acquire module, which drops that copy through `discard` (library files
  are never touched, D-112) and stops its seeding. Deleting one episode of
  a pack ends the pack's seeding: freeing the space the admin asked for
  wins over the seed goal.
- A monitored title read the deleted episode as missing and fetched it
  again. The unit is recorded in `Monitored.skipped` and is not wanted
  until the list is cleared (Acquire › Wanted, `w`). `skipped` is additive
  and server-owned on a write that omits it.
- A title whose every file is missing stayed on the lists. The owner's
  answer to Q-041 stands (missing means something only while the series
  exists); the client-side rule that implemented it was lost when paging
  moved to the server (D-283). `GET /api/catalog` and `GET /api/search`
  leave such a title out. The rows stay (D-068, D-073): nothing is
  hard-deleted, and a returning file lists the title again.

With the file go its subtitle sidecars, its own stills and the folders it
leaves empty (never the library's folder, never a folder holding anything
else). The answer carries `removal` so a client says what happened: bytes
freed, downloads stopped, "not wanted again", or that another hard link
still holds the bytes. A delete while the library's folder is unreachable
is refused (409) instead of answering 200 for a file that would come back
with the disk. Not done: a list of hidden titles for admins, and purging
long-missing rows (D-068's deferred retention question).

## D-302 — metadata is its own pass, not a scan phase

2026-11-27, asked for by the owner: "you must separate enrich from scan".
A scan of 872 files walked, identified and committed in about 100 ms, then
spent minutes in its `enrich` phase asking Kitsu, AniList, Jikan and TVMaze
about titles none of them can know (personal clips), about 0.7 s
each, every scan: "nothing found" is only cached when every provider
answered, and for a day.

- The scan ends at commit. `Scanner` runs the backfill on its own thread
  (`lain-enrich`) with its own status (`ScanStatus.enrich`, `EnrichStatus`)
  and event (`enrich-state`); a scan may start while it runs, and a pass
  asked for meanwhile runs once after it. `POST /api/library/enrich`
  (manager) starts one. The `enrich` scan phase and
  `progress.enrich_total/enrich_done` are gone; `stats.enriched` stays,
  always 0, for older clients.
- `Library.local_metadata` ("Fetch metadata online" off): the library's
  items are enriched like a private library's (D-268), from `.nfo` only.
  Default off, so every existing library keeps fetching.
- An item every network provider answered "unknown" for is remembered
  (`m\0<item>` in the metadata cache table) and left out of later
  backfills until one asks with `retry`; a skipped (failed) provider
  never makes a miss, and a manual `POST /api/catalog/{id}/enrich` always
  asks. Items of local-only libraries are never remembered (a file read).
