# lain server decisions (citable in plans)

Use these IDs in plans and advisor calls: `D-001`, etc.
Update this file when the user answers a `needs-user` question.

## Stack and build

- `D-001` — Prefer the Go standard library. The only three justified dependencies are `golang.org/x/crypto` for bcrypt, `golang.org/x/term` for login, and `go.etcd.io/bbolt` for `lain.db`. A new dependency is a permanent cost and must be discussed first. Source: `AGENTS.md:6`.
- `D-002` — No cgo and no large C-transpiled dependencies. Builds must remain measured in seconds. Source: `AGENTS.md:20`.
- `D-003` — Use standard-library `net/http` method patterns without an HTTP framework. Source: `AGENTS.md:14`.
- `D-004` — Store data in bbolt through `internal/kv` using JSON values and composite keys in the `users`, `libraries`, `items`, `progress`, and `meta` buckets. Keep `internal/store` only for operator configuration and legacy import. Source: `AGENTS.md:15`.
- `D-005` — Use HS256 JWT in `internal/auth`, implemented with standard-library HMAC/SHA-256 and live role, liveness, and password-version checks. Source: `AGENTS.md:18`.

## Boundaries

- `D-006` — Plugins implement `core.Provider` and never touch `net/http`, process-global state, or another plugin's files. Source: `AGENTS.md:33`.
- `D-007` — Media bytes never enter plugin calls; the gateway streams from disk. Source: `AGENTS.md:35`.
- `D-008` — Catalog owns identity and userstate owns progress; never merge them. Source: `AGENTS.md:36`.
- `D-009` — The `merge-many` and `fan-out` modes belong to the metadata slice and must not be repurposed early. Transcode, metadata daemon, and Matrix daemon work begin only in their explicit slices. Source: `AGENTS.md:28`.

## Hygiene and discipline

- `D-010` — Never commit real release-group, fansub, tracker, indexer, or site names. Use fictional placeholders such as `[Fansub-A]`, `[Fansub-B]`, and `tracker-example`. Anime titles are allowed; group identities are not. Source: `AGENTS.md:42`.
- `D-011` — Use typed errors with stable `core.Error` codes; never panic on bad input and never fail silently. Source: `AGENTS.md:26`.
- `D-012` — Work in small slices and keep `go test ./...` green for behavior changes. Source: `AGENTS.md:24`.
- `D-013` — This repository is the server; the desktop client lives in `projects/lain-desktop`. Do not mix their commits. Source: `AGENTS.md:68`.
- `D-014` — English is the only canonical repository language for code, comments, tests, plans, project documentation, contributor instructions, and wikis in both repositories. Non-English content is limited to explicitly identified translations, localization resources, and language-specific documentation variants. Source: interview 2026-09-11.

## Product direction shared with lain-desktop

- `D-015` — Server evolution required by the desktop uses versioned API contracts, with implementation, tests, and commits kept separate in each repository. Source: interview 2026-09-11.
- `D-016` — Anime, movies, and series have product parity; the committed media scope is personal video, excluding music, audiobooks, photos, and live TV. Source: interview 2026-09-11.
- `D-017` — The Linux desktop may provision the server through environment-appropriate Docker, service, or binary methods using Polkit for privileged work, but may control only installations it provisioned. Source: interview 2026-09-11.
- `D-018` — After completing first use, stable identity and a correct collection model take priority over advanced playback, automation, or remote access. Source: interview 2026-09-11.
- `D-019` — Media identity v2 keeps catalog IDs stable across file moves and renames: rescans reconcile by logical fingerprint (library, kind, normalized title, season, episode, compatible year) and adopt the existing ID onto the new path with the previous path in `aliases`, guarded by the scan present-set so genuine duplicates never merge. Export format bumped to `lain.catalog-export` version 2 (import accepts 1 and 2); `ScanStats` gains `migrated`. Source: user answer QQ-S4-03, 2026-09-11.
