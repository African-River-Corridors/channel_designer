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
