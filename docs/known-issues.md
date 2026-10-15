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
`radius_check_chord_m` (`alignment.radius_check`). `radius_ok` allows 2 % below R_min for
reading error.

| Step | How it reads | Default chord |
|---|---|---|
| `rolling_circle` | three-point radius, points half a chord apart, sliding at a stride of 10 m or less, read from both ends | R_min / 3 (one LOA at R_min = 3 × LOA) |
| `optimise_route` | three-point radius on the line's own station vertices, the same reading its smoothing enforces | 2 × `station_step` |

The route reads its vertices, not a resample. A resample reads the corners between straight
segments: a polyline with every vertex on an R circle reads 3-6 % below R at usual spacings.
Pass `check_chord_m` (the vessel length) to `rolling_circle` to read on one LOA. On the route,
`check_chord_m` sets how many stations each side are used (rounded half up), and the chord
reported is nominal (2 × k × `station_step` along the baseline; offsets stretch segments).

The route's reading is the one its smoothing enforces, so `radius_ok` there means the
stations hold R_min. It does not mean every LOA-length chord does: a resampled one-LOA check
can read well below it at a corner between stations. In one synthetic case (R_min 150 m,
50 m stations) a 23° corner read 149.8 m on the vertices and 124.6 m on a resampled 100 m
chord. If a corner matters, use a smaller `station_step` or smooth the line before use.

The route's corridor sweep now prefers a corridor that holds R_min over one that saves more
but does not. Up to 0.1.0 the route relaxed its target to the baseline's own radius,
reported the result as if it passed, and picked on saving alone; it no longer does.

**What to do.** Check `radius_ok`. If it is False, smooth or resample a jagged input first,
or roll the baseline before the route search.
