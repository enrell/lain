# Changelog

All notable changes to the Lain media server are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [Unreleased]

### Added
- Asynchronous browser transcoding (`lain.playback.transcode@2`,
  composition v3): `POST /api/items/{id}/transcode` starts or joins an
  idempotent session, `GET .../transcode/status?session=` polls
  `idle|queued|running|ready|failed`, and playback reports the opaque
  session plus profile/state without starting work. One global worker,
  bounded queue, 202 + `Retry-After` while pending; the synchronous
  `@1` endpoint stays compatible.
- Bounded transcode cache: plugin-private 20 GiB LRU (configurable via
  `--transcode-cache-size` / `LAIN_TRANSCODE_CACHE_SIZE`), atomic
  sidecars, startup/admission/completion cleanup, stale and abandoned
  temporary removal.
- Probe-driven browser compatibility (`lain.media.probe@1`,
  composition v4): ffprobe stream inspection replaces extension
  guesses; single-audio selection, WebVTT subtitle sidecars
  (`GET /api/items/{id}/subtitles?session=`) with player track
  pickers, and explicit HDR refusal until a tone-map profile exists.

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
