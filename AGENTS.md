# Lain — Agent Instructions

Read this file before touching code.

## Audience (drives every UX decision)

Lain is not for the general public. Its users are developers and other
technical people who run their own servers and are comfortable with
power tools; people who only want "a Netflix" never install a media
server. Design for them:

- **Keyboard-first.** Every flow must be completable without a mouse:
  modals trap focus and land it on the primary action, every action has
  a key, and the key is shown next to the action (`<kbd>`). A screen
  that needs the mouse is a bug, not an accessibility nicety.
- Dense, fast, direct. Prefer shortcuts, visible state and precise
  controls over wizards, hand-holding copy and hidden magic.
- Settings follow one shape (D-087): a YOU/SERVER rail, one row pattern
  (`SettingRow`: label + one-line hint left, control right), personal
  settings apply instantly, server settings stage behind one Ctrl+S bar.
  A new setting also gets an entry in `web/src/lib/settings/sections.ts`
  so Ctrl+K can find it.
- Post-modern visual language (see `docs/advisor/decisions.md` D-022):
  full-bleed art, mono uppercase labels, large tight type, glass panels.
  No boxed "banner cards" or dated framed layouts.

## Stack (do not change without explicit user instruction)

Lain is one Rust workspace (D-171): the server, the client core and the
`lain` binary. The Go server it replaced was removed (D-235..D-244); the
conformance suite it was checked against stays as the API spec.

