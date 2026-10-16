# Spec 0005: Water-aware alignment — the fairway fit, joined legs and one radius reading

Status: **approved by the maintainer, 2026-10-16.** Decisions: `docs/design/2026-10-16-water-aware-alignment-grill.md`
(Q1–Q14, L1–L8, R2-Q1–R2-Q4) and `GLOSSARY.md`. Builds on spec 0001 (#20, the Rust core) and spec
0002 (#58, the channel design document and the alignment stage). **It changes engineering results**, so
every result-changing ticket waits for the port to close (#37) and for the design-impact check (#36).

**Amended 2026-10-16 (#126, ADR 0007):** the fairway fit is a minimum-curvature corridor solve, not
an elastic band, and `join_legs` builds one corridor per route instead of fillets and the join guard.
The sections below are updated to match; the research line records what was added.

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
2. **The fairway fit.** A minimum-curvature corridor solve (ADR 0007): the line inside navigable
   water, between fixed poses and through must-follow segments, that minimises its summed squared
   curvature, pushed away from bends tighter than R_min. It is free to be longer than the input. It
   reports every stretch where no compliant line fits, and passes through it with continuous
   curvature.
3. **A water-aware rolling circle.** When a water polygon is given, each bend takes the first rung
   that works:
   1. **inside arc** at R_min, as today, if the bend-widened channel fits in the water;
   2. **push out**: a wider arc, placed further out, at the smallest radius ≥ R_min at which the
      widened channel fits;
   3. **fairway fit** over a window around the bend;
   4. **infeasible**: the line that breaks the rules least, and a failed check for that stretch.

   Without a water polygon, the rolling circle gives exactly today's results.
4. **Joined legs and fixed pieces.** `join_legs` joins ordered seed legs with one corridor per route:
   the legs, end to end, seed one fairway fit over the whole route. A join is a bend inside the
   corridor, with no fillet and no splice. A fixed start pose, end pose and must-follow segments are
   held exactly. The join guard stays for lines changed after a fit (`guard_joins`).
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
- **Open 90° basin:** two straight legs meet at 90° in open water. `join_legs` returns one smooth line
  through the join, with no curvature step, and its tightest bend is at least the largest fillet the
  basin allows (639.3 m), so at least R_min.
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
- **`alignment::fairway_fit` — the fairway fit (new; ADR 0007).** A minimum-curvature corridor
  solve:
  - the line is a reference line moved along its normals by offsets `a`; the first reference is the
    seed, smoothed, and each pass's solution, resampled every 10 m, is the next (up to 16 passes);
  - ray casts along each normal to the edge of navigable water give a box on each offset, the
    narrowest within half a chord either side (half an LOA at the default chord), 1 m inside the edge;
  - a convex QP (`fairway_fit::corridor`, Clarabel) minimises the summed squared second derivative
    inside the boxes. Rows tighter than R_min weigh up to 10⁴ times more (a weight that rises at once
    and falls slowly between passes), and the optional preferred radius adds a smooth ramp, never a
    step. The best pass is kept;
  - fixed poses hold their point, the next point along the heading and a ghost one spacing behind;
    must-follow segments are zero-offset points in the same corridor and are copied exactly. Dubins
    pieces (`dubins_paths`) of radius R_min shape the seed at posed ends (L1);
  - nothing is snapped: a point outside the water is pulled softly towards water within one chord,
    and a point with no navigable width is free, so the line passes with continuous curvature and the
    stretch is reported.
- **`alignment::join` — joined legs (new; ADR 0007).** `join_legs(legs, water, fixed, params) ->
  JoinResult`: one fairway fit over the whole route, seeded by the legs end to end. Each join is read
  by position (L7) on the final line: it holds R_min when the chord reading holds through its window
  and no infeasible stretch touches it. **Join guard (R2-Q2, L8):** `guard_joins` re-reads joins on a
  line changed after a fit, refits a failing window with more weight, and names a join that still
  fails. The same machinery serves tangent joins between optimised pools (#6).
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

- `dubins_paths` (MIT, 3.x) in `cd-core`. The A* stays in the crate (Q8).
- `clarabel` (Apache-2.0, pure Rust, =0.11.1) in `cd-core`, for the corridor QP (ADR 0007).

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
  obstacle repulsion. The fairway fit's first smoother, replaced by ADR 0007. CHOMP and TEB are
  heavier alternatives.
- **Dubins** (Am. J. Math. 1957, doi:10.2307/2372560): shortest joins between two poses at bounded
  curvature. **Clothoid G1 fitting** (Bertolazzi and Frego, MMA 2014, doi:10.1002/mma.3114): curvature-
  continuous joins, if ever needed.
- **Ship routing:** Ari et al., EJOR 2013 (doi:10.1016/j.ejor.2013.03.022): A* on a lattice with a turn
  radius and a safety distance, then smoothing — the closest published match. Candeloro et al., CEP 2017
  (doi:10.1016/j.conengprac.2017.01.007): medial axis, then curvature-continuous smoothing.
- **Inland waterway guidance** (PIANC InCom WG 141, 2019) sets width, depth and bend radius by class,
  but gives no method for placing the line.
- **Minimum-curvature racing lines** (added with #126): Heilmeier et al., "Minimum curvature trajectory
  planning and control for an autonomous race car", Vehicle System Dynamics 58(10), 2020,
  doi:10.1080/00423114.2019.1631455. A reference line, normals, lateral offsets inside track bounds,
  summed squared curvature as a QP, re-linearised. TUM's `global_racetrajectory_optimization` and
  `trajectory_planning_helpers` (LGPL-3.0) implement it; read for the method, never vendored. The
  first research line missed this family; it is the fairway fit since ADR 0007.

**Crates checked:**

| Job | Crate | Licence / state | Verdict |
|---|---|---|---|
| A* | `pathfinding` 4.16 | MIT/Apache, active | Not needed: keep the in-crate A* (Q8) |
| Dubins end pieces | `dubins_paths` 3.2 | MIT, active | **Adopt** |
| Clothoids | `clothoid` 0.1 | EUPL-1.2 (copyleft) | **Avoid** |
| Polygon buffer | `geo` 0.33 `Buffer`, `i_overlay` 9.1 | MIT/Apache, active | Fallback only (Q7) |
| Arc-exact offsets | `cavalier_contours` 0.9 | MIT/Apache, active | Not needed now |
| Hybrid A*, elastic band | none usable | — | Write ourselves |
| Convex QP (corridor solve, #126) | `clarabel` 0.11 | Apache-2.0, pure Rust, active | **Adopt** (ADR 0007) |

**What we reuse:** Dubins pieces, the in-crate A*, the crate's geometry kernel, and the radius check.
**What we build:** navigable water, the corridor solve around Clarabel (ADR 0007; it replaced the
bending-energy band and the fillet joins), the join guard and the rolling-circle ladder. A
project-side prototype tested the band's design on real multi-leg lines (and later measured its
faults, #126); its lessons are L1–L8 in the grill note.

## Tickets

Each ticket is a sub-issue of the spec issue (#91). "Blocked by" uses GitHub's native dependencies.

| # | Ticket | Blocked by | Changes results |
|---|---|---|---|
| T1 (#92) | Polygon buffer in the geometry kernel: shrink, grow, open and close | — | No |
| T2 (#93) | `polygon_from_bank_pair` orients the second bank and refuses crossing banks | — | Only for reversed input |
| T3 (#95) | One radius reading: every reported radius on the chord, lattice and pools included | #36, #37 | Yes (re-route `radius_ok`) |
| T4 (#94) | Navigable water, the per-station width check, ruleset bend-width validation, and the four synthetic fixtures | T1, #37 | No (new functions) |
| T5 (#96) | The fairway fit: coarse-to-fine bending-energy band, capped projection, Dubins end pieces, must-follow segments, infeasible stretches | T3, T4 | Yes (new method) |
| T6 (#97) | Water-aware rolling circle: the four-rung ladder (closes #5) | T5 (#96) | Yes, with water only |
| T7 (#98) | `join_legs` with the join guard (closes #6) | T5 (#96) | Yes (new function) |
| T8 (#99) | Fairness weight on the least-cost re-route, default 0 | #36, #37 | No at default |
| #126 | The fairway fit as a minimum-curvature corridor solve; one corridor per route in `join_legs` (ADR 0007) | T5, T7 | Yes (fairway fit, rung 3, `join_legs`) |
| T9 (#100) | `.cdesign` and CLI: `seed_legs`, `fixed`, the new settings, `fits_in_water` and `join_guard` checks in every export | T6, T7 | Yes (schema version) |

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
