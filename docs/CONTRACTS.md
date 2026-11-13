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

The gateway resolves the catalog only through these bindings (D-075):
a swapped provider serves every read and write below.

Read inputs: `nil` → all items; `GetInput{id}` → one item;
`PageInput{page}` → `CatalogPage`; `EpisodesInput{id}` → every item
sharing the title's key in watch order. Unknown ids fail with the
typed code `not-found`.

Write inputs: `UpsertInput{library_id, proposal, candidate}` →
`CatalogItem` with `origin` + `provenance`;
`CommitScanInput{library_id, entries, reconcile_missing}` commits one
library's scan in a single call — the provider derives identity,
reconciles moves (D-019) and, only when `reconcile_missing` asserts a
clean walk, marks absent items missing (D-068) — returning
`{migrated, missing, restored}`; `SetMissingInput{id, missing}` →
whether the item existed; `DeleteLibraryInput{library_id}` → removed
count. Item id is stable over library+path.

`CatalogItem.missing` (additive, `omitempty`) marks an item whose file
vanished from its library root (D-068). Missing rows stay in the
catalog — list, search, detail and episode-grouping reads all return
them — because the stable id keeps progress and enrichment reattaching
when the same path comes back. A clean rescan of the library clears
the flag. Missing is a state, never a deletion: progress and
enrichment documents are never swept.

Portable document `lain.catalog-export@1` is the replacement contract.

`CatalogItem.thumbnail` (D-282, additive, omitted when absent) says what
`GET /api/items/{id}/thumbnail` serves the item without `t`, so a client
lays out its card before the picture loads: `{source, aspect?}`, where
`source` is `sidecar` (a still that came with the file, D-269), `cover` (a
comic archive's cover page, upright) or `frame` (cut from the video), and
`aspect` is width over height to three decimals when the server read it
from the picture's header (a sidecar's; a frame's is the video's and is
not known). The gateway fills it on `GET /api/catalog`,
`/api/catalog/{id}`, `/api/catalog/{id}/episodes` and `/api/search` on
every read; it is never stored or exported, and a catalog provider never
sets it. An item with no resolved `file_path` has none.

### Pages of titles (D-283)

`GET /api/catalog` and `GET /api/search` take `group=title` (additive;
any other value pages files as before). The page then counts titles, not
files: the files that share a title key (the episodes rule below) are one
title, placed where its first file stands in the sort (`recent`: its
newest file; `title`: its first by title), and `limit`, `offset` and
`total` count titles. `items` holds every file of each title on the page,
in watch order, after the page's own filters (library, query, kind), so a
title is never split across pages and a card counts all of its episodes.
The envelope says `group: "title"`; a page without it (an older server, a
catalog provider that does not group) is a page of files, and a client
falls back to grouping it itself. `PageParams` carries it to the catalog
provider as `Group`. Case: `scan/25-titles.json`.

### Gateway read: one title's files

`GET /api/catalog/{id}/episodes` (auth) → `{items: CatalogItem[]}`, 404
for an unknown id. It answers the web title page: every item whose
`title` normalizes to the same key — lowercase, whitespace collapsed —
in watch order (season, episode, year, id), whichever library it lives
in. `catalog.TitleKey` is the one grouping rule, mirrored by the web
Library grid's `normalizeSeriesTitle`, so a show's card and its own page
cannot disagree about the episode count (D-056). The gateway answers it
through `lain.catalog.read@1`'s `EpisodesInput`, so a swapped catalog
serves the title page too (D-075).

The web UI renders `/item/{id}` as the *title* page (hero, poster rail,
season filter, episode grid; a single-file title keeps the plain item
page) and `/player/{id}` as the watch page — video with the title's
episodes listed beside it, so switching never leaves playback.

### External player handoff

Web links use `lain://play?server=<origin>&id=<item-id>&player=mpv|vlc`.
This is a local CLI protocol, not a server API. It contains no token. A
user-installed `lain` handler accepts only the exact server origin saved
by `lain login`, fetches the item through the authenticated catalog API,
and opens the existing direct stream. The CLI reports measured progress
through the existing item progress endpoint. It never assumes a VLC exit
means the file was watched.

`lain watch` and the web handoff load the ordered catalog episode list
for a title (D-056) and give the player one playlist from the chosen
episode onward. Next/Previous work within that playlist in one player
process. Each entry is associated with its own existing progress record.
`--once` keeps single-item playback. The player preference is local to
each CLI installation or browser account storage; the link still names
the selected player explicitly.

### Local playback

When the browser and a natively running server share a machine, the
server launches the player itself (D-072) — no handler install and no
second login, because nothing crosses a process boundary the server
does not already own.

`GET /api/localplay` → `{"players":["mpv","vlc"]}` lists the player
binaries usable right now: present in PATH plus a graphical session
(`DISPLAY`/`WAYLAND_DISPLAY`). Remote peers, forwarded requests and
Docker deployments get an empty list.

`POST /api/items/{id}/play-local` with optional `{"player":"mpv|vlc"}`
→ `200 {"status":"playing","player":"mpv","count":N}`. The peer must be
loopback and the request must carry no `Forwarded`, `X-Forwarded-For`
or `X-Real-IP` header — otherwise 403. Other typed failures: 401
unauthenticated, 400 unknown player, 404 unknown item, 409 missing
file (D-068) or playback already running, 503 no usable player.

Playback opens local file paths; no token rides a URL. A series plays
the ordered episode list from the chosen episode onward (D-056), and
measured progress lands in userstate under the authenticated account —
the same shape `PUT /api/items/{id}/progress` writes. One local
playback runs at a time and server shutdown stops the player.

### Account playback language

`GET /api/me` includes optional `preferred_language` on the public
user object. `PATCH /api/me/preferences` accepts
`{"preferred_language":"por"}` (or `""` to clear it) and returns the
updated public user object. Only the authenticated account can update
its preference. Values are lowercase three-letter ISO 639-2 codes;
input is normalized to lowercase, and other shapes return 400.
Existing clients ignore the optional field. The field does not change
password version or playback limits (D-015/D-071).

The preference resolves each media item independently: matching audio
wins and subtitles are disabled; otherwise a matching subtitle is
selected when the client can render it. Missing language metadata
leaves the native player selection in control. Explicit player track
changes override the initial automatic selection for that item.

**Audio and subtitles apart (D-280).** The public user object adds three
optional fields, each absent when empty:

- `preferred_audio_language` — ISO 639-2; absent plays the file's
  default audio.
- `preferred_subtitle_language` — ISO 639-2; absent when none is
  preferred.
- `preferred_subtitle_mode` — `always`, `forced` or `off`; absent is
  `auto`.

`PATCH /api/me/preferences` takes any subset of `preferred_language`,
`preferred_audio_language`, `preferred_subtitle_language` and
`preferred_subtitle_mode` (`""` resets the mode to `auto`); only the
fields present change, and a refused value (400, `<field> must be a
three-letter ISO 639-2 code` or `preferred_subtitle_mode must be auto,
always, forced or off`) changes nothing. An empty patch is still 400
`preferred_language is required`. The mode is named apart from
`playback.subtitle_mode` on the same object, the operator's delivery
policy (`auto|extract|burn|off`), whose values mean other things.

Compatibility. `preferred_language` stays in requests and responses:

- Written alone (an older client), it sets both languages and leaves
  the mode as it is.
- Answered, it is the subtitle language when one is set, else the audio
  language: the language the viewer reads, so an older client applying
  D-071 still turns subtitles on for audio in another language.
- An account saved before D-280 has none of the three fields and reads
  as its one language for both, mode `auto`, which is D-071 exactly; the
  server answers it with both split fields filled. A client talking to a
  server before D-280 sees none of the three and does the same.

How players choose (every client and the server's own players apply
the same rule, `lain-watch`'s `tracks.rs`):

- **Audio**: a track in the audio language, else the file's default
  (flagged default, else first).
- **`auto`**: subtitles in the subtitle language when the audio that
  plays is not in it, none when it is; a full track before a forced one.
  With no subtitle language, or no language tag on any audio or
  subtitle track, the player's default stands.
- **`always`**: subtitles in the subtitle language even over audio in
  it; with no subtitle language, the file's default (else first) full
  subtitle track.
- **`forced`**: only a track flagged forced, in the subtitle language,
  else in the language of the audio that plays, else an untagged one.
- **`off`**: none.
- A choice made in a player for a title (the web player and the Android
  app keep one per title on the device) wins over the account.

On the server: local playback hands mpv all three (`lain-audio_language`,
`lain-subtitle_language`, `lain-subtitle_mode` script options) and VLC
`--audio-language`, `--sub-language` plus a per-file "subtitles off". A
transcode start without `audio_stream` takes the preferred-language
audio track when the file's default is another (one ffprobe, only when
the account has an audio language), and the plan's `session`/`state`
describe that same session. A transcode never adds subtitles nobody
asked for: clients choose them, so they are not delivered twice.

## lain.userstate.progress@1 (exactly-one)

