# Lain — Design v2: the media superapp

Status: **proposal for review** (2026-11-08). Nothing here is decided until the
owner approves it. Load-bearing choices are written as proposals `V-1`…, to be
numbered `D-155`… on approval, following the slice convention
(`docs/slices/DECISIONS_TO_CONFIRM.md`). Rows marked **[owner]** fall under
`taste.md` → "Never decide without the user".

## 1. Goal

One local-first media app that covers everything the owner does with media:
watch, read, acquire, subtitle, identify, track and share. It runs as a
**background service** on Windows, Linux and macOS and is used through **native
clients**: a GPU-accelerated desktop app and an Android app. The web UI stays as
the secondary client.

The superapp is **not one big binary with everything switched on**. It is a
small core plus **modules** that are enabled per server. A disabled module
registers no providers, mounts no routes, starts no processes and shows no UI.
Install size is the only cost of a disabled module. Running cost and duplication
are zero by construction, because every module uses the core's library, catalog,
users, progress and player. There is no second media server inside a module.

## 2. Where lain is today (facts, `main` at D-154)

- **Fixed core.** `internal/core`, `store`, `auth`, `gateway`, `matrix` and
  `component` hold the composition, the provider registry (health-gated swap,
  generation fencing, last-good fallback), auth, atomic documents, the HTTP
  boundary and **component supervision**.
- **Component mode exists.** Any provider can run out of process: NDJSON over a
  unix socket, a `hello` handshake, sequenced invoke/result, health, and respawn
  with backoff. Manifests dropped into `<data-dir>/plugins/` install, reload and
  remove components at runtime. Third-party components can be written in any
  language. This is the module seam; v2 builds on it and does not redesign it.
- **Capabilities on `main`:**
  - catalog, ingest, identify, metadata (merge-many), enrich;
  - playback plan, transcode@1–3, probe, thumbnail, local playback;
  - userstate, search, settings and theme;
  - comic pages (reading, D-085…), downloads and offline (D-089…D-094);
  - lists and account links (D-078…);
  - social (graph, activity, reviews, collections);
  - acquisition: release parse, Torznab/Newznab indexer, the in-house
    BitTorrent engine (D-108…), and the OpenSubtitles provider.
- **Library kinds:** `anime`, `movie`, `video`, `manga`, `comic`.
- **Clients:**
  - web SPA (SvelteKit, embedded), with routes `library`, `read`, `player`,
    `acquire`, `list`, `social`, `search` and `settings`;
  - `lain-desktop` (C++/Qt 6 QML + libmpv, Apache-2.0), which uses ~50 endpoints
    listed in §8.4;
  - the CLI (`lain watch`, `lain download`).

Lain is already shaped like a superapp. What is missing is (a) a **module
layer** that the owner and the clients can see and switch, (b) the **service
mode**, (c) the **native clients**, and (d) a home for the owner's other media
repos.

## 3. Principles (unchanged)

D-001/D-002 stdlib-first, no cgo, seconds-long build. Plugins never touch HTTP;
media bytes never enter a plugin call. Catalog stays separate from userstate.
Repository prose is English. A new Go dependency, a storage or auth change, a
public API contract change or a break in desktop compatibility is **[owner]**.

## 4. Architecture v2

```text
 Clients          lain-desktop-next (Rust, GPU)  ·  Android (Compose)  ·  web SPA  ·  CLI
                       │                 │
 Client core      lain-client (Rust): API, auth, offline store, progress sync, discovery
                       │ (UniFFI → Kotlin)
 ───────────────────── HTTP API /api (+ events) ─────────────────────────────────────────
 Server core      auth · libraries/roots · catalog · userstate · player plans · registry
 Modules          watch · read · acquire · subtitles · identify · social · lists · …
 Components       out-of-process providers (any language): lain-parser, animesubs,
                  Anything, animedb, …  ← manifests in <data-dir>/plugins/
```

## 5. Modules

### 5.1 Definition — `V-1`

