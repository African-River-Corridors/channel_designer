# Spec 0005: Water-aware alignment — the fairway fit, joined legs and one radius reading

Status: **approved by the maintainer, 2026-10-16.** Decisions: `docs/design/2026-10-16-water-aware-alignment-grill.md`
(Q1–Q14, L1–L8, R2-Q1–R2-Q4) and `GLOSSARY.md`. Builds on spec 0001 (#20, the Rust core) and spec
0002 (#58, the channel design document and the alignment stage). **It changes engineering results**, so
every result-changing ticket waits for the port to close (#37) and for the design-impact check (#36).

## Problem statement

A design line must keep a vessel inside the water and never turn tighter than R_min. Today no method
in this repo can see the water:

- **The rolling circle** replaces a tight bend with an arc on the inside of the bend. It can only
  shorten the line. When the arc leaves the water, it cuts the bank and says nothing. When there is
  open water outside the bend, it cannot swing wide into it.
- **The least-cost re-route** minimises cut. In open water the cut is zero everywhere, so any line
  scores the same and nothing prefers a smooth one. Its corridor is a narrow sideways offset that folds
  at a corner.
- **Lines joined end to end** (a river leg, a crossing, a creek, a sea approach) meet at corners. No
  function smooths a join, so a design line can carry a 90° corner a kilometre from any bank.
- **Radius is read three ways:** on a resampled chord (rolling circle), on vertices (re-route), and
  not at all (pool lattice). The same line can pass on one reading and fail on another.
- **`polygon_from_bank_pair`** makes a bow-tie when the second bank runs the other way. The result is
  a wrong polygon, not an error.

## Solution

1. **Navigable water.** From the water polygon, build the area the design line may use: the water
   shrunk by half the straight bottom width plus a clearance (metres, default 0), then smoothed by
   opening and closing. A per-station check then tests the channel at its measured local radius,
   using the ruleset's bend-widened width.
2. **The fairway fit.** A smoothing step that fits a design line inside navigable water between two
   fixed poses, never tighter than R_min, minimising bending energy. It is free to be longer than the
   input. It reports every stretch where no compliant line fits.
3. **A water-aware rolling circle.** When a water polygon is given, each bend takes the first rung
   that works:
   1. **inside arc** at R_min, as today, if the bend-widened channel fits in the water;
   2. **push out**: a wider arc, placed further out, at the smallest radius ≥ R_min at which the
      widened channel fits;
   3. **fairway fit** over a window around the bend;
   4. **infeasible**: the line that breaks the rules least, and a failed check for that stretch.

   Without a water polygon, the rolling circle gives exactly today's results.
4. **Joined legs and fixed pieces.** `join_legs` joins ordered seed legs: a fillet of at least R_min
   where one fits inside the water, otherwise a fairway-fit window whose blend length grows with the
   turn angle. A fixed start pose, end pose and must-follow segments are held exactly. A join that
   held R_min must still hold it after any later fit (the join guard).
5. **One radius reading.** Every reported radius, on every method and result, is read on the chord
   (`min_radius_on_chord`, default chord R_min/3).
6. **A fairness weight on the least-cost re-route,** default 0.
7. **`polygon_from_bank_pair` orients the second bank itself,** and refuses banks that cross.

### Worked example (synthetic river)

The synthetic river (`cd_core::synthetic`) is a sine centreline 6,000 m long, wavelength 1,500 m,
amplitude 260 m, width 40 m. With PIANC-B and CEMT IV, R_min is 255 m and the straight bottom width is
30.4 m. At each bend apex the centreline radius is about λ²/(4π²A) ≈ 220 m, so every bend is tighter
than R_min.

- **Today:** the rolling circle cuts each bend on the inside. It reports `radius_ok`, but the arc
  leaves the 40 m water at every apex, and nothing says so.
- **With this spec, at 40 m width:** navigable water is under 10 m wide. Rungs 1 and 2 fail at every
  apex, rung 3 finds no line, and each apex is reported as an infeasible stretch, *too narrow*, with
  its chainage and position. The option still has a line, and its failed check travels into every
  export.
- **With this spec, at 300 m width:** navigable water is about 270 m wide, so the expected result is
  that rung 2 or rung 3 places a line inside the water at R ≥ 255 m at every bend, with no failed
  check. The golden case records which rung each bend took. If a bend comes out infeasible here, the
  builder stops and reports it rather than tuning the case.

New synthetic fixtures, for the known-answer tests:

- **Straight canal:** the fit returns the straight line.
- **Open 90° basin:** two straight legs meet at 90° in open water. `join_legs` returns one fillet. Its
  radius is the largest the basin allows, and it is at least R_min.
- **Narrowing reach:** a channel that narrows below the bottom width over a known stretch. That stretch,
  and only that stretch, is reported infeasible.
- **Fixed tail:** a seed line that must merge tangentially into a fixed segment. The merge is tangent
  (heading error under a set tolerance), and the fixed segment is unchanged.

## User stories

1. As an engineer, I want the design line to stay inside the water I gave, so that I never cost a
   line that runs over land without knowing it.
2. As an engineer, I want a bend that can't fit on the inside to swing wide into open water, so that
   the line uses the river, not the bank.
3. As an engineer, I want a stretch where no compliant line fits to be named, with its chainage,
   position and reason, so that I know where the river, not the method, is the limit.
4. As an engineer, I want an infeasible stretch to keep a line, flagged as failed, so that I can still
   cost the option and compare it.
5. As an engineer, I want legs from different sources joined with no corner, so that one design line
   runs from start to end.
6. As an engineer, I want to fix a start pose (a berth) and must-follow segments (a shared approach),
   so that the fit changes only the free parts.
7. As an engineer, I want a join that already meets R_min to stay compliant after any later fit, so
   that smoothing never makes a line worse.
8. As an engineer, I want an optional preferred radius, so that I can ask for gentler bends without
   making them a limit.
9. As an engineer, I want a clearance from the bank, so that I can keep the channel off the bank toe.
10. As an engineer, I want every radius on every option read the same way, so that a pass on one
    report is a pass on all.
11. As an engineer, I want a reversed bank pair to just work, so that one wrong line direction doesn't
    give me a wrong polygon.
12. As an engineer, I want the output to say "width only, no depth data" when the fit used no
    bathymetry, so that nobody reads it as a depth check.

## Implementation decisions

### Modules

- **`geom::offset` — polygon buffer.** `buffer_polygons` accepts any distance: negative to shrink,
  positive to grow, with round joins. Opening and closing are a shrink then a grow, and a grow then a
  shrink. Pure Rust, in the crate's own kernel (Q7). If it proves hard, `i_overlay` (MIT/Apache) is
  the fallback, behind the same function.
- **`alignment::navigable` — navigable water (new).** `navigable_water(water, half_width_m,
  clearance_m, smooth_m) -> Polygons`. `width_check(line, water, ruleset, r_min, chord) ->
  Vec<Stretch>`: at each station, the bend-widened bottom width at the measured local radius against
  the width of the water there. A `Stretch` holds start and end chainage, start and end position,
  and a reason (`too_narrow`, `r_min_not_possible`).
- **Rulesets.** `validate` refuses a ruleset that gives no bend-widened width (R2-Q3). The default
  widening formula is unchanged.
- **`alignment::fairway_fit` — the fairway fit (new).** A coarse-to-fine elastic band (point spacing
  about 200 → 100 → 50 → 25 m; L2):
  - each step moves points down the gradient of bending energy (discrete fourth differences), plus a
    weak length term and the optional preferred-radius penalty max(0, κ − 1/R_pref)² (Q3). There is
    no second-difference step (L6);
  - after each step, points are projected back inside navigable water, and no projection moves a
    point further than a set cap (L4);
  - curvature is clamped to 1/R_min by projection, never by a soft cost;
  - fixed poses are met by a Dubins piece (`dubins_paths`) of radius ≥ R_min, so the line meets them
    tangentially (L1). Must-follow segments are copied exactly and the band works only between them;
  - where the band can't hold the line inside navigable water at R_min, it keeps the
    least-violating line and returns the stretch.
- **`alignment::join` — joined legs (new).** `join_legs(legs, water, fixed, params) -> JoinResult`.
  At each join: the largest fillet radius, from a set ceiling down to R_min, whose arc stays in
  navigable water; if none fits, a fairway-fit window whose blend length grows with the turn angle
  (L5). Joins are found by position, never by chainage, after any step that changes length (L7).
  **Join guard (R2-Q2, L8):** after any later fit, each join that held R_min is re-read; if it fails,
  that window is refit with more weight; if it still fails, the result carries a failed check. The
  same function serves tangent joins between optimised pools (#6).
- **`alignment::rolling_circle` — water-aware.** A new optional `water` argument. Without it, the
  code path and results are today's. With it, the four-rung ladder in the Solution, bend by bend. The
  result gains `rungs` (which rung each bend took) and `infeasible: Vec<Stretch>`.
- **`alignment::route` — fairness weight.** `RouteParams.fairness_weight`, default 0. When above 0,
  the DP adds the weight × the squared change of heading between stations. With 0, results are
  today's.
- **`alignment::radius_check` — one reading (Q11).** Every result that reports a radius reports
  `measured_min_radius_m`, `radius_check_chord_m` and `radius_ok` from `min_radius_on_chord` at the
  default chord: rolling circle, re-route, fairway fit, `join_legs`, the pool lattice and the pool
  results. The re-route keeps `min_radius_on_vertices` inside its own enforcement loop, and its
  *reported* `radius_ok` becomes the chord reading. The known-issues entry "corners between
  stations" is closed by this.
- **`volumes::footprint::polygon_from_bank_pair`** orients the second bank like the first
  (`orient_like`), records `reoriented: true`, and refuses banks that cross (Q12).

### The channel design document and CLI (R2-Q4)

- Inputs gain optional **`seed_legs`** (ordered lines) and **`fixed`** (`start_pose`, `end_pose`:
  point and heading; `must_follow`: lines). The water stays one polygon, the union of the legs' water.
- The rolling-circle method settings gain `clearance_m` (default 0), `r_pref_m` (default none) and
  `use_water` (default true when a river polygon is present). The re-route settings gain
  `fairness_weight` (default 0).
- **Checks** on an option: `fits_in_water` (failed, with its stretches) and `join_guard` (failed,
  with the join positions). Both travel into `status`, CSV and GeoJSON as spec 0002 requires.
- Every export of a water-aware option carries "width only, no depth data".
- A schema change bumps the document's schema version. A document from before this spec opens and
  runs with the same results, because the new settings default to today's behaviour, except
  `use_water`, which a pre-0005 document records as false on open.

### Dependencies

- `dubins_paths` (MIT, 3.x) in `cd-core`. No other new crate. The A* stays in the crate (Q8).

## Testing decisions

After #37 there is no Python reference. The oracle for this spec is (Q6):

- **Property checks on every result** (property tests over random seeds of the synthetic fixtures):
  outside the reported infeasible stretches, the line lies inside navigable water; the measured
  radius on the chord is ≥ (1 − RADIUS_TOL) × R_min; fixed poses are met within a position and
  heading tolerance; must-follow segments are unchanged; no join that held R_min fails without a
  `join_guard` check.
- **Known-answer cases** on the four new fixtures (straight canal, open 90° basin, narrowing reach,
  fixed tail), with the answers derived by hand in the test.
- **Golden cases** on the synthetic river, at 40 m and 300 m width, regenerated only with a recorded
  reason.
- **The design-impact table (#36)** on every PR that changes a result. The PR for one radius reading
  lists every re-route case whose `radius_ok` changes.
- **No-water regression:** with no water polygon, the rolling circle and re-route give results equal
  to the last port release, on every existing case.
- **A check that can fail:** each property check is shown to fail once, on a corrupted input (a line
  pushed onto land, a corner left in), before it is trusted.

## Research line (2026-10-16, about 20 minutes, sources fetched that day)

**What already exists.** Curvature-limited paths among obstacles are a solved family:

- **Hybrid A\*** (Dolgov et al., IJRR 2010, doi:10.1177/0278364909359210): a lattice search with an
  analytic end connection, then gradient smoothing with obstacle, curvature and smoothness terms. This
  spec's shape.
- **State lattices** (Pivtoraiko, Knepper and Kelly, JFR 2009, doi:10.1002/rob.20285): motion
  primitives with a minimum turning radius. Our pool lattice is one.
- **Elastic bands** (Quinlan and Khatib, ICRA 1993, doi:10.1109/ROBOT.1993.291936): tension plus
  obstacle repulsion. The fairway fit's smoother. CHOMP and TEB are heavier alternatives.
- **Dubins** (Am. J. Math. 1957, doi:10.2307/2372560): shortest joins between two poses at bounded
  curvature. **Clothoid G1 fitting** (Bertolazzi and Frego, MMA 2014, doi:10.1002/mma.3114): curvature-
  continuous joins, if ever needed.
- **Ship routing:** Ari et al., EJOR 2013 (doi:10.1016/j.ejor.2013.03.022): A* on a lattice with a turn
  radius and a safety distance, then smoothing — the closest published match. Candeloro et al., CEP 2017
  (doi:10.1016/j.conengprac.2017.01.007): medial axis, then curvature-continuous smoothing.
- **Inland waterway guidance** (PIANC InCom WG 141, 2019) sets width, depth and bend radius by class,
  but gives no method for placing the line.

**Crates checked:**

| Job | Crate | Licence / state | Verdict |
|---|---|---|---|
| A* | `pathfinding` 4.16 | MIT/Apache, active | Not needed: keep the in-crate A* (Q8) |
| Dubins end pieces | `dubins_paths` 3.2 | MIT, active | **Adopt** |
| Clothoids | `clothoid` 0.1 | EUPL-1.2 (copyleft) | **Avoid** |
| Polygon buffer | `geo` 0.33 `Buffer`, `i_overlay` 9.1 | MIT/Apache, active | Fallback only (Q7) |
| Arc-exact offsets | `cavalier_contours` 0.9 | MIT/Apache, active | Not needed now |
| Hybrid A*, elastic band | none usable | — | Write ourselves |

**What we reuse:** Dubins pieces, the in-crate A*, the crate's geometry kernel, and the radius check.
**What we build:** navigable water, the bending-energy band, the fillet search, the join guard and the
rolling-circle ladder. A project-side prototype proved the band's design on real multi-leg lines; its
lessons are L1–L8 in the grill note.

## Tickets

Each ticket is a sub-issue of the spec issue. "Blocked by" uses GitHub's native dependencies.

| # | Ticket | Blocked by | Changes results |
|---|---|---|---|
| T1 | Polygon buffer in the geometry kernel: shrink, grow, open and close | — | No |
| T2 | `polygon_from_bank_pair` orients the second bank and refuses crossing banks | — | Only for reversed input |
| T3 | One radius reading: every reported radius on the chord, lattice and pools included | #36, #37 | Yes (re-route `radius_ok`) |
| T4 | Navigable water, the per-station width check, ruleset bend-width validation, and the four synthetic fixtures | T1, #37 | No (new functions) |
| T5 | The fairway fit: coarse-to-fine bending-energy band, capped projection, Dubins end pieces, must-follow segments, infeasible stretches | T3, T4 | Yes (new method) |
| T6 | Water-aware rolling circle: the four-rung ladder (closes #5) | T5 | Yes, with water only |
| T7 | `join_legs` with the join guard (closes #6) | T5 | Yes (new function) |
| T8 | Fairness weight on the least-cost re-route, default 0 | #36, #37 | No at default |
| T9 | `.cdesign` and CLI: `seed_legs`, `fixed`, the new settings, `fits_in_water` and `join_guard` checks in every export | T6, T7 | Yes (schema version) |

T1 and T2 start now (Q13). T2 lands before #37 only with the same fix mirrored in the frozen Python
and a parity case regenerated with the reason "bank pair orientation".

## Out of scope

- Drawings, DXF and their notes (spec 0004). Spec 0004 reads the radius rule from T3.
- Changing the default bend-widening formula (a ruleset ticket of its own).
- Depth: bathymetry, the bed, a bar at a river mouth. The fit tests width only.
- Curvature-rate (clothoid) transitions. Hand-editing a design line.
- Making the least-cost re-route's corridor follow the water (Q9).

## Resolved points (maintainer, 2026-10-16)

All grill questions are answered; see the grill note. The ticket breakdown above was approved with the spec.