- Workspace: `Cargo.toml` at the root, crates in `crates/`:
  - `lain-core` — composition, provider registry (modes, health-gated
    swap, generation fencing, last-good fallback, withdraw, module
    suspend/resume) and the module layer (`lain.modules@1`, D-154).
  - `lain-store` — redb tables of JSON records with NUL-joined
    composite keys. A data dir starts empty (D-177).
  - `lain-auth` — setup, Argon2id passwords and PINs, HS256 tokens with
    a session `jti` and live checks.
  - `lain-component` — the component host, `serve` for Rust components
    and the `<data-dir>/plugins/` watcher; the NDJSON wire frozen in
    `conformance/component/`.
  - `lain-read` — the read module (D-195): comic and manga archives
    (`lain.comic.pages@1`), the in-tree ZIP reader, image headers and
    `ComicInfo.xml`.
  - `lain-admin` — core administration (D-196..D-199): the theme
    (`lain.ui.theme@1`), backups (`lain.backup.create@1`, tar of a
    consistent store snapshot), integration settings and avatars.
  - `lain-server` — the axum gateway, `/api/modules`, `/api/events`,
    integrations (API keys other programs feed a library with,
    D-265..D-267),
    service mode, catalog `export`/`import` (`lain.catalog-export@3`)
    and the web SPA's static serving (`webui.rs`, D-235), on the
    canonical data dir (`lain`) and port 9360 (D-178). A library: the
    binary is `crates/lain`.
  - `lain` — the binary (D-237): the server commands (`serve`,
    `service`, `export`/`import`, `backup`/`restore`, `plugins`,
    `doctor`, `component`) and the command-line client (`login`, `watch`,
    `download`, `player`, `language`, `open-url`,
    `install-player-handler`) on `lain-client`. Its `build.rs` embeds
    `crates/lain/webui/` (the web build `just web` copies there; empty
    in a fresh checkout) with `include_bytes!`, no crate (D-235).
  - `lain-identify` — the in-tree parsers (D-185): the release-name
    tokenizer (`lain.release.parse@1`) and the comic, anime and generic
    identifiers (`lain.media.identify@1`), checked name for name
    against the recorded answers of the retired Go parsers
    (`conformance/fixtures/parse-parity.json`, frozen).
  - `lain-ingest` — catalog population: the filesystem source, the scan
    pipeline, the `commit_scan` plan (moves D-019, missing D-068,
    locations D-157/D-186), the library watcher on `notify`, the scan
    controller and the folder picker.
  - `lain-metadata` — the metadata module: enrich (local-only for
    private libraries, D-268), the NFO provider,
    the network providers (Kitsu, AniList, Jikan, TVMaze, in-process,
    D-201), the merge and the overlay and cache tables.
  - `lain-subtitle` — sidecar discovery, naming and the moviehash,
    SRT/WebVTT/ASS to WebVTT, and the subtitles module's
    `lain-subtitle-opensubtitles` (`lain.subtitle@1`, D-202).
  - `lain-lists` — the lists module (D-215): the list store, linked
    accounts, the integrations settings provider, the AniList connector
    on `lain-http`, the link state, imports and scrobbles.
  - `lain-social` — the social module (D-216, D-217): privacy settings,
    relationships, activity, notifications, ratings, comments and
    collections, one provider over its own tables, with the D-156 row
    filter.
  - `lain-http` — outbound HTTP(S) for every network provider (D-200):
    one shared client with rustls, timeouts, retries with backoff,
    per-host pacing, size caps and JSON helpers; no server types, so
    `lain-client` can reuse it.
  - `lain-torrent` — the in-house BitTorrent engine (D-205): bencode,
    metainfo and magnets, the peer wire with `ut_metadata`, HTTP/UDP
    trackers, piece checks, rate limits and resume; a synchronous API
    over its own tokio runtime, and `trackertest` for swarm tests.
  - `lain-downloads` — the server download manager (`/api/downloads`,
    D-207): the resumable fetcher, the shared disk budget, safe names,
    the job queue, and the streaming `Http` seam with its `http://`
    client.
  - `lain-acquire` — the acquire module (D-206): Torznab indexers
    (`lain.indexer@1`), grabs on the engine, import, seeding, Phase 2
    automation and Phase 3 subtitles; outbound calls through
    `lain-http` (D-208).
  - `lain-bridge` — the bridges to the owner's standalone apps served
    as components (P4, D-225..D-231): `lain-release-model` (lain-parser,
    `lain.release.parse@1`), `lain-subtitle-animesubs`
    (`lain.subtitle.translate@1`) and `lain-acquire-anything`
    (`lain.acquire.extract@1`), their discovery of each app's
    `server.json` and token, and a std-only HTTP/1.1 client for loopback
    services; `lain component install|run|list|uninstall`. No crate
    added. `tools/parser-v5` (outside the workspace) is the V-5 parser
    benchmark.
  - `lain-api` — the HTTP API's JSON types (serde only), re-exported by
    the server crates and decoded by `lain-client` (D-179).
  - `lain-client` — the client core every native client shares
    (DESIGN-v2 §8.1, D-179): the typed API, sessions (`TokenStore`),
    discovery (`Discovery`), the offline store (D-094's limits and LRU
    eviction, on redb), progress sync with an offline queue and conflict
    rules (`sync.rs`), the event stream, and `https` servers through
    `lain-http`'s rustls setup (`trust_roots` for a private CA, D-277).
    No GUI code; the public surface stays UniFFI-friendly (owned types,
    object-safe traits).
  - `lain-probe` — ffprobe as an external process (D-190): one
    `-show_format -show_streams` run parsed into bounded stream facts
    (`RawReport`, the wire's `MediaInfo`), plus the deadline-bounded
    tool runner and PATH lookup the watch module shares. Small on
    purpose: other modules (subtitles, catalog) read tracks through it.
  - `lain-watch` — the watch module (D-190..D-194): playback planner,
    transcoder (v1 + v3 sessions, HLS, throttle, idle stop, LRU cache)
    over external ffmpeg on its own threads, thumbnails, local playback
    (mpv/VLC) and the transcode settings, each also an in-process
    provider under its historical id (`lain-transcode-ffmpeg`, ...).
  - `conformance/runner` — the conformance runner (`lain-conformance`).
- Dependencies: the D-172 baseline plus D-173 (argon2 per D-177), versions pinned with `=`
  in the workspace `Cargo.toml`, `Cargo.lock` committed. A crate outside
  this list is asked of the owner first (DESIGN-v2 §13 policy: few,
  maintained, pure Rust, no C-heavy dependency in the server; ffmpeg,
  ffprobe, mpv and VLC stay external processes). New dependencies are
  permanent compile tax.
  - `tokio` — the async runtime under axum, and the Windows named-pipe
    transport of the component host (D-174); only the `rt-multi-thread`,
    `net`, `time`, `sync`, `macros`, `signal` and `io-util` features.
  - `axum` / `hyper` — HTTP routing, default features off (`http1`,
    `json`, `query`, `tokio`). The server uses `hyper` directly only for
    its `Body` trait, which the SSE stream implements; `lain-client`
    adds hyper's `client` feature (its HTTP/1 client connection; it
    brings hyper's internal `want` and `try-lock`), with `hyper-util`
    (`TokioIo` only) and `http-body-util`, both already in the tree under
    axum (D-179).
  - `serde` / `serde_json` — the JSON contracts.
  - `redb` — the embedded store, pure Rust, the bbolt equivalent.
  - `argon2` (D-177, RustCrypto, pure Rust) — passwords and library
    PINs as Argon2id PHC strings, default parameters; features `alloc`,
    `password-hash`, `getrandom` (salts). Built with `opt-level = 3` in
    the dev profile, since unoptimized hashing slows every login in the
    test suites. No Go hash is ever verified, so there is no bcrypt.
  - `notify` — file watching for `plugins/`, `modules.json` and the
    library roots (recursive, D-185), default features (macOS needs
    `macos_fsevent` to build).
  - `tracing` — log events.
  - `tracing-subscriber` (D-173) — JSON log output to stderr or a file
    (`fmt` + `json` only, no env-filter/regex).
  - `sha2` + `hmac` (D-173, RustCrypto, pure Rust) — the HS256 token
    signature and its constant-time check; `sha2` also derives catalog
    item ids (D-186).
  - `sha1` (D-205, RustCrypto, pure Rust, the `sha2` family) — BitTorrent
    info hashes and piece checks; BEP 3 fixes the hash.
  - `getrandom` (D-173) — OS randomness for the token secret and the
    token `jti`.
  - `libc` (D-179, already in the tree on Unix) — `statvfs` for the
    client offline store's free-space floor, (D-190) `kill(2)` with
    SIGSTOP/SIGCONT for transcode throttling in `lain-watch`, and
    (D-237) termios for `lain login`'s password prompt; Unix only
    (elsewhere ffmpeg never pauses, and the Windows prompt calls two
    kernel32 functions directly).
  - `sha2` and `getrandom` are also used by `lain-watch` (session and
    cache keys) and `lain-server` (multipart range boundaries); no
    crate was added for the watch module (D-190).
  - `miniz_oxide` (D-195, pure Rust, brings only `adler2`) — raw
    DEFLATE for CBZ pages; `default-features = false`, `with-alloc`.
    The ZIP container, CRC-32, image headers, `ComicInfo.xml` and the
    backup tar are in-tree.
  - `rustls` + `tokio-rustls` + `webpki-roots` (D-200, Q-045 C) —
    outbound HTTPS in `lain-http`: rustls with the `ring` provider
    (`ring`, `std`, `tls12`; no aws-lc-rs), the Mozilla root set compiled
    in (no OS certificate store or TLS library). They bring
    `rustls-webpki`, `rustls-pki-types`, `untrusted`, `subtle`,
    `zeroize` and `ring`, whose build compiles its C and assembly
    (with `getrandom` 0.2, a second version next to 0.4); `ring` is the
    one C-backed crate, accepted for TLS by the coordinator's D-200.
  - In-tree instead of a crate: the web build embedding
    (`crates/lain/build.rs`, `include_bytes!`; no `rust-embed` or
    `include_dir`), URL-safe base64 for the JWT segments
    (`lain-auth/src/crypto.rs`; no approved crate exposes one), the
    runner's HTTP client and upstream fixture server, the comic
    identifier's patterns (no regex crate), the `.nfo` reader (no XML
    crate), and `lain-http`'s URL parsing, escaping and keep-alive pool
    (no `url`, `reqwest` or `hyper-rustls`).
  - No test-only crates.
