# Spec 0008: Land the OWD brand (tokens, assets, style guide)

Status: **approved by the maintainer, 2026-10-21** (the brand). Apply after the rename (spec 0006) has
merged. The brand decision sits in the maintainer's own records; this spec holds what the builder needs.

## Problem statement

The app and the docs still use ARC's house colours (navy, gold, cream) and neon option colours. OWD now
has its own brand. Before any restyle, the repo needs the brand's single source: the tokens, the fonts, the
logo and illustration assets, the app icons, and a style guide that later tickets follow.

## Solution

Add the brand files. Change nothing the user sees except the app icon.

| Path | What |
|---|---|
| `docs/design/style-guide.md` | The style guide |
| `app/src/tokens.css` | The tokens, light and dark, with `@font-face` for the bundled fonts |
| `app/src/fonts/` | Public Sans, Libre Caslon Text, IBM Plex Mono (woff2, OFL licences) |
| `app/scripts/contrast.mjs` | The contrast check (WCAG AA, stage distance, dark blocks agree, website copy identical) |
| `app/package.json` | `build` runs `node scripts/contrast.mjs` first; a `contrast` script runs it alone |
| `app/src-tauri/icons/` | New app icons (same file names as today) |
| `website/tokens.css`, `website/fonts/` | The same token file and fonts, for the website |
| `website/assets/logo/` | Mark, lockups and icons as SVG; `favicon.svg` |
| `website/assets/illustrations/` | Before and after (wide and stacked, cream and dusk), block diagram (bare and dusk) |
| `NOTICE` | Font credits, and a pointer to `TRADEMARKS.md` for the name and mark |

## Rules

- **Do not import `tokens.css` into `app.css` in this spec.** `tokens.css` sets `--ink` and `--cream`,
  which `app.css` also uses, and its dark block would turn today's text cream in OS dark mode. The restyle
  tickets wire it in, component by component.
- Assets are generated in the brand pack by script; copy them, do not edit them here.
- `website/tokens.css` must be byte-identical to `app/src/tokens.css`. The check enforces it.

## Acceptance

- `cd app && npm run contrast` exits 0. A deliberately bad token (say `--ink-3: #a09a8c`) makes it exit 1.
- `npm run build` runs the check before `svelte-check`.
- The app builds and opens with the new icon; nothing else looks different (screenshot before and after,
  both OS themes).
- CI is green: the release guard and the leftover-names check (`python tools/old_names.py`) pass.

## Out of scope

- The restyle itself (a later spec, cut into tickets: tokens only, no behaviour change, both themes shown).
- The website build, docs theme and deploy.
- The in-app explain mode.
