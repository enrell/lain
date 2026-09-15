# Contracts

Versioned capability names. A name is not a credential; serving rights
come from the composition.

## lain.source.enumerate@1 (exactly-one)

Input: `{root, library_id, type}` → Output: `Candidate[]`.

Discovers files only. Walk errors are counted, never fatal to a scan.

## lain.media.identify@1 (ordered-many)

Input: `Candidate` → Output: `Proposal{kind,title,season,episode,year,
confidence,evidence,plugin_id}`.

First `Accepted()` proposal (non-empty title, confidence > 0) wins.
Built-ins: `lain-identify-anime` (declines without release evidence),
`lain-identify-generic` (always accepts, confidence 0.4).

## lain.catalog.read@1 / lain.catalog.write@1 (exactly-one)

Write input: `{library_id, proposal, candidate}` → `CatalogItem` with
`origin` + `provenance`. Item id is stable over library+path. Read:
`nil` → list, `{id}` → one item.

Portable document `lain.catalog-export@1` is the replacement contract.

## lain.userstate.progress@1 (exactly-one)

`PutInput{user_id, progress}` / `GetInput{user_id, item_id}`. Keyed by
user+item in its own document; catalog rewrites never touch it.

## lain.playback.plan@1 (first-accepted)

Input: `{request{item_id, client, network}, file_path}` → `Plan{mode,
asset, available, reason?}`. `asset` is opaque (`asset:<id>`); the
gateway resolves it. mpv/desktop clients always direct-play; browser
clients get `direct` for web containers and `transcode` (playable
through the transcode endpoint) for the rest. Without a healthy
transcode provider the gateway downgrades those plans to
`transcode-required` with `available:false` instead of faking a
stream.

## lain.search.query@1 (exactly-one)

Input: `{q, kind, limit, offset, sort}` → `CatalogPage{items, total,
limit, offset}` (defaults 50 / cap 500 / title|recent). v0.1 is
case-insensitive substring over titles; ranking policy is the
replaceable unit. Catalog reads (`Page`) share the same envelope and
also accept a `library_id` scope from the gateway (`PageParams`), so a
library view never filters client-side.

## lain.ingest.scan@1 (exactly-one)

Input: `{libraries[]}` → `ScanStats{libraries, candidates, identified,
unidentified, errors, ...}`. Fixed order: enumerate → identify →
catalog write. Unidentified files count, never abort.

## lain.playback.transcode@1 (exactly-one)

Input: `{file_path}` → `Transcode{path, method?, cached?}`. The ffmpeg
provider (`lain-transcode-ffmpeg`) prepares a browser-playable MP4 on
disk: stream-copy remux when the source is already H.264/AAC, else a
`veryfast` H.264/AAC re-encode with `+faststart`. First video track
plus all audio tracks are kept; subtitles are dropped in this slice.
Cache key is source identity (path, mtime, size) plus output profile;
at most one ffmpeg transcode runs at a time. The gateway serves the
file at `GET /api/items/{id}/transcode` with Range support. No
ffmpeg? Health fails, the endpoint 503s and plans downgrade.
Kept as the synchronous compatibility contract.

## lain.playback.transcode@2 (exactly-one)

Async variant over the same worker and cache. Actions: `inspect` and
`status` are side-effect free; only `start` enqueues work. States:
`idle|queued|running|ready|failed`. The session is an opaque,
deterministic key over source identity, the `web-mp4-sdr-v2` profile
and selected stream indices. One global worker, bounded FIFO queue
(8); duplicate sessions converge on one job. HTTP: `POST
/api/items/{id}/transcode` starts/joins (202 while pending, 200 when
ready, 429 `queue-full`), `GET .../transcode/status?session=` polls
without filesystem paths, and `GET .../transcode?session=` resolves
the ready MP4 (202 + `Retry-After` while pending). `GET
/api/items/{id}/playback` reports additive `profile`, `session` and
`state` but never starts work.

