# Known issues

## Fixed: rolling-circle exits and entries left a short kink

**Module:** `owd.alignment.rolling_circle` (issues #5 and #283)

Up to 0.2.0 each spliced arc joined the old line where the circle *crossed* it, so each exit
had a short kink. From 0.3.0 the rolling circle uses tangent exits: circle, then straight, as
in a Dubins path (#5). The entries are tangent too (#283): each arc starts at a vertex of the
line, on a circle tangent to the segment into that vertex. Before, the circle was placed at the
resample point, tangent to the direction read on the resample, up to 0.9° off the line's own
heading. See the `CHANGELOG.md` entries under 0.3.0 (Unreleased until it ships) and the rules in
the module doc of `crates/owd-core/src/alignment/rolling_circle.rs`.

On the package's synthetic river (`synthetic.synthetic_river()`), two passes, minimum
three-point radius of the result resampled at three spacings. "Exits" is after #5 only;
"now" is after #283:

| R_min, rolling step | 10 m resample | 25 m resample | 50 m resample |
|---|---|---|---|
| 300 m, 10 m (was) | 111 m | 288 m | 297 m |
| 300 m, 10 m (exits) | 295 m | 300 m | 300 m |
| 300 m, 10 m (now) | 300 m | 300 m | 300 m |
| 300 m, 25 m (was) | 55 m | 165 m | 296 m |
| 300 m, 25 m (exits) | 280 m | 299 m | 300 m |
| 300 m, 25 m (now) | 300 m | 300 m | 300 m |
| 255 m, 10 m (was) | 227 m | 250 m | 254 m |
| 255 m, 10 m (exits) | 254 m | 254 m | 255 m |
| 255 m, 10 m (now) | 255 m | 255 m | 255 m |
| 255 m, 25 m (exits) | 227 m | 255 m | 255 m |
| 255 m, 25 m (now) | 255 m | 255 m | 255 m |

A circle whose straight cannot meet the old line (the line curves away first, or ends) keeps
the old crossing exit on pass 1; its record says `tangent_exit: False`. From pass 2 on, such a
circle is not spliced, so later passes never add a crossing exit.

## R_min is reported, not guaranteed

**Modules:** `owd.alignment.rolling_circle`, `owd.alignment.route`
(issues #9 and #10)

**What happens.** Both steps aim for R_min but can return a design line tighter than it:

- `rolling_circle` does not resolve zigzags in a jagged input line (a trace digitised at
  coarse vertices). More passes do not converge.
- `optimise_route` only shifts the baseline sideways. It cannot lift a baseline that is
  itself tighter than R_min, and its smoothing stops after a fixed number of passes.

**What the result says.** Each result carries `radius_ok`, `measured_min_radius_m` and
`radius_check_chord_m` (`alignment.radius_check`). `radius_ok` allows 2 % below R_min for
reading error.

Every method reads radius the same way (spec 0005 Q11, #95): a three-point radius, points half
a chord apart, sliding at a stride of 10 m or less, read from both ends (`min_radius_on_chord`).
The default chord is R_min / 3 (one LOA at R_min = 3 × LOA). This holds for `rolling_circle`,
`optimise_route`, the pool lattice (`LatticeResult`, `PoolsResult`), the fairway fit and
`join_legs`. Pass `check_chord_m` to read another chord.

**Closed: corners between stations (#95).** The route used to report the reading on its own
station vertices, which misses the corners between straight segments: in one synthetic case
(R_min 150 m, 50 m stations) a 23° corner read 149.8 m on the vertices and 124.6 m on a
resampled 100 m chord, yet reported `radius_ok`. The route now reports the chord reading, so
that line reports `radius_ok = false`. Its smoothing and its choice between corridors still use
the vertex reading internally.

The route's corridor sweep now prefers a corridor that holds R_min over one that saves more
but does not. Up to 0.1.0 the route relaxed its target to the baseline's own radius,
reported the result as if it passed, and picked on saving alone; it no longer does.

**What to do.** Check `radius_ok`. If it is False, smooth or resample a jagged input first,
or roll the baseline before the route search.