A **module** is a named, user-facing bundle of:
- capabilities (bindings in the composition) and the providers that serve them,
  in-process or as components;
- gateway routes it owns;
- library kinds it introduces;
- UI surfaces in each client: a top-level destination, settings sections and
  Ctrl+K commands;
- optional component manifests it installs.

Modules are declared in a versioned document (`modules.json`,
`lain.modules@1`) that the core loads with the composition. Enabling a module
binds its capabilities and mounts its routes. Disabling it unbinds them through
the existing withdraw path (no restart) and unmounts the routes, which then
answer `module-disabled`. Data a module wrote is kept. The core module cannot be
disabled.

`GET /api/modules` lists modules with state, health and UI surfaces. Every client
builds its navigation from it, so a client never shows a destination the server
does not serve.

### 5.2 The module set — `V-2`

| Module | Capabilities (today → v2) | UI destination | Sources |
|---|---|---|---|
| **core** (always) | auth, libraries/roots, catalog, userstate, search, settings, theme, backup | Home, Search, Settings | `main` |
| **watch** | playback plan, transcode, probe, thumbnail, local playback, external player handoff | Library (video kinds), Player | `main` |
| **read** | comic pages, reader, offline pages | Read (manga, comic) | `main`; books later (§5.4) |
| **acquire** | release parse, indexer, torrent engine, downloads, source watch | Acquire | `main`; adds **web extraction** via Anything (§6) |
| **subtitles** | `lain.subtitle@1` (search/download) → **adds `lain.subtitle.translate@1`** | in Item and Player, plus Settings | `main` + animesubs component (§6) |
| **identify** | `lain.release.parse@1`, `lain.media.identify@1` | none (works behind the scenes) | in-tree tokenizer + lain-parser component |
| **metadata** | metadata (merge-many), enrich | in Item, plus Settings | `main`; optional animedb component (§6) |
| **lists** | list read/write/account, listlink | Lists | `main` |
| **social** | graph, activity, reviews, collections | Social, profiles | `main` |
| **lab** | experimental components (e.g. audio upscaling) | Settings › Lab | opt-in only |

### 5.3 Private libraries (core feature, not a module) — `V-3`

A library can be **private**. Its items never show in Home rails, Continue,
Search, Social activity, thumbnails or recommendations for anyone who has not
unlocked it in this session, and unlocking needs a per-user PIN or the password.
Privacy is enforced in the gateway, never only in clients, so every client gets
it for free. This carries over the one idea worth keeping from `media-manager`.
The owner keeps an adult collection; on a shared TV or a parents' house LAN this
is a requirement, not a nicety.

### 5.4 Portable library roots — `V-4`

Today a library stores absolute paths. v2 stores **named roots**: a library
declares a root (`biblioteca`, `anime-arquivo`…) and items store
`root:relative/path`. Each machine maps root names to directories in its own
config. Moving from Windows/WSL (`V:\media`, `/mnt/v/...`) to Linux (Omarchy)
then becomes a mapping change, not a catalog rewrite. A one-shot
migration converts existing absolute paths. This implements the "portable paths"
rule from hq-app DESIGN "Dados dos apps". **[owner]**: it changes the catalog
export document (`lain.catalog-export@2`, with import of @1).

## 6. The owner's other repos

Facts come from a survey of each repo (2026-11-08). The rule: a repo that has its
own users or its own life **stays a standalone app** and plugs in as a
**component**. A repo that only duplicates the core is **retired** after its good
ideas are taken.

