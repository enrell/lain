# Conformance suite (D-171)

The language-neutral spec of today's server behavior. Both servers run
it; a route moves from Go to Rust only when the Rust side passes it.
It is data, not code: any language can run it.

```text
conformance/
  cases/fresh/    HTTP cases from an empty server: health, setup, login,
                  /api/me (+ profile, password, preferences), users,
                  libraries, catalog, progress, plugins          (tier v1)
  cases/fresh/45  avatars; 85: theme, integrations, backup           (v1)
  cases/v2/       /api/modules and modules.json, /api/events (SSE)   (v2)
  cases/scan/     catalog population from a fresh server: scan, identified
                  catalog, grouping, search, sidecars (WebVTT), NFO metadata
                  overlays, browse, item delete, incremental rescans (v1),
                  scan events (v2); writes its own fixture media
  cases/private/  lain-rs over a populated catalog: catalog items, paging,
                  episodes, per-user progress, then private libraries:
                  nothing leaks to another user or a locked session (v2)
  cases/read/     lain-rs over the read fixture: comic pages, page
                  bytes, ETags, direction, refusals, private libraries
                  and module disable (v2)
  fixtures/       catalog.json, the lain.catalog-export@3 document the
                  private suite starts from; parse-parity.json, the Go
                  parsers' answers the Rust parsers must give (written by
                  internal/plugins/identify/parity_test.go); read-catalog.json
                  and read/ (small CBZ archives) for the read suite
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

Scan (both servers; watching and the post-scan metadata pass off so no
scan races the cases and nothing reaches the network):

```sh
LAIN_WATCH=0 LAIN_AUTO_ENRICH=0 go run ./cmd/lain serve --data-dir /tmp/go-s --port 19460 &
LAIN_URL=http://127.0.0.1:19460 $R --tier v1 --var scan_media=/tmp/go-media conformance/cases/scan
```

Private (Rust only; the catalog comes from the fixture): write `<data dir>/roots.json` as
`{"format":"lain.roots@1","roots":{"media":"<dir>"}}` with a `Frieren/`
directory in `<dir>`, run `lain-rs import --in
conformance/fixtures/catalog.json --data-dir <data dir>`, start
`lain-rs` there and run `cases/private` with `--var media_root=<dir>
--var data_dir=<data dir>`.

Read (Rust only, for the same reason): write `roots.json` mapping
`media` to `conformance/fixtures/read`, `lain-rs import --in
conformance/fixtures/read-catalog.json`, start `lain-rs` and run
`cases/read` with `--var media_root=<that dir> --var data_dir=<data
dir>`. The archives were written once with Python's `zipfile` (fixed
timestamps); the catalog's `size` fields are their byte sizes, which the
page ETags carry.

`run.sh` points `LAIN_OMARCHY_COLORS` at a missing file, so the theme
case never reads the host's own colors.

There is no seeded or imported suite: nothing moves from a Go data dir
to lain-rs (D-177).

`cargo test` also runs `fresh`, `v2` and `private` against an
in-process `lain-rs`, and replays `component/` against the Rust host and the Rust
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

Playback/stream/transcode/thumbnail, lists, social, acquire, downloads,
plugin swap and withdraw writes, pages on the Go server (it has no
fixture import), and the `lain.catalog-export` documents as files.
These move with their modules (DESIGN-v2 §13 order).
