# Water-aware alignment: design decisions (grill, 2026-10-16)

The record of a design interview between Niall O Tuathail and the Systems Architect agent, for spec
0005. The terms are in `GLOSSARY.md`. This note keeps every answer, so the spec can be checked against
it.

## Why

A review of a multi-leg design line found sharp corners where separate lines were joined end to end,
far from any bank. Two causes sit in this repo:

- The rolling circle only cuts the inside of a bend. It cannot see the water, so it can neither swing
  wide into open water nor tell when its arc leaves the water.
- The least-cost re-route minimises cut. In open water the cut is zero everywhere, so it has no reason
  to prefer a smooth line.

A project-side prototype (a Python script, about 330 lines) then fitted smooth lines over three real
multi-leg routes. Its lessons are folded into the answers below (L1–L8).

## Round 1

| # | Question | Decision |
|---|---|---|
| Q1 | Name | **Fairway fit** names the step that fits a design line inside the water. **Navigable water** is a new glossary term. "Fairway" stays banned as a name for a line. |
| Q2 | Where the fit sits | Inside the rolling circle, not a fourth method. See R2-Q1. |
| Q3 | Smoothness cost | Bending energy (∫κ² ds, discrete fourth differences) plus a weak length term, plus an optional one-sided preferred-radius penalty max(0, κ − 1/R_pref)², off by default. R_min is a hard limit. No second-difference (Laplacian) step (L6). No curvature-rate term for now. |
| Q4 | Clearance | A clearance in metres, default 0. Navigable water = water shrunk by half the straight bottom width plus the clearance. The ruleset must give the bend-widened width, and each station is checked at its measured local radius. See R2-Q3. |
| Q5 | Infeasible stretches | Return the option with a failed check that lists each stretch by position and chainage, with a reason (*too narrow*, *R_min not possible*). Inside the stretch, the line is the one that breaks the rules least. Never return the input line, or cross land, without saying so. |
| Q6 | Test oracle after the port | Property checks on every result; known-answer cases; golden cases regenerated only with a recorded reason; the design-impact table (#36) on every PR. All four. |
| Q7 | Shrinking a polygon | Extend the crate's own geometry kernel. Fall back to `i_overlay` only if that proves hard. |
| Q8 | Crates | Keep the in-crate A*. Add `dubins_paths` (MIT). No `pathfinding`. |
| Q9 | Re-route fairness | Add a fairness weight to the least-cost re-route, default 0, so results don't change unless a user sets it. Drop the "corridor follows the water" part; the fairway fit covers open water. |
| Q10 | Water-aware rolling circle | Water polygon is an optional input; without it, today's behaviour exactly. Merged into R2-Q1. |
| Q11 | One radius reading | Every reported radius is read on the chord (`min_radius_on_chord`, default chord). The re-route keeps its vertex reading inside its own enforcement loop. The lattice and pool results gain the three radius fields. Drawing notes (spec 0004) read the same rule. |
| Q12 | Bank pair orientation | Orient the second bank to match the first, and record it. Refuse a pair whose banks cross. |
| Q13 | Blocking | The kernel polygon offset (Q7) and the bank-pair fix (Q12) start now. Everything else waits for #37; the first result-changing ticket also waits for #36. |
| Q14 | Protect good joins | Superseded by R2-Q2. |

## Lessons from the prototype (requirements, not questions)

| # | Lesson |
|---|---|
| L1 | A second-difference band cannot meet a fixed end tangentially. Use bending energy, and Dubins pieces at fixed poses. |
| L2 | One resolution tops out at about R 100 m at 25 m spacing. Fit coarse to fine (about 200 → 100 → 50 → 25 m). |
| L3 | Projecting onto stepped water edges invents tight bends. Smooth the navigable water first (open, then close). |
| L4 | Unbounded projection makes the line wander between slivers of water (one line nearly doubled in length). Cap how far one projection moves a point. |
| L5 | Each join needs its own blend length, growing with its turn angle: about 2.5 km for a 90° merge of a 140 m vessel. |
| L6 | A second-difference step pulls a constrained curve towards the inside bank and tightens it. Bending energy only. |
| L7 | After any step that changes length, find joins and features by position, never by chainage. |
| L8 | Freezing join arcs inside the band leaves kinks at the arc ends. Guard compliant joins after the fit instead. |

## Round 2

| # | Question | Decision |
|---|---|---|
| R2-Q1 | Shape of the method | The rolling circle becomes water-aware when a water polygon is given. Per bend it takes the first rung that works: (1) inside arc at R_min, if the bend-widened channel fits in the water; (2) push out: a wider arc further out, at the smallest radius ≥ R_min at which the widened channel fits; (3) fairway fit over a window around the bend; (4) infeasible: least-violating line plus a failed check. Without water, today's behaviour. Closes #5. |
| R2-Q2 | Join guard | Guard, not freeze: a join that held R_min after its arc must hold it after the fit; else refit that window; else a failed check. |
| R2-Q3 | Bend widening | Ruleset `validate` refuses a ruleset with no bend-widened width. The fit tests width at the measured local radius. The shrink uses the straight width; the bend check runs per station after the fit. The default widening formula is out of scope (separate ruleset ticket). |
| R2-Q4 | Legs and fixed pieces | `.cdesign` inputs gain optional `seed_legs` (ordered lines, joined first) and `fixed` (start pose, end pose, must-follow segments). Water stays one polygon: the union of the legs' water. The join tool also serves #6 (tangent joins between pools), which the join ticket closes. |
