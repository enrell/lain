# lain design system

Extracted from the landing page (`site/index.html`, `site/styles.css`,
`site/app.js`, as published in PR #53) to serve as the base for the web
client (`web/`) and, later, the native clients. The landing page is the
reference: when this file and the page disagree, the page wins until
this file is updated.

It extends the visual language already decided in
`docs/advisor/decisions.md` D-022 (full-bleed art, mono uppercase labels,
large tight type, glass panels, no boxed banner cards) and the audience
rules in `AGENTS.md` (keyboard-first, dense, fast, direct).

## 1. Principles

1. **The product is the picture.** Show the real client, real covers and
   real pages. No illustrations, no stock photos, no mockup art. A still
   is labelled as a still ("A picture of the Android app").
2. **Type does the work, not boxes.** Hierarchy comes from size, weight
   and the serif italic accent, not from cards and borders. A border is a
   hairline; a panel is a tone step.
3. **One accent, used sparingly.** The warm rose accent marks the primary
   action, the current item and keys. Everything else is neutral.
4. **Every action has a key, and shows it.** A `<kbd>` sits next to the
   action it triggers. On a touch device the keys are hidden and the copy
   says "tap" instead of "click" or "press".
5. **Motion explains, then gets out of the way.** It follows the scroll or
   the user's action, eases out, is interruptible, and is off under
   `prefers-reduced-motion`.
6. **Sharp or nothing.** Anything with text in it lands on whole pixels,
   with no rotation and no scale, before it can be read or used.
7. **Honest copy.** Short declaratives, numbers instead of adjectives, and
   a section that says what lain is not.

## 2. Tokens

### 2.1 Colour

Dark only (`color-scheme: dark`). Contrast ratios are against `--bg`
(`#0c0b0c`), then against `--panel`.

| Token | Value | Use | Contrast |
|---|---|---|---|
| `--bg` | `#0c0b0c` | page background | — |
| `--surface` | `#131112` | frames, tab strips, code wells | — |
| `--panel` | `#181516` | raised panels, keycaps, inline code, bars | — |
| `--line` | `rgba(250,252,251,.09)` | hairlines, row rules, quiet borders | — |
| `--line-strong` | `rgba(250,252,251,.16)` | control borders, frame borders, kbd | — |
| `--fg` | `#fafcfb` | text, headings | 19.1 / 17.6 |
| `--muted` | `#b3aeaf` | secondary text, subs, row values | 9.0 / 8.3 |
| `--dim` | `#8b8586` | tertiary text, numerals, footnotes | 5.4 / 5.0 |
| `--accent` | `#b59790` | primary button, focus ring, current item, eyebrow | 7.3 / 6.7 |
| `--accent-hi` | `#cdb1aa` | serif accent words, links, accent hover | 9.8 / 9.0 |
| `--accent-fg` | `#140f0e` | text on `--accent` | 7.1 on accent |
| `--add` | `#9fc6a3` | positive / added (diffs, success) | 10.4 / 9.6 |
| `--del` | `#c98b8b` | negative / removed (diffs, danger) | 7.1 / 6.5 |
| live dot | `#7fd39a` | "live" status dot only, with a pulse | — |

Derived values are written with `color-mix()` from these tokens, never as
new hex values:

- selection: `color-mix(in srgb, var(--accent) 40%, transparent)`
- glass header: `color-mix(in srgb, var(--bg) 90%, transparent)` + `backdrop-filter: blur(14px) saturate(140%)`
- glass panel: `color-mix(in srgb, var(--surface) 86%, transparent)` + `blur(14px)`
- selected tab: `color-mix(in srgb, var(--accent) 26%, var(--panel))`
- quiet button hover border: `color-mix(in srgb, var(--accent) 55%, transparent)`
- link underline: `color-mix(in srgb, var(--accent) 50%, transparent)`

**Background art.** Behind the page sits one WebGL fragment shader
(warped fbm noise in the chapter's hue, three layers of blinking motes, a
dither against banding) on a canvas at 0.75 of the device pixels, with a
CSS radial-gradient fallback. Each chapter sets `--hue: r, g, b` and the
sky eases to it. This is landing-page art; the client uses full-bleed
cover art in the same role (see §6).

### 2.2 Type

Three families, no web font except the serif:

| Token | Stack | Role |
|---|---|---|
| `--sans` | `ui-sans-serif, system-ui, -apple-system, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif` | everything by default |
| `--serif` | `'Instrument Serif'` (400 roman and italic, woff2, `font-display: swap`), then `'Iowan Old Style', 'Palatino Linotype', Georgia, serif` | the accent word of a heading, ledes, subs, captions' second line |
| `--mono` | `ui-monospace, 'SF Mono', SFMono-Regular, Menlo, Consolas, 'Liberation Mono', monospace` | eyebrows, taglines, code, keys, status lines |

Scale (fluid with `clamp`; the client, which is not a scroll story, uses
the smaller end):

| Style | Size | Weight / tracking / leading |
|---|---|---|
| Display (h1) | `clamp(2.7rem, 6.2vw, 5.8rem)` | 700, `-0.055em`, 0.9 |
| Section (h2) | `clamp(2.6rem, 7.4vw, 6.6rem)`; plain sections `clamp(2.2rem, 5.4vw, 4.4rem)` | 700, `-0.055em`, 0.9 |
| Caption title (h3 in a showcase) | `clamp(1.5rem, 3.2vw, 2.4rem)` | 700, `-0.045em`, 1 |
| Item title (h3) | `1.05rem` | 700, `-0.015em`, 1.3 |
| Lede / sub | serif, `clamp(1.2rem, 1.9vw, 1.6rem)`, max `40ch` | 400, 1.12–1.25, `text-wrap: balance` |
| Body | sans `16px` | 400, 1.6 |
| Row term / value | sans `1rem` | 700 / 600, `-0.025em` / `-0.02em`, 1.2 |
| Eyebrow / tagline | mono `11px`, uppercase | `0.26em` tracking, `--accent` |
| Status chip | sans `0.76rem` | 600 |
| Button | sans `0.95rem` (header `0.86rem`) | 600 |

**The accent word.** A heading is set in the bold sans with its last
phrase in the serif italic, `--accent-hi`, weight 400, tracking `-0.02em`:
"Everything you watch *and read.*", "Titles, *not files.*". One accent
phrase per heading, always the closing one.

**Eyebrow with a numeral.** `<p class="eyebrow"><b>II</b> Watch</p>`: the
numeral in the serif, `--dim`, behind a hairline divider; the label in
mono uppercase accent. Large outline numerals (`clamp(14rem,34vw,30rem)`,
serif italic, `rgba(250,252,251,.045)`) may sit behind a section heading.

### 2.3 Space and layout

- Content width: `--wrap: min(1120px, 100vw - 40px)`; below 600px
  `calc(100vw - 32px)` (16px gutters, no horizontal page scroll ever).
- Header height: `--nav: 60px`; anchors use `scroll-padding-top: 76px`.
- Vertical rhythm is in viewport units on the page (sections `26vh` top,
  blocks `8–9vh` apart). In the client use the 4px grid: 4, 8, 10, 12,
  14, 16, 18, 22, 24, 30 px are the steps actually used.
- Rows (`dl.rows`): two columns of term + value, `11fr / 14fr`, a top
  hairline each, `16px 0 18px` padding; one column below 940px, stacked
  below 600px.
- Stats strip: four equal cells divided by hairlines, ruled top and
  bottom; two by two below 940px.

### 2.4 Radius

| Token | Value | Use |
|---|---|---|
| `--radius` | `14px` | panels, frames, palette, code figures |
| control | `10px` | buttons, header controls |
| small | `8px` | tabs inside a strip, copy buttons |
| tiny | `6px` | kbd, inline code |
| pill | `999px` | chips, release pill, toast |
| device | `40px` outer / `31px` screen | phone frames |

### 2.5 Elevation

No drop shadows on controls. Depth is for things that float above the
page:

- floating frame: `0 60px 120px -40px rgba(0,0,0,.95)`, plus an inset top
  highlight `0 1px 0 rgba(255,255,255,.04) inset`
- dialog (palette): `0 60px 140px -30px #000`
- toast: `0 20px 50px -20px #000`
- accent glow (hover on a pickable screen): `0 0 0 4px rgba(181,151,144,.18), 0 0 70px rgba(181,151,144,.35)`

### 2.6 Motion

| Token | Value | Use |
|---|---|---|
| `--out` | `cubic-bezier(0.16, 1, 0.3, 1)` | entrances, transforms |
| quick | `0.15s` linear-ish | colour and border changes on hover |
| rise | `translateY(26px) → 0`, opacity, `1s var(--out)`, children staggered 120–140ms | first appearance of a block |
| word rise | each word of a heading from `translateY(105%) rotate(4deg)`, opacity 0, 70ms per word | section headings when seen |
| press | `scale(0.97)` on `:active` | buttons |
| pop | dialog open, `0.35s var(--out)` | palette |

Rules:

- Seen-once reveals use an `IntersectionObserver` with
  `rootMargin: 0px 0px -14% 0px`, then unobserve.
- Scroll-linked values are eased toward their target each frame
  (`lerp`, factor ≈ 0.1 per 60 Hz frame), never jumped.
- A hidden word is fully hidden (opacity 0), never a sliver over its clip.
- Under `prefers-reduced-motion: reduce` nothing flies, nothing is pinned
  to a moving camera, and every reveal shows its end state.
- The page pauses its frame loop when the tab is hidden.

## 3. Components

### Keycap (`kbd`)

Mono `0.88em`, `min-width: 1.7em`, `padding: 1px 6px`, border
`--line-strong` with a 2px bottom, radius 6px, background `--panel`. In a
primary button the keycap goes translucent dark on the accent
(`rgba(20,15,14,.12)` fill, `.35` border). Shown after the label:
`Install [I]`, `Copy [C]`, `Back to the page [Esc]`, `Jump to [Ctrl][K]`.
Hidden under `(hover: none) and (pointer: coarse)`.

### Buttons

| Variant | Look |
|---|---|
| primary | `--accent` fill, `--accent-fg` text, hover `--accent-hi` |
| quiet | `--line-strong` border, `color-mix(--bg 60%)` fill, hover `--panel` fill and accent border |
| copy | 34px tall, `--panel`, `--line-strong`, radius 8; on success it turns accent and says "Copied" for 1.4s |

Height 46px (header 36px, 44px minimum on touch), padding `0 20px`, gap
10px, radius 10px, `white-space: nowrap`. One primary per view.

### Link

`--accent-hi`, underline in half-strength accent, offset 4px; the
underline goes full accent on hover.

### Eyebrow, tagline, release pill

Mono 11px uppercase, `0.26em`, `--accent`. The release pill is the same
text in `--muted` inside a 999px hairline pill, linked to the release.

### Status chip

28px pill, `--line-strong` border, `0.76rem` 600, a 7px dot. States:
loading (dim dot), live (accent border, `--accent-hi` text, green dot with
a 2s pulse), driving (accent fill, `--accent-fg` text), still (dim dot, no
pulse). The text says what to do next: "Live. Click and use it", "You are
driving. Esc or scroll past its edge to move on".

### Header

Sticky, 60px, transparent over the hero, then glass (`--bg` 90%, blur 14)
with a bottom hairline and a 28px fade under it once scrolled. Brand
left; right: palette opener ("Jump to [Ctrl][K]", "Menu" on phones),
then the primary action and the source link. Actions that matter
anywhere on the page (Install, Live demo) appear in it once the hero is
gone.

### Command palette (Ctrl+K)

A modal `<dialog>`, 620px wide, 14vh from the top, glass panel
(`--panel` 92%, blur 24), radius 14. An input, a listbox of rows (kind in
mono uppercase dim, label, keys right-aligned), a footer of key hints
(`↑↓ move`, `Enter go`, `Esc close`). Arrow keys move, Enter runs, a click
on the backdrop closes. Every section and every action of the screen is
in it; an action with a key shows the key.

### Tabs

A strip on `--surface` with a hairline and radius 12, 4px padding; tabs
36px, radius 8, `--muted`; selected: accent-tinted panel and `--fg`.
WAI-ARIA tab pattern: arrows move, Home/End jump, roving `tabindex`.
On phones, five tabs lay out three over two, none alone on a row.

### Code block

`--bg` well with a hairline and radius 10, mono `0.86rem`, line height
1.7, horizontal scroll, a copy button top-right. Diffs: removed lines in
`--del` that fade and strike through when seen, added lines in `--add`
that slide in after them.

### Terminal

A frame with a 40px bar (three dim dots), mono, `$` in `--accent`,
comments in `--dim`. Commands type themselves once when seen (18–52ms per
character). On phones the comment moves under its command.

### Rows and stats

See §2.3. Values are one line; a term is a noun phrase, never a sentence.

### Live frame

A real client in an iframe, laid out at its own width and zoomed (CSS
`zoom`, not `transform: scale`) so text is rasterised at the shown size.
It stays out of the tab order (`inert`) and may not take focus until the
visitor engages it (click, Enter, or the palette); Esc inside it hands
the keyboard back. A still picture of the same screen stands in until it
has drawn, and where it cannot.

### Rail (chapter index)

Fixed bottom-left, on a small glass scrim. Full titles from 1500px,
numerals plus the current title from 1000px, hidden below. The current
chapter is `--fg` with a dotted leader; the rail steps aside while a
picture fills the window and is `inert` while invisible.

### Toast

Fixed bottom-centre pill on `--panel`, `--line-strong`, 600 `0.84rem`,
one line with ellipsis; rises 12px into place, stays 2.2s; `role=status`,
`aria-live=polite`. It confirms what happened ("Copied: docker run ..."),
and when an action fails it says what to do instead ("Selected. Press
Ctrl+C to copy").

## 4. Interaction and accessibility

- Keyboard-first, as `AGENTS.md` requires. Single-letter keys work
  anywhere outside a text field and with no modifier; `?` and `/` open the
  palette; Ctrl+K always opens it.
- A jump (key, palette or link) moves focus to the target heading
  (`tabindex="-1"`, `focus({ preventScroll: true })`) so Tab continues
  from there.
- Focus ring: 2px `--accent`, offset 3px, radius 6px, `:focus-visible`
  only.
- The skip link is `position: fixed` and appears at the top on focus.
- Touch: targets at least 44px under `(pointer: coarse)`; copy has an
  `.on-fine` / `.on-touch` pair wherever it names an input ("Click a
  screen" / "Tap a screen", "Press / anywhere" / "Search from anywhere").
- Anything that plays sound stops when its container is left.
- Contrast: body text at least `--muted` (9:1); `--dim` (5.4:1) only for
  numerals, hints and footnotes, never for something you must read to act.

## 5. Voice

- Short declarative sentences; the subject is the product or the reader.
  "Plays the file as it is." "Filters say how many they leave."
- Numbers instead of adjectives: "8.7 MB", "1 file of data", "0 accounts
  outside your own server".
- Name the limits: "What lain is not."
- No exclamation marks, no "simply", "seamless", "powerful", "blazing".
- Headings follow the accent-word pattern (§2.2). Captions are a heading
  plus one serif line.

## 6. Applying this to the client (`web/`)

The client's tokens live in `web/src/app.css` (`@theme`) and are replaced
at run time by the server host's validated Omarchy palette
(`docs/THEMES.md`). This system becomes the **default** palette and the
shape every theme keeps; a theme changes colours, not structure.

### Token mapping

| Landing | Client token today | Today's value | Proposed default |
|---|---|---|---|
| `--bg` | `--color-background` | `#08090c` | `#0c0b0c` |
| `--surface` | `--color-surface` | `#101218` | `#131112` |
| `--panel` | `--color-surface-hover` / `--color-panel` | `#171a22` / mix | `#181516` |
| — | `--color-surface-active` | `#1e222c` | `#1e1a1b` |
| `--fg` | `--color-foreground` | `#e7e9f0` | `#fafcfb` |
| `--muted` | `--color-muted` | `#8a90a3` | `#b3aeaf` |
| `--dim` | (none) | — | add `--color-dim: #8b8586` |
| `--line` | `--color-hairline` | fg 9% mix | unchanged (already the same rule) |
| `--line-strong` | `--color-line` | `#1f2430` | `color-mix(in srgb, var(--color-foreground) 16%, transparent)` |
| `--accent` | `--color-accent` | `#5ce1c4` | `#b59790` |
| `--accent-hi` | `--color-accent-hover` | `#7af0d6` | `#cdb1aa` |
| `--accent-fg` | `--color-accent-fg` | `#05231d` | `#140f0e` |
| `--add` | `--color-success` | `#6ee7a8` | `#9fc6a3` |
| `--del` | `--color-danger` | `#ff6b81` | `#c98b8b` (keep a brighter danger for destructive confirmations if contrast on the accent demands it) |
| `--serif` | (none) | — | add `--font-serif`, self-host Instrument Serif 400 roman + italic |
| `--radius` | `--radius-card` | `0.625rem` | panels 14px, controls 10px, cards stay 10px |
| `--out` | `--ease-out-quick` | `(0.2,0.8,0.2,1)` | keep for the player; add `--ease-out: (0.16,1,0.3,1)` for entrances |

### Adopt in the client

- The heading pattern (bold sans + serif italic accent) for page titles:
  Home ("Where you *left off*"), Library, a title's page, Settings.
- Eyebrow / tagline styling for section labels and metadata rows.
- `kbd` beside every action, the palette shape, the toast, the copy
  button, tabs and the status chip as specified in §3.
- Rows for settings and details (it is already the `SettingRow` shape of
  D-087: label left, control right).
- The glass header and the hairline / tone-step panels instead of boxed
  cards.
- Integer-pixel, unrotated, unscaled text wherever something can be read.

### Keep on the landing page only

- The scroll-driven chapters, pinned showcases, the 3D hero and the
  word-by-word manifesto: the client is a tool, not a story.
- The shader sky: in the client, cover art (blurred only behind panels,
  never behind text you read) plays that role.
- The large outline numerals.

### Order of work

1. Add the missing tokens (`--color-dim`, `--font-serif`, `--ease-out`)
   and the serif font files; switch the default palette values; check
   every Omarchy theme and `lain.ui.theme@1` theme still reads (contrast
   of `--color-dim` and the serif accent on each background).
2. Shared components: `kbd`, button variants, tabs, toast, status chip,
   palette rows, to these specs.
3. Page titles and section labels to the type rules.
4. Screen by screen, with the same two-reviewer rounds used on the
   landing page.
