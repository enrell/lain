# Changelog

All notable changes to the Lain media server are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [Unreleased]

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
