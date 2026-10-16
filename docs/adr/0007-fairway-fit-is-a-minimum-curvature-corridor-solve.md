---
status: accepted
---
# The fairway fit is a minimum-curvature corridor solve

The fairway fit places a design line inside navigable water. Spec 0005 built it as an elastic band:
bending-energy steps, points projected back into the water, curvature clamped by projection, and
fillet joins with a join guard. Measured on real multi-leg routes (#126), the band follows the middle
of the water and does not swing wide in bends, makes narrow bends sharper than its input, and leaves
kinks where fixed pieces start. The causes are in the method: a nearest-point snap that copies the
edge's shape, a smoothing that reaches a few hundred metres at most, splices that hold position but
not curvature, and points that cannot be projected staying put while their neighbours move.

We replace the band with the racing line inside known bounds (Heilmeier et al., Vehicle System
Dynamics 58(10), 2020, doi:10.1080/00423114.2019.1631455). The line is a reference line moved
sideways along its normals by offsets `a`. Ray casts to the edge of navigable water give a box on
each offset, the narrowest within half a chord either side. A convex QP minimises the summed squared
second derivative of the line inside the boxes, rows tighter than R_min weighing more, and the
solution, resampled, is the next reference (re-linearised up to 16 times). The QP is solved by
Clarabel (Apache-2.0, pure Rust); an own bounded least squares can replace it behind the same
function. Fixed poses and must-follow segments are points with zero offset in the same corridor, so
there are no splices. A point with no navigable width is left free, so the line passes through with
continuous curvature, and the stretch is reported. Nothing is snapped to an edge. `join_legs` builds
one corridor per route, so fillets and the join guard are no longer needed for its joins.

This supersedes the elastic-band part of spec 0005 (T5, #96) and the fillet joins of T7 (#98). The
rest of spec 0005 stands: navigable water, Dubins end poses (now the seed's shape only), must-follow
segments, the one radius reading and reported infeasible stretches.

## Considered options

- **Keep the band and tune it.** Rejected: the faults are in the snap and the locality, not in the
  weights.
- **A medial axis as the reference line.** Out of scope for now: the smoothed seed, re-linearised,
  reaches the same corridor without a skeleton of the water.
- **An own bounded least squares from the start.** Kept as the fallback: Clarabel's box-constrained
  path is pure arithmetic and square roots (no platform transcendentals), solves a 2,000-point
  corridor in about 12 ms, and adds no C dependency.
- **TUM's `trajectory_planning_helpers` (LGPL-3.0).** Read for the method only, never vendored.
