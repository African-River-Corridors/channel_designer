# Worked example: the Sassandra

> **Screening — not for construction.** The depth here is assumed, not measured, and several defaults are
> provisional (listed below). Read the limits before you use a number.

> **The banks cross at 24 places** (km 24 to 109 of the centreline) on the run below. `owd reach` warns
> `banks_cross` and exits 1; the script goes on, because the study is written. The width stretches near those
> places read banks that are wrong there. Fixing the crossings is a follow-up (#410).

What barge could use the Sassandra, Côte d'Ivoire, from the sea at Sassandra town up to the Soubré dam, and
what would the next size up need? This page is the answer OWD gives with nothing but open data: a river name,
two points, and one depth from people who know the river.

## Run it

```bash
bash examples/sassandra/run.sh
```

It needs the network and takes about 40 minutes, most of it the reach step (the route from Overpass and the
Sentinel-2 scenes of the edge fit). Everything is fetched at run time into `sassandra-study/`; nothing is
bundled. The script:

1. `owd reach` plans the route on the OpenStreetMap waterways (144 km, six ways). It seeds the bank lines from
   the water seen in dry-season Sentinel-2 scenes (spec 0013), checks and repairs them, then moves them onto
   the scenes' edges, tile by tile. On this river its bank checks warn that the banks still cross (see
   "Limits"), so it exits 1. The study is written all the same, and the script goes on.
2. `owd fetch-dem` gets the Copernicus GLO-30 DEM. `owd water-level` sets the low-water level from SWORD and
   SWOT, with the DEM where they give none.
3. `owd fits` gives the plan-only answer: width and radius, no depth. It reads them on the reference line
   (ADR 0014): one smooth line in the water, made from the banks, that belongs to no vessel.
4. `owd bed --depth 3 --season dry` gives the one depth people who use the river know: 3 m at low water in the
   dry season.
5. `owd fits` again gives the full answer: width, radius and depth, with a verdict for each vessel class,
   all read along the reference line.
6. The same full answer, reading the radius on the river centreline instead (`--plan-line centreline`), to
   compare.
7. `owd report --kind all --format all` writes the four reports (spec 0014) as PDF, HTML and Word into
   `reports/`.

## Check the banks by eye

Spec 0013 seeds the bank lines from the water seen in Sentinel-2 itself, checks and repairs them, and
draws them over the image so you can check a stretch:

```bash
owd qa-overlay sassandra.owd --png km148-152.png --kmz km148-152.kmz --from-km 148 --to-km 152
```

This is km 148 to 152, where the river opens into the Soubré reservoir, on the recorded window of the
acceptance test (`crates/owd-reach/tests/s2window.rs`): the left bank in red, the right in blue, the
centreline in cream, and the repaired stretches to check in red bands.

![The banks, centreline and repaired stretches of km 148 to 152 over Sentinel-2 scene S2B_29NQG_20250205](sassandra-qa-km148-152.png)

On this window the median width is 199 m from the seed and 202 m after the edge fit, against 219 m of
open water measured on the scene across nine transects. The JRC mask alone gives 112 m. No station has
both banks on one side, and the centreline is within 1 m of the banks' mid-line at the 95th percentile.

The whole-river numbers below are from a run of `bash examples/sassandra/run.sh` on the spec 0013 seed,
with main at `7609e40`, on 2026-10-22 to 23 (reach 36 min, the rest 3 min).

## The answer

Under PIANC WG 141 quality B, **no vessel class fits the Sassandra as it is.** Every class is
`navigable-with-works`: it can use the river after works. The table gives each class's constraint stretches
(where the river falls short) by kind.

