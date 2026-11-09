# lain desktop prototype: Slint (P2b)

The two screens of the comparative prototype (DESIGN-v2 §8.2, V-13) built with
**Slint 1.18.1**: Home (hero, Continue, rails including one with all 500
episodes) and the Player (libmpv inside the window with Slint controls over
it). Same `lain-client` underneath as the GPUI and iced builds.

Slint is used under the **Slint Royalty-free License 2.0** for this evaluation.
The licence asks for attribution; the app shows Slint's `AboutSlint` widget in
its About panel (top right, "About"). The licence decision stays with the owner
(DESIGN-v2 §12 Q1).

## Build and run

Windows, MSVC toolchain, `libmpv-2.dll` + `mpv.lib` in `D:\lain-proto\libmpv`
(override with `LAIN_MPV_DIR`). This is its own cargo workspace; the build output
goes to `C:\lain-proto-target\slint` (`.cargo/config.toml`).

```
cargo build --release          # or tools/dist.sh (builds and fills dist/)
dist\lain-slint.exe            # double-click: default theme and server
```

`dist/` holds `lain-slint.exe`, `libmpv-2.dll`, `fonts/` and `skins/`. The
defaults expect the shared inputs in `D:\lain-proto` and lain-rs on
`http://127.0.0.1:9360`. The first start logs in with `shared\conta-slint.txt`
(or `LAIN_USER`/`LAIN_PASSWORD`); the session is kept in
`%LOCALAPPDATA%\lain-slint\sessions.json`, so later starts need no password.

Flags:

| flag | what it does |
|---|---|
| `--theme <file>` | theme file (default `<shared>\theme.json`); live reload |
| `--skin <file.slint>` | card skin loaded at run time (see "Skins") |
| `--shared <dir>`, `--media <dir>`, `--server <url>` | inputs |
| `--bench <out.json>` | unattended benchmark, then exits |
| `--mute` | mpv `ao=null` + `mute=yes`, set before `mpv_initialize` |
| `--window-pos X,Y`, `--window-size WxH` | physical rect; the window opens there, not activated |
| `--play` | opens the hero's episode once Home has loaded |
| `--log <file>` | stderr to a file (the exe is a GUI-subsystem binary, no console) |
| `--shots <dir>` | Slint's own snapshot of Home (and Player in bench) |
| `--quit-after <s>`, `--mpv-log <file>` | test helpers |

Test runs on the shared machine go through `D:\lain-proto\tools\rodar-teste.ps1`
(`tools/try.sh` wraps it). `tools/shot-window.ps1` captures the app window with
PrintWindow; `tools/shot-display1.ps1` captures the test panel.

Keys: Home: arrows move between the hero and the rails and along a rail, Enter
or Space opens, Home/End jump in a rail; the mouse wheel scrolls the page, Shift+wheel,
a trackpad or a drag scrolls a rail (kinetic). Player: Space/k pause, ←/→ 5 s,
↑/↓ volume, m mute, f fullscreen, Esc closes a menu, leaves fullscreen, or
goes back.

## How it is built

- **The whole UI is compiled at run time** by `slint-interpreter`
  (`src/ui.rs`): `ui/app.slint`, `ui/theme.slint` and the card are embedded in
  the exe and handed to the interpreter through a file loader, which serves
  `card.slint` from `--skin` when one is given. Rust talks to the UI through
  three globals (`Theme`, `Home`, `Player`) with `set_global_property` /
  `set_global_callback`. This is the price of skins (see below): no generated
  Rust types, property names are strings checked at run time.
- **Home**: rails are absolutely positioned cards in horizontal `Flickable`s
  inside a vertical one. Slint 1.18 passes a vertical wheel through a
  horizontal-only Flickable to the page, and animates wheel scrolling itself.
  Rails are **virtualized in the .slint file**: a repeater over
  `count` (cards in view + 3) whose instances are rebound to
  `first + k` as the rail scrolls, so the 500-card rail has about 12 live cards.
  Art: three worker threads decode the local JPEGs with `image`, crop-resize them
  to the card size × DPI × hover scale (and blur the hero when the theme asks),
  and hand `SharedPixelBuffer`s to the UI thread; one image per series, kept
  in a model (`posters`/`thumbs`), which is the cache. Continue comes from
  `/api/continue`, and the event stream refetches it on `progress-changed`
  and after a reconnect. The hero follows D-022: resume first, else
  recently added.
- **Player**: see the next section. Progress is read before playback (resume)
  and written every 5 s and on close (`put_progress`).

## mpv inside the window

**Approach: mpv render API (OpenGL) as an underlay in Slint's own GL
context.** Slint runs the winit backend with the FemtoVG renderer on
OpenGL. A rendering notifier (`Window::set_rendering_notifier`, stable API):

- `RenderingSetup` creates the `mpv_render_context` (OpenGL) in Slint's
  context. Function pointers come from `wglGetProcAddress`, falling back to
  `opengl32.dll`.
