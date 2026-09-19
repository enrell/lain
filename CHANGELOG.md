# Changelog

All notable changes to the Lain media server are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [Unreleased]

### Added
- The web UI is title-centric. `/item/{id}` is now the anime/series/movie
  page — hero, poster rail, season filter and an episode grid — while a
  single-file title (a movie or a lone special) keeps the plain item
  page. `/player/{id}` becomes the watch page: the video with the title's
  episodes listed beside it, and the route follows the id param instead
  of loading once, so switching episodes cannot leave the previous
  episode's state or its marker on screen. Backed by a new additive read,
  `GET /api/catalog/{id}/episodes`, returning every file of the same
  title in watch order; the catalog owns the grouping rule (D-056).
- On-the-fly subtitle extraction (D-047): `GET
  /api/items/{id}/subtitles?stream=N` serves WebVTT for a chosen track
  of a **directly played** file, cached like any other derivative, so
  subtitles no longer require a transcode. A new
  `allow_subtitle_extraction` server setting (absent = on) turns it off
  with a stable `forbidden`.
- Per-stream stream-copy permissions (`allow_video_stream_copy`,
  `allow_audio_stream_copy`), mirroring Jellyfin's `PlaybackInfo`
  flags: an explicit false forces a re-encode of that stream, and the
  status now reports the outcome as `video_direct`/`audio_direct`.
- `deinterlace_method` (`yadif` or `bwdif`, with a visible fallback to
  `yadif` when the build lacks bwdif).
- Jellyfin-parity advanced transcode configuration (D-046): transcoding
  thread count, max muxing queue size, per-codec preset and CRF
  (H.264/H.265/AV1), audio VBR, downmix gain and stereo algorithm,
  deinterlace double rate, burn-in font fallback directory and family,
  per-codec 10-bit hardware-decode toggles, a server-wide remote client
  bitrate limit, and a configurable transcoding temporary path that
  relocates artifacts to `<path>/lain-transcode/<session>/` while the
  JSON index stays with the data dir.
- A configurable VA-API render node (`hardware_device`, default
  `/dev/dri/renderD128`), used by both the capability probe and the
  decode/encode `-hwaccel_device` argument, with a field in the
  Settings → Playback hardware section.
- Rockchip `rkmpp` joins the hardware backends, so the list matches
  Jellyfin's documented acceleration methods (VA-API, NVENC, QSV, AMF,
  V4L2, VideoToolbox, RKMPP). It is probed like every other backend and
  stays unavailable where the local ffmpeg lacks it.
- Per-user simultaneous stream limit (`max_streams`) enforced with a
  stable `too-many-streams` / HTTP 429, and a per-user default subtitle
  mode; both join the existing allow-video/audio-transcode, allow-remux
  and max-bitrate limits, and the admin session list now names the
  owning account.
- Live transcode metrics in the session status (`fps`,
  `output_bitrate_kbps`) read from ffmpeg's progress stream, shown in
  the Settings → Playback session list.
- Full browser transcode pipeline (`lain.playback.transcode@3`,
  composition v5, D-042..D-045): per-session options (quality ladder,
  resolution/bitrate caps, H.264/HEVC/AV1 output, audio codec and
  track, subtitle track and mode, delivery) and pipeline facts in the
  status (encoder actually used, hardware backend, software-fallback
  reason, why the transcode is needed, `playable` for HLS).
