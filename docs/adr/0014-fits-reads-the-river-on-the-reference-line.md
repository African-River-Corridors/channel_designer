---
status: accepted
---
# `fits` reads the river on the reference line

ADR 0013 made each class's fairway fit the default plan line of `owd fits`. That line belongs to one
class: for a class too big for the river, its water shrinks to nothing, and the line's shape comes from
where it does not fit. It is also made only for the full answer. Spec 0016 (approved by the maintainer on
2026-10-22) asks for one smooth line that belongs to no vessel, on which every vessel is judged.

## Decision

- **The default plan line is the reference line** (spec 0016 T1). It is the smoothed mid-line of the
  banks, refitted in the water: held off the banks by the larger of 5 m and 10 % of the local width, with
  no minimum radius, and pulled towards the mid-line by one weight. The weight (0.3) was tuned on two
  windows of a long river and is fixed in code with its label
  (`owd_core::alignment::reference_line::SEED_PULL_WEIGHT`).
- **Width, radius, depth and the reaches are all read along it,** in the plan-only answer and the full
  one. The full answer walks the reference line itself, so the width and depth stations lie on it. A
  reach is found where the line lies on the river centreline.
- **The study stores it** as `inputs.reference_line`, with its settings, the banks' low-confidence
  stretches carried onto it, and a hash of what it was made from (the banks, the river polygon, the
  project CRS and the settings). A change to any of these makes it stale; the next `owd fits` makes it
  again. No stage reads it, so storing it leaves the stages fresh.
- **Without both bank lines, the centreline is read,** and `plan_line_note` says why.
- `--plan-line fairway` and `--plan-line centreline` still run as ADR 0013 says.

This supersedes the default in ADR 0013. The rest of ADR 0013 (which line the answer says it read, and
the fall-backs of the fairway) still holds.

## Considered options

- **Keep the fairway as the default.** Rejected: one line per class is not a fair line on which to judge
  a smaller vessel, and the plan-only answer cannot use it.
- **The smoothed mid-line alone, with no refit.** Rejected: it keeps the banks' noise and can leave the
  water where the banks are poor.
- **A refit with no pull towards the mid-line.** Rejected: the smoothest line hugs the inside of each bend.
