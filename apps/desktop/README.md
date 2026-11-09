# lain desktop (GPUI)

The desktop app v1 (DESIGN-v2 §10 P3, D-180): the parity checklist of §8.4
on [GPUI](https://crates.io/crates/gpui), with themes tier 1 (§8.3,
[`docs/THEMES.md`](../../docs/THEMES.md)). It grew from the P2b prototype
(`proto/gpui`). Every server call goes through `lain-client`.

## Build

Its own cargo workspace (the server workspace and its `Cargo.lock` are not
touched), with path dependencies on `crates/lain-client` and `crates/lain-api`.

**Windows** (MSVC 14.44, Windows SDK 26100):

```sh
cargo build --release      # output in C:\lain-desktop-target (.cargo/config.toml)
```

GPUI's build script finds `fxc.exe` in the Windows SDK by itself
(`GPUI_FXC_PATH` overrides it). libmpv is **loaded at run time**
(`libmpv-2.dll` next to the exe, D-182), so the build needs no mpv. For
`cargo run`, `build.rs` copies the DLL next to the exe from `LAIN_MPV_DIR`
or `vendor/libmpv/` when it is there (shinchiro's `mpv-dev` x86_64-v3 build
was used; the folder is git-ignored, the DLL is 124 MB).

**Linux** (X11 or Wayland; tested to build on Arch in WSL):

```sh
CARGO_TARGET_DIR=target cargo build --release
```

Needs the GPUI system libraries (Debian/Ubuntu names: `libxkbcommon-dev
libxkbcommon-x11-dev libwayland-dev libx11-xcb-dev libxcb1-dev
libfontconfig1-dev libfreetype-dev libvulkan-dev`) and `mpv` on `PATH` (or
`LAIN_MPV=/path/to/mpv`) to play. `patches/xattr-0.2.3` fixes a transitive
crate that does not build on Linux otherwise (`patches/README.md`).

CI (`desktop` job) runs fmt, clippy `-D warnings`, the build and the unit
tests on Ubuntu and Windows. No GUI runs in CI.

## Run

`lain-desktop-gpui.exe` (the name stays for now: the owner's window manager
ignores it). First run asks for the server URL, then sets up the first admin
on a fresh server or logs in. The session token is kept by `FileTokenStore`
in the client dir, so the next start opens Home directly.

| Flag | |
|---|---|
| `--server URL` | use this server (saved in `desktop.json`) |
| `--client-dir DIR` | client state: `sessions.json`, `client.redb` (progress queue, offline index), `offline/`, `themes/`, `desktop.json`. Default `%LOCALAPPDATA%\lain-client` / `~/.local/share/lain-client` |
| `--theme DIR` | use a theme folder directly (theme work) |
| `--mute` | mpv `ao=null` + `mute=yes` before `mpv_initialize`; the log then shows `current-ao=null` |
| `--window-pos X,Y` / `--window-size WxH` | exact window rect in physical virtual-desktop pixels; shown without activation |
| `--log FILE` | stderr to a file (the release exe has no console) |
| `--demo STEPS` / `--shot DIR` | in-app automation and window captures (below) |

### Test runs (the owner's rules)

GUI test runs go **only** through
`powershell.exe -NoProfile -ExecutionPolicy Bypass -File D:\lain-proto\tools\rodar-teste.ps1 -Exe <exe> -Args "..."`:
it appends `--mute` and the built-in panel's rect, and kills the app if a
window lands elsewhere. No simulated input: the app is driven by `--demo`, a
`;`-separated script run inside the app (see `src/demo.rs`):

```
--demo "login;wait:1500;shot:home.png;item:0;shot:item.png;play:0;controls;wait:4000;shot:player.png;quit"
```

`login` takes `LAIN_DEMO_USER`/`LAIN_DEMO_PASS` from the environment (and
runs first-admin setup on a fresh server). `shot:<name>` writes a PNG of
this window (PrintWindow, video included) into `--shot DIR`. Screenshots
are in `shots/`.

## Screens and keys

Keyboard-first (AGENTS.md): every screen works without a mouse, and keys are
shown next to actions.

- **Global:** `Ctrl+K` command palette (screens, libraries, settings, themes,
  scan, titles), `Ctrl+1…6` destinations, `/` search, `Ctrl+,` settings,
  `F5` refresh, `Tab`/`Shift+Tab` and arrows move focus, `Enter` activates,
  `Esc` leaves a field, then goes back.
- **Home:** hero + rails (Continue, Recently added, one per library). Arrows
  move, `Enter` plays/opens, `I` details, `PgUp/PgDn`. Mouse wheel and drag
  scroll rails with a fling.
- **Library:** one tab per library of the group (Library = watch kinds, Read =
  manga/comic), filter (`F`), sort by title/recent/year, pages sized to the
  window (`PgUp/PgDn`). A locked private library shows its unlock form.
- **Title page:** episodes or volumes with progress, `Enter` play/read, `W`
  watched, `D` download; the selected file's details and subtitle sidecars.
