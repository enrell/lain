# Slice: acquisition (native *arr stack)

Branch `feat/acquisition` (worktree `../lain-acquire`), branched from
`feat/reading-downloads` at `c77ae57` to build on its download manager
and limits. The reading session keeps committing to its own branch; this
slice never edits it. Shared-code touches are listed in
`SHARED_CHANGES.md` (section *Acquisition*).

Goal: what Prowlarr (indexers), Sonarr/Radarr (series/movies),
Mylar/Kaizoku (comics/manga) and Bazarr (subtitles) do, built into Lain.

## Advisor protocol

No `advisor` subagent exists in this harness. Load-bearing choices are
**proposed decisions** `A-1`… for the merge to number as `D-XXX`. The
one choice `taste.md` forbids deciding alone — a new dependency — was
put to the user with measurements (2026-10-03):

> anacrolix/torrent v1.61 with `CGO_ENABLED=0`: Lain's cold build
> 23 s → ~65–70 s, +~19 MB binary, ~150 modules (pion/webrtc, otel,
> DHT, uTP), ~1.1 GB of module/build cache on a disk with 17 GB free.

The user chose **an in-house, standard-library BitTorrent engine**.

## Decisions (accepted 2026-10-04)

Accepted by the user as written, with one condition on cleanup (see
A-5). Recorded as `D-108`…`D-123` in `docs/advisor/decisions.md`
(`A-n` = `D-(107+n)`); the text below is kept for context.

- **A-1 No new dependency.** The BitTorrent engine, bencode, trackers,
  Torznab/Newznab client and the parser-socket client are standard
  library only (D-001, D-002; user answer above).
- **A-2 Engine scope v1.** BitTorrent v1 (BEP 3): `.torrent` and magnet
  links (BEP 9 `ut_metadata` over BEP 10 extensions), HTTP and UDP
  trackers (BEP 15, compact peers BEP 23), peer wire with pipelined 16 KiB
  block requests, SHA-1 piece verification, resume from a persisted
  bitfield, seeding to interested peers, inbound listener. **Not in v1:**
  DHT, PEX, uTP, protocol encryption, v2/hybrid torrents, web seeds.
  Magnets therefore need a tracker (`tr=`) or an indexer-supplied
  `.torrent`; this is stated in the UI. The engine moves bytes and owns
  goroutines, so it is a core package (`internal/torrent`) like
  `internal/downloads`, not a plugin (D-006/D-007, P-2).
- **A-3 Download clients are an interface.** `acquire.DownloadClient`
  (add/list/pause/resume/remove/stats) with the native engine as the
  first implementation; qBittorrent/Transmission adapters can follow
  without touching the import pipeline.
- **A-4 Disk is reserved up front.** A grab reserves the torrent's full
  size against the reading slice's `downloads.Limits` (`max_bytes`
  budget + `min_free_bytes` floor) before any byte is written; a grab
  that does not fit fails with `quota-exceeded`/`disk-full`. Torrent
  data is counted while it exists on disk (downloading or seeding).
- **A-5 Seeding is bounded and configurable.** Per-grab ratio limit and
  seed-time limit (either reached stops seeding), global defaults in
  settings; `0` means "do not seed" so a tight disk can import and
  delete immediately. Upload/download rate caps and peer limits are
  settings, never constants (P-4). Defaults: ratio 1.0 or 24 h, then
  delete the torrent copy. **Accepted with a condition:** cleanup never
  removes or unlinks a library file (hardlinked, moved in place, or
  inside a library root), and the download folder may not overlap a
  library (`cleanup_test.go`).
- **A-6 Release parsing is a capability.** `lain.release.parse@1`
  (ordered-many, first accepted): `lain-release-model` asks the
  installed lain-parser over `<data-dir>/parser.sock`
  (`POST /parse {"filename"}`; a null record is abstention) and
  `lain-release-tokenizer` reuses the identify tokenizer as fallback.
  The model is optional at runtime: no socket, a timeout or an error
  declines to the fallback and is logged, never fatal.
- **A-7 Indexers are a capability with config in the input.**
  `lain.indexer@1` (ordered-many, `unsupported-protocol` declines like
  `lain.listlink@1`); `lain-indexer-torznab` speaks Torznab and
  Newznab, so direct indexers and Jackett/Prowlarr both work. Indexer
  definitions (URL, API key, categories, rate limit) are stored by the
  acquisition store; the API key travels inside the contract input,
  persists server-side, is never logged (D-026) and never serialized
  back (clients see `has_api_key`).
