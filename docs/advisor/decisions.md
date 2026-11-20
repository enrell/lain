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