`PutInput{user_id, progress}` / `GetInput{user_id, item_id}` /
`ListInput{user_id}` → all of one user's progress. Keyed by user+item
in its own document; catalog rewrites never touch it.

## lain.playback.plan@1 (first-accepted)

Input: `{request{item_id, client, network}, file_path}` → `Plan{mode,
asset, available, reason?}`. `asset` is opaque (`asset:<id>`); the
gateway resolves it. mpv/desktop clients always direct-play; browser
clients get `direct` for web containers and `transcode` (playable
through the transcode endpoint) for the rest. An MP4/M4V direct-plays
only when its video is H.264 yuv420p and its audio is AAC or MP3 —
AC-3/E-AC-3 are excluded because Chromium on Linux cannot decode them
(D-030's "verified web-safe streams"). A per-account bitrate cap
(or the server-wide remote limit) downgrades a `direct` plan to a
capped `transcode` when the probed source bitrate exceeds it. Without a
healthy transcode provider the gateway downgrades those plans to
`transcode-required` with `available:false` instead of faking a
stream.

**Media length.** The gateway adds `duration_sec` to the plan response:
the probed length of the file, 0/absent when no probe ran (mpv/desktop)
or when the probe reported none. It is the same fact Jellyfin's
`PlaybackInfo` carries as `RunTimeTicks`, and it exists because an HLS
session cannot express its own total: an `EVENT` playlist lists only the
segments ffmpeg has already written, so hls.js sets the MediaSource
duration to the produced edge (`buffer-controller.getDurationAndRange`)
and, per MSE §10.1, `seekable` becomes `[0, that edge]`. The player
hands hls.js the real length instead (`attachMedia(media, {overrides:
{duration}})`), which makes the seek bar span the episode and a seek
past the produced edge rebuild the session at the target through
`start_sec` — the same mechanism a resume and a track change use. The
probe result is the gateway's, so `lain.playback.plan@1` and the plugin
shape are untouched (D-057).

**Client-reported decode capability.** The same endpoint accepts an
optional `caps` query parameter: a comma-separated list of the tokens a
client has verified it can decode (D-058). The plan is requested before
the client knows the file's streams, so the client reports a vocabulary,
not a verdict, and the planner applies it to the probed streams. Two
token shapes, lowercase:

| token | meaning |
|---|---|
| `mkv` | the container opens |
| `mkv/h264` | that codec family decodes inside that container |

The pairing is load-bearing. A browser can decode HEVC through a
platform decoder inside MP4 and still paint nothing for HEVC inside
Matroska — measured as no error event, an advancing clock, a 0x0 video
size and zero decoded frames — so a codec token is only ever claimed for
a container the client also opened, and the planner requires the
container *and* every track a player would select (first video, default
audio) to be covered. Containers: `mp4`, `mkv`, `webm`, `ogg`, `mp3`,
`flac` (extensions normalize onto them: `m4v`/`mov`/`m4a` are `mp4`).
Codec families: `h264`, `hevc`, `vp8`, `vp9`, `av1`, `aac`, `mp3`,
`opus`, `vorbis`, `flac`, `ac3`, `eac3`, `dts`. Anything else is dropped
on the way in, so a client cannot widen the set the planner reasons
about; and a codec with no family (`pcm_*`, MPEG-4 part 2) can never be
claimed, so it always remuxes or transcodes.

An absent `caps` means *unknown* and reproduces the pre-D-058 rules
exactly, which is what an older client, `curl` and the desktop send. A
present but empty `caps=` means the client claims nothing, which is a
decision: it transcodes.

A claim cannot raise the floor: H.264 High 10, 4:2:2 and 4:4:4 have no
browser decoder, and a codec string cannot express that — `canPlayType`
answers "probably" for the profile bits it does not parse — so a probed
non-8-bit H.264 pixel format refuses direct play whatever the client
claims. VP9 and AV1 do decode 10-bit, so the check is H.264-only.

A claim is trusted no further than the plan decision. On a `direct` plan
the player demands a decoded picture (`videoWidth > 0` and, where the
browser reports it, `totalVideoFrames > 0`) within ~3.2s and starts the
prepared path at the viewer's position when none arrives, saying so in
the UI. A browser whose decoders change mid-session, and the first
direct play of a file the browser refuses, therefore cost one extra
round-trip — never a permanent black frame. Measured end to end on this
host: a direct-played Matroska seek lands in ~126 ms (one Range request)
where a session rebuild costs ~2.6s.

## Catalog population in lain-rs (D-185..D-189)

`lain-rs` serves the scan pipeline in-process with the Go op names
(`enumerate`, `identify`, `scan`, `commit_scan`, `poll`/`dirty`/`close`,
`search`, `resolve`, `enrich`/`get`/`batch`/`delete`/`backfill`) and
`parse` for `lain.release.parse@1`. Rust-only, additive fields: a
`commit_scan` input carries the library's `root`, and each `ScanEntry`
its `rel_path` inside that root (D-157). A rooted item's location is
`rel_path`, an unrooted one's is `file_path`; the item id is the first
16 hex digits of SHA-256(`library_id` NUL location) and `aliases` hold
older locations (D-186). `GET /api/events` adds `scan-state` and the
`library-changed` actions `scanned` and `item-deleted` (D-188).

## lain.search.query@1 (exactly-one)

Input: `{q, kind, limit, offset, sort}` → `CatalogPage{items, total,
limit, offset}` (defaults 50 / cap 500 / title|recent). v0.1 is
case-insensitive substring over titles; ranking policy is the
replaceable unit. Catalog reads (`Page`) share the same envelope and
also accept a `library_id` scope from the gateway (`PageParams`), so a
library view never filters client-side.

## lain.ingest.scan@1 (exactly-one)

Input: `{libraries[]}` → `ScanStats{libraries, candidates, identified,
unidentified, errors, missing, restored, ...}`. Fixed order: enumerate
→ identify → catalog write. Unidentified files count, never abort.

Reconciliation (D-068): at the end of a *clean* walk of a library —
root accessible, no walk errors — every stored item of that library
absent from the present set is marked `missing` (counted in
`stats.missing`) instead of being deleted; a missing item whose path
reappears clears the flag (`stats.restored`). An inaccessible root or
a dirty walk marks nothing, so an unmounted drive cannot empty the
catalog. Identification failures are never proof of disappearance: an
enumerated-but-unidentified path still counts as present, so an
already-cataloged item at that path keeps its state. The gateway runs
the same pipeline per-library from an `fsnotify` watcher (debounced,
serialized with manual scans through the scan lock, reported as
`trigger: "watch"` on `GET /api/library/scan`), so a delete on disk is
reflected without waiting for a manual scan. Watching can be turned
off with `--watch=false` / `LAIN_WATCH=false` for filesystems inotify
cannot see.

While a scan runs, `GET /api/library/scan` carries `progress`
(D-272): `{phase, libraries, libraries_walked, libraries_done, files,
files_done, enrich_total?, enrich_done?}`. `phase` is the furthest step
reached and never goes back: `walk` (roots being listed), `identify`
(`files_done` of `files`; `files` grows until `libraries_walked`
reaches `libraries`, since roots walk in parallel), `commit`
(`libraries_done` of `libraries`) and `enrich` (the metadata backfill,
`enrich_done` of `enrich_total`). It is absent in every other state.

`POST /api/library/scan` (admin) answers 202 `{state: "running"}` when
it starts a scan. While one runs it queues one follow-up scan of every
library instead of refusing (D-286; it answered 409 before): 202
`{state: "running", queued: true}`, and `GET /api/library/scan` carries
`queued: true` until the running scan ends and the follow-up starts by
itself. Asked again while queued, it stays one follow-up. The running
scan walks only the libraries it started with, so a library added
meanwhile is read by the follow-up.

## lain.playback.transcode@1 (exactly-one)

Input: `{file_path}` → `Transcode{path, method?, cached?}`. The ffmpeg
provider (`lain-transcode-ffmpeg`) prepares a browser-playable MP4 on
disk: stream-copy remux when the source is already H.264/AAC, else a
`veryfast` H.264/AAC re-encode with `+faststart`. First video track
plus all audio tracks are kept; subtitles are dropped in this slice.
Cache key is source identity (path, mtime, size) plus output profile;
at most one ffmpeg transcode runs at a time. The gateway serves the
file at `GET /api/items/{id}/transcode` with Range support. No
ffmpeg? Health fails, the endpoint 503s and plans downgrade. The v1
path carries no per-session options, so the gateway enforces the
account's playback policy there: an account denied transcode+remux, or
one whose bitrate cap it cannot honour, gets `403 forbidden` and must
use the session API. Kept as the synchronous compatibility contract.

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

Pending status payloads carry an additive `progress` (0..1) — the
fraction of the source already prepared — while work is queued or
running; the terminal states omit it, and the server never promises an
estimate it cannot keep honest, so any ETA is the client's own (D-039).

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

## lain.playback.transcode@3 (exactly-one)

Full browser pipeline under operator policy (D-042..D-045); `@1`/`@2`
are retained unchanged. The operator settings travel inside the
request, so the plugin never reads a bucket or holds HTTP state
(D-006/D-045).

Actions: `inspect` (side-effect free), `start` (the only action that
enqueues), `status`, `resolve` (trusted-plane cache access that also
marks LRU recency), `position` (the segment the client last
fetched — drives throttling, segment deletion and idle cleanup; the
latest report wins, so a rewind re-pauses ffmpeg and holds off deletion
instead of pacing both against a stale high-water mark),
`cancel` (with a `viewer`, a release: see **Viewers** below), and
`list` (admin view). States remain
`idle|queued|running|ready|failed`.

Request fields: `delivery` (`hls` default, or `progressive`),
`quality` (a ladder name from the settings), `max_bitrate_kbps`,
`video_codec` (`h264` default, `hevc`/`av1` only when the operator
allows them), `audio_codec`, `audio_stream`, `subtitle_stream`,
`subtitle_mode` (`auto|extract|burn|off`), `segment_index`, `policy`
(the requesting user's limits), `viewer` (D-281, never part of the
session identity) and `settings`.

`video_copy_codecs` (D-273, optional) lists the codecs beyond H.264 the
client decodes in fragmented MP4. Only `hevc` is known; other values
are ignored. With it, an SDR HEVC source (4:2:0, 8 or 10 bit) is stream
copied under the rules H.264 already copies by (no burn-in, no
downscale, inside the bitrate limit, stream copy not refused) and
tagged `hvc1`; the status then reports `video_codec: "hevc"`,
`video_direct: true`. It is not `video_codec`, which names the encoder
target and stays behind `allow_hevc`: a copy encodes nothing, so the
operator setting does not gate it. HDR sources are still tone-mapped,
and the MPEG-TS segment container never carries the copy. The claim is
part of the profile key only when it names `hevc`.

`video_effect` (D-274, optional) asks the server to render a video
effect into the session: `anime4k-a`, `anime4k-aa` or `anime4k-lite`,
the web player's presets, run by ffmpeg's `libplacebo` filter with the
player's own shader files. `GET /api/playback/options` lists what this
server renders in `effects` (empty when libplacebo cannot run here; the
admin capabilities say the same in `video_effects`). With an effect the
video is always encoded, the two upscaling presets produce twice the
source up to 1920x1080 (never past a `quality` box, never below the
source), `width`/`height` report that frame and `reasons` gains
`video effect (<preset>)`. An unknown preset is `invalid-message`. A
known one this session cannot carry — no libplacebo, an HDR source, an
image subtitle burned in, a VA-API encode — is not an error: the
session is the plain one and `fallback` says why. The preset is part of
the profile key when set.

Status fields: `delivery`, `playable` (HLS: a playlist with at least
one segment exists), `encoder`, `hardware`, `fallback` (why software
was used instead), `reasons` (why the transcode is needed at all),
`video_codec`, `audio_codec`, `width`, `height`, `bitrate_kbps`,
`has_subtitle`, `progress`, `fps` and `output_bitrate_kbps`, plus the
`@2` fields. `path`, `playlist_path` and `subtitle_path` are
trusted-plane values that never reach a client.

Session identity (and therefore the cache key) is the profile key
`web-mp4-v3-<digest>` over every setting that changes the produced
bytes: delivery, quality (name **and** the resolved ladder entry),
codecs, subtitle mode, audio bitrate/VBR/downmix (boost and algorithm),
general and per-codec preset/CRF, bitrate cap, hardware backend, encode
and low-power, the 10-bit decode flags and the decode codec list, the
ffmpeg binary path, tone-mapping policy, deinterlace, HLS segment
seconds and container, burn-in font path/name, and the selected stream
indices. Operational knobs (thread count, muxing queue, temp path,
cache/queue bounds, VA-API render device) stay out. Any policy change
that alters output changes the key, so stale derivatives never match
(D-030/D-045).

The cache is keyed by source and options, not by account, so the
requesting user's `policy` is re-checked when a cached derivative is
served and when joining a live session: a restricted account is never
handed another account's remux or transcode.

**Delivery.** `hls` writes segments plus a server-owned `index.m3u8`
(`EVENT` playlist, `ENDLIST` when ffmpeg finished), so playback starts
while ffmpeg still runs. A `#EXT-X-DISCONTINUITY` ffmpeg wrote is
preserved: it is attributed to the segment that *follows* it and
re-emitted between segments, so a client resets its timeline where the
source broke (the tag is dropped when segment deletion moves that
boundary to the front of the playlist, where nothing precedes it).
`hls_segment_container` picks the segment
format: `fmp4` (the default, CMAF with a `#EXT-X-MAP` init segment and
`segNNNNN.m4s` files) or `mpegts` (classic `segNNNNN.ts` with no init
segment). `progressive` keeps the complete `+faststart` MP4 with Range
support. `subtitle` is not a transcode: it is the sidecar-only entry an
on-demand subtitle extraction produces (see below).

`start_sec` begins a session at a source position (a resume or a seek):
ffmpeg seeks the input (`-ss`) and the produced timeline starts at zero,
so a client maps its clock by adding `start_sec` back. It is part of the
session identity, so a different start is a different session.
Mid-session seeking is not a session option: seeking backwards past the
session start needs a new session (the player rebuilds one on a track or
quality change).

**Throttling** pauses ffmpeg (SIGSTOP) once production runs more than
`throttle_ahead_sec` ahead of the client's fetched segment and resumes
it at half that budget; **segment deletion** removes segments fully
behind the client minus `segment_keep_sec` and rewrites the playlist;
**idle timeout** stops sessions nobody has fetched from for
`idle_timeout_sec`. All three are settings, defaulting to throttle on,
deletion off.

**Hardware acceleration** is opt-in only (D-031): the configured
backend is probed with a real one-frame encode, hardware decoding
applies only to the allowed codec list, and a backend that fails its
probe — or fails at runtime — falls back to software with a visible
`fallback` string. An HLS session retried in software after a runtime
failure starts over: the failed attempt's playlists and segments are
removed first and `playable` is false again until the retry's first
segment, so no segment of one encoder is ever served beside the other's
init segment. The retry writes `sw-init.mp4` and `sw-segNNNNN.*`, never
a name the failed attempt used: a player keeps an init segment whose
URI did not change, and a browser caches the segments it fetched, so a
viewer of the failed attempt would otherwise decode one encoder's
segments with the other's init segment. Software encoders stay the
default; there is no silent opportunistic switching. The backend list
mirrors Jellyfin's documented methods: `vaapi`, `nvenc`, `qsv`, `amf`,
`v4l2m2m`, `videotoolbox` and `rkmpp` (Rockchip). `hardware_low_power` selects
QSV's low-power H.264/HEVC encoder (Jellyfin's "Intel Low-Power
hardware encoder"); only QSV honours it.

**Per-stream permissions.** `allow_video_stream_copy` and
`allow_audio_stream_copy` mirror the client-side flags of Jellyfin's
`PlaybackInfo` request: an explicit `false` forces a re-encode of that
stream even when a copy would be legal, and the plan records the reason.
Both are part of the session/cache identity, so the forced encode never
collides with the remux. The status reports the outcome per stream as
`video_direct`/`audio_direct` (Jellyfin's `TranscodingInfo`
`IsVideoDirect`/`IsAudioDirect`).

**Subtitles.** Two paths, one endpoint:

- `GET /api/items/{id}/subtitles?session=` serves the WebVTT a transcode
  session produced.
- `GET /api/items/{id}/subtitles?stream=N` extracts track `N` on demand
  and caches the sidecar, so a file that plays **directly** still gets
  subtitles without being transcoded (Jellyfin's "allow subtitle
  extraction on the fly"). Text codecs convert; image codecs are refused
  with `unsupported-media`. The server-wide
  `allow_subtitle_extraction` setting (nil/absent = on) can turn this
  off, and then the endpoint answers `forbidden`. A sidecar-only entry
  is a normal cache entry with delivery `subtitle`: it is evicted by the
  same LRU, its staleness is judged on the `.vtt` alone, and it carries
  `playable:false` and `method:"extract"`.

The ready-cache LRU never evicts a session fetched in the last minute: a
ready HLS session is still playable, so deleting its segments mid-playback
would break the client. While a viewer holds a session the cache may
exceed `cache_bytes`; the overage is logged rather than resolved by
pulling the file away.

**Tone mapping** converts HDR to SDR with `zscale`+`tonemap`
(`hable|reinhard|mobius|clip|linear`, `npl` from
`tone_mapping_peak_nits`) or `libplacebo` BT.2390 when the Vulkan probe
passes; `bt2390` without a working probe degrades to `hable` and says
so. Tone mapping off, or mode `never`, keeps HDR honestly unavailable.
Hardware (VPP) tone mapping — Jellyfin's `tonemap_vaapi`/`vpp_qsv` — is
**not implemented**: HDR is always tone-mapped in software, which yields
correct SDR output at the cost of speed. (Deliberate gap, D-054: the
local `vpp_qsv` exposes no `tonemap` option, and `tonemap_vaapi` needs
VA-API device plumbing that cannot be validated without a device;
shipping unverified filter syntax would be worse than the honest
software path.)
**Deinterlace** applies the configured filter (`deinterlace_method`:
`yadif`, the default, or `bwdif`, which degrades to `yadif` with a
visible fallback note when the build lacks it) when the probe reports
interlaced fields and the setting is `auto`; double rate uses
`mode=send_field`. **Scaling** caps to the selected quality while
preserving aspect ratio and even dimensions.
**Audio** re-encodes to the selected codec with
`audio_bitrate_kbps`, downmixing more than two channels to stereo when
`downmix_audio` is on. **Subtitles** follow `subtitle_mode`: text
tracks are extracted to WebVTT sidecars, image tracks (PGS/VobSub/DVB)
are burned in via `overlay`, text tracks can be burned in via
`libass`, and anything impossible is refused with
`unsupported-media`.

**HTTP.** `POST /api/items/{id}/transcode` starts/joins (202 while
pending, 200 when ready, 429 `queue-full`, 403 `forbidden` when the
user's policy denies the operation), `DELETE
/api/items/{id}/transcode?session=` stops a session (with `&viewer=`,
releases one viewer's hold; see **Viewers** below), `GET
/api/items/{id}/transcode/status?session=` polls, `GET
/api/items/{id}/transcode?session=` serves the progressive MP4 (409 for
an HLS session), `GET /api/items/{id}/transcode/hls/{file}` serves
`index.m3u8`, `init.mp4`, `segNNNNN.m4s` and `segNNNNN.ts`, and a
software retry's `sw-init.mp4` and `sw-segNNNNN.*` (every other
name is 404;
the served playlist signs each media URI with `session` and `token`,
because a relative URI in an m3u8 drops the playlist's query string and
would otherwise 401 — native HLS cannot send headers either),
`GET /api/items/{id}/subtitles?session=` serves the WebVTT sidecar, and
`GET /api/playback/options` publishes the ladder and preferred
delivery to authenticated players.

**Viewers (D-281).** A session is keyed by source and options, not by
client, so two players with the same options and start point (two
tabs, two devices, two accounts) share one. A player names its hold with
a viewer id it makes: 1 to 64 ASCII letters, digits, `-` or `_`
(anything else is 400 `invalid-message`). A fresh id for every start is
the safe choice, and the web player's: a player that leaves a session
and starts the same one again (a rebuild with unchanged options, or two
starts racing) would otherwise end its new hold with the release of the
old one if the server saw that release last. `viewer` in
the `POST /api/items/{id}/transcode` body attaches it to the session it
starts or joins (a session served finished from the cache attaches
nothing), and `DELETE /api/items/{id}/transcode?session=&viewer=`
releases that hold. While a session is produced the server keeps its
viewers, keyed by account and id, so one account cannot release
another's viewer. A release stops and drops the session, as a release
without a viewer does, only when that viewer held it, nothing else
holds it and it is still `queued` or `running`; the answer is then
`idle`. Otherwise it answers the session as it stands (200, its
status): another viewer keeps it running, and a finished session stays
in the cache for a replay, since it holds no runner (the LRU evicts it;
a release without a viewer still drops it). A start without `viewer`
behaves as before: it is an unnamed hold that no viewer's release ends,
so a viewer leaving never stops a session an older client is playing.
A release without `viewer` behaves as before too: it stops the session
for every viewer, and only its owner (or an admin, through
`/api/admin/transcodes/{session}`) may. Past 32 named viewers, further
joins count as unnamed. A viewer that vanishes without releasing (a
closed tab) is not reaped on its own: the idle timeout still stops a
session nobody fetches from.

**Operator surface.** `GET/PUT /api/admin/settings/transcode`
(admin-only) reads and writes the policy — validated, persisted as JSON
in the bbolt `meta` bucket, applied to new sessions without a restart —
and returns the probed capabilities alongside it. `GET
/api/admin/transcodes` lists sessions with their owning `user_id`,
`DELETE /api/admin/transcodes/{session}` stops one. `lain doctor` reports
the same probe, so an operator can see what the local ffmpeg actually
supports before enabling hardware.

**Per-user limits** live on the account (`auth.PlaybackPolicy`):
`allow_video_transcode`, `allow_audio_transcode`, `allow_remux`,
`max_bitrate_kbps`, `max_streams` and `subtitle_mode`, all absent
meaning unrestricted (and `subtitle_mode` empty meaning "use the
server default"). The gateway resolves them into the request `policy`
and `user_id`; the plugin refuses a remux/encode the user may not have
with `forbidden`, and a start beyond `max_streams` with
`too-many-streams` (HTTP 429). Joining a session the account already
runs never counts twice, and a finished session stops counting.

**Advanced settings.** `thread_count` pins ffmpeg's encoder threads
(0 = auto); `max_muxing_queue_size` bounds the output packet queue so a
slow client cannot abort the muxer; `transcode_temp_path` moves
artifacts to another volume as `<path>/lain-transcode/<session>/`
(`media.mp4`, `index.m3u8`, `init.mp4`, `seg*.m4s`, `subs.vtt`) while
the JSON sidecar stays in the data dir — cleanup, eviction and orphan
sweeping only ever touch that owned subdirectory; `remote_bitrate_limit_kbps`
is a server-wide ceiling whose tighter side wins against the per-user
limit. Per-codec encoding follows Jellyfin: `h264_preset`/`h265_preset`/
`av1_preset` (empty inherits `encoder_preset`) and `h264_crf`/`h265_crf`/
`av1_crf` (0 inherits `crf`). `audio_vbr` switches AAC to ffmpeg's native
VBR step, mapped from `audio_bitrate_kbps` by `AudioVBRQuality`
(≥192→2, ≥128→1.5, ≥96→1, ≥64→0.5, else 0.1). `downmix_audio_boost` is
a linear gain applied only while downmixing (default 2, Jellyfin's
default); `downmix_stereo_algorithm` is `none` (ffmpeg's own downmix) or
`nightmode`, which is **lain's own** documented matrix
(`pan=stereo|c0=0.4*c0+0.4*c1+0.8*c2+0.2*c4|c1=…`) and is explicitly not
Jellyfin's Dave750/NightmodeDialogue coefficients. Because the matrix
names 5.1 channel positions it is applied only to a 6-channel source;
another surround layout (quad, 7.1) uses ffmpeg's layout-aware downmix
and the session reason says `nightmode needs a 5.1 source`.
`deinterlace_double_rate`
keeps both fields as frames (`yadif=mode=send_field`), doubling the
output frame rate. `fallback_font_path`/`fallback_font_name` add
`fontsdir=`/`force_style='Fontname=…'` to burned-in subtitles (the
family name is sanitized so a settings value can never inject filter
graph syntax), gated by `fallback_font_enabled` (Jellyfin's "Enable
fallback fonts"; nil/absent = on, so a saved path can be kept but
ignored). `hardware_decode_10bit_hevc`/`hardware_decode_10bit_vp9`
gate hardware decoding per codec, because a backend that mishandles
10-bit output would otherwise fail or produce wrong colors.
`hardware_device` names the VA-API render node (default
`/dev/dri/renderD128`); it feeds both the capability probe and the
decode/encode `-hwaccel_device` argument, and stays out of the session
identity because it picks which GPU runs the work, not the bytes.

`throttle_ahead_sec` is Jellyfin's throttle delay expressed as a
produced-ahead budget: ffmpeg is paused once it runs further than that
ahead of the viewer's fetched segment.

Live pipeline metrics (`fps`, `output_bitrate_kbps`) are reported while
a job runs, so the admin session list shows what the encoder is
actually doing; the terminal states omit them.

## lain.media.probe@1 (exactly-one)

Input: `{file_path}` → `MediaInfo{format, duration?, streams[]}`.
ffprobe-backed, bounded metadata only (index, type, codec, profile,
pixel format, dimensions, channels, bitrate, language, title,
default/forced, HDR color markers, subtitle convertibility). The
per-stream `bit_rate` (bits/s, 0 when unknown) lets the gateway compare
the source against an account's bitrate cap. Never carries bytes.
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
processes; absence degrades this capability, never the boot. Without `t`
the gateway serves a still that came with the file instead (D-269), and
catalog answers say which it will be (`CatalogItem.thumbnail`, D-282).
That still is the cover that stands for the item's title, which beside a
show's first episode is the show's poster. `art=still` (D-284, additive)
asks for the file's own picture instead, for a slot that stands for that
one file (an episode's row, a continue card): `<name>-thumb.*` next to it,
else the frame at `t` (10 s by default; a comic archive's cover page),
never `<name>-poster.*` or the folder's `.metadata/thumbs/` still. An
older server ignores it and answers the cover.

`GET /api/items/{id}/storyboard` (D-275) is how a seek bar previews
without a request per position: `{interval_sec, count, columns, rows,
tile_width, tile_height}` for a video whose length is known (404
otherwise), and `GET /api/items/{id}/storyboard/image` is one JPEG of
`count` frames, `columns` a row, tile `i` (row by row) being the frame
near `i * interval_sec`. At most 120 tiles of 160x90, at least 2 s apart.
The image is made on its first request, which waits for it (keyframes
only, one pass over the file), and is cached like a thumbnail.

`GET /api/durations?ids=a,b,c` (D-276) answers how long several videos
are in one request, so a list of episodes can say "24 min" where the
catalog only knows a file: `{items: [{item_id, duration_sec}]}`. Ids that
are unknown, not visible to the caller, not videos or not probeable are
left out (never an error); at most 200 ids are read. The server probes a
file once and remembers the length by path, size and modified time, a few
files at a time.

## Metadata (merge-many)

`lain.metadata.search@1`: `{query, kind, limit, dir}` →
`MetadataCandidate[]` per provider (provider, remote_id, title,
synonyms, year, poster). `dir` hints local sources; remotes ignore it.

`lain.metadata.resolve@1`: `{provider, remote_id}` → `MetadataRecord`
(full entry; artwork as URLs, never bytes).

The metadata pipeline's orchestration is itself a capability
(`lain.metadata.enrich@1`, below); it fans out via
`Registry.CallMergeReport`, dedups by normalized title, and scores
exact matches first, then binding precedence
(`nfo → kitsu → anilist → jikan → tvmaze`; in-process in both servers,
API roots overridable by `LAIN_METADATA_<NAME>_URL`, D-201). TVMaze is keyless and
TV-first: it answers series/episode/video/anime kinds and stays
silent for movies. TMDB and IMDb are deliberately out of this slice
(TMDB needs a user-supplied key, IMDb has no free official API;
see Q-010/Q-011). Failing providers are skipped, so
an upstream outage degrades the merge instead of failing it. Winners
resolve through `InvokeProvider` (still through authority)
and persist as overlays (`POST /api/catalog/{id}/enrich`), never
inside the catalog: removing a provider deletes its overlays without
touching identity, progress or files. Repeat queries hit a TTL cache
(search 7d, records 30d) instead of the network. Grids read many
overlays at once via `GET /api/enrichments?ids=` (bounded, one read
transaction); missing entries are simply absent.

## lain.metadata.enrich@1 (exactly-one)

The enrichment pipeline as one capability (D-076). Ops:

- `enrich` `{item, only?}` → `Enrichment`: search fan-out, pick,
  resolve, persist. `not-found` when nothing matched.
- `get` `{item_id}` → `Enrichment`. `not-found` on absence.
- `batch` `{ids}` → `{items}`: overlays for a grid page.
- `delete` `{item_id}` → bool.
- `backfill` → `{checked, enriched}`: overlay-less items enriched in
  one pass (auto-enrich after scans).

The built-in provider (`lain-metadata-enrich`) owns the saver and
the search/record caches and calls `metadata.search`/`resolve` and
`catalog.read` through the registry — a swapped catalog or metadata
provider changes enrichment without a code change.

## lain.settings.transcode@1 (exactly-one)

The persisted transcode policy store (D-076). Ops: `get` (nil input →
`TranscodeSettings`, current or defaults), `put` `{settings}` →
validated `TranscodeSettings`, `ensure` `{settings}` → `{seeded}` for
first-boot adoption, `has` → `{stored}`. A corrupt document is a typed
`internal` error — never silently reseeded.

## lain.ui.theme@1 (exactly-one)

Theme derivation (D-076). Input `{colors_path?}` → `Palette` of
semantic roles (bg/fg/accent variants). The built-in
`lain-theme-omarchy` reads Omarchy colors (env `LAIN_OMARCHY_COLORS`
or the default path), falls back to a built-in palette, and owns the
contrast/legibility fixups. The gateway relays the palette at
`GET /api/theme`; an unbound or failing binding returns 503 while the
frontend's CSS defaults keep the UI usable.

## lain.playback.local@1 (exactly-one)

Same-machine playback supervision (D-072/D-076). Ops: `players` (nil →
`{players}`), `play` `{player?, user_id, language?, audio_language?,
subtitle_language?, subtitle_mode?, item}` (D-280; `language` alone is
D-071's one language for both) →
`{status, player, count}`. The provider owns playlist resolution
(series queue from the episode onward), resume positions (through
`userstate.progress`), VLC subtitle policy (through `media.probe`),
and mpv/VLC spawning. `busy` when a session is running;
`dependency-unavailable` when no player exists. The gateway still owns
the loopback check — remote browsers get 403 before the call.

## lain.backup.create@1 (exactly-one)

Backup artifact packaging (D-076). Input `{snapshot_path, out_dir,
docs}` — the trusted core snapshots the live database to a temp file
(never hands the locked db path) plus data-directory JSON documents —
→ `{path, filename}`. The built-in `lain-backup-bundle` passes the
snapshot through unchanged; a replacement can package, encrypt or ship
it elsewhere.

lain-rs (D-197): the input adds `files` (archive name → path: the data
dir's `modules.json`, `roots.json` and `avatars/<user id>`), the docs are
`manifest.json` and `composition.json`, and the snapshot is a redb file
copied in one read transaction. Its `lain-backup-bundle` writes one
uncompressed tar, `lain-backup-<UTC stamp>.tar` (`lain.backup@1`):
`manifest.json`, `lain.redb`, `composition.json`, then the files.
Restore: stop the server and extract into an empty data dir.

## lain.source.watch@1 (exactly-one)

Filesystem watching and debounce as a provider (D-068/D-076). Ops:
`poll` `{libraries}` → `{dirty}` (diffs the watched set against the
input — library adds/removes need no separate signal — and drains ids
whose 2s quiet window elapsed); `dirty` `{library_id}` re-marks with a
fresh timestamp (the gateway's re-nudge when a scan is in flight);
`close` stops the watcher. The gateway polls on a 500ms ticker and
reconciles drained libraries through `lain.ingest.scan@1`.

## lain.comic.pages@1 (exactly-one)

Reading slice (D-085). Input `ComicPagesInput{path}` (supplied by the
gateway after catalog lookup); output `ComicPages{format, pages[]}` with
each page `{index, mime, size, width, height}` in natural reading order
(`page2` before `page10`; directories, dotfiles, `__MACOSX` and non-image
members are dropped). `width`/`height` are 0 when the header could not be
read (external formats, unknown codecs); clients then measure after load.
The archive entry name is internal (`json:"-"`): a client addresses a page
by index, so it can never reach an arbitrary archive member. Bytes never
cross the boundary — the gateway opens the archive (`comic.OpenPage`) and
streams the entry.

Formats: `cbz` natively; `cbr`/`cb7` through `bsdtar` or `7z` when
installed, else `dependency-unavailable`. Errors: `invalid-archive`,
`unsupported-format`, `dependency-unavailable`, `not-found`. Indexes are
cached in memory by (path, mtime, size).

`Candidate` gains an additive `library_type`, and the identify binding is
now `lain-identify-comic`, `lain-identify-anime`, `lain-identify-generic`
(composition v8; a saved composition is upgraded in place). The comic
identifier declines everything that is not `cbz`/`cbr`/`cb7`. It stores a
volume as `season` and a chapter or issue as `episode`, each 0 when absent.

Gateway routes: `GET /api/items/{id}/pages` (auth) returns
`{kind, format, direction, pages[]}` with the kind's default direction
(`rtl` for manga, `ltr` for comics); `GET /api/items/{id}/pages/{n}`
streams one page (bearer or `?token=`, `ETag`, private cache);
`GET /api/items/{id}/thumbnail` renders the cover — the first portrait
page among the first four — through the thumbnail provider. Reading
progress uses the shared progress record with `position_sec` as the
1-based page and `duration_sec` as the page count.

## The tracking list (D-078..D-081)

The lain list is a per-user tracking domain next to catalog and
userstate: entries are standalone records that exist with or without
library files, so manga, comics and other non-played types share one
list with anime, movies and series (D-078). Catalog still owns file
identity; userstate still owns playback progress; list rows never merge
into either.

### lain.list.read@1 / lain.list.write@1 (exactly-one)

`ListEntry` carries `title`, `media_type`
(`anime|manga|movie|series|comic`), `status`
(`current|planning|completed|paused|dropped|repeating`), `progress`,
`progress_total`, `progress_volumes`, `score`, dates, `cover` and the
remote identity (`platform` + `remote_id`); also (D-288) `alt_titles`
(the platform's other names, matched too), `pushed_at` (when lain last
wrote the entry; kept across imports) and `push_error` (the stable code
of the last failed write; an import clears it). `item_id` and
`matched_by` (`id|title`) are the gateway's join with the library,
filled on reads and never stored.

Read: `ListInput{user_id, type?, status?}` → `ListEntry[]` sorted by
title then platform.

Write: `PutPlatformInput{user_id, platform, entries}` →
`ListSyncStats{upserted, removed}` — remote-authoritative replace for
that platform (D-081): incoming remote ids are upserted and every
entry the platform owned that is absent from the response is deleted.
`DeletePlatformInput{user_id, platform}` → removed count (unlink).
`PutEntryInput{user_id, platform, entry}` (`put-entry`) → the stored
entry: one entry upserted, the rest of the set untouched (a write lain
made to the platform, D-288).

### lain.list.account@1 (exactly-one)

`LinkedAccount` is a user's connection to one external platform:
remote identity, token, expiry and sync bookkeeping. `Token` is a
credential — it persists but never crosses a public payload
(`Public()` clears it, `last_sync_error` records a stable code, never a
remote body). Ops: `GetAccountInput`, `PutAccountInput`,
`DeleteAccountInput` → bool, `ListAccountsInput{user_id?}` (empty user
lists every account, for the scheduler).

### lain.listlink@1 (ordered-many)

The platform connector surface; a provider that does not serve the
requested platform declines with `unsupported-platform` and the next
bound provider is tried. Ops:

- `LinkAuthorizeInput{platform, client_id, redirect_uri, state}` →
  `{url}` the user opens.
- `LinkExchangeInput{platform, client_id, client_secret, code,
  redirect_uri}` → `LinkedIdentity{token, token_expires_at,
  remote_user_id, remote_username}`.
- `LinkFetchInput{platform, token, remote_user_id}` →
  `LinkFetchOutput{entries}` — the remote list mapped onto
  `ListEntry` fields.
- `LinkPushInput{platform, token, remote_id, progress?,
  progress_volumes?, status?, score?}` → `LinkPushOutput{progress,
  status, entry?}` — scrobbles and edits: writes the fields given to
  one entry (creating it when the list lacks the media); an absent field
  is left as the platform has it (D-289: `progress` was required before,
  and callers then always sent it). `score` is 0..10 (0 clears it).
  `entry`, when the connector reads the saved entry back, is the whole
  `ListEntry`. Additive to `@1`: connectors that predate it answer
  `invalid-message`, never crash.
Built-in `lain-listlink-anilist`: AniList's authorization-code flow
plus an authenticated GraphQL `MediaListCollection` fetch for `ANIME`
and `MANGA`, and a `SaveMediaListEntry` mutation for push. A dead or revoked token surfaces as `token-invalid`.

### lain.settings.integrations@1 (exactly-one)

The operator's external-platform credentials (D-080), stored in the
`meta` bucket. `nil` → `IntegrationSettings{anilist_client_id,
anilist_client_secret}`, `IntPutInput{settings}` stores the document,
`IntEnsureInput{base}` seeds only when no document exists — an
explicit save (even an empty one) is never overwritten by env flags.

### Gateway routes

`GET /api/me/links` (auth) → `{links}`: remote identity, entry count,
sync state — never the token. `GET /api/me/links/{platform}/authorize`
→ `{url}` built with the operator's client id and an HMAC-signed state
bound to the local user id and platform (10-minute expiry, no server
storage, D-080); `503 not-configured` while the operator has not saved
a client. `GET /api/auth/{platform}/callback` (public) verifies the
state, exchanges the code, stores the account and runs the first
import, then redirects to `/settings/connections?linked=` or
`/settings/connections?link_error=` — failures never silently succeed.

The zero-setup alternative is auth-pin (D-083): `GET
/api/me/links/{platform}/pin` → `{url}` built with the official Lain
application's credentials — compiled into the binary, extractable by
design like every distributed OAuth client's (Taiga model). AniList
refuses the implicit grant for console-created apps, so the pin page
hands back an authorization **code**; the user pastes it into `POST
/api/me/links/{platform}/code` `{code}` and the server runs the
standard exchange with the embedded client id/secret and the pin
redirect URI. A pasted URL or fragment is normalized to the code
inside. `503 not-configured` for platforms without a pin client;
`400 invalid-grant` when AniList rejects the paste.
`DELETE /api/me/links/{platform}` removes the account and its
platform-owned entries (D-079). `POST /api/me/links/{platform}/sync`
re-imports on demand; `401 token-invalid|token-expired` marks the link
for reconnection while imported entries stay visible. `GET /api/list`
(auth) → `{entries}`, filterable with `?type=`/`?status=`.

**The list and the library (D-288, D-289).** `GET /api/list` fills each
entry's `item_id` (the library title it is: the title's first file in
watch order) and `matched_by`: `id` when a file of the title has a
`lain-metadata-anilist` overlay naming the entry's `remote_id`, else
`title` when exactly one title's names (files' title and aliases, the
overlay's title) equal one of the entry's (`title`, `alt_titles`) under
`match_key` (lower-case letters and digits, other runs one space).
Anime entries join watched titles, manga entries read ones (`manga`,
`comic` kinds); private libraries never join, locked or not, and
libraries hidden from the session are left out.

`GET /api/items/{id}/tracking` (auth) → `{tracks: [{platform,
remote_id, matched_by?, scrobble, entry?}]}`, one per linked platform,
from the stored copy (no remote call): `remote_id` is the entry's, else
the id the title's metadata names, else empty. `{tracks: []}` with
nothing linked or for a private library's item; `404 unknown item` for
an item the session cannot see.

`PUT /api/list/{platform}/{remote_id}` (auth) with `{status?, progress?,
progress_volumes?, score?, item_id?}` writes the given fields to the
platform, then to the stored copy, and answers `{entry}` (joined). At
least one field; `status` in the shared vocabulary; `score` 0..10 (AniList
gets `scoreRaw`, the 100-point value, whatever the user's format);
`item_id` describes a work the list lacks (title, cover, kind). Errors:
`400 invalid-message`, `404` not linked or unknown item, `403
private-library`, `401 token-invalid|token-expired`, `429 rate-limited`,
`503` otherwise. Each write, good or failed, is the account's
`last_push{at, remote_id, title, progress?, status?, via, error?}` on
`GET /api/me/links` (`via`: `scrobble` or `edit`).

Automatic updates (`scrobble`, on for a new link; a relink keeps the
stored choice): a finished item advances the entry it is (overlay id
first, then names): an anime episode of season 0 or 1, a film a
one-episode entry, an archive's chapter (`episode`) and volume
(`season`) a manga entry; never lower, never a completed or repeating
entry, `completed` on the last episode or chapter. Finishes wait 1.5 s
of quiet in one worker and go out as one write per entry. Private
libraries never scrobble. `list-changed` gains the actions `saved` (an
edit) and `push-failed` (`remote_id`, `error`).

The callback URL AniList must be registered with is derived per
request as `{scheme}://{host}/api/auth/{platform}/callback` (honoring
`X-Forwarded-Proto`); the admin page prints the exact value to paste
in the developer settings — AniList requires an exact match.

Sync is remote-authoritative for platform-owned entries and runs on
connect, on demand, and on a fixed ~6h server ticker
(`--list-sync=0`/`LAIN_LIST_SYNC=0` leaves manual syncs only; D-081).

**Operator surface.** `GET/PUT /api/admin/settings/integrations`
(admin-only) reads and writes the AniList client credentials. GET
reports `{client_id, secret_set, callback_url}` — the secret is
write-only and never serialized back; a PUT without it keeps the
stored one. `LAIN_ANILIST_CLIENT_ID`/`--anilist-client-id` and
`LAIN_ANILIST_CLIENT_SECRET` (env only, like `LAIN_PASSWORD`) seed the
document on first boot.

## Social (docs/slices/social.md)

A fourth domain beside catalog, userstate and the list. One built-in
provider, `lain-social-bolt`, serves four exactly-one capabilities;
inputs are the Go types in `internal/plugins/social`, shared shapes in
`internal/contracts/social.go`. Works are `WorkRef{kind, title,
item_id?}` keyed by kind + `catalog.TitleKey(title)`, so every media
kind (video or reading) is covered without a contract change.

| Capability | Owns |
|---|---|
| `lain.social.graph@1` | privacy settings, favorites, friend requests, friendships, blocks, the `CanSee` rule |
| `lain.social.activity@1` | activity log (progress / completed / rated), friends feed, notifications |
| `lain.social.reviews@1` | ratings (1..10), reviews, comments and replies |
| `lain.social.collections@1` | collections, collection sharing, work shares |

Typed codes: `invalid-message` (400), `not-found` (404 — also how a
block placed on the viewer reads), `forbidden` (403). Routes live under
`/api/social/*`; the full list is in the slice plan.

In lain-rs (D-216, D-217) the provider is `lain-social-redb` with op
names per capability, and a `WorkRef` named by a catalog item also
carries the item's `library_id` (set by the server): rows naming an
item of a private library this session has not unlocked are left out
of every social answer (D-156). New notifications are announced as
`social-notification` and link changes as `list-changed` on
`/api/events` (D-219).

## Acquisition (docs/slices/acquisition.md)

Three capabilities; the engine, queue, import and subtitle placement
are core code (`internal/torrent`, `internal/acquire`), not plugins
(A-2).

| Capability | Mode | Built-ins | Input → output |
|---|---|---|---|
| `lain.release.parse@1` | ordered-many, first accepted | `lain-release-model` (lain-parser over `<data-dir>/parser.sock`, `LAIN_PARSER_SOCKET` overrides; in lain-rs a component, see "P4 components"), `lain-release-tokenizer` | `ReleaseParseInput{name, kind}` → `Release` (zero `title` = abstain; optional `alternatives`, D-279) |
| `lain.indexer@1` | ordered-many; `unsupported-protocol` declines | `lain-indexer-torznab` (Torznab + Newznab) | `IndexerCapsInput` → `IndexerCaps`; `IndexerSearchInput` → `IndexerSearchOutput` |
| `lain.subtitle@1` | ordered-many; `unsupported-provider` declines | `lain-subtitle-opensubtitles` (OpenSubtitles.com REST v1) | `SubtitleSearchInput` → `SubtitleSearchOutput`; `SubtitleDownloadInput` → `SubtitleDownloadOutput` (a link; the core fetches it, D-150). In lain-rs the ops are `search` and `download` (D-202) |

Indexer and subtitle-provider credentials travel inside the input and
never appear in an error message (errors name the host only, D-114,
D-137). Typed codes: `invalid-message`,
`unsupported-protocol`, `auth-failed`, `rate-limited`, `indexer-error`,
`dependency-unavailable`; the subtitle provider adds
`unsupported-provider` and `not-found`.

In lain-rs (D-205..D-214) the engine, manager and download queue are
the crates `lain-torrent`, `lain-acquire` and `lain-downloads`; every
`/api/acquire/*` and `/api/downloads*` route keeps Go's shapes and
texts. `GET /api/events` adds `grab-changed` and `download-changed`
(admins; `{id, removed: true}` on delete, D-210).

## P4 components: bridges to standalone apps (lain-rs, D-225..D-231)

The owner's standalone apps plug in as components (DESIGN-v2 §6). Each
bridge in `crates/lain-bridge` is a provider that talks to one app over
the app's own local API; `lain component run <id> --sock {sock}`
serves it, and `lain component install <id> [--data-dir D] [--k v…]`
writes `<data-dir>/plugins/<id>.json` (entrypoint: this `lain`), which
the running server's provisioner picks up (`uninstall` removes it,
`list` names them). The apps keep their own lifecycles: nothing in lain
starts them. Credentials never go into a manifest: tokens come from the
app's token file or its own environment variable.

| Bridge | Capability | App and how it is found | Options |
|---|---|---|---|
| `lain-release-model` | `lain.release.parse@1` | lain-parser daemon: `--parser`, else `LAIN_PARSER_URL`, `LAIN_PARSER_SOCKET`, `<data-dir>/parser.sock`, `http://127.0.0.1:9377` | `parser` |
| `lain-subtitle-animesubs` | `lain.subtitle.translate@1` | AnimeSubs server: `<home>/server.json` + `server-token`; home `--home`, else `$ANIMESUBS_HOME`, else `<config dir>/com.enrell.animesubs`; `ANIMESUBS_TOKEN` | `home`, `url`, `model-kind`, `model`, `base-url` |
| `lain-acquire-anything` | `lain.acquire.extract@1` | Anything daemon: `~/.config/Anything/server.json` + `server-token` (`--home`, `ANYTHING_CONFIG_DIR`, `ANYTHING_TOKEN`) | `home`, `url` |

All three are bound by default (they are in `COMPONENT_SERVED`), so
installing the manifest is the only step.

**Health and declines.** A bridge's `health` is a round trip to its app
(`GET /health` or `/v1/health`, which must name the service), cached for
2 s because the registry asks before every call. The registry skips an
unhealthy provider, so:

- app not installed, stopped, still loading, or its discovery file stale:
  `health` fails with `dependency-unavailable` (`parser socket not
  found`, `animesubs is not running (no server.json)`, `anything not
  answering at 127.0.0.1:9642`…) and the next provider answers;
- the app answered but declines the input: the contract's decline (a zero
  `Release`; `unsupported-format`; `unsupported-url`), and the next
  provider answers;