- **A-8 Rate limits and health are per indexer.** Minimum interval
  between requests (default 2 s) enforced before each call; a capability
  probe (`t=caps`) is the health check, its result and last error stored
  on the indexer.
- **A-9 Admin only.** Every acquisition route is `requireAdmin`: it
  makes the server contact remote hosts and write to disk (same rule as
  P-3).
- **A-10 Import = parse, match, place, rescan.** When a grab completes,
  each media file (video, or cbz/cbr/cb7 in reading libraries) is
  parsed, matched to the grab's target library and an existing title
  (catalog `TitleKey` equality) or a new title folder, renamed with a
  fixed scheme, placed by **hardlink** when source and library share a
  filesystem (seeding keeps working), otherwise **copy** while seeding
  or **move** when not seeding, and the library is rescanned. Existing
  files are never overwritten.
- **A-11 Naming scheme v1** (fixed, configurable later), chosen so
  Lain's own identifiers read every imported name back as the same
  title and numbers (`TestImportedNamesRoundTrip`):
  seasoned episodes `Title/Season NN/Title - SNNEMM.ext` (a multi-episode
  file `SNNEMM-EKK`); absolute anime episodes are season 0 in the
  catalog, written `Title/[Group] Title - MMM.ext` when the group is
  known and `Title/Title - S00EMM.ext` otherwise (the anime identifier
  only reads a bare `- 03` after a `[Group]` tag); movies
  `Title (Year)/Title (Year).ext`; manga/comics `Title/Title v01 c001.cbz`.
  A download's title is matched to an existing library title first
  (case and punctuation insensitive) so new episodes join their show.
- **A-15 The engine always listens.** Trackers reject port 0 and a
  seeder must be reachable, so there is no "inbound off" mode:
  `listen_port` 0 means "a free port at each start" (default 51413,
  forward it behind NAT). `LAIN_ACQUIRE_LISTEN_PORT` seeds the port on
  first boot (containers, tests); if the port is taken at boot the
  engine falls back to a free one and logs it — acquisition never
  blocks server start, and its routes answer 503 if it cannot start.
- **A-16 Usenet is searchable, not downloadable.** Newznab results are
  parsed and shown, marked "no usenet client yet"; grabbing one is
  refused with `unsupported-protocol` until an NZB client exists.
- **A-12 Own buckets, own file.** `acq_indexers`, `acq_grabs`,
  `acq_settings`, `acq_torrents` (engine resume state) are declared in
  `internal/kv/acquire.go` and created by their owners, so `kv.Open` is
  not edited.
- **A-13 Tests never touch the network.** Fixtures are generated bytes;
  swarms are in-process (httptest tracker, UDP tracker on loopback,
  seeder and leecher engines on 127.0.0.1). No real tracker, indexer or
  torrent is ever contacted, and fixtures use placeholder names only
  (D-010: `[Fansub-A]`, `tracker-exemplo`).
- **A-14 Phase 2 automation stays out of Phase 1 contracts** but the
  grab record already carries the parsed release, quality and target
  title, so monitoring, upgrades and blocklists can build on it.

## Phases

### Phase 1 — core loop (this delivery)

1. Plan + shared changes (this file).
2. `internal/torrent/bencode` — decode/encode, fuzzed.
3. `internal/torrent` metainfo + magnet parsing.
4. Trackers: HTTP + UDP announce, in-process test trackers.
5. Peer wire + session engine: download, verify, resume, seed;
   ut_metadata for magnets. Loopback swarm tests.
6. `lain.release.parse@1`: model socket client + tokenizer fallback.
7. `lain.indexer@1`: Torznab/Newznab client, caps health, rate limit.
8. `internal/acquire`: store (indexers, settings, grabs), manager over
   `DownloadClient`, quota reservation, seeding limits, import.
9. Gateway `/api/acquire/*` routes.
10. Web: SERVER › Indexers and Acquisition settings; `/acquire` with
    Search and Queue (keys, `t()`).

### Phase 2 — automation (Sonarr/Radarr/Mylar role)

Decisions for Phase 2, **accepted 2026-10-04** and recorded as
`D-124`…`D-134` (`A-n` = `D-(107+n)`). None adds a dependency or
changes a frozen contract; the D-032 metadata change stays deferred.
Note for A-22: as built and accepted (D-129), an import failure keeps
its data for a manual retry; only download failures and stalls are
removed.

