# Modding: skins and extensions

Tiers 2 and 3 of the modding model (DESIGN-v2 §8.3, P6, D-255..D-264).
Tier 1, themes, is [`THEMES.md`](THEMES.md). The desktop app (GPUI,
D-180) implements both tiers; other clients follow this document.

| Tier | What | Format | Can run code |
|---|---|---|---|
| 1 Theme | tokens, icons, wallpapers | `lain.ui.theme@1` (`theme.json`) | no |
| 2 Skin | which blocks appear, where, in what order | `lain.ui.layout@1` (`layout.json`) | no |
| 3 Extension | rails, panels, reactions to events | `lain.extension@1` + a WebAssembly component | yes, sandboxed |

## Tier 2: layout slots (`lain.ui.layout@1`)

GPUI loads no markup at run time, so a skin does not describe widgets.
It picks and orders the app's own building blocks through named slots
(DESIGN-v2 §8.3: "With GPUI or iced this tier becomes a set of layout
slots chosen through config").

A skin is a theme folder with a `layout.json` next to its `theme.json`:

```
<id>/
  theme.json      tokens (tier 1; may be all defaults)
  layout.json     slots (tier 2)
```

It installs, zips and hot-reloads like any theme: editing `layout.json`
re-lays the app out within half a second. A theme without `layout.json`
uses the built-in layout. Every key is optional; an unknown value is
listed in Settings → Appearance ("theme notes") and the default is used
for it. A file whose `format` is not `lain.ui.layout@1` is ignored as a
whole (reported).

### Schema

```json
{
  "format": "lain.ui.layout@1",
  "name": "shelf",
  "home": {
    "hero": "full",
    "rails": ["continue", "recent", "libraries", "extensions"],
    "recent_limit": 30
  },
  "item": {
    "header": ["kicker", "title", "facts", "genres", "synopsis", "actions"],
    "columns": ["episodes", "details", "panels"]
  },
  "player": {
    "title": "top",
    "controls": ["play", "next", "time", "spacer", "mute", "volume", "audio", "subs", "fullscreen"],
    "seek": "above",
    "hints": true
  }
}
```

The values above are the defaults (the built-in layout).

| Key | Values | Meaning |
|---|---|---|
| `home.hero` | `full` · `compact` · `none` | The featured title at the top of Home: full height (`theme.json` `hero.height`), 60 % of it with a smaller title, or no hero (rails start under the navigation, focus starts on the first card). |
| `home.rails` | list of rail slots | Which rails Home shows, top to bottom. A rail not listed is not shown. |
| `home.recent_limit` | 1–500 | Cards in the `recent` rail. |
| `item.header` | list of `kicker` `title` `facts` `genres` `synopsis` `actions` | The parts of a title page's header, in order; a part not listed is hidden. `kicker` is the kind and library, `facts` the file count and progress, `actions` the play/watched/download buttons. |
| `item.columns` | list of `episodes` `details` `panels` | The columns under the header, left to right. `details` is the selected file's panel, `panels` the extension panels (hidden while there are none). |
| `player.title` | `top` · `bottom` · `hidden` | Where the title and episode label sit in the player overlay. |
| `player.controls` | list of `play` `next` `time` `spacer` `mute` `volume` `audio` `subs` `fullscreen` | The control row, left to right; `spacer` takes the free space and may repeat. Keyboard shortcuts keep working for controls not shown. |
| `player.seek` | `above` · `below` | The seek bar above or below the control row. |
| `player.hints` | bool | The keyboard legend under the controls. |

Rail slots:

| Slot | Rails |
|---|---|
| `continue` | Continue watching |
| `recent` | Recently added |
| `libraries` | One rail per library not placed by name |
| `library:<name>` | That library (name, case-insensitive, or id) |
| `extensions` | Every extension rail not placed elsewhere |
| `ext:<extension id>` | Every rail of that extension |
| `ext:<extension id>/<rail id>` | One rail of that extension |

A rail appears once: the first slot that names it places it, so
`["library:Anime", "libraries"]` puts Anime first and the other libraries
after it. Repeating a slot is reported and ignored.

### The sample skin: `shelf`

Built in (`apps/desktop/themes/shelf/`, written to the themes folder on
first run like the other built-ins). Same colors as `lain-dark`, so only
the layout changes:

- Home: a compact hero, extension rails first, then Continue, then the
  libraries; no Recently added.
- Title page: title and actions first, then facts and synopsis; the
  file details and extension panels on the left, the episode list on
  the right.
- Player: the title above the seek bar, the seek bar under the controls,
  no key legend.

Pick it in Settings → Appearance or Ctrl+K → "Theme: lain shelf (skin)".

## Tier 3: extensions (`lain.extension@1`)

An extension is a [WebAssembly component](https://component-model.bytecodealliance.org/)
that targets the `lain:extension` world. The desktop app runs it in
[wasmtime](https://wasmtime.dev) with a capability-based host API, the
model Zed uses for its extensions: it can add rails to Home, add panels
to title pages, react to events and call the lain API scopes the user
granted. It cannot touch files, the network or other extensions unless
it declares a scope for it and the user grants it.

### The folder

```
<id>/
  extension.json     the manifest
  <name>.wasm        the component (at most 32 MiB)
  README.md          optional
  data/              created by the host for fs:data, private to it
```

Installed extensions live in `<client dir>/extensions/<id>/`
(`%LOCALAPPDATA%\lain-client\extensions`,
`~/.local/share/lain-client/extensions`). Settings → Extensions installs a
folder or a zip of one (the folder or its contents at the archive root):
only the manifest, the component and `README.md` are copied, paths with
`..` or a root are refused, a reinstall replaces the code and keeps
`data/`. Uninstall removes the folder, data included.

### `extension.json`

```json
{
  "format": "lain.extension@1",
  "id": "seasonal-stats",
  "name": "Seasonal stats",
  "version": "0.1.0",
  "author": "lain",
  "description": "A Home rail of the titles that got new episodes this season, and season stats on title pages.",
  "api": "0.1.0",
  "wasm": "seasonal_stats.wasm",
  "scopes": ["catalog:read", "progress:read", "events:subscribe", "ui:rail", "ui:panel"],
  "config": { "days": 0 }
}
```

| Key | |
|---|---|
| `format` | `lain.extension@1` (required). |
| `id` | Lowercase letters, digits, `-`, `_`; at most 64; the folder name. |
| `name`, `version`, `author`, `description` | Shown in the permission prompt and in Settings. |
| `api` | The `lain:extension` world version it was built against. The host loads `0.x` with `x` up to its own minor (0.1 today). |
| `wasm` | The component's file name inside the folder. |
| `scopes` | What it asks for (below). An unknown scope refuses the install: the extension needs a newer app. |
| `config` | Any JSON, handed to `activate` as a string. |

### Scopes

| Scope | Allows | Host calls / exports |
|---|---|---|
| `catalog:read` | Libraries and every item the account can see (no file paths; locked private libraries and missing files are left out) | `list-libraries`, `list-catalog`, `get-item` |
| `progress:read` | The user's progress and Continue list | `get-progress`, `continue-watching` |
| `events:subscribe` | Being told about progress and library changes, opened titles, playback start and stop | `on-event` is called |
| `ui:rail` | Rails on Home | `rails` is called |
| `ui:panel` | Panels on title pages | `panels` is called |
| `fs:data` | A private folder, `/data` inside the extension (at most 64 MiB) | WASI filesystem |
| `net:<host>` | HTTP(S) GETs to exactly that host (`example.org`, or `example.org:8080` for another port); no wildcards | `http-get` |

Everything else is denied: a host call without its scope answers
`host-error::denied("<scope>")` and never reaches the server; an export
whose scope is missing is never called. WASI is linked (every
`wasm32-wasip2` program imports it) with nothing behind it: no
environment variables, arguments, stdin, sockets or name lookups, and no
folder unless `fs:data` is granted. Stdout and stderr go to the app's log.

Server capabilities are reached only through the API with the user's
session (DESIGN-v2 §8.3): the host calls `lain-client`, which carries the
token; the extension never sees it. `http-get` never sends the lain token,
does not follow redirects (a 3xx goes back to the extension, which can
follow it only to a granted host), times out after 8 s and caps the body
at 1 MiB.

### Permissions

- The first time the app finds an extension (installed from Settings or
  dropped into the folder), a prompt lists every scope it asks for in
  plain words, sensitive ones (`net:`, `fs:data`) marked. **Enter**
  grants them all and turns the extension on; **Esc** leaves it off with
  nothing granted. Tab moves between the two buttons.
- An update that asks for a scope the user has not seen shows the prompt
  again, with the new scopes marked; the extension does not run until it
  is answered.
- Settings → Extensions lists each extension with its state, a toggle,
  every requested scope with its own Granted/Not granted switch, its last
  log lines and Uninstall. A change restarts the extension with the new
  grants. An extension runs with the requested scopes the user granted,
  never more.
- Grants are kept per extension in `desktop.json` (`extensions.<id>`:
  `enabled`, `granted`, `reviewed`, `error`).

### Limits and crash isolation

Each extension runs on its own thread with its own wasmtime store:

| Limit | Value |
|---|---|
| CPU per call | 4·10⁹ fuel (about that many wasm instructions) |
| Wall clock per call | 10 s, host calls included (epoch interruption, 10 ms ticks) |
| Linear memory | 64 MiB per memory |
| Host calls per call | 4096 |
| Component size | 32 MiB |
| `fs:data` | 64 MiB |
| Output kept | 8 rails × 200 cards, 4 panels × 24 rows per item; titles 120 characters, values 200, panel text 2000 |

A trap ends the extension, never the app: a panic, running out of fuel
or time, memory over the cap, a stack overflow, an invalid component, a
failed `activate`. The host stops its worker, removes its rails and
panels, turns it off, saves the reason and shows it ("Extension Seasonal
stats stopped and was turned off: it used its whole CPU budget for one
call"). It stays off until the user turns it on again in Settings.

### The world (`crates/lain-ext-api/wit/lain-extension.wit`)

```wit
package lain:extension@0.1.0;

world extension {
    import host;                                  // log, granted-scopes, invalidate,
                                                  // list-libraries, list-catalog, get-item,
                                                  // get-progress, continue-watching, http-get
    export activate: func(config: string) -> result<_, string>;
    export rails: func() -> list<rail>;           // ui:rail
    export panels: func(item-id: string) -> list<panel>;   // ui:panel
    export on-event: func(event: event);          // events:subscribe
}
```

The host calls `activate` once, then `rails` (with `ui:rail`), `panels`
when a title page opens (with `ui:panel`) and `on-event` for each event
(with `events:subscribe`). An extension calls `invalidate` to have
`rails` called again after the current call. A rail card with an
`item-id` the account can see shows that item's art and opens its page.

Versioning: adding functions or cases bumps the minor version and the
host keeps linking older guests; changing an existing signature bumps the
major version.

### Writing one (Rust)

`crates/lain-ext-api` is the guest SDK, in the shape of Zed's
`zed_extension_api`: implement `Extension`, register it.

```toml
# Cargo.toml
[lib]
crate-type = ["cdylib"]

[dependencies]
lain-ext-api = { path = "../../crates/lain-ext-api" }
```

```rust
use lain_ext_api::{Card, Extension, Rail, register_extension};

struct Hello;

impl Extension for Hello {
    fn new(_config: &str) -> Result<Self, String> {
        Ok(Hello)
    }
    fn rails(&mut self) -> Vec<Rail> {
        let items = lain_ext_api::catalog().unwrap_or_default(); // catalog:read
        let cards = items
            .iter()
            .take(10)
            .map(|i| Card { title: i.title.clone(), subtitle: i.kind.clone(), item_id: Some(i.id.clone()) })
            .collect();
        vec![Rail { id: "hello".into(), title: "Hello".into(), cards }]
    }
}

register_extension!(Hello);
```

```sh
rustup target add wasm32-wasip2
cargo build --release --target wasm32-wasip2
# target/wasm32-wasip2/release/<name>.wasm is the component
```

Any language with component-model tooling works against the WIT file
(`wit-bindgen`, `componentize-py`, `jco`); the host checks the world, not
the language.

### The sample extension: Seasonal stats

`examples/extensions/seasonal-stats` (its own cargo project, as a
third-party extension would be). Scopes: `catalog:read`, `progress:read`,
`events:subscribe`, `ui:rail`, `ui:panel`.

- A Home rail "<Season> <year> · new this season": every title with a
  file added since the anime season began (Winter = January, Spring =
  April, Summer = July, Fall = October), newest first, "N new · M
  finished".
- A "Seasonal stats" panel on each title page: the window, new files,
  finished files, time spent and the last day played this season.
- It caches the catalog, drops it on `library-changed` and refreshes
  the rail on progress events. `config.days` > 0 uses the last N days
  instead of the calendar season.

CI builds it for `wasm32-wasip2` and uploads it as the
`seasonal-stats-wasm` artifact; install it from Settings → Extensions
with a folder holding that file and its `extension.json`.

### Tests

The host's tests (`apps/desktop/src/ext/tests.rs`) run real components:
`crates/lain-ext-api/probe`, a test guest whose `config` picks what it
does (every host call, a busy loop, an allocation spree, a panic, an
oversized answer, a refused `activate`), and the sample. They check scope
enforcement (denied calls never reach the API, no files or environment
without grants), each limit, trap handling and the manager's
disable-on-failure. They look for the guests in `LAIN_EXT_GUESTS` and skip
without them unless `LAIN_REQUIRE_EXT_GUESTS=1`, which CI sets:

```sh
(cd crates/lain-ext-api && cargo build --release --target wasm32-wasip2 --workspace)
(cd examples/extensions/seasonal-stats && cargo build --release --target wasm32-wasip2)
cd apps/desktop && LAIN_EXT_GUESTS=<dir with both .wasm files> cargo test
```