| Class | Needs depth (m) | Plan only: fits? | Full, fairway (the default): verdict, stretches (width / depth / radius) | Full, centreline: verdict, stretches (width / depth / radius) |
|---|---|---|---|---|
| CEMT-IV | 3.25 | no | navigable-with-works: 55 / 1 / 5 | navigable-with-works: 55 / 1 / 667 |
| CN-IV-MOTOR | 2.08 | no | navigable-with-works: 49 / 2 / 11 | navigable-with-works: 49 / 2 / 572 |
| CN-III-MOTOR | 2.6 | no | navigable-with-works: 54 / 3 / 3 | navigable-with-works: 54 / 3 / 667 |
| CEMT-Va | 3.44 | no | navigable-with-works: 79 / 1 / 10 | navigable-with-works: 79 / 1 / 788 |
| CEMT-Vb | 4.55 | no | navigable-with-works: 237 / 1 / 25 | navigable-with-works: 237 / 1 / 820 |
| CN-II-MOTOR | 3.38 | no | navigable-with-works: 58 / 1 / 11 | navigable-with-works: 58 / 1 / 705 |
| CN-I-MOTOR | 4.16 | no | navigable-with-works: 70 / 1 / 16 | navigable-with-works: 70 / 1 / 732 |
| RWS-M12 | 5.2 | no | navigable-with-works: 134 / 1 / 15 | navigable-with-works: 134 / 1 / 837 |
| CN-III-CONVOY-2x2 | 2.6 | no | navigable-with-works: 230 / 3 / 22 | navigable-with-works: 230 / 3 / 830 |
| CEMT-VIa | 4.55 | no | navigable-with-works: 114 / 1 / 16 | navigable-with-works: 114 / 1 / 788 |
| CN-II-CONVOY-2x2 | 3.38 | no | navigable-with-works: 296 / 1 / 19 | navigable-with-works: 296 / 1 / 821 |
| CN-I-CONVOY-2x2 | 4.55 | no | navigable-with-works: 368 / 1 / 18 | navigable-with-works: 368 / 1 / 812 |

"Needs depth" is the class's design depth under the rule set. Where it is over the assumed 3 m, the whole river
is one depth stretch (works: dredge). Where it is 3 m or less, the few depth stretches left are single
stations (see "Limits").

### Three plan lines

The table above is from the run at `7609e40`, before the reference line became the default (ADR 0014). It
has no reference-line column yet; the next run of `run.sh` adds the default's numbers. Its radius columns
differ by plan line:
- **On each class's fairway fit (`--plan-line fairway`, the default in 0.3.0):** 3 to 25 radius stretches per class.
- **On the river centreline (`--plan-line centreline`):** 572 to 837.

On an earlier run (the spec 0012 bank seed), the 12 classes together had about 1,400 radius stretches on the
reference line, 3,300 on the fairway fits and 12,000 on the centreline.

