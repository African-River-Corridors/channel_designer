# Spec 0009: Restyle the desktop app onto the OWD brand tokens

Status: **approved by the maintainer, 2026-10-21** (the brand, and the restyle mock). Spec 0008 landed the
brand files and left the restyle for this spec. The approved mock of today's layout is in the brand pack
(`app-mock/app.html`, `renders/app-mock-light.png` and `-dark.png`). Style guide: `docs/design/style-guide.md`.

## Problem statement

The app still uses ARC's navy, gold and cream, Georgia, the system font, neon option colours
(`#ffd400`, `#00c2ff` ...) and hard-coded hex in every component. The brand tokens exist
(`app/src/tokens.css`) but nothing imports them. The app does not look like OWD, and it has no dark theme.

## Solution

Move every component onto the tokens. **Tokens only, no behaviour change.**

- Chrome is one ink on cream paper. Colour belongs to the river: the six option colours, and status.
- Option lines carry a colour, a wash and a hatch angle, so colour is never the only signal.
- Status (pass, warn, fail, stale) uses the status tokens and always carries a word and a shape.
- The white drawing sheet (`--sheet`) is for exported drawings. The app's plots sit on `--surface`.
- Light and dark both work, and every ticket shows both.

The work is cut by component group. Each ticket changes only the files of its group, so tickets do not
collide. `app/src/tokens.css` changes only if a token is wrong; then `website/tokens.css` changes in the
same commit.

## Rules for every ticket

- No behaviour change. No new feature, no moved element that a test or a user relies on. Every
  `data-testid`, label and text that `app/e2e/smoke.test.mjs` reads stays.
- Use tokens (`var(--...)`) for every colour, size step, space step, line weight and font in the files you
  touch. No hex, `rgb()` or named colour left in them, except where a map or canvas needs a resolved
  value (then read the token with `lib/theme.ts`).
- Until the last ticket, the app is pinned to the light theme. Each ticket still checks its view in dark:
  see "Screenshots".
- Text pairs meet WCAG AA. `cd app && npm run contrast` stays green. A new pair goes into
  `scripts/contrast.mjs`, not around it.
- A ticket's PR body holds a light and a dark screenshot of the changed view. Look at them before you push.
- Run `cd app && npm run build` (it runs the contrast check, `svelte-check` and `vite build`).

## Screenshots

Foundation adds a dev-only render path: `app/mock.html` serves the real app with a fixed fake session
(no Rust), so any view renders in a browser. Open `http://localhost:1420/mock.html?theme=light` or
`?theme=dark` (`?theme=` sets `data-theme`). The file is not part of the production build.
`node app/scripts/shot.mjs <name>` takes both screenshots in headless Chrome (needs `npm run dev` running).
Ticket PRs commit their screenshots under `docs/design/app-restyle/` and link them by commit; the last
ticket removes the folder.

## Tickets

Each ticket waits for the one before it.

1. **Foundation.** Import `tokens.css` and the fonts. Base type, ground, focus ring and form controls in
   `app.css`. Remove the old colour variables. `lib/theme.ts` (read a token, option colour helpers).
   Pin the light theme. The mock render path.
2. **Header, banners, footer, shell.** Toolbar buttons (primary: Run stale), the mark, file and engine
   labels, error and warning banners, panel dividers and drag handles, footer.
3. **Stages.** Stage rows, status chips (word and shape), the method select, run and cancel buttons,
   progress line.
4. **Data panel.** Data table, drop target, the "moved" note, water level inputs, hint text.
5. **Rule set.** The vessel and rule set form: selects, inputs, source tags, channel shape table and its
   warnings. If the spec 0007 Rule Set picker has merged, restyle it too, with its behaviour untouched.
6. **Map.** Map ground, option lines on imagery (deep line over a cream casing), hillshade note, legend
   with hatch swatches, tight bend and station markers, chainage labels, attribution, offline note.
7. **Long and cross sections.** Plot panes, axes, grid, series colours, station line, facts list, empty
   notes. On screen the plots sit on `--surface` with ink lines, as the mock shows.
8. **Option table and checks.** Table header, selected row, failed rows, option swatches with hatch,
   status marks, the checks list and its citations.
9. **Dark theme switch.** Follow the OS theme by default and add a header Theme control (Auto, Light, Dark)
   that remembers the choice. Redraw map and plots when the theme changes. Remove the light pin. Screenshot
   pass over every view in both themes. Contrast green.

## Acceptance

- Every ticket merged; `grep -rn "#[0-9a-fA-F]\{3,6\}" app/src --include=*.svelte --include=app.css` finds
  nothing outside the places the spec allows (theme code, map resolved values).
- `cd app && npm run build` passes. `app/e2e/smoke.test.mjs` passes unchanged.
- The app in OS light and OS dark matches the approved mock for the same layout.
- `app/src/tokens.css` and `website/tokens.css` are still byte-identical.

## Out of scope

- Any behaviour, layout or wording change; the in-app explain mode; the website.
- Hatch fills on the map polygons (a later feature). Swatches and legends carry the hatch.
