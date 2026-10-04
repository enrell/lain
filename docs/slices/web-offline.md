# Web offline (browser) — plan and first cut

Part of the reading + downloads slice. Goal: items a user saves keep
working in the browser with the server unreachable, inside a byte limit
the user controls and the browser's own quota.

## First cut (built)

Scope: **comics and manga**. Their pages are small, independent images,
so Cache Storage holds them well and nothing needs Range requests.

- **Opt-in worker.** `web/src/service-worker.ts` is built by SvelteKit
  but not auto-registered (`serviceWorker.register: false`). The app
  registers it the first time something is saved offline
  (`lib/offline/store.ts`) and asks for persistent storage. A browser
  that never saves anything runs exactly as before.
- **Network-first, narrow routing** (`lib/offline/routes.ts`, unit
  tested). Online, every request goes to the server unchanged; a cached
  copy answers only when the network fails. The worker answers:
  - navigations → the SPA shell (`/`, refreshed on every online load);
  - the build's own assets → precached at install, per build version;
  - `/api/setup/status`, `/api/me` → so a cold start offline still boots
    signed in (seeded when saving, refreshed online);
  - saved items' reads: `/api/items/{id}/pages`, `/pages/{n}`,
    `/progress`, `/api/catalog/{id}`, `/api/catalog/{id}/episodes`.
    Cache keys drop `?token=`, so an image saved under one token is found
    under the next. Copies are refreshed online only if already saved.
  Streams, transcodes, lists, search, writes: never touched.
- **Store** (`lib/offline/store.ts`, tested with in-memory fakes): one
  cache `lain-offline-v1` plus an index entry. Saving fetches the reader
  view, checks the budget **before** fetching any page, then fetches
  every page, the catalog item, its episode list and progress; a failure
  rolls the item back. Remove and remove-everything free the bytes.
- **Budget** (`lib/offline/budget.ts`): the smaller of the user's cap
  (default 1 GiB, 0 = browser grant only) and 80% of the browser quota
  minus what the origin already stores for other things
  (`navigator.storage.estimate()`).
- **UI.** The title page's "From the archive" panel has Save offline /
  Remove offline copy (`o`). YOU › Offline (`g o`, Ctrl+K "offline")
  shows usage against the budget, the cap (applies instantly, D-087),
  saved items (Enter reads, Delete removes, j/k move) and Remove
  everything.

Verified in headless Chromium against a real `lain serve` with the
server process killed: `/read/{id}` cold-started, rendered page 1,
turned pages from cache, and YOU › Offline listed the saved volume.

## Not yet (in order)

1. **Offline progress.** Page turns while offline fail to PUT and are
   lost. Plan: the worker answers a failed `PUT /api/items/{id}/progress`
   with 202, keeps the latest body per item in IndexedDB, and replays on
   `online`/next page load (last write wins — progress is monotonic in
   practice, the server keeps `updated_at`). The reader reads the cached
   progress copy, so it must also update that copy on each write.
2. **Theme offline.** `/api/theme` is not cached, so the live Omarchy
   palette falls back to the default while offline. Add it to the
   session reads.
3. **Video.** Not with Cache Storage: whole-response entries of
   hundreds of MB are slow to write and cannot serve `Range` without
   reading the entry. Plan: save the original file (`/stream`, or a
   transcoded MP4 when the browser cannot decode it) into OPFS in
   chunks with resume (the server already answers Range), and have the
   worker answer `/api/items/{id}/stream` Range requests by slicing the
   OPFS file. HLS sessions stay online-only. Same budget, larger
   default cap prompt.
4. **Library view of saved items** when offline (Home/Library currently
   show errors offline; YOU › Offline is the entry point).
5. **Eviction policy** like the CLI's (drop watched, synced items
   first) once read state is tracked offline.
6. A web Download button (`/stream?download=1`) for a plain file save —
   waiting for the social branch to land on the title page.