- the app failed at call time: `dependency-unavailable`, or
  `outcome-unknown` on a timeout; a rejected token is `auth-failed`
  (never echoing the token).

### lain.release.parse@1 from the model (D-225)

`lain-release-model` posts `{filename}` to the daemon's `/parse` (with
`alternatives: 5` since D-279, see "Release.alternatives"). A null
`record` (the model's gated abstention, or a name over its 254-character
window) answers the zero `Release`. Otherwise the model is trusted for
title, episode_title, year, season, episodes (absolute when it states no
season), release group and resolution, and the tokenizer for every other
tag (Go's `merge`), except a year the model does not label and its
title holds (see "Release.alternatives"); `parser` is
`lain-release-model`. Default binding:
`[lain-release-model, lain-release-tokenizer]` (V-5,
`docs/benchmarks/parser-v5.md`, D-226).

### Release.alternatives (D-279)

A name can be read more than one way and nothing in it says which is
right: "Show 2 - 03" is episode 3 of the sequel "Show 2", or of season 2
of "Show". `Release` gains the additive, optional
`alternatives: ReleaseIdentity[]`: other identities the provider
considered, best first, at most five. A `ReleaseIdentity` is
`{title, year, season, episodes, absolute}` with `Release`'s meanings
and zeros. The answer itself is unchanged, and a provider with nothing
to offer (the tokenizer, an older parser service) omits the field.

- **Who decides.** The caller that knows which title it expects, never
  the parser. `lain_acquire::util::settle(release, known, year)` keeps
  the answer when `known` accepts its title; otherwise it takes the first
  alternative whose title `known` accepts, moving the identity as a
  whole (title, year, season, episodes, `absolute`; `season_pack` is
  cleared when episodes come with it) and keeping every other field. An
  alternative whose title has no word names nothing and is skipped. So
  is one whose year rules it out: `year`, the year the caller expects,
  and the alternative's, both known and more than one apart
  (`util::years_agree`; 0 is unknown and never rules anything out). It
  always drops the list: alternatives are advice for one decision and
  are never stored in a grab or returned in a search candidate.
- **Where.** A search settles against its query and year, and rejects a
  candidate whose settled year is more than one from the query's
  (`year does not match`). Automation and its grabs settle against the
  monitored title, its aliases and its year, unless the parser's own
  answer names a monitored title (title or alias, any library, enabled
  or not; `monitored::settle_for`): that release is that title's, and no
  other monitored title reads it as an alternative ("Show 2 - 03"
  answered as "Show 2" stays the sequel's while "Show 2" is monitored,
  never S2E3 of "Show"). An import settles each file against the title
  and year of the grabbed release. A manual grab and a library's other
  titles settle nothing (a sequel must not land in the folder of the
  show it follows). `GET /api/acquire/parse` returns the list as the
  provider gave it.
- **From the model.** `lain-release-model` posts `{filename,
  alternatives: 5}` (lain-parser 1.3.0; an older service ignores the key
  and answers none). The list is read entry by entry: a list or an
  entry that does not read is left out, never failing the answer. Each
  alternative is cleaned like the answer, compared with the model's own
  reading (so the answer is not listed again) and carries the model's
  numbers only, never the tokenizer's. A year the model did not read,
  for the answer and each alternative alike, is the tokenizer's, unless
  the reading's title holds it as a word ("Blade Runner 2049" read off
  "Blade.Runner.2049.1080p"): then it is the last other year the
  tokenizer read in the name, or none.