- `BeforeRendering` runs after FemtoVG has cleared the back buffer. It calls
  `mpv_render_context_render` into FBO 0 at the window size, with
  `BLOCK_FOR_TARGET_TIME=0`, so the UI thread never waits for a frame's display
  time.
- FemtoVG then draws the Slint UI on top, with real alpha over the video: the
  gradients, the seek bar and the menus all composite over the picture.
- `AfterRendering` calls `mpv_render_context_report_swap`.
- mpv's update callback asks Slint for a redraw through
  `upgrade_in_event_loop`.

Details that mattered:

- Slint's GL default is **GLES 2.0**, too little for mpv's 10-bit paths. The
  app asks for desktop GL 3.3 with `BackendSelector::require_opengl_with_version(3, 3)`.
- FemtoVG can leave `GL_SCISSOR_TEST` on. mpv assumes the default (off), so
  the app disables it before each mpv render.
- One mpv core lives for the app's lifetime. Leaving the player sends
  `stop`; the render context stays.
- **Hardware decoding**: with WGL, mpv has no zero-copy D3D11→GL interop, so
  `hwdec=auto-safe` resolves to **`d3d11va-copy`**: decode on the GPU, copy back,
  upload as a texture. 4K60 HEVC dropped 1 and 9 frames in 20 s in two
  release bench runs on the Iris Xe (Slint's GL context is on the Intel GPU; nothing
  asks for the RTX). Zero-copy would need ANGLE/EGL (`d3d11egl`) or mpv's D3D11
  render API. This libmpv build has the D3D11 render API, but its header is not
  shipped and Slint exposes no D3D device to render into.
- Subtitles are rendered by mpv (libass), typesetting (`\pos`, `\blur`) and
  karaoke included (`shots/player-10bit-subs.png`). mpv only selects
  tracks flagged default/forced, so the app selects the first subtitle track
  when none is.
- Tearing: none seen. Slint presents with vsync (swap interval 1) and mpv
  draws in the same swap, so video and UI cannot drift apart. Latency is one
  UI frame.
- What I did not try: the `unstable-wgpu-*` route (import a wgpu texture into
  a Slint `Image`). mpv has no wgpu/Vulkan render API to fill it, so it would
  mean mpv→GL→shared texture→wgpu. A child HWND (`wid`) would put the video
  above Slint's surface, so no Slint overlay could be drawn on top.
- Stable vs unstable APIs:
  - the rendering notifier is stable;
  - `BackendSelector::with_winit_window_attributes_hook` and `WinitWindowAccessor`
    (placement, fullscreen on an explicit monitor) need the
    **`unstable-winit-030`** feature;
  - the mpv side is plain libmpv C API, declared by hand.
- Size: `src/mpv.rs` (FFI and render context) is 382 lines; the notifier,
  event thread and track/info code in `src/main.rs` add about 200. The player
  UI is about 350 lines of `.slint`.
- Fullscreen uses `Fullscreen::Borderless(Some(<monitor holding the window>))`.

## Skins (Slint's modding strength)

The card is a `.slint` component loaded at run time:
`--skin skins/card-banner.slint`. A skin exports a `CardMetrics` global (the
space a rail keeps under the art) and a `Card` component with a fixed
interface (`art`, `art-w/h`, `title`, `label`, `progress`, `focused`,
`clicked`). The contract is written at the top of `ui/card.slint`. Skins can import
the `Theme` global, so every theme still applies.

- `skins/card-poster.slint`: the default. The art scales on hover/focus with
  a focus ring; title and label sit under the art.
- `skins/card-banner.slint`: a different layout. Title (display font) and
  progress sit on the art over a scrim, the label is a mono chip, and focus
  lifts the card and draws an accent bar instead of a ring. It has no footer,
  so rails get shorter (`shots/home-alt-theme-banner-skin.png`).

How far it goes: a skin can change the card's whole layout, its animations
and states, and use any Slint element. It cannot run code: logic is limited to
bindings. It cannot add data the Rust side does not provide. Nothing stops
a skin from being slow (a heavy card × 500 cards), and a bad skin
fails at startup with the compiler's diagnostics. Because the interpreter
compiles the whole UI, nothing stops us from opening `app.slint` to skins
too. Hot-reloading a skin would mean recompiling and swapping the window
instance; not done.

Cost:

- **Startup**: compiling the full UI (about 900 lines of `.slint` + std widgets) takes
  **about 55 ms** in release (`ui_compile_ms` in the bench).
- **Binary**: the release exe is **24.8 MB**. It carries the Slint compiler,
  the interpreter and the Fluent style (for `AboutSlint`). I did not build a
  compiled-only variant to measure the difference. The interpreter path also
  gives up compile-time type checks between Rust and the UI.
- **Interpreter quirk**: properties of type `easing` cannot cross the public
  API yet ("not supported yet for public API"), so the theme's easing is chosen
  by index (`Theme.curve`). The loader adds a selector over every known curve
  to `theme.slint` (`src/ui.rs`).

