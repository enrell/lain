# Lain — Agent Instructions

Read this file before touching code. The project's discipline applies
here, adapted to Go.

## Stack (do not change without explicit user instruction)

- Language: Go (stdlib-first). Dependencies, each justified:
  - `golang.org/x/crypto` (bcrypt) — password hashing.
  - `golang.org/x/term` (password prompt) — `lain login` only.
  - `go.etcd.io/bbolt` (embedded KV, pure Go, no cgo) — lain.db.
  - `github.com/fsnotify/fsnotify` (pure Go, no cgo) — library file
    watcher that reconciles deletions in near real time (D-068).
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

## Testing layers (what to use, where, how fast)

The suite is layered so the per-commit path stays seconds while the
expensive engines run scoped or nightly:

```
per commit (seconds):   build + vet → unit/integration → property → race (hot pkgs)
per parser change:      fuzz corpus replay (always) + campaign (time-boxed)
nightly / scheduled:    longer fuzz campaigns → more model seeds → stress -count=20
per refactor/migration: differential oracle runs
```

- **Contract tests** — `internal/testutil/contract` runs one shared
  harness against every `core.Provider`: non-empty ID/capabilities,
  `Health` never panics, unknown capabilities and wrong input types
  return typed `*core.Error`, declared capabilities accept valid
  samples. Operational failures (closed db, missing ffmpeg) may
  surface untyped — the typed contract covers protocol violations only.
- **Native fuzzing** — `go test -fuzz` on untrusted input parsers:
  `identify` (filenames), `probe` (ffprobe JSON), `gateway` theme CSS,
  `transcode` HLS playlists. Fuzzing found real bugs (NaN durations,
  control-char titles, negative geometry); failing inputs live as
  regression corpus under `testdata/fuzz/` — keep them. Run one package
  at a time (`-fuzz` rejects multiple packages):

  ```sh
  go test ./internal/plugins/identify -fuzz=FuzzIdentifyAnime -fuzztime=60s
  go test ./internal/plugins/probe -fuzz=FuzzParse -fuzztime=60s
  go test ./internal/plugins/transcode -fuzz=FuzzParseHLSPlaylist -fuzztime=60s
  go test ./internal/gateway -fuzz=FuzzParseOmarchyTheme -fuzztime=60s
  ```

- **Property tests** — stdlib `testing/quick` (no PBT dependency):
  kv key injectivity and JSON round-trips, `TitleKey` idempotence and
  ASCII case-insensitivity (unicode case-folding is NOT guaranteed
  symmetric — do not assert it), fingerprint determinism, HLS rewrite
  idempotence and token-leak-freedom, contract JSON stability.
- **Stateful models** — seeded random walks with a parallel model:
  `ingest/model_test.go` mutates a real tree (create/delete/rename/
  restore/broken walk) and asserts catalog=missing semantics plus a
  fresh-scan differential oracle; `transcode/lifecycle_model_test.go`
  drives job lifecycles through gated converts asserting legal
  transitions, `done` closing exactly once, and admission bounds.
  Seeds are subtests — a failure names its seed for replay.
- **Deterministic timers** — `testing/synctest` for debounce/throttle
  logic (see `gateway/watch_synctest_test.go`). Inside the bubble the
  root goroutine's `time.Sleep` drives the fake clock and
  `synctest.Wait()` settles goroutines — `Wait` alone does NOT advance
  the clock. Keep real fd I/O (fsnotify, sockets) outside the bubble.
- **Fault injection** — `fault_test.go` files assert degradation, not
  crashes: closed DB (reads empty, writes error), unwritable/vanished
  cache, missing source files, broken walks. A failed walk or an
  identify failure must never read as deletions (D-068).

## Commit discipline

Short imperative subjects. This repo (`projects/lain`) is the server;
the desktop client lives in `projects/lain-desktop`. Do not mix them.

## Repository language

- English is the canonical language for source code, comments, tests, plans,
  project documentation, contributor instructions, and project wikis.
- Other languages are allowed only in explicitly identified translations,
  localization resources, or language-specific documentation variants.
- User-facing text must be translatable; do not hard-code a second language in
  source files as a substitute for localization.

## Advisor protocol (mandatory for long tasks)

An `advisor` subagent owns project direction. It reads
`docs/advisor/decisions.md` (citable `D-XXX`), `docs/advisor/taste.md`,
`docs/advisor/open-questions.md`, plus this file and `docs/`.
It has no edit/shell rights; it only answers. Web is for external
technical facts, never overrides the wiki.

The executor (you) MUST call it via the `subagent` tool with
`agent: advisor` when any of these appear:

- direction/scope doubt, conflict between decisions, irreversible or
  permanent-cost choice (dependency, cgo, HTTP framework, public API
  contract, starting a reserved slice)
- need for external validation before an important change
- anything listed in `docs/advisor/taste.md` as never-decide-alone

Call format: objective + current plan + specific questions (max 5) +
relevant file paths. Load skill `advisor-triage` output contract and
obey it: `answered` cite `D-XXX` and proceed; `needs-user` append the
row to `docs/advisor/open-questions.md` and STOP that slice until the
user answers. Plans MUST cite decisions (`D-001`, ...) for every
load-bearing choice.
