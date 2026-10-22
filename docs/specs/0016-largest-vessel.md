# Spec 0016: The largest vessel the river allows

Status: **approved by the maintainer, 2026-10-22,** issue #387, from an interview (notes:
`docs/design/2026-10-22-largest-vessel-grill.md`). Builds on spec 0005 (water-aware alignment), spec 0007
(rules), spec 0013 (banks), spec 0014 (reports) and spec 0015 (custom vessel). Planned for 0.4.

## Problem statement

`owd fits` answers "which catalogue classes fit this river". It reads the radius on a plan line, and both
plan lines it has today mislead on a narrow, winding river.

- **The bank centreline is jumpy.** Each station is set on its own, between its rays to the two banks, and
  the line is not smoothed after the last pass. It copies bank roughness, creek mouths and islands, so it
  reports far more radius stretches than a vessel would meet. On a 144 km test river it gave about three
  times the fairway's count.
- **The fairway belongs to one class.** Each class gets its own fit in the water less half its bottom
  width. For a class too big for the river, that water shrinks to nothing, and the line's shape comes from
  where it does not fit. It is not a fair line on which to judge a smaller vessel.
- **Classes are the only answers.** A designer wants to know the largest vessel the river allows, not only
  which of a list of classes pass.

## Solution

1. **A reference line.** One smooth, water-aware line that belongs to no vessel. It starts from a smoothed
   mid-line of the banks and is refitted in the water, held back from the banks and pulled towards the
   mid-line. It becomes the plan line of `owd fits`.
2. **A size sweep.** `owd envelope` tests a fixed series of single-hull vessels, 10 × 2 m up to 50 × 10 m,
   each with its own fairway fit seeded from the reference line, under PIANC WG 141 quality C by default.
   It gives the largest size that fits the whole reach, and the largest that fits each kilometre.
3. **In the outputs.** The result is in the CLI (JSON, CSV, a PNG chart), a "Largest vessel" section of the
   screening report, and a "Largest vessel" panel in the app.

## User stories

1. As a river engineer, I want a plan line that is smooth and stays in the water, so that radius limits come
   from the river's real bends, not from bank noise.
2. As a river engineer, I want that line to belong to no vessel, so that one line serves every vessel I
   test.
3. As a decision-maker, I want the largest vessel the river allows, so that I can size the fleet to the
   river and not only pick from a list.
4. As a river engineer, I want the largest vessel for each kilometre, so that I can see which bends hold
   the river back and where works would pay.
5. As a river engineer, I want the catalogue classes and my custom vessel shown against the result, so
   that I can compare them with the limit.
6. As a river engineer, I want the stretches where the banks have low confidence marked on the reference
   line, so that I do not trust a limit set by bad banks.

## Implementation decisions

### Ticket 1: the reference line

Built in `owd-core`, beside the mid-line and the fairway fit.

1. **The seed: a smoothed mid-line.** From the two bank lines:
   - Resample both banks every 20 m.
   - Pair each point on one bank with a point on the other, moving forward only, at most 5 steps ahead.
   - Take the mid-points from both directions, and blend them by share of length.
   - Smooth with two passes of Chaikin corner cutting, then Taubin smoothing (λ 0.5, μ −0.53, 60
     iterations, ends pinned). Taubin does not shrink the line, where a moving average does.
2. **The refit in the water.** Run the existing fairway fit with:
   - the navigable water = the river polygon less a **bank clearance** of the larger of 5 m and 10 % of the
     local width;
   - **no minimum radius**: the line only has to be smooth;
   - a **pull towards the seed**, one weight, so the line stays central and does not hug the inside of
     each bend. The weight is tuned on two test windows and fixed in code with a label.
3. **Low confidence.** Stretches where the banks are flagged low confidence (spec 0013) are carried onto
   the reference line by chainage.
4. **Stored** in the study as `inputs.reference_line`, with its parameters and a hash of the banks, and
   marked stale when the banks change.
5. **The plan line.** `owd fits --plan-line` gains `reference`, and it becomes the default. `centreline` and
   `fairway` stay as options. Width, depth and reaches are read on the plan line too, not only the radius.