- **A-17 Monitored titles are the unit of automation.** A monitored
  title names a library, a kind (the library type), a title (plus
  aliases), a quality profile and a numbering mode, in its own bucket
  `acq_monitored`. It never writes the catalog (D-008): what the library
  has is read from the catalog by title key, every time.
- **A-18 "Wanted" needs no new metadata contract.** Expected numbers
  come from, in order: an explicit range the user sets; the episode
  count of the title's metadata record (`lain.metadata.search@1` +
  `resolve@1`, already returning `episodes`); and "newer than the
  highest present" for ongoing titles (RSS fills it). Missing = expected
  − present. Movies are wanted until a file exists. Manga and comics
  track chapters (and volumes, for volume releases) the same way.
  Per-season episode lists and air dates would need a `MetadataRecord`
  change, frozen by D-032 — out of scope until the user specifies one.
- **A-19 Quality profiles.** A profile is an ordered list of allowed
  resolutions (best first), allowed sources, a cutoff resolution,
  preferred release groups (bonus) and blocked words/groups (reject),
  size bounds per episode/chapter/movie, a minimum seeder count and a
  proper/repack preference. Stored in `acq_profiles`; one default
  profile is created on first use.
- **A-20 One decision engine.** Every candidate (manual search, RSS,
  automatic search) goes through the same pure function: parse →
  title/number fit → profile accept/reject with reasons → score. Manual
  search keeps showing rejected results with their reasons; automation
  grabs only accepted ones, best score first, one grab per wanted
  number set.
- **A-21 Upgrades until cutoff.** A present file whose quality is below
  the profile cutoff stays wanted for upgrade; a better accepted release
  is grabbed. At import the files it replaces are first moved out of the
  library into `<dir>/replaced/<grab>/` — never deleted automatically,
  listed in the UI and purged only by an explicit admin action — and put
  back if placing the new files fails. This keeps D-112's spirit: Lain
  never destroys a library file on its own.
- **A-26 Quality ledger.** Lain's names carry no quality (D-118), so
  each import records `library path → resolution` in its own bucket
  `acq_quality`, independent of the queue; upgrade checks read it before
  the file name. Without it an imported file would look "unknown",
  hence upgradable, and automation would grab the same release forever
  (caught by `TestSearchMonitoredGrabsUntilComplete`).
- **A-27 Metadata episode totals are cached on the monitored title**
  (`metadata_episodes`), refreshed when the title is searched, so
  computing what is wanted never calls the network; tests stub the
  lookup (D-120).
- **A-22 Blocklist on failure.** A grab that fails (download, import,
  metadata) or stalls (no progress for `stall_hours`, default 6) is
  blocklisted by info hash and title, removed with its data (D-112
  path), and the wanted numbers are searched again. The blocklist is
  `acq_blocklist`, viewable and clearable.
- **A-23 Schedules are settings.** RSS sync every `rss_minutes`
  (default 30, minimum 10) per enabled indexer (a query-less Torznab
  search on the indexer's categories, which honours the per-indexer
  rate limit, D-115), and a missing-items search every `search_hours`
  (default 12, 0 = only on demand). Adding a monitored title can search
  immediately. Automation is off until enabled in settings.
- **A-24 Anime numbering.** A monitored anime is `absolute` or
  `seasonal`. A season map (`season → first absolute episode`, e.g.
  S2 starts at 13) converts between the two, so an `S02E01` release
  satisfies absolute 13 and the reverse; split cours are modelled as
  seasons in the map. The release group preference lives in the
  profile.
- **A-25 Automation never exceeds the budget.** Automatic grabs go
  through `Grab` and so through the D-111 reservation and queue slots;
  a full budget pauses automation with a visible reason instead of
  failing grabs one by one.

Work plan (tests first, small commits): quality profiles + decision
engine → numbering map → monitored titles + wanted from the catalog →
blocklist + stall detection → RSS sync and scheduled search → upgrades
→ gateway routes → web (Wanted tab, Monitor action, profiles,
blocklist, automation settings).

### Phase 3 — subtitles (Bazarr role)

Starting point: Lain only knows subtitles *inside* media files (ffprobe
streams, extracted to WebVTT on demand, D-047). Subtitle files next to
the media are neither discovered nor offered to players, so a
downloaded subtitle would be invisible. Phase 3 therefore has two
halves: make sidecars playable, then acquire them.

