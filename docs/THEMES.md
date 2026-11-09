# Themes: `lain.ui.theme@1`

Tier 1 of the modding model (DESIGN-v2 §8.3, D-167, D-183). A theme is
data: a folder that any lain client can read and apply. The desktop app
implements it first; the web UI and Android follow this document.

## The folder

```
<id>/
  theme.json          the tokens (required)
  icons/<name>.svg    optional overrides of built-in icons, by name
  wallpapers/...      optional art (referenced from theme.json)
```

`<id>` is the folder name: lowercase letters, digits, `-` and `_`. A
theme is shared as a zip of that folder (the folder itself, or its
contents at the archive root).

Where clients keep them:
- Desktop: `<client dir>/themes/<id>/` (`%LOCALAPPDATA%\lain-client\themes`,
  `~/.local/share/lain-client/themes`). Settings → Appearance lists them,
  installs a zip and opens the folder. The built-in themes are written
  there on first run, so they can be edited.
- Clients reload the active theme when any of its files changes.

## `theme.json`

Every key is optional: a missing key takes the default below, so a theme
can change one color and nothing else. Unknown keys are ignored (forward
compatible). A value that does not parse is reported and replaced
(colors with magenta, so the mistake is visible).

```json
{
  "format": "lain.ui.theme@1",
  "name": "lain dark",
  "author": "lain",
  "scheme": "dark",
  "colors": {
    "bg": "oklch(0.118 0.024 295.6)",
    "surface": "oklch(0.237 0.015 296.7)",
    "raised": "oklch(0.19 0.02 296 / 0.94)",
    "line": "oklch(0.95 0.044 118.3 / 0.10)",
    "text": "oklch(0.950 0.044 118.3)",
    "muted": "oklch(0.62 0.011 299.0)",
    "accent": "oklch(0.596 0.108 284.5)",
    "on_accent": "oklch(0.985 0.02 118.3)",
    "danger": "oklch(0.697 0.115 20.4)",
    "ok": "oklch(0.76 0.11 150)",
    "warn": "oklch(0.82 0.12 85)",
    "scrim": "oklch(0.118 0.024 295.6 / 0.80)",
    "focus": "oklch(0.70 0.12 284.5)"
  },
  "type": { "ui": "Inter", "display": "Fraunces", "mono": "JetBrains Mono", "base": 15, "scale": 1.25 },
  "radius": { "card": 14, "button": 10, "panel": 16 },
  "spacing": [4, 8, 12, 16, 24, 32, 48],
  "blur": { "hero": 0, "panel": 24 },
  "motion": { "fast_ms": 120, "normal_ms": 220, "slow_ms": 420, "easing": "ease-out-cubic" },
  "rail": { "card_w": 200, "card_h": 300, "gap": 16, "hover_scale": 1.06 },
  "hero": { "height": 0.62, "gradient": ["oklch(0.118 0.024 295.6 / 0)", "oklch(0.118 0.024 295.6)"] },
  "player": { "controls_bg": "oklch(0.118 0.024 295.6 / 0.6)", "seek_h": 4, "autohide_ms": 2500 },
  "wallpaper": ""
}
```

### Keys

| Key | Type | Meaning |
|---|---|---|
| `format` | string | `lain.ui.theme@1`. Empty is accepted; another value is refused. |
| `name`, `author` | string | Shown in the theme picker. |
| `scheme` | `dark` \| `light` | A hint for system surfaces and generated art. |
| `colors.bg` | color | The window background. |
| `colors.surface` | color | Cards and art placeholders. |
| `colors.raised` | color | Glass panels, dialogs, toasts (alpha gives the glass). |
| `colors.line` | color | Hairlines and borders. |
| `colors.text`, `colors.muted` | color | Body text and secondary text. |
| `colors.accent`, `colors.on_accent` | color | Primary actions and the text on them. |
| `colors.danger`, `colors.ok`, `colors.warn` | color | States. |
| `colors.scrim` | color | The top bar over art. |
| `colors.focus` | color | The keyboard focus ring. |
| `type.ui`, `type.display`, `type.mono` | font family | UI text, titles, labels and numbers. A family the system does not have falls back to the platform's (desktop: Segoe UI / Georgia / Cascadia Mono; Linux: sans-serif / serif / monospace). The desktop app bundles Inter, Fraunces and JetBrains Mono. |
| `type.base` | px | The base text size. |
| `type.scale` | ratio | Step between sizes: step *n* is `base × scale^n` (labels are step −2, titles 2–5). |
| `radius.card`, `radius.button`, `radius.panel` | px | Corner radii. |
| `spacing` | px list | The spacing scale, by index (0 smallest). |
| `blur.hero` | px | Gaussian blur of hero art. |
| `blur.panel` | px | Backdrop blur of glass panels, where the client can (the GPUI desktop cannot yet: it uses `colors.raised` alone). |
| `motion.fast_ms`, `motion.normal_ms`, `motion.slow_ms` | ms | Focus/hover; scrolling and fades; large moves. |
| `motion.easing` | curve | `linear`, `ease-in-cubic`, `ease-in-out-cubic`, `ease-out-quad`, `ease-out-cubic` (default), `ease-out-expo`, `spring`, or `cubic-bezier(x1, y1, x2, y2)` as in CSS. |
| `rail.card_w`, `rail.card_h`, `rail.gap` | px | Poster card size and gap (landscape sizes give landscape cards). |
| `rail.hover_scale` | ratio | Card growth on focus; `1` turns it off. |
| `hero.height` | fraction | Of the window height. |
| `hero.gradient` | color list | Top → bottom scrim over hero art (the desktop uses the first and last stops). |
| `player.controls_bg` | color | The player's control scrims. |
| `player.seek_h` | px | Seek bar thickness. |
| `player.autohide_ms` | ms | Idle time before the player's controls hide. |
| `wallpaper` | path | Relative to the theme folder: art behind the connect and login screens. It must stay inside the folder. |

### Colors

`oklch(L C H)` or `oklch(L C H / A)`, as in CSS Color 4: `L` lightness
0–1 (or a percent), `C` chroma (about 0–0.4; a percent maps to 0.4), `H`
hue in degrees, `A` alpha 0–1 (or a percent). Colors outside sRGB are
clipped. Hex `#rgb`, `#rrggbb` and `#rrggbbaa` are accepted too.

OKLCH is the canonical form because equal steps of `L` look equal, which
keeps text contrast predictable when a theme changes hue.

### Icons

`icons/<name>.svg` replaces the built-in icon of that name. Icons are
drawn as a single-color mask tinted by the client, so only the shape
matters (any fill or stroke color works). The names the desktop app uses:

`alert audio back book check chevron clock close command download empty
external eye folder fullscreen home key library lock mute next off offline
palette pause play plus refresh search server settings subtitles trash
unlock user volume windowed`

## Safety

A theme is data and nothing else: no scripts, no URLs fetched, no paths
outside its folder. Installing a zip refuses entries with `..` or
absolute paths, caps the unpacked size (64 MiB) and the entry count.
Logic belongs to tier 3 (WASM extensions, DESIGN-v2 §8.3).

## Versioning

A change that only adds keys keeps `@1`. Changing the meaning of a key
needs `lain.ui.theme@2`, and clients keep reading `@1`.