- Rules: provider boundaries, media bytes never in a provider call,
  catalog/userstate separation, typed errors with stable codes and no
  panic on bad input, English prose.
- Component transport: a Unix socket, or a named pipe on Windows
  (D-174); the NDJSON frames are identical.
- HTTP: axum with default features off; handlers are thin and the
  domain logic lives in the module crates.
- Storage: one redb file (`lain.redb`) of JSON records via
  `lain-store`; JSON documents in the data dir for operator config
  (`composition.json`, `modules.json`, `roots.json`).
- Tokens: HS256 in `lain-auth` with live role, liveness,
  password-version and session checks.

## Desktop app (`apps/desktop`, GPUI, D-180)

Its own cargo workspace and `Cargo.lock` (the server workspace stays
untouched), path dependencies on `crates/lain-client` and `crates/lain-api`.
It is a client, so it may link libmpv (loaded at run time, D-182). Build
natively on Windows (`cargo build` in `apps/desktop`; output on C: per
`.cargo/config.toml`) or on Linux. GUI test runs follow
`apps/desktop/README.md` ("Test runs"). Direct dependencies (D-184):

- `gpui` (=0.2.2) — the UI framework.
- `lain-client` (path) — every server call; the app never speaks HTTP itself.
- `tokio` — `lain-client` is async on it; a two-thread runtime.
- `serde` / `serde_json` — theme and config files.
- `image` (`jpeg`, `png`, `webp`) — art decode, resize and blur off the UI
  thread, and PNG window captures; GPUI already depends on it.
