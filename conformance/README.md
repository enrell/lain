# Conformance suite (D-171)

The language-neutral spec of today's server behavior. Both servers run
it; a route moves from Go to Rust only when the Rust side passes it.
It is data, not code: any language can run it.

```text
conformance/
  cases/fresh/    HTTP cases from an empty server: health, setup, login,
                  /api/me (+ profile, password, preferences), users,
                  libraries, catalog, progress, plugins          (tier v1)
  cases/seed/     seeds a Go server (accounts, a library, a scan, progress)
  cases/seeded/   reads over seeded state: catalog items, paging, episodes,
                  per-user progress, a token minted before the move (v1)
  cases/v2/       /api/modules and modules.json, /api/events (SSE)   (v2)
  component/      NDJSON fixtures of the component protocol
  runner/         lain-conformance, one runner (Rust, std-only HTTP)
  run.sh          both servers end to end in throwaway data dirs
```

Tier `v1` is today's `/api/*` (frozen by D-169) and must pass on Go
first; it is the spec. Tier `v2` is what D-154/D-169 add, which only the
Rust server serves.

## Run it

Everything (builds both, temp data dirs, ports 19460/19461; never your
real data dir):

```sh
conformance/run.sh          # or: conformance/run.sh go | rust
```

Against a server you started yourself:

```sh
cargo build -p lain-conformance
R=target/debug/lain-conformance
mkdir -p /tmp/m/a /tmp/m/b

# Go (fresh data dir)
go run ./cmd/lain serve --data-dir /tmp/go-c --port 19460 &
LAIN_URL=http://127.0.0.1:19460 $R --tier v1 \
  --var media_dir=/tmp/m/a --var media_dir2=/tmp/m/b conformance/cases/fresh

# Rust (fresh data dir; data_dir lets the v2 cases toggle modules.json)
cargo run -p lain-server --bin lain-rs -- serve --data-dir /tmp/rs-c --port 19461 &
LAIN_URL=http://127.0.0.1:19461 $R --var media_dir=/tmp/m/a --var media_dir2=/tmp/m/b \
  --var data_dir=/tmp/rs-c conformance/cases/fresh conformance/cases/v2
```

Seeded: run `cases/seed` against a fresh Go server with
`--var media_dir=<dir with two "[Fansub-A] Frieren - 0N [1080p].mkv">
--save-vars seed.json`, stop it, `lain-rs import --from <its data dir>
--data-dir <new dir>`, start `lain-rs` there and run `cases/seeded` with
`--vars seed.json` (and the same against the Go server).

`cargo test` also runs `fresh`, `v2` and `seeded` (over the bbolt
fixture in `crates/lain-store/testdata`) against an in-process
`lain-rs`, and replays `component/` against the Rust host and the Rust
component side; `go test ./internal/component` replays it against the
Go component side.

## Case format

See the module docs of `runner/src/cases.rs` (steps: `req`, `wait_for`,
`sse_open`, `sse_expect`, `write_file`, `remove_file`, `sleep_ms`) and
`runner/src/matcher.rs` (`$string`, `$int`, `$prefix:`, `$absent`,
`$open`, `$each`, `$contains`, ...). `{{var}}` substitutes variables
saved by earlier steps or passed with `--var`. Numbers compare as
numbers, so Go's `1440` and Rust's `1440.0` are the same value.

## Not covered yet

Search, scan, browse, theme, playback/stream/transcode/thumbnail,
avatars, pages, lists, social, acquire, downloads, backup, plugin swap
and withdraw writes, and the `lain.catalog-export` documents as files.
These move with their modules (DESIGN-v2 §13 order).
