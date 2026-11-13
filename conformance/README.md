# Conformance suite (D-171)

The language-neutral spec of the server's HTTP API. It froze the Go
server's behavior before the Rust port (D-171), which passed it route by
route before Go was removed (D-240); it now runs against `lain` only.
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
                  overlays, overlays from the network providers against
                  the upstream fixtures (35, needs `upstream`), browse,
                  item delete, incremental rescans (v1), scan events (v2);
                  writes its own fixture media
  cases/lists/    the lists module from a fresh server: tracking list,
                  links, OAuth callback and pin flows against the upstream
                  fixtures, sync, scrobble preference, unlink    (v1)
  cases/social/   the social module from a fresh server: settings,
                  friends, blocks, search, profiles, ratings, comments,
                  feed, notifications, shares, collections        (v1)
  cases/private/  the server over a populated catalog: catalog items, paging,
                  episodes, per-user progress, then private libraries:
                  nothing leaks to another user or a locked session, social
                  rows included (v2)
  cases/acquire/  acquire and downloads from a fresh server (v1), their events
                  (v2); nothing reaches the network (D-213)
  cases/read/     the server over the read fixture: comic pages, page
                  bytes, ETags, direction, refusals, private libraries
                  and module disable (v2)
  cases/watch/    the watch module over two real clips: playback plans,
                  streams and ranges, thumbnails, transcode sessions
                  (progressive, HLS with signed URIs, viewers sharing
                  one session, v1), subtitles,
                  settings, local play (v1), and every
                  watch route of a private library (v2)
  fixtures/       catalog.json, the lain.catalog-export@3 document the
                  private suite starts from; parse-parity.json, the retired Go
                  parsers' answers the Rust parsers must give (frozen;
                  its Go generator is gone, D-240); read-catalog.json
                  and read/ (small CBZ archives) for the read suite;
                  watch-catalog.json, the watch suite's catalog;
                  upstream/, recorded answers of the remote APIs the
                  network providers call (routes.json, D-203)
  make-clips.sh   writes the watch clips with ffmpeg's lavfi sources
  component/      NDJSON fixtures of the component protocol
  runner/         lain-conformance, one runner (Rust, std-only HTTP), and
                  `lain-conformance upstream`, the fixture server for
                  upstream/
  run.sh          the server end to end in throwaway data dirs
```

Tier `v1` is the `/api/*` that D-169 froze (what the Go server served);
tier `v2` is what D-154/D-169 add.

## Run it

Everything (builds `lain`, temp data dirs, port 19461, the upstream
fixture server on 19462 so nothing reaches the network; never your real
data dir):

```sh
conformance/run.sh
```

Against a server you started yourself:

```sh
cargo build -p lain-conformance -p lain
R=target/debug/lain-conformance
L=target/debug/lain
mkdir -p /tmp/m/a /tmp/m/b

# fresh data dir; data_dir lets the v2 cases toggle modules.json
$L serve --data-dir /tmp/lain-c --port 19461 &
LAIN_URL=http://127.0.0.1:19461 $R --var media_dir=/tmp/m/a --var media_dir2=/tmp/m/b \
  --var data_dir=/tmp/lain-c conformance/cases/fresh conformance/cases/v2
```

Scan (watching and the post-scan metadata pass off so no scan races the
cases; the network metadata providers point at the upstream fixture
server):

```sh
$R upstream --port 19462 &          # recorded remote APIs
U=http://127.0.0.1:19462
export LAIN_METADATA_KITSU_URL=$U/kitsu LAIN_METADATA_ANILIST_URL=$U/anilist \
  LAIN_METADATA_JIKAN_URL=$U/jikan LAIN_METADATA_TVMAZE_URL=$U/tvmaze
LAIN_WATCH=0 LAIN_AUTO_ENRICH=0 $L serve --data-dir /tmp/lain-s --port 19461 &
LAIN_URL=http://127.0.0.1:19461 $R --var scan_media=/tmp/lain-media --var upstream=$U conformance/cases/scan
```

Private (the catalog comes from the fixture): write `<data dir>/roots.json` as
`{"format":"lain.roots@1","roots":{"media":"<dir>"}}` with a `Frieren/`
directory in `<dir>`, run `lain import --in
conformance/fixtures/catalog.json --data-dir <data dir>`, start
`lain serve` there and run `cases/private` with `--var media_root=<dir>
--var data_dir=<data dir>`.

Read: write `roots.json` mapping `media` to `conformance/fixtures/read`,
`lain import --in conformance/fixtures/read-catalog.json`, start
`lain serve` and run `cases/read` with `--var media_root=<that dir>
--var data_dir=<data dir>`. The archives were written once with Python's
`zipfile` (fixed timestamps); the catalog's `size` fields are their byte
sizes, which the page ETags carry.

`run.sh` points `LAIN_OMARCHY_COLORS` at a missing file, so the theme
case never reads the host's own colors.

Watch (needs ffmpeg; D-194): `conformance/make-clips.sh <dir>/Clips`
writes `Alpha.mp4` and `Beta.mkv`; write `roots.json` mapping `media` to
`<dir>`, `lain import --in conformance/fixtures/watch-catalog.json`,
then `--var clips=1 --var media_root=<dir>`. `run.sh` skips the suite
without ffmpeg unless `LAIN_REQUIRE_FFMPEG=1`. (A `scan_dir` variable,
where the suite creates the library and scans it, was the Go server's
path and is no longer run.)

There is no seeded or imported suite: nothing moves from a Go data dir
(D-177).

`cargo test` also runs `fresh`, `v2`, `private` and the other suites
against an in-process server, and replays `component/` against the
Rust host and the Rust component side.

## Case format

See the module docs of `runner/src/cases.rs` (steps: `req`, `wait_for`,
`sse_open`, `sse_expect`, `write_file`, `remove_file`, `rename_file`, `sleep_ms`) and
`runner/src/matcher.rs` (`$string`, `$int`, `$prefix:`, `$absent`,
`$open`, `$each`, `$contains`, `$substr:`, ...). `{{var}}` substitutes variables
saved by earlier steps or passed with `--var`. Numbers compare as
numbers, so `1440` and `1440.0` are the same value.

## Not covered yet

Plugin swap and withdraw writes, and the `lain.catalog-export`
documents as files.
