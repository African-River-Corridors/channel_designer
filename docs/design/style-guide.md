# OWD style guide

How Open Waterway Designer looks, in the app and on the website. The look comes from H. N. Fisk's 1944
plates of the Mississippi meander belt: cream paper, one dark printed ink for every line and letter, and the
river's past courses as coloured, hatched "stages" keyed in a legend.

This guide sets the look. It does not change what the app does. The restyle is its own spec, cut into
tickets; each ticket changes the look only.

The app and the website share one token file: `app/src/tokens.css`, copied to `website/tokens.css`.
`node app/scripts/contrast.mjs` checks it, and `npm run build` runs that check.

## 1 Principles

1. **Ink for the interface, colour for the river.** The chrome is one ink on cream. Links, buttons, focus,
   rules and text are ink. Colour is kept for the design line options.
2. **Screening, not construction.** "Screening. Not for construction." is part of the look, not small
   print. It is in the status line, the splash and every export.
3. **Show the working.** Every number has its unit. Every check has its value, its limit and its source.
4. **Colour is never the only signal.** Options have a hatch as well as a colour. Pass, warn, fail and
   "out of date" each have a word and a shape.
5. **The drawing sheet is white.** Exported drawings print black on white, in both themes. The app never
   tints or inverts a sheet.

## 2 Colour tokens

### The plate (the same in both themes)

| Token | Value | Use |
|---|---|---|
| `--cream` | `#f2e8cf` | The paper. Light theme ground; the casing of option lines on imagery |
| `--plate-ink` | `#2b2925` | The printed ink: the mark, line work, lettering |
| `--hillshade` | `#8c897f` | Relief. Decoration only, never text |
| `--river` | `#fbf8ef` | Water in illustrations. Decoration only |
| `--dusk` | `#23211c` | Dark theme ground; the dark icon tile |

### Interface

| Token | Light | Dark | Use |
|---|---|---|---|
| `--paper` | `#f2e8cf` | `#23211c` | Window ground |
| `--surface` | `#f8f2e3` | `#2c2a24` | Panels, cards |
| `--surface-2` | `#e8dcbd` | `#1a1915` | Insets: table header, code, tab rail |
| `--ink` | `#2b2925` | `#f2e8cf` | Text and lines |
| `--ink-2` | `#4a463e` | `#dacfb4` | Secondary text |
| `--ink-3` | `#625c50` | `#bdb299` | Captions, hints |
| `--rule` | `#d8caa6` | `#4a463c` | Hairlines (not text) |
| `--rule-strong` | `#8a7d5e` | `#8f8670` | Input borders (3:1) |
| `--accent` | = ink | = ink | Primary button fill, focus ring, selected marker |
| `--accent-ink` | `#f8f2e3` | `#23211c` | Text on `--accent` |
| `--accent-soft` | `#e2d5b3` | `#3a372f` | Selected row ground |
| `--sheet`, `--sheet-ink`, `--sheet-ink-2` | white, `#1f1e1b`, `#55524b` | the same | The drawing sheet |

### Stages: the design line options

Each option has a line colour, a wash and a hatch angle. Options are numbered in the order the session
lists them. The stages mean "option", never "good" or "bad".

| Option | `--opt-N` (light line) | `--opt-N-wash` (fill; dark-theme line) | Hatch |
|---|---|---|---|
| 1 vermilion | `#b8432d` | `#eea594` | 45° |
| 2 teal | `#2c6f7c` | `#9fcad1` | −45° |
| 3 meadow | `#5a7a24` | `#c6d98c` | 0° |
| 4 ochre | `#a65f12` | `#f1c37f` | 90° |
| 5 plum rose | `#96487f` | `#e6b6d6` | 22° |
| 6 indigo slate | `#5b58a0` | `#aeb0ee` | −22° |

- **On cream** (light theme panels, exports): the deep line; areas in the wash with the hatch.
- **On dusk** (dark theme panels): the wash is the line.
- **On imagery** (the map, both themes): `--opt-N-map` (the deep colour) over a `--opt-casing` (cream)
  casing. The selected option is wider: line 5 px on a 9 px casing; others 3 px on 6.5 px.
- More than six options: repeat the colours and change the hatch (add 11° per round).

### Status

| Token | Light | Dark | Shape and words |
|---|---|---|---|
| `--pass`, `--pass-soft` | `#24502b`, `#dde8cf` | `#b5dcb0`, `#283a27` | ✓ and the value against its limit |
| `--warn`, `--warn-soft` | `#5a4106`, `#f0dfb2` | `#fbe8be`, `#3d3420` | ! and what to check |
| `--fail`, `--fail-soft` | `#7c1626`, `#f4d9d6` | `#ffd3d6`, `#47262a` | ✕ and the value against its limit |
| `--stale` | `#625c50` | `#bdb299` | Struck through, and the words "out of date" |
| `--src-ruleset` | `#4a3f78` | `#d3c9f5` | An input set by the chosen rule set |