- `flate2` (`rust_backend`) — deflate for theme zips (in-tree zip reader);
  already in the tree under GPUI.
- `raw-window-handle` — the native window handle mpv renders into.
- `windows` 0.61 (Windows only) — the video host window, window placement,
  PrintWindow captures; the version GPUI compiles.
- `lain-http` (path, P6, D-263) — `net:<host>` GETs of extensions; its
  rustls/ring are already in the desktop tree under GPUI.
- `wasmtime` (=50.0.1; `cranelift`, `runtime`, `component-model`, `std`,
  no defaults) and `wasmtime-wasi` (=50.0.1, `p2` only) — tier 3
  extensions (P6, D-256, D-263): the component-model runtime with fuel,
  epoch interruption and store limits, and the WASI p2 interfaces every
  `wasm32-wasip2` guest imports, linked with an empty context. The owner
  approved them in the P6 brief. They add about 25 MB to the release exe
  (15 → 40 MB) and about a minute to a clean release build.
- `patches/xattr-0.2.3` — a one-constant patch so GPUI's transitive `xattr`
  builds on Linux against the `libc` pin (see `patches/README.md`).

Guest side (`crates/lain-ext-api`, its own workspace, never compiled by
the server or the app): `wit-bindgen` (=0.63.0, `macros`, `realloc`,
`std`) for the `lain:extension` world. Extensions and the probe guest
build for `wasm32-wasip2`; spec in `docs/MODDING.md`.

## Android app (`apps/android`, P5, D-245..D-254, D-277, D-278)

Kotlin + Jetpack Compose + Media3 on `lain-client`. Two builds:

- `apps/android/lain-ffi` — the bindings crate, its own cargo workspace
  and `Cargo.lock` (like the desktop, so the server workspace never
  compiles UniFFI); path dependencies on `crates/lain-client`,
  `crates/lain-api`, `crates/lain-read` (offline CBZ pages) and
  `crates/lain-core` (its error type). Direct dependencies:
  - `uniffi` (=0.32.2, no default features; `cli` + `cargo-metadata`
    only behind the `bindgen` feature of the `uniffi-bindgen` binary) —
    the Kotlin bindings V-12 named for Android; D-245.
  - `tokio`, `serde`, `serde_json` — the workspace pins.
  - dev: `lain-server`, `lain-conformance`, `lain-store` for the
    end-to-end test against the server in-process.
- `apps/android` — Gradle (Kotlin DSL, `gradle/libs.versions.toml`, the
  wrapper is committed). Dependencies (D-251): AndroidX `core-ktx`,
  `activity-compose`, `lifecycle-viewmodel-compose` and
  `lifecycle-runtime-compose` (the activity, back handling, the app
  view model); Compose `ui`, `foundation`, `material3` through the BOM
  (the UI); Media3 `exoplayer`, `exoplayer-hls`, `ui` (the player: direct
  play, HLS transcodes, `PlayerView`); `kotlinx-coroutines-android`
  (UniFFI's suspend functions); `jna` (the bindings' native calls; the
  `aar` in the app, the jar in JVM tests); tests: `junit`,
  `kotlinx-coroutines-test`. No navigation, image-loading or DI library:
  a screen stack, `BitmapFactory` and one `AppGraph` do those jobs.

