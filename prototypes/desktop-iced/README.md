# lain desktop prototype: iced (P2b)

One of the three V-13 prototypes (DESIGN-v2 §8.2): Home (hero + rails, one
with all 500 items) and a Player with libmpv rendered inside the window,
on iced 0.14 (wgpu 27, D3D12) and `lain-client`. Its own cargo workspace;
the main workspace and its `Cargo.lock` are untouched.

## Build and run (Windows)

```
cd prototypes\desktop-iced
cargo build --release -j 6
```

- `.cargo/config.toml` sets `MPV_LIBRARY_DIR=D:\lain-proto\libmpv` (where
  `mpv.lib` is) and puts `target/` on `C:\lain-proto-target\iced`.
  `build.rs` copies `libmpv-2.dll` next to the exe.
- `dist/` holds `lain-iced.exe` + `libmpv-2.dll` (not committed: 145 MB).
  Double-click runs with the defaults below.

```
lain-iced.exe [--theme <file>] [--server <url>] [--shared <dir>] [--media <dir>]
              [--bench <out.json>] [--shots <dir>] [--mute]
              [--window-pos X,Y] [--window-size WxH]
```

Defaults: server `http://127.0.0.1:9360`, shared `D:\lain-proto\shared`
(theme `theme.json`, art, `series.json`, account `conta-iced.txt`, falling
back to `conta.txt`), media `D:\lain-proto\media`. The session is kept by
`lain-client`'s `FileTokenStore` and reused only if it belongs to that
account.

- `--mute`: mpv gets `ao=null` and `mute=yes` before `mpv_initialize`;
  `current-ao` is logged to `lain-iced.log` next to the exe.