| Repo | What it is | v2 role | Interface work needed |
|---|---|---|---|
| **filename-parser** (`lain-parser`) | Python/torch model daemon on a unix socket; `/parse` | **Already** the `lain.release.parse@1` model provider. Formalize it as a component manifest | none; keep the HF release channel |
| **zantetsu** | Rust filename parser plus Kitsu title matcher; library only (napi/PyO3; the advertised C ABI does not exist) | Overlaps the parser. **Measure first** (`V-5`): run both on the same real-name set. The README's 92.4% and lain-parser's 69.9% exact-record are different metrics | if kept: a small socket/HTTP wrapper speaking the component protocol |
| **animesubs** | Rust: engine + local server (`/v1`, bearer token, discovery file) + Tauri app; real users | **Stays standalone.** Provides `lain.subtitle.translate@1` as a component that drives its `/v1/jobs` (`V-6`) | animesubs: job cancel, SSE events, persisted jobs (already on its roadmap) |
| **Anything** | Python web media extractor/downloader; daemon :9642 (no auth, CORS `*`) | **Stays standalone.** Provides a new `lain.acquire.extract@1` (URL → candidates → job) as a component (`V-7`) | Anything: token auth and Origin checks before lain talks to it; layout/library writing stays Anything's own when it is used standalone |
| **animedb** | Rust metadata DB (SQLite, FTS, AniList/Jikan/Kitsu/TVmaze/TMDB/IMDb) with a GraphQL server | Optional **offline metadata provider** in the merge-many set (`V-8`). It does not replace lain's catalog | a thin component adapter (GraphQL → `lain.metadata.*`) |
| **just-stream** | Go TUI: torrent → RAM → mpv | **Retire** once acquire supports *stream-while-downloading* (`V-9`). Its only unique feature is RAM-only streaming | none |
| **hfest** | Flutter **nhentai client** (not a local reader), with encrypted downloads | **Not a module.** It is an online-source client, so it stays standalone or is retired by the owner. Its downloads can land in a private `manga` library (§5.3) | none |
| **boogle** | Python + Rust BM25 public-domain book search | Future **read › books** source, once a `book` kind and an EPUB/PDF reader exist. Not in v2's first phases | an indexer-style component |
| **audio-enhancer** | Python AudioSR CLI, file in → file out | **lab** module component, opt-in (`V-10`) | JSON progress output |
| **psyche, custom-server, media-manager** | Earlier media servers | **Retire.** Ideas taken: private libraries (media-manager), UI notes (custom-server) | — |
| **lain-desktop** | Qt/QML client | Maintained until the new desktop app reaches parity (§8.4), then retired | none |

## 7. Service mode — `V-11`

`lain service install|uninstall|start|stop|status|logs`, stdlib only, wrapping the
platform's own manager:
- **Linux:** a systemd **user** unit (`~/.config/systemd/user/lain.service`),
  with `loginctl enable-linger` optional for headless boxes.
- **macOS:** a LaunchAgent plist in `~/Library/LaunchAgents`.
- **Windows:** a **per-user logon task** (Task Scheduler) by default, not a
  LocalSystem Windows Service. A Windows Service runs as LocalSystem in session 0
  and cannot see the user's mapped drives (the owner's media is on a mapped
  `V:`) or per-user paths. `--system` opts into a real service for headless
  machines and documents the drive caveat.

The data dir follows each OS (`%LOCALAPPDATA%\lain`, `~/.local/share/lain`,
`~/Library/Application Support/lain`), overridable by `LAIN_HOME`, the data
convention of hq-app. The hq-app declares lain in `servicos.toml`; `hq svc`
start/stop call `lain service`.

## 8. Clients

### 8.1 Shared client core: `lain-client` (Rust) — `V-12`

A Rust crate used by every native client:
- the typed API client and auth (token storage in the OS keychain);
- server discovery (mDNS on the LAN, a manual URL, a QR pairing code from the
  web UI);
- the offline store, mirroring D-094's CLI store with the same limits and LRU
  eviction;
- progress sync with conflict rules, and the event stream.

Android gets it through **UniFFI** (Kotlin bindings). Writing the logic once is
what keeps two native apps affordable. It lives in a new repo (`lain-client`)
because lain itself is Go and must keep its seconds-long build.

### 8.2 Desktop app: framework selection — `V-13` **[owner]**