- **Search:** as you type, kind chips; an empty query browses every title.
- **Player:** `Space`/`K` pause, `←/→` 5 s (`Shift` 30 s), `J/L` 10 s, `0–9`
  jump, `↑/↓` volume, `M` mute, `A`/`S` cycle audio/subtitles, `N` next,
  `F` fullscreen, `Esc` back. Next episode after a 5 s countdown.
- **Reader:** one page fit to the window, arrows in the archive's direction
  (`rtl` for manga), `Space`/`PgDn` forward, `Home/End`.
- **Downloads:** offline copies with live progress; play, pause, resume, remove.
- **Settings:** YOU (account, profile and avatar, private libraries and PIN,
  appearance, playback & downloads) and, for admins, SERVER (users,
  libraries with scan, plugins, modules, transcode & integrations, backup).

## Playback (D-182)

Source order: an offline copy; the server's plan
(`/api/items/{id}/playback?client=desktop`: `direct` → the token-carrying
stream URL, `transcode` → the session's HLS playlist or the progressive
output); when the server cannot stream (501 not-ported, watch module off,
offline) the item's file through **Settings → Playback → Shared storage**
(`server path => local path`), if this device can reach it. Sidecars are
added with `sub-add`. Progress goes through `ProgressSync` (queued offline)
every 5 s, on close, at the end and on window close.

**Windows:** libmpv renders into a disabled child window under GPUI's
DirectComposition surface (`src/mpv.rs`): GPUI's controls blend over the
video with real alpha, no frame passes through the UI thread
(`vo=gpu-next`, `gpu-api=d3d11`, `hwdec=auto-safe`). Details of the
technique, its pitfalls and numbers are in the P2b notes (D-180).

**Linux:** an external `mpv` in its own window, driven over its JSON IPC
socket (`src/mpv_ipc.rs`). Why not embedded: GPUI 0.2.2 has no surface or
texture hook, libmpv's render API needs a GL context GPUI does not share,
and Wayland has no cross-process subsurface. What works: resume, progress
sync (`time-pos`/`duration`), tracks, sidecars, next episode, and the app's
control bar as a remote; mpv's own keys and OSC in its window. When mpv
exits the app returns to where it was.

## Themes (D-183)

`lain.ui.theme@1` folders in `<client dir>/themes/`; the spec is
[`docs/THEMES.md`](../../docs/THEMES.md). Two ship: `lain-dark` (the owner's
colors from the prototype) and `paper-light`. Settings → Appearance picks,
installs a zip and opens the folder; edits to the active theme apply live.

| Token | Desktop |
|---|---|
| colors (all 13), OKLCH and hex | honored |
| `type.*` families, `base`, `scale` | honored (bundled Inter, Fraunces, JetBrains Mono; system fallback) |
| `radius.*`, `spacing` | honored |
| `blur.hero` | honored, at decode time (GPUI has no blur filter) |
| `blur.panel` | no: GPUI has no backdrop filter; panels use `colors.raised` alpha |
| `motion.*` incl. `cubic-bezier()` and `spring` | honored |
| `rail.*` | honored; `hover_scale` by resizing the layout box (no transforms) |
| `hero.height` | honored |
| `hero.gradient` | partial: first and last stops (GPUI gradients take two) |
| `player.*` | honored |
| `wallpaper` | honored on the connect/login screens |
| `icons/<name>.svg` | honored, live |

## Dependencies (D-184)

| Crate | Why |
|---|---|
| `gpui` =0.2.2 | the UI framework |
| `lain-client` (path) | every server call |
| `tokio` | `lain-client` is async on it; a two-thread runtime GPUI awaits |
| `serde`, `serde_json` | theme and config files |
| `image` (jpeg, png, webp) | art decode, resize and blur off the UI thread; PNG captures (GPUI already depends on it) |
| `flate2` | deflate for theme zips (in-tree reader); already in the tree |
| `raw-window-handle` | the native window handle |
| `windows` 0.61 (Windows) | the video host window, placement, captures; the version GPUI compiles |

No gpui-component: plain `div`, `img`, `svg` and `canvas`. libmpv FFI is
hand-written and loaded at run time.

## Source map

| File | What |
|---|---|
| `src/main.rs` | args, assets (icons with theme overrides, fonts), the window |
| `src/app.rs` | state, actions (`Act`), key routing, focus, navigation from `/api/modules`, palette, toasts |
| `src/ui.rs` | widgets: button, text field, label, kbd, settings row, notices |
| `src/backend.rs` | `lain-client`, `ProgressSync`, `OfflineStore`, the event stream |
| `src/theme.rs`, `src/zip.rs` | `lain.ui.theme@1`, OKLCH, hot reload, zip install |
| `src/images.rs` | thumbnails, avatars, comic pages, wallpapers: off-thread decode and cache |
| `src/engine.rs`, `src/mpv.rs`, `src/mpv_ipc.rs` | player engines (embedded / external) |
| `src/screens/*` | one file per screen |
| `src/demo.rs`, `src/platform.rs` | test automation; placement, captures |