Proposed decisions (`A-28`…, for review; none adds a dependency, none
touches `MetadataRecord` (D-032) or the frozen playback/transcode
shapes):

- **A-28 Sidecars are discovered by name, never by content scan.** A
  sidecar is a file in the media's folder named
  `<media basename>[.<lang>][.forced|.sdh|.hi].<srt|ass|ssa|vtt>`
  (the Plex/Jellyfin/Bazarr convention, which mpv also loads). The
  language tag may be ISO 639-1 (`en`), 639-2 (`eng`, `fre`/`fra`) or a
  region form (`pt-BR`); matching folds them to one ISO 639-2 code
  (D-071's form) through a built-in table. Unknown tags stay listed as
  `und`. Sidecars are not catalog items (the library scan ignores them,
  as today) — they are read from disk when asked.
- **A-29 Sidecars reach players through new, additive routes**, not
  through the frozen playback plan: `GET /api/items/{id}/sidecars`
  lists them for any signed-in user and `GET
  /api/items/{id}/sidecars/{n}` serves one as WebVTT (SRT and VTT are
  converted in Go; ASS/SSA dialogue is converted to plain-text cues,
  styling dropped). The web player adds them to its subtitle menu next
  to embedded tracks; mpv/VLC already load them by name.
- **A-30 Subtitle providers are a capability with config in the
  input**, the D-114 pattern: `lain.subtitle@1` (ordered-many,
  `unsupported-provider` declines). Provider accounts (API key,
  optional username/password) live in acquisition storage, travel in
  the contract input, are never logged or returned (`has_*` flags), and
  errors name the host only. First provider: `lain-subtitle-opensubtitles`
  (OpenSubtitles.com REST API, the operator's own API key; base URL is a
  setting so tests use a local fake). Other providers follow the same
  contract later.
- **A-31 Matching prefers the file's hash.** Lain computes the
  OpenSubtitles "moviehash" (size + 64 KiB head + 64 KiB tail, stdlib)
  and searches by hash and by parsed title/season/episode (or
  volume/chapter-less: subtitles are for video only). Ranking: hash
  match, then same release group, then same resolution/source, then
  provider downloads; hearing-impaired and forced variants follow the
  profile.
- **A-32 Wanted subtitle languages live on the quality profile**
  (`subtitle_languages`, ISO 639-2, ordered; empty = no subtitle
  automation), with `subtitle_skip_if_audio` (default on, the D-071
  rule: audio in the language makes subtitles unnecessary) and
  `subtitle_hi` (`include`/`prefer`/`exclude`). A language is satisfied
  by an embedded subtitle stream, an existing sidecar, or (with
  skip-if-audio) a matching audio stream — read with the existing
  `lain.media.probe@1` (ffprobe), never by guessing from names.
- **A-33 Sync check before placing.** A downloaded subtitle is parsed;
  it is refused when it has no cues, when its last cue ends more than
  10 % (and at least 2 min) after the media's duration (wrong episode,
  wrong cut, or a 25↔23.976 fps drift), or when its first cue starts
  after the media ends. Refusals are recorded like the acquisition
  blocklist (by provider file id) so automation does not retry them.
  No automatic retiming in v1.
- **A-34 Encoding is normalized to UTF-8 without a dependency.** Valid
  UTF-8 (BOM stripped) and UTF-16 with a BOM are decoded with the
  stdlib; anything else is read as Windows-1252 (a superset of Latin-1
  that covers most legacy SRT); other legacy encodings would need
  `golang.org/x/text` — a dependency decision, not taken.
- **A-35 Placement and ownership.** A downloaded subtitle is written next
  to the media as `<basename>.<lang 639-1 or 639-2>[.forced|.sdh].<ext>`
  with the same never-overwrite rule as media (D-117); an existing name
  is kept and the new file gets no second copy. Lain records the
  sidecars it wrote (`acq_subtitles` ledger: path, provider, file id,
  language, hash match). Replacing or removing a sidecar moves the old
  one to the D-128 holding folder — library files, sidecars included,
  are never deleted automatically (D-112).
- **A-36 Subtitle automation reuses Phase 2.** For files of monitored
  titles: after each import, and every `subtitle_hours` (default 24, 0
  = on demand), wanted languages are searched and the best accepted
  subtitle per language is downloaded. Manual search and download work
  for any video item. Admin only, like all acquisition (D-116).