The owner's requirements:
- native and GPU-accelerated (Zed-like);
- full control of the look, aesthetics first (the post-modern visual language,
  D-022);
- **user-moddable UI**;
- secure;
- a framework that will still be alive in five years;
- Windows, Linux and macOS, with Android a plus.

Data gathered 2026-11-08:

| Framework | Repo activity (stars · last push · release) | Backing | Platforms | Look control | Runtime-moddable UI | Video (libmpv) | Risk |
|---|---|---|---|---|---|---|---|
| **GPUI** (+ gpui-kit) | Zed 94k ★, daily; gpui crate 0.2.2 on crates.io; gpui-kit 20k ★, v0.9 (2026-11-02) | Zed Industries (VC-funded). GPUI evolves for Zed's needs first; community forks (gpui-ce) exist for that reason | Win/Linux/macOS; Android only via the early `gpui-mobile` | Excellent: Zed-level rendering, text, perf | Code only. Mods need our own layer (themes as data, WASM logic, like Zed's extensions) | No known integration; we write a texture/surface hook | Pre-1.0 API churn; priorities set by Zed |
| **iced** | 32k ★, last push 2026-11-01, 0.14 (2025-12) | Community (+ System76 COSMIC uses it) | Win/Linux/macOS; mobile is a non-goal | Good; custom widgets/shaders, Elm architecture | Code only (same as GPUI) | **Exists:** `mpv-engine` has an iced adapter and wgpu texture import (young, 0.1) | Slower releases; no mobile |
| **Slint** | 24k ★, daily, v1.18 (2026-09) | SixtyFPS GmbH (commercial, stable 1.x) | Win/Linux/macOS, **Android**, embedded | Good, declarative `.slint`; custom styles | **Strong:** `slint-interpreter` loads `.slint` at runtime, so users can ship UI files (skins) | Needs a custom texture/underlay hook (unverified) | License: GPLv3 / royalty-free with conditions / paid. Lain is Apache-2.0 → **[owner]** |
| **egui** | 32k ★, active, 0.37 | Community + Rerun | All, including Android | Weak for a cinematic UI (immediate-mode look) | Code only | wgpu texture is easy | Wrong fit for the visual goal |
| **Dioxus Native** (Blitz) | Dioxus 40k ★; Blitz 4.4k ★, pre-alpha | DioxusLabs (funded) | All, including mobile | HTML/CSS, so very high | **Strongest:** user CSS | Unknown | Pre-alpha, incomplete CSS — too early |
| **Freya / Floem / Xilem / Makepad** | 3–7k ★ each | Small teams | Desktop (Xilem targets mobile) | Good | Code only | Unknown | Too small a community for a 5-year bet |
| **Qt Quick via cxx-qt** | Qt (huge); cxx-qt 1.6k ★, v0.10 | The Qt Company + KDAB | All, including Android | Excellent; **lain-desktop already uses it** | **Strong:** QML loads at runtime (but QML runs JS, so mods need sandboxing) | **Proven:** lain-desktop's `MpvItem` | C++ toolchain and the Rust bridge's small community; not "pure Rust" |
| *(baseline)* Tauri / webview | 113k ★ | CrabNebula + community | All | Total | Total (CSS/JS) | `<video>`/mpv texture is hard | Not GPU-native; WebKitGTK on Linux |

**Reading the table:**
- No framework wins everything.
- **GPUI** has the best feel and the largest proven app behind it, but its API
  serves Zed and it has no runtime UI loading.
- **Slint** is the most mature as a *product* (stable 1.x, a company, Android,
  runtime markup for mods), but its license needs a decision.
- **iced** has the only ready libmpv adapter.
- **Qt Quick** is the safe, proven path (it is what lain-desktop is), but it is
  C++-centric.
- **Modding is solved above the framework either way** (§8.3). Runtime markup
  (Slint, QML) only adds "skins".