`scripts/build-rust.sh` builds the host library, generates the Kotlin
bindings into `app/generated/uniffi` (git-ignored) and builds the ABIs
(`arm64-v8a`, `x86_64`) with cargo-ndk into `app/build/rustJniLibs`;
Gradle adds both. Nothing generated is committed. **Do not install the
Android SDK/NDK on the owner's machine** (the disk is tight): Rust parts
build and test on the host (`cargo test --manifest-path
apps/android/lain-ffi/Cargo.toml`), Kotlin compiles only in CI (the
`android` job, artifact `lain-android-debug`). See `apps/android/README.md`.

## Workflow

1. Read the relevant source + `docs/` before changing conventions.
2. Test first for behavior changes (`cargo test --workspace` must stay
   green).
3. Typed errors with stable codes (`lain_core::Error`); never panic on
   bad input; never silent failures — log or propagate.
4. Small slices. Do not implement transcode/metadata/matrix-daemon mode
   "while here"; contracts for those are reserved, not started.

## Boundaries (load-bearing)

- Providers implement `lain_core::Provider` and never touch the HTTP
  layer, process-global state, or another provider's files.
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
just web                           # web build -> crates/lain/webui (embedded)
cargo build -p lain                # the binary (keep builds incremental)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace             # every crate, the conformance suites
                                   # in-process, the CLI end to end
conformance/run.sh                 # the suite against a throwaway `lain serve`
cargo run -p lain -- serve --data-dir /tmp/lain-dev
cargo run -p lain -- doctor        # environment report
cargo run -p lain -- login --server http://127.0.0.1:9360
cargo run -p lain -- watch --next  # resume in mpv
```

Use `CARGO_BUILD_JOBS=4` on shared machines.

## Testing layers (what to use, where)

- **Unit tests** next to the code (`#[cfg(test)]`), integration tests
  in `crates/*/tests/`, the CLI end to end in `crates/lain/tests/cli.rs`
  (the real binary against a throwaway server; no player, no network).
- **Conformance** — `conformance/cases` is the HTTP API spec (D-169):
  every `/api/*` route clients use, run in-process by
  `crates/lain-server/tests` and against the binary by
  `conformance/run.sh`. A behavior change to a route changes its case.
- **Real-ffmpeg tests** — `lain-watch` unit tests use injected
  probe/capability/run hooks and need no ffmpeg; the server's
  `tests/watch.rs` and the conformance watch suite run the real
  ffmpeg/ffprobe on clips made with lavfi (`conformance/make-clips.sh`;
  nothing is committed). They skip without ffmpeg unless
  `LAIN_REQUIRE_FFMPEG=1`, which CI sets on all three OSes.
- **Fault injection** — assert degradation, not crashes: closed or
  locked stores, unwritable or vanished caches, missing source files,
  broken walks. A failed walk or an identify failure must never read
  as deletions (D-068).
- **Shell stubs are POSIX sh** — tests that stub `ffmpeg`/`ffprobe`
  write a `#!/bin/sh` script; CI's `/bin/sh` is dash (Ubuntu), not
  bash, so a bashism passes locally and fails only in CI. Keep stub
  bodies POSIX: `for last; do :; done` for the final argument, never
  `${@: -1}`, `[[ ]]`, arrays or `$'...'`. Tests must also not assume
  a host binary exists: control `PATH`, and skip only on genuinely
  absent platforms.

## Commit discipline

Short imperative subjects. This repo (`projects/lain`) is the server and,
since P3, the GPUI desktop app (`apps/desktop`); the Qt desktop client
lives in `projects/lain-desktop` until v1 replaces it. Do not mix them.

## Repository language

- English is the canonical language for source code, comments, tests, plans,
  project documentation, contributor instructions, and project wikis.
- Other languages are allowed only in explicitly identified translations,
  localization resources, or language-specific documentation variants.
- User-facing text must be translatable; do not hard-code a second language in
  source files as a substitute for localization.
- Web: every user-visible string goes through `t()` from `$lib/i18n`
  with its key in `web/src/lib/i18n/messages/en.ts`; dates, numbers and
  plurals go through the i18n formatters. Read `docs/I18N.md` before
  writing UI text. `npm run i18n:report` lists what still bypasses it.

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