### Ticket 2: `owd envelope`

- **The series.** By default, five single-hull vessels with a length-to-beam ratio of 5:

  | Length × beam (m) | 10 × 2 | 20 × 4 | 30 × 6 | 40 × 8 | 50 × 10 |
  |---|---|---|---|---|---|

  `--sizes 10x2,20x4,…` sets another series. Each size is a motor vessel.
- **The draught** is 2 m for every size; `--draught` changes it. The sweep is plan only: width and radius.
  The draught sets PIANC's width allowances, and the draught limit from the bed model, if there is one,
  is reported beside the result.
- **The rules.** PIANC WG 141 quality C by default; `--rule-set` changes it. The strictest selected Rule
  governs, as everywhere in OWD.
- **For each size:** its own fairway fit, seeded from the reference line, in the water less half its bottom
  width, with its own R_min. Width and radius are read on that fitted line.
- **The answers.**
  - The largest size with no blocked stretch in the whole reach.
  - For each kilometre: the largest size that passes that kilometre.
  - For each size: its blocked stretches (kind, chainage, deficit).
  - The catalogue's 12 default classes and the study's custom vessel, marked against the series by length
    and beam (not refitted).
- **Outputs.** `--json`, `--csv`, and `--png FILE`: a chart of the series (pass or fail along the river),
  and the per-kilometre strip. Every output carries "Screening — not for construction", the rule set, the
  draught and the bank caveat.
- **Stored** in the study as `stages.envelope`, stale when the banks, the reference line or the settings
  change.

### Ticket 3: the report

The screening report (spec 0014) gains a "Largest vessel" section: the series table, the chart, the
per-kilometre strip, and the bank caveat in Data and limits.

### Ticket 4: the app

A "Largest vessel" panel: the series table and chart, and the per-kilometre strip drawn on the map, coloured
by the largest size that passes. It runs through the shell, so the desktop and the web app share one path.

## Testing decisions

- **A good test goes through a public seam:** `owd fits … --json`, `owd envelope … --json`, the core API,
  the report, or the app.
- **Reference line.**
  - It lies inside the water less the clearance everywhere (property).
  - Its curvature is smoother than the mid-line's (property: fewer sign changes per km, lower peak).
  - On a straight synthetic channel it is the centreline (property).
  - On a synthetic bend it stays nearer the centre than a 10 × 2 m fairway fit does.
  - It is the same for any vessel (it takes no vessel input).
- **Envelope.**
  - On a synthetic channel built to pass 30 × 6 m and fail 40 × 8 m, the answer is 30 × 6 m.
  - A kilometre with a tight bend gets a smaller limit than its neighbours.
  - Raising the clearance or the rule quality never raises the answer (property).
- **Fits.** The default plan line is `reference`; `centreline` and `fairway` still run; a design-impact
  fragment records the moved counts on both CI examples.
- **Acceptance.** The Sassandra study and one narrow-river window: the series, the chart and the strip are
  attached to the last PR.

## Out of scope

- Convoys, and any formation other than a single hull.
- Sweeping the draught.
- Refitting each catalogue class in the envelope (they are marked, not refitted).
- Works: the envelope answers "as the river is".
- A full length × beam grid.

## Tickets

Built in order, one PR each, test first. Each is a sub-issue of the spec issue, with native "blocked by"
links.

1. **The reference line (#388)** in the core, stored in the study, and the default plan line of `owd fits`.
2. **`owd envelope` (#389).** Blocked by 1.
3. **The report's "Largest vessel" section (#390).** Blocked by 2.
4. **App: the "Largest vessel" panel (#391).** Blocked by 2. Takes the app slot.

## Risks

- **Bad banks set the limit.** A narrow, flagged stretch can cap the whole river. The per-kilometre strip
  and the low-confidence marks show where; the bank fixes in spec 0013 T8 reduce it.
- **Run time.** Five fits on a long river take minutes, not seconds. The study caches each size.
- **The pull weight.** Too strong, and the line keeps the mid-line's noise; too weak, and it hugs the
  inside of bends. Ticket 1 tunes it on two windows and records the value with a label.