**Proposal `V-13`:** do not pick from the table. Run a **comparative prototype**
(`taste.md`: "prefer choosing from working comparative prototypes"):
- Build the same two screens in **GPUI, Slint and iced**: the Home rail with
  full-bleed art plus the Player with libmpv and on-screen controls.
- Use the same `lain-client` underneath, and give each one a theme loaded from a
  data file.
- Measure:
  - cold start and memory;
  - frame time while scrolling a 500-item rail;
  - video playback (4K HEVC and 10-bit anime) with overlays;
  - how hard the mpv integration was;
  - lines of code;
  - how far the theme file can change the look.
- The owner picks after using the three side by side.

Recommended order of preference **before** the prototype, for transparency:
1. GPUI, for the look and feel;
2. Slint, for maturity, Android and skins;
3. iced, for the video path.

### 8.3 Modding model (framework-independent) — `V-14`

Three tiers, from safest to most powerful:

1. **Themes (data):** a theme folder with tokens (colors in OKLCH, type scale,
   radii, blur, motion curves), wallpapers/art and icon overrides. It is
   hot-reloaded, shared as a zip and can be installed from the web UI. The same
   `lain.ui.theme@1` tokens drive the web, the desktop and Android, so one theme
   styles every client.
2. **Skins (layout):** only if the chosen framework loads markup at runtime
   (Slint `.slint`, QML). They are declarative, with logic limited to bindings.
   With GPUI or iced this tier becomes a set of layout slots chosen through
   config.
3. **Extensions (logic):** **WASM components** (wasmtime) with a
   capability-based host API, the model Zed uses for its extensions. An
   extension can:
   - add a panel or rail;
   - react to events;
   - call lain API scopes it was granted.

   An extension cannot touch the filesystem, the network or other extensions
   unless it declares it and the user grants it.

Server-side plugins stay what they are (components). A client extension can
call a server capability only through the API with the user's token and
declared scopes.

### 8.4 Desktop parity checklist (to retire lain-desktop)

The endpoints lain-desktop uses today are the parity contract:
- setup and auth;
- `/api/me*`, including links, avatar and preferences;
- catalog, search and browse;
- item playback, progress, stream, thumbnail and pages;
- libraries and scan;
- users;
- plugins (list and swap);
- admin settings, transcodes and backup.

The new app reaches parity when it covers all of these. Until then both apps are
supported (the "never break desktop compatibility" rule), and the API stays
backward compatible.

### 8.5 Android app — `V-15`

- **UI:** Kotlin + Jetpack Compose.
- **Video:** Media3/ExoPlayer, the best decoder and DRM-free path on Android,
  with hardware decoding. An mpv-android fallback covers codecs or ASS subtitles
  ExoPlayer can't handle; ASS through libass, or server burn-in via transcode@3.
- **Logic:** `lain-client` via UniFFI.
- **Offline downloads** within the device budget.
- **Later:** Chromecast/DLNA handoff and Android TV leanback.

If the desktop prototype picks Slint, sharing UI code with Android becomes
possible. The proposal still prefers Compose for Android, because it gives the
most native feel and the best video stack.

### 8.6 Web UI

Stays first-class for administration, remote access and friends without the
app. It builds its navigation from `/api/modules`, so it follows module state.

## 9. API and events

- **Freeze today's `/api/*` as v1** for clients (documented in `CLIENTS.md`); new
  routes are additive.
- **Add `GET /api/events` (SSE):** library changed, progress changed, download,
  module state. Native clients and the hq dashboard need it to stop polling.
- **`GET /api/modules`** (§5.1).

**[owner]**: all three are public API changes.

## 10. Phases

