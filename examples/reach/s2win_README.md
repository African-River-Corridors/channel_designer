# The Sassandra acceptance window (spec 0013)

Km 148 to 152 of the spec 0012 worked example, where the Sassandra opens into the Soubré reservoir. The
JRC water mask saw a channel of a median 17 m here; the open water is about 200 m. Recorded by
`tools/make_s2_window_fixture.py`, except the two files written by hand below.

| File | What |
|---|---|
| `s2win_trace.json` | The route, drawn by hand on the true-colour image of scene `S2B_29NQG_20250205_0_L2A`, down the middle of the water, along the worked example's route from its km 148 to its km 152 (downstream first). Past the junction near km 150 it takes the branch the worked example's route takes. |
| `s2win_widths.json` | The open water measured across the trace (below). |
| `stac_search_s2win.json` | The Earth Search answer for the 2024-25 dry season, cut to the two scenes of tile 29NQG (26 January, 5 February 2025) and the assets read. |
| `s2win_<date>_<band>.tif` | Windows of B02, B03, B04, B08 (10 m), B11 and SCL (20 m) of both scenes, zero beyond about 700 m from the trace. Contains modified Copernicus Sentinel data 2025. |
| `jrc_occurrence_s2win.tif` | A window of JRC occurrence. Source: EC JRC/Google. Pekel et al. 2016, Nature 540, 418-422. |

## How the widths were measured

Label: sourced - measured on the scene, by the method below.

- **Where.** A transect every 400 m along the trace, from 200 m, square to the trace's segment.
- **What.** The near-infrared reflectance (B08, 10 m) of the 5 February scene along the transect, every
  2.5 m, the mean of three samples 10 m apart along the trace. Water is dark in the near-infrared (about
  0.05 here) and the vegetated banks bright (about 0.3).
- **The edge.** Walking out from the trace each way, the first place where the reflectance crosses the
  midpoint between the water at the trace (the median within 20 m) and the land (the 90th percentile of
  the transect), placed linearly between samples.
- **Checked by eye** on the true-colour image of the same scene, transect by transect: 269 m against
  about 275 m at 600 m, 211 m against about 210 m at 1000 m. The near-infrared is not the band the water
  seed uses (MNDWI is green and shortwave-infrared), so the measure does not share the seed's
  assumptions.
- The first transect (200 m) lies among the islands just below km 148 and crosses into a side channel
  (451 m): it is kept in the median, and left out of the per-transect check.

The median of the nine transects is 219 m.