- **A-37 Real-ffmpeg tests stay behind `-tags e2e`.** Probe-dependent
  behavior is tested with an injected probe; a gated e2e test checks the
  real ffprobe path on a generated clip.

Work plan (tests first, small commits): subtitle formats (parse SRT/
VTT/ASS, convert to WebVTT, cue timing, charset normalization) →
sidecar naming and discovery → moviehash → `lain.subtitle@1` contract
and the OpenSubtitles provider against a fake server → acquisition
store and logic (wanted languages via probe, search, rank, sync check,
placement, ledger, automation) → gateway routes (sidecars for players,
admin subtitle routes) → web (player menu, providers and per-title
subtitle status).

## API (Phase 1, all admin)

| Route | Purpose |
|---|---|
| `GET/POST /api/acquire/indexers`, `PATCH/DELETE /api/acquire/indexers/{id}`, `POST /api/acquire/indexers/{id}/test` | indexer CRUD + health |
| `GET /api/acquire/search?q=&kind=&season=&episode=` | search every enabled indexer, parsed + ranked |
| `GET /api/acquire/parse?name=` | parser diagnostics |
| `GET/POST /api/acquire/grabs`, `POST /api/acquire/grabs/{id}/{pause,resume,import}`, `DELETE /api/acquire/grabs/{id}?data=1` | queue |
| `GET/PUT /api/acquire/settings` | engine + seeding + import settings |

## Status (2026-10-04) — Phase 1 built, not merged

- `internal/torrent`: stdlib BitTorrent engine (bencode, metainfo,
  magnets, HTTP/UDP trackers, peer wire, ut_metadata, rarest-first
  picking with endgame, SHA-1 verification, resume/recheck, choking and
  seeding, rate limits). Fuzzed: bencode, metainfo, tracker responses,
  live peer messages. Loopback swarm tests incl. a corrupt seeder.
- `lain.release.parse@1` (model over parser.sock, tokenizer fallback),
  `lain.indexer@1` (Torznab/Newznab).
- `internal/acquire`: indexers with write-only keys and rate limits,
  parallel search with parse + rank, grabs from results/.torrent
  URLs/magnets (incl. indexer redirects to magnets), shared disk budget,
  queue slots, import with identifier-round-tripped naming and
  hardlink/copy/move, seeding to ratio/time then cleanup, restart resume.
- Gateway `/api/acquire/*` (admin), web `/acquire` (Search, Queue),
  SERVER › Indexers (`g n`) and › Acquisition (`g a`).
- Verified in headless Chromium against an isolated server and a
  loopback swarm: keyboard-only search → grab → download → import →
  seeding, catalog shows the episodes.

Left in Phase 1 scope:

- No DHT/PEX/uTP/encryption (A-2): trackerless magnets are refused.
- No usenet client (A-16).
- qBittorrent/Transmission adapters (the interface is ready, A-3).
- `/settings/downloads` is missing from `ADMIN_ROUTES` on the reading
  branch (pre-existing; not changed here).

## Status (2026-10-04) — Phase 2 built and accepted (D-124…D-134)

Accepted: A-1…A-16 (`D-108`…`D-123`), including the cleanup condition
(`cleanup_test.go`). Phase 1's mobile Acquire entry is done.

Phase 2 built on `feat/acquisition`:

- Quality profiles and one decision engine (`decide.go`), monitored
  titles with absolute/seasonal/chapter/volume/movie numbering, season
  maps, wanted units from the catalog plus the cached metadata total
  (`monitored.go`), RSS sync, on-demand and scheduled missing search,
  greedy best-first grabbing without double grabs, upgrades with held
  replacements and rollback, the quality ledger, blocklist on download
  failure, stall and import failure, budget pause (`automation.go`).
- Admin API (`gateway/acquire_auto.go`) and web: Wanted (3) and
  Blocklist (4) tabs, `m` on a search result to monitor it, automation
  settings and quality profiles in Settings › Acquisition.
- Verified in headless Chromium against an isolated server and a
  loopback swarm: search → monitor (`m`, Enter) → search now (`s`) →
  monitored grab of two episodes → import → wanted updated (and the
  metadata total, 28, extending the gap-fill).

Left / needs a decision:

- Per-season episode lists and air dates need a `MetadataRecord` change
  (frozen by D-032).
- DHT, usenet and external client adapters stay as decided (D-108,
  D-110, D-123).
- Phase 3 (subtitles) is planned only.