| Phase | Content | Exit criteria |
|---|---|---|
| **P0 — this doc** | Owner review; `V-*` become `D-155+` | Approved table |
| **P1 — server foundations** | Module layer (`V-1`, `V-2`), `/api/modules`, `/api/events`, service mode (`V-11`), private libraries (`V-3`), portable roots (`V-4`) | Web client follows modules; `lain service` works on all three OSes; a private library is invisible to another user end to end |
| **P2 — client core + desktop prototype** | `lain-client` crate; prototype in GPUI / Slint / iced (`V-13`) | Owner picks a framework from working builds |
| **P3 — desktop app v1** | Parity checklist (§8.4) with the chosen framework; themes (tier 1) | Owner uses it daily instead of lain-desktop |
| **P4 — components** | lain-parser manifest, parser benchmark (`V-5`), animesubs translate (`V-6`), Anything extract (`V-7`) | Each works as a component with health and declines |
| **P5 — Android v1** | Compose + Media3 + `lain-client`; offline | Watch and read with resume, offline copies |
| **P6 — modding tiers 2–3** | Skins (if supported), WASM extensions with scopes | One sample extension per tier |
| **Later** | animedb provider (`V-8`), stream-while-downloading → retire just-stream (`V-9`), books (boogle), lab (`V-10`) | — |

## 11. Proposals to confirm

| # | Proposal | Owner-gated |
|---|---|---|
| V-1 | Modules as a bundle layer over composition; `modules.json` (`lain.modules@1`); disable = withdraw + unmount, data kept | public API |
| V-2 | The module set in §5.2 | — |
| V-3 | Private libraries enforced in the gateway; per-session unlock with PIN/password | auth surface |
| V-4 | Named library roots; `lain.catalog-export@2` with @1 import | storage contract |
| V-5 | Benchmark lain-parser vs zantetsu on the same real names before keeping either as the default | — |
| V-6 | `lain.subtitle.translate@1` served by an animesubs component | public contract |
| V-7 | `lain.acquire.extract@1` served by an Anything component; Anything must add auth first | public contract |
| V-8 | animedb as an optional merge-many metadata component | — |
| V-9 | Stream-while-downloading in acquire, then retire just-stream | — |
| V-10 | `lab` module for opt-in experimental components (audio-enhancer) | — |
| V-11 | `lain service` with a per-user logon task on Windows by default | — |
| V-12 | `lain-client` Rust crate in its own repo; UniFFI for Android | new repo |
| V-13 | Choose the desktop framework by a 3-way prototype (GPUI, Slint, iced) | **framework choice** |
| V-14 | Three modding tiers; WASM extensions with capability scopes | security model |
| V-15 | Android: Compose + Media3 + `lain-client`; mpv-android fallback | — |
| V-16 | Freeze `/api/*` as v1; add `/api/events` and `/api/modules` | public API |
| V-17 | Retire psyche, custom-server and media-manager after taking the listed ideas (owner prefers deletion over archiving) | repo deletion |

## 12. Open questions for the owner

1. **Slint licensing**, if it wins the prototype. Options:
   - GPLv3, which turns the desktop app GPL;
   - the royalty-free license, with its attribution condition;
   - paid.
2. Should the Android app also target **Android TV** in v1?
3. Should **hfest** downloads feed a private manga library, or does hfest stay
   fully separate?
4. **Books:** is reading EPUB/PDF in lain wanted soon enough to plan the `book`
   kind in P1's roots migration?
5. **Remote access:** native clients outside the LAN mean Tailscale/WireGuard
   (owner's "no internet exposure" rule). Should `lain-client` support a direct
   Tailscale URL, or a relay?

## Sources (framework survey)

- GitHub API data (stars, last push, releases, contributors) collected 2026-11-08.
- `gpui` crate 0.2.2 (Apache-2.0) in `zed-industries/zed/crates/gpui`. gpui-box
  and gpui-pre docs on pre-1.0 churn. gpui-mobile (early Android) on docs.rs.
- `mpv-engine` crate: iced adapter, wgpu texture import (docs.rs).
- Slint interpreter and licensing: slint-interpreter docs; Slint README.
- Dioxus Native / Blitz status: Blitz README ("pre-alpha"), Dioxus native docs.
- Android notes for iced, egui and Slint: Hacker News thread 46350641 and
  community surveys (2025–2026).