Ready artifacts live in a plugin-private 20 GiB LRU cache (default;
`--transcode-cache-size` / `LAIN_TRANSCODE_CACHE_SIZE`) with atomic
JSON sidecars tracking identity, method, size and last access.
Cleanup runs at startup and around admission/completion; stale-source
and abandoned-temporary entries are removed, active jobs excluded. A
single artifact larger than the quota is kept as the sole entry with
a warning.

Stream policy is probe-driven (`lain.media.probe@1`): copy compatible
H.264 video, encode other SDR video (including HEVC/AV1) to
H.264/yuv420p, keep one audio track (explicit selection, else default,
else first; AAC copied, others to AAC). HDR is detected and refused
with `unsupported-media` until a tone-map profile exists. Convertible
text subtitles (`SRT`/`ASS`/`SSA`/WebVTT/`mov_text`) are extracted to
WebVTT sidecars served at `GET /api/items/{id}/subtitles?session=`
(`text/vtt`); bitmap/styled tracks report unavailable, and ASS loses
styling. PGS/VobSub and burn-in stay out.

## lain.media.probe@1 (exactly-one)

Input: `{file_path}` → `MediaInfo{format, duration?, streams[]}`.
ffprobe-backed, bounded metadata only (index, type, codec, profile,
pixel format, dimensions, channels, language, title, default/forced,
HDR color markers, subtitle convertibility). Never carries bytes.
Absence of ffprobe degrades browser planning to the conservative
extension fallback; the v2 transcode profile requires it.

## Declared (next slice)

`lain.sync.*@1`. Names are reserved here so first implementers do not collide.
(`lain.transform.thumbnail@1` and `lain.playback.transcode@1` were
implemented from this reserved family; see below.)

## lain.transform.thumbnail@1 (exactly-one)

Input: `{file_path, time_sec, width}`. `file_path` is supplied by the
gateway after catalog lookup; `width` is clamped to 32..1280. Output:
`{path, width, cached}` pointing at a JPEG in the provider's cache —
bytes never enter the call. Keys include source mtime/size, timestamp
and width, so changed files regenerate without invalidation bookkeeping.
A seek past the end retries at 0. The gateway streams the file with
`Cache-Control: private, max-age=86400` and `GET /api/items/{id}/thumbnail?t=&w=`
accepts `?token=` (image elements cannot attach headers). ffmpeg is
spawned by the provider with a 20s timeout and at most 2 concurrent
processes; absence degrades this capability, never the boot.

## Metadata (merge-many)

`lain.metadata.search@1`: `{query, kind, limit, dir}` →
`MetadataCandidate[]` per provider (provider, remote_id, title,
synonyms, year, poster). `dir` hints local sources; remotes ignore it.

`lain.metadata.resolve@1`: `{provider, remote_id}` → `MetadataRecord`
(full entry; artwork as URLs, never bytes).

The gateway fans out (`Registry.CallMerge`), dedups by normalized
title, and scores exact matches first, then binding precedence
(`nfo → kitsu → anilist → jikan → tvmaze`). TVMaze is keyless and
TV-first: it answers series/episode/video/anime kinds and stays
silent for movies. TMDB and IMDb are deliberately out of this slice
(TMDB needs a user-supplied key, IMDb has no free official API;
see Q-010/Q-011). Failing providers are skipped, so
an upstream outage degrades the merge instead of failing it. Winners
resolve through `Registry.InvokeProvider` (still through authority)
and persist as overlays (`POST /api/catalog/{id}/enrich`), never
inside the catalog: removing a provider deletes its overlays without
touching identity, progress or files. Repeat queries hit a TTL cache
(search 7d, records 30d) instead of the network. Grids read many
overlays at once via `GET /api/enrichments?ids=` (bounded, one read
transaction); missing entries are simply absent.

Operator surfaces can list registered providers with their
capabilities and live health (`provider_info` on `GET /api/plugins`) so
a swap UI only ever offers candidates that can serve the capability.