The centreline is drawn from the bank lines, and it wiggles at the scale of its 5 m stations. Each wiggle
reads as a tight bend, so most of its radius stretches are not the river. The fairway fit (ADR 0007) is
the smoothest line a channel of one class can take in the water. The reference line (spec 0016) is one
smooth line in the water for every vessel: the smoothed mid-line of the banks, refitted in the water. Width
and depth are read along the plan line too. The assumed bed is deepest on the reference line (#398).

The answer says which line it read: `Plan line: reference` in the table, `plan_line` in the JSON.

## Labels and notices

Every output carries the screening label, and every answer carries "depth assumed, not measured". The study
carries these notices, and every export carries them too:

- © OpenStreetMap contributors (the route);
- Source: EC JRC/Google, Pekel et al. 2016 (the water mask);
- Contains modified Copernicus Sentinel data 2024 and 2025 (the bank lines: six scenes,
  S2A_29NRF_20241222, S2B_29NRF_20250205, S2C_29NRF_20251217, S2B_29NQF_20250205, S2B_29NQG_20250126 and
  S2B_29NQG_20250205);
- SWOT data from NASA and CNES, served by PO.DAAC Hydrocron, and SWORD v17c, CC BY 4.0 (the water level,
  from 10 SWORD reaches, observations from July 2023 to October 2026);
- Produced using Copernicus WorldDEM-30 (the DEM).

## Provisional defaults

These are not yet calibrated against a surveyed reach. Each is named so in the study file.

| Default | Value | Used here |
|---|---|---|
| Bank seed | the water seen in dry-season Sentinel-2 (spec 0013) | as default; it agrees with the JRC mask on 56 % of the water |
| Water mask threshold (JRC occurrence) | 75 % | **30 %**: at 75 % the mask sees almost no water on this cloudy reach |
| Low-confidence width | under 3 mask pixels (90 m); under 30 m after the edge fit | as default |
| Edge fit | ±40 m search, at most 30 m shift, at most 1.5 m change per 5 m | as default, in parts of 15 km |
| Water level | SWOT 10th percentile per SWORD reach; the DEM under 100 m wide or with no SWOT | as default |
| Depth | none: the user gives one | 3 m, dry season |
| Bed shape | a parabola across the water | as default |

## Limits

- **The banks.** The centreline is 150.7 km for the 144.0 km route (1.05 times). 649 stretches are
  low-confidence width:
  - 357 `repaired`: a check failed there and the bank was rebuilt. The checks behind the repairs:
    width-narrow 159, jump 104, excursion 79, width-wide 30, **crossing 23**, side 11, missing 4.
  - 191 `no-edge`: no clear edge, so the seed bank is kept.
  - 92 `at-shift-limit`: the water runs on past the 30 m limit, so the bank moved 30 m and the edge lies
    farther.
  - 9 `narrow`: under 30 m wide.
- **The banks still cross.** After the repairs, the left and right banks cross at 24 places, between km 24
  and km 109 of the centreline. `owd reach` warns `banks_cross` and exits 1; the study is still written, and
  the script goes on. The width stretches near those places read banks that are wrong there. Fixing the
  crossings is a follow-up (#410).
- **The depth is one assumed number.** Most classes need more than 3 m, so the whole river is one depth
  stretch for them. At a few stations the edge-fitted centreline leaves the water the bed model covers. There
  the DEM surface (which sits above the water) is read instead, and the depth found there is negative.
  These show as single-station depth stretches.
- **The plan line** is the reference line (above), in the plan-only answer and the full one. The bed
  model's parabola is centred on the reference line (#398), and the depth table and water level still read
  the centreline's chainage.
- **Width:** the width stretches use the banks as found, with the limits above.
- **Air draught, rapids and rock** are out of scope (#12, #13). The Soubré dam is the end of the reach.

## Reports

The four reports of the study, from step 7, as PDF:

- [Screening report](sassandra-reports/owd-sassandra-sr.pdf): the reach, the data and its limits, the verdict
  for each vessel class, and the Rule Trace.
- [Quantity schedule](sassandra-reports/owd-sassandra-qs.pdf), [section pack](sassandra-reports/owd-sassandra-sp.pdf):
  the study stops at the fits answer, so it has no design line option. These two say so; they fill in once a
  vessel is chosen and `owd size`, `owd align` and `owd volumes` have run.
- [Run report](sassandra-reports/owd-sassandra-rr.pdf): the versions, the stages and the hashes.

They were made from the same run. Read them with the bank limits: the centreline is 150.7 km for the 144 km
route (1.05 times), 649 stretches are low-confidence width, and the banks cross at 24 places. The first page of each
report says so in its "Data and limits" box. Run `bash examples/sassandra/run.sh` to make them again.

## In the app

1. Press **Start from a river**.
2. Type *Sassandra*, then pick the start at the mouth and the end below the dam on the map (or type
   `-6.0780, 4.9605` and `-6.6550, 5.8020`). Press **Find the route**.
3. On the desktop app the JRC tiles are read for you. On the web, download the tile listed (`occurrence_10W_10N…`)
   and give it back. Keep **Move the banks onto Sentinel-2 imagery** ticked, then press **Make the inputs**.
4. Fetch or load the DEM (the tiles are listed). The water level comes from SWORD and SWOT.
5. Give a depth of 3 m, season *dry*, and press **Set the depth**.
6. Press **Answer what fits**. Pick a row to see its stretches on the map, and **Use this vessel** to go on to
   the alignment and the volumes.

The app's mask threshold is the default (75 %). For this river, use the CLI script above, or lower the
threshold once the app offers it.