## 3 Contrast (WCAG 2.2 AA)

Text pairs need 4.5:1. Lines and borders that carry meaning need 3:1. The check also keeps each stage at
least ΔE 20 from the other stages and ΔE 15 from each status colour, so an option never looks like a
verdict. Every pair passes in both themes.

| Foreground on background | Kind | Light | Dark |
|---|---|---|---|
| `ink` on `paper` | text | 11.90 | 13.19 |
| `ink` on `surface` | text | 13.00 | 11.76 |
| `ink` on `surface-2` | text | 10.65 | 14.42 |
| `ink` on `accent-soft` | text | 9.96 | 9.74 |
| `ink-2` on `paper` | text | 7.70 | 10.39 |
| `ink-2` on `surface` | text | 8.41 | 9.27 |
| `ink-2` on `surface-2` | text | 6.89 | 11.36 |
| `ink-2` on `accent-soft` | text | 6.44 | 7.68 |
| `ink-3` on `paper` | text | 5.44 | 7.65 |
| `ink-3` on `surface` | text | 5.94 | 6.83 |
| `ink-3` on `surface-2` | text | 4.87 | 8.37 |
| `ink-3` on `accent-soft` | text | 4.55 | 5.65 |
| `accent-ink` on `accent` | text | 13.00 | 13.19 |
| `pass` on `pass-soft` | text | 7.32 | 8.02 |
| `fail` on `fail-soft` | text | 7.87 | 9.84 |
| `warn` on `warn-soft` | text | 7.25 | 10.17 |
| `pass` on `surface` | text | 8.33 | 9.45 |
| `fail` on `surface` | text | 9.40 | 10.62 |
| `warn` on `surface` | text | 8.57 | 11.88 |
| `stale` on `surface` | text | 5.94 | 6.83 |
| `src-ruleset` on `surface` | text | 8.27 | 9.18 |
| `sheet-ink` on `sheet` | text | 16.67 | 16.67 |
| `sheet-ink-2` on `sheet` | text | 7.79 | 7.79 |
| `plate-ink` on `opt-1-wash` | text | 7.22 | 7.22 |
| `plate-ink` on `opt-2-wash` | text | 8.19 | 8.19 |
| `plate-ink` on `opt-3-wash` | text | 9.47 | 9.47 |
| `plate-ink` on `opt-4-wash` | text | 8.88 | 8.88 |
| `plate-ink` on `opt-5-wash` | text | 8.31 | 8.31 |
| `plate-ink` on `opt-6-wash` | text | 7.08 | 7.08 |
| `rule-strong` on `surface` | line | 3.63 | 3.97 |
| `rule-strong` on `paper` | line | 3.33 | 4.45 |
| `ink` on `paper` | line | 11.90 | 13.19 |
| `opt-1` on `paper` | line | 4.44 | 8.00 |
| `opt-1` on `surface` | line | 4.85 | 7.14 |
| `opt-2` on `paper` | line | 4.69 | 9.07 |
| `opt-2` on `surface` | line | 5.12 | 8.09 |
| `opt-3` on `paper` | line | 4.06 | 10.50 |
| `opt-3` on `surface` | line | 4.43 | 9.36 |
| `opt-4` on `paper` | line | 4.04 | 9.84 |
| `opt-4` on `surface` | line | 4.41 | 8.78 |
| `opt-5` on `paper` | line | 4.88 | 9.21 |
| `opt-5` on `surface` | line | 5.33 | 8.22 |
| `opt-6` on `paper` | line | 5.14 | 7.85 |
| `opt-6` on `surface` | line | 5.62 | 7.00 |
| `opt-1` on `sheet` | line | 5.41 | – |
| `opt-2` on `sheet` | line | 5.72 | – |
| `opt-3` on `sheet` | line | 4.95 | – |
| `opt-4` on `sheet` | line | 4.92 | – |
| `opt-5` on `sheet` | line | 5.95 | – |
| `opt-6` on `sheet` | line | 6.27 | – |

## 4 Typography

All fonts are SIL OFL 1.1 and bundled as woff2 (subset to Latin) in `app/src/fonts/` and
`website/fonts/`, with their licences. No font loads from a third party.

| Role | Face | Size and weight |
|---|---|---|
| Interface | Public Sans | 13 px, 400; labels 12 px; buttons 12 to 14 px, 500 to 600 |
| Numbers | Public Sans, `font-variant-numeric: tabular-nums` | Right-aligned in tables |
| Section labels | Public Sans capitals, `--track-caps` (0.12 em) | 11 to 12 px, 600 |
| Headings, notes | Libre Caslon Text; italic for notes and captions | Website 30 to 52 px; app About only |
| Code, CLI | IBM Plex Mono | 13 to 14 px |
| Display lettering | Drawn single-stroke capitals, as SVG (logo, drawing titles, illustrations) | Never set as text |

## 5 Space, lines, corners

