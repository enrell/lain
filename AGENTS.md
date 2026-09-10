# Lain — Agent Instructions

Read this file before touching code. The project's discipline applies here,
adapted to Go.

## Stack (do not change without explicit user instruction)

- Language: Go (stdlib-first). Dependencies, each justified:
  - `golang.org/x/crypto` (bcrypt) — password hashing.
  - `golang.org/x/term` (password prompt) — `lain login` only.
  - `go.etcd.io/bbolt` (embedded KV, pure Go, no cgo) — lain.db.
  New dependencies are permanent compile tax — discuss first, and
  never accept cgo or C-transpiled giants: the build must stay seconds.
- HTTP: stdlib `net/http` with method patterns. No framework.
- Storage: bbolt buckets (`users`, `libraries`, `items`, `progress`,
  `meta`) via `internal/kv`; JSON values, composite keys for scoping.
  `internal/store` remains for operator config + legacy import only.
- JWT: HS256 in `internal/auth` (stdlib hmac/sha256) with live role,
  liveness and password-version checks.
- No cgo. The whole point is a seconds-long build; keep it that way.

## Workflow

1. Read the relevant source + `docs/` before changing conventions.
2. Test first for behavior changes (`go test ./...` must stay green).
3. Typed errors with stable codes (`core.Error`); never panic on bad
   input; never silent failures — log or propagate.
4. Small slices. Do not implement transcode/metadata/matrix-daemon mode
   "while here"; contracts for those are reserved, not started.

## Boundaries (load-bearing)

- Plugins implement `core.Provider` and never touch `net/http`,
  process-global state, or another plugin's files.
- Media bytes never enter plugin calls; the gateway streams from disk.
- Catalog owns identity; userstate owns progress; never merge them.
- `merge-many`/`fan-out` modes exist for the metadata slice — do not
  repurpose them early.

## Naming hygiene

- Never commit real release-group, fansub, tracker, indexer or site
  names in code, tests, fixtures, docs or comments.
- Tests and fixtures use clearly fictional placeholders instead:
  `[Fansub-A]`, `[Fansub-B]`, `tracker-exemplo`, and equivalent.
  Generic technical tags (`1080p`, `HEVC`, `Dual-Audio`) are fine.
- Anime/media titles themselves (`Frieren`, `One Piece`) are fine —
  only the group/tracker/publisher identity is banned.
- If a real-world filename is needed to reproduce a parser case, keep
  the structure and swap the group tag for a placeholder before
  committing.

## Commands

```sh
go build ./...          # must be seconds, not minutes
go vet ./...
go test ./...           # full suite, fast
go build -o lain ./cmd/lain
./lain doctor            # environment report
./lain bench scan --path ~/Videos --runs 3   # scan benchmark -> benchmarks/
./lain login --server http://127.0.0.1:9360   # save API token
./lain watch --next                           # resume in mpv
```

## Commit discipline

Short imperative subjects. This repo (`projects/lain`) is the server;
the desktop client lives in `projects/lain-desktop`. Do not mix them.