### lain.subtitle.translate@1 (ordered-many; `unsupported-format` declines; D-228)

| Op | Input | Output |
|---|---|---|
| `start` | `TranslateStartInput{input, output?, target_language, model?, overwrite?}` | `TranslateJob` |
| `status` | `JobRef{job}` | `TranslateJob` |
| `cancel` | `JobRef{job}` | `TranslateJob` (`cancelled`) |

- `input`/`output` are absolute paths on the host both run on; the
  component reads and writes the files (no subtitle bytes in the call).
  An empty `output` is the app's default, `<stem>.<lang>.<ext>` next to
  the input. `target_language` is BCP 47.
- `model` (`TranslateModel{kind, model?, base_url?, api_key?}`, `Debug`
  redacts the key) picks the translation backend; absent, the bridge's
  configured default (`--model-kind/--model/--base-url`, never a key; the
  app then reads the key from its own environment).
- `TranslateJob{job, provider, state, phase, done, total, input, output,
  target_language, created_at_ms, finished_at_ms?, error?{code, msg}}`;
  `state` is `queued`, `running`, `succeeded`, `failed` or `cancelled`;
  `phase` is the app's finer step (clients accept unknown ones);
  `done`/`total` count cues, `total` 0 = unknown. `status` and `cancel`
  go to the `provider` that started the job.