- `--window-pos/--window-size`: physical virtual-desktop pixels. The window
  is created hidden, born at the target position (converted with the
  system DPI winit uses at creation), moved with `SetWindowPos` and shown
  with `SetWindowPlacement(SW_SHOWNOACTIVATE)` (no activation; avoids the
  STARTUPINFO rewrite of the first `ShowWindow`). Fullscreen (`f`, double
  click) is a borderless window on an explicit monitor (the one under
  `--window-pos`, else the window's), not iced's "current monitor" mode.
  winit's always-`WS_VISIBLE` "Thread Event Target" helper window is hidden
  at startup. **Open issue:** a tiling WM on the owner's machine (vertigo)
  re-adopts new windows onto the primary monitor, which no app-side code
  can prevent; the coordinator is handling that.
- `--bench <out.json>`: unattended run (below). `--shots <dir>`: in-app
  screenshots (`window::screenshot`, the app's own frame) of Home, a
  focused card, the 10-bit file with subtitles, the 4K file, and Home again
  (shows a theme reloaded while the player was up).
- Debug: `LAIN_WINDOW_TRACE=1` logs every visible window of the process;
  `LAIN_MPV_OPTS="k=v,k=v"` passes extra mpv options.

Keys: Home: arrows move a focus ring across rails (the rail and page
scroll to it), Enter opens, Esc clears. Mouse: drag a rail (with fling),
Shift+wheel or a horizontal wheel/touchpad scrolls a rail, the plain wheel
scrolls the page. Player: Space, ←/→ 5 s, ↑/↓ volume, `m` mute, `f`
fullscreen, Esc (menu → fullscreen → back), Backspace back.

## mpv integration (the key result)

**Path:** `mpv-engine` 0.2.1 (feature `export`) renders mpv (`vo=libmpv`,
OpenGL on a hidden WGL context) into a pool of shared D3D11 BGRA8 textures
through `WGL_NV_DX_interop2`. This app opens each texture's shared NT
handle on **iced's own D3D12 device** (`ID3D12Device::OpenSharedHandle`
via `wgpu::Device::as_hal::<Dx12>`), wraps it as a `wgpu::Texture`
(`wgpu::hal::dx12::Device::texture_from_raw` +
`create_texture_from_hal`) and samples it in an iced `shader` widget
primitive (fullscreen triangle, sRGB decode when iced's target is sRGB).
No CPU copies; mpv-engine does one GPU-local `CopyResource` per frame.
`src/video.rs` is 386 lines including the pipeline; the import itself is
~40.

What I tried and why:
- DESIGN-v2 mentions an "iced adapter" for mpv-engine: it is not
  published; only the toolkit-agnostic core is on crates.io.
- mpv-engine's own `into_wgpu_texture` (feature `wgpu`) is built on
  **wgpu 30**; iced 0.14 is on **wgpu 27**, so the types do not match. I
  re-did the D3D12 import against wgpu 27 (`windows = "=0.58.0"` so the
  COM types are wgpu-hal's).
- Each pool buffer is opened once and cached by handle (re-opened on
  resize). Frames are held two frames past display before going back to
  mpv (pool of 5), so mpv never renders into a texture the GPU may still
  read. There is no cross-device fence (mpv-engine flushes its D3D11 copy
  but does not wait; a D3D11 fence is on its roadmap); no tearing seen.
- Fallback if the open fails (wgpu not on DX12, GL on another GPU): CPU
  readback (`copy_pixels`) + `queue.write_texture`, reported as
  `import` in the bench JSON. Not hit on this machine.
- **Same GPU is required**: the exe exports `NvOptimusEnablement` /
  `AmdPowerXpressRequestHighPerformance` (build.rs `/EXPORT`) so mpv's
  OpenGL lands on the RTX 3050 Ti, the adapter wgpu picks with
  HighPerformance; `WGPU_BACKEND=dx12` is set at startup.
- mpv events and frames: `set_wakeup_callback` and the render-update
  callback push into a channel (coalesced with atomics) that an iced
  `Subscription::run_with(session_id, …)` turns into messages; the update
  pumps `pump_events()` (observed properties) on wakeups.

Results: `hwdec-current=nvdec` (auto-safe) on both files, `vo=libmpv`;
ASS typesetting/karaoke rendered by mpv (shots/dark/player-10bit.jpg).
The overlay is correct by construction: the video is the stack's base
layer and the chrome is a later layer, so iced draws it on top; it fades
with the theme's motion and auto-hides (cursor too). Resize re-renders at
the new physical size (`set_export_size`). Latency: newest-wins frames, one
UI frame of delay plus the 2-frame hold for buffer reuse (not for display).

## Theme coverage (`shared/theme.json`, `--theme`)

| key | status | notes |
|---|---|---|
| colors.bg / surface / text / muted / accent | honored | |
| colors.danger | honored | playback error text |
| colors.scrim | honored | left scrim behind the hero title |
| fonts.ui / display | honored | Inter and Fraunces are **bundled** (OFL, `assets/fonts`; Fraunces as static 400/600 instances cut from the variable font with fonttools, opsz 72) |
| fonts.mono | partial | "JetBrains Mono" is not installed under that family name (only "JetBrainsMono Nerd Font"), so cosmic-text falls back to a sans; not bundled |
| fonts.size_base | honored | every text size derives from it |
| radius.card / button | honored | `image.border_radius`, buttons, chips, menus |
| spacing | honored | paddings/gaps index the scale |
| rail.card_w / card_h / gap | honored | landscape cards (theme-alt 240x180, Continue) use the backdrop art, portrait use the poster |
| rail.hover_scale | honored | `float` widget scale + shadow, animated; 1.0 disables the lift |
| hero.height | honored | fraction of window height |
| hero.gradient | honored | linear gradient scrim |
| hero.blur | honored (static) | the backdrop is blurred on the CPU when decoded (quarter-res gaussian); iced has no live backdrop filter |
| motion.fast_ms / normal_ms | honored | hover lift / rail scroll and chrome fade |
| motion.easing | honored / partial | CSS-like names map to iced's easings; `spring` is a custom curve (under-damped, time-normalised), not a physical spring |
| player.controls_bg / seek_h / autohide_ms | honored | |
| name | n/a | written into the bench notes |

**Live reload works**: the file's mtime is polled every 500 ms; a change
rebuilds the theme, re-lays rails and re-decodes art at new sizes.
Verified with `shots/reload/` (started on lain-dark, `theme-alt.json`
copied over the file mid-run, `home-end.jpg` is paper-light).

## Bench (`--bench`)

Logs in, waits for Home with all art (cold start = OS process creation →
first `RedrawRequested` after the art is in), settles 1.5 s (RSS = working
set), jumps to the 500-card rail and scrolls it end to end at 4000 px/s
(106,182 px), then plays the 4K file 20 s and the 10-bit file 20 s with the
chrome pinned, then exits. Frame times are the intervals between iced's
own `window::frames()` instants (real redraws; this panel runs at 75 Hz, so
13.3 ms is a perfect frame). mpv stats are read at the end of each file.

My runs are **not final** (three agents compiling at once, and they predate
the `--mute`/placement rules): 

- Run A (heavy CPU contention): cold 1639 ms, RSS home 235 MB, RSS 4K
  565 MB; scroll p50 13.33 / p95 14.27 / p99 15.30 / max 148.7 ms; UI during
  video p50 13.33 / p95 39.8 ms; 4K: 246 dropped (939 shown in 20 s),
  10-bit (the old 24 fps encode): 23 dropped.
- Run B (quieter): cold 869 ms, RSS home 205 MB, RSS 4K 536 MB, 4K: 0
  dropped, 1194 frames shown in 20 s, 0 delayed.
- Run C (`LAIN_MPV_OPTS=hwdec=nvdec-copy`): scroll p50 13.31 / p95 15.92 /
  p99 17.69 / max 19.69 ms; UI during video p50 13.31 / p95 13.98 ms; 0
  drops on both files.

## Numbers

- Lines (`wc -l`): 3,774 in `src/` + 30 `build.rs` = 3,804. video.rs 386,
  app.rs 1,107 (state/update/subscriptions/bench/shots), home.rs 425,
  placement.rs 272 (window rules, Win32), ui.rs 215, player.rs 285,
  theme.rs 264, data.rs 244, seekbar.rs 150, strip.rs 142, bench.rs 159,
  main.rs 125.
- Release exe: 21.3 MB (fonts embedded, ~1.4 MB) + libmpv-2.dll 124 MB.
- Clean release build: 4 min 44 s with `-j 6` while two other agents were
  compiling (not a quiet-machine number).

## What was hard / framework notes

- No virtualized list in iced: a rail is a small custom widget (`Strip`,
  children at explicit x in one clipped layer) fed only the ~12 visible
  cards; `Stack` + `pin` works but opens a render layer per child.
- iced's `scrollable` is not smooth; rail motion is an
  `Animation<f32>` (iced's `animation` module, lilt) retargeted on each
  wheel/keypress; hover lift is `Animation<bool>` per card.
- Posters are decoded and cropped off-thread (tokio blocking pool), then
  pinned in the GPU atlas with `image::allocate` so scrolling never
  re-uploads.
- A container border draws under its content, so the focus ring is a
  separate stack layer.
- `float` draws its shadow even when not floating; disabled when the theme
  has no lift.
- winit + Windows: the helper window above, the WM_DPICHANGED handler that
  moves windows when they first show on another-DPI monitor, and the
  STARTUPINFO rewrite of the first `ShowWindow` all fought exact
  placement (`src/placement.rs`).

## Not done / not verified

- Mouse wheel, drag/fling, keyboard nav and fullscreen were exercised only
  through the in-app automation (focus/keyboard reveal in `--shots`); no
  synthetic input is allowed on the shared desktop.
- The placement rules could not be proven end to end yet (vertigo WM, see
  above); GUI runs are on hold per the coordinator.
- macOS/Linux: the D3D12 import is Windows-only; elsewhere the CPU fallback
  path would be used (mpv-engine has dmabuf/IOSurface exports for later).

## Direct dependencies

| crate | why |
|---|---|
| iced 0.14 (`wgpu`, `tokio`, `image`, `advanced`, `crisp`, `web-colors`) | the framework; `advanced` for the custom widgets |
| lain-client (path) | login, catalog, Continue, progress, events |
| mpv-engine 0.2.1 (`export`) | libmpv lifecycle/events and the exported-frame (shared D3D11) renderer; pulls rsmpv/rsmpv-sys (links `mpv.lib`) |
| tokio | lain-client runs on it; blocking pool for decoding |
| image 0.25 (`jpeg`, `png`) | off-thread art decode/crop/blur; PNG for screenshots |
| serde, serde_json | theme file, bench JSON, series.json |
| windows =0.58.0 (Win32 Foundation, Direct3D12) | `OpenSharedHandle` with the exact COM types wgpu-hal 27 uses |

Other Win32 calls (memory info, process times, window placement) are a few
hand-written `extern "system"` declarations, no crate.
