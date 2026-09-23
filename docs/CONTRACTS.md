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

`CatalogItem.missing` (additive, `omitempty`) marks an item whose file
vanished from its library root (D-068). Missing rows stay in the
catalog — list, search, detail and episode-grouping reads all return
them — because the stable id keeps progress and enrichment reattaching
when the same path comes back. A clean rescan of the library clears
the flag. Missing is a state, never a deletion: progress and
enrichment documents are never swept.

Portable document `lain.catalog-export@1` is the replacement contract.

### Gateway read: one title's files

`GET /api/catalog/{id}/episodes` (auth) → `{items: CatalogItem[]}`, 404
for an unknown id. It answers the web title page: every item whose
`title` normalizes to the same key — lowercase, whitespace collapsed —
in watch order (season, episode, year, id), whichever library it lives
in. `catalog.TitleKey` is the one grouping rule, mirrored by the web
Library grid's `normalizeSeriesTitle`, so a show's card and its own page
cannot disagree about the episode count (D-056). The plugin capability
`lain.catalog.read@1` is untouched: the gateway already reads the
concrete catalog service for `Page`/`Get`.

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

## lain.userstate.progress@1 (exactly-one)

`PutInput{user_id, progress}` / `GetInput{user_id, item_id}`. Keyed by
user+item in its own document; catalog rewrites never touch it.

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
`cancel`, and `list` (admin view). States remain
`idle|queued|running|ready|failed`.

Request fields: `delivery` (`hls` default, or `progressive`),
`quality` (a ladder name from the settings), `max_bitrate_kbps`,
`video_codec` (`h264` default, `hevc`/`av1` only when the operator
allows them), `audio_codec`, `audio_stream`, `subtitle_stream`,
`subtitle_mode` (`auto|extract|burn|off`), `segment_index`, `policy`
(the requesting user's limits) and `settings`.

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
`fallback` string. Software encoders stay the default; there is no
silent opportunistic switching. The backend list mirrors Jellyfin's
documented methods: `vaapi`, `nvenc`, `qsv`, `amf`, `v4l2m2m`,
`videotoolbox` and `rkmpp` (Rockchip). `hardware_low_power` selects
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
/api/items/{id}/transcode?session=` stops a session, `GET
/api/items/{id}/transcode/status?session=` polls, `GET
/api/items/{id}/transcode?session=` serves the progressive MP4 (409 for
an HLS session), `GET /api/items/{id}/transcode/hls/{file}` serves
`index.m3u8`, `init.mp4`, `segNNNNN.m4s` and `segNNNNN.ts` (every other
name is 404;
the served playlist signs each media URI with `session` and `token`,
because a relative URI in an m3u8 drops the playlist's query string and
would otherwise 401 — native HLS cannot send headers either),
`GET /api/items/{id}/subtitles?session=` serves the WebVTT sidecar, and
`GET /api/playback/options` publishes the ladder and preferred
delivery to authenticated players.

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