- HLS delivery with a server-owned `EVENT` playlist
  (`GET /api/items/{id}/transcode/hls/{file}`), playable while ffmpeg
  still runs, alongside the retained progressive MP4. The segment
  container is selectable (`fmp4` CMAF or `mpegts`). Throttling
  (SIGSTOP/SIGCONT against the client's fetched segment), optional
  segment deletion with a keep window, and idle-session cleanup.
- Hardware acceleration, opt-in only: real probe of the configured
  backend (VAAPI/NVENC/QSV/AMF/V4L2/VideoToolbox/RKMPP), per-codec
  hardware decoding, QSV low-power encoding, a configurable VA-API
  render node, and a visible software fallback when the probe or the
  run fails — never silent switching (D-031).
- HDR tone mapping (`zscale`+`tonemap`, libplacebo BT.2390 when its
  probe passes, with a visible downgrade to `hable` otherwise),
  auto-deinterlacing, quality-based scaling, audio downmix, and
  subtitle burn-in for image and styled text tracks.
- Operator surface: `GET/PUT /api/admin/settings/transcode` (validated,
  persisted in the bbolt `meta` bucket, applied to new sessions without
  a restart) returning the probed capabilities, `GET
  /api/admin/transcodes` plus `DELETE /api/admin/transcodes/{session}`
  for live sessions, `GET /api/playback/options` for the player ladder,
  and a transcode section in `lain doctor`.
- Per-user playback limits on the account: allow video transcoding,
  audio transcoding and remuxing, plus a maximum output bitrate; the
  gateway resolves them into the session and the plugin refuses what
  the user may not have (`forbidden`).
- Web: a Settings → Playback tab covering the whole policy (delivery,
  throttling, segment deletion, encoder preset/CRF per codec, codec
  permissions, quality ladder editor, hardware backend, decode codecs
  and 10-bit toggles, tone mapping, audio VBR/downmix/font fallback,
  thread count, muxing queue, cache/queue/concurrency, transcoding temp
  path, ffmpeg paths) with a live session list, a quality menu in the
  player with the transcode reasons shown, and per-user limits in the
  Users page. HLS playback uses `hls.js` (web-only dependency; D-043).
- Asynchronous browser transcoding (`lain.playback.transcode@2`,
  composition v3): `POST /api/items/{id}/transcode` starts or joins an
  idempotent session, `GET .../transcode/status?session=` polls
  `idle|queued|running|ready|failed`, and playback reports the opaque
  session plus profile/state without starting work. One global worker,
  bounded queue, 202 + `Retry-After` while pending; the synchronous
  `@1` endpoint stays compatible.
- Bounded transcode cache: plugin-private 20 GiB LRU (configurable via
  `--transcode-cache-size` / `LAIN_TRANSCODE_CACHE_SIZE`, or the admin
  UI), atomic sidecars, startup/admission/completion cleanup, stale and
  abandoned temporary removal.
- Probe-driven browser compatibility (`lain.media.probe@1`,
  composition v4): ffprobe stream inspection replaces extension
  guesses; single-audio selection, WebVTT subtitle sidecars
  (`GET /api/items/{id}/subtitles?session=`) with player track
  pickers, and explicit HDR refusal until a tone-map profile exists.
- Transcode sessions can begin at a source position (`start_sec`): a
  resume or a seek now seeks the input instead of re-encoding from the
  beginning, and the player offsets its clock and progress accordingly.

### Fixed
- The player's seek bar only ever reached as far as ffmpeg had encoded.
  An HLS session is an `EVENT` playlist listing just the produced
  segments, so under MSE the element's duration — and therefore
  `seekable` — was the produced edge: a 24-minute episode showed a 0:37
  timeline and refused a drag to minute 13. The plan response now carries
  the probed media length (`duration_sec`, the same fact Jellyfin's
  `PlaybackInfo` exposes as `RunTimeTicks`) and the player hands it to
  hls.js as a MediaSource duration override, so the bar spans the episode
  and progress is reported against the real length instead of the encoded
  prefix. A seek past what the encoder has written rebuilds the session
  at that position through `start_sec`, so it starts playing there rather
  than waiting for the encoder to arrive; a seek inside the produced range
  stays an ordinary, instant element seek. The buffered region is now
  drawn where it actually is, which a resumed or re-seeked session starts
  partway into. Found by the user watching a real episode.
- HLS playback in a real browser: the served playlist now signs every
  media URI with the session and the caller's token. A relative URI in
  an m3u8 drops the playlist's query string, so `hls.js` and native
  Safari fetched `init.mp4` and the segments unauthenticated and every
  request returned 401 — HLS never started. The signed URIs keep the
  existing `?token=` model and also work for native HLS. Covered by a Go
  regression test plus real-Chromium steps that assert the HLS playlist,
  an fMP4 segment and advancing playback, the player quality menu
  rebuilding the session, the retained progressive MP4 with Range,
  on-the-fly subtitle extraction rendering real cues during direct play,
  the advanced Settings → Playback UI, and the per-user playback limits
  dialog.
- Enabling hardware acceleration **without** hardware encoding did
  nothing: the plan reported hardware decode but `ffmpeg` received no
  `-hwaccel`, so the decode half stayed on the CPU. The decode hwaccel is
  now emitted from the configured backend, the runtime software fallback
  covers decode-only failures, and the session status names the backend
  actually in use.
- A configured transcoding temp path resolved to a doubled
  `lain-transcode/lain-transcode/<session>` directory when a finished
  session was reopened from its sidecar: `artifact_root` already carried
  the owned namespace and the resolver appended it again. Both sides now
  agree, and the previously-skipping round-trip test asserts it.
- The transcode test helper `makeMKV` placed `-pix_fmt` before a second
  `-i`, so `ffmpeg` rejected it and `TestPrepareTranscodesIncompatibleStreams`
  silently skipped; inputs now precede output options and the test runs.
- A VA-API session that also burns in an image subtitle kept its
  `hwupload`/`format=nv12` chain in the software retry: `softwareFallback`
  cleaned `filters` but not the burn-in `complexFilter`, so a failed
  hardware attempt failed again instead of falling back. The upload tokens
  are now stripped from both. Found by an adversarial subagent test.
- The requested bitrate cap was ignored whenever the plan chose a copy, so
  a web-safe source above the cap was remuxed at full bitrate while the
  status reported the cap; a cap below the probed source bitrate now
  forces a re-encode.
- An HLS session extracted its WebVTT sidecar only after the whole file
  had been produced, so it never offered a subtitle track while running
  (and the endpoint refused any session that was not ready). The sidecar
  is now extracted before the encode and served as soon as it exists.
- A source that already matched the requested MP4 audio codec was
  re-encoded unless the codec was AAC, and an unknown `audio_codec`
  reached the `ffmpeg` argv verbatim; matching codecs are copied and
  unknown ones are rejected.
- The ready-cache LRU could delete a fully produced HLS session while a
  client was still fetching its segments. A session accessed in the last
  minute is no longer an eviction victim (the overage is logged instead).
- `nightmode` downmixing applied its 5.1 coefficient matrix to any
  surround source; it now requires a real 5.1 source and says so in the
  session reasons, falling back to ffmpeg's layout-aware downmix.
- The reason a copy was refused by a client-side flag was overwritten by
  the later reason pass, so the status hid a cause it had recorded.
- The served HLS playlist dropped the `#EXT-X-DISCONTINUITY` ffmpeg
  wrote, and the parser attributed it to the segment *before* it (HLS
  puts the tag before the segment that starts the new timeline). A
  client could therefore stitch across a timestamp jump. The tag is now
  attributed to the following segment and re-emitted between segments.
- `position` was a high-water mark, so after a rewind ffmpeg kept racing
  ahead and segment deletion removed the very segments the client was
  about to re-fetch. The latest report now wins, matching Jellyfin's use
  of the reported playback position; an out-of-order report only makes
  the position older, so the change errs toward pausing sooner and
  keeping more segments, never toward deleting or racing ahead.
- An H.264/MP3 MP4 was transcoded even though browsers decode MP3 in
  MP4 as reliably as AAC. MP3 now direct-plays; AC-3/E-AC-3 stay
  excluded (Chromium on Linux cannot decode them).

## [0.2.0] - 2026-09-11

### Added
- Stable catalog identity across moves and renames: rescans reconcile
  by logical fingerprint and adopt the existing ID onto the new path,
  recording the previous path in `aliases`. Progress and enrichments
  survive moves untouched.
- Catalog export format version 2 (import accepts versions 1 and 2);
  scan stats report a `migrated` count.
- Installer: desktop launcher with icon and Lain identity, terminal
  setup wizard, and tiered uninstall.

### Fixed
- Installer media prompt keeps its default instead of losing it.

## [0.1.0] - 2026-09-10

- Initial public release: Go gateway with plugin composition, Bolt
  catalog, JWT auth, direct-play plans, metadata providers, library
  scanning, Docker image, and web administration.