- A 4 px step: `--sp-1` 4 px to `--sp-7` 48 px.
- Line weights 1 : 2 : 4 (`--lw-thin`, `--lw-thick`, `--lw-design`). **The design line is always the
  heaviest line on the screen.**
- Corners are small: `--radius` 2 px, `--radius-lg` 4 px. Panels have square corners and ink rules, like
  the plate's neatlines.

## 6 Icons and the mark

- The mark: two bends of a meandering river and one bold design line through them, in one ink. Use the
  SVGs in `website/assets/logo/`; never redraw or recolour them. Three drawings by size: 64 px up, 24 to
  63 px (`-small`), under 24 px (`-tiny`).
- The app icon: the mark on a cream tile inside a double neatline, with one hatched vermilion stage behind
  it (128 px and up). Files in `app/src-tauri/icons/`.
- Interface icons: text first. Where a symbol helps, use ✓, !, ✕ and arrows in the interface font. No
  icon font.

## 7 Components

- **Header:** paper ground, an ink rule below. The mark at 26 px, then the file buttons. The file name,
  study title and engine version at the right in `--ink-3`.
- **Buttons:** surface ground, `--rule-strong` border, ink text. One primary button per view, ink-filled.
  Disabled: `--ink-3` text, `--rule` border, no fill. Focus: a 2 px ink ring, 2 px offset.
- **Section labels:** capitals, `--track-caps`, with an ink rule under them.
- **Stage list:** name, a chip (`✓ fresh` pass colours, `! stale` warn colours, `not run` on
  `--surface-2`), and a Run button.
- **Inputs:** paper ground, `--rule-strong` border. A source note under the input when a value comes from
  the rule set (`--src-ruleset`) or a default (`--src-default`).
- **Banners:** warn banners in `--warn-soft` with `--warn` text and a leading "!"; errors in
  `--fail-soft` with "✕".
- **Option table:** a hatched swatch per option, the name, tabular numbers, and a status chip. The
  selected row has `--accent-soft` ground and a 3 px ink rule at its left.
- **Checks:** a status chip, the value against its limit, and the source on a second line in `--ink-3`.
- **Map legend:** a surface box with an ink border at the top right: "OPTIONS", then each swatch and name.
- **Map markers:** a tight bend is a cream disc with an ink rim and a ✕ in `--fail`, with a cream label
  ("✕ R 610 m < R min 800 m"). The station is plate ink in a cream ring.
- **Plots:** ink lines on surface; design low water dashed; the option's channel in its own colour,
  dashed; dredge hatched in the option's colour.
- **Status line:** an ink bar at the foot: "Screening. Not for construction." at the left.

## 8 App icon, splash and About

- **Splash:** the paper inside a double neatline; the stacked lockup; the promise in Caslon italic ("Explore
  the navigation possibilities of a river."); the loading state; "Screening tool · Not for construction"
  at the lower left; a title block at the lower right with the version, the engine version, Apache 2.0 and
  "Hosted by African River Corridors".
- **About:** the same title block, the font credits and the licences.

## 9 Light and dark

- Light: cream paper, plate ink. Dark: "dusk", a warm near-black, with cream ink.
- The app follows the OS setting (`prefers-color-scheme`) unless the user chooses; the choice sets
  `data-theme` on the root. Both dark blocks in `tokens.css` hold the same values; the check enforces it.
- Every component has both themes from its first ticket. No light-only colours.

## 10 Motion

- `--dur-1` 120 ms for hovers and presses; `--dur-2` 200 ms for panels; `--dur-3` 320 ms for the map.
  Easing `--ease`.
- Motion only where it explains: a selected option widening, a panel opening. Respect
  `prefers-reduced-motion`.

## 11 Voice

Simplified Technical English. Name the thing, say the fact, cite the source. Short sentences, active voice.

| Write | Do not write |
|---|---|
| Screening tool. Not for construction. | Design-ready channels in minutes. |
| Option 3 cuts the bank for 240 m. | Option 3 may require some minor earthworks. |
| R 610 m is less than R min 800 m (rule set, clause). | Bend too tight! |
| The terrain covers 82 % of this option. 18 % has no data. | Terrain loaded. |
| Fails: depth 2.6 m is less than the draught plus clearance, 3.2 m. | Something went wrong. |
| Runs offline. Your data stays on your computer. | Secure cloud-powered analysis. |
| Three design lines, screened. | The optimal channel. |
| Option, design line, channel limit, bend widening, design low water. | Route, path, corridor, lane (mixed for one thing). |
| Hosted by African River Corridors. | An ARC product. |

Never claim approval, certification, or a feature that is not built. No robots, sparkles, chat bubbles or
fake ageing.

## 12 Do not

- Do not colour the mark, or any chrome, with a stage colour.
- Do not use a stage colour to mean pass or fail.
- Do not tint, recolour or invert a drawing sheet.
- Do not load fonts or scripts from a third party.
- Do not add a brand accent colour. The accent is ink.