- Codes: `invalid-message`, `not-found`, `not-cancellable` (already
  finished), `already-exists` (output exists, no `overwrite`),
  `unsupported-format`, `provider-error` (the model backend failed),
  `auth-failed`, `dependency-unavailable`, `outcome-unknown`.
- AnimeSubs: `start` is `POST /v1/jobs` (`translate_subtitle`), `status`
  `GET /v1/jobs/{id}`, `cancel` `POST /v1/jobs/{id}/cancel`
  (animesubs#8). Its jobs persist across its restarts; one found
  unfinished after a restart reads `failed` with code `interrupted`.

**Routes (subtitles module, admin):**

- `POST /api/items/{id}/sidecars/{n}/translate` `{target_language, model?,
  overwrite?}` → `202 TranslateJob`. The sidecar is named by its index in
  `GET /api/items/{id}/sidecars` (never by path); the item must be
  visible to the session (D-156). 400 without `target_language`, 404
  `unknown item` / `no such sidecar`, 503 `dependency-unavailable` when no
  translator is installed or healthy, other component errors by code
  (`unsupported-format` 422, `already-exists` 409, `provider-error` 502).
- `GET /api/subtitle-translations/{provider}/{job}` → `200 TranslateJob`;
  `DELETE` the same path cancels (`200`, or `409 not-cancellable`); 404
  for an unknown provider or job.
- Clients poll the job; when it succeeds the new file is one more sidecar
  of the item. No `/api/events` event yet.

### lain.acquire.extract@1 (ordered-many; `unsupported-url` declines; D-230)

| Op | Input | Output |
|---|---|---|
| `extract` | `ExtractInput{url}` | `ExtractOutput{url, title, site, duration_s, thumbnail, content_class, candidates[], subtitles[], provider}` |
| `enqueue` | `ExtractEnqueueInput{url, format?, output_dir?}` | `ExtractJob` |
| `status` | `JobRef{job}` | `ExtractJob` |
| `cancel` | `JobRef{job}` | `ExtractJob` (`cancelled`) |

- `ExtractCandidate{id, kind: video|video-only|audio, quality, width,
  height, codec, mime_type, filesize}`. Stream URLs (often signed) never
  leave the component: lain chooses with `format` (the app's selection,
  e.g. `best` or `1080p`) and the app downloads.
- `output_dir` (absolute) sends the download into a library folder;
  empty leaves it to the app's own routing.
- `ExtractJob{job, provider, state, url, title, output_path,
  downloaded_bytes, total_bytes, error?}`; `state` is `queued`,
  `running`, `paused`, `succeeded`, `failed` or `cancelled`.
- Codes: `unsupported-url`, `extract-failed`, `invalid-message`,
  `not-found`, `not-cancellable`, `auth-failed`, `dependency-unavailable`,
  `outcome-unknown` (an extraction can take a minute; the bridge waits
  75 s).
- Anything: `extract` is `POST /extract`, `enqueue` `POST /queue`
  (`options.format`, `options.output_dir`), `status` `GET /queue/{id}`
  (`pending`→`queued`, `completed`→`succeeded`), `cancel`
  `DELETE /queue/{id}`. It needs Anything's token auth (Anything#1).
- **For the acquire module** (it owns the capability; no route is added
  here): a URL a user pastes goes to
  `registry.call_first(CAP, "extract", {url}, |o| !o["candidates"].is_empty())`;
  the chosen candidate becomes
  `invoke_provider(CAP, out.provider, "enqueue", {url, format, output_dir})`
  with `output_dir` the target library's folder; the downloads view polls
  `status` with the job's `provider` and `job`; a finished job's
  `output_path` is picked up by the library scan or watcher like any file.

## Component mode (wire protocol)

Any provider can run as a supervised process (D-074). The host
(`internal/component`, and `lain-component` in the Rust workspace)
listens on a unix socket it allocates; the component dials it and both
sides speak NDJSON frames:

- **Handshake**: the component's first frame is
  `{op:"hello", provider, protocol:1, capabilities[]}`. The host checks
  the id and protocol against the manifest and that every declared
  capability is served.
- **Invoke**: `{seq, cap, op, input}` →
  `{seq, output}` or `{seq, error:{code, msg}}`.
  `op` selects the operation inside a capability (the `(capability,
  op) → input type` table lives in `internal/component/ops.go`); an
  empty op resolves when the capability has exactly one input shape.
- **Health**: `{seq, op:"health", input:null}` → `{seq, output:{ok:true}}`
  or `{seq, error}` — transport-level, never routed through a
  capability.

Recorded sessions live in `conformance/component/*.ndjson`; the Go
component side and the Rust host and component sides replay them.

Frames are bounded (8 MiB). Sequence ids correlate requests and
responses. A provider panic answers `internal` — it never kills the
socket. Timeouts and mid-call deaths are `outcome-unknown`; an
unreachable or unspawnable component is `dependency-unavailable`.
Crashed processes respawn with backoff under the host's supervision.
Media bytes never cross the socket — the data-plane rule holds for
components exactly as it holds in-process.

`lain plugin-run --id <provider> --sock <path> [--data-dir <dir>]`
serves one built-in provider as a component — the same code that runs
embedded. Third parties get the protocol from `internal/component` or
any language that speaks NDJSON over a unix socket.

## Component manifests and provisioning

A manifest names a runnable component:

```json
{
  "id": "community.anime-parser",
  "version": "1.3.0",
  "capabilities": ["lain.media.identify@1"],
  "execution": {
    "kind": "process",
    "entrypoint": "/path/to/bin",
    "args": ["--sock", "{sock}"]
  }
}
```

`{sock}` expands to the socket path the host allocates. The
provisioner (`internal/component`) watches `<data-dir>/plugins/` for
`*.json` manifests: dropping one in spawns, handshakes, health-checks
and registers the provider; removing it unregisters and kills the
process; rewriting it reloads the component. Install/remove is logged;
a bad manifest or an unspawnable binary degrades to a log line, never
a failed boot. `Unregister` removes the provider from every binding
and deletes the registration (distinct from `Withdraw`, which keeps
the registration for a later swap-back).

Operator surfaces can list registered providers with their
capabilities and live health (`provider_info` on `GET /api/plugins`) so
a swap UI only ever offers candidates that can serve the capability.

Each binding of `composition` (on `GET /api/plugins` and the swap and
withdraw answers) carries `summary` (D-285, additive, omitted when
empty): what the job is, in one plain sentence. The server describes
the capabilities it knows (`lain_core::capabilities`, matched without
the version, so a new version keeps its sentence); for one it does not
know, the first bound provider that describes it does
(`Provider::describe`; a component manifest's optional
`summaries: {capability: sentence}`).

## Private libraries and portable roots (lain-rs, D-156, D-157, D-175)

Served by the Rust server only (P1b); the Go server is unchanged.

- **Library fields** (additive, omitted when unset): `private`, `owner`,
  `members`, `root`, `root_path`, and in responses `locked` for a
  private library (whether this session has unlocked it).
- `PATCH /api/libraries/{id}` (admin): `{name?, type?, path?, private?,
  owner?, members?}`. Making a library private makes the caller its owner
  unless `owner` is given; making it public clears owner and members.
  A new `path` (absolute, recorded under a mapped root when inside one)
  must be a readable directory no other library reads; the id stays, so
  progress and overlays stay attached, and the next scan reads the new
  folder and marks what it no longer finds missing (D-068). `type`
  changes which files the next scan reads.
- `PUT /api/libraries/order` (admin): `{ids}`, every library id once,
  first to last; answers the list in that order. Libraries carry
  `position` (1 first; omitted, 0, for one never ordered), and every
  list of libraries is in that order, the never-ordered ones last by
  name. 400 for a missing or repeated id, 404 for an unknown one.
- `POST /api/libraries/{id}/unlock` `{pin}` or `{password}` and
  `POST /api/libraries/{id}/lock`: per session (per token). Only the
  owner and members can unlock (403 `not a member of this library` for an
  admin who is not one); a user who cannot see the record gets 404
  `unknown library`; a wrong secret is 403 `wrong pin or password`; five
  failures in five minutes answer 429 until the window passes.
- `GET|PUT|DELETE /api/me/pin`: `{set}`; `PUT {pin, password}` (4–12
  digits), `DELETE {password}`.
- **Visibility** (one check in the gateway): items of a library this
  session has not unlocked are absent from `GET /api/catalog` (and its
  `total`), `GET /api/search`, `GET /api/me/continue` and episode lists;
  every item route (`/api/catalog/{id}…`, `/api/items/{id}/…` except
  progress, `/api/acquire/items/{id}/…`) answers 404 `unknown item`,
  exactly like a missing item; progress reads answer the zero value;
  events about them reach only sessions that unlocked them, and
  `library-changed` of a private library only admins and members.
  Admins see the record (to manage it) but not its items unless they
  are a member who unlocked it. Unlocks live in memory and end with the
  token. Every `lain-rs` token carries a random `jti` (its session), so
  two logins are two sessions; a token without one is refused.
- **Roots**: `<data-dir>/roots.json` (`lain.roots@1`) maps root names to
  this machine's directories, with `aliases` for other spellings
  (`V:\media`, `/mnt/v/media`). A rooted item stores only `root` +
  `/`-separated `rel_path`; responses fill `file_path` from them on
  every read.
  `POST /api/libraries` accepts `{root, path}` with `path` relative to the
  root; an absolute path inside a root is recorded under it, and a
  rooted library's id derives from `root:root_path`. `GET /api/roots`
  (admin) lists the mapping with `available`.
- **Export**: `lain-rs export` writes `lain.catalog-export@3` (`roots`
  plus per-item `root`/`rel_path`) and `lain-rs import --in` reads it
  back, replacing the catalog; @3 is the only version lain-rs reads
  (D-177). Nothing is imported from a Go data dir.
