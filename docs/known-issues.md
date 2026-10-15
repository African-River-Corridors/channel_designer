# Known issues

## Rolling-circle exits leave a short kink

**Module:** `channel_designer.alignment.rolling_circle`

**What happens.** The rolling circle replaces a tight bend with an arc of radius R_min. The arc
joins the old line where the circle *crosses* it, not where it is tangent to it. So each arc
exit has a short kink. Over a vessel length the line still holds R_min. Over a short chord the
kink reads as a much tighter radius.

**Measured** on the package's synthetic river (`synthetic.synthetic_river()`), R_min = 300 m,
minimum three-point radius of the result resampled at three chord lengths:

| Rolling step | 10 m chord | 25 m chord | 50 m chord |
|---|---|---|---|
| 10 m | 111 m | 288 m | 297 m |
| 25 m | 55 m | 165 m | 296 m |

At R_min = 255 m and a 10 m step the same figures are 227 m, 250 m and 254 m.

**Effect.** Volumes that read curvature on a vessel-length chord (about 50 m), such as
`volumes.cut_model.widened_hb`, are barely affected. Anything that reads curvature on a short
chord is affected: bend widening on a 10 m resample, channel limits, drawings.

**What to do now.** Read radius on a chord of about one vessel length, or smooth the exits
before you use the line for curvature. Do not quote a minimum radius measured on a short chord.

**Status.** Kept as is for this release. A test (`test_property_rolling_circle_lifts_min_radius_to_target`)
pins the behaviour, so a fix shows up as a failing test. The fix is tangent exits; it changes
every rolled line, so it ships as its own change.

## R_min is reported, not guaranteed

**Modules:** `channel_designer.alignment.rolling_circle`, `channel_designer.alignment.route`
(issues #9 and #10)

**What happens.** Both steps aim for R_min but can return a design line tighter than it:

- `rolling_circle` does not resolve zigzags in a jagged input line (a trace digitised at
  coarse vertices). More passes do not converge.
- `optimise_route` only shifts the baseline sideways. It cannot lift a baseline that is
  itself tighter than R_min, and its smoothing stops after a fixed number of passes.

**What the result says.** Each result carries `radius_ok`, `measured_min_radius_m` and
`radius_check_chord_m`. The radius is read on a chord (`alignment.radius_check`):
three-point radius with points half a chord apart, sliding at a stride of 10 m or less.
`radius_ok` allows 2 % below R_min for reading error. Default chords:

| Step | Default chord | Why |
|---|---|---|
| `rolling_circle` | R_min / 3 | one LOA at R_min = 3 × LOA |
| `optimise_route` | larger of R_min / 3 and 2 × `station_step` | the line has a vertex per station; a polyline reads its own corners as tight radii on a shorter chord |

Pass `check_chord_m` (the vessel length) to read on one LOA. Up to 0.1.0 the route relaxed
its target to the baseline's own radius and reported the result as if it passed; it no longer
does.

**What to do.** Check `radius_ok`. If it is False, smooth or resample a jagged input first,
or roll the baseline before the route search.