## Theme coverage (`shared/theme.json`)

Applied at startup and **live-reloaded** (the file is polled every 400 ms; a
change applies in about 0.5 ms, verified by editing colors, card size, radius and
display font while the app ran: `theme reloaded in 0.5 ms`). Unknown keys
are logged.

| key | status | notes |
|---|---|---|
| `name` | honored | shown in the page footer |
| `colors.bg/surface/text/muted/accent/danger/scrim` | honored | `danger` is the offline dot |
| `fonts.ui/display/mono` | honored | Inter, Fraunces and JetBrains Mono (OFL) are bundled in `fonts/` and registered at startup; any other family is looked up in the system fonts, else Slint's default (Segoe UI) |
| `fonts.size_base` | honored | base of every text size |
| `radius.card/button` | honored | |
| `spacing[0..6]` | honored | page margins, gaps, paddings; more than 7 steps are ignored (logged) |
| `rail.card_w/card_h/gap` | honored | a landscape `card_w > card_h` switches cards to backdrop art |
| `rail.hover_scale` | honored | art is decoded at this scale so hover stays sharp |
| `hero.height` | honored | fraction of the window height |
| `hero.gradient` | partial | first and last stops (Slint gradients are fixed at compile time; the 2 stops are bound) |
| `hero.blur` | partial | Slint has no blur filter: the hero art is blurred on the CPU when decoded (static, re-done on reload) |
| `motion.fast_ms/normal_ms` | honored | every animation duration |
| `motion.easing` | honored | Slint names pass through; `ease-*-cubic` map to cubic-bezier; `spring` → Slint's `spring(0.3)`; a `cubic-bezier(...)` works at startup only |
| `player.controls_bg/seek_h/autohide_ms` | honored | |

## Bench (`--bench`)

Logs in, opens Home, scrolls the 500-card rail end to end at 4000 px/s
(106,200 px), then plays the 4K file for 20 s and the 10-bit file for 20 s
with the controls pinned, writes the JSON and exits. Frame times come from
Slint's rendering notifier: the interval between `AfterRendering` callbacks,
and the CPU time from `BeforeRendering` to `AfterRendering` (`*_cpu_ms`).
Bench runs never write progress. Numbers on this machine (busy, not final):
`shots/bench.json`.

## Lines, size, build

- Rust: `src/` 2,617 lines (main 1,185, mpv 382, metrics 291, bench 232,
  theme 179, ui 126, art 120, data 82) + `build.rs` 26.
- Slint: `ui/app.slint` 899, `ui/card.slint` 106, `ui/theme.slint` 53;
  `skins/card-banner.slint` 111.
- Release exe: 24.8 MB (plus `libmpv-2.dll`, 124 MB).
- Clean release build: **173 s** (`-j 6`, crates already downloaded, other
  builds running).

## Dependencies

| crate | why |
|---|---|
| `slint` 1.18.1 (`backend-winit`, `renderer-femtovg`, `unstable-winit-030`) | the UI; winit hook for window placement and monitor fullscreen |
| `slint-interpreter` 1.18.1 | run-time UI compilation (skins) |
| `lain-client` (path) | everything the server is asked |
| `tokio` | runtime for `lain-client` |
| `image` (jpeg, png) | art decoding, crop-resize and blur; PNG for snapshots |
| `serde`, `serde_json` | theme.json, series.json, bench JSON |

libmpv, the process time/RSS calls (`kernel32`) and the window placement
calls (`user32`) use hand-written FFI. There is no `windows-sys` or mpv crate.

## What was hard / framework bugs hit

- **Window placement on the shared test machine.** The fixes:
  - winit's message window ("Winit Thread Event Target") is visible at 0,0 on
    the primary monitor, and placement checks catch it. It is now hidden
    right after the backend starts.
  - The requested position is also written as the window's normal placement
    before the first show.
  - A tiling window manager on the machine re-tiled the window onto the
    primary monitor until the owner added an ignore rule.
- **Screenshots of a GL window.** PrintWindow/CopyFromScreen of an occluded or
  partly off-screen GL window can return stale pixels. Slint's
  `take_snapshot` re-renders the scene but without the mpv underlay (the
  video is black there). The window captures in `shots/` are PrintWindow. Their
  bottom strip can show stale pixels, because on the test panel the window
  extends past the bottom of the screen. `player-10bit-display1.png` is the
  real screen.
- Built-in names: `row`, `col` and `Row` are GridLayout names in Slint, so a
  component property called `row` fails ("Cannot override property").
- A Flickable whose content height comes from a layout that depends on the
  window height makes a binding loop through the window's layout constraints.
  The fix was `preferred-height: 0` on the page Flickable.
- `easing` properties are not exposed to the interpreter's public API (above).

## Not done

- Skin hot reload; a skin for anything but the card.
- Zero-copy hardware decoding (see mpv section).
- The bench JSON from this machine is not final (other builds were running).
